# De-hand-roll W1-P — media-doctor, media-plane, dvb-ci-runtime Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Replace media-doctor's hand-rolled metrics HTTP server, Prometheus renderer and UDP bind with `metrics` + `metrics-exporter-prometheus` + `hyper` + `socket2`; measure `media-plane`'s `Trunk` lock contention before touching it and move to `parking_lot`; swap `dvb-ci-runtime`'s `libc::poll` for `rustix::event::poll`; remove sleep-based waits and reserve-then-rebind ports from these crates' tests.

**Architecture:** One branch `w1/p` in worktree `.worktree/w1-p`, one commit per task. Three independent crate groups, executed in this order: (1) media-plane (benchmark first, then `parking_lot`, then a measured decision on a lock split); (2) dvb-ci-runtime (Linux-only, verified through the Docker recipe and Linux cross-clippy); (3) media-doctor (goldens first, an additive snapshot/exposition layer proven equivalent to the old renderer, then the server, UDP bind and binary rewrite). `render_prometheus` is deleted only after the new path has been proven semantically equal to it on the real fixture.

**Tech Stack:** Rust 1.95 workspace, cargo `--locked`; `metrics` 0.24.6 + `metrics-exporter-prometheus` 0.18.3 (both already locked; `default-features = false`, recorder + `render()` only); `hyper` 1.11 + `hyper-util` 0.1.20 + `http-body-util` 0.1 + `tokio` 1.53 + `tokio-util` 0.7.19 (all already locked); `socket2` 0.6.5 (locked); `parking_lot` 0.12.5 (locked); `criterion` 0.8.2 (locked); NEW to the lock: `rustix` 1.1.5 (MSRV 1.65, checked in its `Cargo.toml`) and `wait-timeout` 0.2.1.

**Spec:** `docs/superpowers/specs/2026-10-03-protocol-runtime-dehandroll-design.md` — §7 W1 row **P** = SP2.3 (media-doctor metrics), SP1.6 (media-doctor UDP bind), SP6.4 / SP6.6 (media-plane `Trunk`, `parking_lot`), SP6.7 (`rustix` poll), SP7 for these crates, §5 guards, §8 versioning.

## Global Constraints

- MSRV is **1.95.0**. `Cargo.lock` is committed; always build and test with `--locked`. A dependency add or bump may change only the intended lock entries; restore anything else with `cargo update -p <pkg> --precise <old>`. (Adding a *new* dependency needs one unlocked build to write the lock entry — Task steps say where; follow it with the lock-diff check.)
- Every new or bumped crate supports MSRV 1.95: `tokio-util` 1.71, `socket2` 1.70, `parking_lot` 1.71, `wait-timeout`, `rustix` 1.65 (verified), `metrics-exporter-prometheus` 1.71.1.
- No Co-Authored-By or Claude-Session trailers on commits.
- Nothing is tagged or published without the owner's explicit sign-off.
- Epoch purity: if a bumped dependency's types appear in a crate's public API, that crate takes a major-class version change. Each wave records this in `.delegate/release-versions.txt` (the orchestrator updates it; this plan writes the notes to `.delegate/w1-p-report.md`). No new dependency in this plan appears in a public type signature (see the version notes in the last task).
- Wire and serialised output is byte-identical to the pre-change goldens, except for differences listed in the CHANGELOG with an example each. (Here: the `/metrics` exposition text and the HTTP response head — both goldened in Task 1.)
- Every fixed defect has a regression test shown to fail against the old code (revert-check evidence recorded). **Spec §3 numbers no defect to cluster P**; the plan still revert-checks every behaviour it newly guarantees (slow-loris deadline, concurrency cap, final-datagram flush, stale per-PID series, StallIngest wake, poll timeout).
- No new test relies on wall-clock sleeps; every test touched in SP7 passes 20 consecutive runs.
- Never run two cargo commands concurrently (lock contention). Never `git add -A`.
- Never weaken, skip or delete a test: a deleted test must appear in Task 12's disposition table with its reason.
- macOS cannot verify Linux-gated code: every dvb-ci-runtime change is verified with the cross-clippy in CLAUDE.md **and** the Docker Linux test recipe (memory note `linux-only-tests-via-docker-tarball`).
- Owner decisions that apply to this cluster: Q1 (`no_std` may be dropped only in crates this work touches — media-doctor's *library* stays `no_std`+`alloc` for `watch`/diagnostics; the new `metrics`/`net` features are `std`, same pattern as the existing `std` gate); "take all dependency bumps" (nothing outdated here after W0); SP5 not applicable; ZERO inline styles etc. not applicable (no frontend).
- **CLAUDE.md compliance:** `Cargo.toml` manifests keep their manual column alignment; public enums get `name()` + `impl_spec_display!` or a documented SKIP entry in the crate's `label_coverage` test; `#[non_exhaustive]` on new public structs/enums as elsewhere in the crate.

## Review Focus

The inputs most likely to bite users that the existing suites would not cover. Each has a test in the named task.

1. **Slow, idle and flooding TCP clients on the new metrics server** (the old server's audit MD-W9). hyper only enforces a header-read timeout if a `Timer` is set — without `.timer(TokioTimer::new())` the timeout is silently ignored (verified in `hyper-1.11.0/src/server/conn/http1.rs:346`). Dribbling client, idle client, over-cap client, flood-then-drain, and shutdown-releases-the-port are all tested in Task 10, and each is revert-checked.
2. **Scrape freshness.** (a) The last datagrams before a feed goes quiet must still become visible (a throttled publisher that only publishes on the next datagram would hide them forever) — Task 12 sets a read timeout on the socket and flushes; tested in Tasks 10 and 12. (b) The very first scrape, before any datagram, must already contain every metric family (the old suite asserted `media_doctor_packets_total` on a fresh server). (c) A released PID's per-PID gauge must disappear (`watch.rs` tests at 1698–1790 assert absence): the `metrics` registry cannot unregister a series, so each publish renders from a **fresh** recorder — Task 8 proves old-vs-new equality on every one of those scenarios.
3. **UDP bind.** `SO_RCVBUF` is silently capped by the kernel (`net.core.rmem_max`) and Linux reports double the request; `SO_REUSEADDR` must default OFF (old behaviour) and, when on, must actually allow a second bind and must not allow it when off; a multicast group with a mismatching interface kind (IPv6 index for an IPv4 group) must be an `InvalidInput` error, not a silent wildcard join; the multicast group must still be joined on the requested interface. Task 11.
4. **`Trunk` back-pressure after the Condvar swap.** `StallIngest` must still (a) block until the pin advances, (b) wake on pin advance **and** on pin drop, (c) terminate the pin when `STALL_INGEST_MAX_WAIT` elapses, (d) not hold the state lock while parked (other publishes proceed). And a panic while holding the lock must not poison later calls (parking_lot has no poisoning — the old test is rewritten, Task 3). Existing tests cover (a)–(c); Task 6 adds a deterministic "publisher is parked" probe and a new test for (d).
5. **`rustix::event::poll` semantics vs `libc::poll`.** Timeout 0 must not block; a timeout shorter than a millisecond used to truncate to 0 ms and now sleeps its true duration; `POLLHUP`/`POLLERR` without `POLLIN` must still return `false` (old code tested `revents & POLLIN` only); `EINTR` still surfaces as an `io::Error` (old behaviour — documented, not retried, so callers' loops are unchanged). Task 7 (Linux, Docker).

---

### Task 0: Worktree setup and baseline

**Files:** none (environment only).

- [ ] **Step 1: Create the worktree off main**

```bash
cd /Volumes/External/Projects/rust-broadcast
git fetch -q origin
git worktree add -b w1/p .worktree/w1-p origin/main
cd .worktree/w1-p
git -c protocol.file.allow=always submodule update -q --init --reference /Volumes/External/Projects/rust-broadcast/private private
ln -s /Volumes/External/Projects/rust-broadcast/.test-streams .test-streams
ln -s /Volumes/External/Projects/rust-broadcast/multimux/tests/assets/node_modules multimux/tests/assets/node_modules
```

- [ ] **Step 2: Baseline test run of this cluster's crates (macOS)**

```bash
timeout 1800 cargo test --locked --all-features -p media-plane -p media-doctor -p dvb-ci-runtime 2>&1 | grep -E '^test result|FAILED|panicked' | sort | uniq -c
```

Expected: only `test result: ok` lines. Record the per-binary passed counts in `.delegate/w1-p-report.md` under "baseline (macOS)". (dvb-ci-runtime's `linux.rs` tests do not run here.)

- [ ] **Step 3: Baseline Linux run of dvb-ci-runtime (Docker recipe)**

```bash
S=/private/tmp/claude-501/-Volumes-External-Projects-rust-broadcast/afee462d-1b95-42c9-a99a-a771bfef51c0/scratchpad/ci-linux
mkdir -p $S
cat > $S/run.sh <<'EOF'
set -e
mkdir /w && tar -xf /in/x.tar -C /w && cd /w
export CARGO_TARGET_DIR=/tmp/t
cargo test -p dvb-ci-runtime --all-features --locked 2>&1 | grep -E '^test |^test result|FAILED|panicked|^error' | tail -60
EOF
git ls-files -co --exclude-standard | grep -v '^private/' | tar -cf $S/x.tar -T -
docker run --rm -v $S:/in:ro rust:1.95-slim bash /in/run.sh
```

Expected: `linux::tests::read_on_empty_device_returns_immediately_not_blocking`, `read_rejects_a_frame_that_fills_the_scratch_buffer`, `read_still_returns_short_frames_normally`, `slot_info_falls_back_on_enotty` all `ok`. Record the counts. (Target dir is inside the container — never a bind mount.) Every later "Linux run" in this plan re-runs these four lines with `$S` re-tarred.

- [ ] **Step 4: Record the machine** (cores, OS, `sysctl -n hw.ncpu`) in `.delegate/w1-p-report.md`; Task 2/4 numbers are only comparable on this machine.

---

### Task 1: media-doctor goldens from main (before any change)

**Files:**
- Create: `media-doctor/tests/watch_metrics_golden.rs`
- Create: `media-doctor/tests/golden/watch/README.md`
- Create: `media-doctor/tests/golden/watch/m6-single.prom` (generated)
- Create: `media-doctor/tests/golden/watch/http-response-head.txt` (generated)

**Interfaces:** none (tests only). Task 9 replaces the first test, Task 12 the second.

The outputs touched by SP2.3 are (1) the `WatchState::render_prometheus()` text for the committed capture `fixtures/ts/m6-single.ts`, (2) the HTTP response head of `GET /metrics` from the old bin.

- [ ] **Step 1: Write the golden tests (they create the golden when `UPDATE_GOLDEN=1`)**

`media-doctor/tests/watch_metrics_golden.rs`:

```rust
//! Goldens taken from `main` at the commit named in `golden/watch/README.md`,
//! BEFORE the metrics rewrite (de-hand-roll W1-P, SP2.3). Compared
//! byte-for-byte while the old renderer exists; Tasks 9 and 12 replace each
//! comparison with a documented semantic one.

use std::fs;
use std::io::{Read, Write};
use std::net::{TcpStream, UdpSocket};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use media_doctor::WatchState;

const TS_PACKET_SIZE: usize = 188;
const DATAGRAM_PACKETS: usize = 7;

fn golden(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/golden/watch").join(name)
}

fn check_or_update(name: &str, actual: &str) {
    let path = golden(name);
    if std::env::var_os("UPDATE_GOLDEN").is_some() {
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(&path, actual).unwrap();
        return;
    }
    let want = fs::read_to_string(&path).unwrap_or_else(|e| panic!("read {path:?}: {e}"));
    assert_eq!(actual, want, "golden {name} differs (rerun with UPDATE_GOLDEN=1 on main only)");
}

#[test]
fn render_prometheus_matches_golden_for_the_real_fixture() {
    let bytes = fs::read(concat!(env!("CARGO_MANIFEST_DIR"), "/../fixtures/ts/m6-single.ts")).unwrap();
    let mut state = WatchState::new();
    let mut clock = Duration::ZERO;
    for datagram in bytes.chunks(DATAGRAM_PACKETS * TS_PACKET_SIZE) {
        state.feed_datagram(datagram, clock);
        clock += Duration::from_millis(1);
    }
    check_or_update("m6-single.prom", &state.render_prometheus());
}

/// The response head (status line + headers, no body) the OLD bin sends for
/// `GET /metrics`. Port allocation here is the old reserve-then-rebind; this
/// test exists only to capture the golden from main and is deleted in Task 12.
#[test]
fn old_bin_http_response_head_matches_golden() {
    let free = || {
        let l = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        l.local_addr().unwrap().port()
    };
    let (udp_port, metrics_port) = (free(), free());
    let mut child = Command::new(env!("CARGO_BIN_EXE_media-doctor"))
        .args(["watch", "--udp", &format!("127.0.0.1:{udp_port}"),
               "--metrics-addr", &format!("127.0.0.1:{metrics_port}")])
        .stdout(Stdio::null()).stderr(Stdio::null()).spawn().unwrap();
    let _ = UdpSocket::bind("127.0.0.1:0"); // keep the import used on every cfg
    let deadline = Instant::now() + Duration::from_secs(10);
    let head = loop {
        if let Ok(mut s) = TcpStream::connect(("127.0.0.1", metrics_port)) {
            s.write_all(b"GET /metrics HTTP/1.1\r\nHost: localhost\r\n\r\n").unwrap();
            let mut buf = String::new();
            s.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
            let _ = s.read_to_string(&mut buf);
            break buf;
        }
        assert!(Instant::now() < deadline, "watch never listened");
    };
    let _ = child.kill();
    let _ = child.wait();
    let (head, _body) = head.split_once("\r\n\r\n").expect("response has a head");
    check_or_update("http-response-head.txt", &format!("{head}\r\n"));
}
```

(The `connect` retry loop spins without sleeping; it ends within the bin's start-up time. It is a golden-capture test only and is deleted in Task 12.)

- [ ] **Step 2: Generate the goldens on main's code, then run in compare mode**

```bash
UPDATE_GOLDEN=1 cargo test --locked -p media-doctor --test watch_metrics_golden 2>&1 | grep -E '^test |test result'
cargo test --locked -p media-doctor --test watch_metrics_golden 2>&1 | grep -E '^test |test result'
git rev-parse HEAD
```

Expected: both runs `2 passed`. Open the two files and confirm: the `.prom` file starts with `# HELP media_doctor_packets_total` and contains `media_doctor_packets_total 1264`; the head file starts `HTTP/1.1 200 OK` and has `Content-Type: text/plain; version=0.0.4`, `Content-Length:` and `Connection: close`.

- [ ] **Step 3: Write `golden/watch/README.md`** with the commit hash printed above, the exact command from Step 2, and the rule "regenerate only on a commit that still has the old renderer".

- [ ] **Step 4: Commit**

```bash
git add media-doctor/tests/watch_metrics_golden.rs media-doctor/tests/golden
git commit -m "test(media-doctor): golden /metrics text and response head from main before the metrics rewrite"
```

---

### Task 2: media-plane criterion contention benchmark (BEFORE any change)

**Files:**
- Modify: `media-plane/Cargo.toml` (dev-dependency + `[[bench]]`, after the `[features]` block, lines 33–37 region)
- Create: `media-plane/benches/trunk_contention.rs`
- Create: `media-plane/benches/RESULTS.md`

**Interfaces:** none public. Bench-only.

Design: one publisher thread calls `TrunkWriter::publish(track, Timed, sample)` in a loop; `N` reader threads each own a `SampleCursor` and spin on `poll()` (worst case — every reader contends the same `Mutex<TrunkState>` as hard as it can; documented in the file). `iter_custom` measures the **publisher's** wall time for `iters` publishes, so the metric is "ns per publish at N readers". N ∈ {1, 4, 16, 64}.

- [ ] **Step 1: Cargo.toml** — add (aligned like the other crates, `dvb-si/Cargo.toml:35-37` pattern):

```toml
[dev-dependencies]
scte35-splice = { path = "../scte35-splice", version = "2.1", default-features = false }
criterion     = { version = "0.8", features = ["html_reports"] }

[[bench]]
name              = "trunk_contention"
harness           = false
required-features = ["std"]
```

(The existing `scte35-splice` line at `media-plane/Cargo.toml:31` stays; only `criterion` is appended to the existing `[dev-dependencies]` table.)

- [ ] **Step 2: Write the bench**

```rust
//! Contention benchmark for `Trunk`'s single `Mutex<TrunkState>`
//! (de-hand-roll W1-P, SP6.4): 1 publisher, N spinning `SampleCursor`s.
//! Metric: publisher nanoseconds per `publish`. Readers spin on `poll()` —
//! the worst case; real readers park on `listen()`. Decision rule and
//! recorded numbers: `benches/RESULTS.md`.

use std::num::NonZeroUsize;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
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
    let sample = Sample::new(Bytes::from(vec![0xAB; PAYLOAD]), Some(0), Some(0), None, true);
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
    group.sample_size(20).measurement_time(Duration::from_secs(5));
    for readers in READER_COUNTS {
        group.bench_with_input(BenchmarkId::from_parameter(readers), &readers, |b, &n| {
            b.iter_custom(|iters| publish_with_readers(n, iters));
        });
    }
    group.finish();
}

criterion_group!(benches, bench);
criterion_main!(benches);
```

- [ ] **Step 3: Build, lint, run**

```bash
CARGO_RESOLVER_INCOMPATIBLE_RUST_VERSIONS=fallback cargo build -p media-plane --benches --all-features
git diff --stat Cargo.lock    # only media-plane's dependency list gains criterion
cargo clippy --locked -p media-plane --all-features --all-targets -- -D warnings
cargo bench --locked -p media-plane --bench trunk_contention 2>&1 | grep -E 'time:|trunk_publish' | tee /tmp/bench-before.txt
```

Expected: four `time:` lines (one per N), no panics. (`criterion` is already locked at 0.8.2, so the lock diff is only the new edge.)

- [ ] **Step 4: Write `benches/RESULTS.md`** with: the machine from Task 0 step 4; the decision rule (Task 4 states it numerically); a table "std `Mutex` (baseline, this commit)": N = 1/4/16/64 → median ns/publish from the `time: [low mid high]` middle value; the command used. Leave the "after parking_lot" and "after split" tables with the heading and `(Task 3 / Task 5)`.

- [ ] **Step 5: Commit**

```bash
git add media-plane/Cargo.toml media-plane/benches Cargo.lock
git commit -m "bench(media-plane): trunk publisher-vs-N-cursors contention benchmark, std Mutex baseline recorded"
```

---

### Task 3: media-plane → parking_lot (SP6.6) — poison handling and the Condvar go with it

**Files:**
- Modify: `media-plane/Cargo.toml` (deps lines 13–22, features line 34)
- Modify: `media-plane/src/trunk.rs`: imports 587–588; `Trunk` struct 1425–1469; `Trunk::new` 1485–1505; `lock_state` 1510–1528; `publish_segment` wait loop 2379–2383; module docs 103, 137, 159–177, 431; the poison test at 5256–5290
- Modify: `media-plane/src/lib.rs` (crate-root "no_std note" that says `std::sync::Mutex`)
- Modify: `media-plane/benches/RESULTS.md`

**Interfaces:** no public change. `Trunk::lock_state(&self) -> parking_lot::MutexGuard<'_, TrunkState>` (private). `segment_pin_released: parking_lot::Condvar` (private). No public type mentions the lock, so `media-plane` stays source-compatible (patch bump; Task 15).

Why the Condvar moves too: a `parking_lot::Mutex` guard cannot be waited on by `std::sync::Condvar`. `parking_lot::Condvar::wait_until(&mut guard, Instant)` also removes the manual `remaining` computation and the `PoisonError::into_inner` at line 2382. This is the "simplify" case for the spec's Condvar item. The CAS waiter cap is decided in Task 4.

- [ ] **Step 1: Rewrite the poison test first (it must fail to compile/behave against the new type)**

Replace the test at `trunk.rs:5256-5290` (`subscriber_side_panic_while_holding_the_lock_does_not_poison_later_calls`):

```rust
    /// A panic on another thread while it holds `state` must not break later
    /// calls. `parking_lot::Mutex` has no poisoning, so there is nothing to
    /// recover from — the test pins that no future change reintroduces a
    /// poisoning lock (a `.lock().unwrap()` on the std type would turn this
    /// panic into a permanent one).
    #[test]
    fn subscriber_side_panic_while_holding_the_lock_does_not_poison_later_calls() {
        let trunk = Trunk::new(TrunkConfig::new(nz(4), nz(4), nz(4), nz(8), nz(8)));
        let writer = trunk.writer().unwrap();

        let trunk_for_panic = Arc::clone(&trunk);
        let handle = thread::spawn(move || {
            let _guard = trunk_for_panic.state.lock();
            panic!("simulated panic while holding Trunk::state");
        });
        assert!(handle.join().is_err(), "setup: the thread must have panicked");

        writer.publish(1, RetentionClass::Timed, sample(1, 4));
        assert_eq!(trunk.timed_len(), 1, "publish after the panic must take effect");
        assert!(
            trunk.state.try_lock().is_some(),
            "the lock must be free after the panicking holder unwound"
        );
    }
```

- [ ] **Step 2: Run — expect a compile failure against std `Mutex`** (`try_lock()` returns `Result`, so `.is_some()` does not exist):

```bash
cargo test --locked -p media-plane --all-features --lib subscriber_side_panic 2>&1 | grep -E '^error|no method named'
```

Expected: `no method named is_some found for enum Result` (FAIL — proves the test pins the new type).

- [ ] **Step 3: Implement**

`media-plane/Cargo.toml` — add under `[dependencies]` (aligned):

```toml
# `Trunk`'s lock and back-pressure Condvar (de-hand-roll W1-P, SP6.6): no
# poisoning (the hand-written `lock_state` poison-recovery wrapper is gone) and
# `Condvar::wait_until(&mut guard, Instant)`.
parking_lot      = { version = "0.12", optional = true }
```

and `std = [..., "dep:event-listener", "dep:parking_lot"]`.

`trunk.rs`:

```rust
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
// ...
use parking_lot::{Condvar, Mutex, MutexGuard};
```

(remove `Condvar, Mutex, MutexGuard, PoisonError` from the `std::sync` import at 588.) Replace `lock_state` and its 18-line poison rationale doc with:

```rust
    /// Lock `state`. `parking_lot::Mutex` does not poison: a panic on some
    /// other thread while it held this lock cannot turn every later writer or
    /// reader into a panic, so no recovery wrapper is needed. (Every critical
    /// section here is a bounded push/pop or counter update with no foreign
    /// code inside — see [`crate::retention`] for why the sink hand-off stays
    /// outside any lock this type takes.)
    fn lock_state(&self) -> MutexGuard<'_, TrunkState> {
        self.state.lock()
    }
```

In `publish_segment` replace the `remaining`/`wait_timeout` block (lines 2358–2383) with:

```rust
            if Instant::now() >= deadline {
                // (the existing terminate-the-StallIngest-pins block, unchanged)
                for pin in state.segments.pins.values_mut() {
                    if !pin.terminated
                        && pin.consumed <= oldest
                        && pin.policy == ArchiveOverrun::StallIngest
                    {
                        pin.terminated = true;
                    }
                }
                break;
            }
            self.trunk.segment_pin_released.wait_until(&mut state, deadline);
```

`state` is now `let mut state` guard used via `&mut state`; the `state = next_state;` line is deleted. Update every `//!` doc line that names `std::sync::{Arc, Mutex}` / `std::sync::Condvar::wait_timeout` (lines 159, 2299) to name `parking_lot`; update the `lib.rs` no_std note equally. `grep -n 'PoisonError\|std::sync::Mutex\|poison' media-plane/src/trunk.rs media-plane/src/lib.rs` must show only the rewritten test's comment.

multimux: it does not name `Trunk`'s lock (grep `PoisonError\|lock_state` in `multimux/src` shows nothing for media-plane types) — no multimux change needed. Confirm with `cargo build --locked -p multimux --all-features` in Step 4.

- [ ] **Step 4: Run the whole media-plane suite, the dependents, and the no-default build**

```bash
CARGO_RESOLVER_INCOMPATIBLE_RUST_VERSIONS=fallback cargo build -p media-plane --all-features
git diff Cargo.lock | grep -E '^[-+]' | grep -v '^[-+][-+]'   # only the media-plane dependency list gains parking_lot
cargo test  --locked -p media-plane --all-features 2>&1 | grep -E '^test result|FAILED|panicked'
cargo build --locked -p media-plane --no-default-features
cargo build --locked -p compliance-probe -p multimux --all-features
```

Expected: all `ok`; the StallIngest tests (`archive_overrun_stall_ingest_*`) still pass; no-default builds (parking_lot is optional behind `std`).

- [ ] **Step 5: Re-run the benchmark and record "after parking_lot"**

```bash
cargo bench --locked -p media-plane --bench trunk_contention 2>&1 | grep -E 'time:' | tee /tmp/bench-after-pl.txt
```

Fill the "after parking_lot" table in `benches/RESULTS.md`.

- [ ] **Step 6: Revert-check the deleted poison wrapper's protection.** Temporarily change `use parking_lot::{...}` back to std types for `Mutex` only in a scratch edit and confirm the Step 1 test fails to compile (done in step 2). Restore. Then temporarily delete the `self.trunk.segment_pin_released.notify_all();` at `trunk.rs:2940` and run `cargo test --locked -p media-plane --lib archive_overrun_stall_ingest` — expect the unblock `recv_timeout(60s)`/join to fail or the test to hang past the timeout (use `timeout 120`); restore the line. Record both outputs.

- [ ] **Step 7: Commit**

```bash
git add media-plane Cargo.lock
git commit -m "refactor(media-plane): Trunk lock and StallIngest Condvar on parking_lot, poison-recovery wrapper deleted"
```

---

### Task 4: Decision on the Trunk lock split, and the recorded reasons for what stays

**Files:**
- Modify: `media-plane/benches/RESULTS.md`
- Modify: `media-plane/src/trunk.rs` module docs at 160–200 (the "benchmark verdict" and "why the one lock is kept" paragraphs)

**Decision rule (numeric, fixed before looking at the numbers).** Let `t(N)` be the median ns per publish from the **after-parking_lot** table (Task 3 step 5).

- **SPLIT** iff `t(16) >= 3.0 × t(1)` **or** `t(64) >= 6.0 × t(1)`. Rationale: the spike recorded in `trunk.rs:172-177` measured 956 ns → 9.98 µs (10.4×) from 1 → 16 readers; a split is only worth its invariant cost if contention is still at least 3× at 16 readers once the lock itself is cheaper.
- **NO-SPLIT ("measured, no change")** otherwise.
- If SPLIT: the split is **accepted** only if, after it (Task 5), `t'(16) <= 0.7 × t(16)` and the full media-plane suite and the ordering stress test pass; otherwise Task 5 is reverted and the outcome is recorded as "split attempted, below the 30 % gain bar, reverted".

- [ ] **Step 1: Compute the ratios** from `benches/RESULTS.md` (write them in the file: `t(16)/t(1)`, `t(64)/t(1)`) and apply the rule above. Write the verdict line `VERDICT: SPLIT` or `VERDICT: NO-SPLIT` in the file.

- [ ] **Step 2: Record the CAS waiter cap and Condvar decisions** in the same file, under "Not changed, and why":

  - **Condvar back-pressure:** moved to `parking_lot::Condvar` in Task 3 because the `Mutex` moved (they must pair); it simplified the deadline arithmetic and removed poison handling.
  - **`Trunk::listen` CAS waiter cap** (`trunk.rs:2013-2019`, a 12-line `compare_exchange` loop over `waiter_count`): **left as is.** The spec's replacement ("a `Semaphore`-equivalent") would be `tokio::sync::Semaphore`, which would make this runtime-agnostic crate depend on tokio (the module docs at `trunk.rs:536-545` require it to work "under any executor or none"); `parking_lot` has no counting semaphore; `std` has none; `AtomicUsize::fetch_update` would shorten the loop by four lines without removing it, so it does not clear the "only where they simplify" bar. Evidence for the claim: `grep -n 'Semaphore' ~/.cargo/registry/src/*/parking_lot-0.12.5/src/lib.rs` returns nothing.

- [ ] **Step 3: Update the `trunk.rs` module docs** (160–200) so the "benchmark verdict" paragraph cites `benches/RESULTS.md` and states the verdict and the numbers; the sentence "No deterministic measurement of contention benefit exists in this crate" is replaced by the measured numbers.

- [ ] **Step 4: Commit**

```bash
git add media-plane
git commit -m "docs(media-plane): record Trunk contention numbers and the lock-split decision"
```

---

### Task 5: Split the Trunk lock — ONLY IF Task 4 says `VERDICT: SPLIT`

If the verdict is `NO-SPLIT`, skip this entire task, write "Task 5 skipped: NO-SPLIT, see benches/RESULTS.md" in `.delegate/w1-p-report.md`, and go to Task 6.

**Files:**
- Modify: `media-plane/src/trunk.rs` (struct 1425; `new` 1485; `lock_state` and all 43 `lock_state()` call sites listed by `grep -n 'lock_state()' media-plane/src/trunk.rs`; module docs 96–148)
- Modify: `media-plane/benches/RESULTS.md`

**Interfaces:** none public. Private: `Trunk::samples: Mutex<SampleState>` (`timed`, `sparse`, `handovers`) and `Trunk::rest: Mutex<RestState>` (`segments`, `events`, `parts`, `tracks`, `track_generation`); `lock_samples()`, `lock_rest()`. The `StallIngest` Condvar pairs with `rest`.

Design that preserves the documented causal guarantee (a segment never overtakes its samples; `trunk.rs:96-130`): the guarantee relied on one lock. With two locks it holds by this argument, which the new test exercises: the segmenter's `SampleCursor::poll` acquires the **sample** lock (observing samples), then `publish_segment` acquires the **rest** lock; any reader that observes the segment (acquired `rest` after the segmenter released it) and then acquires `samples` sees at least those samples (happens-before via program order + the `rest` release/acquire, plus per-location coherence on the sample lock). Rule: **no method holds both locks; any method that reads both reads `rest` first, then `samples`.** `Trunk::writer`'s handover log spans both sample classes — both live in `SampleState`, so it takes only the sample lock.

- [ ] **Step 1: Classify every call site** (write the list in `.delegate/w1-p-report.md`): each of the 43 `lock_state()` sites becomes `lock_samples()` or `lock_rest()`; any site that touches fields of both is split into two sequential critical sections (rest first) or the task is escalated. Print: `grep -n 'lock_state()' media-plane/src/trunk.rs`.

- [ ] **Step 2: Write the failing ordering stress test**

```rust
    /// A segment must never be observable before the samples it was built
    /// from — the guarantee the single lock gave structurally and the split
    /// keeps by program order + rest-then-samples read order.
    #[test]
    fn segment_never_overtakes_its_samples_under_a_split_lock() {
        const ROUNDS: u32 = 2_000;
        let trunk = Trunk::new(TrunkConfig::new(nz(4096), nz(4), nz(4096), nz(8), nz(8)));
        let writer = trunk.writer().unwrap();
        let seg_writer = trunk.segment_writer().unwrap();
        let mut seg_samples = trunk.subscribe();
        let mut watcher_samples = trunk.subscribe();
        let mut watcher_segments = trunk.subscribe_segments();

        let ingest = thread::spawn(move || {
            for i in 0..ROUNDS {
                writer.publish(1, RetentionClass::Timed, sample((i % 251) as u8, 4));
            }
        });
        // The "segmenter": publishes segment k only after polling k+1 samples.
        let segmenter = thread::spawn(move || {
            let mut seen = 0u32;
            let mut seq = 1u32;
            while seq <= ROUNDS {
                if let Some(SampleCursorItem::Timed { .. }) = seg_samples.poll() {
                    seen += 1;
                    if seen == seq {
                        seg_writer.publish_segment(segment_entry(0, seq)).unwrap();
                        seq += 1;
                    }
                }
            }
        });
        // The watcher: for each segment it sees (rest lock first), the sample
        // cursor (sample lock second) must already be able to deliver at least
        // `seq` samples.
        let mut delivered = 0u32;
        let mut segs = 0u32;
        while segs < ROUNDS {
            if let Some(SegmentCursorItem::Segment(entry)) = watcher_segments.poll() {
                segs += 1;
                while delivered < entry.sequence_number {
                    match watcher_samples.poll() {
                        Some(SampleCursorItem::Timed { .. }) => delivered += 1,
                        Some(_) => {}
                        None => panic!(
                            "segment {} visible but only {delivered} samples deliverable",
                            entry.sequence_number
                        ),
                    }
                }
            }
        }
        ingest.join().unwrap();
        segmenter.join().unwrap();
    }
```

(Adapt the `SegmentCursorItem::Segment(entry)` pattern to the real variant name by reading `SegmentCursorItem` at its definition; the test's invariant — *segment seq `k` visible ⇒ k samples deliverable* — is the part that must not change. `entry.sequence_number` is the field read at `trunk.rs:2362`.) The test also passes on main's single lock; it is a **guard for the split**, and its mutation check is Step 5.

- [ ] **Step 3: Implement the split** per the interfaces above.

- [ ] **Step 4: Run** `cargo test --locked -p media-plane --all-features` (all pass, including the new test run 20× — `for i in $(seq 20); do cargo test --locked -p media-plane --all-features --lib segment_never_overtakes 2>&1 | grep -c '1 passed'; done | sort | uniq -c` → `20 1`), then `cargo bench --locked -p media-plane --bench trunk_contention`, and fill the "after split" table.

- [ ] **Step 5: Mutation check.** Swap the order in one reader so it reads `samples` before `rest` (e.g. in `SegmentCursor::poll`, read `lock_samples()` first when computing something that needs both) or move `publish_segment`'s push before the stall loop's sample observation; confirm the new test or an existing one fails; restore. Record the output.

- [ ] **Step 6: Apply the acceptance bar.** If `t'(16) > 0.7 × t(16)`: `git checkout media-plane/src/trunk.rs`, write "split attempted, below 30 % gain bar, reverted" with both tables in `RESULTS.md`, and keep only the RESULTS/doc commit.

- [ ] **Step 7: Commit** (`refactor(media-plane): split Trunk state lock into sample and rest groups (measured)`), or the revert note commit.

---

### Task 6: media-plane tests — remove sleep-based waits (SP7)

**Files:**
- Modify: `media-plane/src/trunk.rs`: `Trunk` struct + `publish_segment` (add a test-only counter); tests at 3795–3815, 3932–3950, 4900–4930, 5236–5252
- Modify: `media-plane/src/retention.rs` test at 724–770

**Interfaces:** `#[cfg(test)] Trunk::stalled_publishers: AtomicUsize` (private) and `#[cfg(test)] fn wait_until_stalled(trunk: &Trunk, n: usize)` in `trunk::tests`, re-exported `pub(crate)` for `retention::tests`.

The sites: two fixed `thread::sleep(Duration::from_millis(50))` (`trunk.rs:4908`, `5242`) that "let the main thread reach `wait_deadline`" — unnecessary, because the listener is registered *before* the thread spawns (event_listener's register-before-check contract), so a `notify` that lands first is not lost; and three **negative** waits `done_rx.recv_timeout(Duration::from_millis(200)).is_err()` (`retention.rs:747`, `trunk.rs:3810`, `3947`) that assert "still blocked" by waiting a fixed time. A fixed wait cannot prove "blocked"; the replacement observes the blocked state directly.

- [ ] **Step 1: Write the failing test for the probe** (in `trunk::tests`):

```rust
    /// The stalled-publisher probe is what replaces the 200 ms negative waits:
    /// it must read 1 exactly while `publish_segment` is parked and 0 after.
    #[test]
    fn stalled_publisher_probe_tracks_a_parked_publish_segment() {
        let trunk = Trunk::new(TrunkConfig::new(nz(4), nz(4), nz(1), nz(8), nz(8)));
        let writer = Arc::new(trunk.segment_writer().unwrap());
        let mut pin = trunk.pin_segments(ArchiveOverrun::StallIngest);
        writer.publish_segment(segment_entry(1, 1)).unwrap();
        assert_eq!(trunk.stalled_publishers.load(Ordering::Acquire), 0);

        let bg = Arc::clone(&writer);
        let handle = thread::spawn(move || bg.publish_segment(segment_entry(2, 2)).unwrap());
        wait_until_stalled(&trunk, 1);
        assert_eq!(trunk.stalled_publishers.load(Ordering::Acquire), 1);

        while pin.poll().is_none() {}      // consume seq 1: releases the pin
        handle.join().unwrap();
        assert_eq!(trunk.stalled_publishers.load(Ordering::Acquire), 0);
    }
```

(Use the same `pin_segments` call shape as the existing test at `trunk.rs:3932`; copy its setup lines exactly.)

- [ ] **Step 2: Run — expect compile error** `no field stalled_publishers on type Trunk` / `cannot find function wait_until_stalled`.

```bash
cargo test --locked -p media-plane --all-features --lib stalled_publisher_probe 2>&1 | grep -E '^error' | head -3
```

- [ ] **Step 3: Implement**

In `Trunk`: `#[cfg(test)] stalled_publishers: AtomicUsize,` (init `AtomicUsize::new(0)` under `#[cfg(test)]` in `new`). In `publish_segment` around the `wait_until`:

```rust
            #[cfg(test)]
            self.trunk.stalled_publishers.fetch_add(1, Ordering::AcqRel);
            self.trunk.segment_pin_released.wait_until(&mut state, deadline);
            #[cfg(test)]
            self.trunk.stalled_publishers.fetch_sub(1, Ordering::AcqRel);
```

Helper (bounded; spins with `yield_now`, never sleeps):

```rust
    pub(crate) fn wait_until_stalled(trunk: &Trunk, n: usize) {
        let deadline = Instant::now() + Duration::from_secs(60);
        while trunk.stalled_publishers.load(Ordering::Acquire) < n {
            assert!(Instant::now() < deadline, "publisher never parked in StallIngest");
            thread::yield_now();
        }
    }
```

Then replace each negative wait. Example for `trunk.rs:3808-3812` and `3945-3949`:

```rust
        wait_until_stalled(&trunk, 1);
        assert!(
            done_rx.try_recv().is_err(),
            "publish_segment must still be blocked: archive has not consumed seq 1 yet"
        );
```

and for `retention.rs:745-749` (`use crate::trunk::tests::wait_until_stalled;`). `done_rx.try_recv().is_err()` is sound because the probe has already observed the publisher parked *after* its `fetch_add`, i.e. inside the wait, before any release. Delete the `thread::sleep(Duration::from_millis(50))` at 4908 and 5242 (listener is already registered above them) and shorten the `wait_deadline(... 60s)` bound to 10 s in those two tests (a lost wake now fails in 10 s instead of 60 s).

- [ ] **Step 4: Add the "lock not held while parked" test** (Review Focus 4d):

```rust
    /// While a publisher is parked in StallIngest, ordinary sample publish and
    /// cursor polls on the same Trunk must proceed (the Condvar releases the lock).
    #[test]
    fn sample_publish_proceeds_while_a_segment_publisher_is_stalled() {
        let trunk = Trunk::new(TrunkConfig::new(nz(4), nz(4), nz(1), nz(8), nz(8)));
        let seg_writer = Arc::new(trunk.segment_writer().unwrap());
        let writer = trunk.writer().unwrap();
        let mut pin = trunk.pin_segments(ArchiveOverrun::StallIngest);
        seg_writer.publish_segment(segment_entry(1, 1)).unwrap();
        let bg = Arc::clone(&seg_writer);
        let handle = thread::spawn(move || bg.publish_segment(segment_entry(2, 2)).unwrap());
        wait_until_stalled(&trunk, 1);

        writer.publish(1, RetentionClass::Timed, sample(7, 4)); // must not deadlock
        assert_eq!(trunk.timed_len(), 1);

        while pin.poll().is_none() {}
        handle.join().unwrap();
    }
```

- [ ] **Step 5: Run, then 20 consecutive runs**

```bash
cargo test --locked -p media-plane --all-features 2>&1 | grep -E '^test result|FAILED|panicked'
for i in $(seq 20); do cargo test --locked -p media-plane --all-features --lib 2>&1 | grep -E '^test result'; done | sort | uniq -c
```

Expected: one distinct line, count 20.

- [ ] **Step 6: Revert-check.** (a) Remove the `fetch_add` line: the probe test hangs 60 s then panics "never parked" — record. (b) Replace the `wait_until_stalled` + `try_recv` at `trunk.rs:3945` with nothing and make `publish_segment` skip the wait (`must_wait = false`): `archive_overrun_stall_ingest_actually_blocks_the_writer` must fail. Restore both.

- [ ] **Step 7: Commit**

```bash
git add media-plane
git commit -m "test(media-plane): observe parked StallIngest publishers directly instead of sleeping"
```

---

### Task 7: dvb-ci-runtime — `libc::poll` → `rustix::event::poll` (SP6.7, Linux-gated)

**Files:**
- Modify: `dvb-ci-runtime/Cargo.toml` (deps 27–28, feature 36)
- Modify: `dvb-ci-runtime/src/linux.rs`: `poll_readable` 23–37; callers 233–235 and 313–315; tests module (append)

**Interfaces:** `fn poll_readable(fd: &impl AsFd, timeout: Duration) -> io::Result<bool>` (private). `CaDevice::poll`/`CiDataDevice::poll` signatures unchanged. `libc` stays (ioctl, `mkfifo`, `O_NONBLOCK`, `EINVAL`/`ENOTTY`): only the poll leaves it.

- [ ] **Step 1: Write the characterization tests (they pass on the old code; the mutation in step 6 proves they bite)** — append to `linux.rs` `mod tests`:

```rust
    /// `poll_readable` on an empty FIFO with a short timeout reports `false`
    /// after about that long; with data it reports `true` immediately.
    #[test]
    fn poll_readable_reports_data_and_honours_the_timeout() {
        let path = make_fifo();
        let mut dev = LinuxCiDataDevice::open_path(&path).expect("open_path");
        let mut writer = OpenOptions::new().write(true).open(&path).expect("writer");

        let start = std::time::Instant::now();
        assert!(!dev.poll(Duration::from_millis(80)).unwrap(), "empty FIFO is not readable");
        assert!(start.elapsed() >= Duration::from_millis(70), "poll returned before its timeout");

        writer.write_all(&[0x47; TS_PACKET_LEN]).unwrap();
        let start = std::time::Instant::now();
        assert!(dev.poll(Duration::from_secs(5)).unwrap(), "data written: readable");
        assert!(start.elapsed() < Duration::from_secs(4), "must return as soon as readable");

        let _ = std::fs::remove_file(&path);
    }

    /// A zero timeout never blocks.
    #[test]
    fn poll_readable_zero_timeout_does_not_block() {
        let path = make_fifo();
        let mut dev = LinuxCiDataDevice::open_path(&path).expect("open_path");
        let _writer = OpenOptions::new().write(true).open(&path).expect("writer");
        let start = std::time::Instant::now();
        assert!(!dev.poll(Duration::ZERO).unwrap());
        assert!(start.elapsed() < Duration::from_millis(500));
        let _ = std::fs::remove_file(&path);
    }

    /// Sub-millisecond timeouts used to truncate to `poll(.., 0)`; they must
    /// still return promptly and never error.
    #[test]
    fn poll_readable_sub_millisecond_timeout_is_accepted() {
        let path = make_fifo();
        let mut dev = LinuxCiDataDevice::open_path(&path).expect("open_path");
        let _writer = OpenOptions::new().write(true).open(&path).expect("writer");
        assert!(!dev.poll(Duration::from_micros(300)).unwrap());
        let _ = std::fs::remove_file(&path);
    }

    /// Hang-up without data (writer closed) is not "readable" for this API:
    /// the old code tested `revents & POLLIN` only.
    #[test]
    fn poll_readable_ignores_hangup_without_data() {
        let path = make_fifo();
        let mut dev = LinuxCiDataDevice::open_path(&path).expect("open_path");
        let writer = OpenOptions::new().write(true).open(&path).expect("writer");
        drop(writer); // POLLHUP on the read end, no POLLIN
        assert!(!dev.poll(Duration::from_millis(50)).unwrap());
        let _ = std::fs::remove_file(&path);
    }
```

(`make_fifo`, `OpenOptions`, `Write`, `TS_PACKET_LEN` are already in scope in `mod tests` via `use super::*;` — verify; add `use std::io::Write;` if the module lacks it. On Linux a FIFO whose only writer closed reports `POLLHUP` with `POLLIN` clear when no data is pending, which is exactly the case under test.)

- [ ] **Step 2: Run on Linux (Docker) against the OLD code — expect 4 new passes**

```bash
S=/private/tmp/claude-501/-Volumes-External-Projects-rust-broadcast/afee462d-1b95-42c9-a99a-a771bfef51c0/scratchpad/ci-linux
git ls-files -co --exclude-standard | grep -v '^private/' | tar -cf $S/x.tar -T -
docker run --rm -v $S:/in:ro rust:1.95-slim bash /in/run.sh
```

Expected: the 4 baseline tests + the 4 new `poll_readable_*` all `ok` (characterization).

- [ ] **Step 3: Implement**

`Cargo.toml`:

```toml
libc       = { version = "0.2.108", optional = true } # `libc::Ioctl` (used in linux.rs) first appears in 0.2.108
rustix     = { version = "1", default-features = false, features = ["std", "event"], optional = true } # `rustix::event::poll` replaces `libc::poll` (de-hand-roll SP6.7)
...
linux = ["dep:libc", "dep:rustix", "dep:clap"]
```

`linux.rs` lines 23–37 become:

```rust
/// Poll a file descriptor for readability up to `timeout`.
///
/// `EINTR` surfaces as an `io::Error` exactly as it did with `libc::poll`
/// (the driver's pump loop owns the retry decision). Only `POLLIN` counts as
/// readable; `POLLHUP`/`POLLERR` alone report `false`, as before. Unlike the
/// old millisecond truncation the timeout keeps its sub-millisecond part.
fn poll_readable(fd: &impl AsFd, timeout: Duration) -> io::Result<bool> {
    let mut fds = [PollFd::new(fd, PollFlags::IN)];
    let ts = Timespec {
        tv_sec: i64::try_from(timeout.as_secs()).unwrap_or(i64::MAX),
        tv_nsec: Nsecs::try_from(timeout.subsec_nanos()).unwrap_or(0),
    };
    rustix::event::poll(&mut fds, Some(&ts))?;
    Ok(fds[0].revents().contains(PollFlags::IN))
}
```

with imports `use std::os::unix::io::AsFd;` (keep `AsRawFd` for the ioctl calls) and `use rustix::event::{Nsecs, PollFd, PollFlags, Timespec};`. Callers: `poll_readable(&self.file, timeout)` at both sites (`File: AsFd`). The line `rustix::event::poll(...)?` converts `rustix::io::Errno` through `impl From<Errno> for io::Error` (enabled by the `std` feature).

- [ ] **Step 4: Lock, cross-clippy, Linux tests**

```bash
CARGO_RESOLVER_INCOMPATIBLE_RUST_VERSIONS=fallback cargo build -p dvb-ci-runtime --all-features
git diff Cargo.lock | grep -E '^[-+]' | grep -v '^[-+][-+]'    # rustix (+ linux-raw-sys, bitflags if not present) only
rustup target add x86_64-unknown-linux-gnu
cargo clippy -p dvb-ci-runtime --all-features --all-targets --locked --target x86_64-unknown-linux-gnu -- -D warnings
RUSTDOCFLAGS="-D warnings" cargo doc -p dvb-ci-runtime --all-features --no-deps --locked --target x86_64-unknown-linux-gnu
git ls-files -co --exclude-standard | grep -v '^private/' | tar -cf $S/x.tar -T -
docker run --rm -v $S:/in:ro rust:1.95-slim bash /in/run.sh
```

Expected: lock gains only `rustix` (and its `linux-raw-sys`/`bitflags` if absent — list them); clippy and doc clean; all 8 Linux tests `ok`. If `ring`-style C-toolchain errors appear in the cross-clippy, they are unrelated to this crate (CLAUDE.md note) — it builds only this crate.

- [ ] **Step 5: macOS default build unaffected:** `cargo build --locked -p dvb-ci-runtime` and `cargo build --locked -p dvb-ci-runtime --no-default-features`.

- [ ] **Step 6: Revert-check inside the container.** Add to a copy of `run.sh`:

```bash
sed -i 's/Ok(fds\[0\].revents().contains(PollFlags::IN))/Ok(true)/' dvb-ci-runtime/src/linux.rs
cargo test -p dvb-ci-runtime --all-features --locked poll_readable 2>&1 | grep -E 'test |FAILED'
```

Expected: `poll_readable_reports_data_and_honours_the_timeout`, `poll_readable_zero_timeout_does_not_block` and `poll_readable_ignores_hangup_without_data` FAIL (the sed must visibly recompile — require `Compiling dvb-ci-runtime` in the output as proof, per memory note `prove-the-mutation-actually-mutated`). Second mutation: `tv_nsec: 0` — `poll_readable_reports_data_and_honours_the_timeout` fails on the `>= 70 ms` assertion. Record both transcripts.

- [ ] **Step 7: Commit**

```bash
git add dvb-ci-runtime Cargo.lock
git commit -m "refactor(dvb-ci-runtime): rustix::event::poll replaces libc::poll, with Linux poll tests"
```

---

### Task 8: media-doctor — `WatchSnapshot` and the `metrics` exposition, proven equal to the old renderer (additive)

**Files:**
- Modify: `media-doctor/Cargo.toml` (deps + features)
- Modify: `media-doctor/src/watch.rs`: add `snapshot()` after `render_prometheus` (528–705); test helper in `mod tests` (728+)
- Create: `media-doctor/src/watch_metrics.rs`
- Modify: `media-doctor/src/lib.rs` (module + exports, lines 42 and 60)

**Interfaces (exact):**

```rust
// watch.rs — no_std + alloc, always compiled
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub struct WatchSnapshot {
    pub packets: u64,
    pub datagrams: u64,
    pub resync_events: u64,
    pub dropped_bytes: u64,
    pub in_sync: bool,
    pub conformance: Vec<ConformanceSample>,
    pub scte35_events: u64,
    pub scte35_open: u64,
    pub pts_dts_anomalies: u64,
    pub codec_signalling: Vec<PidFlag>,   // only PIDs with >= 1 access unit
    pub pts_dts_anomaly: Vec<PidFlag>,    // only PIDs with a decode timestamp seen
    pub last_packet_clock_seconds: f64,
}
#[derive(Debug, Clone, PartialEq, Eq)] #[non_exhaustive]
pub struct ConformanceSample { pub indicator: &'static str, pub priority: &'static str, pub clause: &'static str, pub count: u64 }
#[derive(Debug, Clone, Copy, PartialEq, Eq)] #[non_exhaustive]
pub struct PidFlag { pub pid: u16, pub set: bool }
impl WatchState { pub fn snapshot(&self) -> WatchSnapshot }

// watch_metrics.rs — feature `metrics` (std)
pub fn render(state: &WatchState) -> String;             // Prometheus text via metrics-exporter-prometheus
pub fn publish(snapshot: &WatchSnapshot);                // writes the facade (counters .absolute(), gauges .set())
```

`ConformanceCount` (private, `watch.rs:135`) already stores `priority`, `clause`, `count` — `ConformanceSample` is built from it. Public `PidFlag.set` is `true` for a codec mismatch / a decode anomaly.

- [ ] **Step 1: Write the failing equivalence test.** In `watch.rs` `mod tests`, add a helper and the comparator; change nothing else yet:

```rust
    /// Parse Prometheus text into (family -> (help, type)) and
    /// (series -> value), where a series key is `name` plus its labels sorted.
    /// `# clauses:` comment lines (a media-doctor extension the exporter does
    /// not emit) are ignored.
    pub(crate) fn parse_exposition(
        text: &str,
    ) -> (BTreeMap<String, (String, String)>, BTreeMap<String, f64>) {
        let mut meta: BTreeMap<String, (String, String)> = BTreeMap::new();
        let mut series = BTreeMap::new();
        for line in text.lines() {
            if let Some(rest) = line.strip_prefix("# HELP ") {
                let (name, help) = rest.split_once(' ').unwrap();
                meta.entry(name.into()).or_default().0 = help.into();
            } else if let Some(rest) = line.strip_prefix("# TYPE ") {
                let (name, ty) = rest.split_once(' ').unwrap();
                meta.entry(name.into()).or_default().1 = ty.into();
            } else if line.starts_with('#') || line.is_empty() {
                continue;
            } else {
                let (key, value) = line.rsplit_once(' ').unwrap();
                let key = match key.split_once('{') {
                    Some((name, labels)) => {
                        let mut l: Vec<&str> = labels.trim_end_matches('}').split("\",").collect();
                        l.sort_unstable();
                        alloc::format!("{name}{{{}}}", l.join("\","))
                    }
                    None => key.into(),
                };
                series.insert(key, value.parse::<f64>().unwrap());
            }
        }
        (meta, series)
    }

    /// Render through the old renderer AND the exporter, assert they agree
    /// semantically, and return the old text (so the existing assertions in
    /// this module run unchanged until Task 9 deletes the old renderer).
    fn render_both(state: &WatchState) -> String {
        let old = state.render_prometheus();
        let new = crate::watch_metrics::render(state);
        let (old_meta, old_series) = parse_exposition(&old);
        let (new_meta, new_series) = parse_exposition(&new);
        assert_eq!(old_meta, new_meta, "HELP/TYPE differ:\nOLD:\n{old}\nNEW:\n{new}");
        assert_eq!(old_series, new_series, "series differ:\nOLD:\n{old}\nNEW:\n{new}");
        old
    }
```

Then replace every `state.render_prometheus()` in the `mod tests` module (lines 1003, 1059, 1081, 1187, 1331, 1482, 1621, 1642, 1673, 1686, 1730, 1771 — re-grep) with `render_both(&state)`. Gate the module `#[cfg(all(test, feature = "metrics"))]`. This covers: the real fixture, resync, garbage, SCTE-35 reassembly/auto-return, discontinuity, PMT version change retrack, shared-PID release (Review Focus 2c), stream-type change. It also gives an **empty-state** check: add

```rust
    #[test]
    fn empty_state_exposes_every_family_before_any_datagram() {
        let text = render_both(&WatchState::new());
        for name in [
            "media_doctor_packets_total", "media_doctor_datagrams_total",
            "media_doctor_resync_events_total", "media_doctor_dropped_bytes_total",
            "media_doctor_conformance_in_sync", "media_doctor_scte35_events_total",
            "media_doctor_scte35_open_events", "media_doctor_pts_dts_anomalies_total",
            "media_doctor_last_packet_clock_seconds",
        ] {
            assert!(text.contains(&format!("\n{name} ")) || text.starts_with(&format!("# HELP {name}")) || text.contains(&format!("{name} 0")),
                "family {name} missing on a fresh state:\n{text}");
        }
    }
```

(Simplify the assertion to `metric_value(&text, name).is_some()` — the helper at 1876 already returns the unlabelled value.)

- [ ] **Step 2: Run — expect compile failure** (`could not find watch_metrics`, no `metrics` feature):

```bash
cargo test --locked -p media-doctor --all-features --lib watch 2>&1 | grep -E '^error' | head -3
```

- [ ] **Step 3: Cargo.toml** — features and deps (aligned; comment each):

```toml
# Prometheus exposition for `watch` (de-hand-roll W1-P, SP2.3): the exporter's
# recorder + `PrometheusHandle::render()` do the text format (HELP/TYPE,
# label escaping). `default-features = false`: no listener, no push gateway —
# its built-in listener has no header-read timeout, no connection cap and no
# way to learn a port-0 address, so serving is `metrics_server` (hyper).
metrics                    = { version = "0.24", optional = true }
metrics-exporter-prometheus = { version = "0.18", default-features = false, optional = true }
...
[features]
default = ["std", "serde", "cli"]
metrics = ["std", "dep:metrics", "dep:metrics-exporter-prometheus"]
cli     = ["dep:clap", "dep:thiserror", "dep:container-probe", "std", "metrics"]
```

(Keep the existing `std`/`serde` lines; `net` is added in Task 10.)

- [ ] **Step 4: Implement `snapshot()`** in `watch.rs` (replaces nothing yet):

```rust
    /// A typed, `no_std` copy of every accumulated figure — what the
    /// Prometheus exposition (`watch_metrics`, feature `metrics`) publishes.
    #[must_use]
    pub fn snapshot(&self) -> WatchSnapshot {
        let conformance_stats = self.conformance.stats();
        let resync_stats = self.resync.stats();
        WatchSnapshot {
            packets: conformance_stats.packets,
            datagrams: self.datagrams_total,
            resync_events: resync_stats.resyncs,
            dropped_bytes: resync_stats.dropped_bytes,
            in_sync: conformance_stats.in_sync,
            conformance: self
                .conformance_counts
                .iter()
                .map(|(name, c)| ConformanceSample {
                    indicator: name,
                    priority: c.priority,
                    clause: c.clause,
                    count: c.count,
                })
                .collect(),
            scte35_events: self.scte35_events_total,
            scte35_open: self
                .scte35_tracks
                .values()
                .map(|t| u64::try_from(t.open_count()).unwrap_or(u64::MAX))
                .sum(),
            pts_dts_anomalies: self.pts_dts_anomalies_total,
            codec_signalling: self
                .es_tracks
                .iter()
                .filter(|(_, t)| t.any_au)
                .map(|(&pid, t)| PidFlag { pid, set: !t.structured })
                .collect(),
            pts_dts_anomaly: self
                .es_tracks
                .iter()
                .filter(|(_, t)| t.prev_decode.is_some())
                .map(|(&pid, t)| PidFlag { pid, set: t.decode_anomaly })
                .collect(),
            last_packet_clock_seconds: self.last_clock.as_secs_f64(),
        }
    }
```

(If `conformance_counts`' key type is not `&'static str`, match the actual type in `watch.rs:135-140` — the field is `BTreeMap<&'static str, ConformanceCount>`.) Export the three types from `lib.rs`: `pub use watch::{ConformanceSample, PidFlag, WatchSnapshot, WatchState};`.

- [ ] **Step 5: Implement `watch_metrics.rs`**

```rust
//! Prometheus exposition for [`WatchState`] through the `metrics` facade and
//! `metrics-exporter-prometheus` (de-hand-roll W1-P, SP2.3) — replaces the
//! hand-written `render_prometheus` text builder.
//!
//! Every publish builds a **fresh** recorder: the `metrics` registry cannot
//! unregister a series, and `watch` must stop exposing a PID's gauge once the
//! PID is released, exactly as the old snapshot renderer did. Counters carry
//! the accumulated total (`absolute`), not a delta.

use metrics::{counter, describe_counter, describe_gauge, gauge, with_local_recorder};
use metrics_exporter_prometheus::PrometheusBuilder;

use crate::{WatchSnapshot, WatchState};

const PACKETS_TOTAL: &str = "media_doctor_packets_total";
const DATAGRAMS_TOTAL: &str = "media_doctor_datagrams_total";
const RESYNC_EVENTS_TOTAL: &str = "media_doctor_resync_events_total";
const DROPPED_BYTES_TOTAL: &str = "media_doctor_dropped_bytes_total";
const IN_SYNC: &str = "media_doctor_conformance_in_sync";
const CONFORMANCE_EVENTS_TOTAL: &str = "media_doctor_conformance_events_total";
const SCTE35_EVENTS_TOTAL: &str = "media_doctor_scte35_events_total";
const SCTE35_OPEN_EVENTS: &str = "media_doctor_scte35_open_events";
const PTS_DTS_ANOMALIES_TOTAL: &str = "media_doctor_pts_dts_anomalies_total";
const CODEC_SIGNALLING_MISMATCH: &str = "media_doctor_codec_signalling_mismatch";
const PTS_DTS_ANOMALY: &str = "media_doctor_pts_dts_anomaly";
const LAST_PACKET_CLOCK_SECONDS: &str = "media_doctor_last_packet_clock_seconds";

/// Render `state` as Prometheus text exposition format (a `GET /metrics` body).
#[must_use]
pub fn render(state: &WatchState) -> String {
    let snapshot = state.snapshot();
    let recorder = PrometheusBuilder::new().build_recorder();
    with_local_recorder(&recorder, || publish(&snapshot));
    recorder.handle().render()
}

/// Write `snapshot` into the current `metrics` recorder.
pub fn publish(s: &WatchSnapshot) {
    describe_counter!(PACKETS_TOTAL, "Total well-formed 188-byte TS packets processed (ISO/IEC 13818-1 section 2.4.3.2).");
    describe_counter!(DATAGRAMS_TOTAL, "Total ingest datagrams fed (e.g. UDP payloads).");
    describe_counter!(RESYNC_EVENTS_TOTAL, "Times TS byte-stream sync was lost and reacquired (mpeg_ts::resync::TsResync).");
    describe_counter!(DROPPED_BYTES_TOTAL, "Bytes dropped before/while reacquiring TS packet sync.");
    describe_gauge!(IN_SYNC, "Whether the ETSI TR 101 290 monitor currently considers the stream in sync (1) or not (0).");
    describe_counter!(CONFORMANCE_EVENTS_TOTAL, "ETSI TR 101 290 indicator events observed, by indicator and priority tier.");
    describe_counter!(SCTE35_EVENTS_TOTAL, "Total SCTE-35 splice_insert events observed (ANSI/SCTE 35 section 9.7.3.1), excluding cancelled events.");
    describe_gauge!(SCTE35_OPEN_EVENTS, "Currently-unmatched (\"out\" with no \"in\" yet, and no auto-return) SCTE-35 splice_insert events.");
    describe_counter!(PTS_DTS_ANOMALIES_TOTAL, "Non-monotonic decode-timestamp (DTS, else PTS) events observed on tracked PES PIDs.");
    describe_gauge!(CODEC_SIGNALLING_MISMATCH, "Whether a PMT-declared codec PID has ever shown bitstream framing disagreeing with the declared stream_type (1) or not (0); only emitted once at least one access unit has been observed on that PID (ISO/IEC 13818-1 Table 2-34).");
    describe_gauge!(PTS_DTS_ANOMALY, "Whether a tracked PES PID has ever shown a non-monotonic decode timestamp (1) or not (0); only emitted once a decode timestamp has been observed on that PID.");
    describe_gauge!(LAST_PACKET_CLOCK_SECONDS, "Elapsed ingest wall-clock time (seconds) of the most recently processed TS packet.");

    counter!(PACKETS_TOTAL).absolute(s.packets);
    counter!(DATAGRAMS_TOTAL).absolute(s.datagrams);
    counter!(RESYNC_EVENTS_TOTAL).absolute(s.resync_events);
    counter!(DROPPED_BYTES_TOTAL).absolute(s.dropped_bytes);
    gauge!(IN_SYNC).set(f64::from(u8::from(s.in_sync)));
    for c in &s.conformance {
        counter!(CONFORMANCE_EVENTS_TOTAL, "indicator" => c.indicator, "priority" => c.priority)
            .absolute(c.count);
    }
    counter!(SCTE35_EVENTS_TOTAL).absolute(s.scte35_events);
    #[allow(clippy::cast_precision_loss)]
    gauge!(SCTE35_OPEN_EVENTS).set(s.scte35_open as f64);
    counter!(PTS_DTS_ANOMALIES_TOTAL).absolute(s.pts_dts_anomalies);
    for f in &s.codec_signalling {
        gauge!(CODEC_SIGNALLING_MISMATCH, "pid" => alloc::format!("0x{:04X}", f.pid))
            .set(f64::from(u8::from(f.set)));
    }
    for f in &s.pts_dts_anomaly {
        gauge!(PTS_DTS_ANOMALY, "pid" => alloc::format!("0x{:04X}", f.pid))
            .set(f64::from(u8::from(f.set)));
    }
    gauge!(LAST_PACKET_CLOCK_SECONDS).set(s.last_packet_clock_seconds);
}
```

(`media-doctor/src/lib.rs` is `#![cfg_attr(not(feature = "std"), no_std)]` with `extern crate alloc`; `metrics` implies `std`. Replace `#[allow(clippy::cast_precision_loss)]` with an `f64::from(u32::try_from(..).unwrap_or(u32::MAX))` if clippy `-D warnings` objects to the allow — workspace convention: no stray `allow`s.)

`lib.rs`: `#[cfg(feature = "metrics")] mod watch_metrics;` and `#[cfg(feature = "metrics")] pub use watch_metrics::{publish as publish_metrics, render as render_metrics};`.

- [ ] **Step 6: Run — expect PASS, and read any diff carefully.** If `render_both` reports a HELP/TYPE or series difference, fix the exposition (not the comparator) until every one of the 13 scenarios is equal.

```bash
cargo test --locked -p media-doctor --all-features --lib watch 2>&1 | grep -E '^test |test result|FAILED'
cargo build --locked -p media-doctor --no-default-features
cargo build --locked -p media-doctor --no-default-features --features std
cargo build --locked -p media-doctor --features metrics --no-default-features
```

Lock check first: `CARGO_RESOLVER_INCOMPATIBLE_RUST_VERSIONS=fallback cargo build -p media-doctor --all-features; git diff Cargo.lock | grep -E '^[-+]' | grep -v '^[-+][-+]'` — only media-doctor's dependency list gains `metrics` and `metrics-exporter-prometheus`.

- [ ] **Step 7: Revert-check.** In `publish`, change `.absolute(s.packets)` to `.absolute(s.packets + 1)`: the real-fixture test fails on a series difference. Change the `pts_dts_anomaly` loop to read `f.set` from `codec_signalling`: the PMT-retrack test fails. Restore.

- [ ] **Step 8: Commit**

```bash
git add media-doctor Cargo.lock
git commit -m "feat(media-doctor): WatchSnapshot and metrics-exporter-prometheus exposition, proven equal to render_prometheus"
```

---

### Task 9: media-doctor — delete `render_prometheus` (breaking) and move the golden check to the new path

**Files:**
- Modify: `media-doctor/src/watch.rs`: delete `render_prometheus` (520–705), `metric_header` (707–712), `escape_label` (714–727), the `core::fmt::Write` and `String` imports if now unused; module docs lines 18, 145, plus the "socket glue" paragraph at 20–26; `render_both` → `render`
- Modify: `media-doctor/src/watch_metrics.rs` (golden comparison test)
- Delete: `media-doctor/tests/watch_metrics_golden.rs` first test (keep the HTTP-head test until Task 12)
- Modify: `media-doctor/tests/golden/watch/README.md`

**Interfaces:** removed: `WatchState::render_prometheus(&self) -> String` (breaking). Replacement documented in the CHANGELOG: `media_doctor::render_metrics(&WatchState)` (feature `metrics`) or `WatchState::snapshot()`.

- [ ] **Step 1: Write the golden comparison against the new path**, in `watch_metrics.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::watch::tests::parse_exposition;
    use core::time::Duration;

    #[test]
    fn real_fixture_exposition_equals_the_main_golden_semantically() {
        let bytes = std::fs::read(concat!(env!("CARGO_MANIFEST_DIR"), "/../fixtures/ts/m6-single.ts")).unwrap();
        let mut state = WatchState::new();
        let mut clock = Duration::ZERO;
        for datagram in bytes.chunks(7 * 188) {
            state.feed_datagram(datagram, clock);
            clock += Duration::from_millis(1);
        }
        let golden = std::fs::read_to_string(concat!(
            env!("CARGO_MANIFEST_DIR"), "/tests/golden/watch/m6-single.prom")).unwrap();
        let (gm, gs) = parse_exposition(&golden);
        let (nm, ns) = parse_exposition(&render(&state));
        assert_eq!(gm, nm, "HELP/TYPE differ from the golden");
        assert_eq!(gs, ns, "series differ from the golden");
    }
}
```

(Make `watch::tests::parse_exposition` `pub(crate)` inside `#[cfg(test)] pub(crate) mod tests`.) Run it **before** deleting the old renderer: expect PASS.

- [ ] **Step 2: Delete `render_prometheus`, `metric_header`, `escape_label`; change `render_both` to**

```rust
    fn render(state: &WatchState) -> String {
        crate::watch_metrics::render(state)
    }
```

and replace each `render_both(&state)` with `render(&state)`. Update `watch.rs` docs: the module paragraph "Neither function touches a socket … `Arc<Mutex<WatchState>>` shared with the metrics HTTP thread" becomes "[`WatchState::snapshot`] returns the accumulated figures; the `metrics` feature turns them into Prometheus text (`render_metrics`)". Delete the first test in `tests/watch_metrics_golden.rs` (its job is now the in-crate test above) but keep the golden file.

- [ ] **Step 3: Run**

```bash
cargo test --locked -p media-doctor --all-features 2>&1 | grep -E '^test result|FAILED|panicked'
cargo clippy --locked -p media-doctor --all-features --all-targets -- -D warnings
cargo build --locked -p media-doctor --no-default-features
```

Expected: all pass; no `dead_code` warnings under `--no-default-features` (all snapshot fields are read by `snapshot()`).

- [ ] **Step 4: Revert-check.** Re-add a `datagrams_total` off-by-one in `snapshot()` (`self.datagrams_total + 1`): the golden test fails with a series difference. Restore.

- [ ] **Step 5: README of the golden**: add "Text differences between the old renderer and the exporter, all semantically neutral: (1) the `# clauses: …` comment line is gone (the clause is now only on `ConformanceSample::clause`), (2) metric/series ordering follows the exporter's, (3) `HELP`/`TYPE` lines are identical." Verified by Task 8's equivalence run — paste the one-line diff summary from a manual `diff <(sort old) <(sort new)` run on `m6-single`.

- [ ] **Step 6: Commit**

```bash
git add media-doctor
git commit -m "feat(media-doctor)!: remove WatchState::render_prometheus in favour of snapshot() + metrics exposition"
```

---

### Task 10: media-doctor — metrics HTTP server on hyper (`metrics_server`, feature `net`)

**Files:**
- Modify: `media-doctor/Cargo.toml` (feature `net` + deps + dev-deps)
- Create: `media-doctor/src/metrics_server.rs`
- Modify: `media-doctor/src/lib.rs` (module, exports)

**Interfaces (exact):**

```rust
// feature `net` (implies `metrics`, `std`)
#[derive(Debug, Clone, Copy, PartialEq, Eq)] #[non_exhaustive]
pub struct MetricsServerConfig { pub max_conns: usize, pub io_timeout: Duration }
impl Default for MetricsServerConfig { /* 32, 5 s — the old CLI defaults */ }

pub struct MetricsPublisher { /* watch::Sender<Arc<str>>, interval, last publish clock */ }
pub fn channel(initial: &WatchState, interval: Duration) -> (MetricsPublisher, tokio::sync::watch::Receiver<Arc<str>>);
impl MetricsPublisher {
    pub fn maybe_publish(&mut self, state: &WatchState, clock: Duration) -> bool; // true if it rendered
    pub fn publish_now(&mut self, state: &WatchState, clock: Duration);           // flush: unconditional
}

pub async fn serve(
    listener: tokio::net::TcpListener,
    metrics: tokio::sync::watch::Receiver<Arc<str>>,
    config: MetricsServerConfig,
    shutdown: tokio_util::sync::CancellationToken,
);   // returns after `shutdown` and every connection task has ended; the listener is dropped (port released)
```

W2/other-crate note: nothing outside media-doctor consumes these.

Design facts, all verified in source: `http1::Builder::timer(TokioTimer::new()).header_read_timeout(d)` (hyper-1.11.0 `http1.rs:352,411`); `keep_alive(false)` gives `Connection: close`; `TaskTracker::{spawn, close, wait}` (`tokio-util` feature `rt`); `watch::Sender::send_replace` is callable from a non-async thread. Over-cap connections are still served — by a service that always answers `503` — so no HTTP bytes are hand-written; every connection (admitted or refused) is bounded by the same total deadline.

- [ ] **Step 1: Cargo.toml**

```toml
# `watch` metrics server (de-hand-roll W1-P, SP2.3 / SP1.6): hyper does the HTTP
# framing; tokio-util owns the connection tasks and the shutdown token;
# socket2 binds the UDP socket (Task 11).
tokio            = { version = "1", default-features = false, features = ["rt", "net", "time", "sync", "macros"], optional = true }
tokio-util       = { version = "0.7", default-features = false, features = ["rt"], optional = true }
hyper            = { version = "1", default-features = false, features = ["server", "http1"], optional = true }
hyper-util       = { version = "0.1", default-features = false, features = ["tokio"], optional = true }
http-body-util   = { version = "0.1", optional = true }
socket2          = { version = "0.6", optional = true }
...
net     = ["metrics", "dep:tokio", "dep:tokio-util", "dep:hyper", "dep:hyper-util", "dep:http-body-util", "dep:socket2"]
cli     = ["dep:clap", "dep:thiserror", "dep:container-probe", "std", "net"]

[dev-dependencies]
tokio = { version = "1", features = ["rt", "net", "time", "io-util", "macros"] }
```

`bytes` types come from `hyper::body::Bytes`. Lock check as in Task 8 (`tokio-util` gains the `rt` feature edge only; no new package).

- [ ] **Step 2: Write the failing tests first** — `metrics_server.rs` `#[cfg(test)] mod tests`. Helper (real sockets on port 0, no sleeps):

```rust
    use super::*;
    use std::sync::Arc;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::{TcpListener, TcpStream};

    const REQ: &[u8] = b"GET /metrics HTTP/1.1\r\nHost: localhost\r\n\r\n";
    const BODY: &str = "# HELP media_doctor_packets_total x\n# TYPE media_doctor_packets_total counter\nmedia_doctor_packets_total 0\n";

    struct Server { addr: std::net::SocketAddr, token: CancellationToken, task: tokio::task::JoinHandle<()> }

    async fn start(cfg: MetricsServerConfig) -> (Server, tokio::sync::watch::Sender<Arc<str>>) {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let (tx, rx) = tokio::sync::watch::channel(Arc::<str>::from(BODY));
        let token = CancellationToken::new();
        let task = tokio::spawn(serve(listener, rx, cfg, token.clone()));
        (Server { addr, token, task }, tx)
    }

    async fn scrape(addr: std::net::SocketAddr) -> String {
        let mut s = TcpStream::connect(addr).await.unwrap();
        s.write_all(REQ).await.unwrap();
        let mut out = String::new();
        tokio::time::timeout(std::time::Duration::from_secs(5), s.read_to_string(&mut out))
            .await.expect("scrape timed out").unwrap();
        out
    }

    fn cfg(max_conns: usize, io_timeout_ms: u64) -> MetricsServerConfig {
        MetricsServerConfig { max_conns, io_timeout: std::time::Duration::from_millis(io_timeout_ms) }
    }
```

Tests (each names the retired test it re-expresses):

```rust
    /// Re-expresses `idle_client_does_not_block_a_second_scraper`.
    #[tokio::test]
    async fn idle_client_does_not_block_a_second_scraper() {
        let (srv, _tx) = start(cfg(8, 3_000)).await;
        let _idle = TcpStream::connect(srv.addr).await.unwrap();
        let body = scrape(srv.addr).await;
        assert!(body.starts_with("HTTP/1.1 200 OK"), "{body}");
        assert!(body.contains("media_doctor_packets_total 0"), "{body}");
        srv.token.cancel();
        srv.task.await.unwrap();
    }

    /// Re-expresses `repeated_scrapes_all_succeed_with_a_stalled_client_outstanding`.
    #[tokio::test]
    async fn repeated_scrapes_succeed_with_stalled_clients_outstanding() {
        let (srv, _tx) = start(cfg(8, 3_000)).await;
        let mut idle = Vec::new();
        for _ in 0..3 { idle.push(TcpStream::connect(srv.addr).await.unwrap()); }
        for round in 0..3 {
            assert!(scrape(srv.addr).await.starts_with("HTTP/1.1 200 OK"), "round {round}");
        }
        srv.token.cancel();
        srv.task.await.unwrap();
    }

    /// Re-expresses `connections_beyond_the_cap_are_refused` and
    /// `refusal_response_uses_crlf_framing`: the refusal is now written by
    /// hyper, so the CRLF-framing test is retired (see Task 12's table); this
    /// test pins the 503.
    #[tokio::test]
    async fn connection_beyond_the_cap_is_answered_503() {
        const CAP: usize = 2;
        let (srv, _tx) = start(cfg(CAP, 60_000)).await;
        // Connect sequentially: the accept loop admits in connect order, so
        // the first CAP connections own the permits (no probing needed).
        let mut held = Vec::new();
        for _ in 0..CAP { held.push(TcpStream::connect(srv.addr).await.unwrap()); }
        let over = scrape(srv.addr).await;
        assert!(over.starts_with("HTTP/1.1 503"), "{over}");
        drop(held);
        srv.token.cancel();
        srv.task.await.unwrap();
    }

    /// Re-expresses `connection_flood_beyond_the_cap_still_serves_after_it_drains`.
    #[tokio::test]
    async fn flood_beyond_the_cap_drains_and_serving_resumes() {
        const CAP: usize = 4;
        let (srv, _tx) = start(cfg(CAP, 400)).await;
        let mut flood = Vec::new();
        for _ in 0..CAP * 6 { flood.push(TcpStream::connect(srv.addr).await.unwrap()); }
        drop(flood);
        // Condition-wait with a bound (no fixed sleep): retry until a scrape is 200.
        let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(30);
        loop {
            if scrape(srv.addr).await.starts_with("HTTP/1.1 200 OK") { break; }
            assert!(tokio::time::Instant::now() < deadline, "never recovered after the flood");
            tokio::task::yield_now().await;
        }
        srv.token.cancel();
        srv.task.await.unwrap();
    }

    /// Re-expresses `dribbling_client_is_dropped_at_the_total_deadline`. The
    /// dribbler paces itself with an interval (pacing the client is the
    /// behaviour under test, not a wait for a condition); the assertion is
    /// that the server ends the connection near `io_timeout`.
    #[tokio::test]
    async fn dribbling_client_is_dropped_at_the_deadline() {
        const IO_TIMEOUT_MS: u64 = 300;
        let (srv, _tx) = start(cfg(8, IO_TIMEOUT_MS)).await;
        let mut s = TcpStream::connect(srv.addr).await.unwrap();
        let start = tokio::time::Instant::now();
        let mut tick = tokio::time::interval(std::time::Duration::from_millis(40));
        let mut buf = [0u8; 256];
        let closed_after = loop {
            tokio::select! {
                _ = tick.tick() => { if s.write_all(b"G").await.is_err() { break start.elapsed(); } }
                r = s.read(&mut buf) => match r {
                    Ok(0) | Err(_) => break start.elapsed(),
                    Ok(n) => {
                        // hyper may answer a timed-out request head with a
                        // status line before closing; that is a close, not a metrics body.
                        assert!(!String::from_utf8_lossy(&buf[..n]).contains("media_doctor_packets_total"),
                            "an incomplete request head must not get the metrics body");
                    }
                },
            }
            assert!(start.elapsed() < std::time::Duration::from_secs(10), "server never dropped the dribbler");
        };
        assert!(closed_after >= std::time::Duration::from_millis(IO_TIMEOUT_MS / 2), "dropped far too early: {closed_after:?}");
        srv.token.cancel();
        srv.task.await.unwrap();
    }

    /// New: shutdown ends `serve`, and the port is free again at once
    /// (the accept pump must not leak its bound socket — spec defect class 3).
    #[tokio::test]
    async fn shutdown_releases_the_port() {
        let (srv, _tx) = start(cfg(8, 3_000)).await;
        let addr = srv.addr;
        let _idle = TcpStream::connect(addr).await.unwrap();
        srv.token.cancel();
        srv.task.await.unwrap();
        std::net::TcpListener::bind(addr).expect("port must be released after shutdown");
    }

    /// New: a body published after start is served by the next scrape.
    #[tokio::test]
    async fn scrape_serves_the_latest_published_body() {
        let (srv, tx) = start(cfg(8, 3_000)).await;
        tx.send_replace(Arc::<str>::from("# TYPE x gauge\nx 42\n"));
        assert!(scrape(srv.addr).await.contains("\nx 42\n"));
        srv.token.cancel();
        srv.task.await.unwrap();
    }

    /// Re-expresses the HTTP golden (Task 1) semantically: same status,
    /// same content type, `Content-Length` equals the body, `Connection:
    /// close`; hyper additionally sends `date` and lower-cases names.
    #[tokio::test]
    async fn response_head_matches_the_main_golden_after_normalisation() {
        let (srv, _tx) = start(cfg(8, 3_000)).await;
        let text = scrape(srv.addr).await;
        let (head, body) = text.split_once("\r\n\r\n").unwrap();
        let norm = |h: &str| {
            let mut lines: Vec<String> = h.lines().skip(1)
                .map(|l| l.to_ascii_lowercase())
                .filter(|l| !l.starts_with("date:") && !l.starts_with("content-length:"))
                .collect();
            lines.sort();
            lines
        };
        let golden = std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"),
            "/tests/golden/watch/http-response-head.txt")).unwrap();
        assert_eq!(head.lines().next(), golden.lines().next(), "status line");
        assert_eq!(norm(head), norm(golden.trim_end()), "headers");
        assert!(head.to_ascii_lowercase().contains(&format!("content-length: {}", body.len())));
        srv.token.cancel();
        srv.task.await.unwrap();
    }

    /// Publisher cadence is driven by the caller's clock (deterministic).
    #[test]
    fn publisher_renders_at_most_once_per_interval_and_flushes_on_demand() {
        let state = WatchState::new();
        let (mut p, rx) = channel(&state, std::time::Duration::from_millis(250));
        let t = std::time::Duration::from_millis;
        assert!(p.maybe_publish(&state, t(0)),    "first call publishes");
        assert!(!p.maybe_publish(&state, t(100)), "inside the interval: skipped");
        assert!(!p.maybe_publish(&state, t(249)));
        assert!(p.maybe_publish(&state, t(250)),  "at the interval: published");
        p.publish_now(&state, t(251));
        assert!(rx.borrow().contains("media_doctor_packets_total 0"), "fresh state exposes every family");
    }
```

- [ ] **Step 3: Run — expect FAIL (compile: module does not exist)**

```bash
cargo test --locked -p media-doctor --features net --lib metrics_server 2>&1 | grep -E '^error' | head -3
```

- [ ] **Step 4: Implement `metrics_server.rs`**

```rust
//! The `media-doctor watch` metrics HTTP server (de-hand-roll W1-P, SP2.3).
//!
//! hyper does all HTTP framing; this module only decides *who may connect and
//! for how long* — the policy the old thread-per-connection server enforced
//! (audit MD-W9): a concurrency cap (`max_conns`; over-cap peers get `503`),
//! a header-read timeout and a total per-connection deadline (`io_timeout`)
//! so a dribbling or idle peer cannot hold a connection, and a
//! `CancellationToken` that stops the accept loop and drops the listener.
//!
//! Tasks are owned by a [`TaskTracker`]; `serve` returns only after every
//! connection task has ended.

use std::convert::Infallible;
use std::sync::Arc;
use std::time::Duration;

use http_body_util::Full;
use hyper::body::{Bytes, Incoming};
use hyper::header::{CONTENT_TYPE, HeaderValue};
use hyper::server::conn::http1;
use hyper::service::service_fn;
use hyper::{Request, Response, StatusCode};
use hyper_util::rt::{TokioIo, TokioTimer};
use tokio::net::TcpListener;
use tokio::sync::{Semaphore, watch};
use tokio_util::sync::CancellationToken;
use tokio_util::task::TaskTracker;

use crate::WatchState;

/// Prometheus text exposition content type (format 0.0.4), as the old server sent.
const EXPOSITION_CONTENT_TYPE: &str = "text/plain; version=0.0.4";

/// Pause after a failed `accept()` (EMFILE under a connection storm, a peer that
/// aborted between connect and accept) so a persistent error cannot spin the loop.
const ACCEPT_ERROR_BACKOFF: Duration = Duration::from_millis(50);

/// Default concurrent-connection cap (matches the old CLI default).
pub const DEFAULT_MAX_CONNS: usize = 32;
/// Default total per-connection deadline (matches the old CLI default).
pub const DEFAULT_IO_TIMEOUT: Duration = Duration::from_millis(5_000);

/// Server limits.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub struct MetricsServerConfig {
    /// Connections served with the metrics body at once; further peers get `503`.
    pub max_conns: usize,
    /// Both the header-read timeout and the total deadline per connection.
    pub io_timeout: Duration,
}

impl Default for MetricsServerConfig {
    fn default() -> Self {
        Self { max_conns: DEFAULT_MAX_CONNS, io_timeout: DEFAULT_IO_TIMEOUT }
    }
}

/// Renders [`WatchState`] into the shared body at most once per `interval` of
/// the caller's clock, so the ingest loop pays for exposition at the scrape
/// scale, not the datagram scale.
pub struct MetricsPublisher {
    tx: watch::Sender<Arc<str>>,
    interval: Duration,
    last: Option<Duration>,
}

/// Create the publisher and the receiver [`serve`] reads. The initial body is
/// rendered from `initial`, so the very first scrape already lists every family.
#[must_use]
pub fn channel(initial: &WatchState, interval: Duration) -> (MetricsPublisher, watch::Receiver<Arc<str>>) {
    let (tx, rx) = watch::channel(Arc::<str>::from(crate::render_metrics(initial)));
    (MetricsPublisher { tx, interval, last: None }, rx)
}

impl MetricsPublisher {
    /// Publish if `interval` has elapsed since the last publish on `clock`.
    pub fn maybe_publish(&mut self, state: &WatchState, clock: Duration) -> bool {
        let due = self.last.is_none_or(|last| clock.saturating_sub(last) >= self.interval);
        if due {
            self.publish_now(state, clock);
        }
        due
    }

    /// Publish unconditionally (call when the feed goes quiet so the final
    /// figures become visible).
    pub fn publish_now(&mut self, state: &WatchState, clock: Duration) {
        self.tx.send_replace(Arc::<str>::from(crate::render_metrics(state)));
        self.last = Some(clock);
    }
}

fn respond(body: Option<Arc<str>>) -> Response<Full<Bytes>> {
    match body {
        Some(text) => {
            let mut r = Response::new(Full::new(Bytes::from(text.as_bytes().to_vec())));
            r.headers_mut().insert(CONTENT_TYPE, HeaderValue::from_static(EXPOSITION_CONTENT_TYPE));
            r
        }
        None => {
            let mut r = Response::new(Full::new(Bytes::new()));
            *r.status_mut() = StatusCode::SERVICE_UNAVAILABLE;
            r.headers_mut().insert(CONTENT_TYPE, HeaderValue::from_static("text/plain"));
            r
        }
    }
}

/// Serve `GET /metrics` (and, this being a single-endpoint probe, any other
/// request) until `shutdown` is cancelled. See the module docs for the limits.
pub async fn serve(
    listener: TcpListener,
    metrics: watch::Receiver<Arc<str>>,
    config: MetricsServerConfig,
    shutdown: CancellationToken,
) {
    let permits = Arc::new(Semaphore::new(config.max_conns));
    let tracker = TaskTracker::new();
    loop {
        let accepted = tokio::select! {
            () = shutdown.cancelled() => break,
            accepted = listener.accept() => accepted,
        };
        let stream = match accepted {
            Ok((stream, _peer)) => stream,
            Err(e) => {
                eprintln!("media-doctor watch: metrics accept error: {e}");
                tokio::time::sleep(ACCEPT_ERROR_BACKOFF).await;
                continue;
            }
        };
        // Taken here, in accept order, so "the first N connections own the N
        // permits" is deterministic; released when the connection task ends.
        let permit = Arc::clone(&permits).try_acquire_owned().ok();
        let metrics = metrics.clone();
        let token = shutdown.clone();
        tracker.spawn(async move {
            let admitted = permit.is_some();
            let service = service_fn(move |_req: Request<Incoming>| {
                let body = admitted.then(|| Arc::clone(&metrics.borrow()));
                async move { Ok::<_, Infallible>(respond(body)) }
            });
            let mut http = http1::Builder::new();
            http.timer(TokioTimer::new())
                .header_read_timeout(config.io_timeout)
                .keep_alive(false);
            let conn = http.serve_connection(TokioIo::new(stream), service);
            tokio::select! {
                result = tokio::time::timeout(config.io_timeout, conn) => {
                    if let Ok(Err(e)) = result {
                        eprintln!("media-doctor watch: metrics request error: {e}");
                    }
                }
                () = token.cancelled() => {}
            }
            drop(permit);
        });
    }
    tracker.close();
    tracker.wait().await;
}
```

Notes the implementer must verify rather than assume: `watch::Ref` is held across no `.await` (the `borrow()` is inside the non-async closure body before the `async move`); `Option::is_none_or` is stable since 1.82; the eprintln! and `sleep` in this non-test async code are the two allowlist entries in Task 13's guard.

`lib.rs`: `#[cfg(feature = "net")] pub mod metrics_server;`.

- [ ] **Step 5: Run — expect PASS, then 20 consecutive runs**

```bash
cargo test --locked -p media-doctor --features net --lib metrics_server 2>&1 | grep -E '^test |test result|FAILED'
for i in $(seq 20); do cargo test --locked -p media-doctor --features net --lib metrics_server 2>&1 | grep -E '^test result'; done | sort | uniq -c
cargo clippy --locked -p media-doctor --all-features --all-targets -- -D warnings
```

Expected: 9 passed ×20.

- [ ] **Step 6: Revert-check each guarantee** (each mutation must produce the named failure; record):
  1. Delete `.timer(TokioTimer::new())` → `dribbling_client_is_dropped_at_the_deadline` still passes because of the outer `tokio::time::timeout`; therefore ALSO delete the `tokio::time::timeout(config.io_timeout, conn)` wrapper (keep `select!` with `conn` directly) and expect the dribbler test to fail ("server never dropped the dribbler" after 10 s). This proves each of the two defences is individually necessary only together — record that finding; do **not** simplify either away.
  2. `Semaphore::new(config.max_conns + 1000)` → `connection_beyond_the_cap_is_answered_503` fails.
  3. Remove `drop(listener)` indirectly by `std::mem::forget(listener)` at the end of `serve` → `shutdown_releases_the_port` fails.
  4. `last.is_none_or(..)` → `true` → the cadence test fails on `!p.maybe_publish(.., t(100))`.
  5. `keep_alive(false)` removed → `response_head_matches_the_main_golden_after_normalisation` fails (`connection: close` missing).

- [ ] **Step 7: List the HTTP head differences for the CHANGELOG** from the Step 6 test (with a captured example): header names are lower-case, a `date:` header is present, header order differs, `content-length` unchanged, `connection: close` unchanged, `content-type: text/plain; version=0.0.4` unchanged. Paste the old and new heads into `.delegate/w1-p-report.md`.

- [ ] **Step 8: Commit**

```bash
git add media-doctor Cargo.lock
git commit -m "feat(media-doctor): hyper-based metrics server with connection cap, header-read and total deadlines, token shutdown"
```

---

### Task 11: media-doctor — UDP/multicast bind via socket2 (SP1.6)

**Files:**
- Create: `media-doctor/src/udp.rs`
- Modify: `media-doctor/src/lib.rs` (`#[cfg(feature = "net")] pub mod udp;`)

**Interfaces (exact):**

```rust
#[derive(Debug, Clone, Default, PartialEq, Eq)] #[non_exhaustive]
pub struct UdpConfig {
    /// `SO_RCVBUF` request in bytes; `None` leaves the OS default.
    pub recv_buffer: Option<usize>,
    /// `SO_REUSEADDR` (and `SO_REUSEPORT` on Apple targets, where sharing a multicast port needs it). Default `false`.
    pub reuse_addr: bool,
    /// Interface for the multicast join (`None` = OS choice).
    pub interface: Option<MulticastInterface>,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)] #[non_exhaustive]
pub enum MulticastInterface { V4(std::net::Ipv4Addr), V6Index(u32) }
// label convention: name() + impl_spec_display!? — this is a config selector, not a spec/field enum:
// add `MulticastInterface` to the SKIP list in tests/label_coverage.rs with that reason.
pub fn bind_udp(addr: std::net::SocketAddr, config: &UdpConfig) -> std::io::Result<std::net::UdpSocket>;
pub fn recv_buffer_size(socket: &std::net::UdpSocket) -> std::io::Result<usize>;
```

Behaviour carried over from the old `bind_udp` (`media-doctor.rs:295-312`): a multicast address binds the wildcard on the port, then joins the group; unicast binds `addr` itself. New: IPv6 multicast joins with `join_multicast_v6`; `reuse_addr`; `recv_buffer`; `interface`.

- [ ] **Step 1: Write the failing tests** (in `udp.rs` `mod tests`):

```rust
    use super::*;
    use std::net::{Ipv4Addr, SocketAddr, UdpSocket};

    fn loopback0() -> SocketAddr { "127.0.0.1:0".parse().unwrap() }

    #[test]
    fn unicast_binds_the_requested_address_on_port_zero() {
        let s = bind_udp(loopback0(), &UdpConfig::default()).unwrap();
        let a = s.local_addr().unwrap();
        assert_eq!(a.ip(), Ipv4Addr::LOCALHOST);
        assert_ne!(a.port(), 0, "the kernel-assigned port is reported");
    }

    #[test]
    fn second_bind_to_the_same_port_fails_without_reuse_addr() {
        let first = bind_udp(loopback0(), &UdpConfig::default()).unwrap();
        let addr = first.local_addr().unwrap();
        let err = bind_udp(addr, &UdpConfig::default()).unwrap_err();
        assert_eq!(err.kind(), std::io::ErrorKind::AddrInUse);
    }

    #[test]
    fn reuse_addr_allows_a_second_bind_to_the_same_port() {
        let cfg = UdpConfig { reuse_addr: true, ..UdpConfig::default() };
        let first = bind_udp(loopback0(), &cfg).unwrap();
        let addr = first.local_addr().unwrap();
        bind_udp(addr, &cfg).expect("SO_REUSEADDR must allow the second bind");
    }

    /// The kernel may cap the request (`rmem_max`) and Linux doubles it, so
    /// assert only what is guaranteed: a large request grows the buffer over
    /// the default, and the readback API works.
    #[test]
    fn recv_buffer_request_is_applied_or_capped_but_never_ignored() {
        let default = recv_buffer_size(&bind_udp(loopback0(), &UdpConfig::default()).unwrap()).unwrap();
        let want = 4 * 1024 * 1024;
        let cfg = UdpConfig { recv_buffer: Some(want), ..UdpConfig::default() };
        let got = recv_buffer_size(&bind_udp(loopback0(), &cfg).unwrap()).unwrap();
        assert!(got >= default, "requested {want}, got {got}, default {default}");
    }

    #[test]
    fn datagrams_arrive_on_the_bound_socket() {
        let rx = bind_udp(loopback0(), &UdpConfig::default()).unwrap();
        rx.set_read_timeout(Some(std::time::Duration::from_secs(5))).unwrap();
        let tx = UdpSocket::bind("127.0.0.1:0").unwrap();
        tx.send_to(b"\x47ts", rx.local_addr().unwrap()).unwrap();
        let mut buf = [0u8; 16];
        let (n, _) = rx.recv_from(&mut buf).unwrap();
        assert_eq!(&buf[..n], b"\x47ts");
    }

    #[test]
    fn interface_kind_must_match_the_group_family() {
        let cfg = UdpConfig { interface: Some(MulticastInterface::V6Index(1)), ..UdpConfig::default() };
        let err = bind_udp("239.255.42.42:0".parse().unwrap(), &cfg).unwrap_err();
        assert_eq!(err.kind(), std::io::ErrorKind::InvalidInput);
        let cfg = UdpConfig { interface: Some(MulticastInterface::V4(Ipv4Addr::LOCALHOST)), ..UdpConfig::default() };
        let err = bind_udp("[ff02::1234]:0".parse().unwrap(), &cfg).unwrap_err();
        assert_eq!(err.kind(), std::io::ErrorKind::InvalidInput);
    }

    /// Joining a group on the loopback interface needs a multicast-capable
    /// route that CI containers sometimes lack; skip loudly (the repo's
    /// oracle-test convention) rather than pass silently or flake.
    #[test]
    fn ipv4_multicast_group_is_joined_on_the_requested_interface() {
        let cfg = UdpConfig { reuse_addr: true, interface: Some(MulticastInterface::V4(Ipv4Addr::LOCALHOST)), ..UdpConfig::default() };
        let rx = match bind_udp("239.255.42.42:0".parse().unwrap(), &cfg) {
            Ok(s) => s,
            Err(e) => { eprintln!("SKIP multicast join: {e} (no multicast route on lo)"); return; }
        };
        let port = rx.local_addr().unwrap().port();
        rx.set_read_timeout(Some(std::time::Duration::from_secs(3))).unwrap();
        let tx = UdpSocket::bind("127.0.0.1:0").unwrap();
        tx.set_multicast_if_v4(&Ipv4Addr::LOCALHOST).unwrap();
        tx.set_multicast_loop_v4(true).unwrap();
        if tx.send_to(b"mc", ("239.255.42.42", port)).is_err() { eprintln!("SKIP multicast send"); return; }
        let mut buf = [0u8; 8];
        match rx.recv_from(&mut buf) {
            Ok((n, _)) => assert_eq!(&buf[..n], b"mc"),
            Err(e) => eprintln!("SKIP multicast receive: {e}"),
        }
    }
```

- [ ] **Step 2: Run — expect FAIL (compile: no `udp` module)**

- [ ] **Step 3: Implement `udp.rs`**

```rust
//! UDP/multicast bind for `media-doctor watch` (de-hand-roll W1-P, SP1.6):
//! `socket2` called directly, with configurable `SO_RCVBUF`, `SO_REUSEADDR`
//! and the multicast interface. No shared wrapper crate (spec §4 SP1.6).

use std::io;
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr, UdpSocket};

use socket2::{Domain, Protocol, SockRef, Socket, Type};

/// ... (config struct and enum exactly as in the Interfaces block, with docs) ...

/// Bind a UDP socket for `addr`; a multicast `addr` binds the wildcard on its
/// port and joins the group (on [`UdpConfig::interface`] when given).
pub fn bind_udp(addr: SocketAddr, config: &UdpConfig) -> io::Result<UdpSocket> {
    let (bind_ip, group) = match addr.ip() {
        IpAddr::V4(ip) if ip.is_multicast() => (IpAddr::V4(Ipv4Addr::UNSPECIFIED), Some(IpAddr::V4(ip))),
        IpAddr::V6(ip) if ip.is_multicast() => (IpAddr::V6(Ipv6Addr::UNSPECIFIED), Some(IpAddr::V6(ip))),
        ip => (ip, None),
    };
    // Reject a mismatched interface before any socket exists.
    match (group, config.interface) {
        (Some(IpAddr::V4(_)), Some(MulticastInterface::V6Index(_)))
        | (Some(IpAddr::V6(_)), Some(MulticastInterface::V4(_))) => {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "multicast interface kind does not match the group's address family",
            ));
        }
        _ => {}
    }
    let socket = Socket::new(Domain::for_address(addr), Type::DGRAM, Some(Protocol::UDP))?;
    if config.reuse_addr {
        socket.set_reuse_address(true)?;
        // Sharing a multicast port on macOS/BSD needs SO_REUSEPORT too; on
        // Linux it has load-balancing semantics we do not want.
        #[cfg(target_vendor = "apple")]
        socket.set_reuse_port(true)?;
    }
    if let Some(bytes) = config.recv_buffer {
        socket.set_recv_buffer_size(bytes)?;
    }
    socket.bind(&SocketAddr::new(bind_ip, addr.port()).into())?;
    match group {
        Some(IpAddr::V4(ip)) => {
            let iface = match config.interface {
                Some(MulticastInterface::V4(i)) => i,
                _ => Ipv4Addr::UNSPECIFIED,
            };
            socket.join_multicast_v4(&ip, &iface)?;
        }
        Some(IpAddr::V6(ip)) => {
            let index = match config.interface {
                Some(MulticastInterface::V6Index(i)) => i,
                _ => 0,
            };
            socket.join_multicast_v6(&ip, index)?;
        }
        None => {}
    }
    Ok(socket.into())
}

/// The kernel's actual `SO_RCVBUF` (Linux reports twice the request and caps at `rmem_max`).
pub fn recv_buffer_size(socket: &UdpSocket) -> io::Result<usize> {
    SockRef::from(socket).recv_buffer_size()
}
```

Verify `set_reuse_port` is available for apple targets in `socket2-0.6.5/src/sys/unix.rs:2247` (it is `cfg`-gated by platform list; the plan's `target_vendor = "apple"` is a subset of it). Add the `label_coverage` SKIP entry for `MulticastInterface` in `media-doctor/tests/label_coverage.rs` (read the file's SKIP list syntax first).

- [ ] **Step 4: Run — expect PASS, 20 runs, clippy**

```bash
cargo test --locked -p media-doctor --features net --lib udp 2>&1 | grep -E '^test |test result|FAILED|SKIP'
for i in $(seq 20); do cargo test --locked -p media-doctor --features net --lib udp 2>&1 | grep -E '^test result'; done | sort | uniq -c
cargo clippy --locked -p media-doctor --all-features --all-targets -- -D warnings
cargo test --locked -p media-doctor --all-features --test label_coverage
```

Expected: 7 passed ×20 (a "SKIP" line for multicast is acceptable and must be listed in the report with the machine).

- [ ] **Step 5: Linux check of the same tests** (the multicast path differs): run `cargo test -p media-doctor --features net --lib udp` inside the Task 0 Docker recipe (change the test line in `run.sh`) — record whether the multicast test ran or skipped.

- [ ] **Step 6: Revert-check.** (a) Remove `socket.set_reuse_address(true)?;` → `reuse_addr_allows_a_second_bind_to_the_same_port` fails. (b) Make the bind ignore `config.recv_buffer` → `recv_buffer_request_is_applied_or_capped_but_never_ignored` fails (4 MiB vs default; if the kernel cap makes `got == default` on this machine, record that and use `got > default || cap_active` — decide from the observed numbers and state which). (c) Delete the family-mismatch guard → `interface_kind_must_match_the_group_family` fails. Restore.

- [ ] **Step 7: Commit**

```bash
git add media-doctor
git commit -m "feat(media-doctor): socket2 UDP/multicast bind with configurable SO_RCVBUF, SO_REUSEADDR and interface"
```

---

### Task 12: media-doctor — the binary, CLI flags, and the retired/re-expressed server tests

**Files:**
- Modify: `media-doctor/src/bin/media-doctor.rs`: imports 3–10; `run_watch` 230–291; delete `bind_udp` 293–312, `MetricsConfig` 313–320, `serve_metrics` 322–383, `ACCEPT_ERROR_BACKOFF`, `refuse_connection`, `MIN_SOCKET_TIMEOUT`, `HEADER_TERMINATOR`, `MAX_REQUEST_BYTES`, `SERVICE_UNAVAILABLE_RESPONSE`, `reserve_connection`, `ConnectionGuard`, `handle_metrics_request` (to end of file, 541)
- Modify: `media-doctor/src/cli.rs` (`WatchArgs` 43–66, plus a `udp_config()` helper)
- Replace: `media-doctor/tests/watch_metrics_server.rs` (439 lines) with a binary smoke test
- Delete: `media-doctor/tests/watch_metrics_golden.rs` and `media-doctor/tests/golden/watch/http-response-head.txt` stays (used by Task 10's test)
- Modify: `media-doctor/README.md` lines 46–92

**Interfaces:** new CLI flags on `watch` (all named, per `docs/CLI-STANDARD.md`):

```rust
    /// Requested SO_RCVBUF for the UDP socket, in bytes (the kernel may cap it).
    #[arg(long = "udp-rcvbuf")] pub udp_rcvbuf: Option<usize>,
    /// Set SO_REUSEADDR on the UDP socket (lets several probes share a multicast port).
    #[arg(long = "udp-reuse-addr")] pub udp_reuse_addr: bool,
    /// Interface for the multicast join: an IPv4 address (IPv4 groups) or an interface index (IPv6 groups).
    #[arg(long = "udp-interface")] pub udp_interface: Option<String>,
```

`impl WatchArgs { pub fn udp_config(&self) -> Result<UdpConfig, String> }` (feature `net`): parses `--udp-interface` — an `Ipv4Addr` parse first, else a `u32` index, else an error naming the flag. Existing flags `--udp`, `--metrics-addr`, `--metrics-max-conns`, `--metrics-io-timeout-ms` and their defaults are unchanged (no CLI break).

- [ ] **Step 1: Disposition of the old `watch_metrics_server.rs` tests — the table every reviewer checks.** Put this table in the CHANGELOG-adjacent `.delegate/w1-p-report.md` and as the module doc of the replacement test file:

| Old test (`tests/watch_metrics_server.rs`) | Disposition | New test |
|---|---|---|
| `idle_client_does_not_block_a_second_scraper` (197) | **Re-expressed** | `metrics_server::tests::idle_client_does_not_block_a_second_scraper` |
| `repeated_scrapes_all_succeed_with_a_stalled_client_outstanding` (223) | **Re-expressed** | `…::repeated_scrapes_succeed_with_stalled_clients_outstanding` |
| `connections_beyond_the_cap_are_refused` (245) | **Re-expressed** (503, deterministic accept order, no probe loop / no sleeps) | `…::connection_beyond_the_cap_is_answered_503` |
| `connection_flood_beyond_the_cap_still_serves_after_it_drains` (≈290) | **Re-expressed** | `…::flood_beyond_the_cap_drains_and_serving_resumes` |
| `dribbling_client_is_dropped_at_the_total_deadline` (≈320) | **Re-expressed** (this is NOT made moot by hyper: hyper ignores `header_read_timeout` unless a timer is set; the exporter's own listener sets none) | `…::dribbling_client_is_dropped_at_the_deadline` |
| `refusal_response_uses_crlf_framing` (≈395) | **Retired**: it pinned hand-written `HTTP/1.1 503\r\n…` bytes; hyper now writes every status line and header, so a CRLF defect in this crate is impossible by construction. The 503 itself is still pinned (cap test) | — |
| helpers `free_port`, `spawn_watch`'s readiness poll, `hold_connections`, `all_open`, `PROBE_PATIENCE` | **Retired**: reserve-then-rebind port allocation and accept-order probing existed only because the old server could not report its bound address or admit deterministically (SP7 item 1). Replaced by port-0 listeners passed into `serve` | — |
| (new, no old equivalent) | shutdown releases the port; latest body served; cadence | `shutdown_releases_the_port`, `scrape_serves_the_latest_published_body`, `publisher_renders_at_most_once_per_interval_and_flushes_on_demand` |

Retired = each has a reason above; nothing is retired for being inconvenient.

- [ ] **Step 2: Write the failing binary smoke test** — replace `tests/watch_metrics_server.rs` with:

```rust
//! End-to-end: the real `media-doctor watch` binary. Ports are kernel-assigned
//! (`:0`) and learnt from the binary's own start-up line — no reserve-then-
//! rebind, no readiness poll, no sleeps. The server's connection-policy
//! behaviours are tested in-process in `src/metrics_server.rs`; see the
//! disposition table in this PR's report for every retired/re-expressed
//! former test.

use std::io::{BufRead, BufReader, Read, Write};
use std::net::{SocketAddr, TcpStream, UdpSocket};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

const TS_PACKET_SIZE: usize = 188;
const DATAGRAM_PACKETS: usize = 7;

struct ChildGuard(Child);
impl Drop for ChildGuard {
    fn drop(&mut self) { let _ = self.0.kill(); let _ = self.0.wait(); }
}

/// Spawn `watch` on port 0 for both sockets and return the addresses it printed.
fn spawn_watch(extra: &[&str]) -> (ChildGuard, SocketAddr, SocketAddr) {
    let mut child = Command::new(env!("CARGO_BIN_EXE_media-doctor"))
        .args(["watch", "--udp", "127.0.0.1:0", "--metrics-addr", "127.0.0.1:0"])
        .args(extra)
        .stdout(Stdio::null()).stderr(Stdio::piped()).spawn().expect("spawn watch");
    let stderr = child.stderr.take().unwrap();
    let guard = ChildGuard(child);
    let mut line = String::new();
    BufReader::new(stderr).read_line(&mut line).expect("start-up line");
    // "media-doctor watch: ingesting UDP 127.0.0.1:PORT, metrics on http://127.0.0.1:PORT/metrics"
    let udp = line.split("UDP ").nth(1).and_then(|s| s.split(',').next()).expect(&line).parse().expect(&line);
    let http = line.split("http://").nth(1).and_then(|s| s.split("/metrics").next()).expect(&line).parse().expect(&line);
    (guard, udp, http)
}

fn scrape(addr: SocketAddr) -> String {
    let mut s = TcpStream::connect(addr).expect("connect");
    s.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
    s.write_all(b"GET /metrics HTTP/1.1\r\nHost: localhost\r\n\r\n").unwrap();
    let mut out = String::new();
    let _ = s.read_to_string(&mut out);
    out
}

fn metric(text: &str, name: &str) -> Option<f64> {
    text.lines().find_map(|l| l.strip_prefix(name)?.strip_prefix(' ')?.trim().parse().ok())
}

#[test]
fn first_scrape_already_lists_every_family() {
    let (_g, _udp, http) = spawn_watch(&[]);
    let body = scrape(http);
    assert!(body.starts_with("HTTP/1.1 200 OK"), "{body}");
    assert_eq!(metric(&body, "media_doctor_packets_total"), Some(0.0), "{body}");
}

/// The last datagrams before the feed goes quiet must still be published
/// (the binary flushes when its socket read times out).
#[test]
fn final_datagrams_become_visible_after_the_feed_stops() {
    let (_g, udp, http) = spawn_watch(&[]);
    let bytes = std::fs::read(concat!(env!("CARGO_MANIFEST_DIR"), "/../fixtures/ts/m6-single.ts")).unwrap();
    let packets = bytes.len() / TS_PACKET_SIZE;
    let tx = UdpSocket::bind("127.0.0.1:0").unwrap();
    for chunk in bytes.chunks(DATAGRAM_PACKETS * TS_PACKET_SIZE) {
        tx.send_to(chunk, udp).unwrap();
    }
    // Condition-wait, bounded; each scrape is one real round trip.
    let deadline = Instant::now() + Duration::from_secs(30);
    loop {
        let body = scrape(http);
        // UDP on loopback can drop under load; require "most", and stability of the flush.
        if metric(&body, "media_doctor_packets_total").is_some_and(|n| n >= (packets as f64) * 0.5) { return; }
        assert!(Instant::now() < deadline, "packets never became visible: {body}");
    }
}

#[test]
fn bad_metrics_address_is_a_startup_error_not_a_hang() {
    let out = Command::new(env!("CARGO_BIN_EXE_media-doctor"))
        .args(["watch", "--udp", "127.0.0.1:0", "--metrics-addr", "not-an-address"])
        .output().unwrap();
    assert!(!out.status.success());
    assert!(String::from_utf8_lossy(&out.stderr).contains("--metrics-addr"));
}

#[test]
fn udp_interface_flag_is_validated() {
    let out = Command::new(env!("CARGO_BIN_EXE_media-doctor"))
        .args(["watch", "--udp", "127.0.0.1:0", "--udp-interface", "not-an-interface"])
        .output().unwrap();
    assert!(!out.status.success());
    assert!(String::from_utf8_lossy(&out.stderr).contains("--udp-interface"));
}
```

(The `>= 50 %` bound is deliberate: loopback UDP can drop under load and the test is about *flush after quiet*, not delivery. If on the dev machine delivery is lossless, tighten to `== packets` and record that.)

- [ ] **Step 3: Run — expect FAIL against the old bin** (the old bin prints `ingesting UDP 127.0.0.1:0` — port 0 — so address parsing yields port 0, the first test cannot connect; `--udp-interface` is an unknown flag):

```bash
cargo test --locked -p media-doctor --all-features --test watch_metrics_server 2>&1 | grep -E '^test |FAILED|panicked' | head
```

- [ ] **Step 4: Implement the binary.** Replace `run_watch` with:

```rust
/// `media-doctor watch` — the socket glue around [`media_doctor::WatchState`]
/// (issue #665). Ingest and accounting live in the library; HTTP framing is
/// hyper's (`metrics_server`), the Prometheus text is the exporter's
/// (`watch_metrics`), the UDP bind is socket2's (`udp`). This function owns
/// only the wiring: a blocking ingest loop on the main thread and a
/// current-thread tokio runtime on a `metrics` thread serving the latest body.
fn run_watch(args: &WatchArgs) -> Result<(), Box<dyn std::error::Error>> {
    let udp_addr: SocketAddr = args.udp.parse()
        .map_err(|e| format!("invalid --udp address {:?}: {e}", args.udp))?;
    let metrics_addr: SocketAddr = args.metrics_addr.parse()
        .map_err(|e| format!("invalid --metrics-addr address {:?}: {e}", args.metrics_addr))?;
    let udp_config = args.udp_config()?;

    let socket = media_doctor::udp::bind_udp(udp_addr, &udp_config)?;
    if let (Some(want), Ok(got)) = (udp_config.recv_buffer, media_doctor::udp::recv_buffer_size(&socket)) {
        if got < want {
            eprintln!("media-doctor watch: warning: SO_RCVBUF requested {want} bytes, kernel granted {got}");
        }
    }
    socket.set_read_timeout(Some(PUBLISH_INTERVAL))?;
    let listener = std::net::TcpListener::bind(metrics_addr)?;
    listener.set_nonblocking(true)?;
    eprintln!(
        "media-doctor watch: ingesting UDP {}, metrics on http://{}/metrics",
        socket.local_addr()?, listener.local_addr()?
    );

    let mut state = WatchState::new();
    let (mut publisher, receiver) = metrics_server::channel(&state, PUBLISH_INTERVAL);
    let shutdown = CancellationToken::new();
    let config = MetricsServerConfig {
        max_conns: args.metrics_max_conns,
        io_timeout: Duration::from_millis(args.metrics_io_timeout_ms),
    };
    let runtime = tokio::runtime::Builder::new_current_thread().enable_all().build()?;
    let server_token = shutdown.clone();
    let server = thread::Builder::new().name("metrics".into()).spawn(move || {
        runtime.block_on(async move {
            match tokio::net::TcpListener::from_std(listener) {
                Ok(listener) => metrics_server::serve(listener, receiver, config, server_token).await,
                Err(e) => eprintln!("media-doctor watch: metrics listener unavailable: {e}"),
            }
        });
    })?;

    let result = ingest_loop(&socket, &mut state, &mut publisher);
    shutdown.cancel();
    let _ = server.join();
    result
}

/// How often the exposition is re-rendered, and the socket read timeout that
/// guarantees a flush after the feed goes quiet.
const PUBLISH_INTERVAL: Duration = Duration::from_millis(250);

fn ingest_loop(
    socket: &UdpSocket,
    state: &mut WatchState,
    publisher: &mut MetricsPublisher,
) -> Result<(), Box<dyn std::error::Error>> {
    let start = Instant::now();
    let mut buf = [0u8; 65536];
    let mut dirty = false;
    loop {
        match socket.recv_from(&mut buf) {
            Ok((n, _src)) => {
                let clock = start.elapsed();
                state.feed_datagram(&buf[..n], clock);
                dirty = !publisher.maybe_publish(state, clock);
            }
            Err(e) if matches!(e.kind(), std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut) => {
                if dirty {
                    publisher.publish_now(state, start.elapsed());
                    dirty = false;
                }
            }
            Err(e) => return Err(e.into()),
        }
    }
}
```

Imports: `std::net::{SocketAddr, UdpSocket}`, `std::thread`, `std::time::{Duration, Instant}`, `media_doctor::metrics_server::{self, MetricsPublisher, MetricsServerConfig}`, `tokio_util::sync::CancellationToken` (add `tokio`/`tokio-util` as non-optional-by-feature deps already pulled by `net`; the bin has `required-features = ["cli"]` which implies `net`). Delete everything listed under Files from `bind_udp` onward; delete `use std::io::{Read, Write}`, `TcpStream`, the atomics, `Mutex`, `Arc` if unused.

`WatchArgs::udp_config` (in `cli.rs`, feature `net`):

```rust
impl WatchArgs {
    /// The UDP socket settings from `--udp-rcvbuf`, `--udp-reuse-addr`, `--udp-interface`.
    pub fn udp_config(&self) -> Result<crate::udp::UdpConfig, String> {
        use crate::udp::{MulticastInterface, UdpConfig};
        let interface = match self.udp_interface.as_deref() {
            None => None,
            Some(s) => Some(
                s.parse::<std::net::Ipv4Addr>().map(MulticastInterface::V4)
                    .or_else(|_| s.parse::<u32>().map(MulticastInterface::V6Index))
                    .map_err(|_| format!("invalid --udp-interface {s:?}: expected an IPv4 address or an interface index"))?,
            ),
        };
        Ok(UdpConfig { recv_buffer: self.udp_rcvbuf, reuse_addr: self.udp_reuse_addr, interface })
    }
}
```

(`UdpConfig` is `#[non_exhaustive]`; construct it inside the crate — fine — or give it a `Default`+field-update at call sites in tests.)

- [ ] **Step 5: Run — expect PASS; 20 consecutive runs of the smoke test and the library tests**

```bash
cargo test --locked -p media-doctor --all-features 2>&1 | grep -E '^test result|FAILED|panicked'
for i in $(seq 20); do cargo test --locked -p media-doctor --all-features --test watch_metrics_server 2>&1 | grep -E '^test result'; done | sort | uniq -c
cargo clippy --locked -p media-doctor --all-features --all-targets -- -D warnings
cargo build --locked -p media-doctor --no-default-features
```

Expected: 4 passed ×20. Manual verification (record in the report): `cargo run -p media-doctor -- watch --udp 127.0.0.1:0 --metrics-addr 127.0.0.1:0`, `curl -si http://<printed>/metrics | head`, feed `ffmpeg`/`nc -u` with the fixture, confirm the counters move, Ctrl-C.

- [ ] **Step 6: Revert-check the flush.** Remove the `Err(WouldBlock|TimedOut)` arm's `publish_now` (leave the arm): `final_datagrams_become_visible_after_the_feed_stops` fails by deadline (the throttled publisher never re-renders). Remove `socket.set_read_timeout(...)`: same failure (the loop blocks in `recv_from`). Restore both.

- [ ] **Step 7: Delete the old golden-capture test file** `media-doctor/tests/watch_metrics_golden.rs` (its second test captured the old bin's head, now preserved as `golden/watch/http-response-head.txt` and compared by Task 10's test). Update `golden/watch/README.md`: list the two live comparators.

- [ ] **Step 8: README (lines 46–92)** — update the flag table (`--metrics-max-conns`, `--metrics-io-timeout-ms` were never in it; add them and the three new `--udp-*` flags with defaults), state metrics are rendered by `metrics-exporter-prometheus` and served by hyper, `render_prometheus` is gone (`WatchState::snapshot()` / `render_metrics`), and replace "two `std::thread`s sharing `Arc<Mutex<WatchState>>`; no async runtime" with the new description (ingest on the main thread, one current-thread tokio runtime on the `metrics` thread; `WatchState` is not shared).

- [ ] **Step 9: Commit**

```bash
git add media-doctor
git commit -m "feat(media-doctor)!: watch serves metrics via hyper+exporter, binds UDP via socket2, adds --udp-rcvbuf/--udp-reuse-addr/--udp-interface"
```

---

### Task 13: media-doctor — `bounded.rs` on `wait-timeout` (SP7)

**Files:**
- Modify: `media-doctor/Cargo.toml` (`[dev-dependencies]`)
- Modify: `media-doctor/tests/support/bounded.rs` (74 lines)
- Tests that include it via `#[path]`: `tests/bounded_cmd.rs`, `tests/mediastreamvalidator_oracle.rs` (unchanged callers)

**Interfaces:** `pub fn output_bounded(cmd: &mut Command, deadline: Duration) -> io::Result<Output>` — **unchanged** (callers at `mediastreamvalidator_oracle.rs:119,220`, `bounded_cmd.rs:56,70`).

- [ ] **Step 1: The existing tests are the spec.** `tests/bounded_cmd.rs` already contains the two behaviour tests (`bounded_runner_returns_despite_pipe_holding_grandchild`, `bounded_runner_kills_an_overrunning_tool_with_a_clear_error`). Run them on the old implementation first:

```bash
cargo test --locked -p media-doctor --all-features --test bounded_cmd 2>&1 | grep -E '^test |test result'
```

Expected: 2 passed.

- [ ] **Step 2: Implement.** `Cargo.toml` dev-dep: `wait-timeout = "0.2"  # replaces the hand-written try_wait/sleep deadline poll in tests/support/bounded.rs`. New `run`:

```rust
use wait_timeout::ChildExt;

fn run(cmd: &mut Command, deadline: Duration, out_path: &PathBuf, err_path: &PathBuf) -> io::Result<Output> {
    let mut child = cmd
        .stdin(Stdio::null())
        .stdout(Stdio::from(File::create(out_path)?))
        .stderr(Stdio::from(File::create(err_path)?))
        .spawn()?;
    // Blocks in the OS (waitpid/SIGCHLD-driven on Unix) until the child exits
    // or the deadline passes — no polling interval, no sleep.
    let Some(status) = child.wait_timeout(deadline)? else {
        let _ = child.kill();
        let _ = child.wait();
        return Err(io::Error::new(
            io::ErrorKind::TimedOut,
            format!("{:?} still running after the {deadline:?} hard deadline; killed", cmd.get_program()),
        ));
    };
    Ok(Output { status, stdout: fs::read(out_path)?, stderr: fs::read(err_path)? })
}
```

Delete `POLL_INTERVAL` and the `Instant` import; keep the temp-file capture (the doc comment's reason — pipe-holding grandchildren — is unchanged, and `wait_timeout` only waits for the child, so the redirect-to-files design still applies). Update the file's doc comment: "polls `try_wait`" → "waits with `wait-timeout`".

- [ ] **Step 3: Run, 20×, and the oracle suite**

```bash
CARGO_RESOLVER_INCOMPATIBLE_RUST_VERSIONS=fallback cargo build -p media-doctor --all-features --tests
git diff Cargo.lock | grep -E '^[-+]' | grep -v '^[-+][-+]'     # adds wait-timeout only (libc already locked)
for i in $(seq 20); do cargo test --locked -p media-doctor --all-features --test bounded_cmd 2>&1 | grep -E '^test result'; done | sort | uniq -c
cargo test --locked -p media-doctor --all-features --test mediastreamvalidator_oracle -- --nocapture 2>&1 | grep -E '^test result|SKIP|skipp'
```

Expected: `2 passed` ×20; the oracle suite as in the baseline (it self-skips without `mediastreamvalidator`).

- [ ] **Step 4: Revert-check.** Replace `wait_timeout(deadline)?` with `wait_timeout(Duration::from_secs(60))?`: `bounded_runner_kills_an_overrunning_tool_with_a_clear_error` waits out the child's `sleep 30` and fails its assertion on the deadline message/elapsed bound (record; abort after 90 s with `timeout 120`). Restore.

- [ ] **Step 5: Note for the other clusters in `.delegate/w1-p-report.md`:** `hls-runtime/tests/support/bounded.rs` (cluster R-low, W1) and `multimux/tests/support/bounded.rs` (W2) are byte-identical copies of the old file; they take this exact replacement (same `wait-timeout = "0.2"` dev-dep, same `run` body); the lock entry for `wait-timeout` lands with this branch.

- [ ] **Step 6: Commit**

```bash
git add media-doctor Cargo.lock
git commit -m "test(media-doctor): bounded.rs waits with wait-timeout instead of polling try_wait"
```

---

### Task 14: Guards (spec §5) for the three crates — last code task

**Files:**
- Create: `media-doctor/tests/no_handroll_guard.rs`
- Create: `media-plane/tests/no_handroll_guard.rs`
- Create: `dvb-ci-runtime/tests/no_handroll_guard.rs`

Same lexical-tripwire pattern as `media-doctor/tests/no_dom_guard.rs` (read it for `rs_files`/`brace_delta` and reuse that shape): scan every `src/**/*.rs`, drop lines inside `#[cfg(test)] mod … { }` bodies and `//` comment lines, and assert none of the patterns matches outside a reasoned allowlist. The module doc of each file says it is a tripwire and that review is the real control.

**Patterns (spec §5), per crate**

media-doctor (the crate the spec's HTTP/URL/SDP/time list applies to): `"HTTP/1.`, `"\r\n\r\n"`, `find("://")`, `strip_prefix("` ending in `://")`, string building `"a=` / `"m=` / `"v=0`, `civil_from_days` / `days_from_civil`, a base64 alphabet literal (`ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/`), `thread::sleep` and `sleep(`; plus crate-specific: `TcpListener::bind` outside `src/bin/` is allowed (none expected), `UdpSocket::bind` (must go through `udp.rs`), `render_prometheus`.

media-plane: the generic list, plus `std::sync::Mutex`, `std::sync::Condvar`, `PoisonError`, `.lock().unwrap()`, `.lock().expect(` (SP6.6: the lock is `parking_lot`; the poison wrapper is gone).

dvb-ci-runtime: the generic list, plus `libc::poll`, `libc::pollfd`, `POLLIN` (SP6.7).

Allowlists (each entry has a reason string asserted non-empty):

| crate | file | pattern | reason |
|---|---|---|---|
| media-doctor | `src/metrics_server.rs` | `sleep(` | accept-error backoff (EMFILE/ECONNABORTED must not spin the accept loop); not a wait for a condition |
| media-doctor | `src/udp.rs` | `UdpSocket::bind` | none — `socket2` binds; only the *type* `UdpSocket` appears (pattern is the call `UdpSocket::bind(`; no allowlist expected) |
| dvb-ci-runtime | `src/linux.rs` | `thread::sleep` | CAM `CA_RESET` settle delay (`RESET_SETTLE`): a fixed hardware reset time in synchronous device code, not a wait on a condition and not async |
| media-plane | — | — | none |

- [ ] **Step 1: Write the three guard files.** Core of each (media-plane shown; the other two differ only in the `PATTERNS` and `ALLOW` tables):

```rust
//! Tripwire (de-hand-roll W1-P, spec §5): fails if `src/` (outside
//! `#[cfg(test)] mod` bodies) reintroduces a hand-rolled pattern the
//! migration removed — poison-recovering std locks, sleep-based waits, HTTP
//! framing literals, … **This is a lexical tripwire, not a proof**; a
//! renamed helper or a split literal evades it. Code review is the real
//! control. Every allowlist entry carries a reason.

use std::fs;
use std::path::Path;

const PATTERNS: &[(&str, &str)] = &[
    ("\"HTTP/1.", "hand-built HTTP status line"),
    ("\"\\r\\n\\r\\n\"", "HTTP head terminator literal"),
    ("find(\"://\")", "hand-rolled URL scheme split"),
    ("civil_from_days", "hand-rolled calendar maths (use jiff)"),
    ("days_from_civil", "hand-rolled calendar maths (use jiff)"),
    ("ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/", "base64 alphabet (use base64)"),
    ("thread::sleep", "sleep-based wait"),
    ("sleep(", "sleep-based wait"),
    ("std::sync::Mutex", "std Mutex poisons; use parking_lot"),
    ("std::sync::Condvar", "pair Condvar with the parking_lot Mutex"),
    ("PoisonError", "poison-recovery wrapper (parking_lot does not poison)"),
    (".lock().unwrap()", "poisoning lock unwrap"),
];
/// (file suffix, pattern, reason). Empty for media-plane.
const ALLOW: &[(&str, &str, &str)] = &[];

fn rs_files(dir: &Path, out: &mut Vec<(String, String)>) { /* as in no_dom_guard.rs */ }

/// Source with `#[cfg(test)] mod … { … }` bodies and `//` comments removed.
fn production_source(src: &str) -> String { /* brace_delta walk from no_dom_guard.rs */ }

#[test]
fn no_handrolled_patterns_in_production_source() {
    let mut files = Vec::new();
    rs_files(&Path::new(env!("CARGO_MANIFEST_DIR")).join("src"), &mut files);
    assert!(!files.is_empty(), "guard found no source files — path bug");
    let mut hits = Vec::new();
    for (path, text) in &files {
        let prod = production_source(text);
        for (pat, why) in PATTERNS {
            if prod.contains(pat) && !ALLOW.iter().any(|(f, p, r)| path.ends_with(f) && p == pat && !r.is_empty()) {
                hits.push(format!("{path}: `{pat}` — {why}"));
            }
        }
    }
    assert!(hits.is_empty(), "hand-rolled patterns reintroduced:\n{}", hits.join("\n"));
}

/// The guard must bite: a synthetic offender is detected.
#[test]
fn guard_detects_a_synthetic_offender() {
    let prod = production_source("fn f() { std::thread::sleep(d); }\n#[cfg(test)]\nmod tests { fn g() { std::thread::sleep(d); } }\n");
    assert!(prod.contains("thread::sleep"));
    assert_eq!(prod.matches("thread::sleep").count(), 1, "test-module bodies are excluded");
}
```

(Write the two helper functions in full by copying `rs_files` and `brace_delta` from `media-doctor/tests/no_dom_guard.rs:46-90` and adding a `#[cfg(test)] mod` stripper: when a line, trimmed, starts with `#[cfg(test)]` and the next non-attribute line starts with `mod `, skip until `brace_delta` returns to zero. Test each stripper path in `guard_detects_a_synthetic_offender`.)

- [ ] **Step 2: Run each guard — expect the first runs to expose real hits; resolve each by fixing the source or adding a reasoned allowlist entry from the table above — never by weakening a pattern**

```bash
cargo test --locked -p media-plane --test no_handroll_guard 2>&1 | grep -E '^test |FAILED|reintroduced' -A6
cargo test --locked -p media-doctor --all-features --test no_handroll_guard 2>&1 | grep -E '^test |FAILED|reintroduced' -A6
cargo test --locked -p dvb-ci-runtime --all-features --test no_handroll_guard 2>&1 | grep -E '^test |FAILED|reintroduced' -A6
```

Expected after wiring the allowlist: PASS. (The dvb-ci-runtime guard file compiles on macOS: it only reads `src/`; `libc::poll` is in the pattern list and must NOT appear in `src/linux.rs` any more.)

- [ ] **Step 3: Revert-check every guard against the OLD code.** Check out main's file contents for one offender per crate and confirm the guard fails (`git stash`-free: copy the file, edit, run, restore):
  - media-plane: re-add `use std::sync::Mutex;` to a non-test position in `trunk.rs` → `std::sync::Mutex` hit.
  - media-doctor: add `"HTTP/1.1 200 OK\r\n"` to `src/metrics_server.rs` → hit; add `thread::sleep(` to `src/udp.rs` → hit.
  - dvb-ci-runtime: re-add `libc::poll(` to `src/linux.rs` → hit.
  Record each failure line.

- [ ] **Step 4: Commit**

```bash
git add media-doctor/tests/no_handroll_guard.rs media-plane/tests/no_handroll_guard.rs dvb-ci-runtime/tests/no_handroll_guard.rs
git commit -m "test: no-handroll tripwire guards for media-doctor, media-plane and dvb-ci-runtime (spec §5)"
```

---

### Task 15: Full gate, CHANGELOGs, version notes, hand-off

**Files:**
- Modify: `media-doctor/CHANGELOG.md` (`[Unreleased]`, top of file)
- Modify: `media-plane/CHANGELOG.md` (`[Unreleased]`, line 8)
- Modify: `dvb-ci-runtime/CHANGELOG.md` (`[Unreleased]`, line 8)
- Modify: `media-doctor/README.md` (final read-through), `media-plane/README.md` and crate docs if they name `std::sync::Mutex`
- Create: `.delegate/w1-p-report.md`

- [ ] **Step 1: CHANGELOG `[Unreleased]` entries**

`media-doctor/CHANGELOG.md`:

```markdown
### Changed (breaking)
- **BREAKING: `WatchState::render_prometheus` is removed.** The Prometheus text is now produced by `metrics-exporter-prometheus`: use `media_doctor::render_metrics(&WatchState)` (feature `metrics`) or read the typed `WatchState::snapshot()`. Differences in the exposition text for the same input (checked semantically against the golden from `main`, `tests/golden/watch/m6-single.prom`): the `# clauses: …` comment line is no longer emitted (the clause is on `ConformanceSample::clause`); series order follows the exporter's. Example, before: `# clauses: continuity_count_error=…` after the `conformance_events_total` series; after: absent. HELP, TYPE and every sample value are unchanged.
- The `watch` HTTP response head differs only cosmetically (hyper writes it): header names are lower-case, a `date` header is added, header order differs. Before: `Content-Type: text/plain; version=0.0.4` / `Content-Length: N` / `Connection: close`. After: `content-type: text/plain; version=0.0.4` / `content-length: N` / `connection: close` / `date: …`.
- `watch` no longer spawns a thread per metrics connection. Over-cap connections are answered `503` by hyper (formerly hand-written bytes); the concurrency cap, the total per-connection deadline and `--metrics-max-conns` / `--metrics-io-timeout-ms` keep their meaning and defaults. A header-read timeout (same value) is now also enforced.
- `watch` renders the exposition at most every 250 ms and flushes when the feed goes quiet, instead of rendering on every scrape.

### Added
- Features `metrics` (exposition) and `net` (hyper metrics server + socket2 UDP bind; implied by `cli`).
- `WatchSnapshot`, `ConformanceSample`, `PidFlag`, `WatchState::snapshot()`.
- `metrics_server` (`serve`, `channel`, `MetricsServerConfig`, `MetricsPublisher`) and `udp` (`bind_udp`, `UdpConfig`, `MulticastInterface`, `recv_buffer_size`) modules.
- `watch` flags `--udp-rcvbuf`, `--udp-reuse-addr` (default off, as before), `--udp-interface`; IPv6 multicast groups are now joined.
- Dev: `wait-timeout` in the test harness; `tests/no_handroll_guard.rs`.
```

`media-plane/CHANGELOG.md`:

```markdown
### Changed
- `Trunk`'s state lock and the `StallIngest` back-pressure `Condvar` are now `parking_lot` (no poisoning; the poison-recovery wrapper is deleted). No public API change.
- Measured the single-lock contention with the new `trunk_contention` criterion benchmark; numbers and the lock-split decision are in `benches/RESULTS.md` (<verdict from Task 4>).
### Added
- `benches/trunk_contention` (criterion): 1 publisher × N = 1, 4, 16, 64 cursors.
- `tests/no_handroll_guard.rs`.
```

`dvb-ci-runtime/CHANGELOG.md`:

```markdown
### Changed
- (`linux` feature) device readiness polling uses `rustix::event::poll` instead of `libc::poll`; sub-millisecond timeouts are no longer truncated to zero. New dependency `rustix` (feature `linux` only). `libc` stays for the CA ioctls. No public API change.
```

- [ ] **Step 2: The full gate, exactly as CI runs it**

```bash
/Volumes/External/Projects/rust-broadcast/.delegate/gate-wt.sh "$PWD" > /Volumes/External/Projects/rust-broadcast/.delegate/gate-w1-p.log 2>&1
grep -c '^rc=0' /Volumes/External/Projects/rust-broadcast/.delegate/gate-w1-p.log
grep -B1 -A10 '^rc=[1-9]' /Volumes/External/Projects/rust-broadcast/.delegate/gate-w1-p.log | head -30
```

Expected: `14` and no failure block. (Includes the dvb-ci-runtime Linux cross-clippy and Linux doc steps.) Then re-run the Docker Linux test of dvb-ci-runtime once more on the final commit (`git ls-files … | tar`, same recipe) and `python3 tools/check-published-dep-consistency.py`.

- [ ] **Step 3: 20-run flakiness sweep of every test touched by SP7** (record the counts in the report):

```bash
for i in $(seq 20); do cargo test --locked -p media-plane --all-features 2>&1 | grep -E '^test result: (FAILED|ok)'; done | sort | uniq -c
for i in $(seq 20); do cargo test --locked -p media-doctor --all-features 2>&1 | grep -E '^test result: (FAILED|ok)'; done | sort | uniq -c
```

Expected: only `ok` lines, each with count 20.

- [ ] **Step 4: Write `.delegate/w1-p-report.md`** with: baseline vs after test counts per crate; the Task 4 benchmark tables and verdict (and whether Task 5 ran/was reverted); the revert-check transcripts (Tasks 3, 6, 7, 8, 9, 10, 11, 12, 13, 14); the disposition table from Task 12; the HTTP head and exposition differences; every lock change (expected: `media-plane` gains `criterion`+`parking_lot` edges; `media-doctor` gains `metrics`, `metrics-exporter-prometheus`, `tokio`/`tokio-util`/`hyper`/`hyper-util`/`http-body-util`/`socket2`/`wait-timeout` edges and `wait-timeout` the only new package besides `rustix`; `dvb-ci-runtime` gains `rustix` and its `linux-raw-sys`/`bitflags` if absent); and the **version notes**:

  | crate | current | proposed | reason / epoch purity |
  |---|---|---|---|
  | media-doctor | 0.8.0 | **0.9.0** | breaking: `render_prometheus` removed. No new dependency type is public, so no epoch effect on a workspace sibling |
  | media-plane | 0.4.1 | **0.4.2** | no public change (internal lock, bench, guard) |
  | dvb-ci-runtime | 0.16.0 | **0.16.1** | internal dependency swap behind feature `linux`; no public change |

  Also record the cross-cluster notes: (a) `bounded.rs` copies in `hls-runtime/tests/support/` (R-low) and `multimux/tests/support/` (W2) take the Task 13 replacement; (b) no multimux compile fix was needed by this cluster (confirm `cargo build --locked -p multimux --all-features` and `-p compliance-probe`); (c) multimux's `lock.rs` is untouched (W2).

- [ ] **Step 5: Commit and hand off — do not merge**

```bash
git add media-doctor/CHANGELOG.md media-plane/CHANGELOG.md dvb-ci-runtime/CHANGELOG.md media-doctor/README.md .delegate/w1-p-report.md
git commit -m "docs: W1-P changelogs and report"
```

**Do not merge.** The orchestrator runs the adversarial reviewer, updates `.delegate/release-versions.txt`, merges with `git merge --squash`, pushes and confirms CI green.

---

## Coverage table (spec §3 / §4 / §7 → task)

| Spec item | Where | Task |
|---|---|---|
| §3 HTTP: media-doctor `src/bin/media-doctor.rs` (thread-per-connection server, head reader) | deleted; hyper server | 10, 12 |
| §3 HTTP: media-doctor `watch.rs` `render_prometheus` | deleted, replaced by exporter | 8, 9 |
| §4 SP2.3 `metrics` + `metrics-exporter-prometheus`, `WatchState` feeds gauges/counters, atomic cap deleted | snapshot + exposition; cap is a tokio `Semaphore` in `metrics_server` (deviation E1) | 8, 9, 10, 12 |
| §4 SP2.3 `watch_metrics_server.rs` behaviours | re-expressed / retired with reasons (disposition table) | 10, 12 |
| §4 SP1.6 media-doctor UDP/multicast via socket2 (`SO_RCVBUF`, `SO_REUSEADDR`, interface) | `udp.rs` + CLI flags | 11, 12 |
| §3 runtime: media-plane single `Mutex<TrunkState>` | benchmark first, numeric rule, conditional split | 2, 4, 5 |
| §3 runtime: media-plane Condvar back-pressure | `parking_lot::Condvar` (forced by the Mutex swap, and simpler) | 3 |
| §3 runtime: media-plane CAS waiter cap | left, reason recorded with evidence | 4 |
| §4 SP6.4 benchmark numbers before and after recorded | `benches/RESULTS.md` | 2, 3, 4, 5 |
| §4 SP6.6 `parking_lot` in media-plane; poison wrappers deleted (multimux `lock.rs` = W2) | `lock_state` wrapper deleted | 3 |
| §3 runtime: dvb-ci-runtime `libc::poll`; §4 SP6.7 | `rustix::event::poll`, Linux Docker + cross-clippy | 7 |
| §3 test harness: ~40 sleep waits (media-doctor, media-plane share) | 2 sleeps + 3 negative waits in media-plane; media-doctor server tests rewritten | 6, 10, 12 |
| §3 test harness: `bounded.rs` ×3 (media-doctor copy) | `wait-timeout`; other two noted for their clusters | 13 |
| §3 test harness: reserve-then-rebind ports; hand-rolled HTTP test origins (media-doctor) | port-0 listeners; in-process server tests; bin test learns ports from its start-up line | 10, 12 |
| §3 defects 1–8 | **none owned by cluster P** (stated in Global Constraints); new guarantees are still revert-checked | — |
| §5 guards for these crates | three tripwire files | 14 |
| §6 goldens from main before the wave | `/metrics` text and HTTP head | 1 |
| §6 20 consecutive runs of every touched test | per-task loops + final sweep | 6, 10, 11, 12, 13, 15 |
| §7 gate 14/14, CHANGELOG, §8 versions | | 15 |
| §8 media-doctor breaking (`render_prometheus`) | recorded, 0.9.0 | 9, 15 |

## Escalations

**E1 — SP2.3 deviation: the exporter's built-in HTTP listener is not used (owner decision needed, default applied).** Spec §4 SP2.3 says `metrics-exporter-prometheus` "serve[s] /metrics". Evidence from `metrics-exporter-prometheus-0.18.3/src/exporter/http_listener.rs` and `hyper-1.11.0`: (a) the listener builds `hyper::server::conn::http1::Builder::new()` with **no `.timer(..)`**, and hyper's header-read timeout "requires a Timer … to take effect" (`http1.rs:346`) — so a dribbling client is never dropped, which is exactly audit MD-W9 and the old `dribbling_client_is_dropped_at_the_total_deadline` test; (b) it has no concurrency cap (the only access control is an IP allow-list; `process_tcp_stream` spawns unboundedly); (c) `with_http_listener(addr)` binds inside `build()` with no way to read the bound address, so the exporter cannot be started on port 0 and SP7's "bind port 0 and pass the listener in" cannot be honoured. The plan therefore uses the exporter for what it is good at (metric registry + `PrometheusHandle::render()` text exposition, `default-features = false` so it pulls no tokio/hyper itself) and serves the text with `hyper` directly under the old server's policy. If the owner prefers the literal spec reading, the cost is: no header-read/total deadline (a re-opened MD-W9), no cap, and port-0 tests impossible for `watch` (reserve-then-rebind returns). Applied default: this plan. Alternative if the owner wants `axum` here instead: axum 0.7.9's `serve` sets no timer either (`axum-0.7.9/src/serve.rs:254`), so it has the same slow-client gap; hyper direct is the smallest correct option.

**E2 — Lock-split outcome is data-dependent.** Task 5 exists only if the Task 4 rule says SPLIT (`t(16) ≥ 3× t(1)` or `t(64) ≥ 6× t(1)` after the `parking_lot` swap) and is reverted unless it wins ≥ 30 % at N = 16. The spike on record (`trunk.rs:172-177`, 956 ns → 9.98 µs at 16 readers) predicts SPLIT on the std `Mutex`; parking_lot may change that. The 64-spinning-reader point oversubscribes any laptop: the report must state the core count, and "spinning readers" is the stated worst case, not the production shape.

**E3 — dvb-ci-runtime verification is Linux-only.** macOS compiles `linux.rs` out; Task 7's proof is the Linux cross-clippy and doc (both in the 14-step gate) plus the Docker run of the new `poll_readable_*` tests. If Docker is unavailable on the executing machine, Task 7 is **not done** — it is escalated, never claimed from a macOS build.

**E4 — media-plane's two test-only items depend on a `#[cfg(test)]` counter inside production code (`stalled_publishers`).** Chosen over a sleep or a pure timeout because a fixed wait cannot prove "blocked". It compiles out of release builds; the guard (Task 14) excludes test code. No alternative found that observes the parked state without it.

**E5 — Cross-cluster follow-ups, not dropped:** the `hls-runtime` (R-low) and `multimux` (W2) copies of `bounded.rs` take Task 13's body; the W2 multimux migration should adopt `parking_lot` for `lock.rs` independently of this branch (no media-plane type crosses that boundary, verified by the Task 3 multimux build).
