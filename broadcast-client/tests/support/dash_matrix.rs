//! The fault-injection matrix harness (round-8 template): a fault kind x request stage x
//! transient/permanent grid driven through a DASH `Session`, classified per cell. A cell never
//! panics, never hangs, never loops on the wire, never delivers a duplicate or out-of-order
//! segment, and ends in the state the fault calls for.

#![allow(dead_code)]

use std::cell::RefCell;
use std::collections::{BTreeMap, BTreeSet};
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::rc::Rc;

use broadcast_client::{Config, Event, Status};

use super::dash_common::synth::parse_segment_url;
use super::dash_sim::{Answer, Backend, DashSim, Logged, SEC};

/// What a request is for. A fault names the classes it can be applied to; a source names the
/// stages (one per class at least) it offers; the exhaustiveness guard crosses the two.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Class {
    Mpd,
    MpdRefresh,
    MpdBoundary,
    Utc,
    Init,
    Index,
    Media,
}

impl Class {
    pub const ALL: &'static [Class] = &[
        Class::Mpd,
        Class::MpdRefresh,
        Class::MpdBoundary,
        Class::Utc,
        Class::Init,
        Class::Index,
        Class::Media,
    ];

    pub fn name(self) -> &'static str {
        match self {
            Class::Mpd => "mpd",
            Class::MpdRefresh => "mpd-refresh",
            Class::MpdBoundary => "mpd-boundary",
            Class::Utc => "utc",
            Class::Init => "init",
            Class::Index => "index",
            Class::Media => "media",
        }
    }
}

/// What a source must have for a fault to be applicable to it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Cap {
    Live,
    HeadUtc,
    MultiPeriod,
    Sidx,
}

macro_rules! faults {
    ($($variant:ident => $short:literal),+ $(,)?) => {
        #[derive(Clone, Copy, Debug, PartialEq, Eq)]
        pub enum F {
            $($variant),+
        }

        impl F {
            pub const ALL: &'static [F] = &[$(F::$variant),+];

            pub fn short(self) -> &'static str {
                match self {
                    $(F::$variant => $short),+
                }
            }
        }
    };
}

faults! {
    S500 => "500",
    S503 => "503",
    S429 => "429",
    S429Huge => "429H",
    S404 => "404",
    S401 => "401",
    S403 => "403",
    ConnFail => "conn",
    ResetMid => "rst",
    ShortBody => "shrt",
    TruncChunk => "trnc",
    Stall => "stall",
    NoResp => "nores",
    Garbage => "garb",
    Html => "html",
    Empty => "empty",
    Huge => "huge",
    Late => "late",
    DupResp => "dup",
    Unknown => "unk",
    ClosedTwice => "cls2",
    RedirLoop => "loop",
    S425 => "425",
    S425RetryAfter => "425RA",
    S410 => "410",
    RetryAfterDate => "RAdate",
    UtcSkew => "uskew",
    UtcRttTooLarge => "urtt",
    UtcDateMissing => "udate",
    UtcHeadRefused => "uhead",
    MpdTruncatedXml => "mtrnc",
    MpdStale => "mstale",
    MpdNotModified => "m304",
    MpdAstChanged => "mast",
    MpdTypeFlip => "mflip",
    MpdNoPeriod => "mnoper",
    MpdRepVanishes => "mrepv",
    PeriodOldestRemoved => "poldrm",
    PeriodNewestDropped => "pnewdr",
    PeriodDuplicated => "pdup",
    SidxSizeMismatch => "sxsize",
    SidxWrongBox => "sxbox",
    RangeIgnored => "rng200",
    InitFtypOnly => "ftyp",
    MediaNoMoof => "nomoof",
}

const ANY_REQUEST: &[Class] = Class::ALL;
const ANY_MPD: &[Class] = &[Class::Mpd, Class::MpdRefresh, Class::MpdBoundary];
const REFRESH_ONLY: &[Class] = &[Class::MpdRefresh, Class::MpdBoundary];
const BOUNDARY_ONLY: &[Class] = &[Class::MpdBoundary];
const UTC_ONLY: &[Class] = &[Class::Utc];
const INIT_ONLY: &[Class] = &[Class::Init];
const INDEX_ONLY: &[Class] = &[Class::Index];
const MEDIA_ONLY: &[Class] = &[Class::Media];
const RANGED: &[Class] = &[Class::Init, Class::Index, Class::Media];

impl F {
    pub fn benign(self) -> bool {
        matches!(self, F::DupResp | F::Unknown | F::ClosedTwice)
    }

    /// The request classes this fault is applied to. Exhaustive on purpose: a new fault does not
    /// compile until it states where it belongs.
    pub fn classes(self) -> &'static [Class] {
        match self {
            F::S500
            | F::S503
            | F::S429
            | F::S429Huge
            | F::S404
            | F::S401
            | F::S403
            | F::ConnFail
            | F::ResetMid
            | F::ShortBody
            | F::TruncChunk
            | F::Stall
            | F::NoResp
            | F::Garbage
            | F::Html
            | F::Empty
            | F::Huge
            | F::Late
            | F::DupResp
            | F::Unknown
            | F::ClosedTwice
            | F::RedirLoop
            | F::S425
            | F::S425RetryAfter
            | F::S410
            | F::RetryAfterDate => ANY_REQUEST,
            F::UtcSkew | F::UtcRttTooLarge | F::UtcDateMissing | F::UtcHeadRefused => UTC_ONLY,
            F::MpdTruncatedXml | F::MpdNoPeriod => ANY_MPD,
            F::MpdStale
            | F::MpdNotModified
            | F::MpdAstChanged
            | F::MpdTypeFlip
            | F::MpdRepVanishes => REFRESH_ONLY,
            F::PeriodOldestRemoved | F::PeriodNewestDropped | F::PeriodDuplicated => BOUNDARY_ONLY,
            F::SidxSizeMismatch | F::SidxWrongBox => INDEX_ONLY,
            F::RangeIgnored => RANGED,
            F::InitFtypOnly => INIT_ONLY,
            F::MediaNoMoof => MEDIA_ONLY,
        }
    }

    pub fn needs(self) -> &'static [Cap] {
        match self {
            F::UtcDateMissing | F::UtcHeadRefused => &[Cap::HeadUtc],
            F::MpdStale
            | F::MpdNotModified
            | F::MpdAstChanged
            | F::MpdTypeFlip
            | F::MpdRepVanishes => &[Cap::Live],
            F::PeriodOldestRemoved | F::PeriodNewestDropped | F::PeriodDuplicated => {
                &[Cap::MultiPeriod]
            }
            F::SidxSizeMismatch | F::SidxWrongBox | F::RangeIgnored => &[Cap::Sidx],
            _ => &[],
        }
    }

    pub fn applies_to(self, class: Class, caps: &[Cap]) -> bool {
        self.classes().contains(&class) && self.needs().iter().all(|c| caps.contains(c))
    }
}

#[derive(Clone, Debug)]
pub enum Matcher {
    Url(String),
    /// A ranged request to `url` whose first byte is `from`.
    Ranged {
        url: String,
        from: u64,
    },
    MpdAll,
    MpdRefresh,
    /// The MPD request with this zero-based index and every later one.
    MpdFrom(usize),
}

#[derive(Clone, Debug)]
pub struct Stage {
    pub name: String,
    pub class: Class,
    pub matcher: Matcher,
}

#[derive(Clone, Debug)]
pub struct Plan {
    pub stage: Stage,
    pub fault: F,
    pub permanent: bool,
}

pub struct Limits {
    pub manifest: usize,
    pub segment: usize,
}

pub type Faulted = Rc<RefCell<Vec<(String, u64, bool)>>>;

pub struct Faulty {
    pub inner: Box<dyn Backend>,
    pub plan: Option<Plan>,
    pub limits: Limits,
    mpd_seen: usize,
    first_mpd: Option<Vec<u8>>,
    faulted: Faulted,
}

impl Faulty {
    pub fn new(
        inner: Box<dyn Backend>,
        plan: Option<Plan>,
        limits: Limits,
        faulted: Faulted,
    ) -> Self {
        Self {
            inner,
            plan,
            limits,
            mpd_seen: 0,
            first_mpd: None,
            faulted,
        }
    }
}

fn bytes_of(answer: &Answer) -> Option<&Vec<u8>> {
    match answer {
        Answer::Full { body, .. } => Some(body),
        _ => None,
    }
}

fn garbage() -> Vec<u8> {
    (0..4096u32)
        .map(|i| (i.wrapping_mul(2_654_435_761) >> 13) as u8)
        .collect()
}

const UTC_SKEW_US: i64 = 45 * 1_000_000;
const UTC_RTT_TOO_LARGE_US: u64 = 3 * SEC;
const RETRY_AFTER_DATE_US: i64 = 3 * 1_000_000;
const TRUNCATED_XML_DIVISOR: usize = 5;
const SIDX_SIZE_SKEW: u32 = 16;
const AST_SHIFT_SECS: i64 = 3_600;
const BOX_HEADER: usize = 8;

fn full_ok(good: &Answer) -> Option<(u16, &Headers, &Vec<u8>)> {
    match good {
        Answer::Full {
            status: status @ (200 | 206),
            headers,
            body,
        } => Some((*status, headers, body)),
        _ => None,
    }
}

type Headers = Vec<(String, String)>;

fn edit_text(good: &Answer, edit: impl FnOnce(&str) -> Option<String>) -> Option<Answer> {
    let (status, headers, body) = full_ok(good)?;
    let new = edit(std::str::from_utf8(body).ok()?)?;
    Some(Answer::Full {
        status,
        headers: headers.clone(),
        body: new.into_bytes(),
    })
}

fn edit_bytes(good: &Answer, edit: impl FnOnce(&mut Vec<u8>) -> bool) -> Option<Answer> {
    let (status, headers, body) = full_ok(good)?;
    let mut new = body.clone();
    edit(&mut new).then(|| Answer::Full {
        status,
        headers: headers.clone(),
        body: new,
    })
}

fn period_spans(text: &str) -> Vec<(usize, usize)> {
    const OPEN: &str = "<Period";
    const CLOSE: &str = "</Period>";
    let mut spans = Vec::new();
    let mut from = 0;
    while let Some(start) = text[from..].find(OPEN).map(|i| from + i) {
        let Some(end) = text[start..].find(CLOSE).map(|i| start + i + CLOSE.len()) else {
            break;
        };
        spans.push((start, end));
        from = end;
    }
    spans
}

fn shift_instant(text: &str, secs: i64) -> Option<String> {
    let ts: jiff::Timestamp = text.trim().parse().ok()?;
    let shifted = ts.checked_add(jiff::SignedDuration::from_secs(secs)).ok()?;
    Some(super::dash_common::world::xs_datetime_ms(
        shifted.as_microsecond(),
    ))
}

fn shift_http_date(value: &str, secs: i64) -> Option<String> {
    let zoned = jiff::fmt::rfc2822::parse(value).ok()?;
    Some(super::dash_common::world::http_date(
        zoned.timestamp().as_microsecond() + secs * 1_000_000,
    ))
}

fn remove_first_representation(text: &str) -> Option<String> {
    const OPEN: &str = "<Representation";
    let start = text.find(OPEN)?;
    let open_end = start + text[start..].find('>')?;
    let end = if text[..open_end].ends_with('/') {
        open_end + 1
    } else {
        let close = "</Representation>";
        open_end + text[open_end..].find(close)? + close.len()
    };
    Some(format!("{}{}", &text[..start], &text[end..]))
}

fn attribute<'t>(text: &'t str, name: &str) -> Option<&'t str> {
    let key = format!("{name}=\"");
    let from = text.find(&key)? + key.len();
    let len = text[from..].find('"')?;
    Some(&text[from..from + len])
}

fn flip_to_static(text: &str) -> Option<String> {
    const DYNAMIC: &str = "type=\"dynamic\"";
    let ast: jiff::Timestamp = attribute(text, "availabilityStartTime")?.parse().ok()?;
    let published: jiff::Timestamp = attribute(text, "publishTime")?.parse().ok()?;
    let published_secs = published.as_second() - ast.as_second();
    (published_secs > 0 && text.contains(DYNAMIC)).then(|| {
        text.replace(
            DYNAMIC,
            &format!("type=\"static\" mediaPresentationDuration=\"PT{published_secs}S\""),
        )
    })
}

fn move_ast(text: &str) -> Option<String> {
    const KEY: &str = "availabilityStartTime=\"";
    let from = text.find(KEY)? + KEY.len();
    let len = text[from..].find('"')?;
    let moved = shift_instant(&text[from..from + len], AST_SHIFT_SECS)?;
    Some(format!("{}{}{}", &text[..from], moved, &text[from + len..]))
}

fn apply(
    f: F,
    good: Answer,
    req: &Logged,
    limit: usize,
    first_mpd: Option<&Vec<u8>>,
) -> Option<Answer> {
    let body = bytes_of(&good).cloned().unwrap_or_default();
    let half = body.len() / 2;
    Some(match f {
        F::S500 => Answer::status(500),
        F::S503 => Answer::status(503),
        F::S429 => Answer::status_with(429, vec![("Retry-After".into(), "3".into())]),
        F::S429Huge => Answer::status_with(429, vec![("Retry-After".into(), u64::MAX.to_string())]),
        F::S404 => Answer::status(404),
        F::S401 => Answer::status_with(
            401,
            vec![("WWW-Authenticate".into(), "Basic realm=\"r\"".into())],
        ),
        F::S403 => Answer::status(403),
        F::ConnFail => Answer::TransportError,
        F::ResetMid | F::TruncChunk => Answer::ResetMid(body[..half].to_vec()),
        F::ShortBody => Answer::Truncated {
            body: body[..half].to_vec(),
            declared: body.len(),
        },
        F::Stall => Answer::Stall(body[..half].to_vec()),
        F::NoResp => Answer::Hang,
        F::Garbage => Answer::ok(garbage()),
        F::Html => Answer::ok(b"<html><body>Service unavailable, try later</body></html>".to_vec()),
        F::Empty => Answer::ok(Vec::new()),
        F::Huge => Answer::ok(vec![0u8; limit + (1 << 16)]),
        F::Late => Answer::Late(Box::new(good), 300 * SEC),
        F::DupResp => Answer::Duplicate(Box::new(good)),
        F::Unknown => Answer::Unknown(Box::new(good)),
        F::ClosedTwice => Answer::ClosedTwice(Box::new(good)),
        F::RedirLoop => Answer::Redirect(req.url.clone()),
        F::S425 => Answer::Full {
            status: 425,
            headers: Vec::new(),
            body: b"earl".to_vec(),
        },
        F::S425RetryAfter => Answer::status_with(425, vec![("Retry-After".into(), "2".into())]),
        F::S410 => Answer::status(410),
        F::RetryAfterDate => Answer::status_with(
            503,
            vec![(
                "Retry-After".into(),
                super::dash_common::world::http_date(req.send_utc_us + RETRY_AFTER_DATE_US),
            )],
        ),
        F::UtcSkew => {
            if let Some(a) = edit_text(&good, |t| shift_instant(t, UTC_SKEW_US / 1_000_000)) {
                a
            } else {
                let Answer::Full {
                    status,
                    mut headers,
                    body,
                } = good
                else {
                    return None;
                };
                let date = headers
                    .iter_mut()
                    .find(|(k, _)| k.eq_ignore_ascii_case("date"))?;
                date.1 = shift_http_date(&date.1, UTC_SKEW_US / 1_000_000)?;
                Answer::Full {
                    status,
                    headers,
                    body,
                }
            }
        }
        F::UtcRttTooLarge => Answer::Late(Box::new(good), UTC_RTT_TOO_LARGE_US),
        F::UtcDateMissing => {
            let Answer::Full {
                status,
                mut headers,
                body,
            } = good
            else {
                return None;
            };
            let before = headers.len();
            headers.retain(|(k, _)| !k.eq_ignore_ascii_case("date"));
            (headers.len() != before).then_some(())?;
            Answer::Full {
                status,
                headers,
                body,
            }
        }
        F::UtcHeadRefused => {
            (req.method == "HEAD").then_some(())?;
            Answer::status(405)
        }
        F::MpdTruncatedXml => edit_text(&good, |t| {
            let mut cut = t.len() - t.len() / TRUNCATED_XML_DIVISOR;
            while !t.is_char_boundary(cut) {
                cut -= 1;
            }
            Some(t[..cut].to_owned())
        })?,
        F::MpdStale => {
            let first = first_mpd?;
            (full_ok(&good)?.2 != first).then_some(())?;
            Answer::ok(first.clone())
        }
        F::MpdNotModified => {
            full_ok(&good)?;
            Answer::status(304)
        }
        F::MpdAstChanged => edit_text(&good, move_ast)?,
        F::MpdTypeFlip => edit_text(&good, flip_to_static)?,
        F::MpdNoPeriod => edit_text(&good, |t| {
            let spans = period_spans(t);
            let (first, last) = (spans.first()?, spans.last()?);
            Some(format!("{}{}", &t[..first.0], &t[last.1..]))
        })?,
        F::MpdRepVanishes => edit_text(&good, remove_first_representation)?,
        F::PeriodOldestRemoved => edit_text(&good, |t| {
            let spans = period_spans(t);
            (spans.len() >= 2).then(|| format!("{}{}", &t[..spans[0].0], &t[spans[0].1..]))
        })?,
        F::PeriodNewestDropped => edit_text(&good, |t| {
            let spans = period_spans(t);
            let last = spans.last().filter(|_| spans.len() >= 2)?;
            Some(format!("{}{}", &t[..last.0], &t[last.1..]))
        })?,
        F::PeriodDuplicated => edit_text(&good, |t| {
            let spans = period_spans(t);
            let last = spans.last()?;
            Some(format!("{}{}", &t[..last.1], &t[last.0..]))
        })?,
        F::SidxSizeMismatch => edit_bytes(&good, |b| {
            let Some(size) = b
                .get(..4)
                .map(|s| u32::from_be_bytes([s[0], s[1], s[2], s[3]]))
            else {
                return false;
            };
            b[..4].copy_from_slice(&(size + SIDX_SIZE_SKEW).to_be_bytes());
            true
        })?,
        F::SidxWrongBox => edit_bytes(&good, |b| {
            if b.len() < BOX_HEADER {
                return false;
            }
            b[4..8].copy_from_slice(b"free");
            true
        })?,
        F::RangeIgnored => return None,
        F::InitFtypOnly => edit_bytes(&good, |b| {
            let Some(size) = b
                .get(..4)
                .map(|s| u32::from_be_bytes([s[0], s[1], s[2], s[3]]))
            else {
                return false;
            };
            let size = size as usize;
            if size < BOX_HEADER || size >= b.len() {
                return false;
            }
            b.truncate(size);
            true
        })?,
        F::MediaNoMoof => edit_bytes(&good, |b| {
            let Some(at) = b.windows(4).position(|w| w == b"moof") else {
                return false;
            };
            b[at..at + 4].copy_from_slice(b"free");
            true
        })?,
    })
}

impl Backend for Faulty {
    fn serve(&mut self, at_utc_us: i64, req: &Logged) -> Answer {
        let good = self.inner.serve(at_utc_us, req);
        let is_mpd = req.is_mpd();
        let index = if is_mpd {
            let i = self.mpd_seen;
            self.mpd_seen += 1;
            i
        } else {
            0
        };
        let first_mpd_before = self.first_mpd.clone();
        if is_mpd
            && index == 0
            && let Some(b) = bytes_of(&good)
        {
            self.first_mpd = Some(b.clone());
        }
        let Some(plan) = self.plan.clone() else {
            return good;
        };
        let matched = match &plan.stage.matcher {
            Matcher::Url(u) => req.url == *u,
            Matcher::Ranged { url, from } => {
                req.url == *url && req.byte_range.is_some_and(|(off, _)| off == *from)
            }
            Matcher::MpdAll => is_mpd,
            Matcher::MpdRefresh => is_mpd && index >= 1,
            Matcher::MpdFrom(n) => is_mpd && index >= *n,
        };
        if !matched || (!plan.permanent && !self.faulted.borrow().is_empty()) {
            return good;
        }
        let limit = if is_mpd {
            self.limits.manifest
        } else {
            self.limits.segment
        };
        let changed = if plan.fault == F::RangeIgnored {
            req.byte_range.map(|_| {
                let whole = Logged {
                    byte_range: None,
                    ..req.clone()
                };
                self.inner.serve(at_utc_us, &whole)
            })
        } else {
            apply(
                plan.fault,
                good.clone(),
                req,
                limit,
                first_mpd_before.as_ref(),
            )
        };
        let Some(answer) = changed else {
            return good;
        };
        self.faulted.borrow_mut().push((
            req.url.clone(),
            req.send_elapsed_us,
            !plan.fault.benign(),
        ));
        answer
    }
}

pub struct Out {
    pub events: Vec<Event>,
    pub log: Vec<Logged>,
    pub end_elapsed_us: u64,
    pub hung: bool,
    pub loop_cap: bool,
    pub panicked: bool,
    pub hit: bool,
    pub faulted: Vec<(String, u64, bool)>,
}

pub fn terminal(events: &[Event]) -> bool {
    events.iter().any(|e| {
        matches!(
            e,
            Event::Status(Status::Ended)
                | Event::Status(Status::NotFound)
                | Event::Status(Status::WaitingForCredentials)
                | Event::Error { fatal: true, .. }
                | Event::Forbidden { .. }
        )
    })
}

pub fn sample_key(e: &Event) -> Option<(u32, String, usize)> {
    if let Event::Sample { track, sample, .. } = e {
        Some((
            track.value(),
            format!("{:?}", sample.dts),
            sample.data.len(),
        ))
    } else {
        None
    }
}

fn per_track(s: &[(u32, String, usize)]) -> BTreeMap<u32, Vec<(String, usize)>> {
    let mut m: BTreeMap<u32, Vec<(String, usize)>> = BTreeMap::new();
    for (t, d, l) in s {
        m.entry(*t).or_default().push((d.clone(), *l));
    }
    m
}

pub fn samples(events: &[Event]) -> Vec<(u32, String, usize)> {
    events.iter().filter_map(sample_key).collect()
}

pub struct Src {
    pub name: &'static str,
    pub live: bool,
    pub caps: &'static [Cap],
    pub config: Config,
    pub limits: fn() -> Limits,
    pub build: Box<dyn Fn() -> (DashSim, Vec<Stage>)>,
    pub horizon_us: u64,
}

pub fn run_cell(src: &Src, plan: Option<Plan>) -> Out {
    let r = catch_unwind(AssertUnwindSafe(|| {
        let (mut sim, _) = (src.build)();
        let inner = std::mem::replace(&mut sim.backend, Box::new(NullBackend));
        let faulted = Rc::new(RefCell::new(Vec::new()));
        sim.backend = Box::new(Faulty::new(
            inner,
            plan.clone(),
            (src.limits)(),
            faulted.clone(),
        ));
        let horizon = src.horizon_us;
        if src.live {
            sim.run_for(horizon);
        } else {
            sim.run_to_terminal(horizon);
        }
        let hung = !src.live && !terminal(&sim.events) && sim.idle();
        Out {
            end_elapsed_us: sim.world.elapsed_us,
            loop_cap: sim.log.len() >= sim.request_cap,
            events: sim.events,
            log: sim.log,
            hung,
            panicked: false,
            hit: plan.is_none() || !faulted.borrow().is_empty(),
            faulted: faulted.borrow().clone(),
        }
    }));
    r.unwrap_or(Out {
        events: vec![],
        log: vec![],
        end_elapsed_us: 0,
        hung: false,
        loop_cap: false,
        panicked: true,
        hit: true,
        faulted: vec![],
    })
}

pub struct NullBackend;

impl Backend for NullBackend {
    fn serve(&mut self, _: i64, _: &Logged) -> Answer {
        Answer::Hang
    }
}

/// What the successful segment fetches of a live run look like, per Representation.
#[derive(Debug, Default)]
pub struct Served {
    pub repeated: Option<String>,
    pub gap: Option<String>,
}

pub fn served_segments(log: &[Logged], faulted: &[(String, u64, bool)]) -> Served {
    let mut by_rep: BTreeMap<String, Vec<u64>> = BTreeMap::new();
    for l in log {
        if faulted
            .iter()
            .any(|(u, at, bad)| *bad && *u == l.url && *at == l.send_elapsed_us)
        {
            continue;
        }
        if !l.status.is_some_and(|s| (200..300).contains(&s)) {
            continue;
        }
        if let Some((rep, n)) = parse_segment_url(&l.url) {
            by_rep.entry(rep).or_default().push(n);
        }
    }
    let mut out = Served::default();
    for (rep, mut v) in by_rep {
        let total = v.len();
        v.sort_unstable();
        v.dedup();
        if v.len() != total && out.repeated.is_none() {
            out.repeated = Some(format!("{rep}: a segment fetched twice"));
        }
        let step = v.windows(2).map(|w| w[1] - w[0]).min().unwrap_or(1);
        if v.windows(2).any(|w| w[1] - w[0] != step) && out.gap.is_none() {
            out.gap = Some(format!("{rep}: gap in {v:?}"));
        }
    }
    out
}

const RATE_WINDOW_US: u64 = SEC;
const REDIRECT_HOP_US: u64 = 100_000;
pub const RATE_CAP_PER_WINDOW: usize = 4;

/// The most requests to any ONE url inside any one-second window: the signature of a retry loop
/// (the round-8 finding was 604 requests in 602 s on one target). The hops of a redirect chain
/// are one attempt, and catch-up bursts across different segments are not a loop.
pub fn peak_rate_per_url(log: &[Logged]) -> (usize, String) {
    let mut times: BTreeMap<&str, Vec<u64>> = BTreeMap::new();
    let mut previous: BTreeMap<&str, &Logged> = BTreeMap::new();
    for l in log {
        let hop = previous.get(l.url.as_str()).is_some_and(|p| {
            p.status == Some(302)
                && l.send_elapsed_us.saturating_sub(p.send_elapsed_us) < REDIRECT_HOP_US
        });
        previous.insert(l.url.as_str(), l);
        if !hop {
            times
                .entry(l.url.as_str())
                .or_default()
                .push(l.send_elapsed_us);
        }
    }
    let mut worst = (0, String::new());
    for (url, t) in times {
        let mut lo = 0;
        for hi in 0..t.len() {
            while t[hi] - t[lo] >= RATE_WINDOW_US {
                lo += 1;
            }
            if hi - lo + 1 > worst.0 {
                worst = (hi - lo + 1, url.to_owned());
            }
        }
    }
    worst
}

pub struct Baseline {
    pub samples: Vec<(u32, String, usize)>,
    pub rate_cap: usize,
}

impl Baseline {
    fn of(out: &Out) -> Self {
        Self {
            samples: samples(&out.events),
            rate_cap: peak_rate_per_url(&out.log).0.max(RATE_CAP_PER_WINDOW),
        }
    }

    fn over_cap(&self, log: &[Logged]) -> Option<String> {
        let (peak, url) = peak_rate_per_url(log);
        (peak > self.rate_cap).then(|| format!("{url} {peak}/s over {}", self.rate_cap))
    }
}

impl F {
    /// Faults that make the client drop its timeline and rejoin (a changed availability start,
    /// a Period that vanished or was duplicated): the rejoin may repeat or skip segments, but it
    /// must announce itself with a `Discontinuity`.
    pub fn resets(self) -> bool {
        matches!(
            self,
            F::MpdAstChanged
                | F::PeriodOldestRemoved
                | F::PeriodNewestDropped
                | F::PeriodDuplicated
        )
    }

    /// The fatal error a single occurrence of this fault is specified to end the session with
    /// (an MPD with no Period cannot start a presentation).
    pub fn specified_fatal(self, class: Class) -> Option<&'static str> {
        match (self, class) {
            (F::MpdNoPeriod, Class::Mpd) => Some("mpd_invalid"),
            _ => None,
        }
    }
}

fn is_disc(e: &Event) -> bool {
    matches!(e, Event::Discontinuity)
}

pub fn classify(
    src: &Src,
    plan: &Plan,
    perm: bool,
    o: &Out,
    baseline: &Baseline,
) -> (String, String) {
    let base = &baseline.samples[..];
    if o.panicked {
        return ("PANIC".into(), String::new());
    }
    let s = samples(&o.events);
    let fatals: Vec<&'static str> = o
        .events
        .iter()
        .filter_map(|e| match e {
            Event::Error { kind, fatal: true } => Some(kind.name()),
            _ => None,
        })
        .collect();
    let ended = o
        .events
        .iter()
        .any(|e| matches!(e, Event::Status(Status::Ended)));
    let notfound = o
        .events
        .iter()
        .any(|e| matches!(e, Event::Status(Status::NotFound)));
    let wait = o
        .events
        .iter()
        .any(|e| matches!(e, Event::Status(Status::WaitingForCredentials)));
    let forb = o
        .events
        .iter()
        .any(|e| matches!(e, Event::Forbidden { .. }));
    let discs = o.events.iter().filter(|e| is_disc(e)).count();
    let tail = format!(
        "samples={}/{} disc={} fatals={:?} ended={} nf={} wait={} forb={} reqs={} t={}s",
        s.len(),
        base.len(),
        discs,
        fatals,
        ended,
        notfound,
        wait,
        forb,
        o.log.len(),
        o.end_elapsed_us / SEC
    );
    if !o.hit {
        return ("NOHIT".into(), tail);
    }
    if o.hung {
        return ("HANG".into(), tail);
    }
    if fatals.len() > 1 {
        return ("MULTIFATAL".into(), tail);
    }
    let mut seen_fatal = false;
    for e in &o.events {
        if matches!(e, Event::Error { fatal: true, .. }) {
            seen_fatal = true;
        } else if seen_fatal && sample_key(e).is_some() {
            return ("POSTFATAL".into(), tail);
        }
    }
    let busy = baseline.over_cap(&o.log).is_some();
    if src.live {
        if o.loop_cap {
            return ("LOOP".into(), tail);
        }
        let served = served_segments(&o.log, &o.faulted);
        let allow_gaps = perm || plan.fault == F::S429Huge;
        if plan.fault.resets() {
            if (served.repeated.is_some() || served.gap.is_some()) && discs == 0 {
                return ("NODISC".into(), format!("{tail} {served:?}"));
            }
        } else if let Some(why) = served.repeated.or(served.gap.filter(|_| !allow_gaps)) {
            return ("GAPDUP".into(), format!("{tail} {why}"));
        }
        if let Some(why) = baseline.over_cap(&o.log) {
            return ("BUSY".into(), format!("{tail} {why}"));
        }
        if !perm {
            if !fatals.is_empty() {
                return (format!("FATAL:{}", fatals[0]), tail);
            }
            if wait {
                return ("AUTH".into(), tail);
            }
            if s.is_empty() {
                return ("SILENT".into(), tail);
            }
            return ("OK".into(), tail);
        }
        if fatals.len() == 1 || notfound || wait || forb || ended {
            return ("OK".into(), tail);
        }
        if busy {
            return ("BUSY".into(), tail);
        }
        return ("RETRY".into(), tail);
    }
    if !perm {
        let complete = per_track(&s) == per_track(base);
        if complete && fatals.is_empty() && ended {
            return ("OK".into(), tail);
        }
        if wait {
            return ("AUTH".into(), tail);
        }
        if !fatals.is_empty() {
            return (format!("FATAL:{}", fatals[0]), tail);
        }
        if notfound {
            return ("NOTFOUND".into(), tail);
        }
        if ended {
            return ("LOSS".into(), tail);
        }
        if o.loop_cap {
            return ("LOOP".into(), tail);
        }
        return ("INCOMPLETE".into(), tail);
    }
    if ended && fatals.is_empty() && !notfound && !wait && !forb && s.len() < base.len() {
        return ("ENDEDLOSS".into(), tail);
    }
    if fatals.len() == 1 || notfound || wait || forb || ended {
        return ("OK".into(), tail);
    }
    if busy {
        return ("BUSY".into(), tail);
    }
    ("RETRY".into(), tail)
}

/// One executed cell: which source, stage class and fault, transient or permanent.
pub type Cell = (&'static str, Class, F, bool);

pub fn matrix(src: &Src) -> (Vec<String>, Vec<Cell>) {
    let (base_o, stages) = {
        let (mut sim, stages) = (src.build)();
        if src.live {
            sim.run_for(src.horizon_us);
        } else {
            sim.run_to_terminal(src.horizon_us);
        }
        (
            Out {
                end_elapsed_us: sim.world.elapsed_us,
                loop_cap: false,
                events: sim.events,
                log: sim.log,
                hung: false,
                panicked: false,
                hit: true,
                faulted: vec![],
            },
            stages,
        )
    };
    let base = Baseline::of(&base_o);
    assert!(
        !base.samples.is_empty(),
        "{}: the baseline produced no samples",
        src.name
    );
    let mut bad = Vec::new();
    let mut table = Vec::new();
    let mut cells = Vec::new();
    for stage in &stages {
        let mut row = String::new();
        for &f in F::ALL {
            if !f.applies_to(stage.class, src.caps) {
                continue;
            }
            let tplan = Plan {
                stage: stage.clone(),
                fault: f,
                permanent: false,
            };
            let pplan = Plan {
                permanent: true,
                ..tplan.clone()
            };
            let t = run_cell(src, Some(tplan.clone()));
            let p = run_cell(src, Some(pplan.clone()));
            let (tc, tt) = classify(src, &tplan, false, &t, &base);
            let (pc, pt) = classify(src, &pplan, !f.benign(), &p, &base);
            cells.push((src.name, stage.class, f, false));
            cells.push((src.name, stage.class, f, true));
            row.push_str(&format!("{}:{}/{} ", f.short(), tc, pc));
            if !transient_ok(&tc, f, stage.class) {
                bad.push(format!(
                    "{} T {} {} {tc} :: {tt}",
                    src.name,
                    stage.name,
                    f.short()
                ));
            }
            if !permanent_ok(&pc) {
                bad.push(format!(
                    "{} P {} {} {pc} :: {pt}",
                    src.name,
                    stage.name,
                    f.short()
                ));
            }
        }
        table.push(format!("[{}] {row}", stage.name));
    }
    for line in &table {
        println!("{} {line}", src.name);
    }
    (bad, cells)
}

pub fn transient_ok(label: &str, fault: F, class: Class) -> bool {
    match fault.specified_fatal(class) {
        Some(kind) => label == format!("FATAL:{kind}"),
        None => label == "OK" || (label == "AUTH" && fault == F::S401),
    }
}

pub fn permanent_ok(label: &str) -> bool {
    matches!(label, "OK" | "RETRY")
}

pub fn distinct_urls(log: &[Logged]) -> Vec<String> {
    let mut seen = BTreeSet::new();
    log.iter()
        .map(|l| l.url.clone())
        .filter(|u| seen.insert(u.clone()))
        .collect()
}
