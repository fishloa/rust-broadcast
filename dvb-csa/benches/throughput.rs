use criterion::{Criterion, Throughput, black_box, criterion_group, criterion_main};
use dvb_csa::{ControlWord, descramble, scramble};

fn bench_scramble(c: &mut Criterion) {
    let cw = ControlWord::from_bytes([0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07, 0x08]);
    let mut data = vec![0xAAu8; 184];

    let mut group = c.benchmark_group("scramble");
    group.throughput(Throughput::Bytes(184));
    group.bench_function("184B", |b| {
        b.iter(|| {
            scramble(black_box(&cw), black_box(&mut data));
        })
    });
    group.finish();
}

fn bench_descramble(c: &mut Criterion) {
    let cw = ControlWord::from_bytes([0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07, 0x08]);
    let mut data = vec![0xAAu8; 184];
    scramble(&cw, &mut data);

    let mut group = c.benchmark_group("descramble");
    group.throughput(Throughput::Bytes(184));
    group.bench_function("184B", |b| {
        b.iter(|| {
            descramble(black_box(&cw), black_box(&mut data));
        })
    });
    group.finish();
}

criterion_group!(benches, bench_scramble, bench_descramble);
criterion_main!(benches);
