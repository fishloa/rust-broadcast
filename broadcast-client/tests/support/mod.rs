pub mod dash_checks;
pub mod dash_common;
pub mod dash_matrix;
pub mod dash_sim;
pub mod fmp4_split;
pub mod hls_host;
pub mod renditions;
pub mod replay;
pub mod rtmp_host;
pub mod rtmp_wire;
pub mod rtsp_host;
pub mod srt_peer;
pub mod srt_sim;
pub mod tls_pki;
pub mod truth;
pub mod ts_host;
pub mod ts_pmt;
pub mod ts_probe;
pub mod ts_shift;
pub mod ts_split;

/// Test-only reply headers from `(name, value)` pairs, so a fixture reads as the fields it names.
pub fn headers(pairs: &[(&str, &str)]) -> http::HeaderMap {
    let mut map = http::HeaderMap::new();
    for (name, value) in pairs {
        let name = http::HeaderName::from_bytes(name.as_bytes()).expect("a test names a header");
        let value = http::HeaderValue::from_str(value).expect("a test writes a valid header value");
        map.append(name, value);
    }
    map
}

/// Reply statuses from the numeric codes a fixture table names.
pub fn status(code: u16) -> http::StatusCode {
    http::StatusCode::from_u16(code).expect("a test names a real status")
}

/// The `Content-Type` a TS-over-HTTP origin answers with.
pub fn ts_content_type() -> http::HeaderMap {
    let mut map = http::HeaderMap::new();
    map.insert(
        http::header::CONTENT_TYPE,
        http::HeaderValue::from_static("video/MP2T"),
    );
    map
}
pub mod udp_host;
pub mod video_run;
pub mod wire_host;

use broadcast_client::{Action, Event, Session};
use broadcast_common::stage::Timestamp;

/// The harnesses' own microsecond clock as a [`Timestamp`] for the session's `now` boundary.
pub fn ts(us: u64) -> Timestamp {
    Timestamp::from_micros(us)
}

pub fn read_tf1_headers() -> Option<(http::StatusCode, http::HeaderMap)> {
    let path = test_fixtures::require("http/live-channels/tf1.headers.txt", "support")?;
    let text = std::fs::read_to_string(path).unwrap();
    let code: u16 = text
        .lines()
        .next()
        .unwrap()
        .split_whitespace()
        .nth(1)
        .unwrap()
        .parse()
        .unwrap();
    let status = crate::support::status(code);
    let mut headers = http::HeaderMap::new();
    for line in text.lines().skip(1) {
        if line.trim().is_empty() {
            continue;
        }
        if let Some((k, v)) = line.split_once(':') {
            append_header(&mut headers, k.trim(), v.trim());
        }
    }
    Some((status, headers))
}

pub fn read_tf1_bytes() -> Option<Vec<u8>> {
    test_fixtures::require_bytes("http/live-channels/tf1-live.ts", "support")
}

pub fn read_missing_headers() -> Option<(http::StatusCode, http::HeaderMap)> {
    let path = test_fixtures::require("http/live-channels/missing.headers.txt", "support")?;
    let text = std::fs::read_to_string(path).unwrap();
    let code: u16 = text
        .lines()
        .next()
        .unwrap()
        .split_whitespace()
        .nth(1)
        .unwrap()
        .parse()
        .unwrap();
    let status = crate::support::status(code);
    let mut headers = http::HeaderMap::new();
    for line in text.lines().skip(1) {
        if line.trim().is_empty() {
            continue;
        }
        if let Some((k, v)) = line.split_once(':') {
            append_header(&mut headers, k.trim(), v.trim());
        }
    }
    Some((status, headers))
}

/// Append a captured reply header (a fixture line) to a reply map.
fn append_header(headers: &mut http::HeaderMap, name: &str, value: &str) {
    let name = http::HeaderName::from_bytes(name.as_bytes()).expect("a capture names a header");
    if let Ok(value) = http::HeaderValue::from_str(value) {
        headers.append(name, value);
    }
}

pub fn feed_chunks(
    session: &mut Session,
    bytes: &[u8],
    chunk_size: usize,
    status: http::StatusCode,
    resp_headers: &http::HeaderMap,
    now_us: u64,
) -> Vec<Event> {
    let mut conn = None;
    // initial request
    while let Some(action) = session.poll(ts(now_us)) {
        match action {
            Action::HttpRequest { conn: c, .. } => {
                conn = Some(c);
                session.on_response(c, status, resp_headers, ts(now_us));
            }
            Action::Close { conn: c } => {
                session.on_closed(c, None, ts(now_us));
            }
            Action::WakeAt(t) => {
                if t.as_micros() <= now_us {
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
        session.on_data(c, chunk, ts(now_us));
    }
    // collect
    let mut out = Vec::new();
    while let Some(e) = session.next_event() {
        out.push(e);
    }
    out
}

macro_rules! skip_none {
    ($e:expr) => {
        match $e {
            Some(v) => v,
            None => return,
        }
    };
}

macro_rules! tf1_bytes {
    () => {
        skip_none!($crate::support::read_tf1_bytes())
    };
}

macro_rules! tf1_headers {
    () => {
        skip_none!($crate::support::read_tf1_headers())
    };
}

macro_rules! missing_headers {
    () => {
        skip_none!($crate::support::read_missing_headers())
    };
}
