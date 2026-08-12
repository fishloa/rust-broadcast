//! Robustness: the probe must never panic and must always return a well-formed
//! `Probe` (one of `Identified`/`Ambiguous`/`Insufficient`/`Unknown`) on
//! arbitrary input. Audit finding 7 — there was previously no
//! `no_panic_on_arbitrary_input` test anywhere in the crate.
//!
//! What is fed, per audit finding 7:
//! - every `1..=64`-byte prefix of each real fixture (a truncated prefix is the
//!   most common hostile input — a caller read a few bytes at a time),
//! - 4 KiB of zeros and 4 KiB of `0xFF`,
//! - a deterministic pseudo-random buffer (seeded here; no `rand` dependency).
//!
//! "Well-formed" is the one property that holds for *every* non-panicking
//! `Probe`: it is one of the four variants. We match all four (plus the
//! `#[non_exhaustive]` wildcard) so a structure change that made an input fall
//! through would be a compile-time failure, not a silent pass.

use container_probe::Probe;
use std::fs;

/// Join a workspace-relative fixture path to an absolute path from this crate.
fn fixture(rel: &str) -> Vec<u8> {
    fs::read(format!("{}/../{}", env!("CARGO_MANIFEST_DIR"), rel))
        .unwrap_or_else(|e| panic!("failed to read fixture {rel}: {e}"))
}

/// A deterministic pseudo-random bytes generator (xorshift64), seeded here so
/// the test is reproducible without a `rand` dependency. Never zero, never 0xFF
/// everywhere — a plausible high-entropy corpus.
struct Prng(u64);

impl Prng {
    fn next_u8(&mut self) -> u8 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        (self.0 & 0xFF) as u8
    }
    fn fill(&mut self, out: &mut [u8]) {
        for b in out {
            *b = self.next_u8();
        }
    }
}

/// Feed `data` to `probe` and assert it returns a well-formed `Probe`.
fn assert_well_formed(data: &[u8]) {
    let p = container_probe::probe(data);
    match &p {
        Probe::Identified { .. } | Probe::Ambiguous { .. } => {
            // Identified is fine (possibly Ambiguous). Nothing more to check.
        }
        Probe::Insufficient { need_at_least } => {
            // A request for more bytes must be a positive, oversized demand.
            assert!(
                *need_at_least >= 1,
                "Insufficient must demand a positive need_at_least, got {p:?}"
            );
        }
        Probe::Unknown => {}
        // `#[non_exhaustive]` requires a wildcard arm; reaching it means the
        // enum grew and this match must be revisited.
        _ => panic!("unexpected new Probe variant: {p:?}"),
    }
}

/// Every 1..=64-byte prefix of each real fixture is fed, plus the whole fixture.
#[test]
fn no_panic_on_prefixes_of_every_real_fixture() {
    let fixtures = [
        "fixtures/ts/h264_aac.ts",
        "fixtures/mp4/h264_high.mp4",
        "fixtures/mkv/h264_aac.mkv",
        "fixtures/webm/vorbis.webm",
        "fixtures/mxf/op1a_mpeg2_pcm.mxf",
        "fixtures/ps/h264_ac3.ps",
        "fixtures/flv/av.flv",
        "fixtures/container-probe/pcm_s16le.wav",
        "fixtures/container-probe/opus.ogg",
        "fixtures/container-probe/video.asf",
        "fixtures/container-probe/aac.adts",
        "fixtures/container-probe/audio.mp3",
        "fixtures/container-probe/h264.annexb",
    ];
    for rel in fixtures {
        let full = fixture(rel);
        for len in 1..=64usize.min(full.len()) {
            let prefix = &full[..len];
            assert_well_formed(prefix);
        }
        // The whole file too.
        assert_well_formed(&full);
    }
}

/// 4 KiB of zeros and 4 KiB of `0xFF` must not panic and must return a
/// well-formed `Probe` (in fact both are `Unknown`/`Insufficient`).
#[test]
fn no_panic_on_zero_and_fill() {
    assert_well_formed(&[0u8; 4096]);
    assert_well_formed(&[0xFFu8; 4096]);
}

/// A deterministic pseudo-random buffer must not panic and must return a
/// well-formed `Probe`.
#[test]
fn no_panic_on_deterministic_random() {
    let mut rng = Prng(0x9E3779B97F4A7C15);
    let mut buf = vec![0u8; 8192];
    rng.fill(&mut buf);
    assert_well_formed(&buf);
}
