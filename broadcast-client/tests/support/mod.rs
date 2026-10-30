#![allow(dead_code)]
pub mod fmp4_split;
pub mod hls_host;
pub mod renditions;
pub mod replay;
pub mod rtsp_host;
pub mod truth;
pub mod ts_host;
pub mod ts_pmt;
pub mod ts_probe;
pub mod ts_shift;
pub mod ts_split;

use broadcast_client::{Action, Event, Session};

/// `test_fixtures::require` already printed the loud `SKIPPED` line and returned `None`, which
/// only happens when `private/` is not checked out (or `SKIP_PRIVATE_FIXTURES=1`); with `private/`
/// present a missing fixture, or `REQUIRE_PRIVATE_FIXTURES=1`, panics inside `require` and never
/// reaches here. Libtest cannot skip one test at runtime, so the binary stops cleanly instead.
fn skip_without_private() -> ! {
    eprintln!(
        "broadcast-client: SKIPPED - the rest of this test binary needs the private zenith-http fixtures"
    );
    std::process::exit(0)
}

pub fn read_tf1_headers() -> (u16, Vec<(String, String)>) {
    let path = test_fixtures::require("zenith-http/tf1.headers.txt", "support")
        .unwrap_or_else(|| skip_without_private());
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
    (status, headers)
}

pub fn read_tf1_bytes() -> Vec<u8> {
    test_fixtures::require_bytes("zenith-http/tf1-live.ts", "support")
        .unwrap_or_else(|| skip_without_private())
}

pub fn read_missing_headers() -> (u16, Vec<(String, String)>) {
    let path = test_fixtures::require("zenith-http/missing.headers.txt", "support")
        .unwrap_or_else(|| skip_without_private());
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
    (status, headers)
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
