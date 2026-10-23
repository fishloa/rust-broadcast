#![no_main]

use libfuzzer_sys::fuzz_target;
use tshttp_runtime::{Config, TsHttpClient};

fuzz_target!(|data: &[u8]| {
    let mut client = TsHttpClient::new("http://example/live.ts", Config::default(), 0);
    let mut now = 0u64;
    for chunk in data.chunks(64) {
        now = now.saturating_add(1);
        if chunk.first().copied().unwrap_or(0) & 1 == 0 {
            client.feed(chunk, now);
        } else {
            match chunk.first().copied().unwrap_or(0) % 6 {
                0 => client.on_response(200, now),
                1 => client.on_response(404, now),
                2 => client.on_eof(now),
                3 => client.on_error(now),
                4 => client.reconnect(now),
                _ => client.poll_timeout(now),
            }
        }
        while let Some(_) = client.poll() {}
        while let Some(_) = client.poll_event() {}
        let _ = client.next_deadline();
    }
});
