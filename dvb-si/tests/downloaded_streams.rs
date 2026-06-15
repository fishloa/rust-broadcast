//! Extended-capture tests over **fetch-on-demand** broadcast streams (issue #67).
//!
//! The captures are large, real, third-party muxes that are NOT vendored in the
//! repo. Pull them with `tools/fetch-test-streams.sh` (they land in the
//! gitignored `<repo>/.test-streams/`). This test then runs every present
//! capture through `SiDemux` and asserts it parses real SI without panicking.
//!
//! When no captures are present (the default — nothing downloaded), the test
//! **skips cleanly** so `cargo test` passes without the downloads. CI may run
//! the fetch script first to exercise the extended corpus.
#![cfg(feature = "ts")]

use std::fs;
use std::path::{Path, PathBuf};

use dvb_si::demux::SiDemux;
use dvb_si::ts::TS_PACKET_SIZE;

/// `<repo>/.test-streams/` — `CARGO_MANIFEST_DIR` is the `dvb-si/` crate dir.
fn streams_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join(".test-streams")
}

fn ts_files(dir: &Path) -> Vec<PathBuf> {
    let mut v = Vec::new();
    if let Ok(rd) = fs::read_dir(dir) {
        for entry in rd.flatten() {
            let p = entry.path();
            if p.extension().is_some_and(|x| x == "ts") {
                v.push(p);
            }
        }
    }
    v.sort();
    v
}

#[test]
fn downloaded_streams_parse_without_panic() {
    let files = ts_files(&streams_dir());
    if files.is_empty() {
        eprintln!(
            "downloaded_streams: SKIPPED — no captures in .test-streams/. \
             Run `tools/fetch-test-streams.sh` to enable the extended-corpus tests."
        );
        return;
    }

    for f in files {
        let data = fs::read(&f).expect("read capture");
        let name = f
            .file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .into_owned();

        let mut demux = SiDemux::builder().build();
        let (mut sections, mut errors) = (0usize, 0usize);
        for chunk in data.chunks(TS_PACKET_SIZE) {
            if chunk.len() != TS_PACKET_SIZE || chunk[0] != 0x47 {
                continue;
            }
            for ev in demux.feed(chunk) {
                match ev.table_section() {
                    Ok(_) => sections += 1,
                    Err(_) => errors += 1,
                }
            }
        }

        eprintln!(
            "downloaded_streams: {name} — {sections} SI sections parsed, \
             {errors} parse errors, {} bytes",
            data.len()
        );
        // A real mux must yield SI through the PAT-following demux; the whole
        // walk completing also proves no panic on real-world data.
        assert!(
            sections > 0,
            "{name}: expected at least one SI section from a real broadcast mux"
        );
    }
}
