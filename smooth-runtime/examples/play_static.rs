//! Play a static Smooth Streaming presentation from a directory on disk and print
//! the tracks and per-track sample counts, answering every request the engine
//! makes from the files next to the manifest.
//!
//! # Usage
//!
//! ```bash
//! cargo run -p smooth-runtime --example play_static
//! cargo run -p smooth-runtime --example play_static -- --dir ../fixtures/smooth/av
//! ```

use std::collections::BTreeMap;
use std::path::PathBuf;

use broadcast_common::stage::Timestamp;
use smooth_runtime::{Action, FetchFailure, Output, SmoothClient, SmoothClientConfig};
use url::Url;

/// The example's microsecond clock as the client's `now`.
fn now(us: u64) -> Timestamp {
    Timestamp::from_micros(us)
}

const DEFAULT_DIR: &str = "../fixtures/smooth/av";
const ORIGIN: &str = "http://origin.test/";
const MANIFEST_FILE: &str = "Manifest";

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(DEFAULT_DIR);
    let mut args = std::env::args().skip(1);
    while let Some(flag) = args.next() {
        match (flag.as_str(), args.next()) {
            ("--dir", Some(value)) => dir = PathBuf::from(value),
            _ => {
                return Err(
                    format!("usage: play_static [--dir <fixture dir>] (got {flag})").into(),
                );
            }
        }
    }

    let config = SmoothClientConfig::new(format!("{ORIGIN}{MANIFEST_FILE}"));
    assert_eq!(
        config.manifest_url,
        Url::parse(&format!("{ORIGIN}{MANIFEST_FILE}"))?,
        "the constructor carries the manifest URL"
    );
    let mut client = SmoothClient::new(config);
    let mut samples: BTreeMap<u32, usize> = BTreeMap::new();
    let mut now_us = 0u64;
    loop {
        while let Some(action) = client.poll(now(now_us)) {
            let (id, url, is_manifest) = match action {
                Action::FetchManifest { id, url } => (id, url, true),
                Action::FetchFragment { id, url, .. } => (id, url, false),
                _ => continue,
            };
            let relative = url
                .path()
                .trim_start_matches('/')
                .replace("%28", "(")
                .replace("%29", ")");
            match std::fs::read(dir.join(relative)) {
                Ok(body) if is_manifest => client.on_manifest(now(now_us), id, &body),
                Ok(body) => client.on_fragment(now(now_us), id, &body),
                Err(_) => client.on_failure(
                    now(now_us),
                    id,
                    FetchFailure {
                        status: Some(404),
                        ..FetchFailure::default()
                    },
                ),
            }
        }
        while let Some(output) = client.next_output() {
            match output {
                Output::Tracks(tracks) => {
                    for t in &tracks {
                        println!(
                            "track {} timescale {}",
                            t.track_id.value(),
                            t.timescale.get()
                        );
                    }
                }
                Output::Sample { track, .. } => *samples.entry(track.value()).or_default() += 1,
                Output::Fatal(error) => return Err(error.into()),
                _ => {}
            }
        }
        if client.is_ended() {
            break;
        }
        match client.next_deadline() {
            Some(t) => now_us = now_us.max(t),
            None => break,
        }
    }
    for (track, count) in samples {
        println!("track {track}: {count} samples");
    }
    Ok(())
}
