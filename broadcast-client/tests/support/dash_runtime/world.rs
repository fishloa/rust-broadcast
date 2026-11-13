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

pub const MICROS: i64 = 1_000_000;

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
