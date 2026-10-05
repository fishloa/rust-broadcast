//! The shared fetch/retry/wait engine for the three pull sources
//! (HLS/DASH/Smooth — SP6.2). Each source keeps its own `feed` translation;
//! the fan-out, the retry policy, the `WaitMs` handling and the idle park
//! live here once.
//!
//! # Why a scheduler and not `buffer_unordered`
//!
//! The spec names `buffer_unordered(MAX_INFLIGHT_FETCHES)` + `backon`. The
//! fetches have heterogeneous keys (`HlsFetchId`, a fragment URL, a Smooth
//! fragment) and a retry that must re-enter the SAME in-flight bound rather
//! than bypass it, so the fan-out is a `JoinSet` behind a small explicit
//! scheduler: `push` (the backlog), `push_retry` (due retries, which take a
//! free slot before the backlog), `pump` (fill free slots) and `next` (join
//! one, bounded by the session's own `WaitMs` hint). It is the same
//! structure `hls_pull` grew by hand, extracted — the point of the wave.
//!
//! # The `WaitMs` hint must not starve a ready fetch (defect 5)
//!
//! The pre-migration loop slept the session's `Action::WaitMs` hint inline
//! *before* joining an in-flight fetch, so a fetch that completed 50 ms into
//! a 1 s hint was not serviced for another 950 ms. [`PullScheduler::next`]
//! instead races `join_next()` against the hint, so a fetch completing during
//! the hint is returned as soon as it completes.

use std::collections::VecDeque;
use std::fmt;
use std::future::Future;
use std::pin::Pin;
use std::time::Duration;

use tokio::task::JoinSet;

use crate::error::MultimuxError;

/// The outcome of one spawned fetch.
pub(crate) enum FetchOutcome<K> {
    /// The fetch resolved: `(key, bytes)`.
    Ready(K, Vec<u8>),
    /// The fetch failed: `(key, error)` — the source decides retry vs terminal.
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

/// One fetch to spawn. The key travels inside `fut`'s result, so it is not
/// repeated here.
pub(crate) struct PendingFetch<K> {
    pub delay: Duration,
    pub fut: FetchFuture<K>,
}

/// The single fetch/retry/wait engine for the pull sources. This module was
/// compile-verified with its tests in a scratch crate against main's
/// `MultimuxError` and tokio 1.53.
pub(crate) struct PullScheduler<K: Send + 'static> {
    inflight: JoinSet<(K, Result<Vec<u8>, MultimuxError>)>,
    backlog: VecDeque<PendingFetch<K>>,
    retries: VecDeque<PendingFetch<K>>,
    max_inflight: usize,
}

impl<K: Send + 'static> PullScheduler<K> {
    /// A scheduler that keeps at most `max_inflight` fetches running.
    pub fn new(max_inflight: usize) -> Self {
        Self {
            inflight: JoinSet::new(),
            backlog: VecDeque::new(),
            retries: VecDeque::new(),
            max_inflight,
        }
    }

    /// Queue a fetch behind everything already backlogged.
    pub fn push(&mut self, fetch: PendingFetch<K>) {
        self.backlog.push_back(fetch);
    }

    /// Queue a due retry — a retry takes a free slot before the backlog, so
    /// an eviction race is repaired before new work is started.
    pub fn push_retry(&mut self, fetch: PendingFetch<K>) {
        self.retries.push_back(fetch);
    }

    /// How many fetches are running (used by the cap test).
    #[cfg(test)]
    pub fn inflight_len(&self) -> usize {
        self.inflight.len()
    }

    /// Whether there is nothing running, queued or awaiting a retry.
    pub fn is_idle(&self) -> bool {
        self.inflight.is_empty() && self.backlog.is_empty() && self.retries.is_empty()
    }

    /// Fill free slots, retries first, applying each fetch's own delay.
    pub fn pump(&mut self) {
        while self.inflight.len() < self.max_inflight {
            let Some(PendingFetch { delay, fut }) = self
                .retries
                .pop_front()
                .or_else(|| self.backlog.pop_front())
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

    /// Join one fetch, bounded by `wait_hint` when the session gave one
    /// (`Action::WaitMs`). A fetch that completes while the hint is
    /// outstanding is returned as soon as it completes — the hint bounds how
    /// long we wait, it does not delay a ready result (defect 5).
    ///
    /// Returns `None` when nothing is in flight (the hint or `idle_park` is
    /// then slept alone) or when the hint elapsed first.
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

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::time::Instant;

    fn fetch_after(key: u32, after: Duration, bytes: &'static [u8]) -> PendingFetch<u32> {
        PendingFetch {
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
    /// 100 ms (`a_pre_fix_inline_hint_sleep_would_take_the_whole_hint` below
    /// measures that shape).
    #[tokio::test(start_paused = true)]
    async fn a_fetch_completing_during_a_waitms_hint_is_returned_when_it_completes() {
        let t0 = Instant::now();
        let mut s = PullScheduler::<u32>::new(8);
        s.push(fetch_after(1, Duration::from_millis(100), b"seg"));
        s.pump();
        let out = s
            .next(Some(Duration::from_millis(1000)), Duration::from_millis(5))
            .await;
        assert!(
            matches!(&out, Some(FetchOutcome::Ready(1, b)) if b == b"seg"),
            "{out:?}"
        );
        assert_eq!(
            t0.elapsed(),
            Duration::from_millis(100),
            "serviced when ready, not after the hint"
        );
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
        assert!(
            s.next(Some(Duration::from_millis(300)), Duration::from_millis(5))
                .await
                .is_none()
        );
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
        assert!(
            matches!(first, Some(FetchOutcome::Ready(2, _))),
            "{first:?}"
        );
        s.pump();
        let second = s.next(None, Duration::from_millis(5)).await;
        assert!(
            matches!(second, Some(FetchOutcome::Ready(1, _))),
            "{second:?}"
        );
        assert!(s.is_idle());
    }

    /// A panicking fetch task is reported, not swallowed.
    #[tokio::test(start_paused = true)]
    async fn a_task_panic_is_reported_not_swallowed() {
        let mut s = PullScheduler::<u32>::new(8);
        s.push(PendingFetch {
            delay: Duration::ZERO,
            fut: Box::pin(async { boom() }),
        });
        s.pump();
        let out = s.next(None, Duration::from_millis(5)).await;
        assert!(matches!(out, Some(FetchOutcome::TaskPanic(_))), "{out:?}");
    }

    /// The in-flight cap actually caps: `pump` fills to `max_inflight` and
    /// never beyond, however much is queued.
    #[tokio::test(start_paused = true)]
    async fn pump_never_exceeds_the_in_flight_cap() {
        let mut s = PullScheduler::<u32>::new(2);
        for k in 0..10 {
            s.push(fetch_after(k, Duration::from_millis(50), b"x"));
        }
        s.pump();
        assert_eq!(s.inflight_len(), 2, "pump must stop at the cap");
    }

    /// A retry's own delay is applied before its fetch runs.
    #[tokio::test(start_paused = true)]
    async fn a_retry_delay_is_applied_before_the_fetch_runs() {
        let t0 = Instant::now();
        let mut s = PullScheduler::<u32>::new(1);
        s.push_retry(PendingFetch {
            delay: Duration::from_millis(250),
            fut: Box::pin(async { (7u32, Ok(b"late".to_vec())) }),
        });
        s.pump();
        let out = s.next(None, Duration::from_millis(5)).await;
        assert!(matches!(out, Some(FetchOutcome::Ready(7, _))), "{out:?}");
        assert_eq!(t0.elapsed(), Duration::from_millis(250));
    }
}
