// SPDX-License-Identifier: MIT OR Apache-2.0
//! Issue #1162 R5 — `StreamingTsDemux` chunk-size invariance.
//!
//! The documented invariant (see the `StreamingTsDemux` rustdoc): for any
//! partition of the same input bytes into `feed()` calls, the full event
//! sequence — kinds, order, track ids, provenance, and every sample's
//! `(track, pts, dts, duration, length, content hash)` — is identical,
//! including `TrackAdded`/`TrackUpdated`/`Discontinuity`/`InputDegraded`/
//! `ClockReference`/`TracksResolved`. Draining with `poll_event` after every
//! `feed` or only after `finish` makes no difference either.
//!
//! The baseline is the whole buffer in a single `feed`; it is compared with
//! 1 B, 7, 187, 188, 189, 1000 and 65536 B chunks plus 32 fixed-seed
//! pseudo-random chunkings of 1..=64 KiB (xorshift64*, no extra dependency),
//! over audio+video, multi-track, discontinuity and track-update fixtures.

use std::path::PathBuf;
use transmux::{DemuxEvent, StreamingTsDemux};

fn fixture(rel: &str) -> Vec<u8> {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join(rel);
    std::fs::read(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()))
}

/// FNV-1a 64 over the sample bytes.
fn fnv(data: &[u8]) -> u64 {
    data.iter().fold(0xcbf2_9ce4_8422_2325u64, |h, &b| {
        (h ^ u64::from(b)).wrapping_mul(0x0100_0000_01b3)
    })
}

/// One event reduced to a comparable line. A sample is summarised as
/// `(track, pts, dts, duration, len, hash, sync)`; every other event by its
/// full `Debug` rendering (which includes track specs, provenance and
/// discontinuity/degradation kinds).
fn summarize(ev: &DemuxEvent) -> String {
    match ev {
        DemuxEvent::Sample {
            track_id, sample, ..
        } => format!(
            "Sample track={track_id} pts={:?} dts={:?} dur={:?} len={} hash={:016x} sync={}",
            sample.pts,
            sample.dts,
            sample.duration,
            sample.data.len(),
            fnv(&sample.data),
            sample.flags.is_sync
        ),
        other => format!("{other:?}"),
    }
}

/// Feed `ts` in chunks of the given sizes (cycled), draining after each feed
/// (`drain_each`) or only at the end, then `finish()`.
fn run(ts: &[u8], mut sizes: impl FnMut() -> usize, drain_each: bool) -> Vec<String> {
    let mut d = StreamingTsDemux::new();
    let mut out = Vec::new();
    let mut pos = 0;
    while pos < ts.len() {
        let n = sizes().max(1).min(ts.len() - pos);
        d.feed(&ts[pos..pos + n]);
        pos += n;
        if drain_each {
            while let Some(ev) = d.poll_event() {
                out.push(summarize(&ev));
            }
        }
    }
    d.finish();
    while let Some(ev) = d.poll_event() {
        out.push(summarize(&ev));
    }
    out
}

fn fixed(n: usize) -> impl FnMut() -> usize {
    move || n
}

/// xorshift64* — fixed seed, deterministic.
struct XorShift(u64);
impl XorShift {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 >> 12;
        self.0 ^= self.0 << 25;
        self.0 ^= self.0 >> 27;
        self.0.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }
    /// A chunk length in `1..=64 KiB`.
    fn chunk(&mut self) -> usize {
        (self.next() % (64 * 1024)) as usize + 1
    }
}

fn kinds(events: &[String]) -> std::collections::BTreeMap<&str, usize> {
    let mut m = std::collections::BTreeMap::new();
    for e in events {
        let k = e.split([' ', '(', '{']).next().unwrap();
        *m.entry(k).or_insert(0) += 1;
    }
    m
}

/// PR runs check chunk-size invariance on at most this many packets of each
/// capture (every committed fixture but the two large captures is smaller);
/// `RUST_BROADCAST_SOAK` runs the whole file.
const PR_MAX_PACKETS: usize = 4_250;
const TS_PACKET_SIZE: usize = 188;

fn assert_invariant(rel: &str) -> Vec<String> {
    let mut ts = fixture(rel);
    if std::env::var_os("RUST_BROADCAST_SOAK").is_none() {
        ts.truncate(PR_MAX_PACKETS * TS_PACKET_SIZE);
    }
    let baseline = run(&ts, fixed(ts.len()), true);
    assert!(!baseline.is_empty(), "{rel}: no events");
    for n in [1usize, 7, 187, 188, 189, 1000, 65536] {
        assert_eq!(run(&ts, fixed(n), true), baseline, "{rel}: chunk {n}");
    }
    for seed in 1..=32u64 {
        let mut rng = XorShift(0x9E37_79B9_7F4A_7C15 ^ seed.wrapping_mul(0xD1B5_4A32_D192_ED03));
        assert_eq!(
            run(&ts, || rng.chunk(), true),
            baseline,
            "{rel}: random chunking seed {seed}"
        );
    }
    // Draining only after finish() sees the same sequence.
    assert_eq!(run(&ts, fixed(188), false), baseline, "{rel}: late drain");
    baseline
}

#[test]
fn audio_and_video_fixtures_are_chunk_size_invariant() {
    for rel in [
        "fixtures/ts/h264_aac.ts",
        "fixtures/ts/aac-5_1-640k.ts",
        "fixtures/ts/h264/interlaced.ts",
        "fixtures/ts/hevc/main.ts",
        "fixtures/ts/legacy/mpeg2_mp2.ts",
        "fixtures/ts/legacy/mpeg2_mp2_pes_misaligned.ts",
        "fixtures/ts/dolby/eac3_dvb_0x06.ts",
        "fixtures/ts/dolby/ac3.ts",
        "fixtures/ts/dts/dts_core.ts",
        "fixtures/ts/m6-single.ts",
        "transmux/tests/fixtures/timeline/av_offset.ts",
        "transmux/tests/fixtures/audio_pes_straddle/sintel-ac3-eac3-aligned.ts",
        "transmux/tests/fixtures/paff/paff_tff_12f_64x64.ts",
    ] {
        let events = assert_invariant(rel);
        let k = kinds(&events);
        assert!(k.contains_key("TrackAdded"), "{rel}: {k:?}");
        assert!(k.get("Sample").copied().unwrap_or(0) > 0, "{rel}: {k:?}");
    }
}

#[test]
fn discontinuity_and_track_update_events_are_chunk_size_invariant() {
    // A signalled discontinuity_indicator + PCR re-base on a real capture.
    let events = assert_invariant("fixtures/ts/france-pcr-discontinuity.ts");
    let k = kinds(&events);
    assert!(
        k.get("Discontinuity").copied().unwrap_or(0) > 0,
        "france-pcr-discontinuity.ts must raise Discontinuity: {k:?}"
    );

    // SD <-> HD parameter-set change on one PID: TrackUpdated, same track_id.
    let events = assert_invariant("transmux/tests/fixtures/ts/h264-two-resolutions.ts");
    let k = kinds(&events);
    assert!(
        k.get("TrackUpdated").copied().unwrap_or(0) > 0,
        "h264-two-resolutions.ts must raise TrackUpdated: {k:?}"
    );

    // Discontinuity / data-track fixtures too.
    for rel in [
        "fixtures/ts/m6-discontinuity.ts",
        "fixtures/ts/scte35-other-pid.ts",
        "fixtures/ts/scte35-pcr.ts",
    ] {
        assert_invariant(rel);
    }
}

#[test]
fn large_captures_are_chunk_size_invariant() {
    for rel in [
        "fixtures/ts/gulli-opengop.ts",
        "fixtures/ts/h264_aac_40s.ts",
    ] {
        assert_invariant(rel);
    }
}
