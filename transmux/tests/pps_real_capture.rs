//! Issue #1186 (follow-up of #1166): PPS tracking is per `pic_parameter_set_id`
//! (H.264 §7.3.2.2 / H.265 §7.3.2.3.1: the first syntax element), so a PPS id
//! announced with new content (A -> B -> A on one id) is re-reported at every
//! switch, while several ids rotating (#1166) stay silent.
//!
//! Real captures: the committed TS fixtures and the first 48 MB of the local
//! archive's `france-tnt-uhf32.ts` / `hotbird-mhp.ts` /
//! `transformers-eia608-h264.ts` (skipped cleanly when absent; never
//! committed). The assertion is that the number of `TrackUpdated` events each
//! produces equals what the demuxer produced before the per-id change (a
//! real encoder repeating or rotating its headers must not start flapping).

use std::io::Read;
use std::path::PathBuf;

use transmux::{DemuxEvent, StreamingTsDemux};

fn updates(ts: &[u8]) -> usize {
    let mut d = StreamingTsDemux::new();
    let mut n = 0;
    let mut drain = |d: &mut StreamingTsDemux| {
        while let Some(e) = d.poll_event() {
            n += usize::from(matches!(e, DemuxEvent::TrackUpdated(_)));
        }
    };
    for c in ts.chunks(188 * 64) {
        d.feed(c);
        drain(&mut d);
    }
    d.finish();
    drain(&mut d);
    n
}

fn root(parts: &[&str]) -> PathBuf {
    let mut p = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    p.push("..");
    p.extend(parts);
    p
}

fn head(path: &PathBuf, n: u64) -> Option<Vec<u8>> {
    let f = std::fs::File::open(path).ok()?;
    let mut buf = Vec::new();
    f.take(n).read_to_end(&mut buf).ok()?;
    buf.truncate(buf.len() / 188 * 188);
    Some(buf)
}

#[test]
fn real_captures_track_update_counts() {
    let mut checked = 0usize;
    for name in [
        "h264_aac.ts",
        "gulli-opengop.ts",
        "france2.ts",
        "m6-single.ts",
    ] {
        let path = root(&["fixtures", "ts", name]);
        let bytes = std::fs::read(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
        // Pinned from the pre-change demuxer over the same bytes: 0.
        assert_eq!(updates(&bytes), 0, "{name}: no config flapping");
        checked += 1;
    }
    for name in [
        "france-tnt-uhf32.ts",
        "hotbird-mhp.ts",
        "transformers-eia608-h264.ts",
    ] {
        let path = root(&[".test-streams", name]);
        match head(&path, 48 * 1024 * 1024) {
            Some(b) => {
                assert_eq!(updates(&b), 0, "{name}: no config flapping");
                checked += 1;
            }
            None => eprintln!("SKIP {name}: local archive not present"),
        }
    }
    assert!(checked >= 4, "the committed fixtures always run");
}
