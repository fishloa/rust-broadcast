# De-hand-roll W2b-2-multimux-runtime — backon, pull scheduler, ingest driver, RTSP/RTMP push, socket2, parking_lot Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Collapse multimux's four backoff implementations onto `backon` (with `max_times` set explicitly and the cap clamped *after* jitter), its three pull drive loops onto one `JoinSet`+`backon` scheduler whose `WaitMs` hint never blocks servicing a ready fetch, its six per-source read loops onto one written-out generic scaffold (a per-source `IngestStep` struct keeps each source's read+`feed`; the #1083 stall check is preserved), migrate the RTSP source and both RTSP/RTMP pushes onto the runtime-crate adapters (with the two missing `AsyncRtspClient` methods added first), switch the UDP binds to `socket2`, and replace `lock.rs` with `parking_lot`.

**Architecture:** Branch `w2/b` (continued from W2b-1, same worktree), rebasing over W2b-1's merged result (itself over W2a; W1-R-low-a is already on `main` at `3340fdaa`/`31ba01ec`). One commit per task.

**Tech Stack:** `backon` 1.6.0 (verified: `ExponentialBuilder::new()` defaults `max_times: Some(3)` — MUST be overridden; `with_jitter_seed(u64)` exists; jitter is ADDED after the internal max clamp, so the caller re-clamps), `tokio::task::JoinSet`, `rtsp-runtime` 0.8 (`AsyncRtspClient` + the two added methods), `rtmp-runtime` 0.7 (`AsyncRtmpClient`/`RtmpTarget`/`RtmpTimeouts`), `socket2` 0.6, `parking_lot` 0.12, `media-plane` `IngestDriver` (verified: `feed(&mut self, input, now)`, `next_deadline`, `health()`, `trunk()`, `programs()`).

**Spec:** `docs/superpowers/specs/2026-10-03-protocol-runtime-dehandroll-design.md` (§3 runtime rows, §4 SP1.5/SP1.6/SP6.1/SP6.2/SP6.6, §5, §6, §7 W2). W2a is `…-w2a-multimux-http.md`; W2b-1 is `…-w2b-multimux-runtime.md`.

## Global Constraints

Copied verbatim from spec §2:

- MSRV 1.95.0; committed `Cargo.lock`, always `--locked`. A dependency add or bump may change only the intended lock entries; restore anything else with `cargo update -p <pkg> --precise <old>`.
- No Co-Authored-By or Claude-Session trailers on commits.
- Nothing is tagged or published without the owner's explicit sign-off.
- Epoch purity: recorded in `.delegate/release-versions.txt`.

## Review Focus

1. **A reconnect schedule must have a hard cap even with jitter, and must not silently stop after 3 attempts.** backon's default `max_times` is 3; the schedule sets it explicitly and re-clamps after jitter. **Task 1**.
2. **A `WaitMs` hint must not starve an in-flight fetch.** The pre-fix loop sleeps the hint inline before joining. **Task 2**.
3. **The RTSP push needs an interleaved SEND on the client** — `AsyncRtspClient` has `recv_interleaved` but no send; the method is added to rtsp-runtime (with tests) before the push migrates. **Task 5**.
4. **The generic ingest scaffold is real and keeps the #1083 stall check**: one written-out select/{advance,reap}/stall/timeout/cancel body, compile-verified, with a paused-time test that a silent peer still fails at `read` (and fails the revert that maps a timeout to "received"). **Task 4**.
5. **`socket2`'s SO_RCVBUF differs per OS** (Linux doubles, macOS reports set+overhead): assert the *requested* value round-trips via `get_recv_buffer_size`, not against the OS default. **Task 7**.

---

### Task 0: (inherited) — rebase over W2b-1, baseline

- [ ] **Step 1:** Rebase over W2b-1's merged result; run the baseline:

```bash
timeout 2400 cargo test --locked --all-features -p multimux -p rtsp-runtime -p rtmp-runtime 2>&1 | grep -E '^test result|FAILED|panicked' | sort | uniq -c
```

Record counts in `.delegate/w2b-report.md`. **Check the W2b-1 Task 6 `announce`/`record` methods exist on `AsyncRtspClient` before Task 5; escalate if not.**

---

### Task 1: `backon` replaces `supervisor::Backoff`, `ReconnectEngine` timing, `file_reader` retry, `hls_pull` retry (SP1.5)

**Files:**
- New `multimux/src/reconnect.rs` — a `backon`-backed schedule (real code below).
- Modify `origin/supervisor.rs` (`Backoff` 141–207), `push/mod.rs` (`ReconnectEngine` 238–311), `source/file_reader.rs` (380–394), `source/hls_pull.rs` (`retry_backoff` 117–128, `spawn_resource_fetch` 130–150), `config.rs` (`ReconnectPolicy::backoff_for` ~370–410).
- New test `multimux/tests/reconnect_policy.rs`.

**Interfaces:**
```rust
// reconnect.rs
pub struct ReconnectSchedule { builder: backon::ExponentialBuilder, max: Duration }
impl ReconnectSchedule {
    pub fn from_policy(policy: &crate::config::ReconnectPolicy) -> Self;
    pub fn from_parts(min: Duration, max: Duration, factor: f64) -> Self;
    /// The delay before the (attempt+1)-th retry. Bounded by `max` AFTER
    /// jitter (backon adds jitter after its own internal clamp).
    pub fn delay_for_attempt(&self, attempt: u32) -> Duration;
}
```
`supervisor::Backoff` keeps its public shape (re-exported `lib.rs:71`) but delegates to `ReconnectSchedule`.

- [ ] **Step 1: Write the failing tests**

```rust
//! SP1.5: one backon-backed schedule with an explicit attempt bound and a
//! jitter-proof cap. These FAIL on the old hand-rolled `powi` backoff.

use std::time::Duration;

#[test]
fn the_schedule_uses_backon_not_hand_arithmetic() {
    // The old `powi` code returns exactly `min` for attempt 0 —
    // deterministic and unjittered — so the jitter-band assertions fail on
    // it. backon's jitter adds `(0, current_delay)` to the pre-jitter
    // delay, so attempt 0 lands strictly inside (min, 2*min).
    let a = multimux::reconnect::ReconnectSchedule::from_parts(
        Duration::from_secs(10), Duration::from_secs(30), 2.0,
    );
    let d0 = a.delay_for_attempt(0);
    assert!(
        d0 > Duration::from_secs(10) && d0 < Duration::from_secs(20),
        "attempt 0 must be jittered into (min, 2*min): got {d0:?}"
    );
    let b = multimux::reconnect::ReconnectSchedule::from_parts_seeded(
        Duration::from_secs(10), Duration::from_secs(30), 2.0, 0xDEADBEEF,
    );
    let d0b = b.delay_for_attempt(0);
    assert_ne!(d0, d0b, "different seeds must jitter differently: {d0:?} vs {d0b:?}");
    // And every jittered delay stays within [min, cap] (the re-clamp).
    for i in 0..64 {
        let d = a.delay_for_attempt(i);
        assert!(d >= Duration::from_secs(10) && d <= Duration::from_secs(30),
            "attempt {i}: jitter escaped [min, cap]: {d:?}");
    }
}

#[test]
fn the_schedule_doubles_from_the_min_and_never_exceeds_the_cap_even_with_jitter() {
    let s = multimux::reconnect::ReconnectSchedule::from_parts(
        Duration::from_millis(500), Duration::from_secs(30), 2.0,
    );
    let mut d = Duration::ZERO;
    for i in 0..64 {
        d = s.delay_for_attempt(i);
        assert!(d <= Duration::from_secs(30), "attempt {i}: {d:?} exceeds the cap");
        assert!(d >= Duration::from_millis(500), "attempt {i}: {d:?} precedes the min");
    }
    // Once the raw delay saturates at the cap, every jittered+re-clamped
    // value is exactly the cap.
    assert_eq!(d, Duration::from_secs(30));
}

#[test]
fn the_schedule_yields_attempts_past_backons_default_three() {
    // backon's default max_times is 3 (`ExponentialBuilder::new`, backon
    // 1.6.0 exponential.rs:62) — the schedule must override it or the 4th
    // retry would return the cap-sentinel. Ask for attempt 10 and get a real
    // (capped) delay, not the exhaustion fallback.
    let s = multimux::reconnect::ReconnectSchedule::from_parts(
        Duration::from_secs(1), Duration::from_secs(30), 2.0,
    );
    assert_eq!(s.delay_for_attempt(10), Duration::from_secs(30));
}
```

- [ ] **Step 2: Run pre-fix — FAIL.** Against the current `Backoff::delay_for_attempt` (hand `powi`, no jitter): test 1 FAILS (`d0 == 10s` exactly, not `> 10s`). Tests 2/3 pass on old code (characterisation — the cap and attempt reach already hold); test 1 is the bite. Record.

- [ ] **Step 3: Implement `ReconnectSchedule` on backon (real code)**

```rust
//! One reconnect schedule for every multimux retry, backed by `backon`
//! (SP1.5). Two verified backon facts shape this wrapper:
//! 1. `ExponentialBuilder::new()` defaults `max_times: Some(3)` — an
//!    un-overridden builder stops yielding after 3 attempts, so every
//!    schedule here sets `without_max_times()` and applies its own bound.
//! 2. Jitter is ADDED after backon's internal max-delay clamp
//!    (exponential.rs: `tmp_cur.saturating_add(tmp_cur.mul_f32(rng))`),
//!    so a jittered delay can exceed `max_delay` — this wrapper re-clamps
//!    with `.min(max)` for a hard cap.

use std::time::Duration;

pub struct ReconnectSchedule {
    builder: backon::ExponentialBuilder,
    max: Duration,
}

impl ReconnectSchedule {
    pub fn from_parts(min: Duration, max: Duration, factor: f64) -> Self {
        Self::from_parts_seeded(min, max, factor, rand_seed())
    }

    pub fn from_parts_seeded(min: Duration, max: Duration, factor: f64, seed: u64) -> Self {
        Self {
            // `with_jitter()` is REQUIRED: backon 1.6.0's builder defaults
            // `jitter: false` (exponential.rs:58) and `with_jitter_seed`
            // only seeds the rng — it does not enable jitter. Jitter adds
            // `(0, current_delay)` to the pre-jitter delay (exponential.rs
            // `tmp_cur.saturating_add(tmp_cur.mul_f32(self.rng.f32()))`),
            // so attempt 0 lands in (min, 2*min) and the caller-side
            // `.min(max)` re-clamp keeps the hard cap.
            builder: backon::ExponentialBuilder::new()
                .with_jitter()
                .with_jitter_seed(seed)
                .with_factor(factor as f32)
                .with_min_delay(min)
                .with_max_delay(max)
                .without_max_times(),
            max,
        }
    }

    pub fn from_policy(policy: &crate::config::ReconnectPolicy) -> Self {
        Self::from_parts(
            Duration::from_millis(policy.initial_backoff_ms),
            Duration::from_millis(policy.max_backoff_ms),
            crate::config::RECONNECT_BACKOFF_FACTOR,
        )
    }

    pub fn delay_for_attempt(&self, attempt: u32) -> Duration {
        match self.builder.build().nth(attempt as usize) {
            Some(d) => d.min(self.max),
            None => self.max,
        }
    }
}

fn rand_seed() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos() as u64)
        .unwrap_or(0)
}
```

Then rewire each site through it:
- `supervisor::Backoff::delay_for_attempt(&self, attempt) -> Duration` becomes `self.schedule.delay_for_attempt(attempt)`; `Backoff::next()` calls `self.schedule.delay_for_attempt(self.attempt)` then increments — the public shape (`new`/`production_default`/`next`/`delay_for_attempt`/`reset`) is unchanged, so `lib.rs`'s re-export and every test keep compiling.
- `push/mod.rs::ReconnectEngine::time_until_retry` (298–305) computes from `self.policy` → `ReconnectSchedule::from_policy(&self.policy).delay_for_attempt(self.attempt)`; `on_disconnect`'s `attempt >= max` exhaustion logic is UNCHANGED (permanent-failure classification stays domain logic, SP1.5).
- `source/file_reader.rs::read_probe_demux` (380–394): the fixed-interval loop becomes

```rust
async fn read_probe_demux(&self) -> Result<ParsedFile, FileReaderError> {
    let schedule = ReconnectSchedule::from_parts(
        self.config.retry_interval,
        self.config.retry_interval,   // constant retry, as today: factor 1.0
        1.0,
    );
    for attempt in 0..=self.config.max_retries {
        match self.read_probe_demux_once().await {
            Ok(parsed) => return Ok(parsed),
            Err(FileReaderError::Read { .. }) => {
                if attempt < self.config.max_retries {
                    tokio::time::sleep(schedule.delay_for_attempt(attempt)).await;
                }
            }
            Err(e) => return Err(e),
        }
    }
    self.read_probe_demux_once().await   // the final attempt's own error propagates
}
```

- `source/hls_pull.rs::retry_backoff` (117–128): the whole fn becomes `ReconnectSchedule::from_parts(RESOURCE_RETRY_BASE_DELAY, RESOURCE_RETRY_MAX_DELAY, RESOURCE_RETRY_FACTOR).delay_for_attempt(attempt.saturating_sub(1))`; `spawn_resource_fetch`'s `tokio::time::sleep(delay)` (144) stays (it is the delay's *application*, now fed by backon).

- [ ] **Step 4: Run — PASS** (`reconnect_policy`, `push_negotiate`, `file_reader`, `hls_pull` suites).

- [ ] **Step 5: Revert-check (two mutations, each shown failing)**
  1. Restore the old `powi` `delay_for_attempt`: `the_schedule_uses_backon_not_hand_arithmetic` FAILS (`d0 == 10s` exactly, outside `(10s, 20s)`). Restore.
  2. Remove `.with_jitter()` (keeping `with_jitter_seed`): the SAME test FAILS (`d0 == 10s`, the seed alone adds nothing — backon 1.6.0 `jitter: false` default, exponential.rs:58). Restore. Record both.

- [ ] **Step 6: Commit**

```bash
git add multimux && git commit -m "refactor(multimux): one backon-backed ReconnectSchedule (jitter-clamped, explicit attempt bound) for every retry (SP1.5)"
```

---

### Task 2: One pull scheduler for HLS/DASH/Smooth (SP6.2)

**Files:**
- New `multimux/src/source/pull.rs` — `PullScheduler` (full code below).
- Modify `source/hls_pull.rs` (`run_hls_pull` 451–660), `source/dash_pull.rs` (1107–1299), `source/smooth_pull.rs` (1131–1280) to drive it.
- Tests live INSIDE `source/pull.rs` (`#[cfg(test)] mod tests`): `PullScheduler`/`FetchOutcome`/`PendingFetch` are `pub(crate)`, which an integration test under `tests/` cannot name.

**`hls_pull.rs:7-20` resolution (spec requires it first):** the module doc's reason holds — multimux drives the sans-IO `HlsClient` core (not `TokioClient`) because `TokioClient` owns its own IO and cannot fit `IngestSession::feed`/`poll_transmit`. The scheduler is built over the `HlsClient` core's `Action` stream, which IS the shared core. Record on the module doc: "SP6.2 outcome: drives the shared `HlsClient` core; `TokioClient` remains the executor-bound adapter for a different consumer."

**Interfaces:**

```rust
// source/pull.rs — the shared fetch/retry/wait engine for the three pull
// sources. Each source keeps its own `feed` translation; the fan-out, the
// retry policy, the WaitMs handling and the idle park live here once.
// COMPILE-VERIFIED (scratch crate against main's `MultimuxError`, tokio 1.53).
use std::collections::VecDeque;
use std::fmt;
use std::future::Future;
use std::pin::Pin;
use std::time::Duration;

use multimux::MultimuxError;
use tokio::task::JoinSet;

/// The outcome of one spawned fetch.
pub(crate) enum FetchOutcome<K> {
    /// The fetch resolved: (key, bytes).
    Ready(K, Vec<u8>),
    /// The fetch failed: (key, error) — the source decides retry vs terminal.
    Failed(K, MultimuxError),
    /// The spawned fetch task itself panicked or was aborted.
    TaskPanic(String),
}

impl<K: fmt::Debug> fmt::Debug for FetchOutcome<K> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            FetchOutcome::Ready(k, b) => write!(f, "Ready({k:?}, {} bytes)", b.len()),
            FetchOutcome::Failed(k, e) => write!(f, "Failed({k:?}, {e})"),
            FetchOutcome::TaskPanic(e) => write!(f, "TaskPanic({e})"),
        }
    }
}

type FetchFuture<K> = Pin<Box<dyn Future<Output = (K, Result<Vec<u8>, MultimuxError>)> + Send>>;

/// One fetch to spawn.
pub(crate) struct PendingFetch<K> {
    pub key: K,
    pub delay: Duration,
    pub fut: FetchFuture<K>,
}

pub(crate) struct PullScheduler<K: Send + 'static> {
    inflight: JoinSet<(K, Result<Vec<u8>, MultimuxError>)>,
    backlog: VecDeque<PendingFetch<K>>,
    retries: VecDeque<PendingFetch<K>>,
    max_inflight: usize,
}

impl<K: Send + 'static> PullScheduler<K> {
    pub fn new(max_inflight: usize) -> Self {
        Self { inflight: JoinSet::new(), backlog: VecDeque::new(), retries: VecDeque::new(), max_inflight }
    }
    pub fn push(&mut self, fetch: PendingFetch<K>) {
        self.backlog.push_back(fetch);
    }
    pub fn push_retry(&mut self, fetch: PendingFetch<K>) {
        self.retries.push_back(fetch);
    }
    pub fn inflight_len(&self) -> usize {
        self.inflight.len()
    }
    pub fn is_idle(&self) -> bool {
        self.inflight.is_empty() && self.backlog.is_empty() && self.retries.is_empty()
    }
    pub fn pump(&mut self) {
        while self.inflight.len() < self.max_inflight {
            let Some(PendingFetch { key: _, delay, fut }) =
                self.retries.pop_front().or_else(|| self.backlog.pop_front())
            else {
                break;
            };
            self.inflight.spawn(async move {
                if !delay.is_zero() {
                    tokio::time::sleep(delay).await;
                }
                fut.await
            });
        }
    }
    pub async fn next(
        &mut self,
        wait_hint: Option<Duration>,
        idle_park: Duration,
    ) -> Option<FetchOutcome<K>> {
        if self.inflight.is_empty() {
            tokio::time::sleep(wait_hint.unwrap_or(idle_park)).await;
            return None;
        }
        let joined = match wait_hint {
            Some(hint) => {
                tokio::select! {
                    joined = self.inflight.join_next() => joined,
                    () = tokio::time::sleep(hint) => return None,
                }
            }
            None => self.inflight.join_next().await,
        };
        match joined {
            Some(Ok((key, Ok(bytes)))) => Some(FetchOutcome::Ready(key, bytes)),
            Some(Ok((key, Err(e)))) => Some(FetchOutcome::Failed(key, e)),
            Some(Err(e)) => Some(FetchOutcome::TaskPanic(e.to_string())),
            None => None,
        }
    }
}
```


- [ ] **Step 1: Write the failing tests** (virtual time, fake fetchers, `tokio::time::Instant` — never `std::time::Instant`, which does not move under `start_paused`)

Append to `source/pull.rs`. This module was compiled and run in a scratch crate: all five tests pass, and the two mutations named in the doc comments were run and fail as stated.

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use tokio::time::Instant;

    fn fetch_after(key: u32, after: Duration, bytes: &'static [u8]) -> PendingFetch<u32> {
        PendingFetch {
            key,
            delay: Duration::ZERO,
            fut: Box::pin(async move {
                tokio::time::sleep(after).await;
                (key, Ok(bytes.to_vec()))
            }),
        }
    }

    fn boom() -> (u32, Result<Vec<u8>, MultimuxError>) {
        panic!("fetch task died")
    }

    /// Defect 5. A fetch completes at t=100 ms while a `WaitMs(1000)` hint is
    /// outstanding: it must be returned at t=100 ms, not after the hint.
    ///
    /// Revert-check: in `next`, replace the `select!` with
    /// `tokio::time::sleep(hint).await; self.inflight.join_next().await`
    /// (the pre-fix inline hint sleep) and this FAILS: elapsed is 1 s, not
    /// 100 ms (`a_pre_fix_inline_hint_sleep_...` below measures that shape).
    #[tokio::test(start_paused = true)]
    async fn a_fetch_completing_during_a_waitms_hint_is_returned_when_it_completes() {
        let t0 = Instant::now();
        let mut s = PullScheduler::<u32>::new(8);
        s.push(fetch_after(1, Duration::from_millis(100), b"seg"));
        s.pump();
        let out = s.next(Some(Duration::from_millis(1000)), Duration::from_millis(5)).await;
        assert!(matches!(&out, Some(FetchOutcome::Ready(1, b)) if b == b"seg"), "{out:?}");
        assert_eq!(t0.elapsed(), Duration::from_millis(100), "serviced when ready, not after the hint");
    }

    /// The discriminating witness: the pre-fix ordering (sleep the hint
    /// inline, THEN join) measured on the same inputs takes the whole hint.
    #[tokio::test(start_paused = true)]
    async fn a_pre_fix_inline_hint_sleep_would_take_the_whole_hint() {
        let t0 = Instant::now();
        let mut s = PullScheduler::<u32>::new(8);
        s.push(fetch_after(1, Duration::from_millis(100), b"seg"));
        s.pump();
        tokio::time::sleep(Duration::from_millis(1000)).await; // the old inline WaitMs
        let out = s.next(None, Duration::from_millis(5)).await;
        assert!(matches!(out, Some(FetchOutcome::Ready(1, _))));
        assert_eq!(t0.elapsed(), Duration::from_millis(1000));
    }

    /// With nothing in flight the hint is honoured alone and reported as
    /// "the hint elapsed" (`None`).
    #[tokio::test(start_paused = true)]
    async fn the_hint_alone_elapses_when_nothing_is_in_flight() {
        let t0 = Instant::now();
        let mut s = PullScheduler::<u32>::new(8);
        assert!(s.next(Some(Duration::from_millis(300)), Duration::from_millis(5)).await.is_none());
        assert_eq!(t0.elapsed(), Duration::from_millis(300));
    }

    /// `pump` takes a due retry before the backlog when only one slot is free.
    ///
    /// Revert-check: swap the `retries`/`backlog` order in `pump`; the first
    /// outcome is then key 1 and this FAILS.
    #[tokio::test(start_paused = true)]
    async fn retries_take_the_slot_before_the_backlog() {
        let mut s = PullScheduler::<u32>::new(1);
        s.push(fetch_after(1, Duration::ZERO, b"backlog"));
        s.push_retry(fetch_after(2, Duration::ZERO, b"retry"));
        s.pump();
        assert_eq!(s.inflight_len(), 1);
        let first = s.next(None, Duration::from_millis(5)).await;
        assert!(matches!(first, Some(FetchOutcome::Ready(2, _))), "{first:?}");
        s.pump();
        let second = s.next(None, Duration::from_millis(5)).await;
        assert!(matches!(second, Some(FetchOutcome::Ready(1, _))), "{second:?}");
        assert!(s.is_idle());
    }

    /// A panicking fetch task is reported, not swallowed.
    #[tokio::test(start_paused = true)]
    async fn a_task_panic_is_reported_not_swallowed() {
        let mut s = PullScheduler::<u32>::new(8);
        s.push(PendingFetch { key: 1, delay: Duration::ZERO, fut: Box::pin(async { boom() }) });
        s.pump();
        let out = s.next(None, Duration::from_millis(5)).await;
        assert!(matches!(out, Some(FetchOutcome::TaskPanic(_))), "{out:?}");
    }

    /// Production-wiring TRIPWIRE (lexical, honestly labelled): once `hls_pull`
    /// is migrated, every wait in its non-test code lives in `PullScheduler`
    /// (the `WaitMs` hint, the idle park and the retry delay), so its
    /// production half contains no `tokio::time::sleep(` at all. Reintroducing
    /// the inline `Action::WaitMs => tokio::time::sleep(..)` arm anywhere in
    /// `run_hls_pull` trips this, independent of the engine tests above. On
    /// main today the production half has THREE such sites
    /// (`hls_pull.rs:144`, `:518`, `:576`), so this FAILS pre-migration.
    #[test]
    fn hls_pull_has_no_inline_sleep_outside_the_scheduler() {
        let src = include_str!("hls_pull.rs");
        let production = src.split("#[cfg(test)]").next().expect("split yields one part");
        let hits: Vec<usize> = production
            .match_indices("tokio::time::sleep(")
            .map(|(at, _)| production[..at].matches('\n').count() + 1)
            .collect();
        assert!(hits.is_empty(), "inline sleep(s) in hls_pull.rs at lines {hits:?}: waits belong in PullScheduler");
    }
}
```

- [ ] **Step 2: Run pre-fix — FAIL.** `PullScheduler` does not exist pre-fix, so first land ONLY the tripwire test (it needs no new code): `hls_pull_has_no_inline_sleep_outside_the_scheduler` FAILS on main's `hls_pull.rs` (three production `sleep(` sites at 144/518/576). The engine tests are new behaviour with the scheduler; their discriminating power is shown by Step 5.

- [ ] **Step 3: Implement `PullScheduler` (code above), then migrate the three loops.**

Migration shape for `hls_pull` (the other two are the same scaffold with their own `feed` translation):
- The `backlog`/`retry_queue` VecDeques (hls_pull.rs:472-481) move INTO `PullScheduler`; the spawn loop (489–560) collapses to: drain `driver.poll_transmit()` into `scheduler.push(...)`/collect the `WaitMs` value, then `scheduler.pump()`.
- The inline `Action::WaitMs => tokio::time::sleep(ms)` arm (510–519) is DELETED — the hint passes to `scheduler.next(Some(ms), IDLE_POLL_INTERVAL)`, which services a ready fetch before the hint (the defect-5 fix).
- The `if inflight.is_empty() { … sleep(IDLE_POLL_INTERVAL) }` block (567–578) collapses into `scheduler.next(None, IDLE_POLL_INTERVAL)` + the source's own `ended()`/`next_deadline` checks between calls.
- The join arm (580–644) becomes a `match` on `FetchOutcome::{Ready, Failed, TaskPanic}` with the SAME retry/auth-fail logic (the bodies at 588–644 move verbatim into the match arms — `HlsFetchId` is the key `K`).
- `dash_pull`/`smooth_pull`: same scaffold; `dash_pull`'s `next_deadline`-driven sleep (1199) becomes the source's own deadline wait BETWEEN `scheduler.next()` calls (it is a session deadline, not a fetch wait — keep it in the source loop), and the tolerated-404 retry (1250–1263) becomes `scheduler.push_retry` with the backon delay from Task 1's schedule.

- [ ] **Step 4: Run — PASS** (`cargo test -p multimux --all-features --locked --lib source::pull`, the three `*_pull` suites, `glass_to_glass`, `golden_gate`).

- [ ] **Step 5: Revert-checks** (each run, each recorded):
  1. In `PullScheduler::next` replace the `select!` with `tokio::time::sleep(hint).await; self.inflight.join_next().await` (the pre-fix inline hint sleep): `a_fetch_completing_during_a_waitms_hint_is_returned_when_it_completes` FAILS — measured `left: 1s / right: 100ms`. (`a_pre_fix_inline_hint_sleep_would_take_the_whole_hint` is the in-file measurement of exactly that ordering: 1000 ms for the same inputs, so the 100 ms assertion is discriminating.)
  2. In `pump` swap to `self.backlog.pop_front().or_else(|| self.retries.pop_front())`: `retries_take_the_slot_before_the_backlog` FAILS.
  3. Reintroduce `Action::WaitMs(ms) => tokio::time::sleep(Duration::from_millis(ms)).await` in the migrated `run_hls_pull`: `hls_pull_has_no_inline_sleep_outside_the_scheduler` FAILS (line number reported).
  Steps 1 and 2 were run against the scratch copy of the module above and fail as stated.

- [ ] **Step 6: Commit**

```bash
git add multimux && git commit -m "refactor(multimux): one JoinSet+backon pull scheduler; a WaitMs hint never starves a ready fetch (SP6.2, defect 5)"
```

---

### Task 3: `socket2` UDP/multicast binds with configurable `SO_RCVBUF`/`SO_REUSEADDR`/interface (SP1.6)

**Files:** `source/udp.rs` (`bind_udp` 21–50), `config.rs` (add `recv_buffer_bytes: Option<usize>`, `reuse_address: bool`, `multicast_interface: Option<String>` on the UDP input specs — additive serde defaults). New test `multimux/tests/udp_bind.rs`.

**Verified API:** `socket2::Socket::{new, bind, set_recv_buffer_size, get_recv_buffer_size, set_reuse_address, join_multicast_v4_n, from}`; `socket2::{Domain, Type, Protocol, SockAddr}`; `tokio::net::UdpSocket::from_std`.

- [ ] **Step 1: Write the failing test**

```rust
//! SP1.6: the UDP binds go through socket2 with configurable SO_RCVBUF and
//! SO_REUSEADDR. The RCVBUF assertion round-trips the REQUESTED value (the
//! OS clamps/doubles — Linux doubles and floors at a minimum, macOS reports
//! set+overhead — so comparing against the OS default proves nothing; the
//! getter proves the option was APPLIED).

use std::time::Duration;

#[tokio::test]
async fn the_configured_receive_buffer_is_applied_to_the_socket() {
    // 64 KiB: below every observed OS default (macOS ~786 KiB, Linux ~208 KiB),
    // so the getter returning ~64 KiB (Linux reports the doubled 128 KiB)
    // proves the option took; a plain bind reports the untouched default.
    let opts = multimux::source::udp::UdpBindOptions {
        recv_buffer_bytes: Some(64 * 1024),
        reuse_address: false,
        multicast_interface: None,
    };
    let socket = multimux::source::udp::bind_udp("127.0.0.1:0", None, opts).await.unwrap();
    let got = socket.recv_buffer_size_for_test();
    // Linux doubles the request; macOS adds overhead. Accept 64..=256 KiB,
    // and assert it is NOT the untouched default the plain socket reports.
    assert!(
        got >= 64 * 1024 && got <= 256 * 1024,
        "SO_RCVBUF was not applied: got {got} (expected the requested 64 KiB, possibly doubled)"
    );
    // On a host whose OS default happens to equal the doubled request this
    // would be flaky — so do NOT compare against the plain socket; the
    // getter-vs-request range above is the assertion (Linux doubles 64 KiB
    // to 128 KiB; macOS reports 64 KiB+overhead; both land in range).
}

#[tokio::test]
async fn reuse_address_is_applied_and_a_multicast_group_can_join_a_specified_interface() {
    let opts = multimux::source::udp::UdpBindOptions {
        recv_buffer_bytes: None,
        reuse_address: true,
        multicast_interface: Some("0.0.0.0".into()),
    };
    // A multicast join needs a group route on the host; on a host without
    // one the join errors — this test asserts the OPTION PATH, tolerating
    // the join failure only when it is EADDRNOTAVAIL/EHOSTUNREACH.
    let socket = match multimux::source::udp::bind_udp(
        "0.0.0.0:0",
        Some("239.255.0.1"),
        opts,
    )
    .await
    {
        Ok(s) => s,
        Err(e) if join_unavailable(&e) => return, // no multicast route on this host
        Err(e) => panic!("bind failed unexpectedly: {e}"),
    };
    assert!(socket.local_addr().unwrap().port() > 0);
}
```

`recv_buffer_size_for_test` is a `#[doc(hidden)]` helper on the returned `tokio::net::UdpSocket` (a small extension trait in the test file), defined concretely:

```rust
trait RecvBufProbe {
    fn recv_buffer_size_for_test(&self) -> usize;
}
impl RecvBufProbe for tokio::net::UdpSocket {
    fn recv_buffer_size_for_test(&self) -> usize {
        use std::os::fd::AsRawFd;
        let sock = unsafe { socket2::Socket::from_raw_fd(self.as_raw_fd()) };
        let got = sock.get_recv_buffer_size().expect("SO_RCVBUF readable");
        std::mem::forget(sock); // do not close the borrowed fd
        got
    }
}
```

`join_unavailable(&e)` (the multicast test's skip arm) is a local `fn` matching the errno text of `EADDRNOTAVAIL`/`EHOSTUNREACH`/`ENETUNREACH` in the error string (the same tolerance dvb-stream's W1 multicast test used).

- [ ] **Step 2: Run pre-fix — FAIL.** `UdpBindOptions`/`bind_udp(addr, group, opts)` do not exist (the current `bind_udp` takes two args and never sets options); the compile failure is the pre-fix observable, and with a temporary three-arg shim that ignores `opts`, the RCVBUF assertion FAILS against the untouched OS default (`got == plain_got`). Record.

- [ ] **Step 3: Implement (real code)**

```rust
pub(crate) struct UdpBindOptions {
    pub recv_buffer_bytes: Option<usize>,
    pub reuse_address: bool,
    pub multicast_interface: Option<String>,
}

pub(crate) async fn bind_udp(
    addr: &str,
    multicast_group: Option<&str>,
    opts: UdpBindOptions,
) -> Result<tokio::net::UdpSocket> {
    let bind_addr: std::net::SocketAddr = addr.parse().map_err(|e| MultimuxError::Connect {
        reason: format!("bad UDP bind address {addr:?}: {e}"),
    })?;
    let domain = socket2::Domain::for_address(bind_addr);
    let socket = socket2::Socket::new(domain, socket2::Type::DGRAM, Some(socket2::Protocol::UDP))
        .map_err(|e| MultimuxError::Connect { reason: format!("udp socket: {e}") })?;
    // SO_REUSEADDR before bind (RFC-style two-listener tests rely on it being
    // a deliberate option, not a default — dvb-stream's W1 made the same call).
    if opts.reuse_address {
        socket
            .set_reuse_address(true)
            .map_err(|e| MultimuxError::Connect { reason: format!("SO_REUSEADDR: {e}") })?;
    }
    if let Some(bytes) = opts.recv_buffer_bytes {
        // The OS clamps (Linux doubles, floors at rmem_default); the getter
        // proves application, the value is a request not a guarantee.
        let _ = socket.set_recv_buffer_size(bytes);
    }
    let sin = socket2::SockAddr::from(bind_addr);
    socket.bind(&sin).map_err(|e| MultimuxError::Connect {
        reason: format!("udp bind {addr}: {e}"),
    })?;
    socket.set_nonblocking(true).map_err(|e| MultimuxError::Connect {
        reason: format!("udp nonblocking: {e}"),
    })?;
    if let Some(group) = multicast_group {
        let group_ip: std::net::IpAddr = group.parse().map_err(|e| MultimuxError::Connect {
            reason: format!("bad multicast group {group:?}: {e}"),
        })?;
        match group_ip {
            std::net::IpAddr::V4(v4) => {
                let iface: std::net::Ipv4Addr = opts
                    .multicast_interface
                    .as_deref()
                    .map(str::parse)
                    .transpose()
                    .map_err(|e| MultimuxError::Connect {
                        reason: format!("bad multicast interface: {e}"),
                    })?
                    .unwrap_or(std::net::Ipv4Addr::UNSPECIFIED);
                socket
                    .join_multicast_v4_n(&v4, &iface)
                    .map_err(|e| MultimuxError::Connect {
                        reason: format!("join multicast group {group}: {e}"),
                    })?;
            }
            std::net::IpAddr::V6(v6) => {
                socket
                    .join_multicast_v6(&v6, opts.multicast_interface.as_deref().and_then(|s| s.parse().ok()).unwrap_or(0))
                    .map_err(|e| MultimuxError::Connect {
                        reason: format!("join multicast group {group}: {e}"),
                    })?;
            }
        }
    }
    let std_socket: std::net::UdpSocket = socket.into();
    Ok(tokio::net::UdpSocket::from_std(std_socket)
        .map_err(|e| MultimuxError::Connect { reason: format!("udp async: {e}") })?)
}
```

- [ ] **Step 4: Run — PASS; Step 5: Revert-check** — remove the `set_recv_buffer_size` call; `the_configured_receive_buffer_is_applied…` FAILS (`got == plain_got`, the untouched default). Restore.

- [ ] **Step 6: Commit**

```bash
git add multimux && git commit -m "refactor(multimux): socket2 UDP binds with configurable SO_RCVBUF/REUSEADDR/interface (SP1.6)"
```

---

### Task 4: One generic ingest scaffold (SP6.1) — real media-plane types, a trait-based per-source step, the #1083 stall check kept

**Files:**
- New `multimux/src/source/driver.rs` (register `mod driver;` in `source/mod.rs`).
- Modify `source/rtsp.rs`, `source/{rtp_udp,ts_udp,ts_http,srt}.rs` to implement `IngestStep` and call the scaffold; `source/file_reader.rs`, `source/rtmp.rs`, `source/whip.rs` are NOT migrated (reasons below).
- Tests live INSIDE `source/driver.rs` (`#[cfg(test)] mod tests`): the scaffold is `pub(crate)`, which an integration test cannot name. The existing loopback regression `source::rtsp::tests::a_media_silent_session_fails_on_the_read_timeout` (main rtsp.rs:2032) must keep passing through the migration — it is the end-to-end witness for R1 below.

**Real types this scaffold is written against** (all grep-verified on main 31ba01ec):
- `media_plane::ingress::IngestDriver<S>`: `feed`, `on_deadline`, `poll_transmit`, `next_deadline`, `health`, `finish`, `into_health(self)` (ingress.rs:822/850/887/896/907/864/933); `HealthState::{Establishing, Live, Ended, Failed(E), HandshakeTimedOut{deadline}}` + `is_running()` (ingress.rs:666-718).
- `broadcast_common::Timestamp::from_instant(base: std::time::Instant, now: std::time::Instant)` (stage.rs:129), `Timestamp::{from_nanos, as_nanos}`.
- `crate::source::{DriverProgress, advance_route(&IngestDriver, &RouteHandle, &mut DriverProgress), release_route(&IngestDriver, &RouteHandle), IngestTimeouts{connect, read}}` (source/mod.rs:415/636/675/712).
- `MultimuxError::{Connect{reason}, Protocol{phase: &'static str, reason}}` (error.rs:49/57) — no `Cancelled`/`SessionEnded` variant exists.
- Session error types differ: `RtspIngestSession::Error = MultimuxError` (rtsp.rs:394) and `RtpUdpIngestSession::Error = MultimuxError` (rtp_udp.rs:182), but `TsIngestSession::Error = Infallible` (ts_program.rs:308), and that one session type serves ts_udp, srt and ts_http (their `Dialer::Error = Infallible` at ts_udp.rs:99 / srt.rs:210 / ts_http.rs:141) — hence the `map_failed: fn(S::Error) -> MultimuxError` argument (`|e| e` vs `|never| match never {}`).

**Design — why a trait, not a closure.** `IngestSession::In<'_>` is a lifetime-bound associated type, so the scaffold cannot name the input; each source therefore supplies its own *read + `driver.feed`*. The first draft passed a `for<'a> FnMut(&'a mut IngestDriver<S>, &'a mut W, Duration) -> Pin<Box<dyn Future + 'a>>` closure; that does not compile (a returned future cannot borrow closure-captured state — the lending-closure problem — and `async move` would move the read half out of an `FnMut`), and `W: for<'a> &'a mut W: AsyncWrite` is not valid syntax. The fix is a small trait whose `step(&mut self, ..)` owns the source's state (`rd`, `buf`, `start`) in a struct, with `-> impl Future<Output = StepOutcome> + Send` (RPITIT; each source writes a plain `async fn`). The scaffold takes `source: &mut T` and the write half `writer: &mut W` as SEPARATE arguments, so no borrow ever crosses a returned future. Nothing is spawned with a local borrow: the scaffold is awaited inline by the supervisor attempt, and the tests below `.await`/`join!` it directly.

```rust
//! source/driver.rs — the shared drive loop for the one-connection dial
//! sources (SP6.1). Each source keeps its own bounded read + `driver.feed`
//! (an `IngestStep`); the deadline / cancel / bounded-write / stall /
//! advance / terminal-tail scaffolding lives here once.
//!
//! COMPILE-VERIFIED: this file was built and its five tests run in a scratch
//! crate against main's `media-plane`, `broadcast-common` and `multimux`
//! public API (only the three `use crate::` lines differ).
use std::future::Future;
use std::sync::Arc;
use std::time::Duration;

use broadcast_common::Timestamp;
use media_plane::ingress::{HealthState, IngestDriver, IngestSession};
use crate::error::MultimuxError;
use crate::route::RouteHandle;
use crate::source::{DriverProgress, IngestTimeouts, advance_route, release_route};
use tokio::io::{AsyncWrite, AsyncWriteExt};
use tokio::time::Instant;
use tokio_util::sync::CancellationToken;

/// What one source `step` observed.
pub(crate) enum StepOutcome {
    /// Bytes arrived from the peer and were fed to the driver. ONLY this
    /// outcome resets the stall clock.
    Received,
    /// The bounded read window elapsed with nothing received.
    Idle,
    /// The transport ended cleanly.
    Eof,
    /// The transport failed; the source's own typed error.
    Failed(MultimuxError),
}

/// The per-source half.
pub(crate) trait IngestStep<S: IngestSession>: Send {
    /// ONE bounded read (at most `window`) plus the source's own
    /// `driver.feed(..)`.
    fn step(
        &mut self,
        driver: &mut IngestDriver<S>,
        window: Duration,
    ) -> impl Future<Output = StepOutcome> + Send;

    /// The error for "no bytes for `read`".
    fn stalled(&self, read: Duration) -> MultimuxError {
        MultimuxError::Protocol {
            phase: "recv",
            reason: format!("no data within {read:?}"),
        }
    }
}

fn ts(start: Instant, now: Instant) -> Timestamp {
    Timestamp::from_instant(start.into_std(), now.into_std())
}

/// Default bound on one outbound write (defect 4: pre-fix `run_rtsp`'s
/// `wr.write_all` had none). Not a field on `IngestTimeouts` — that struct is
/// built by literal in public code and tests, so adding a field would break
/// them; the scaffold takes it as an argument.
pub(crate) const DEFAULT_WRITE_TIMEOUT: Duration = Duration::from_secs(10);

/// The drive loop. Takes the driver BY VALUE (the terminal tail consumes it
/// with `into_health`, exactly as `run_rtsp` does today).
///
/// Order per iteration, preserving main's `run_rtsp` (rtsp.rs:858-934):
/// cancel check -> session deadline (`on_deadline`) -> drain `poll_transmit`
/// under `write_timeout` -> health check -> **stall check** -> one bounded
/// `source.step` raced against `cancel` -> `advance_route`.
///
/// **Stall semantics (issue #1083 item 1, preserved):** the stall clock
/// `last_rx` is reset ONLY by `StepOutcome::Received`. `Idle` (the read window
/// elapsed with nothing received — including a window shortened to wake for
/// a keepalive) never resets it, so a peer that holds the socket open and
/// answers keepalives but sends nothing fails the session `read` after the
/// last byte, exactly as main does.
#[allow(clippy::too_many_arguments)]
pub(crate) async fn run_ingest_scaffold<S, W, T>(
    mut driver: IngestDriver<S>,
    route_handle: &Arc<RouteHandle>,
    progress: &mut DriverProgress,
    cancel: CancellationToken,
    timeouts: IngestTimeouts,
    write_timeout: Duration,
    writer: &mut W,
    source: &mut T,
    map_failed: fn(S::Error) -> MultimuxError,
) -> MultimuxError
where
    S: IngestSession + Sync,
    S::Error: Send + Sync,
    S::Request: AsRef<[u8]>,
    W: AsyncWrite + Unpin + Send,
    T: IngestStep<S>,
{
    let start = Instant::now();
    let mut last_rx = start;
    loop {
        if cancel.is_cancelled() {
            release_route(&driver, route_handle);
            driver.finish();
            return MultimuxError::Connect { reason: "cancelled".into() };
        }
        let now = ts(start, Instant::now());
        if let Some(deadline) = driver.next_deadline()
            && now >= deadline
        {
            driver.on_deadline(now);
        }
        while let Some(bytes) = driver.poll_transmit() {
            match tokio::time::timeout(write_timeout, writer.write_all(bytes.as_ref())).await {
                Ok(Ok(())) => {}
                Ok(Err(e)) => {
                    return MultimuxError::Connect { reason: format!("write: {e}") };
                }
                Err(_) => {
                    return MultimuxError::Connect {
                        reason: format!("write: no progress within {write_timeout:?}"),
                    };
                }
            }
        }
        if !driver.health().is_running() {
            break;
        }
        if last_rx.elapsed() >= timeouts.read {
            return source.stalled(timeouts.read);
        }
        let keepalive_wait = driver.next_deadline().map(|d| {
            let now = ts(start, Instant::now());
            Duration::from_nanos(d.as_nanos().saturating_sub(now.as_nanos()))
        });
        let remaining = timeouts.read.saturating_sub(last_rx.elapsed());
        let window = keepalive_wait.map_or(remaining, |w| w.min(remaining));
        let outcome = tokio::select! {
            biased;
            () = cancel.cancelled() => None,
            o = source.step(&mut driver, window) => Some(o),
        };
        let Some(outcome) = outcome else {
            release_route(&driver, route_handle);
            driver.finish();
            return MultimuxError::Connect { reason: "cancelled".into() };
        };
        match outcome {
            StepOutcome::Received => {
                last_rx = Instant::now();
                advance_route(&driver, route_handle, progress).await;
            }
            StepOutcome::Idle => {}
            StepOutcome::Eof => {
                driver.finish();
                break;
            }
            StepOutcome::Failed(e) => return e,
        }
    }
    advance_route(&driver, route_handle, progress).await;
    match driver.into_health() {
        HealthState::Failed(e) => map_failed(e),
        HealthState::Ended => MultimuxError::Connect {
            reason: "session ended (stream completed)".to_string(),
        },
        HealthState::HandshakeTimedOut { deadline } => MultimuxError::Connect {
            reason: format!("handshake timed out at {deadline:?}"),
        },
        other => MultimuxError::Connect {
            reason: format!("session ended: {:?}", std::mem::discriminant(&other)),
        },
    }
}
```

**What `StepOutcome` buys (R1).** The first draft mapped a read timeout to `Fed`, and the scaffold's `Fed` arm set `last_rx = now` — so a peer that holds the TCP connection open and sends nothing was never failed, silently dropping main's #1083 stall check (`if last_rx.elapsed() >= read_timeout { return Protocol{"recv","no data within .."} }`, rtsp.rs:889). Now `Received` is the ONLY outcome that touches `last_rx`; `Idle` never does; the check sits before every step. A source whose historical error text differs overrides `IngestStep::stalled` (ts_udp keeps `Connect { "ts/udp recv: no data within {read:?}" }`).

**Per-source steps (rtsp and ts_udp compile-verified against main's real session types):**

```rust
use std::time::Duration;
use broadcast_common::Timestamp;
use media_plane::ingress::IngestDriver;
use tokio::io::{AsyncRead, AsyncReadExt};
use tokio::net::UdpSocket;
use tokio::time::Instant;
use crate::error::MultimuxError;
use crate::source::driver::{IngestStep, StepOutcome};
use crate::source::rtsp::RtspIngestSession;
use crate::source::ts_program::TsIngestSession;

type BoxedRead = Box<dyn AsyncRead + Send + Unpin>;

/// rtsp.rs: the read half + scratch buffer + the clock origin.
pub(crate) struct RtspStep {
    pub rd: BoxedRead,
    pub buf: Vec<u8>,
    pub start: Instant,
}

impl IngestStep<RtspIngestSession> for RtspStep {
    async fn step(&mut self, driver: &mut IngestDriver<RtspIngestSession>, window: Duration) -> StepOutcome {
        match tokio::time::timeout(window, self.rd.read(&mut self.buf)).await {
            Ok(Ok(0)) => StepOutcome::Eof,
            Ok(Ok(n)) => {
                let now = Timestamp::from_instant(self.start.into_std(), Instant::now().into_std());
                driver.feed(&self.buf[..n], now);
                StepOutcome::Received
            }
            Ok(Err(e)) => StepOutcome::Failed(MultimuxError::Protocol { phase: "recv", reason: e.to_string() }),
            Err(_) => StepOutcome::Idle,
        }
    }
}

/// ts_udp.rs
pub(crate) struct TsUdpStep {
    pub socket: UdpSocket,
    pub buf: Vec<u8>,
    pub start: Instant,
}

impl IngestStep<TsIngestSession> for TsUdpStep {
    async fn step(&mut self, driver: &mut IngestDriver<TsIngestSession>, window: Duration) -> StepOutcome {
        match tokio::time::timeout(window, self.socket.recv(&mut self.buf)).await {
            Ok(Ok(n)) => {
                let now = Timestamp::from_instant(self.start.into_std(), Instant::now().into_std());
                driver.feed(&self.buf[..n], now);
                StepOutcome::Received
            }
            Ok(Err(e)) => StepOutcome::Failed(MultimuxError::Connect { reason: format!("udp recv: {e}") }),
            Err(_) => StepOutcome::Idle,
        }
    }
    fn stalled(&self, read: Duration) -> MultimuxError {
        MultimuxError::Connect { reason: format!("ts/udp recv: no data within {read:?}") }
    }
}
```

The RTSP call site (also compile-verified, including that the whole future is `Send`):

```rust
pub(crate) async fn call_rtsp(
    driver: IngestDriver<RtspIngestSession>,
    route_handle: std::sync::Arc<crate::route::RouteHandle>,
    cancel: tokio_util::sync::CancellationToken,
    timeouts: crate::source::IngestTimeouts,
    mut wr: Box<dyn tokio::io::AsyncWrite + Send + Unpin>,
    mut step: RtspStep,
) -> MultimuxError {
    let mut progress = crate::source::DriverProgress::new();
    crate::source::driver::run_ingest_scaffold(driver, &route_handle, &mut progress, cancel, timeouts,
        Duration::from_secs(10), &mut wr, &mut step, |e| e).await
}
```

In `run_rtsp` the connect/dial prologue (main 803-851) is unchanged; the `loop { .. }` (858-934) and the terminal `match driver.into_health()` tail (946-960) are DELETED in favour of that call; `cancel` is the token W2b-1 Task 8 threads into `InputCtx::cancel`. `RtspIngestSession` already implements `poll_transmit() -> Option<Bytes>` (rtsp.rs:456), so the scaffold's bounded write replaces the unbounded `wr.write_all` (876) — the defect-4 fix on the RTSP SOURCE.

- `ts_udp.rs`, `rtp_udp.rs`, `srt.rs`, `ts_http.rs`: each gets a step struct like `TsUdpStep` (`socket.recv` / `sock.recv()` / `stream.next()` under `timeout(window, ..)`, `driver.feed(.., now)`, `Received`; timeout -> `Idle`). `recv_and_feed`'s own timeout block (~16-18 lines each) is deleted; ts_udp's step also calls `route_handle.feed_si_ts(&buf[..n])` (the DVR EIT hook, ts_udp.rs:190, crate-private) from a held `Arc<RouteHandle>`. `map_failed` is `|never| match never {}` for the `Infallible` sessions. **Behaviour changes to record in the CHANGELOG and re-run the suites for:** these loops previously never called `on_deadline` and never checked `health()`, so a session that went `HandshakeTimedOut` kept reading until the stall; the scaffold now ends the route promptly with `Connect { "handshake timed out .." }`.
- `file_reader.rs` (loop 1383-1464): NOT migrated — its pacer IS its read loop and `In<'a> = ()`; forcing it through a byte-shaped step would re-add a fixed-sleep back door. `rtmp.rs`/`whip.rs` (`ListenDriver`, one concurrent read per session): NOT migrated — many sessions, one loop. All three recorded in `.delegate/w2b-report.md`.

- [ ] **Step 1: Write the failing tests.** Append this module to `source/driver.rs`. **All five tests compile and pass in the scratch crate**, and every revert-check below was run and fails as stated.

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use broadcast_common::{Demand, Stage};
    use bytes::Bytes;
    use media_plane::ingress::{HandshakePolicy, SessionEvent};
    use media_plane::trunk::TrunkConfig;
    use std::convert::Infallible;

    fn nz(n: usize) -> std::num::NonZeroUsize {
        std::num::NonZeroUsize::new(n).unwrap()
    }

    /// A session that never establishes and never yields media. Optional
    /// keepalive: `next_deadline` is `Some(t)` and `on_deadline` re-arms it
    /// `every` later; each firing queues `tx` (the bytes `poll_transmit`
    /// hands the scaffold to write).
    struct QuietSession {
        deadline: Option<u64>,
        every: u64,
        tx: Option<Bytes>,
        tx_per_deadline: Bytes,
    }

    impl QuietSession {
        fn silent() -> Self {
            Self { deadline: None, every: 0, tx: None, tx_per_deadline: Bytes::new() }
        }
        fn keepalive(every: Duration) -> Self {
            let every = u64::try_from(every.as_nanos()).unwrap();
            Self {
                deadline: Some(every),
                every,
                tx: None,
                tx_per_deadline: Bytes::from_static(b"OPTIONS"),
            }
        }
        fn writing(tx: Bytes) -> Self {
            Self { deadline: None, every: 0, tx: Some(tx), tx_per_deadline: Bytes::new() }
        }
    }

    impl Stage for QuietSession {
        type In<'a> = &'a [u8];
        type Out = SessionEvent;
        type Error = Infallible;
        fn demand(&self) -> Demand {
            Demand::new(4096)
        }
        fn feed(&mut self, _i: &[u8], _n: Timestamp) -> Result<(), Infallible> {
            Ok(())
        }
        fn poll(&mut self) -> Option<SessionEvent> {
            None
        }
        fn next_deadline(&self) -> Option<Timestamp> {
            self.deadline.map(Timestamp::from_nanos)
        }
        fn on_deadline(&mut self, now: Timestamp) {
            self.deadline = Some(now.as_nanos() + self.every);
            self.tx = Some(self.tx_per_deadline.clone());
        }
        fn finish(&mut self) -> Result<(), Infallible> {
            Ok(())
        }
    }

    impl IngestSession for QuietSession {
        type Request = Bytes;
        fn poll_transmit(&mut self) -> Option<Bytes> {
            self.tx.take()
        }
    }

    fn driver(session: QuietSession) -> IngestDriver<QuietSession> {
        IngestDriver::new(
            session,
            TrunkConfig::new(nz(4), nz(4), nz(4), nz(4), nz(4)),
            HandshakePolicy::establish_by(Timestamp::from_nanos(u64::MAX)),
            nz(4),
        )
    }

    fn route() -> Arc<RouteHandle> {
        Arc::new(RouteHandle::new(1.0, 250, 8))
    }

    /// Scripted source: each call sleeps `delay` then returns the outcome
    /// (the last entry repeats forever).
    struct Script(Vec<(Duration, fn() -> StepOutcome)>, usize);

    impl IngestStep<QuietSession> for Script {
        async fn step(
            &mut self,
            _driver: &mut IngestDriver<QuietSession>,
            window: Duration,
        ) -> StepOutcome {
            let i = self.1.min(self.0.len() - 1);
            self.1 += 1;
            let (delay, outcome) = self.0[i];
            // A source honours its window: a read longer than the window is
            // an Idle (timeout) at the window.
            tokio::time::sleep(delay.min(window)).await;
            if delay > window { StepOutcome::Idle } else { outcome() }
        }
    }

    /// A source whose read just waits out its window (what
    /// `timeout(window, rd.read(..))` does against a peer that sends nothing).
    struct WaitsOutWindow;
    impl IngestStep<QuietSession> for WaitsOutWindow {
        async fn step(
            &mut self,
            _driver: &mut IngestDriver<QuietSession>,
            window: Duration,
        ) -> StepOutcome {
            tokio::time::sleep(window).await;
            StepOutcome::Idle
        }
    }

    struct Pending;
    impl IngestStep<QuietSession> for Pending {
        async fn step(
            &mut self,
            _driver: &mut IngestDriver<QuietSession>,
            _window: Duration,
        ) -> StepOutcome {
            std::future::pending().await
        }
    }

    fn timeouts(read: Duration) -> IngestTimeouts {
        IngestTimeouts { connect: Duration::from_secs(5), read }
    }

    const MAP: fn(Infallible) -> MultimuxError = |n| match n {};

    /// Owns every borrowed local so the returned future is self-contained
    /// (nothing here is spawned; callers `.await` or `join!` it directly).
    async fn run<T: IngestStep<QuietSession>>(
        session: QuietSession,
        mut source: T,
        read: Duration,
        write_timeout: Duration,
        pipe: usize,
        cancel: CancellationToken,
    ) -> MultimuxError {
        let (mut w, _peer) = tokio::io::duplex(pipe);
        let route = route();
        let mut progress = DriverProgress::new();
        run_ingest_scaffold(
            driver(session),
            &route,
            &mut progress,
            cancel,
            timeouts(read),
            write_timeout,
            &mut w,
            &mut source,
            MAP,
        )
        .await
    }

    /// Issue #1083 item 1, preserved: a peer that holds the connection open
    /// and ANSWERS keepalives (so the session keeps waking) but sends no
    /// bytes must fail at `read`, not be kept alive by the wakeups.
    ///
    /// Revert-check: change `StepOutcome::Idle => {}` in the scaffold to
    /// `StepOutcome::Idle => { last_rx = Instant::now(); }` (the "map a
    /// timeout to Fed" mistake) and this test fails: the scaffold never
    /// returns and the outer 60 s virtual timeout fires.
    #[tokio::test(start_paused = true)]
    async fn a_silent_peer_fails_at_the_stall_deadline_despite_keepalive_wakeups() {
        let t0 = Instant::now();
        let err = tokio::time::timeout(
            Duration::from_secs(60),
            run(
                QuietSession::keepalive(Duration::from_secs(10)),
                WaitsOutWindow,
                Duration::from_secs(35),
                Duration::from_secs(1),
                1024,
                CancellationToken::new(),
            ),
        )
        .await
        .expect("a silent peer must fail the session, not be kept alive by keepalive wakeups");
        assert!(
            matches!(&err, MultimuxError::Protocol { phase: "recv", reason } if reason.contains("no data within")),
            "{err:?}"
        );
        assert_eq!(t0.elapsed(), Duration::from_secs(35), "fails exactly at the stall window");
    }

    /// Received bytes DO reset the stall clock (the over-eager-failure
    /// guard): receives at t=20 s and t=40 s, then silence, fail at 40+30 s.
    ///
    /// Revert-check: delete `last_rx = Instant::now();` from the `Received`
    /// arm; the stall then fires at 30 s and the elapsed assertion fails.
    #[tokio::test(start_paused = true)]
    async fn received_bytes_reset_the_stall_clock() {
        let t0 = Instant::now();
        let src = Script(
            vec![
                (Duration::from_secs(20), || StepOutcome::Received),
                (Duration::from_secs(20), || StepOutcome::Received),
                (Duration::from_secs(1000), || StepOutcome::Idle),
            ],
            0,
        );
        let err = tokio::time::timeout(
            Duration::from_secs(500),
            run(
                QuietSession::silent(),
                src,
                Duration::from_secs(30),
                Duration::from_secs(1),
                1024,
                CancellationToken::new(),
            ),
        )
        .await
        .expect("must fail on the stall, not hang");
        assert!(matches!(&err, MultimuxError::Protocol { phase: "recv", .. }), "{err:?}");
        assert_eq!(t0.elapsed(), Duration::from_secs(70));
    }

    /// Defect 4: a stalled outbound write fails at the write bound.
    ///
    /// Revert-check: replace the `tokio::time::timeout(write_timeout, ..)`
    /// around `write_all` with a bare `writer.write_all(..).await`; the 5 s
    /// outer timeout fires and the `.expect` panics.
    #[tokio::test(start_paused = true)]
    async fn a_stalled_outbound_write_fails_at_the_write_bound() {
        let t0 = Instant::now();
        // A 64 B request into a 16 B pipe nobody reads.
        let err = tokio::time::timeout(
            Duration::from_secs(5),
            run(
                QuietSession::writing(Bytes::from(vec![0u8; 64])),
                Pending,
                Duration::from_secs(30),
                Duration::from_secs(1),
                16,
                CancellationToken::new(),
            ),
        )
        .await
        .expect("the stalled write must fail at the write bound, not hang");
        assert!(
            matches!(&err, MultimuxError::Connect { reason } if reason.contains("write: no progress")),
            "{err:?}"
        );
        assert_eq!(t0.elapsed(), Duration::from_secs(1));
    }

    /// Cancel beats a step that never resolves.
    ///
    /// Revert-check: drop the `() = cancel.cancelled() => None` arm of the
    /// `select!`; the step never resolves and the 5 s outer timeout panics.
    #[tokio::test(start_paused = true)]
    async fn cancel_beats_a_pending_step() {
        let cancel = CancellationToken::new();
        let canceller = cancel.clone();
        let (err, ()) = tokio::time::timeout(Duration::from_secs(5), async {
            tokio::join!(
                run(
                    QuietSession::silent(),
                    Pending,
                    Duration::from_secs(30),
                    Duration::from_secs(1),
                    1024,
                    cancel,
                ),
                async move {
                    tokio::time::sleep(Duration::from_millis(100)).await;
                    canceller.cancel();
                }
            )
        })
        .await
        .expect("cancel must end the scaffold promptly");
        assert!(matches!(&err, MultimuxError::Connect { reason } if reason == "cancelled"), "{err:?}");
    }

    /// The scaffold future must be `Send` (it runs inside the supervisor's
    /// spawned task). Compile-time witness: if a bound or borrow makes the
    /// future !Send this stops compiling.
    #[test]
    fn the_scaffold_future_is_send() {
        fn assert_send<T: Send>(_: &T) {}
        assert_send(&run(
            QuietSession::silent(),
            WaitsOutWindow,
            Duration::from_secs(1),
            Duration::from_secs(1),
            8,
            CancellationToken::new(),
        ));
    }
}
```

- [ ] **Step 2: Run pre-fix.** The scaffold does not exist yet, so land the tests first against a stub that reproduces the draft's behaviour (`Idle => { last_rx = Instant::now(); }`): `a_silent_peer_fails_at_the_stall_deadline_despite_keepalive_wakeups` FAILS (`Elapsed(())` — the scaffold never returns). For the SOURCE-level defect, also run main's `a_media_silent_session_fails_on_the_read_timeout` after the migration with that stub and see it hang to its 10 s guard.

- [ ] **Step 3: Implement `run_ingest_scaffold` + `IngestStep` (code above); migrate `rtsp.rs` first, then ts_udp/rtp_udp/srt/ts_http.**

- [ ] **Step 4: Run — PASS** (`cargo test -p multimux --all-features --locked --lib source::driver`, then the `rtsp_ingest`, `ts_udp`/`rtp_udp`/`srt`/`ts_http`/`file_reader` suites and `a_media_silent_session_fails_on_the_read_timeout`).

- [ ] **Step 5: Revert-checks** (each was run against the scratch file; failure lines recorded):
  1. **Stall (R1):** `StepOutcome::Idle => {}` -> `StepOutcome::Idle => { last_rx = Instant::now(); }`. `a_silent_peer_fails_at_the_stall_deadline_despite_keepalive_wakeups` FAILS (`a silent peer must fail the session...: Elapsed(())`) and `received_bytes_reset_the_stall_clock` FAILS too.
  2. Delete `last_rx = Instant::now();` from the `Received` arm: `received_bytes_reset_the_stall_clock` FAILS (`left: 30s / right: 70s`).
  3. Replace the `tokio::time::timeout(write_timeout, writer.write_all(..))` with a bare `write_all`: `a_stalled_outbound_write_fails_at_the_write_bound` FAILS (`Elapsed(())`).
  4. Replace `() = cancel.cancelled() => None` with `() = std::future::pending::<()>() => None`: `cancel_beats_a_pending_step` FAILS (panics at the 5 s virtual timeout).

- [ ] **Step 6: Commit**

```bash
git add multimux && git commit -m "refactor(multimux): one generic ingest scaffold (stall/deadline/cancel/bounded-write/advance) for the dial sources (SP6.1, defect 4)"
```

---

### Task 5: RTSP push on `AsyncRtspClient` (ANNOUNCE via sdp-types, RECORD with interleaved send, every IO bounded — defect 4, SP4, SP6.1)

**Files:** `multimux/src/push/rtsp.rs` (the hand-rolled `ClientSession` + raw `TcpStream` loop at 92-148, 223-280; the `build_sdp` `format!` block 340-349); new `multimux/tests/push_rtsp_bounds.rs`. Depends on W2b-1 Task 6's `announce`/`record`/`send_interleaved`.

**What is actually unbounded on main (verified, so the tests target the right thing).** `drive_push` already bounds a stalled FLUSH at `PUSH_FLUSH_TIMEOUT` = 10 s and reconnects (main push/mod.rs:407, 599; exercised by `push::egress::tests::a_push_stalled_on_a_live_peer_times_out_and_reconnects`). What has NO bound at all is inside `RtspTransport` itself: `connect`'s OPTIONS exchange and `setup`'s ANNOUNCE/SETUP/RECORD exchanges (`roundtrip`'s `stream.read` at push/rtsp.rs:243 waits forever for a peer that accepted TCP and never answers) and each `write_all` (:213, :234). The migration puts every one of those under `RtspTimeouts`, and exposes the bound as a config knob so a test can use 300 ms instead of the 10 s/30 s defaults.

- [ ] **Step 1: Golden the ANNOUNCE SDP body first** (it is wire output): add `multimux/tests/golden/rtsp_announce.sdp` via the `GOLDEN_BLESS` pattern (`output/smooth.rs:972`), generated from the current `build_sdp` on main, plus the README entry. Commit separately:

```bash
GOLDEN_BLESS=multimux/tests/golden cargo test -p multimux --all-features --locked --lib push::rtsp::tests::announce_sdp_golden 2>&1 | grep -E 'test result|FAILED'
git add multimux/tests/golden && git commit -m "test(multimux): golden the RTSP ANNOUNCE SDP body before the sdp-types migration"
```

- [ ] **Step 2: Write the failing tests** — `multimux/tests/push_rtsp_bounds.rs`. **Real time, small bounds, no `start_paused`:** the peers are real loopback sockets, and a paused clock auto-advances whenever the runtime is idle waiting on IO, which would fire the connect/response timeouts spuriously. Each test wraps the call under test in a 5 s `GUARD`: a bounded path returns in ~300 ms, an unbounded one trips the guard. Four tests: a CONTROL (a draining record-mode peer accepts connect/setup/send — proves the scripted peer is a faithful server, so the failures below are the transport's), connect against a peer that never answers OPTIONS, `send` against a peer that stopped reading after RECORD, and the end-to-end `drive_push` test over a trunk WITH a video track and a stream of 256 KiB samples (so the write genuinely stalls and the 300 ms transport bound must beat `drive_push`'s own 10 s flush bound).

```rust
//! RTSP push: every awaited IO is bounded (SP6.1, defect 4).
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use multimux::config::{PushFormat, ReconnectPolicy};
use multimux::push::{PushTransport, RtspTransport, RtspTransportConfig, drive_push};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};

/// Real-time guard: a bounded path returns long before this; an unbounded
/// one hangs and trips it.
const GUARD: Duration = Duration::from_secs(5);

/// Tight bounds so a bounded path returns in ~300 ms while an unbounded one
/// hits `GUARD`. `with_timeouts` is the config knob Step 2a adds.
fn config_with_bounds() -> RtspTransportConfig {
    RtspTransportConfig::default().with_timeouts(
        rtsp_runtime::RtspTimeouts::default()
            .with_read_idle(Duration::from_millis(300))
            .with_write(Duration::from_millis(300)),
    )
}

/// How the scripted peer treats the connection after RECORD.
#[derive(Clone, Copy)]
enum AfterRecord {
    /// Hold the socket open and never read again (a live peer that stopped
    /// consuming).
    StopReading,
    /// Keep reading and discarding (a healthy peer).
    Drain,
}

/// Read one RTSP request (headers + `Content-Length` body) off `sock`;
/// returns `(method, cseq)`.
async fn read_request(sock: &mut TcpStream, buf: &mut Vec<u8>) -> Option<(String, String)> {
    loop {
        if let Some(end) = buf.windows(4).position(|w| w == b"\r\n\r\n") {
            let head = String::from_utf8_lossy(&buf[..end]).into_owned();
            let body_len = head
                .lines()
                .find_map(|l| l.to_ascii_lowercase().strip_prefix("content-length:").map(|v| v.trim().parse::<usize>().unwrap_or(0)))
                .unwrap_or(0);
            if buf.len() >= end + 4 + body_len {
                buf.drain(..end + 4 + body_len);
                let method = head.split_whitespace().next()?.to_string();
                let cseq = head
                    .lines()
                    .find_map(|l| l.strip_prefix("CSeq:").map(|v| v.trim().to_string()))?;
                return Some((method, cseq));
            }
        }
        let mut chunk = [0u8; 2048];
        let n = sock.read(&mut chunk).await.ok()?;
        if n == 0 {
            return None;
        }
        buf.extend_from_slice(&chunk[..n]);
    }
}

/// An RTSP record-mode peer: answers OPTIONS/ANNOUNCE/SETUP/RECORD with 200,
/// then behaves per `after`. `accepted` counts connections.
async fn scripted_record_peer(
    listener: TcpListener,
    accepted: Arc<AtomicUsize>,
    after: AfterRecord,
) {
    loop {
        let Ok((mut sock, _)) = listener.accept().await else { return };
        accepted.fetch_add(1, Ordering::SeqCst);
        tokio::spawn(async move {
            let mut buf = Vec::new();
            loop {
                let Some((method, cseq)) = read_request(&mut sock, &mut buf).await else { return };
                let extra = if method == "SETUP" {
                    "Transport: RTP/AVP/TCP;unicast;interleaved=0-1;mode=record\r\nSession: 1\r\n"
                } else {
                    "Session: 1\r\n"
                };
                let resp = format!("RTSP/1.0 200 OK\r\nCSeq: {cseq}\r\n{extra}\r\n");
                if sock.write_all(resp.as_bytes()).await.is_err() {
                    return;
                }
                if method == "RECORD" {
                    break;
                }
            }
            match after {
                AfterRecord::StopReading => std::future::pending::<()>().await,
                AfterRecord::Drain => {
                    let mut sink = [0u8; 65536];
                    while matches!(sock.read(&mut sink).await, Ok(n) if n > 0) {}
                }
            }
        });
    }
}

async fn peer(after: AfterRecord) -> (String, Arc<AtomicUsize>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("rtsp://{}/live/key", listener.local_addr().unwrap());
    let accepted = Arc::new(AtomicUsize::new(0));
    tokio::spawn(scripted_record_peer(listener, Arc::clone(&accepted), after));
    (url, accepted)
}

/// Whole TS packets, a multiple of the 7-packet RTP payload.
const CHUNK: usize = 188 * 7 * 50;

/// CONTROL: the scripted peer is a faithful record-mode server — a healthy
/// (draining) peer accepts connect/setup/send.
#[tokio::test]
async fn control_a_healthy_peer_accepts_connect_setup_and_send() {
    let (url, _) = peer(AfterRecord::Drain).await;
    let mut t = tokio::time::timeout(GUARD, RtspTransport::connect(&url, &config_with_bounds()))
        .await.expect("connect must not hang").expect("connect");
    tokio::time::timeout(GUARD, t.setup(&[])).await.expect("setup must not hang").expect("setup");
    tokio::time::timeout(GUARD, t.send(&vec![0u8; CHUNK])).await.expect("send must not hang").expect("send");
}

/// A peer that accepts TCP and never answers OPTIONS fails `connect` at the
/// response bound.
#[tokio::test]
async fn a_peer_that_never_answers_options_fails_connect_at_the_bound() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("rtsp://{}/live/key", listener.local_addr().unwrap());
    let _held = tokio::spawn(async move {
        let (sock, _) = listener.accept().await.unwrap();
        std::future::pending::<()>().await;
        drop(sock);
    });
    let r = tokio::time::timeout(GUARD, RtspTransport::connect(&url, &config_with_bounds()))
        .await
        .expect("connect must fail at its bound, not hang on a peer that never answers OPTIONS");
    assert!(r.is_err(), "no answer is a failure");
}

/// A peer that completes RECORD then stops reading fails `send` at the write
/// bound instead of blocking forever.
#[tokio::test]
async fn a_stalled_interleaved_write_fails_send_at_the_write_bound() {
    let (url, _) = peer(AfterRecord::StopReading).await;
    let mut t = RtspTransport::connect(&url, &config_with_bounds()).await.expect("connect");
    t.setup(&[]).await.expect("setup");
    let chunk = vec![0u8; CHUNK];
    let sent = tokio::time::timeout(GUARD, async {
        for _ in 0..4096 {
            // 4096 * CHUNK (~330 KB) = ~1.3 GB: far beyond any loopback buffer.
            if t.send(&chunk).await.is_err() {
                return true;
            }
        }
        false
    })
    .await
    .expect("send must fail at the write bound, not block forever against a peer that stopped reading");
    assert!(sent, "a peer that stopped reading must eventually fail a send");
}

fn avc_spec(track_id: u32) -> transmux::ir::TrackSpec {
    transmux::ir::TrackSpec::new(
        track_id,
        90_000,
        transmux::CodecConfig::Avc {
            config: transmux::AVCConfigurationBox::new(transmux::AVCDecoderConfigurationRecord {
                configuration_version: 1,
                profile_indication: 0x42,
                profile_compatibility: 0,
                level_indication: 0x1f,
                length_size_minus_one: 3,
                sps: Vec::new(),
                pps: Vec::new(),
                chroma_format: None,
                bit_depth_luma_minus8: None,
                bit_depth_chroma_minus8: None,
                sps_ext: Vec::new(),
            }),
            width: 0,
            height: 0,
        },
    )
}

fn big_sample() -> transmux::ir::Sample {
    let mut nal = vec![0, 0, 0, 1, 0x65];
    nal.resize(256 * 1024, 0xAA);
    transmux::ir::Sample::new(bytes::Bytes::from(nal), Some(0), Some(0), Some(3_000), true)
}

/// END TO END through `drive_push`: a trunk WITH a video track and a steady
/// stream of large samples, pushed at a peer that stops reading after RECORD.
/// The transport's write bound (300 ms) must trip and `drive_push` must
/// reconnect long before `drive_push`'s own 10 s flush bound would.
#[tokio::test]
async fn drive_push_reconnects_at_the_transport_write_bound_not_the_ten_second_flush_bound() {
    use media_plane::trunk::{RetentionClass, Trunk, TrunkConfig};
    let nz = |n| std::num::NonZeroUsize::new(n).unwrap();
    let trunk = Trunk::new(TrunkConfig::new(nz(64), nz(64), nz(64), nz(64), nz(64)));
    let writer = trunk.writer().expect("writer is free");
    writer.set_tracks(vec![avc_spec(1)]);

    let (url, accepted) = peer(AfterRecord::StopReading).await;
    let cancel = tokio_util::sync::CancellationToken::new();
    let task = tokio::spawn(drive_push::<RtspTransport>(
        Arc::clone(&trunk),
        url,
        config_with_bounds(),
        PushFormat::Ts,
        ReconnectPolicy { initial_backoff_ms: 0, max_backoff_ms: 0, max_attempts: None },
        cancel.clone(),
    ));
    // Load generator: one 256 KiB sample every 2 ms until the 2nd connection.
    let reconnected = tokio::time::timeout(GUARD, async {
        let mut tick = tokio::time::interval(Duration::from_millis(2));
        while accepted.load(Ordering::SeqCst) < 2 {
            tick.tick().await;
            writer.publish(1, RetentionClass::Timed, big_sample());
        }
    })
    .await;
    cancel.cancel();
    let _ = tokio::time::timeout(Duration::from_secs(2), task).await;
    assert!(
        reconnected.is_ok(),
        "the stalled write must be failed by the transport's own bound (reconnect within {GUARD:?}); \
         connections seen: {}",
        accepted.load(Ordering::SeqCst)
    );
}
```

- [ ] **Step 2a: Add the config knob first (unused), so the file compiles, then run PRE-FIX.** `RtspTransportConfig` gains `pub timeouts: rtsp_runtime::RtspTimeouts` (default `RtspTimeouts::default()`) and `pub fn with_timeouts(mut self, t: RtspTimeouts) -> Self`; the old transport ignores it. Run `cargo test -p multimux --all-features --locked --test push_rtsp_bounds`. **Observed against main's transport** (knob stubbed to the default config): `control_a_healthy_peer_accepts_connect_setup_and_send` passes; `a_peer_that_never_answers_options_fails_connect_at_the_bound`, `a_stalled_interleaved_write_fails_send_at_the_write_bound` and `drive_push_reconnects_at_the_transport_write_bound_not_the_ten_second_flush_bound` all FAIL at the 5 s guard (`connect must fail at its bound...: Elapsed(())`, `send must fail at the write bound...: Elapsed(())`, and `connections seen: 1`). Record.

- [ ] **Step 3: Migrate (real shape).** `RtspTransport::connect` becomes `AsyncRtspClient::connect_with_timeouts(addr, ClientSession::new() [+ .with_credentials(..)], config.timeouts)` then `client.options(&url)`; `setup` drives `client.announce(&url, &sdp)` (body from `sdp_types::Session` + `Session::write`, replacing `build_sdp`'s `format!` block), `client.setup(&control_url, &transport)` and `client.record(&url)`; `send` builds the RTP packet as today and writes each chunk with `client.send_interleaved(self.channel, &packet)` (bounded by `RtspTimeouts::write`; the method W2b-1 Task 6 added); the control URL is `Url::path_segments_mut().push("trackID=0")` (W2b-1 Task 2). Auth retries are handled inside the adapter. `RtspTransport` holds `AsyncRtspClient<TcpStream>` by value; `rtsp_runtime::Error` maps to `RtspPushError::Protocol(e.to_string())`.

- [ ] **Step 4: PASS** (`push_rtsp_bounds`, `push_rtsp` against mediamtx, `push::egress::tests`).

- [ ] **Step 5: Revert-checks** (run each): (a) build the client with `RtspTimeouts::default()` instead of `config.timeouts` — the three bound tests FAIL at the 5 s guard (30 s read / 10 s write defaults); (b) in `send`, write the interleaved frame with a bare `TcpStream::write_all` (bypassing `send_interleaved`) — `a_stalled_interleaved_write_fails_send_at_the_write_bound` FAILS; (c) keep `write` but leave `read_idle` at its 30 s default — only `a_peer_that_never_answers_options_fails_connect_at_the_bound` FAILS. Restore each.

- [ ] **Step 6: Commit**

```bash
git add multimux && git commit -m "refactor(multimux)!: RTSP push on AsyncRtspClient (announce/record/send_interleaved); every push IO bounded; ANNOUNCE body via sdp-types"
```

---

### Task 6: RTMP push on rtmp-runtime's `AsyncRtmpClient` + `RtmpTarget` (write bound — defect 4; tcUrl via `RtmpTarget` — defect 8, SP6.1)

**Files:** `multimux/src/push/rtmp.rs` (the hand-rolled `TcpStream` + C0/C1/C2 + `connect`/`publish` exchange 346-395 and the bare `stream.write_all` at 133, 142, 190, 195, 292; the `format!("rtmp://{host}:{port}/{}")` tcUrl at 359 — already `RtmpTarget` if W2b-1 Task 2 took that path); new `multimux/tests/push_rtmp_bounds.rs`.

**Verified API** (main `rtmp-runtime/src/io.rs` + `target.rs`): `AsyncRtmpClient::connect(&RtmpTarget, RtmpTimeouts) -> io::Result<Self>`, `publish(&mut self)`, `send_video/send_audio/send_metadata`, `next_events()`; `RtmpTarget { host, port, app, stream_key, tc_url }` with IPv6 bracketed in `tc_url`; `RtmpTimeouts { connect, handshake, read_idle, write }` + `with_*`.

**What the test reaches.** Every test below goes through **multimux's own `RtmpTransport`** (the public `PushTransport::{connect, send}`), on real loopback sockets, against a real `rtmp_runtime::server::ServerSession` peer — none of them touches `AsyncRtmpClient` directly, so each exercises the push path the migration replaces.

**Honest finding on defect 8 for the push path.** The earlier draft claimed old code emits an unbracketed IPv6 `tcUrl`. It does not: `url::Url::host_str()` keeps the brackets for an IPv6 literal, so main's `format!("rtmp://{host}:{port}/{app}")` already produces `rtmp://[::1]:PORT/live`. This was observed, not assumed: `an_ipv6_push_target_sends_a_bracketed_tc_url` PASSES against main's transport (the bytes the server received are decoded with rtmp-runtime's own `ChunkAssembler` + `amf0::Command`, not substring-matched). It is therefore a **characterisation test that pins the migration** (`RtmpTarget::tc_url` must produce the same wire value), labelled as such. The bite for this task is the write bound.

- [ ] **Step 1: Write the tests** — `multimux/tests/push_rtmp_bounds.rs` (real time, 5 s `GUARD`; same reason as Task 5 for not pausing the clock). Three tests: a CONTROL (IPv4 `tcUrl` is readable from the captured bytes — validates the extraction used below; the client announces its own chunk size via `SetChunkSize` before `connect`, so the assembler is built with `ClientConfig::default().chunk_size`), the IPv6 characterisation (skips LOUDLY if the host has no `[::1]`), and the bite: a peer that completed `publish` and then stopped reading must fail `send` at the write bound.

```rust
//! RTMP push through multimux's `RtmpTransport`: the IPv6 tcUrl the peer
//! actually receives (defect 8) and the write bound on a stalled peer.
use std::time::Duration;

use multimux::push::{PushTransport, RtmpTransport, RtmpTransportConfig};
use rtmp_runtime::amf0::{Amf0Value, Command};
use rtmp_runtime::chunk::ChunkAssembler;
use rtmp_runtime::server::{ServerEvent, ServerSession};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;

const GUARD: Duration = Duration::from_secs(5);
/// RTMP simple handshake bytes the client sends before its first chunk:
/// C0 (1) + C1 (1536) + C2 (1536) (Adobe RTMP 1.0 §5.2).
const HANDSHAKE_PREFIX: usize = 1 + 1536 + 1536;
/// RTMP message type id of an AMF0 command (Adobe RTMP 1.0 §7.1.1).
const MSG_COMMAND_AMF0: u8 = 20;

/// `write_timeout` is the config knob Step 2a adds, mirroring the existing
/// `connect_timeout: Option<Duration>`.
fn config_with_write_bound() -> RtmpTransportConfig {
    RtmpTransportConfig { write_timeout: Some(Duration::from_millis(300)), ..Default::default() }
}

/// What the scripted peer does once the client's `publish` was accepted.
#[derive(Clone, Copy)]
enum AfterPublish {
    /// Read until the client closes, returning every raw byte received.
    DrainToEof,
    /// Hold the socket open and never read again.
    StopReading,
}

/// A real RTMP ingest (`ServerSession`) behind `listener`; returns the raw
/// bytes the client sent (DrainToEof) after the connection ends.
fn spawn_peer(
    listener: TcpListener,
    after: AfterPublish,
) -> tokio::task::JoinHandle<Vec<u8>> {
    tokio::spawn(async move {
        let (mut sock, _) = listener.accept().await.expect("accept");
        let mut session = ServerSession::with_defaults();
        let mut raw = Vec::new();
        let mut buf = vec![0u8; 65536];
        loop {
            let n = match sock.read(&mut buf).await {
                Ok(0) | Err(_) => return raw,
                Ok(n) => n,
            };
            raw.extend_from_slice(&buf[..n]);
            let (out, events) = session.handle_data(&buf[..n]).expect("server decode");
            if !out.is_empty() {
                sock.write_all(&out).await.expect("server reply");
            }
            let published = events.iter().any(|e| matches!(e, ServerEvent::Publish { .. }));
            if published && matches!(after, AfterPublish::StopReading) {
                std::future::pending::<()>().await;
            }
        }
    })
}

/// The `tcUrl` property of the `connect` command in the client's raw bytes.
fn captured_tc_url(raw: &[u8]) -> String {
    assert!(raw.len() > HANDSHAKE_PREFIX, "client sent no chunks after the handshake");
    // The client announces its own chunk size (SetChunkSize) before `connect`
    // and frames everything after it with that size.
    let messages = ChunkAssembler::new()
        .with_chunk_size(rtmp_runtime::client::ClientConfig::default().chunk_size)
        .push(&raw[HANDSHAKE_PREFIX..])
        .expect("the client's chunk stream decodes");
    for m in messages.iter().filter(|m| m.message_type_id == MSG_COMMAND_AMF0) {
        let cmd = Command::parse(&m.payload).expect("amf0 command");
        if cmd.name != "connect" {
            continue;
        }
        let Some(Amf0Value::Object(props)) = cmd.arguments.first() else {
            panic!("connect has no command object: {cmd:?}")
        };
        return props
            .iter()
            .find_map(|(k, v)| match (k.as_str(), v) {
                ("tcUrl", Amf0Value::String(s)) => Some(s.clone()),
                _ => None,
            })
            .expect("connect carries tcUrl");
    }
    panic!("no connect command in the client's bytes");
}

/// CONTROL (IPv4): the extraction works on a healthy push.
#[tokio::test]
async fn control_the_captured_tc_url_is_readable_for_an_ipv4_target() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let peer = spawn_peer(listener, AfterPublish::DrainToEof);
    let cfg = RtmpTransportConfig { stream_key: "key".into(), ..Default::default() };
    let mut t = tokio::time::timeout(GUARD, RtmpTransport::connect(&format!("rtmp://127.0.0.1:{port}/live/key"), &cfg))
        .await.expect("connect must not hang").expect("connect");
    t.close();
    drop(t);
    let raw = tokio::time::timeout(GUARD, peer).await.expect("peer").expect("join");
    assert_eq!(captured_tc_url(&raw), format!("rtmp://127.0.0.1:{port}/live"));
}

/// Defect 8: an IPv6 push target's `tcUrl` must keep its brackets — what the
/// server receives, not what a helper computes.
///
/// Revert-check: build the tcUrl with `format!("rtmp://{host}:{port}/{app}")`
/// from `Url::host_str()` again (host WITHOUT its brackets) and this fails
/// with `rtmp://::1:PORT/live`.
#[tokio::test]
async fn an_ipv6_push_target_sends_a_bracketed_tc_url() {
    let Ok(listener) = TcpListener::bind("[::1]:0").await else {
        eprintln!("SKIP an_ipv6_push_target_sends_a_bracketed_tc_url: no IPv6 loopback on this host");
        return;
    };
    let port = listener.local_addr().unwrap().port();
    let peer = spawn_peer(listener, AfterPublish::DrainToEof);
    let cfg = RtmpTransportConfig { stream_key: "key".into(), ..Default::default() };
    let mut t = tokio::time::timeout(GUARD, RtmpTransport::connect(&format!("rtmp://[::1]:{port}/live/key"), &cfg))
        .await.expect("connect must not hang").expect("connect over IPv6");
    t.close();
    drop(t);
    let raw = tokio::time::timeout(GUARD, peer).await.expect("peer").expect("join");
    assert_eq!(captured_tc_url(&raw), format!("rtmp://[::1]:{port}/live"));
}

/// A peer that accepted the publish and then stopped reading fails `send` at
/// the write bound instead of blocking forever.
#[tokio::test]
async fn a_stalled_peer_fails_send_at_the_write_bound() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let _peer = spawn_peer(listener, AfterPublish::StopReading);
    let cfg = RtmpTransportConfig { stream_key: "key".into(), ..config_with_write_bound() };
    let mut t = tokio::time::timeout(GUARD, RtmpTransport::connect(&format!("rtmp://127.0.0.1:{port}/live/key"), &cfg))
        .await.expect("connect must not hang").expect("connect");
    let chunk = vec![0u8; 256 * 1024];
    let failed = tokio::time::timeout(GUARD, async {
        for _ in 0..4096 {
            // 4096 * 256 KiB = 1 GiB: far beyond any loopback buffer.
            if t.send(&chunk).await.is_err() {
                return true;
            }
        }
        false
    })
    .await
    .expect("send must fail at the write bound, not block forever against a peer that stopped reading");
    assert!(failed, "a peer that stopped reading must eventually fail a send");
}
```

- [ ] **Step 2: Add the knob (`RtmpTransportConfig::write_timeout: Option<Duration>`, mirroring `connect_timeout`; unused) so the file compiles, then run PRE-FIX.** **Observed against main's transport:** `control_the_captured_tc_url_is_readable_for_an_ipv4_target` and `an_ipv6_push_target_sends_a_bracketed_tc_url` PASS (characterisation, as above); `a_stalled_peer_fails_send_at_the_write_bound` FAILS at the 5 s guard (`send must fail at the write bound...: Elapsed(())`: main's `send` is a bare `stream.write_all`). Record.

- [ ] **Step 3: Migrate.** `RtmpTransport::connect` becomes `RtmpTarget::from_parts(<scheme://host:port of url>, &config.app, &config.stream_key)` (or `RtmpTarget::parse(url)` when the URL carries app/key) -> `AsyncRtmpClient::connect(&target, timeouts)` -> `publish()`, with `timeouts = RtmpTimeouts::default()` overridden by `config.connect_timeout` (-> `with_connect` and `with_handshake`) and `config.write_timeout` (-> `with_write`); `send`/`write_message`/`send_media`/`setup` call `client.send_video/send_audio/send_metadata` (each bounded by `RtmpTimeouts::write` inside the adapter), keeping the FLV helpers (`flv_sequence_header_payloads`/`flv_frame_payloads`) that feed them; the hand-rolled C0/C1/C2 + `ClientConfig`/`ClientSession` block and the `RTMP_CONNECT_TIMEOUT` wrapper are deleted (the existing in-module W17 connect-bound test must still pass through `RtmpTimeouts`).

- [ ] **Step 4: PASS** (`push_rtmp_bounds`, `push_rtmp` + the mediamtx oracle, the W17 connect-bound test).

- [ ] **Step 5: Revert-checks** (run each): (a) ignore `config.write_timeout` (use `RtmpTimeouts::default()`'s 10 s) — `a_stalled_peer_fails_send_at_the_write_bound` FAILS at the 5 s guard; (b) rebuild the tcUrl by hand without brackets (`format!("rtmp://{}:{port}/{app}", url.host_str()...strip_brackets)`) — `an_ipv6_push_target_sends_a_bracketed_tc_url` FAILS with the decoded `rtmp://::1:PORT/live`. Restore each. Record that (b) is a mutation of the NEW code, since main itself already passes it.

```bash
git add multimux && git commit -m "refactor(multimux)!: RTMP push on rtmp-runtime's AsyncRtmpClient and RtmpTarget; every push write bounded"
```

---

### Task 7: `parking_lot`; delete `lock.rs`'s poison wrappers (SP6.6)

**Files:** delete `multimux/src/lock.rs`; migrate the 47 `crate::lock::` call sites (`route.rs` 22, `origin/admin.rs` 12, `output/smooth.rs` 4, `catchup.rs` 3, `source/srt.rs` 4 — counts verified by grep; the router_slot sites were already moved to `ArcSwap` by W2a Task 3). New test `multimux/tests/lock_recovery.rs`.

- [ ] **Step 1: Write the failing test.** This is a CHARACTERISATION test of parking_lot itself (it cannot fail on any multimux behaviour — the review said so and the plan agrees); the REAL witness for this task is "lock.rs deleted + the workspace builds + the full multimux suite passes" (Step 2/3), and the compile failure pre-dependency is the pre-fix observable:

```rust
//! SP6.6: parking_lot has no poisoning; a panicking holder's guard drops
//! and the next acquire succeeds. Pre-fix this test cannot even compile as
//! written (the lock types are std's), and the equivalent std behaviour
//! needed the lock.rs recovery wrapper.

use std::sync::Arc;

#[test]
fn a_panicking_lock_holder_does_not_wedge_the_next_acquire() {
    let shared: Arc<parking_lot::Mutex<Vec<u32>>> = Arc::new(parking_lot::Mutex::new(vec![1]));
    let clone = Arc::clone(&shared);
    let _ = std::thread::spawn(move || {
        let _g = clone.lock();
        panic!("holder dies mid-hold");
    })
    .join(); // the panic is expected

    // The next acquire succeeds and sees the pre-panic state.
    let got = shared.lock();
    assert_eq!(*got, vec![1], "the lock must be usable after a panicking holder");
}
```

- [ ] **Step 2: Run pre-fix — FAIL** (multimux does not depend on `parking_lot`; compile failure — recorded as such). Then migrate: replace `std::sync::Mutex/RwLock` with `parking_lot::{Mutex, RwLock}` at every `crate::lock::` call site — `crate::lock::lock(m)` → `m.lock()` (infallible), `crate::lock::read/write(rw)` → `rw.read()/rw.write()`, `lock_or_recovered` → `.lock()` (drop the recovered flag; the ONE caller at `route.rs:657` currently logs on poison — that branch disappears, which is the intended simplification and is CHANGELOG'd). Delete `lock.rs` and its `mod` line.

- [ ] **Step 3: Full multimux suite + gate steps** (the 47 call sites are compile-checked, and `route.rs`'s DVR tests exercise the lock paths).

- [ ] **Step 4: Commit**

```bash
git add multimux && git commit -m "refactor(multimux): parking_lot locks; delete the poison-recovery wrappers (SP6.6)"
```

---

### Task 8: The §5 guard — clear the W2b-2 allowlist entries (last code task)

**Files:** `multimux/tests/no_handroll_guard.rs` (the file W2a Task 9 added; W2b-1 Task 10 tightened).

- [ ] **Step 1: Remove the allowlist entries Tasks 1–6 clear** (the supervisor/push `tokio::time::sleep(` entries — Task 1's schedule feeds the sleeps but the *application* sites remain; allowlist ONLY the legitimate application sites with reasons: `supervisor.rs`'s backoff-application sleep raced against cancel, `file_reader.rs`'s pacer `sleep_until`, `push/mod.rs`'s no-slot park, and `source/pull.rs`'s three sleeps (the session's own `WaitMs` hint, the backon retry delay, the bounded idle park — Task 2's `hls_pull_has_no_inline_sleep_outside_the_scheduler` is what keeps every OTHER pull-source wait out of the sources). The idle park is the one remaining fixed-interval wait in the scheduler; record it in `.delegate/w2b-report.md` rather than hiding it. `civil_from_days`/`days_from_civil` (cleared by W2b-1 Task 3) and the URL needles (W2b-1 Task 2) were already removed by W2b-1 Task 10.

- [ ] **Step 2: Run — zero un-allowlisted `src/` hits; bite-check** (plant `let _ = "a=ice-ufrag:";` in a non-test fn → FAIL; remove).

- [ ] **Step 3: Commit**

```bash
git add multimux && git commit -m "test(multimux): clear the no-hand-roll allowlist after the W2b-2 sites"
```

---

### Task 9: CHANGELOG, version notes, full gate, hand-off — **do not merge**

- [ ] **Step 1: CHANGELOG** — `multimux` under `## [Unreleased]`: `### Changed (breaking)` (the ingest scaffold signature, the RTSP/RTMP push adapters, `ReconnectSchedule`); `### Fixed` (defect 4's RTSP write bound, defect 5's WaitMs starvation); `### Changed` (socket2 options, parking_lot, the queue→shed semantics already declared in W2a). `rtsp-runtime`/`rtmp-runtime`: the three/four additive methods, breaking-class additive, recorded in `.delegate/release-versions.txt`.

- [ ] **Step 2: Full gate** — `/Volumes/External/Projects/rust-broadcast/.delegate/gate-wt.sh "$PWD"` (expect 14 rc=0 steps).

- [ ] **Step 3: `.delegate/w2b-report.md`** — baseline/after counts, per-source loop shrink measurements (Task 4's list), all revert-check evidence, version notes.

- [ ] **Step 4: Hand off — do not merge, push, or tag.**

---

## Coverage table (W2b-2)

| Spec site / item | Where | Task |
|---|---|---|
| §3 Runtime: four backoffs; three pull loops + 5 ms idle poll; six read loops; RTMP/RTSP push loops | | 1, 2, 4, 5, 6 |
| §3 defect 4 (multimux push/rtsp.rs; RTSP source write) | `push/rtsp.rs:213`, `source/rtsp.rs:876` | 4, 5 |
| §3 defect 5 (`hls_pull` WaitMs blocks join servicing) | `hls_pull.rs:510-519` | 2 |
| §3 defect 8 (IPv6 tcUrl via RtmpTarget) | `push/rtmp.rs` | 6 (target: W2b-1 Task 2) |
| §4 SP1.5 (backon + jitter, explicit attempt bound) | | 1 |
| §4 SP1.6 (socket2, configurable SO_RCVBUF/REUSEADDR/interface) | | 3 |
| §4 SP4.1 (push/rtsp.rs ANNOUNCE via sdp-types) | | 5 |
| §4 SP6.1 (runtime-crate adapters; one generic scaffold, an `IngestStep` per source) | | 4, 5, 6 |
| §4 SP6.2 (one pull scheduler; `hls_pull.rs:7-20` resolution) | | 2 |
| §4 SP6.6 (parking_lot; delete lock.rs) | | 7 |
| §5 guard (W2b-2 clear) | | 8 |
| rtsp-runtime `send_interleaved` (+ announce/record, W2b-1 Task 6) | | 5 (consumes W2b-1 Task 6) |

## Escalations

0. **Defect 8 on the RTMP push path does not reproduce on main.** `Url::host_str()` keeps IPv6 brackets, so `format!("rtmp://{host}:{port}/{app}")` already yields `rtmp://[::1]:PORT/live` (observed: Task 6's IPv6 test passes against main's transport). The W2b-1 Task 2 row that says old code emits an unbracketed IPv6 tcUrl for the push is therefore not a bite for that path; Task 6 labels its test as characterisation. W2b-1 is not edited here.

1. **The one generic scaffold keeps a per-source `IngestStep` (read + `feed`)** (accepted by the round-2 review *with the scaffold written out* — Task 4 writes it in full and lists the per-loop shrink). Not hidden.
2. **backon's default `max_times` is 3** (verified exponential.rs:62): every schedule sets `.without_max_times()` and applies the caller's own bound; the wrapper re-clamps after jitter (jitter is added after backon's internal clamp — verified `tmp_cur.saturating_add(tmp_cur.mul_f32(rng))`).
3. **`PullScheduler`'s `TaskPanic` variant** replaces the key-carrying panic arm (a panicked fetch task loses its key to the JoinSet error); the source treats it as session-terminal exactly as hls_pull's current `Some(Err(join_err))` arm does.
4. **`Trunk::waiter_slot_freed()` (W2b-1 Task 9's additive media-plane API)** keeps its declared fallback (bounded `sleep_until` raced against cancel) if adding it to media-plane is out of bounds.
