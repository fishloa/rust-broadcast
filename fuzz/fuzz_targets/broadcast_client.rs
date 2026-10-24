#![no_main]

use broadcast_client::{Action, Config, Credentials, Session};
use libfuzzer_sys::fuzz_target;

const URLS: [&str; 2] = ["http://example/live.ts", "http://example/live.m3u8"];
const HLS_SEED: &[u8] = b"#EXTM3U\n#EXT-X-VERSION:3\n#EXT-X-TARGETDURATION:2\n#EXT-X-MEDIA-SEQUENCE:0\n#EXTINF:2.0,\nseg0.ts\n#EXTINF:2.0,\nseg1.ts\n";
const STEP_US: u64 = 700_000;
const COOKIE_NAMES: [&str; 3] = ["sid", "a", "t"];
const STATUS_CODES: [u16; 6] = [200, 200, 404, 302, 401, 500];

fn drive(url: &str, data: &[u8]) {
    let config = Config {
        initial_cookies: vec![("seed".to_string(), "1".to_string())],
        extra_headers: vec![("X-Fuzz".to_string(), "1".to_string())],
        ..Config::default()
    };
    let mut session = match Session::open(url, config, 0) {
        Ok(s) => s,
        Err(_) => return,
    };
    let mut now = 0u64;
    let mut conn_ids: Vec<u32> = Vec::new();
    for chunk in data.chunks(32) {
        now = now.saturating_add(STEP_US);
        let pick = chunk.first().copied().unwrap_or(0);
        let conn = conn_ids.get(usize::from(pick) % conn_ids.len().max(1)).copied();
        match pick % 9 {
            0 => {
                while let Some(action) = session.poll(now) {
                    if let Action::HttpRequest { conn, .. } = action {
                        conn_ids.push(conn);
                        session.on_response(conn, 200, &[]);
                        session.on_data(conn, HLS_SEED, now);
                        session.on_data(conn, chunk, now);
                    }
                }
            }
            1 => {
                if let Some(conn) = conn {
                    session.on_data(conn, chunk, now);
                }
            }
            2 => {
                if let Some(conn) = conn {
                    session.on_closed(conn, None, now);
                }
            }
            3 => {
                session.set_credentials(Credentials::bearer("token"));
            }
            4 => {
                session.reconnect(now);
            }
            5 => {
                if let Some(conn) = conn {
                    let status = STATUS_CODES[usize::from(chunk.get(1).copied().unwrap_or(0)) % STATUS_CODES.len()];
                    let mut headers = vec![("Location".to_string(), "http://example/next.ts".to_string())];
                    let attrs = String::from_utf8_lossy(chunk.get(2..).unwrap_or(&[])).into_owned();
                    let name = COOKIE_NAMES[usize::from(chunk.get(1).copied().unwrap_or(0)) % COOKIE_NAMES.len()];
                    headers.push(("Set-Cookie".to_string(), format!("{name}=v; {attrs}")));
                    headers.push(("Set-Cookie".to_string(), attrs));
                    session.on_response(conn, status, &headers);
                }
            }
            6 => {
                if let Some(conn) = conn {
                    session.on_closed(conn, Some("reset".to_string()), now);
                }
            }
            7 => {
                let _ = session.select_variant(usize::from(chunk.get(1).copied().unwrap_or(0)));
            }
            _ => {
                let _ = session.poll(now);
            }
        }
        if pick == u8::MAX {
            session.close();
        }
        while let Some(action) = session.poll(now) {
            if let Action::HttpRequest { conn, .. } = action {
                conn_ids.push(conn);
            }
        }
        let _ = session.cookies();
        while session.next_event().is_some() {}
    }
}

fuzz_target!(|data: &[u8]| {
    for url in URLS {
        drive(url, data);
    }
});
