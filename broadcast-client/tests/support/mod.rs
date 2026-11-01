#![allow(dead_code)]
pub mod dash_checks;
pub mod dash_common;
pub mod dash_matrix;
pub mod dash_sim;
pub mod fmp4_split;
pub mod hls_host;
pub mod renditions;
pub mod replay;
pub mod rtsp_host;
pub mod srt_peer;
pub mod srt_sim;
pub mod truth;
pub mod ts_host;
pub mod ts_pmt;
pub mod ts_probe;
pub mod ts_shift;
pub mod ts_split;
pub mod udp_host;
pub mod wire_host;

use broadcast_client::{Action, Event, Session};

pub fn read_tf1_headers() -> Option<(u16, Vec<(String, String)>)> {
    let path = test_fixtures::require("zenith-http/tf1.headers.txt", "support")?;
    let text = std::fs::read_to_string(path).unwrap();
    let status: u16 = text
        .lines()
        .next()
        .unwrap()
        .split_whitespace()
        .nth(1)
        .unwrap()
        .parse()
        .unwrap();
    let mut headers = Vec::new();
    for line in text.lines().skip(1) {
        if line.trim().is_empty() {
            continue;
        }
        if let Some((k, v)) = line.split_once(':') {
            headers.push((k.trim().to_string(), v.trim().to_string()));
        }
    }
    Some((status, headers))
}

pub fn read_tf1_bytes() -> Option<Vec<u8>> {
    test_fixtures::require_bytes("zenith-http/tf1-live.ts", "support")
}

pub fn read_missing_headers() -> Option<(u16, Vec<(String, String)>)> {
    let path = test_fixtures::require("zenith-http/missing.headers.txt", "support")?;
    let text = std::fs::read_to_string(path).unwrap();
    let status: u16 = text
        .lines()
        .next()
        .unwrap()
        .split_whitespace()
        .nth(1)
        .unwrap()
        .parse()
        .unwrap();
    let mut headers = Vec::new();
    for line in text.lines().skip(1) {
        if line.trim().is_empty() {
            continue;
        }
        if let Some((k, v)) = line.split_once(':') {
            headers.push((k.trim().to_string(), v.trim().to_string()));
        }
    }
    Some((status, headers))
}

pub fn feed_chunks(
    session: &mut Session,
    bytes: &[u8],
    chunk_size: usize,
    status: u16,
    resp_headers: &[(String, String)],
    now_us: u64,
) -> Vec<Event> {
    let mut conn = None;
    // initial request
    while let Some(action) = session.poll(now_us) {
        match action {
            Action::HttpRequest { conn: c, .. } => {
                conn = Some(c);
                session.on_response(c, status, resp_headers, now_us);
            }
            Action::Close { conn: c } => {
                session.on_closed(c, None, now_us);
            }
            Action::WakeAt(t) => {
                if t <= now_us {
                    // loop
                } else {
                    break;
                }
            }
            _ => {}
        }
    }
    let c = conn.unwrap();
    for chunk in bytes.chunks(chunk_size.max(1)) {
        session.on_data(c, chunk, now_us);
    }
    // collect
    let mut out = Vec::new();
    while let Some(e) = session.next_event() {
        out.push(e);
    }
    out
}

#[allow(unused_macros)]
macro_rules! skip_none {
    ($e:expr) => {
        match $e {
            Some(v) => v,
            None => return,
        }
    };
}

#[allow(unused_macros)]
macro_rules! tf1_bytes {
    () => {
        skip_none!($crate::support::read_tf1_bytes())
    };
}

#[allow(unused_macros)]
macro_rules! tf1_headers {
    () => {
        skip_none!($crate::support::read_tf1_headers())
    };
}

#[allow(unused_macros)]
macro_rules! missing_headers {
    () => {
        skip_none!($crate::support::read_missing_headers())
    };
}
