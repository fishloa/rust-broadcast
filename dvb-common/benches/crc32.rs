//! CRC-32 MPEG-2 throughput bench — dvb-common.
//!
//! Measures `dvb_common::crc32_mpeg2::compute` over three payload sizes that
//! bracket the section-scale payloads seen in practice:
//!
//! - 188 B  — one full MPEG-TS packet (also the PAT/PMT common case)
//! - 4096 B — a large private section
//! - 65536 B — stress / theoretical max PSI section payload

use criterion::{black_box, criterion_group, criterion_main, BenchmarkId, Criterion, Throughput};
use dvb_common::crc32_mpeg2;

fn bench_crc32(c: &mut Criterion) {
    let mut group = c.benchmark_group("crc32_mpeg2");

    for size in [188usize, 4096, 65536] {
        let data: Vec<u8> = (0..size).map(|i| i as u8).collect();
        group.throughput(Throughput::Bytes(size as u64));
        group.bench_with_input(BenchmarkId::from_parameter(size), &data, |b, d| {
            b.iter(|| crc32_mpeg2::compute(black_box(d)));
        });
    }

    group.finish();
}

criterion_group!(benches, bench_crc32);
criterion_main!(benches);
