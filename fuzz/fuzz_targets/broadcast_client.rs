#![no_main]

use libfuzzer_sys::fuzz_target;
use broadcast_client::{Config, Session};

fuzz_target!(|data: &[u8]| {
    let mut session = match Session::open("http://example/live.ts", Config::default(), 0) {
        Ok(s) => s,
        Err(_) => return,
    };
    let mut now = 0u64;
    let mut conn_ids = Vec::new();
    for chunk in data.chunks(32) {
        now = now.saturating_add(1);
        match chunk.first().copied().unwrap_or(0) % 7 {
            0 => {
                if let Some(broadcast_client::Action::HttpRequest { conn, .. }) = session.poll(now) {
                    conn_ids.push(conn);
                    let _ = session.on_response(conn, 200, &[]);
                    session.on_data(conn, chunk, now);
                }
            }
            1 => {
                if let Some(conn) = conn_ids.first().copied() {
                    session.on_data(conn, chunk, now);
                }
            }
            2 => {
                if let Some(conn) = conn_ids.first().copied() {
                    session.on_closed(conn, None, now);
                }
            }
            3 => {
                session.set_credentials(broadcast_client::Credentials::bearer("token"));
            }
            4 => {
                session.reconnect(now);
            }
            5 => {
                session.close();
            }
            _ => {
                let _ = session.poll(now);
            }
        }
        while let Some(_) = session.poll(now) {}
        while let Some(_) = session.next_event() {}
    }
});
