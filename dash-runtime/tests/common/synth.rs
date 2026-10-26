//! The synthesised live origin of the `World` simulation: an MPD generated
//! for any true time, with segments that exist exactly when 23009-1
//! 5.3.9.5.3 says (computed here from first principles, independently of the
//! crate), answered with the four REAL fMP4 segments of
//! `fixtures/dash/live-number` recycled in turn. See `world.rs` for why.

#![allow(dead_code)]

use std::collections::BTreeMap;

use super::world::{HttpReq, HttpResp, MICROS, Method, Origin, Outcome, http_date, xs_datetime_ms, xs_datetime_s};
use dash_runtime::client::Failure;

pub const MPD_URL: &str = "http://dash.test/live/manifest.mpd";
pub const UTC_HOST: &str = "utc.test";

pub const VIDEO_TIMESCALE: u64 = 90_000;
pub const AUDIO_TIMESCALE: u64 = 44_100;
pub const POOL: usize = 4;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Addressing {
    Number,
    TimelineNumber,
    TimelineTime,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum UtcMode {
    Ok,
    Garbage,
    Empty,
    Huge,
    Hang,
    Fail,
}

#[derive(Clone, Debug)]
pub struct UtcServer {
    pub skew_us: i64,
    pub mode: UtcMode,
    pub head_status: Option<u16>,
    pub date_skew_us: i64,
    pub by_path: BTreeMap<String, UtcMode>,
}

impl Default for UtcServer {
    fn default() -> Self {
        Self {
            skew_us: 0,
            mode: UtcMode::Ok,
            head_status: None,
            date_skew_us: 0,
            by_path: BTreeMap::new(),
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub struct Geo {
    pub ast_us: i64,
    pub seg_us: i64,
    pub start_number: u64,
    pub t0_ticks: u64,
    pub addressing: Addressing,
    pub tsb_us: Option<i64>,
}

impl Geo {
    pub fn ticks(&self, timescale: u64) -> u64 {
        (self.seg_us as u64) * timescale / 1_000_000
    }

    pub fn avail_from_us(&self, idx: i64) -> i64 {
        self.ast_us + (idx + 1) * self.seg_us
    }

    pub fn avail_until_us(&self, idx: i64) -> Option<i64> {
        self.tsb_us.map(|t| self.ast_us + (idx + 2) * self.seg_us + t)
    }

    pub fn complete_count(&self, utc_us: i64) -> i64 {
        (utc_us - self.ast_us).div_euclid(self.seg_us)
    }

    pub fn index_of(&self, rep: &str, n: u64) -> Option<i64> {
        match self.addressing {
            Addressing::Number | Addressing::TimelineNumber => {
                n.checked_sub(self.start_number).map(|v| v as i64)
            }
            Addressing::TimelineTime => {
                let ts = if rep == "V" { VIDEO_TIMESCALE } else { AUDIO_TIMESCALE };
                let d = self.ticks(ts);
                n.checked_sub(self.t0_ticks).map(|v| (v / d) as i64)
            }
        }
    }
}

pub struct Pool {
    pub video: Vec<Vec<u8>>,
    pub audio: Vec<Vec<u8>>,
    pub video_init: Vec<u8>,
    pub audio_init: Vec<u8>,
}

impl Pool {
    pub fn load() -> Self {
        let read = |n: &str| test_fixtures::read(&format!("dash/live-number/{n}"));
        Self {
            video: (5..9).map(|i| read(&format!("seg-0-{i:03}.m4s"))).collect(),
            audio: (5..9).map(|i| read(&format!("seg-1-{i:03}.m4s"))).collect(),
            video_init: read("init-0.m4s"),
            audio_init: read("init-1.m4s"),
        }
    }
}

pub struct Synth {
    pub ast_us: i64,
    pub seg_us: i64,
    pub tsb_us: Option<i64>,
    pub mup_us: Option<i64>,
    pub spd_us: Option<i64>,
    pub addressing: Addressing,
    pub window: u64,
    pub utc_elements: Vec<String>,
    pub early_status: u16,
    pub start_number: u64,
    pub t0_ticks: u64,
    pub utc: UtcServer,
    pub send_date: bool,
    pub send_etag: bool,
    pub location: Option<String>,
    pub redirect_to: Option<String>,
    pub static_from_utc_us: Option<i64>,
    pub with_audio: bool,
    pub mpd_override: Option<Box<dyn FnMut(i64) -> Option<String>>>,
    pub utc_direct: bool,
    pub pool: Pool,
    pub mpd_requests: Vec<i64>,
    pub if_none_match_hits: u64,
    pub segment_bytes_overrides: BTreeMap<String, Vec<u8>>,
    pub min_update_attr: bool,
}

impl Synth {
    pub fn new(ast_us: i64, addressing: Addressing) -> Self {
        Self {
            ast_us,
            seg_us: 2 * MICROS,
            tsb_us: Some(30 * MICROS),
            mup_us: Some(2 * MICROS),
            spd_us: Some(6 * MICROS),
            addressing,
            window: 8,
            utc_elements: Vec::new(),
            early_status: 404,
            start_number: 1,
            t0_ticks: 0,
            utc: UtcServer::default(),
            send_date: false,
            send_etag: false,
            location: None,
            redirect_to: None,
            static_from_utc_us: None,
            with_audio: true,
            mpd_override: None,
            utc_direct: false,
            pool: Pool::load(),
            mpd_requests: Vec::new(),
            if_none_match_hits: 0,
            segment_bytes_overrides: BTreeMap::new(),
            min_update_attr: true,
        }
    }

    pub fn with_utc(mut self, scheme: &str, value: &str) -> Self {
        self.utc_elements.push(format!(
            "<UTCTiming schemeIdUri=\"urn:mpeg:dash:utc:{scheme}:2014\" value=\"{value}\"/>"
        ));
        self
    }

    pub fn url(&self) -> &'static str {
        MPD_URL
    }

    pub fn geo(&self) -> Geo {
        Geo {
            ast_us: self.ast_us,
            seg_us: self.seg_us,
            start_number: self.start_number,
            t0_ticks: self.t0_ticks,
            addressing: self.addressing,
            tsb_us: self.tsb_us,
        }
    }

    pub fn complete_count(&self, utc_us: i64) -> i64 {
        self.geo().complete_count(utc_us)
    }

    pub fn avail_from_us(&self, idx: i64) -> i64 {
        self.geo().avail_from_us(idx)
    }

    pub fn avail_until_us(&self, idx: i64) -> Option<i64> {
        self.geo().avail_until_us(idx)
    }

    pub fn ticks(&self, timescale: u64) -> u64 {
        self.geo().ticks(timescale)
    }

    fn media_attr(&self) -> &'static str {
        match self.addressing {
            Addressing::TimelineTime => "seg/$RepresentationID$/$Time$.m4s",
            _ => "seg/$RepresentationID$/$Number$.m4s",
        }
    }

    fn template(&self, rep_ticks: u64, timescale: u64, first_idx: i64, count: i64) -> String {
        let media = self.media_attr();
        match self.addressing {
            Addressing::Number => format!(
                "<SegmentTemplate timescale=\"{timescale}\" duration=\"{rep_ticks}\" startNumber=\"{}\" media=\"{media}\" initialization=\"seg/$RepresentationID$/init.mp4\"/>",
                self.start_number
            ),
            _ => {
                let t = self.t0_ticks + first_idx as u64 * rep_ticks;
                format!(
                    "<SegmentTemplate timescale=\"{timescale}\" startNumber=\"{}\" media=\"{media}\" initialization=\"seg/$RepresentationID$/init.mp4\"><SegmentTimeline><S t=\"{t}\" d=\"{rep_ticks}\" r=\"{}\"/></SegmentTimeline></SegmentTemplate>",
                    self.start_number + first_idx as u64,
                    count - 1
                )
            }
        }
    }

    pub fn mpd_at(&mut self, at_utc: i64) -> String {
        if let Some(f) = self.mpd_override.as_mut()
            && let Some(t) = f(at_utc)
        {
            return t;
        }
        let c = self.complete_count(at_utc).max(1);
        let first_idx = (c - self.window as i64).max(0);
        let count = c - first_idx;
        let is_static = self.static_from_utc_us.is_some_and(|s| at_utc >= s);
        let mut attrs = String::new();
        if is_static {
            let total = self
                .static_from_utc_us
                .map_or(c, |s| self.complete_count(s).max(1));
            attrs.push_str(&format!(
                " type=\"static\" mediaPresentationDuration=\"PT{}S\"",
                total * self.seg_us / MICROS
            ));
        } else {
            attrs.push_str(&format!(
                " type=\"dynamic\" availabilityStartTime=\"{}\" publishTime=\"{}\"",
                xs_datetime_ms(self.ast_us),
                xs_datetime_ms(at_utc)
            ));
            if let Some(m) = self.mup_us.filter(|_| self.min_update_attr) {
                attrs.push_str(&format!(" minimumUpdatePeriod=\"PT{}S\"", m as f64 / 1e6));
            }
            if let Some(t) = self.tsb_us {
                attrs.push_str(&format!(" timeShiftBufferDepth=\"PT{}S\"", t as f64 / 1e6));
            }
            if let Some(s) = self.spd_us {
                attrs.push_str(&format!(
                    " suggestedPresentationDelay=\"PT{}S\"",
                    s as f64 / 1e6
                ));
            }
        }
        let vt = self.template(self.ticks(VIDEO_TIMESCALE), VIDEO_TIMESCALE, first_idx, count);
        let at = self.template(self.ticks(AUDIO_TIMESCALE), AUDIO_TIMESCALE, first_idx, count);
        let audio = if self.with_audio {
            format!(
                "<AdaptationSet id=\"1\" contentType=\"audio\" mimeType=\"audio/mp4\" segmentAlignment=\"true\" startWithSAP=\"1\"><Representation id=\"A\" codecs=\"mp4a.40.2\" bandwidth=\"98701\" audioSamplingRate=\"44100\">{at}</Representation></AdaptationSet>"
            )
        } else {
            String::new()
        };
        let location = self
            .location
            .as_ref()
            .map(|l| format!("<Location>{l}</Location>"))
            .unwrap_or_default();
        let mut utc = self.utc_elements.concat();
        if self.utc_direct {
            utc.push_str(&format!(
                "<UTCTiming schemeIdUri=\"urn:mpeg:dash:utc:direct:2014\" value=\"{}\"/>",
                xs_datetime_ms(at_utc + self.utc.skew_us)
            ));
        }
        format!(
            "<?xml version=\"1.0\"?><MPD xmlns=\"urn:mpeg:dash:schema:mpd:2011\" profiles=\"urn:mpeg:dash:profile:isoff-live:2011\" minBufferTime=\"PT2S\" maxSegmentDuration=\"PT{}S\"{attrs}>{location}<Period id=\"P0\" start=\"PT0S\"><AdaptationSet id=\"0\" contentType=\"video\" mimeType=\"video/mp4\" segmentAlignment=\"true\" startWithSAP=\"1\"><Representation id=\"V\" codecs=\"avc1.4d400d\" bandwidth=\"62440\" width=\"320\" height=\"240\">{vt}</Representation></AdaptationSet>{audio}</Period>{utc}</MPD>",
            self.seg_us / MICROS
        )
    }

    fn serve_utc(&mut self, at_utc: i64, req: &HttpReq) -> Outcome {
        let u = self.utc.clone();
        let path = req.url.rsplit('/').next().unwrap_or("").to_owned();
        let mode = u.by_path.get(&path).copied().unwrap_or(u.mode);
        match mode {
            UtcMode::Hang => return Outcome::Hang,
            UtcMode::Fail => return Outcome::Fail(Failure::Timeout),
            UtcMode::Garbage => return Outcome::Resp(HttpResp::ok(b"not a time at all".to_vec())),
            UtcMode::Empty => return Outcome::Resp(HttpResp::ok(Vec::new())),
            UtcMode::Huge => return Outcome::Resp(HttpResp::ok(vec![b'x'; 5000])),
            UtcMode::Ok => {}
        }
        let server_now = at_utc + u.skew_us;
        if req.url.ends_with("/head") {
            if req.method == Method::Head
                && let Some(s) = u.head_status
            {
                return Outcome::Resp(HttpResp::status(s));
            }
            let mut r = HttpResp::ok(Vec::new());
            r.date = Some(http_date(at_utc + u.date_skew_us + u.skew_us));
            return Outcome::Resp(r);
        }
        let body = if req.url.ends_with("/iso") {
            xs_datetime_s(server_now)
        } else {
            xs_datetime_ms(server_now)
        };
        Outcome::Resp(HttpResp::ok(body.into_bytes()))
    }

    fn rep_of(path: &str) -> Option<(&str, &str)> {
        let rest = path.strip_prefix("/live/seg/")?;
        rest.split_once('/')
    }

    pub fn index_of(&self, rep: &str, n: u64) -> Option<i64> {
        self.geo().index_of(rep, n)
    }

    fn serve_media(&mut self, at_utc: i64, req: &HttpReq) -> Outcome {
        let path = req.url.strip_prefix("http://dash.test").unwrap_or("");
        if path == "/live/manifest.mpd" {
            self.mpd_requests.push(at_utc);
            if let Some(to) = self.redirect_to.clone()
                && req.url != to
            {
                let body = self.mpd_at(at_utc).into_bytes();
                let mut r = HttpResp::ok(body);
                r.final_url = Some(to);
                return Outcome::Resp(r);
            }
            let body = self.mpd_at(at_utc).into_bytes();
            let mut r = HttpResp::ok(body.clone());
            if self.send_date {
                r.date = Some(http_date(at_utc));
            }
            if self.send_etag {
                let tag = format!("\"{:x}\"", fnv(&body));
                if req.if_none_match.as_deref() == Some(tag.as_str()) {
                    self.if_none_match_hits += 1;
                    return Outcome::Resp(HttpResp {
                        status: 304,
                        ..HttpResp::default()
                    });
                }
                r.etag = Some(tag);
            }
            return Outcome::Resp(r);
        }
        let Some((rep, file)) = Self::rep_of(path) else {
            return Outcome::Resp(HttpResp::status(404));
        };
        if file == "init.mp4" {
            let b = if rep == "V" {
                self.pool.video_init.clone()
            } else {
                self.pool.audio_init.clone()
            };
            return Outcome::Resp(HttpResp::ok(b));
        }
        let Some(n) = file
            .strip_suffix(".m4s")
            .and_then(|s| s.parse::<u64>().ok())
        else {
            return Outcome::Resp(HttpResp::status(404));
        };
        let Some(idx) = self.index_of(rep, n) else {
            return Outcome::Resp(HttpResp::status(404));
        };
        if at_utc < self.avail_from_us(idx) {
            return Outcome::Resp(HttpResp::status(self.early_status));
        }
        if self.avail_until_us(idx).is_some_and(|u| at_utc > u) {
            return Outcome::Resp(HttpResp::status(410));
        }
        if let Some(b) = self.segment_bytes_overrides.get(file) {
            return Outcome::Resp(HttpResp::ok(b.clone()));
        }
        let pool = if rep == "V" { &self.pool.video } else { &self.pool.audio };
        Outcome::Resp(HttpResp::ok(pool[(idx as usize) % POOL].clone()))
    }
}

pub fn fnv(bytes: &[u8]) -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325u64;
    for b in bytes {
        h ^= u64::from(*b);
        h = h.wrapping_mul(0x0100_0000_01b3);
    }
    h
}

impl Origin for Synth {
    fn serve(&mut self, at_utc_us: i64, req: &HttpReq) -> Outcome {
        if req.url.starts_with(&format!("http://{UTC_HOST}/")) {
            return self.serve_utc(at_utc_us, req);
        }
        self.serve_media(at_utc_us, req)
    }
}

/// `(rep name, number-or-time)` of a segment URL of the synthesised origin.
pub fn parse_segment_url(url: &str) -> Option<(String, u64)> {
    let path = url.strip_prefix("http://dash.test")?;
    let (rep, file) = Synth::rep_of(path)?;
    let n = file.strip_suffix(".m4s")?.parse().ok()?;
    Some((rep.to_owned(), n))
}
