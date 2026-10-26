//! The DESIGN 3.6 simulation: a `World` of true UTC, a host clock with skew,
//! drift and steps, and a fake origin that decides availability from the TRUE
//! time, driving a `DashClient` through its sans-IO API. No wall clock is read
//! anywhere; time is whatever the simulation says it is.
//!
//! Why the synthesised origin recycles four real segments: the committed live
//! captures (`fixtures/dash/live-number`, `live-timeline`) hold four segments
//! per track, and the livesim2 MPDs carry no media at all. A 24-hour or
//! 6-hour run needs thousands of segments, so [`Synth`] rewrites only the MPD
//! (availability, window, numbering) and answers every segment request with
//! one of the four REAL fMP4 segments (and the real init). The client never
//! sees a hand-made byte.

#![allow(dead_code)]

use std::collections::BTreeMap;

use dash_runtime::client::{
    Action, DashClient, DashConfig, Error, Failure, FailureClass, Output, PullLimits, ResourceId,
    ResponseMeta, UtcMethod, UtcResponse,
};

pub const MICROS: i64 = 1_000_000;
pub const MS: i64 = 1_000;

/// True time, and the host's view of it.
#[derive(Debug, Clone, Copy)]
pub struct World {
    pub start_utc_us: i64,
    pub elapsed_us: u64,
    pub host_skew_us: i64,
    pub host_drift_ppm: i64,
    pub host_step_us: i64,
}

impl World {
    pub fn new(start_utc_us: i64) -> Self {
        Self {
            start_utc_us,
            elapsed_us: 0,
            host_skew_us: 0,
            host_drift_ppm: 0,
            host_step_us: 0,
        }
    }

    pub fn true_utc_at(&self, elapsed_us: u64) -> i64 {
        self.start_utc_us + elapsed_us as i64
    }

    pub fn true_utc(&self) -> i64 {
        self.true_utc_at(self.elapsed_us)
    }

    pub fn mono_at(&self, elapsed_us: u64) -> u64 {
        let scaled = i128::from(elapsed_us) * (1_000_000 + i128::from(self.host_drift_ppm));
        (scaled / 1_000_000).max(0) as u64
    }

    pub fn mono(&self) -> u64 {
        self.mono_at(self.elapsed_us)
    }

    pub fn elapsed_for_mono(&self, mono_us: u64) -> u64 {
        let den = 1_000_000 + i128::from(self.host_drift_ppm);
        let n = i128::from(mono_us) * 1_000_000;
        ((n + den - 1) / den).max(0) as u64
    }

    pub fn host_unix(&self) -> u64 {
        (i128::from(self.start_utc_us)
            + i128::from(self.host_skew_us)
            + i128::from(self.host_step_us)
            + i128::from(self.mono()))
        .max(0) as u64
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Method {
    Get,
    Head,
}

#[derive(Debug, Clone)]
pub struct HttpReq {
    pub method: Method,
    pub url: String,
    pub range: Option<(u64, u64)>,
    pub if_none_match: Option<String>,
}

#[derive(Debug, Clone, Default)]
pub struct HttpResp {
    pub status: u16,
    pub body: Vec<u8>,
    pub date: Option<String>,
    pub etag: Option<String>,
    pub final_url: Option<String>,
}

impl HttpResp {
    pub fn ok(body: Vec<u8>) -> Self {
        Self {
            status: 200,
            body,
            ..Self::default()
        }
    }

    pub fn status(status: u16) -> Self {
        Self {
            status,
            body: b"error".to_vec(),
            ..Self::default()
        }
    }
}

pub enum Outcome {
    Resp(HttpResp),
    Fail(Failure),
    Hang,
}

pub trait Origin {
    fn serve(&mut self, at_utc_us: i64, req: &HttpReq) -> Outcome;
}

/// An origin the test keeps a handle to, to change it mid-run.
pub struct Shared<O: Origin>(pub std::rc::Rc<std::cell::RefCell<O>>);

impl<O: Origin> Origin for Shared<O> {
    fn serve(&mut self, at_utc_us: i64, req: &HttpReq) -> Outcome {
        self.0.borrow_mut().serve(at_utc_us, req)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Kind {
    Mpd,
    Utc,
    Init,
    Index,
    Segment,
}

#[derive(Debug, Clone)]
pub struct Logged {
    pub kind: Kind,
    pub url: String,
    pub method: Method,
    pub send_utc_us: i64,
    pub send_mono_us: u64,
    pub status: Option<u16>,
    pub id: Option<ResourceId>,
}

pub type FaultFn = Box<dyn FnMut(&HttpReq, &Kind, i64) -> Option<Outcome>>;

enum Delivery {
    Response {
        action: Action,
        sent_mono: u64,
        received_mono: u64,
        outcome: Outcome,
    },
    RetryResource(ResourceId),
    RetryMpd,
}

struct Pending {
    at: u64,
    seq: u64,
    delivery: Delivery,
}

pub struct Sim {
    pub client: DashClient,
    pub world: World,
    origin: Box<dyn Origin>,
    pending: Vec<Pending>,
    seq: u64,
    pub log: Vec<Logged>,
    pub outputs: Vec<Output>,
    pub errors: Vec<Error>,
    pub classes: Vec<FailureClass>,
    pub rtt_us: u64,
    pub rtt_utc_us: Option<u64>,
    pub fault: Option<FaultFn>,
    pub host_clock_calls: Vec<u64>,
    pub send_headers: bool,
    pub stalls: u64,
    pub max_stall_run: u64,
    pub steps: u64,
    pub trace_stall: bool,
}

fn backoff_us(attempt: u32) -> u64 {
    let shift = attempt.saturating_sub(1).min(6);
    (250_000u64 << shift).min(4_000_000)
}

impl Sim {
    pub fn new(
        url: &str,
        dash: DashConfig,
        limits: PullLimits,
        world: World,
        origin: Box<dyn Origin>,
    ) -> Self {
        Self {
            client: DashClient::new(url, dash, limits),
            world,
            origin,
            pending: Vec::new(),
            seq: 0,
            log: Vec::new(),
            outputs: Vec::new(),
            errors: Vec::new(),
            classes: Vec::new(),
            rtt_us: 40_000,
            rtt_utc_us: None,
            fault: None,
            host_clock_calls: Vec::new(),
            send_headers: true,
            stalls: 0,
            max_stall_run: 0,
            steps: 0,
            trace_stall: false,
        }
    }

    pub fn give_host_clock(&mut self) {
        let now = self.world.mono();
        let unix = self.world.host_unix();
        self.client.set_wall_clock(now, unix);
        self.host_clock_calls.push(now);
    }

    fn push(&mut self, at: u64, delivery: Delivery) {
        self.seq += 1;
        self.pending.push(Pending {
            at,
            seq: self.seq,
            delivery,
        });
    }

    fn dispatch(&mut self, action: Action) {
        let sent_elapsed = self.world.elapsed_us;
        let sent_mono = self.world.mono();
        let is_utc = matches!(action, Action::FetchUtcTiming { .. });
        let rtt = if is_utc {
            self.rtt_utc_us.unwrap_or(self.rtt_us)
        } else {
            self.rtt_us
        };
        let origin_elapsed = sent_elapsed + rtt / 2;
        let arrive = sent_elapsed + rtt;
        let (kind, method, url) = match &action {
            Action::FetchMpd { url, .. } => (Kind::Mpd, Method::Get, url.clone()),
            Action::FetchUtcTiming { url, method, .. } => (
                Kind::Utc,
                match method {
                    UtcMethod::Head => Method::Head,
                    _ => Method::Get,
                },
                url.clone(),
            ),
            Action::FetchInit { url, .. } => (Kind::Init, Method::Get, url.clone()),
            Action::FetchIndex { url, .. } => (Kind::Index, Method::Get, url.clone()),
            Action::FetchSegment { url, .. } => (Kind::Segment, Method::Get, url.clone()),
            _ => return,
        };
        let if_none_match = match &action {
            Action::FetchMpd { validators, .. } if self.send_headers => validators.etag.clone(),
            _ => None,
        };
        let req = HttpReq {
            method,
            url: url.clone(),
            range: action.byte_range(),
            if_none_match,
        };
        let at_utc = self.world.true_utc_at(origin_elapsed);
        let mut outcome = None;
        if let Some(f) = self.fault.as_mut() {
            outcome = f(&req, &kind, at_utc);
        }
        let outcome = outcome.unwrap_or_else(|| self.origin.serve(at_utc, &req));
        let status = match &outcome {
            Outcome::Resp(r) => Some(r.status),
            _ => None,
        };
        self.log.push(Logged {
            kind,
            url,
            method,
            send_utc_us: self.world.true_utc_at(sent_elapsed),
            send_mono_us: sent_mono,
            status,
            id: action.id(),
        });
        let received_mono = self.world.mono_at(arrive);
        if matches!(outcome, Outcome::Hang) {
            return;
        }
        self.push(
            arrive,
            Delivery::Response {
                action,
                sent_mono,
                received_mono,
                outcome,
            },
        );
    }

    fn drain_outputs(&mut self) {
        while let Some(o) = self.client.next_output() {
            self.outputs.push(o);
        }
    }

    fn poll_all(&mut self) {
        loop {
            let now = self.world.mono();
            let Some(a) = self.client.poll(now) else {
                break;
            };
            self.dispatch(a);
        }
        self.drain_outputs();
    }

    fn deliver(&mut self, d: Delivery) {
        let now = self.world.mono();
        match d {
            Delivery::RetryResource(id) => {
                self.client.retry(now, id);
            }
            Delivery::RetryMpd => self.client.retry_mpd(now),
            Delivery::Response {
                action,
                sent_mono,
                received_mono,
                outcome,
            } => self.deliver_response(now, action, sent_mono, received_mono, outcome),
        }
        self.drain_outputs();
    }

    fn after_class(&mut self, now: u64, id: ResourceId, class: FailureClass) {
        self.classes.push(class);
        if let FailureClass::Retry { attempt, .. } = class {
            let at = self.world.elapsed_us + backoff_us(attempt);
            let _ = now;
            self.push(at, Delivery::RetryResource(id));
        }
    }

    fn deliver_response(
        &mut self,
        now: u64,
        action: Action,
        sent_mono: u64,
        received_mono: u64,
        outcome: Outcome,
    ) {
        let resp = match outcome {
            Outcome::Resp(r) => Ok(r),
            Outcome::Fail(f) => Err(f),
            Outcome::Hang => return,
        };
        match action {
            Action::FetchMpd { .. } => match resp {
                Ok(r) if r.status == 304 => {
                    let meta = ResponseMeta::new(sent_mono, received_mono);
                    if let Err(e) = self.client.on_mpd_unchanged(now, &meta) {
                        self.errors.push(e);
                    }
                }
                Ok(r) if (200..300).contains(&r.status) => {
                    let mut meta = ResponseMeta::new(sent_mono, received_mono);
                    if let Some(d) = r.date.as_deref() {
                        meta = meta.with_date(d);
                    }
                    if let Some(e) = r.etag.as_deref() {
                        meta = meta.with_etag(e);
                    }
                    if let Some(f) = r.final_url.as_deref() {
                        meta = meta.with_final_url(f);
                    }
                    if let Err(e) = self.client.on_mpd_with(now, &r.body, &meta) {
                        if matches!(e, Error::Mpd(_)) {
                            let at = self.world.elapsed_us + backoff_us(1);
                            self.push(at, Delivery::RetryMpd);
                        }
                        self.errors.push(e);
                    }
                }
                other => {
                    let failure = match other {
                        Ok(r) => Failure::Status(r.status),
                        Err(f) => f,
                    };
                    let class = self.client.on_mpd_failure(now, failure);
                    let attempt = match class {
                        FailureClass::Retry { attempt, .. } => attempt,
                        _ => 1,
                    };
                    self.classes.push(class);
                    let at = self.world.elapsed_us + backoff_us(attempt);
                    self.push(at, Delivery::RetryMpd);
                }
            },
            Action::FetchUtcTiming { id, .. } => match resp {
                Ok(r) if (200..300).contains(&r.status) => {
                    let mut ur = UtcResponse::new(sent_mono, received_mono, &r.body);
                    if let Some(d) = r.date.as_deref() {
                        ur = ur.with_date(d);
                    }
                    if let Err(e) = self.client.on_utc_response(now, id, &ur) {
                        self.errors.push(e);
                    }
                }
                other => {
                    let failure = match other {
                        Ok(r) => Failure::Status(r.status),
                        Err(f) => f,
                    };
                    if let Err(e) = self.client.on_utc_failure(now, id, failure) {
                        self.errors.push(e);
                    }
                }
            },
            Action::FetchInit { id, .. }
            | Action::FetchIndex { id, .. }
            | Action::FetchSegment { id, .. } => {
                let result = match resp {
                    Ok(r) if (200..300).contains(&r.status) => Ok(r.body),
                    Ok(r) => Err(Failure::Status(r.status)),
                    Err(f) => Err(f),
                };
                match result {
                    Err(f) => match self.client.on_failure(now, id, f) {
                        Ok(class) => self.after_class(now, id, class),
                        Err(e) => self.errors.push(e),
                    },
                    Ok(body) => {
                        let r = match id {
                            ResourceId::Init { .. } => self.client.on_init(now, id, &body),
                            ResourceId::Index { .. } => self.client.on_index(now, id, &body),
                            _ => self.client.on_segment(now, id, &body),
                        };
                        if let Err(e) = r {
                            if let Error::Rejected { id, class, .. } = &e {
                                self.after_class(now, *id, *class);
                            }
                            self.errors.push(e);
                        }
                    }
                }
            }
            _ => {}
        }
    }

    /// Advance simulated time to `end_elapsed_us`, driving the client.
    pub fn run_until(&mut self, end_elapsed_us: u64) {
        let mut stall_run = 0u64;
        loop {
            self.steps += 1;
            self.poll_all();
            if self.client.presentation_ended() {
                break;
            }
            let mut next = end_elapsed_us;
            for p in &self.pending {
                next = next.min(p.at);
            }
            if let Some(w) = self.client.next_wake() {
                next = next.min(self.world.elapsed_for_mono(w));
            }
            if next <= self.world.elapsed_us {
                if next >= end_elapsed_us {
                    break;
                }
                let due = self.pending.iter().any(|p| p.at <= self.world.elapsed_us);
                if !due {
                    stall_run += 1;
                    self.stalls += 1;
                    self.max_stall_run = self.max_stall_run.max(stall_run);
                    if self.trace_stall && stall_run < 4 {
                        eprintln!(
                            "stall at elapsed {} wake {:?} mono {}",
                            self.world.elapsed_us,
                            self.client.next_wake(),
                            self.world.mono()
                        );
                    }
                    next = self.world.elapsed_us + 1_000;
                } else {
                    stall_run = 0;
                }
            } else {
                stall_run = 0;
            }
            self.world.elapsed_us = next.min(end_elapsed_us);
            let mut due: Vec<Pending> = Vec::new();
            let mut rest = Vec::new();
            for p in self.pending.drain(..) {
                if p.at <= self.world.elapsed_us {
                    due.push(p);
                } else {
                    rest.push(p);
                }
            }
            self.pending = rest;
            due.sort_by_key(|p| (p.at, p.seq));
            for p in due {
                self.deliver(p.delivery);
            }
            if self.world.elapsed_us >= end_elapsed_us {
                self.poll_all();
                break;
            }
        }
    }

    pub fn run_for(&mut self, dur_us: u64) {
        let end = self.world.elapsed_us + dur_us;
        self.run_until(end);
    }

    pub fn samples_per_track(&self) -> BTreeMap<u32, usize> {
        let mut m = BTreeMap::new();
        for o in &self.outputs {
            if let Output::Samples { track_id, samples } = o {
                *m.entry(track_id.value()).or_insert(0) += samples.len();
            }
        }
        m
    }

    pub fn count(&self, pred: impl Fn(&Output) -> bool) -> usize {
        self.outputs.iter().filter(|o| pred(o)).count()
    }

    pub fn requests(&self, kind: Kind) -> Vec<&Logged> {
        self.log.iter().filter(|l| l.kind == kind).collect()
    }

    pub fn fatal(&self) -> Option<&dash_runtime::client::FatalKind> {
        self.outputs.iter().find_map(|o| match o {
            Output::Fatal(k) => Some(k),
            _ => None,
        })
    }
}

pub fn http_date(us: i64) -> String {
    let ts = jiff::Timestamp::from_microsecond(us).expect("timestamp in range");
    ts.strftime("%a, %d %b %Y %H:%M:%S GMT").to_string()
}

pub fn xs_datetime_ms(us: i64) -> String {
    let ms = us.div_euclid(1_000);
    let ts = jiff::Timestamp::from_millisecond(ms).expect("timestamp in range");
    ts.strftime("%Y-%m-%dT%H:%M:%S%.3fZ").to_string()
}

pub fn xs_datetime_s(us: i64) -> String {
    let ts = jiff::Timestamp::from_second(us.div_euclid(MICROS)).expect("timestamp in range");
    ts.strftime("%Y-%m-%dT%H:%M:%SZ").to_string()
}
