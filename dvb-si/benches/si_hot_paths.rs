//! Criterion benchmarks for dvb-si hot paths.
//!
//! Covers:
//! - `SiDemux::feed` — packets/sec and Mbit/s on real fixture streams
//! - `AnyTableSection::parse` dispatch — on a section corpus from the m6 fixture
//! - `parse_loop` (descriptor-loop walk) — on synthetic descriptor bytes
//! - `DvbText::decode` — on representative byte sequences

use criterion::{black_box, criterion_group, criterion_main, BenchmarkId, Criterion, Throughput};

use dvb_si::demux::SiDemux;
use dvb_si::descriptors::parse_loop;
use dvb_si::tables::AnyTableSection;
use dvb_si::text::DvbText;

// ── Fixtures (vendored in dvb-si/tests/fixtures/) ─────────────────────────────

const M6_FIXTURE: &[u8] = include_bytes!("../tests/fixtures/m6-single.ts");
const TNT_FIXTURE: &[u8] = include_bytes!("../tests/fixtures/tnt-5w-12732v-isi6-10s.ts");

const TS_PACKET_SIZE: usize = 188;

// ── Helpers ───────────────────────────────────────────────────────────────────

/// Run a SiDemux over an entire fixture consuming all events; return (packets, events).
fn demux_fixture(data: &[u8]) -> (u64, u64) {
    let mut demux = SiDemux::builder().build();
    let mut events = 0u64;
    for pkt in data.chunks(TS_PACKET_SIZE) {
        if pkt.len() < TS_PACKET_SIZE {
            break;
        }
        for _ in demux.feed(pkt) {
            events += 1;
        }
    }
    let packets = (data.len() / TS_PACKET_SIZE) as u64;
    (packets, events)
}

/// Collect section bytes emitted by a full demux pass (emit_repeats=true).
fn collect_sections(data: &[u8]) -> Vec<Vec<u8>> {
    let mut demux = SiDemux::builder().emit_repeats(true).build();
    let mut sections = Vec::new();
    for pkt in data.chunks(TS_PACKET_SIZE) {
        if pkt.len() < TS_PACKET_SIZE {
            break;
        }
        for evt in demux.feed(pkt) {
            sections.push(evt.bytes().to_vec());
        }
    }
    sections
}

// ── Benchmarks ────────────────────────────────────────────────────────────────

/// `SiDemux::feed` throughput on both real fixture streams.
fn bench_demux_feed(c: &mut Criterion) {
    let mut group = c.benchmark_group("si_demux_feed");

    let fixtures: &[(&str, &[u8])] = &[("m6-single", M6_FIXTURE), ("tnt-10s", TNT_FIXTURE)];

    for (name, data) in fixtures {
        let bytes = (data.len() / TS_PACKET_SIZE * TS_PACKET_SIZE) as u64;
        group.throughput(Throughput::Bytes(bytes));
        group.bench_with_input(BenchmarkId::new("fixture", name), data, |b, data| {
            b.iter(|| demux_fixture(black_box(data)));
        });
    }

    group.finish();
}

/// `AnyTableSection::parse` dispatch over a section corpus from the m6 fixture.
fn bench_any_table_section_parse(c: &mut Criterion) {
    let sections = collect_sections(M6_FIXTURE);
    // First 200 sections (or all if fewer) — enough variety without huge iterations.
    let corpus: Vec<Vec<u8>> = sections.into_iter().take(200).collect();
    let total_bytes: u64 = corpus.iter().map(|s| s.len() as u64).sum();

    let mut group = c.benchmark_group("any_table_section_parse");
    group.throughput(Throughput::Bytes(total_bytes));
    group.bench_function("m6-corpus", |b| {
        b.iter(|| {
            for section in &corpus {
                let _ = AnyTableSection::parse(black_box(section));
            }
        });
    });
    group.finish();
}

/// `parse_loop` over a synthetic descriptor loop of realistic size.
///
/// The loop contains 10 short_event descriptors (tag 0x4D, 9 bytes each)
/// followed by 10 unknown private descriptors (tag 0xA7, 4 bytes each) — 130
/// bytes total, representative of a real EIT descriptor loop.
fn bench_parse_loop(c: &mut Criterion) {
    // short_event: tag=0x4D, len=7, lang="eng"(3), name_len=2, "Hi"(2), text_len=0
    const SHORT_EVENT: [u8; 9] = [0x4D, 0x07, b'e', b'n', b'g', 0x02, b'H', b'i', 0x00];
    // unknown private: tag=0xA7, len=2, payload=0xCA 0xFE
    const UNKNOWN: [u8; 4] = [0xA7, 0x02, 0xCA, 0xFE];

    let mut loop_bytes: Vec<u8> = Vec::new();
    for _ in 0..10 {
        loop_bytes.extend_from_slice(&SHORT_EVENT);
    }
    for _ in 0..10 {
        loop_bytes.extend_from_slice(&UNKNOWN);
    }

    let total_bytes = loop_bytes.len() as u64;
    let mut group = c.benchmark_group("parse_loop");
    group.throughput(Throughput::Bytes(total_bytes));
    group.bench_function("10x_short_event_10x_unknown", |b| {
        b.iter(|| {
            for item in parse_loop(black_box(&loop_bytes)) {
                let _ = black_box(item);
            }
        });
    });
    group.finish();
}

/// `DvbText::decode` on representative byte sequences.
///
/// Three cases:
/// - Pure ASCII (selector-less default Latin, fast path)
/// - UTF-8 with selector byte 0x15
/// - ISO 6937 with combining diacritical sequences (0xC2 = combining acute)
fn bench_dvbtext_decode(c: &mut Criterion) {
    // ASCII — no selector, all printable ASCII
    let ascii_bytes: &[u8] = b"BBC One HD";

    // UTF-8 selector 0x15 + "M6 Actualites"
    let utf8_bytes: Vec<u8> = {
        let mut v = vec![0x15u8];
        v.extend_from_slice(b"M6 Actualites");
        v
    };

    // ISO 6937: combining acute (0xC2) + 'e' → é, repeated
    let iso6937_bytes: Vec<u8> = {
        // "café" x5 in ISO 6937: c-a-f-combining_acute-e
        let mut v: Vec<u8> = Vec::new();
        for _ in 0..5 {
            v.extend_from_slice(&[b'c', b'a', b'f', 0xC2, b'e']);
        }
        v
    };

    let mut group = c.benchmark_group("dvbtext_decode");

    group.bench_function("ascii", |b| {
        b.iter(|| {
            let t = DvbText::new(black_box(ascii_bytes));
            black_box(t.decode())
        });
    });

    group.bench_function("utf8_selector", |b| {
        b.iter(|| {
            let t = DvbText::new(black_box(utf8_bytes.as_slice()));
            black_box(t.decode())
        });
    });

    group.bench_function("iso6937_combining", |b| {
        b.iter(|| {
            let t = DvbText::new(black_box(iso6937_bytes.as_slice()));
            black_box(t.decode())
        });
    });

    group.finish();
}

criterion_group!(
    benches,
    bench_demux_feed,
    bench_any_table_section_parse,
    bench_parse_loop,
    bench_dvbtext_decode,
);
criterion_main!(benches);
