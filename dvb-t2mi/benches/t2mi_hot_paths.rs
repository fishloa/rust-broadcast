//! Criterion benchmarks for dvb-t2mi hot paths.
//!
//! Covers:
//! - `T2miPump::feed_ts` — packets/sec and Mbit/s over the Colombia T2-MI fixture

#![cfg(feature = "ts")]

use criterion::{black_box, criterion_group, criterion_main, Criterion, Throughput};
use dvb_t2mi::pump::T2miPump;

// ── Fixture (vendored in dvb-t2mi/tests/fixtures/) ────────────────────────────

const COLOMBIA_FIXTURE: &[u8] = include_bytes!("../tests/fixtures/colombia-capital-t2mi.ts");

/// PID carrying T2-MI in the Colombia fixture (from the PMT).
const T2MI_PID: u16 = 0x0040;

const TS_PACKET_SIZE: usize = 188;

// ── Benchmark ────────────────────────────────────────────────────────────────

/// `T2miPump::feed_ts` throughput over the Colombia T2-MI fixture.
fn bench_t2mi_pump_feed(c: &mut Criterion) {
    let data = COLOMBIA_FIXTURE;
    let bytes = (data.len() / TS_PACKET_SIZE * TS_PACKET_SIZE) as u64;

    let mut group = c.benchmark_group("t2mi_pump_feed_ts");
    group.throughput(Throughput::Bytes(bytes));

    group.bench_function("colombia-t2mi", |b| {
        b.iter(|| {
            let mut pump = T2miPump::new(T2MI_PID);
            let mut events = 0u64;
            for pkt in black_box(data).chunks(TS_PACKET_SIZE) {
                if pkt.len() < TS_PACKET_SIZE {
                    break;
                }
                for _ in pump.feed_ts(pkt) {
                    events += 1;
                }
            }
            black_box(events)
        });
    });

    group.finish();
}

criterion_group!(benches, bench_t2mi_pump_feed);
criterion_main!(benches);
