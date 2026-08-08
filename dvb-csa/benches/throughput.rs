//! Throughput benchmarks.
//!
//! `single` measures one 184-byte TS payload at a time — the shape a demuxer
//! sees. `batch` measures a full batch of 64 such payloads, which is the unit
//! the bitsliced path works in: it runs the same work through the scalar path
//! and (with `--features bitsliced`) through the batch path, so the two numbers
//! are directly comparable per byte.
use criterion::{Criterion, Throughput, black_box, criterion_group, criterion_main};
use dvb_csa::{ControlWord, descramble, scramble};

/// A full TS payload with no adaptation field.
const PAYLOAD: usize = 184;
/// Payloads per batch — the bitsliced slicing width.
const BATCH: usize = 64;

fn cw() -> ControlWord {
    ControlWord::from_bytes([0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07, 0x08])
}

/// Distinct, non-degenerate payloads — one buffer repeated would let the
/// branch predictor and the caches flatter the scalar path.
fn payloads() -> Vec<Vec<u8>> {
    (0..BATCH)
        .map(|i| (0..PAYLOAD).map(|j| (i * 31 + j * 17) as u8).collect())
        .collect()
}

fn bench_single(c: &mut Criterion) {
    let cw = cw();
    let mut group = c.benchmark_group("single");
    group.throughput(Throughput::Bytes(PAYLOAD as u64));

    let mut data = vec![0xAAu8; PAYLOAD];
    group.bench_function("scramble/scalar", |b| {
        b.iter(|| scramble(black_box(&cw), black_box(&mut data)))
    });

    let mut data = vec![0xAAu8; PAYLOAD];
    scramble(&cw, &mut data);
    group.bench_function("descramble/scalar", |b| {
        b.iter(|| descramble(black_box(&cw), black_box(&mut data)))
    });

    group.finish();
}

fn bench_batch(c: &mut Criterion) {
    let cw = cw();
    let mut group = c.benchmark_group("batch");
    group.throughput(Throughput::Bytes((PAYLOAD * BATCH) as u64));

    let mut data = payloads();
    group.bench_function("scramble/scalar", |b| {
        b.iter(|| {
            for p in data.iter_mut() {
                scramble(black_box(&cw), black_box(p));
            }
        })
    });

    let mut data = payloads();
    group.bench_function("descramble/scalar", |b| {
        b.iter(|| {
            for p in data.iter_mut() {
                descramble(black_box(&cw), black_box(p));
            }
        })
    });

    #[cfg(feature = "bitsliced")]
    {
        use dvb_csa::bitsliced::{descramble_batch, scramble_batch};

        let mut data = payloads();
        group.bench_function("scramble/bitsliced", |b| {
            b.iter(|| {
                let mut refs: Vec<&mut [u8]> = data.iter_mut().map(|p| p.as_mut_slice()).collect();
                scramble_batch(black_box(&cw), black_box(&mut refs));
            })
        });

        let mut data = payloads();
        group.bench_function("descramble/bitsliced", |b| {
            b.iter(|| {
                let mut refs: Vec<&mut [u8]> = data.iter_mut().map(|p| p.as_mut_slice()).collect();
                descramble_batch(black_box(&cw), black_box(&mut refs));
            })
        });
    }

    group.finish();
}

criterion_group!(benches, bench_single, bench_batch);
criterion_main!(benches);
