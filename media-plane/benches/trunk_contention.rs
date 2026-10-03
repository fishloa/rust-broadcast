//! Contention benchmark for `Trunk`'s single `Mutex<TrunkState>`
//! (de-hand-roll W1-P, SP6.4): 1 publisher, N spinning `SampleCursor`s.
//! Metric: publisher nanoseconds per `publish`. Readers spin on `poll()` —
//! the worst case; real readers park on `listen()`. Decision rule and
//! recorded numbers: `benches/RESULTS.md`.

use std::num::NonZeroUsize;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use bytes::Bytes;
use criterion::{BenchmarkId, Criterion, criterion_group, criterion_main};
use media_plane::{RetentionClass, Trunk, TrunkConfig};
use transmux::Sample;

const READER_COUNTS: [usize; 4] = [1, 4, 16, 64];
const TRACK: u32 = 1;
const PAYLOAD: usize = 188 * 7;

fn nz(n: usize) -> NonZeroUsize {
    NonZeroUsize::new(n).expect("non-zero")
}

fn publish_with_readers(readers: usize, iters: u64) -> Duration {
    let trunk = Trunk::new(TrunkConfig::new(nz(4096), nz(64), nz(8), nz(64), nz(64)));
    let writer = trunk.writer().expect("first writer");
    let stop = Arc::new(AtomicBool::new(false));
    let handles: Vec<_> = (0..readers)
        .map(|_| {
            let mut cursor = trunk.subscribe();
            let stop = Arc::clone(&stop);
            std::thread::spawn(move || {
                while !stop.load(Ordering::Relaxed) {
                    if cursor.poll().is_none() {
                        std::hint::spin_loop();
                    }
                }
            })
        })
        .collect();
    let sample = Sample::new(
        Bytes::from(vec![0xAB; PAYLOAD]),
        Some(0),
        Some(0),
        None,
        true,
    );
    let start = Instant::now();
    for _ in 0..iters {
        writer.publish(TRACK, RetentionClass::Timed, sample.clone());
    }
    let elapsed = start.elapsed();
    stop.store(true, Ordering::Relaxed);
    for h in handles {
        h.join().expect("reader thread");
    }
    elapsed
}

fn bench(c: &mut Criterion) {
    let mut group = c.benchmark_group("trunk_publish_vs_readers");
    group
        .sample_size(20)
        .measurement_time(Duration::from_secs(5));
    for readers in READER_COUNTS {
        group.bench_with_input(BenchmarkId::from_parameter(readers), &readers, |b, &n| {
            b.iter_custom(|iters| publish_with_readers(n, iters));
        });
    }
    group.finish();
}

criterion_group!(benches, bench);
criterion_main!(benches);
