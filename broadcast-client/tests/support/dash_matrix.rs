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

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum F {
    S500,
    S503,
    S429,
    S429Huge,
    S404,
    S401,
    S403,
    ConnFail,
    ResetMid,
    ShortBody,
    TruncChunk,
    Stall,
    NoResp,
    Garbage,
    Html,
    Empty,
    Huge,
    Late,
    DupResp,
    Unknown,
    ClosedTwice,
    RedirLoop,
}

pub const ALL: [F; 22] = [
    F::S500,
    F::S503,
    F::S429,
    F::S429Huge,
    F::S404,
    F::S401,
    F::S403,
    F::ConnFail,
    F::ResetMid,
    F::ShortBody,
    F::TruncChunk,
    F::Stall,
    F::NoResp,
    F::Garbage,
    F::Html,
    F::Empty,
    F::Huge,
    F::Late,
    F::DupResp,
    F::Unknown,
    F::ClosedTwice,
    F::RedirLoop,
];

impl F {
    pub fn short(self) -> &'static str {
        match self {
            F::S500 => "500",
            F::S503 => "503",
            F::S429 => "429",
            F::S429Huge => "429H",
            F::S404 => "404",
            F::S401 => "401",
            F::S403 => "403",
            F::ConnFail => "conn",
            F::ResetMid => "rst",
            F::ShortBody => "shrt",
            F::TruncChunk => "trnc",
            F::Stall => "stall",
            F::NoResp => "nores",
            F::Garbage => "garb",
            F::Html => "html",
            F::Empty => "empty",
            F::Huge => "huge",
            F::Late => "late",
            F::DupResp => "dup",
            F::Unknown => "unk",
            F::ClosedTwice => "cls2",
            F::RedirLoop => "loop",
        }
    }

    pub fn benign(self) -> bool {
        matches!(self, F::DupResp | F::Unknown | F::ClosedTwice)
    }
}

#[derive(Clone, Debug)]
pub enum Matcher {
    Url(String),
    MpdAll,
    MpdRefresh,
}

#[derive(Clone, Debug)]
pub struct Stage {
    pub name: String,
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

pub struct Faulty {
    pub inner: Box<dyn Backend>,
    pub plan: Option<Plan>,
    pub limits: Limits,
    mpd_seen: usize,
    faulted: Rc<RefCell<Vec<(String, u64, bool)>>>,
}

impl Faulty {
    pub fn new(
        inner: Box<dyn Backend>,
        plan: Option<Plan>,
        limits: Limits,
        faulted: Rc<RefCell<Vec<(String, u64, bool)>>>,
    ) -> Self {
        Self {
            inner,
            plan,
            limits,
            mpd_seen: 0,
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

fn apply(f: F, good: Answer, req: &Logged, limit: usize) -> Answer {
    let body = bytes_of(&good).cloned().unwrap_or_default();
    let half = body.len() / 2;
    match f {
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
    }
}

impl Backend for Faulty {
    fn serve(&mut self, at_utc_us: i64, req: &Logged) -> Answer {
        let good = self.inner.serve(at_utc_us, req);
        let Some(plan) = self.plan.clone() else {
            return good;
        };
        let is_mpd = req.is_mpd();
        let index = if is_mpd {
            let i = self.mpd_seen;
            self.mpd_seen += 1;
            i
        } else {
            0
        };
        let matched = match &plan.stage.matcher {
            Matcher::Url(u) => req.url == *u,
            Matcher::MpdAll => is_mpd,
            Matcher::MpdRefresh => is_mpd && index >= 1,
        };
        if !matched || (!plan.permanent && !self.faulted.borrow().is_empty()) {
            return good;
        }
        self.faulted.borrow_mut().push((
            req.url.clone(),
            req.send_elapsed_us,
            !plan.fault.benign(),
        ));
        let limit = if is_mpd {
            self.limits.manifest
        } else {
            self.limits.segment
        };
        apply(plan.fault, good, req, limit)
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

/// The segments a live run fetched successfully, per Representation: each at most once, and
/// (unless `allow_gaps`) with no gap in the numbering or time sequence.
pub fn served_segments_ok(
    log: &[Logged],
    faulted: &[(String, u64, bool)],
    allow_gaps: bool,
) -> Result<(), String> {
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
    for (rep, mut v) in by_rep {
        let total = v.len();
        v.sort_unstable();
        v.dedup();
        if v.len() != total {
            return Err(format!("{rep}: a segment fetched twice"));
        }
        if allow_gaps {
            continue;
        }
        let step = v.windows(2).map(|w| w[1] - w[0]).min().unwrap_or(1);
        if v.windows(2).any(|w| w[1] - w[0] != step) {
            return Err(format!("{rep}: gap in {v:?}"));
        }
    }
    Ok(())
}

fn is_disc(e: &Event) -> bool {
    matches!(e, Event::Discontinuity)
}

pub fn classify(
    src: &Src,
    plan: &Plan,
    perm: bool,
    o: &Out,
    base: &[(u32, String, usize)],
) -> (String, String) {
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
    let busy =
        attempts_of(plan, &o.log) > MIN_BACKOFF_BOUND_DIVISOR_FREE + src.horizon_us / (2 * SEC);
    if src.live {
        if o.loop_cap {
            return ("LOOP".into(), tail);
        }
        if let Err(why) = served_segments_ok(&o.log, &o.faulted, perm || plan.fault == F::S429Huge)
        {
            return ("GAPDUP".into(), format!("{tail} {why}"));
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

const MIN_BACKOFF_BOUND_DIVISOR_FREE: u64 = 4;
const NEW_ATTEMPT_GAP_US: u64 = 500_000;

/// How many separate attempts (requests more than half a second after the previous one to the
/// same target, so a redirect chain is one attempt) the session made at the faulted target.
pub fn attempts_of(plan: &Plan, log: &[Logged]) -> u64 {
    let mut last: Option<u64> = None;
    let mut n = 0;
    for l in log {
        let hit = match &plan.stage.matcher {
            Matcher::Url(u) => l.url == *u,
            Matcher::MpdAll | Matcher::MpdRefresh => l.is_mpd(),
        };
        if !hit {
            continue;
        }
        if last.is_none_or(|t| l.send_elapsed_us.saturating_sub(t) >= NEW_ATTEMPT_GAP_US) {
            n += 1;
        }
        last = Some(l.send_elapsed_us);
    }
    n
}

pub fn matrix(src: &Src) -> Vec<String> {
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
    let base = samples(&base_o.events);
    assert!(
        !base.is_empty(),
        "{}: the baseline produced no samples",
        src.name
    );
    let mut bad = Vec::new();
    let mut table = Vec::new();
    for stage in &stages {
        let mut row = String::new();
        for f in ALL {
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
            row.push_str(&format!("{}:{}/{} ", f.short(), tc, pc));
            if !transient_ok(&tc, f) {
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
    bad
}

pub fn transient_ok(label: &str, fault: F) -> bool {
    label == "OK" || (label == "AUTH" && fault == F::S401)
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
