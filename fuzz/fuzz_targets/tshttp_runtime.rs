#![no_main]

use libfuzzer_sys::fuzz_target;
use tshttp_runtime::{Config, TsHttpClient, TsHttpPolicy};

fuzz_target!(|data: &[u8]| {
    let mut client = TsHttpClient::new("http://example/live.ts", Config::default(), 0);
    let mut now = 0u64;
    for chunk in data.chunks(64) {
        now = now.saturating_add(1);
        if chunk.first().copied().unwrap_or(0) & 1 == 0 {
            client.feed(chunk, now);
        } else {
            match chunk.first().copied().unwrap_or(0) % 7 {
                0 => client.on_response(200, &[], now),
                1 => client.on_response(404, &[], now),
                6 => {
                    let len = u64::from(chunk.get(1).copied().unwrap_or(0)) * 188;
                    client.on_response(
                        200,
                        &[("Content-Length".to_string(), len.to_string())],
                        now,
                    )
                }
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

    let mut policy = TsHttpPolicy::new("http://example/live.ts", Config::default(), 0);
    let mut now = 0u64;
    for chunk in data.chunks(64) {
        now = now.saturating_add(1);
        let tag = chunk.first().copied().unwrap_or(0);
        match tag % 6 {
            0 => policy.on_response(
                200,
                &[(
                    "Content-Length".to_string(),
                    (u64::from(chunk.get(1).copied().unwrap_or(0)) * 188).to_string(),
                )],
                now,
            ),
            1 => policy.on_response(404, &[], now),
            2 => policy.on_eof(now),
            3 => policy.on_error(now),
            4 => policy.poll_timeout(now),
            _ => policy.on_bytes(chunk.len(), now, tag & 0x80 != 0),
        }
        while let Some(_) = policy.poll() {}
        while let Some(_) = policy.poll_event() {}
        let _ = policy.next_deadline();
    }
});
