//! Regenerates the committed PAFF fixtures under `fixtures/video/paff/`.
//!
//! `cargo run -p transmux --example regen_paff_fixtures -- --write`
//!
//! Without `--write` it only prints the SHA-256 of what it would emit, so a
//! checkout can be compared against `fixtures/video/paff/PROVENANCE.md`
//! without touching the tree. The generator is the same spec-built I_PCM
//! H.264 writer the tests use (`tests/it/support/paff_gen.rs`); it lives here,
//! not behind `#[ignore]` in a test, so no `cargo test` invocation can rewrite
//! a committed fixture from the code under test.

#[path = "../tests/it/support/bit_writer.rs"]
mod bit_writer;
#[path = "../tests/it/support/paff_gen.rs"]
mod paff_gen;

use std::path::Path;
use transmux::Media;

const FRAMES: usize = 12;

fn main() {
    let write = std::env::args().skip(1).any(|a| a == "--write");
    let dir = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../fixtures/video/paff")
        .canonicalize()
        .unwrap_or_else(|e| panic!("fixtures/video/paff: {e}"));
    for (name, g) in [
        (
            "paff-tff-12f-64x64",
            paff_gen::generate(FRAMES, 64, 64, false),
        ),
        (
            "paff-bff-12f-64x64",
            paff_gen::generate(FRAMES, 64, 64, true),
        ),
    ] {
        let rewrapped = paff_gen::ts_from_aus(&g.aus);
        let want =
            timestamps(&test_harness::collect::media_from_ts(&g.ts).expect("demux generated TS"));
        let got = timestamps(
            &test_harness::collect::media_from_ts(&rewrapped).expect("demux re-wrapped AUs"),
        );
        assert_eq!(got, want, "{name}: the AUs re-wrapped carry the same clock");
        println!(
            "{name}: {} B elementary stream, {} field AUs",
            g.es.len(),
            g.aus.len()
        );
        for (ext, bytes) in [("ts", &g.ts), ("yuv", &g.yuv)] {
            println!("{name}.{ext} sha256 {}", digest(bytes));
            if write {
                let path = dir.join(format!("{name}.{ext}"));
                std::fs::write(&path, bytes).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
            }
        }
    }
    if !write {
        println!("(dry run; pass --write to rewrite the fixtures)");
    }
}

fn timestamps(media: &Media) -> Vec<(Option<i64>, Option<i64>)> {
    media
        .tracks
        .iter()
        .flat_map(|t| t.samples.iter())
        .map(|s| {
            (
                s.timing.and_then(|t| t.dts()),
                s.timing.and_then(|t| t.pts()),
            )
        })
        .collect()
}

fn digest(bytes: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    broadcast_common::hex::hex_encode(&Sha256::digest(bytes))
}
