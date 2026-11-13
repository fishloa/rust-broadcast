#![no_main]

use hls_runtime::client::{Action, HlsClient, ResourceId};
use libfuzzer_sys::fuzz_target;

const PLAYLIST_URL: &str = "http://origin/live/stream.m3u8";
const MAX_ROUNDS: usize = 16;
const MAX_ACTIONS_PER_ROUND: usize = 64;
const SPLIT_SEED: usize = 3;

fuzz_target!(|data: &[u8]| {
    let mut client = HlsClient::new(hls_runtime::client::HlsClientConfig::new(PLAYLIST_URL));
    let mut rest = data;
    for round in 0..MAX_ROUNDS {
        let cut = rest.len() / (SPLIT_SEED + round % 3).max(1);
        let (chunk, tail) = rest.split_at(cut.max(rest.len().min(1)));
        rest = tail;
        let _ = client.on_playlist(chunk);
        for _ in 0..MAX_ACTIONS_PER_ROUND {
            match client.poll() {
                Some(Action::FetchResource { id, .. }) => {
                    let body = rest.get(..rest.len().min(4096)).unwrap_or(&[]);
                    let _ = match id {
                        ResourceId::Init | ResourceId::Part { .. } | ResourceId::Segment { .. } => {
                            client.on_resource(id, body)
                        }
                        _ => Ok(()),
                    };
                }
                Some(Action::FetchKey { id, .. }) => {
                    let _ = client.on_key(id, &rest[..rest.len().min(16)]);
                }
                Some(_) => {}
                None => break,
            }
        }
        while client.next_output().is_some() {}
    }
});
