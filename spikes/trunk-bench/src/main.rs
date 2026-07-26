//! trunk-bench: throwaway spike measuring the Trunk fan-out premise from
//! docs/superpowers/specs/2026-07-26-media-plane-architecture.md §1.2 / §7.
//!
//! NOT production code. Naive shape only: ONE `Mutex`-guarded shared log
//! (not per-track sharded locks, not ArcSwap, not lock-free) fed by a single
//! writer thread, read by N reader cursors via `Bytes` clones (refcount bump,
//! no payload copy). This is the shape the spec's `Trunk`/`SampleCursor` API
//! implies before any optimisation. Do NOT hand-tune this file to make case 2
//! pass -- the whole point is an honest number for the naive shape first.

use bytes::Bytes;
use std::alloc::{GlobalAlloc, Layout, System};
use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

// ---------------------------------------------------------------------
// Counting allocator (case 5: "count allocations if practical")
// ---------------------------------------------------------------------
struct CountingAlloc;
static ALLOC_COUNT: AtomicUsize = AtomicUsize::new(0);
static ALLOC_BYTES: AtomicUsize = AtomicUsize::new(0);

unsafe impl GlobalAlloc for CountingAlloc {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        ALLOC_COUNT.fetch_add(1, Ordering::Relaxed);
        ALLOC_BYTES.fetch_add(layout.size(), Ordering::Relaxed);
        unsafe { System.alloc(layout) }
    }
    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        unsafe { System.dealloc(ptr, layout) }
    }
}

#[global_allocator]
static GLOBAL: CountingAlloc = CountingAlloc;

fn alloc_snapshot() -> (usize, usize) {
    (
        ALLOC_COUNT.load(Ordering::Relaxed),
        ALLOC_BYTES.load(Ordering::Relaxed),
    )
}

// ---------------------------------------------------------------------
// The naive Trunk: single Mutex<VecDeque<Sample>> shared log, one writer,
// N cursors. This mirrors spec §1.2's Trunk/SampleCursor shape directly.
// ---------------------------------------------------------------------
#[derive(Clone)]
struct Sample {
    track: u32,
    seq: u64,
    data: Bytes,
    dts: Option<i64>,
    pts: Option<i64>,
    duration: Option<u32>,
    flags: u8,
}

struct TrunkInner {
    log: VecDeque<Sample>,
    next_seq: u64,
    base_seq: u64, // seq of log[0]; entries below this have been evicted
    cap: usize,
}

struct Trunk {
    inner: Mutex<TrunkInner>,
    version: AtomicU64,
}

impl Trunk {
    fn new(cap: usize) -> Self {
        Trunk {
            inner: Mutex::new(TrunkInner {
                log: VecDeque::with_capacity(cap),
                next_seq: 0,
                base_seq: 0,
                cap,
            }),
            version: AtomicU64::new(0),
        }
    }

    /// Single writer, must never block on readers. Returns the lock+push
    /// latency (excludes any writer-side pacing/sleep -- that's harness
    /// scheduling, not Trunk cost).
    fn publish(&self, s: Sample) -> Duration {
        let t0 = Instant::now();
        {
            let mut inner = self.inner.lock().unwrap();
            if inner.log.len() == inner.cap {
                inner.log.pop_front();
                inner.base_seq += 1;
            }
            inner.log.push_back(s);
        }
        self.version.fetch_add(1, Ordering::Release);
        t0.elapsed()
    }

    fn subscribe(self: &Arc<Self>) -> Cursor {
        let pos = self.inner.lock().unwrap().next_seq;
        Cursor {
            trunk: Arc::clone(self),
            read_seq: pos,
            lagged: 0,
        }
    }
}

struct Cursor {
    trunk: Arc<Trunk>,
    read_seq: u64,
    lagged: u64,
}

enum PollResult {
    Empty,
    Got(usize),
}

impl Cursor {
    fn poll(&mut self, out: &mut Vec<Sample>, budget: usize) -> PollResult {
        let inner = self.trunk.inner.lock().unwrap();
        if self.read_seq < inner.base_seq {
            self.lagged += inner.base_seq - self.read_seq;
            self.read_seq = inner.base_seq;
        }
        let start = (self.read_seq - inner.base_seq) as usize;
        let end = std::cmp::min(start + budget, inner.log.len());
        if start >= end {
            return PollResult::Empty;
        }
        out.reserve(end - start);
        for i in start..end {
            // Sample::clone() clones a Bytes handle (Arc-style refcount bump)
            // plus a few Copy/Option scalars -- no payload byte copy.
            out.push(inner.log[i].clone());
        }
        self.read_seq += (end - start) as u64;
        PollResult::Got(end - start)
    }
}

// ---------------------------------------------------------------------
// Workload model
// ---------------------------------------------------------------------
#[derive(Clone, Copy)]
struct TrackSpec {
    rate_hz: f64,
    sample_size: usize,
}

/// 200-track MPTS: ~40 services x ~5 ES, modelled as 40 "video" ES
/// (25 fps, larger frames) + 160 "audio/data" ES (50 Hz, small frames),
/// weighted 80/20 by bitrate share of a ~1 Gbit/s aggregate.
fn mpts_200_tracks() -> Vec<TrackSpec> {
    let mut v = Vec::with_capacity(200);
    let target_bps = 1_000_000_000f64 / 8.0; // bytes/s aggregate
    let video_share = 0.8 * target_bps;
    let audio_share = 0.2 * target_bps;
    let video_hz = 25.0;
    let audio_hz = 50.0;
    let video_size = (video_share / 40.0 / video_hz) as usize; // per-track
    let audio_size = (audio_share / 160.0 / audio_hz) as usize;
    for _ in 0..40 {
        v.push(TrackSpec {
            rate_hz: video_hz,
            sample_size: video_size,
        });
    }
    for _ in 0..160 {
        v.push(TrackSpec {
            rate_hz: audio_hz,
            sample_size: audio_size,
        });
    }
    v
}

fn baseline_2_tracks() -> Vec<TrackSpec> {
    vec![
        TrackSpec {
            rate_hz: 60.0,
            sample_size: 1_000_000 / 60,
        }, // video ~1MB/s @60fps
        TrackSpec {
            rate_hz: 43.0,
            sample_size: 188,
        }, // audio @43fps, small frames
    ]
}

/// Build a time-ordered schedule of (time_s, track_id, size) for `duration_s`
/// seconds given a track mix.
fn build_schedule(tracks: &[TrackSpec], duration_s: f64) -> Vec<(f64, u32, usize)> {
    let mut sched = Vec::new();
    for (tid, t) in tracks.iter().enumerate() {
        let period = 1.0 / t.rate_hz;
        let mut time = 0.0;
        while time < duration_s {
            sched.push((time, tid as u32, t.sample_size));
            time += period;
        }
    }
    sched.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap());
    sched
}

struct ReaderStats {
    received: u64,
    lagged: u64,
    max_gap: Duration,
}

/// Drives one writer thread (paced to wall clock per `schedule`) and
/// `num_readers` reader threads against a fresh Trunk. `reader_sleep`, if
/// Some, makes readers "lazy" (sleep between polls) instead of busy-polling.
fn run_case(
    schedule: &[(f64, u32, usize)],
    cap: usize,
    num_readers: usize,
    reader_sleep: Option<Duration>,
) -> (Vec<Duration>, Vec<ReaderStats>, Duration, u64, u64) {
    let trunk = Arc::new(Trunk::new(cap));
    let done = Arc::new(AtomicBool::new(false));

    std::thread::scope(|scope| {
        let mut readers = Vec::new();
        for _ in 0..num_readers {
            let trunk = Arc::clone(&trunk);
            let done = Arc::clone(&done);
            readers.push(scope.spawn(move || {
                let mut cursor = trunk.subscribe();
                let mut received = 0u64;
                let mut out = Vec::new();
                let mut max_gap = Duration::ZERO;
                let mut last_recv = Instant::now();
                loop {
                    out.clear();
                    match cursor.poll(&mut out, 256) {
                        PollResult::Got(n) => {
                            received += n as u64;
                            let now = Instant::now();
                            let gap = now.duration_since(last_recv);
                            if gap > max_gap {
                                max_gap = gap;
                            }
                            last_recv = now;
                        }
                        PollResult::Empty => {
                            if done.load(Ordering::Relaxed) {
                                // one more drain attempt then exit
                                out.clear();
                                if let PollResult::Got(n) = cursor.poll(&mut out, 256) {
                                    received += n as u64;
                                } else {
                                    break;
                                }
                                continue;
                            }
                            match reader_sleep {
                                Some(d) => std::thread::sleep(d),
                                None => std::thread::yield_now(),
                            }
                        }
                    }
                }
                ReaderStats {
                    received,
                    lagged: cursor.lagged,
                    max_gap,
                }
            }));
        }

        let latencies = {
            let trunk = Arc::clone(&trunk);
            let start = Instant::now();
            let mut latencies = Vec::with_capacity(schedule.len());
            let mut published_bytes = 0u64;
            for (t, tid, size) in schedule.iter().copied() {
                let deadline = start + Duration::from_secs_f64(t);
                let now = Instant::now();
                if now < deadline {
                    std::thread::sleep(deadline - now);
                }
                let data = Bytes::from(vec![0xABu8; size]);
                published_bytes += size as u64;
                let lat = trunk.publish(Sample {
                    track: tid,
                    seq: 0,
                    data,
                    dts: Some(0),
                    pts: Some(0),
                    duration: None,
                    flags: 0,
                });
                latencies.push(lat);
            }
            let elapsed = start.elapsed();
            done.store(true, Ordering::Relaxed);
            (latencies, elapsed, published_bytes)
        };

        let reader_stats: Vec<ReaderStats> = readers.into_iter().map(|h| h.join().unwrap()).collect();
        (
            latencies.0,
            reader_stats,
            latencies.1,
            schedule.len() as u64,
            latencies.2,
        )
    })
}

fn percentile(sorted: &[Duration], p: f64) -> Duration {
    if sorted.is_empty() {
        return Duration::ZERO;
    }
    let idx = ((sorted.len() as f64 - 1.0) * p).round() as usize;
    sorted[idx]
}

fn report_latencies(label: &str, mut latencies: Vec<Duration>) {
    latencies.sort();
    let p50 = percentile(&latencies, 0.50);
    let p99 = percentile(&latencies, 0.99);
    let max = *latencies.last().unwrap_or(&Duration::ZERO);
    let mean: Duration = if latencies.is_empty() {
        Duration::ZERO
    } else {
        latencies.iter().sum::<Duration>() / latencies.len() as u32
    };
    println!(
        "{label}: n={} mean={:?} p50={:?} p99={:?} max={:?}",
        latencies.len(),
        mean,
        p50,
        p99,
        max
    );
}

fn main() {
    println!("=== trunk-bench: Trunk fan-out spike ===");
    println!(
        "cpu: {}  logical_cpus: {}",
        std::env::var("TB_CPU_LABEL").unwrap_or_else(|_| "see sysctl output in report".into()),
        std::thread::available_parallelism()
            .map(|n| n.get())
            .unwrap_or(0)
    );

    // ---------------- Case 1: baseline sanity ----------------
    {
        println!("\n--- Case 1: baseline sanity (2 tracks, 1 reader) ---");
        let tracks = baseline_2_tracks();
        let sched = build_schedule(&tracks, 5.0);
        let (lat, readers, elapsed, published, bytes) = run_case(&sched, 10_000, 1, None);
        println!(
            "published={} in {:?} ({:.1} samples/s, {:.2} Mbit/s)",
            published,
            elapsed,
            published as f64 / elapsed.as_secs_f64(),
            (bytes as f64 * 8.0 / elapsed.as_secs_f64()) / 1e6
        );
        report_latencies("publish latency", lat);
        for (i, r) in readers.iter().enumerate() {
            println!(
                "reader[{i}]: received={} lagged={} max_gap={:?}",
                r.received, r.lagged, r.max_gap
            );
        }
    }

    // ---------------- Case 2: 200 tracks x 6 readers ----------------
    let case2_lat_for_report;
    {
        println!("\n--- Case 2: 200-track MPTS x 6 readers (~1 Gbit/s aggregate) ---");
        let tracks = mpts_200_tracks();
        let sched = build_schedule(&tracks, 5.0);
        println!("schedule events: {}", sched.len());
        let (lat, readers, elapsed, published, bytes) = run_case(&sched, 20_000, 6, None);
        println!(
            "published={} in {:?} ({:.1} samples/s, {:.2} Mbit/s)",
            published,
            elapsed,
            published as f64 / elapsed.as_secs_f64(),
            (bytes as f64 * 8.0 / elapsed.as_secs_f64()) / 1e6
        );
        report_latencies("publish latency", lat.clone());
        case2_lat_for_report = lat;
        let mut any_starved = false;
        for (i, r) in readers.iter().enumerate() {
            let starved = r.lagged > 0 || r.received < published;
            any_starved |= starved;
            println!(
                "reader[{i}]: received={} (expected {}) lagged={} max_gap={:?} starved={}",
                r.received, published, r.lagged, r.max_gap, starved
            );
        }
        println!("any reader starved/lagged: {any_starved}");
    }
    let _ = case2_lat_for_report;

    // ---------------- Case 3: fan-out scaling ----------------
    {
        println!("\n--- Case 3: fan-out scaling (200-track mix, readers 1/2/4/8/16) ---");
        let tracks = mpts_200_tracks();
        let sched = build_schedule(&tracks, 3.0);
        for &n in &[1usize, 2, 4, 8, 16] {
            let (lat, readers, elapsed, published, _bytes) = run_case(&sched, 20_000, n, None);
            let mut sorted = lat.clone();
            sorted.sort();
            let mean: Duration = sorted.iter().sum::<Duration>() / sorted.len() as u32;
            let p99 = percentile(&sorted, 0.99);
            let any_lag = readers.iter().any(|r| r.lagged > 0);
            println!(
                "readers={n:>2}: published={published} elapsed={:?} mean_publish={:?} p99_publish={:?} any_reader_lagged={any_lag}",
                elapsed, mean, p99
            );
        }
    }

    // ---------------- Case 4: contention shape ----------------
    {
        println!("\n--- Case 4: contention shape (200-track x 6 readers, aggressive vs lazy) ---");
        let tracks = mpts_200_tracks();
        let sched = build_schedule(&tracks, 4.0);

        let (lat_a, readers_a, elapsed_a, published_a, _) = run_case(&sched, 20_000, 6, None);
        println!("aggressive (busy-poll, yield_now):");
        println!(
            "  published={published_a} elapsed={:?} ({:.1} samples/s)",
            elapsed_a,
            published_a as f64 / elapsed_a.as_secs_f64()
        );
        report_latencies("  publish latency", lat_a);
        for (i, r) in readers_a.iter().enumerate() {
            println!(
                "  reader[{i}]: received={} lagged={} max_gap={:?}",
                r.received, r.lagged, r.max_gap
            );
        }

        let (lat_l, readers_l, elapsed_l, published_l, _) =
            run_case(&sched, 20_000, 6, Some(Duration::from_millis(5)));
        println!("lazy (sleep 5ms between polls):");
        println!(
            "  published={published_l} elapsed={:?} ({:.1} samples/s)",
            elapsed_l,
            published_l as f64 / elapsed_l.as_secs_f64()
        );
        report_latencies("  publish latency", lat_l);
        for (i, r) in readers_l.iter().enumerate() {
            println!(
                "  reader[{i}]: received={} lagged={} max_gap={:?}",
                r.received, r.lagged, r.max_gap
            );
        }
    }

    // ---------------- Case 5: the Bytes claim ----------------
    {
        println!("\n--- Case 5: Bytes fan-out is refcount-only, no payload copy ---");
        let trunk = Arc::new(Trunk::new(16));
        let mut c1 = trunk.subscribe();
        let mut c2 = trunk.subscribe();
        let mut c3 = trunk.subscribe();

        let (pre_count, pre_bytes) = alloc_snapshot();
        let payload = Bytes::from(vec![0x42u8; 65536]); // one 64KiB "coded sample"
        let (after_alloc_count, after_alloc_bytes) = alloc_snapshot();
        trunk.publish(Sample {
            track: 0,
            seq: 0,
            data: payload,
            dts: Some(1),
            pts: Some(1),
            duration: None,
            flags: 0,
        });

        let (pre_read_count, pre_read_bytes) = alloc_snapshot();
        let mut o1 = Vec::new();
        let mut o2 = Vec::new();
        let mut o3 = Vec::new();
        c1.poll(&mut o1, 8);
        c2.poll(&mut o2, 8);
        c3.poll(&mut o3, 8);
        let (post_read_count, post_read_bytes) = alloc_snapshot();

        let p1 = o1[0].data.as_ptr();
        let p2 = o2[0].data.as_ptr();
        let p3 = o3[0].data.as_ptr();
        println!("payload ptr reader1={:p} reader2={:p} reader3={:p}", p1, p2, p3);
        println!("same backing pointer across all 3 readers: {}", p1 == p2 && p2 == p3);
        println!(
            "allocator: publish-path added {} allocs / {} bytes",
            after_alloc_count - pre_count,
            after_alloc_bytes - pre_bytes
        );
        println!(
            "allocator: 3-reader poll added {} allocs / {} bytes (should be small, no 64KiB copies)",
            post_read_count - pre_read_count,
            post_read_bytes - pre_read_bytes
        );
    }

    println!("\n=== done ===");
}
