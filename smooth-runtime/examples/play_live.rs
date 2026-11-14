//! Turn a static fixture into a live presentation (`IsLive` plus a DVR window) and
//! show where a live join starts: which fragments the engine asks for first, and
//! how far behind the newest one it begins.
//!
//! # Usage
//!
//! ```bash
//! cargo run -p smooth-runtime --example play_live
//! cargo run -p smooth-runtime --example play_live -- --look-ahead 0
//! ```

use std::path::PathBuf;

use broadcast_common::stage::Timestamp;
use smooth_runtime::{Action, FetchFailure, Output, SmoothClient, SmoothConfig};
use url::Url;

/// The example's microsecond clock as the client's `now`.
fn now(us: u64) -> Timestamp {
    Timestamp::from_micros(us)
}

const FIXTURE_DIR: &str = "../fixtures/smooth/av";
const ORIGIN: &str = "http://origin.test/";
const STATIC_ATTRS: &str = "Duration=\"30415941\"";
const LIVE_ATTRS: &str = "Duration=\"30415941\" IsLive=\"TRUE\" DVRWindowLength=\"20000000\"";
const DEFAULT_LOOK_AHEAD: usize = 2;
const RUN_US: u64 = 10_000_000;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut look_ahead = DEFAULT_LOOK_AHEAD;
    let mut args = std::env::args().skip(1);
    while let Some(flag) = args.next() {
        match (flag.as_str(), args.next()) {
            ("--look-ahead", Some(value)) => look_ahead = value.parse()?,
            _ => return Err(format!("usage: play_live [--look-ahead <n>] (got {flag})").into()),
        }
    }

    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(FIXTURE_DIR);
    let manifest = std::fs::read_to_string(dir.join("Manifest"))?.replace(STATIC_ATTRS, LIVE_ATTRS);
    let config = SmoothConfig::default().with_live_join_fragments(look_ahead);
    let mut client = SmoothClient::new(Url::parse(&format!("{ORIGIN}Manifest"))?, config);
    let mut requested = Vec::new();
    let mut now_us = 0u64;
    while now_us < RUN_US {
        while let Some(action) = client.poll(now(now_us)) {
            match action {
                Action::FetchManifest { id, .. } => {
                    client.on_manifest(now(now_us), id, manifest.as_bytes())
                }
                Action::FetchFragment { id, url, start, .. } => {
                    requested.push(format!("{} (start {start})", url.path()));
                    let relative = url
                        .path()
                        .trim_start_matches('/')
                        .replace("%28", "(")
                        .replace("%29", ")");
                    match std::fs::read(dir.join(relative)) {
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
                _ => {}
            }
        }
        while let Some(output) = client.next_output() {
            if let Output::Fatal(error) = output {
                return Err(error.into());
            }
        }
        match client.next_deadline() {
            Some(t) => now_us = now_us.max(t),
            None => break,
        }
    }
    for line in &requested {
        println!("{line}");
    }
    Ok(())
}
