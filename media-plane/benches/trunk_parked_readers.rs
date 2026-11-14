//! Parked-reader benchmark for `Trunk`: 1 publisher, N readers each awaiting
//! `SampleCursor::next`, the shape production consumers use (contrast
//! `trunk_contention`, whose readers spin on `poll`). Reports publisher
//! nanoseconds per `publish` and the delay from a publish to a reader
//! receiving that sample (wake latency). Recorded numbers: `benches/RESULTS.md`.

use std::num::NonZeroUsize;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::task::{Context, Poll, Wake, Waker};
use std::time::{Duration, Instant};

use bytes::Bytes;
use media_plane::{RetentionClass, SampleCursorItem, Trunk, TrunkConfig};
use transmux::{Sample, TrackId};

const READER_COUNTS: [usize; 5] = [1, 16, 64, 256, 1024];
const PUBLISHES: usize = 2000;
const PUBLISH_INTERVAL: Duration = Duration::from_micros(500);
const TRACK: u32 = 1;
const PAYLOAD: usize = 188 * 7;
const SETTLE: Duration = Duration::from_millis(200);
const NANOS_PER_MICRO: u64 = 1_000;

struct ThreadWake(std::thread::Thread);

impl Wake for ThreadWake {
    fn wake(self: Arc<Self>) {
        self.0.unpark();
    }
}

fn nz(n: usize) -> NonZeroUsize {
    NonZeroUsize::new(n).expect("non-zero")
}

fn percentile(sorted: &[u64], p: f64) -> u64 {
    let rank = ((sorted.len() as f64 - 1.0) * p).round() as usize;
    sorted[rank]
}

fn run(readers: usize) -> (Vec<u64>, Vec<u64>) {
    let trunk = Trunk::new(
        TrunkConfig::default()
            .with_timed_capacity(nz(4096))
            .with_sparse_capacity(nz(64))
            .with_segment_capacity(nz(8))
            .with_event_capacity(nz(64))
            .with_part_capacity(nz(64)),
    );
    let writer = trunk.writer().expect("first writer");
    let base = Instant::now();
    let stop = Arc::new(AtomicBool::new(false));
    let handles: Vec<_> = (0..readers)
        .map(|_| {
            let mut cursor = trunk.subscribe();
            let stop = Arc::clone(&stop);
            std::thread::spawn(move || {
                let waker = Waker::from(Arc::new(ThreadWake(std::thread::current())));
                let mut cx = Context::from_waker(&waker);
                let mut latencies = Vec::with_capacity(PUBLISHES);
                while !stop.load(Ordering::Relaxed) {
                    let mut next = Box::pin(cursor.next());
                    let item = loop {
                        match next.as_mut().poll(&mut cx) {
                            Poll::Ready(item) => break item,
                            Poll::Pending => {
                                std::thread::park_timeout(Duration::from_millis(50));
                                if stop.load(Ordering::Relaxed) {
                                    return latencies;
                                }
                            }
                        }
                    };
                    if let SampleCursorItem::Timed { sample, .. } = item
                        && let Some(stamp) = sample.timing.and_then(|t| t.pts())
                    {
                        let now = base.elapsed().as_nanos() as i64;
                        latencies.push(u64::try_from(now - stamp).unwrap_or(0));
                    }
                }
                latencies
            })
        })
        .collect();
    std::thread::sleep(SETTLE);

    let mut publish_ns = Vec::with_capacity(PUBLISHES);
    for _ in 0..PUBLISHES {
        let stamp = base.elapsed().as_nanos() as i64;
        let sample = Sample::new(
            Bytes::from(vec![0xAB; PAYLOAD]),
            Some(stamp),
            Some(stamp),
            None,
            true,
        );
        let started = Instant::now();
        writer.publish(TrackId::new(TRACK), RetentionClass::Timed, sample);
        publish_ns.push(started.elapsed().as_nanos() as u64);
        std::thread::sleep(PUBLISH_INTERVAL);
    }
    std::thread::sleep(SETTLE);
    stop.store(true, Ordering::Relaxed);
    let mut wake_ns = Vec::new();
    for handle in handles {
        wake_ns.extend(handle.join().expect("reader thread"));
    }
    (publish_ns, wake_ns)
}

fn main() {
    println!("readers  publish_p50_ns  publish_p99_ns  wake_p50_us  wake_p99_us  deliveries");
    for readers in READER_COUNTS {
        let (mut publish_ns, mut wake_ns) = run(readers);
        publish_ns.sort_unstable();
        wake_ns.sort_unstable();
        println!(
            "{readers:>7}  {:>14}  {:>14}  {:>11}  {:>11}  {:>10}",
            percentile(&publish_ns, 0.5),
            percentile(&publish_ns, 0.99),
            percentile(&wake_ns, 0.5) / NANOS_PER_MICRO,
            percentile(&wake_ns, 0.99) / NANOS_PER_MICRO,
            wake_ns.len(),
        );
    }
}
