//! The shared drive loop for the one-connection dial sources (SP6.1). Each
//! source keeps its own bounded read + `driver.feed` (an [`IngestStep`]); the
//! deadline / cancel / bounded-write / stall / advance / terminal-tail
//! scaffolding lives here once.
//!
//! # Why a trait, not a closure
//!
//! [`IngestSession::In`] is a lifetime-bound associated type, so this scaffold
//! cannot name the input; each source therefore supplies its own *read +
//! `driver.feed`* through [`IngestStep`]. A closure cannot do it: a returned
//! future cannot borrow closure-captured state (the lending-closure problem),
//! and `async move` would move the read half out of an `FnMut`. A small trait
//! whose `step(&mut self, ..)` owns the source's state in a struct, returning
//! `impl Future<..> + Send` (so each source writes a plain `async fn`), keeps
//! the read half and the driver borrow separate, so nothing is ever spawned
//! with a local borrow — the scaffold is awaited inline by the supervisor
//! attempt.

use std::future::Future;
use std::sync::Arc;
use std::time::Duration;

use broadcast_common::Timestamp;
use media_plane::ingress::{HealthState, IngestDriver, IngestSession};
use tokio::io::{AsyncWrite, AsyncWriteExt};
use tokio::time::Instant;
use tokio_util::sync::CancellationToken;

use crate::error::MultimuxError;
use crate::route::RouteHandle;
use crate::source::{DriverProgress, IngestTimeouts, advance_route, release_route};

/// What one source [`IngestStep::step`] observed.
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

/// The per-source half of the scaffold: one bounded read plus the source's
/// own `driver.feed`.
pub(crate) trait IngestStep<S: IngestSession>: Send {
    /// ONE bounded read (at most `window`) plus the source's own
    /// `driver.feed(..)`.
    fn step(
        &mut self,
        driver: &mut IngestDriver<S>,
        window: Duration,
    ) -> impl Future<Output = StepOutcome> + Send;

    /// The error for "no bytes for `read`". A source whose historical error
    /// text differs overrides this.
    fn stalled(&self, read: Duration) -> MultimuxError {
        MultimuxError::Protocol {
            phase: "recv",
            reason: format!("no data within {read:?}"),
        }
    }
}

/// A [`Timestamp`] on the scaffold's clock origin.
pub(crate) fn ts(start: Instant, now: Instant) -> Timestamp {
    Timestamp::from_instant(start.into_std(), now.into_std())
}

/// Default bound on one outbound write (defect 4: the pre-migration
/// `run_rtsp`'s `wr.write_all` had none). Not a field on [`IngestTimeouts`] —
/// that struct is built by literal in public code and tests, so adding a
/// field would break them; the scaffold takes it as an argument.
pub(crate) const DEFAULT_WRITE_TIMEOUT: Duration = Duration::from_secs(10);

/// The drive loop. Takes the driver BY VALUE (the terminal tail consumes it
/// with `into_health`, exactly as `run_rtsp` does).
///
/// Order per iteration, preserving the pre-migration `run_rtsp`:
/// cancel check -> session deadline (`on_deadline`) -> drain `poll_transmit`
/// under `write_timeout` -> health check -> **stall check** -> one bounded
/// `source.step` raced against `cancel` -> `advance_route`.
///
/// **Stall semantics (issue #1083 item 1, preserved):** the stall clock
/// `last_rx` is reset ONLY by [`StepOutcome::Received`]. [`StepOutcome::Idle`]
/// (the read window elapsed with nothing received — including a window
/// shortened to wake for a keepalive) never resets it, so a peer that holds
/// the socket open and answers keepalives but sends nothing fails the session
/// `read` after the last byte, exactly as before.
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
            return MultimuxError::Connect {
                reason: "cancelled".into(),
            };
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
                    return MultimuxError::Connect {
                        reason: format!("write: {e}"),
                    };
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
            return MultimuxError::Connect {
                reason: "cancelled".into(),
            };
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
            Self {
                deadline: None,
                every: 0,
                tx: None,
                tx_per_deadline: Bytes::new(),
            }
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
            Self {
                deadline: None,
                every: 0,
                tx: Some(tx),
                tx_per_deadline: Bytes::new(),
            }
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
            if delay > window {
                StepOutcome::Idle
            } else {
                outcome()
            }
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
        IngestTimeouts {
            connect: Duration::from_secs(5),
            read,
        }
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
        assert_eq!(
            t0.elapsed(),
            Duration::from_secs(35),
            "fails exactly at the stall window"
        );
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
        assert!(
            matches!(&err, MultimuxError::Protocol { phase: "recv", .. }),
            "{err:?}"
        );
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
        assert!(
            matches!(&err, MultimuxError::Connect { reason } if reason == "cancelled"),
            "{err:?}"
        );
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
