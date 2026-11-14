#![no_main]

use libfuzzer_sys::fuzz_target;
use broadcast_client::tshttp::{Config, TsHttpClient, TsHttpPolicy};
use broadcast_common::stage::Timestamp;

fuzz_target!(|data: &[u8]| {
    let mut client = TsHttpClient::new("http://example/live.ts", Config::default(), Timestamp::ZERO);
    let mut now = 0u64;
    for chunk in data.chunks(64) {
        now = now.saturating_add(1);
        let at = Timestamp::from_millis(now);
        if chunk.first().copied().unwrap_or(0) & 1 == 0 {
            client.feed(chunk, at);
        } else {
            match chunk.first().copied().unwrap_or(0) % 7 {
                0 => client.on_response(200, &[], at),
                1 => client.on_response(404, &[], at),
                6 => {
                    let len = u64::from(chunk.get(1).copied().unwrap_or(0)) * 188;
                    client.on_response(200, &[("Content-Length".to_string(), len.to_string())], at)
                }
                2 => client.on_eof(at),
                3 => client.on_error(at),
                4 => client.reconnect(at),
                _ => client.poll_timeout(at),
            }
        }
        while client.poll().is_some() {}
        while client.poll_event().is_some() {}
        let _ = client.next_deadline();
    }

    let mut policy = TsHttpPolicy::new("http://example/live.ts", Config::default(), Timestamp::ZERO);
    let mut now = 0u64;
    for chunk in data.chunks(64) {
        now = now.saturating_add(1);
        let at = Timestamp::from_millis(now);
        let tag = chunk.first().copied().unwrap_or(0);
        match tag % 6 {
            0 => policy.on_response(
                200,
                &[(
                    "Content-Length".to_string(),
                    (u64::from(chunk.get(1).copied().unwrap_or(0)) * 188).to_string(),
                )],
                at,
            ),
            1 => policy.on_response(404, &[], at),
            2 => policy.on_eof(at),
            3 => policy.on_error(at),
            4 => policy.poll_timeout(at),
            _ => policy.on_bytes(chunk.len(), at, tag & 0x80 != 0),
        }
        while policy.poll().is_some() {}
        while policy.poll_event().is_some() {}
        let _ = policy.next_deadline();
    }
});
