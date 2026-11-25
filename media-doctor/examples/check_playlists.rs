//! `check_playlists` — lint HLS playlists with `media_doctor::check_playlist`.
//!
//! Runs the validator over the committed `fixtures/hls/*.m3u8` set (one valid
//! playlist and several deliberately broken ones) and prints each file's
//! findings. Optional arguments replace the default set with your own files:
//!
//! ```text
//! cargo run -p media-doctor --example check_playlists [playlist.m3u8 ...]
//! ```
//!
//! A deliberately broken fixture is *expected* to produce findings; the
//! example exits non-zero only if a file cannot be read, or if the valid
//! fixture stops being clean (which would mean the validator regressed).

use broadcast_common::bounded::{Limit, read_file_bounded_string};
use media_doctor::{Report, check_playlist};
use std::path::Path;

/// A whole HLS playlist: the ceiling `broadcast-hls` itself accepts.
const PLAYLIST_LIMIT: Limit = Limit::new(
    "HLS playlist",
    broadcast_hls::limits::MAX_PLAYLIST_BYTES as u64,
);

const DEFAULT_SET: [&str; 5] = [
    "valid.m3u8",
    "reference-vod.m3u8",
    "missing-extm3u.m3u8",
    "bad-extinf.m3u8",
    "malformed-daterange.m3u8",
];

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let paths: Vec<String> = if args.is_empty() {
        // `CARGO_MANIFEST_DIR`-relative, so it works from any working directory.
        DEFAULT_SET
            .iter()
            .map(|n| {
                test_fixtures::path(&format!("hls/{}", n))
                    .to_string_lossy()
                    .into_owned()
            })
            .collect()
    } else {
        args
    };

    let mut failed = false;
    for path in &paths {
        let text = match read_file_bounded_string(Path::new(path), PLAYLIST_LIMIT) {
            Ok(t) => t,
            Err(e) => {
                eprintln!("failed to read {path}: {e}");
                std::process::exit(2);
            }
        };
        let mut report = Report::new();
        check_playlist(&text, &mut report);
        println!("{path}: {} finding(s)", report.len());
        for finding in report.findings() {
            println!("    {finding}");
        }
        if path.ends_with("/valid.m3u8") && !report.is_empty() {
            eprintln!("regression: the valid fixture should be clean");
            failed = true;
        }
    }
    if failed {
        std::process::exit(1);
    }
}
