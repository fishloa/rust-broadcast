//! Extended-capture tests over real third-party broadcast muxes (issue #67).
//!
//! The captures are large, real, third-party muxes kept in the private
//! fixtures repo (`private/fixtures/ts/`, the `private/` submodule; resolved by
//! `test_fixtures`, which also falls back to a fetched `.test-streams/` — see
//! `tools/fetch-test-streams.sh`). This test runs every resolvable capture
//! through `SiDemux` and asserts it parses real SI without panicking.
//!
//! When the `private/` submodule is not checked out (public/CI clones), the
//! test **skips loudly**; with it checked out, a missing capture is a failure.
#![cfg(feature = "ts")]

use std::fs;

use dvb_si::demux::SiDemux;
use mpeg_ts::ts::TS_PACKET_SIZE;

#[test]
fn downloaded_streams_parse_without_panic() {
    let files = test_fixtures::private_files("ts", &["ts"]);
    if files.is_empty() {
        eprintln!(
            "downloaded_streams: SKIPPED - no captures under private/fixtures/ts (private repo not checked out)."
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
