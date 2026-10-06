//! Criterion benchmarks for the Annex B start-code scan.
//!
//! Covers:
//! - `annexb_to_length_prefixed` over every H.264 access unit of a fixture —
//!   the scan on its own, as the TS → CMAF path runs it per access unit
//! - `StreamingTsDemux` over a whole fixture (feed, finish, drain events) —
//!   the TS → IR path end to end, where the scan runs while splitting access
//!   units, classifying them and converting them to length-prefixed form

use std::hint::black_box;
use std::path::PathBuf;

use criterion::{Criterion, Throughput, criterion_group, criterion_main};
use transmux::annexb::{annexb_to_length_prefixed, length_prefixed_to_annexb};
use transmux::{CodecConfig, StreamingTsDemux, TsDemux};

// ── Fixtures (shared /fixtures/ts/) ─────────────────────────────────────────

/// Real H.264 broadcast captures.
const FIXTURES: [&str; 3] = ["france2.ts", "gulli-opengop.ts", "h264_aac_40s.ts"];

fn fixture(name: &str) -> Vec<u8> {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../fixtures/ts")
        .join(name);
    std::fs::read(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()))
}

/// The fixture's H.264 access units, back in Annex B form.
fn avc_access_units(ts: &[u8]) -> Vec<Vec<u8>> {
    let media = TsDemux::new().demux(ts).expect("fixture demuxes");
    media
        .tracks
        .iter()
        .filter(|t| matches!(t.spec.config, CodecConfig::Avc { .. }))
        .flat_map(|t| &t.samples)
        .map(|s| length_prefixed_to_annexb(&s.data).expect("length-prefixed sample"))
        .collect()
}

// ── Benchmarks ──────────────────────────────────────────────────────────────

/// `annexb_to_length_prefixed` throughput over each fixture's video.
fn bench_annexb_to_length_prefixed(c: &mut Criterion) {
    let mut group = c.benchmark_group("annexb_to_length_prefixed");
    for name in FIXTURES {
        let aus = avc_access_units(&fixture(name));
        assert!(!aus.is_empty(), "{name}: no H.264 access units");
        group.throughput(Throughput::Bytes(aus.iter().map(|a| a.len() as u64).sum()));
        group.bench_function(name, |b| {
            b.iter(|| {
                let mut out = 0usize;
                for au in black_box(&aus) {
                    out += annexb_to_length_prefixed(au).len();
                }
                black_box(out)
            });
        });
    }
    group.finish();
}

/// `StreamingTsDemux` throughput over each whole fixture.
fn bench_streaming_ts_demux(c: &mut Criterion) {
    let mut group = c.benchmark_group("streaming_ts_demux");
    for name in FIXTURES {
        let ts = fixture(name);
        group.throughput(Throughput::Bytes(ts.len() as u64));
        group.bench_function(name, |b| {
            b.iter(|| {
                let mut demux = StreamingTsDemux::new();
                demux.feed(black_box(&ts));
                demux.finish();
                let mut events = 0u64;
                while let Some(e) = demux.poll_event() {
                    black_box(&e);
                    events += 1;
                }
                black_box(events)
            });
        });
    }
    group.finish();
}

criterion_group!(
    benches,
    bench_annexb_to_length_prefixed,
    bench_streaming_ts_demux
);
criterion_main!(benches);
