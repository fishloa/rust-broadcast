// SPDX-License-Identifier: MIT OR Apache-2.0
//! Issue #1171 — damaged TS input must never make the demuxer hand a consumer
//! a codec config whose serialization fails (and so panics `to_bytes()`).
//!
//! Original repro (zenith-tv, orf1 720p50 H.264): keep the first third intact,
//! then XOR every TS payload byte except the first 32 after the PES start in
//! PUSI packets (framing intact, NAL bodies scrambled). A `TrackUpdated`
//! arrived carrying an `avcC` with an 85 704-byte "SPS" (> the u16 length
//! field of ISO/IEC 14496-15 §5.3.3.1.2), and `config.to_bytes()` panicked.
//!
//! This file applies the same recipe (own implementation) to **committed**
//! fixtures — H.264, HEVC, AAC, AC-3/E-AC-3 — across seeds, damage start
//! offsets and two damage modes, feeds the result through `TsDemux` and
//! `StreamingTsDemux` at several chunk sizes, and asserts: no panic, every
//! emitted config serializes via both `to_bytes()` and `try_to_bytes()`, and
//! the full event sequence is chunk-size invariant.

use std::path::PathBuf;

use broadcast_common::{Serialize, Unpackage};
use transmux::{CodecConfig, DemuxEvent, StreamingTsDemux, TsDemux};

const TS_PACKET_SIZE: usize = 188;
const TS_HEADER_SIZE: usize = 4;
/// Bytes after the PES start left intact in PUSI packets (covers the PES
/// header so framing survives — the recipe from the issue).
const KEEP_AFTER_PES_START: usize = 32;

fn fixture(rel: &str) -> Vec<u8> {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join(rel);
    std::fs::read(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()))
}

struct XorShift(u64);
impl XorShift {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 >> 12;
        self.0 ^= self.0 << 25;
        self.0 ^= self.0 >> 27;
        self.0.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }
}

#[derive(Clone, Copy, Debug)]
enum Mode {
    /// XOR every damaged byte with one constant (the issue's recipe).
    Constant,
    /// XOR with a pseudo-random byte stream.
    Noise,
}

/// Damage `ts` from `start_permille / 1000` of its length onwards.
fn garble(ts: &[u8], start_permille: usize, seed: u64, mode: Mode) -> Vec<u8> {
    let mut out = ts.to_vec();
    let packets = out.len() / TS_PACKET_SIZE;
    let first = packets * start_permille / 1000;
    let mut rng = XorShift(0x9E37_79B9_7F4A_7C15 ^ seed.wrapping_mul(0xD1B5_4A32_D192_ED03));
    let constant = (rng.next() as u8) | 1;
    for p in first..packets {
        let pkt = &mut out[p * TS_PACKET_SIZE..(p + 1) * TS_PACKET_SIZE];
        let pusi = pkt[1] & 0x40 != 0;
        let afc = (pkt[3] >> 4) & 3;
        if afc & 1 == 0 {
            continue;
        }
        let mut off = TS_HEADER_SIZE;
        if afc & 2 != 0 {
            off += 1 + pkt[4] as usize;
        }
        if pusi {
            off += KEEP_AFTER_PES_START;
        }
        for b in pkt.iter_mut().skip(off) {
            *b ^= match mode {
                Mode::Constant => constant,
                Mode::Noise => (rng.next() as u8) | 1,
            };
        }
    }
    out
}

/// Serialize every serializable box a `CodecConfig` carries, both ways.
fn assert_serializable(cfg: &CodecConfig, what: &str) {
    macro_rules! both {
        ($b:expr) => {{
            let b = $b;
            let tb = b
                .try_to_bytes()
                .unwrap_or_else(|e| panic!("{what}: try_to_bytes failed: {e:?}"));
            assert_eq!(b.to_bytes(), tb, "{what}: to_bytes != try_to_bytes");
        }};
    }
    match cfg {
        CodecConfig::Avc { config, .. } => both!(config),
        CodecConfig::Hevc { config, .. } => both!(config),
        CodecConfig::Vvc { config, .. } => both!(config),
        CodecConfig::Aac { esds, .. } => both!(esds),
        CodecConfig::Ac3 { config, .. } => both!(config),
        CodecConfig::Eac3 { config, .. } => both!(config),
        CodecConfig::Av1 { config, .. } => both!(config),
        CodecConfig::Vp9 { config, .. } => both!(config),
        CodecConfig::Opus { config, .. } => both!(config),
        CodecConfig::Flac { config, .. } => both!(config),
        CodecConfig::Ac4 { config, .. } => both!(config),
        CodecConfig::MpegH { config, .. } => both!(config),
        CodecConfig::Mpeg2Video { esds, .. } => both!(esds),
        CodecConfig::MpegAudio { esds, .. } => both!(esds),
        CodecConfig::Dts { config, .. } => both!(config),
        _ => {}
    }
}

fn summarize(ev: &DemuxEvent, what: &str) -> String {
    match ev {
        DemuxEvent::TrackAdded(spec) | DemuxEvent::TrackUpdated(spec) => {
            assert_serializable(&spec.config, what);
            format!("{ev:?}")
        }
        DemuxEvent::Sample {
            track_id, sample, ..
        } => format!(
            "Sample {track_id} {:?} {:?} {}",
            sample.pts,
            sample.dts,
            sample.data.len()
        ),
        other => format!("{other:?}"),
    }
}

fn run(ts: &[u8], chunk: usize, what: &str) -> Vec<String> {
    let mut d = StreamingTsDemux::new();
    let mut out = Vec::new();
    for c in ts.chunks(chunk) {
        d.feed(c);
        while let Some(ev) = d.poll_event() {
            out.push(summarize(&ev, what));
        }
    }
    d.finish();
    while let Some(ev) = d.poll_event() {
        out.push(summarize(&ev, what));
    }
    out
}

const FIXTURES: &[&str] = &[
    "fixtures/ts/h264_aac.ts",
    "fixtures/ts/h264_aac_40s.ts",
    "fixtures/ts/h264/main.ts",
    "fixtures/ts/hevc/main.ts",
    "fixtures/ts/hevc/main10.ts",
    "fixtures/ts/aac-5_1-640k.ts",
    "fixtures/ts/gulli-opengop.ts",
    "fixtures/ts/france2.ts",
    "fixtures/ts/m6-single.ts",
    "fixtures/ts/h264/high_1080_cropped.ts",
    "transmux/tests/fixtures/ts/h264-two-resolutions.ts",
    "transmux/tests/fixtures/sample_aes_eac3/clear.ts",
    "transmux/tests/fixtures/audio_pes_straddle/sintel-ac3-eac3-aligned.ts",
];

#[test]
fn garbled_fixtures_never_yield_unserializable_configs() {
    let mut garbled_runs = 0usize;
    let mut config_events = 0usize;
    for rel in FIXTURES {
        let clean = fixture(rel);
        for start in [0usize, 333, 500, 900] {
            for seed in 1..=3u64 {
                for mode in [Mode::Constant, Mode::Noise] {
                    let ts = garble(&clean, start, seed, mode);
                    let what = format!("{rel} start={start} seed={seed} {mode:?}");
                    let baseline = run(&ts, ts.len(), &what);
                    config_events += baseline
                        .iter()
                        .filter(|e| e.starts_with("TrackAdded") || e.starts_with("TrackUpdated"))
                        .count();
                    for n in [1usize, 7, 187, 188, 189, 1000] {
                        assert_eq!(run(&ts, n, &what), baseline, "{what}: chunk {n}");
                    }
                    // The whole-buffer batch API must not panic either, and
                    // every track it reports must serialize.
                    let media = TsDemux::new().unpackage(&ts[..]).expect("demux");
                    for t in &media.tracks {
                        assert_serializable(&t.spec.config, &what);
                    }
                    garbled_runs += 1;
                }
            }
        }
    }
    assert_eq!(garbled_runs, FIXTURES.len() * 4 * 3 * 2);
    assert!(
        config_events > 0,
        "no config events at all: test is vacuous"
    );
}
