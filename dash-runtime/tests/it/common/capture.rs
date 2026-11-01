//! Origins that serve REAL committed captures: the ffmpeg live captures
//! (`fixtures/dash/live-number`, `live-timeline`: MPD and segments exactly as
//! ffmpeg wrote them) and the two real DASH-IF livesim2 MPD refreshes
//! (`fixtures/dash/livesim2/*_a.mpd`/`_b.mpd`, media recycled from the ffmpeg
//! capture because livesim2 serves a generator, not files).

#![allow(dead_code)]

use std::collections::BTreeMap;

use super::synth::Pool;
use super::world::{HttpReq, HttpResp, Origin, Outcome, xs_datetime_ms};

/// Serves the files of one fixture directory under `http://dash.test/<mount>/`,
/// each only from its own availability instant.
pub struct CaptureOrigin {
    pub mount: String,
    pub dir: String,
    pub avail_utc_us: BTreeMap<String, i64>,
    pub early_status: u16,
    pub mpd_requests: Vec<i64>,
}

impl CaptureOrigin {
    pub fn new(mount: &str, dir: &str) -> Self {
        Self {
            mount: mount.to_owned(),
            dir: dir.to_owned(),
            avail_utc_us: BTreeMap::new(),
            early_status: 404,
            mpd_requests: Vec::new(),
        }
    }

    pub fn url(&self, file: &str) -> String {
        format!("http://dash.test/{}/{file}", self.mount)
    }

    pub fn available(mut self, file: &str, at_utc_us: i64) -> Self {
        self.avail_utc_us.insert(file.to_owned(), at_utc_us);
        self
    }
}

impl Origin for CaptureOrigin {
    fn serve(&mut self, at_utc_us: i64, req: &HttpReq) -> Outcome {
        if req.url.starts_with("http://utc.test/") {
            return Outcome::Resp(HttpResp::ok(xs_datetime_ms(at_utc_us).into_bytes()));
        }
        let prefix = format!("http://dash.test/{}/", self.mount);
        let Some(file) = req.url.strip_prefix(&prefix) else {
            return Outcome::Resp(HttpResp::status(404));
        };
        if file.ends_with(".mpd") {
            self.mpd_requests.push(at_utc_us);
        } else if let Some(&a) = self.avail_utc_us.get(file)
            && at_utc_us < a
        {
            return Outcome::Resp(HttpResp::status(self.early_status));
        }
        let path = format!("dash/{}/{file}", self.dir);
        if !test_fixtures::path(&path).is_file() {
            return Outcome::Resp(HttpResp::status(404));
        }
        Outcome::Resp(HttpResp::ok(test_fixtures::read(&path)))
    }
}

/// The two real livesim2 MPDs: `a` until `switch_utc_us`, `b` after it.
pub struct Livesim2Pair {
    pub a: String,
    pub b: String,
    pub switch_utc_us: i64,
    pub avail_utc_us: BTreeMap<(String, u64), i64>,
    pub pool: Pool,
    pub segment_requests: Vec<(String, u64, i64, u16)>,
}

impl Livesim2Pair {
    pub fn url() -> &'static str {
        "http://dash.test/ls2/manifest.mpd"
    }
}

impl Origin for Livesim2Pair {
    fn serve(&mut self, at_utc_us: i64, req: &HttpReq) -> Outcome {
        if req.url.starts_with("https://time.akamai.com/") {
            return Outcome::Resp(HttpResp::ok(xs_datetime_ms(at_utc_us).into_bytes()));
        }
        let Some(path) = req.url.strip_prefix("http://dash.test/ls2/") else {
            return Outcome::Resp(HttpResp::status(404));
        };
        if path == "manifest.mpd" {
            let text = if at_utc_us >= self.switch_utc_us {
                &self.b
            } else {
                &self.a
            };
            return Outcome::Resp(HttpResp::ok(text.clone().into_bytes()));
        }
        let Some((rep, file)) = path.split_once('/') else {
            return Outcome::Resp(HttpResp::status(404));
        };
        if file == "init.mp4" {
            let b = if rep == "V300" {
                &self.pool.video_init
            } else {
                &self.pool.audio_init
            };
            return Outcome::Resp(HttpResp::ok(b.clone()));
        }
        let Some(t) = file
            .strip_suffix(".m4s")
            .and_then(|s| s.parse::<u64>().ok())
        else {
            return Outcome::Resp(HttpResp::status(404));
        };
        let Some(&avail) = self.avail_utc_us.get(&(rep.to_owned(), t)) else {
            self.segment_requests
                .push((rep.to_owned(), t, at_utc_us, 404));
            return Outcome::Resp(HttpResp::status(404));
        };
        if at_utc_us < avail {
            self.segment_requests
                .push((rep.to_owned(), t, at_utc_us, 425));
            return Outcome::Resp(HttpResp::status(425));
        }
        self.segment_requests
            .push((rep.to_owned(), t, at_utc_us, 200));
        let pool = if rep == "V300" {
            &self.pool.video
        } else {
            &self.pool.audio
        };
        Outcome::Resp(HttpResp::ok(pool[(t as usize / 7) % 4].clone()))
    }
}
