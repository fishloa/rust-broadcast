//! Per-route ingest supervisor: drive a route's ingest attempt (dial/listen,
//! establish, ingest) and — on a connect/handshake failure, an ingest error,
//! or the source cleanly ending before ever reconnecting — retry with capped
//! exponential backoff, forever, until shutdown fires.
//!
//! Before this module, `origin::serve` spawned a one-shot per-route task:
//! connect once, ingest once, and on any failure just `eprintln!` and let the
//! task die for good — after which the HTTP origin kept serving the frozen
//! last playlist as `200 OK` forever. [`supervise_driver`] replaces that
//! one-shot task with a loop, and keeps [`RouteHandle::health`] in sync so a
//! client/output can (eventually) see that a route stopped producing new
//! media rather than silently going stale.
//!
//! [`supervise_driver`] is `#[tracing::instrument]`ed with the route name as a
//! `tracing` span field, so every event it emits (attempt success/failure,
//! reconnect, backoff) is attributed to its route without repeating the name
//! in every message. Never logs the source URL/credentials — every
//! `crate::source::*::run_*` entry point it wraps already keeps those out of
//! its own error/log messages.
//!
//! # History: the deleted `supervise`/`SourceConnector` pair
//!
//! Every input kind once drove a hand-rolled `connect()` + pull loop behind an
//! associated-type `SourceConnector` trait, reconnected by a `supervise`
//! function that this module also defined. Issue #805 ported all nine input
//! kinds (rtsp/rtp/ts_udp/ts_http/srt/hls_pull/dash_pull/smooth_pull/rtmp) onto
//! `media_plane::ingress`'s `Dialer`/`Listener` + `IngestSession` traits
//! instead — task 5 deleted `SourceConnector`/`supervise`/`crate::pipeline`
//! outright once RTMP (task 4, the last holdout) left them with no remaining
//! caller. [`supervise_driver`] is the one surviving supervisor loop; a
//! [`crate::registry::SchemeRegistry`]-registered `Custom` input factory
//! drives its own `media_plane::ingress::Dialer`/`IngestSession` through it
//! exactly the same way every built-in `run_*` entry point does — see
//! `examples/custom_scheme.rs`.

use std::future::Future;
use std::sync::Arc;
use std::time::Duration;

use tokio::sync::watch;

use crate::route::{HealthState, RouteHandle};

/// Production default backoff: starts at 500 ms, doubles, caps at 30 s.
const DEFAULT_BACKOFF_MIN: Duration = Duration::from_millis(500);
const DEFAULT_BACKOFF_MAX: Duration = Duration::from_secs(30);
const DEFAULT_BACKOFF_FACTOR: f64 = 2.0;

/// Capped exponential backoff: [`Backoff::next`] returns the current delay
/// then grows it by `factor` (capped at `max`); [`Backoff::reset`] restores
/// it to `min` after a successful (re)connect so a long outage doesn't
/// permanently slow down subsequent quick recoveries.
#[derive(Debug, Clone)]
pub struct Backoff {
    min: Duration,
    max: Duration,
    factor: f64,
    current: Duration,
}

impl Backoff {
    /// A backoff starting at `min`, doubling (or whatever `factor` is) on
    /// every [`next`](Backoff::next) call, never exceeding `max`.
    pub fn new(min: Duration, max: Duration, factor: f64) -> Self {
        Backoff {
            min,
            max,
            factor,
            current: min,
        }
    }

    /// Reasonable production defaults: 500 ms min, 30 s max, factor 2.0.
    pub fn production_default() -> Self {
        Backoff::new(
            DEFAULT_BACKOFF_MIN,
            DEFAULT_BACKOFF_MAX,
            DEFAULT_BACKOFF_FACTOR,
        )
    }

    /// Returns the delay to wait for *this* attempt, then grows the
    /// internal delay (capped at `max`) for the next call.
    ///
    /// `#[allow(clippy::should_implement_trait)]`: this is deliberately
    /// named `next` (not `next_delay` or similar) to read naturally at the
    /// supervisor's call site (`backoff.next()`); `Backoff` is not an
    /// `Iterator` (it never ends) so there's no real risk of confusion in
    /// practice.
    #[allow(clippy::should_implement_trait)]
    pub fn next(&mut self) -> Duration {
        let delay = self.current;
        let grown = self.current.mul_f64(self.factor);
        self.current = grown.min(self.max);
        delay
    }

    /// Resets the delay back to `min` — call after a successful connect so
    /// the *next* outage starts backing off from the bottom again.
    pub fn reset(&mut self) {
        self.current = self.min;
    }
}

/// Mirrors `state` into the [`crate::prometheus::ROUTE_UP`] gauge for `name`:
/// 1.0 while `state` is [`HealthState::Live`], 0.0 otherwise. Called
/// alongside every `route_handle.set_health(..)` in [`supervise_driver`] —
/// this is the one place that has both the route's name (for the label) and
/// every health transition, since [`RouteHandle`] itself doesn't carry its
/// own route name.
fn record_route_up(name: &str, state: HealthState) {
    let up = if matches!(state, HealthState::Live) {
        1.0
    } else {
        0.0
    };
    metrics::gauge!(crate::prometheus::ROUTE_UP, "route" => name.to_string()).set(up);
}

/// Bumps [`crate::prometheus::SOURCE_RECONNECTS_TOTAL`] for `name`: called
/// every time [`supervise_driver`]'s loop is about to retry after an attempt
/// ended (whether it ever reached [`HealthState::Live`] or failed outright) —
/// i.e. every transition into [`HealthState::Reconnecting`].
fn record_reconnect(name: &str) {
    metrics::counter!(crate::prometheus::SOURCE_RECONNECTS_TOTAL, "route" => name.to_string())
        .increment(1);
}

/// Drives one route's ingest through a `media_plane`
/// `Dialer`/`Listener`/`IngestSession`/`IngestDriver` — the one supervisor
/// loop every input kind now uses (issue #805 tasks 2/4/5), including a
/// [`crate::registry::SchemeRegistry`]-registered `Custom` input factory
/// driving its own `Dialer`/`IngestSession` (see `examples/custom_scheme.rs`).
///
/// `attempt` is one full dial-through-disconnect cycle for this route's input
/// kind — a caller-supplied closure that closes over its own `*Route` config,
/// `media_plane::trunk::TrunkConfig`, and `media_plane::ingress::HandshakePolicy`,
/// and calls the matching `crate::source::*::run_*` entry point (or, for a
/// `Custom` factory's own driver loop, the equivalent hand-written attempt —
/// see [`crate::source::report_driver_progress`]/
/// [`crate::source::segment::drive_program_segmenters`]). Every in-tree
/// entry point already calls `report_driver_progress` from inside its own
/// drive loop: that is what flips `route_handle` to [`HealthState::Live`] the
/// moment the driver's session establishes, and publishes each
/// newly-announced program's `Trunk` into `route_handle`'s registry
/// (`RouteHandle::publish_program`) — the ingest-side half of issue #805's
/// registry reconciliation.
///
/// [`Backoff`] runs between attempts, reset only once an attempt actually
/// reached [`HealthState::Live`]; `record_route_up`/`record_reconnect` fire on
/// every transition; a shutdown [`watch::Receiver<bool>`] is checked before
/// each attempt and around the backoff sleep so it cancels promptly.
///
/// # Why this reads `route_handle.health()` back, rather than being told
///
/// A driver-backed `run_*`/attempt fuses dial and drive into one call — there
/// is no externally-observable midpoint for this loop to hook `Live` onto
/// other than `route_handle`'s own health, which `attempt`'s inner
/// `report_driver_progress` call already sets. So this loop resets
/// `route_handle` to [`HealthState::Connecting`] before every attempt (so a
/// stale `Live` left over from a *previous* attempt can never be mistaken for
/// this one succeeding), then reads `route_handle.health()` back once
/// `attempt` returns to decide whether this attempt reached `Live` at all.
#[tracing::instrument(
    name = "route",
    skip(attempt, route_handle, backoff, name, shutdown),
    fields(route = %name)
)]
pub async fn supervise_driver<F, Fut>(
    mut attempt: F,
    route_handle: Arc<RouteHandle>,
    mut backoff: Backoff,
    name: String,
    mut shutdown: watch::Receiver<bool>,
) where
    F: FnMut(Arc<RouteHandle>) -> Fut + Send + 'static,
    Fut: Future<Output = crate::Result<()>> + Send,
{
    tracing::info!("connecting");
    route_handle.set_health(HealthState::Connecting);
    record_route_up(&name, HealthState::Connecting);
    let mut attempt_no: u64 = 0;

    loop {
        if *shutdown.borrow() {
            break;
        }
        route_handle.set_health(HealthState::Connecting);

        let result = attempt(route_handle.clone()).await;
        let reached_live = route_handle.health() == HealthState::Live;

        match result {
            Ok(()) if reached_live => tracing::info!("ingest ended after being live"),
            Ok(()) => {
                attempt_no += 1;
                tracing::warn!(attempt = attempt_no, "ended before ever becoming live");
            }
            Err(e) if reached_live => tracing::warn!(error = %e, "pipeline stopped"),
            Err(e) => {
                attempt_no += 1;
                tracing::warn!(error = %e, attempt = attempt_no, "failed to connect");
            }
        }

        if reached_live {
            attempt_no = 0;
            backoff.reset();
        }
        route_handle.set_health(HealthState::Reconnecting);
        record_route_up(&name, HealthState::Reconnecting);
        record_reconnect(&name);

        if *shutdown.borrow() {
            break;
        }

        let delay = backoff.next();
        tracing::warn!(
            delay_ms = delay.as_millis() as u64,
            attempt = attempt_no,
            "reconnecting after backoff"
        );
        tokio::select! {
            () = tokio::time::sleep(delay) => {}
            _ = shutdown.changed() => {
                break;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    /// Tiny backoff for tests: keeps the whole suite fast regardless of how
    /// many reconnect cycles a test drives through.
    fn tiny_backoff() -> Backoff {
        Backoff::new(Duration::from_millis(1), Duration::from_millis(20), 2.0)
    }

    /// Polls `f` every millisecond until it returns `true` or `timeout`
    /// elapses, returning whether it succeeded — used instead of a fixed
    /// sleep so tests are both fast and not flaky under load.
    async fn wait_until(timeout: Duration, mut f: impl FnMut() -> bool) -> bool {
        let deadline = tokio::time::Instant::now() + timeout;
        loop {
            if f() {
                return true;
            }
            if tokio::time::Instant::now() >= deadline {
                return false;
            }
            tokio::time::sleep(Duration::from_millis(1)).await;
        }
    }

    #[test]
    fn backoff_grows_and_caps() {
        let mut b = Backoff::new(Duration::from_millis(10), Duration::from_millis(100), 2.0);
        assert_eq!(b.next(), Duration::from_millis(10));
        assert_eq!(b.next(), Duration::from_millis(20));
        assert_eq!(b.next(), Duration::from_millis(40));
        assert_eq!(b.next(), Duration::from_millis(80));
        // Would grow to 160ms, but caps at 100ms.
        assert_eq!(b.next(), Duration::from_millis(100));
        assert_eq!(b.next(), Duration::from_millis(100), "stays capped");
    }

    #[test]
    fn backoff_reset_returns_to_min() {
        let mut b = Backoff::new(Duration::from_millis(10), Duration::from_millis(100), 2.0);
        let _ = b.next();
        let _ = b.next();
        b.reset();
        assert_eq!(
            b.next(),
            Duration::from_millis(10),
            "back to min after reset"
        );
    }

    /// A fake `attempt` closure: fails (`Err`, never touching health) the
    /// first `fail_times` calls, then on every call after that sets
    /// `route_handle` to `HealthState::Live`, holds it there for a real (if
    /// brief) wall-clock span (mirroring every real driver-backed `run_*`,
    /// which stays `Live` for the life of its connection rather than for a
    /// single synchronous instant — without a genuine `.await` point in
    /// between, an external poller could never observe `Live` before
    /// `supervise_driver`'s own loop immediately overwrites it with
    /// `Reconnecting` right after `attempt` returns), then ends cleanly
    /// (`Ok(())`) — standing in for a real ingest session that establishes,
    /// serves for a while, then disconnects.
    fn flaky_attempt(
        fail_times: usize,
        call_count: Arc<AtomicUsize>,
    ) -> impl FnMut(
        Arc<RouteHandle>,
    ) -> std::pin::Pin<Box<dyn Future<Output = crate::Result<()>> + Send>>
    + Send
    + 'static {
        move |route_handle: Arc<RouteHandle>| {
            let call_count = call_count.clone();
            Box::pin(async move {
                let n = call_count.fetch_add(1, Ordering::SeqCst);
                if n < fail_times {
                    return Err(crate::MultimuxError::Connect {
                        reason: "flaky attempt: simulated failure".into(),
                    });
                }
                route_handle.set_health(HealthState::Live);
                tokio::time::sleep(Duration::from_millis(20)).await;
                Ok(())
            })
        }
    }

    /// Biting test: an attempt that fails once, then reaches `Live`, then
    /// cleanly ends, must still bring (and keep bringing) the route up —
    /// health reaches `Live` after the failure, and the loop calls `attempt`
    /// again after the live attempt ends (proving both retry paths
    /// `supervise_driver`'s `match result { .. }` arms cover: "failed before
    /// ever live" and "ended after being live"). Reverting the loop to a
    /// one-shot (no retry at all) breaks both: the route would stay dead
    /// after the first failure, and would never be attempted a third time
    /// after the live attempt ends.
    ///
    /// MUTATION VERIFIED: changing `supervise_driver`'s loop so it `break`s
    /// immediately after any attempt that reached `Live` ends (instead of
    /// falling through to the backoff+retry at the bottom of the loop) makes
    /// this test fail: `assert!(call_count.load(Ordering::SeqCst) >= 3, ...)`
    /// fails, comparing the actual call count (2 — the failing attempt plus
    /// the one live attempt) against the required minimum of 3, because the
    /// loop never attempts again once an attempt has been live. Recompiled
    /// and re-run to confirm that exact assertion failure, then reverted.
    #[tokio::test]
    async fn reconnects_after_a_failing_attempt_reaches_live_and_retries_again_after_ending() {
        let route = Arc::new(RouteHandle::new(1.0, 500, 8));
        let call_count = Arc::new(AtomicUsize::new(0));
        let attempt = flaky_attempt(1, call_count.clone());
        let (shutdown_tx, shutdown_rx) = watch::channel(false);

        let handle = tokio::spawn(supervise_driver(
            attempt,
            route.clone(),
            tiny_backoff(),
            "test-route".to_string(),
            shutdown_rx,
        ));

        let reached_live = wait_until(Duration::from_secs(2), || {
            route.health() == HealthState::Live
        })
        .await;
        assert!(
            reached_live,
            "route must reach Live after one failing attempt"
        );

        // The live attempt (a 20ms sleep, above) ends and the loop must
        // retry a third time — proving the "ended after being live" arm
        // retries exactly like the "failed before ever live" arm already
        // proven above, not just once each.
        let retried_again = wait_until(Duration::from_secs(2), || {
            call_count.load(Ordering::SeqCst) >= 3
        })
        .await;
        assert!(
            retried_again,
            "attempt must be called again after a live attempt ends"
        );

        shutdown_tx.send(true).unwrap();
        tokio::time::timeout(Duration::from_secs(1), handle)
            .await
            .expect("supervise_driver returns promptly on shutdown")
            .expect("supervise_driver task did not panic");
    }

    /// Biting test: firing shutdown must stop the loop promptly even
    /// mid-backoff, well under the (deliberately large, relative to the
    /// test) backoff cap — proving the sleep is cancellable, not a plain
    /// `tokio::time::sleep` the loop blindly awaits to completion.
    ///
    /// MUTATION VERIFIED: replacing `supervise_driver`'s cancellable
    /// `tokio::select! { () = tokio::time::sleep(delay) => {}, _ =
    /// shutdown.changed() => { break; } }` with a plain, un-cancellable
    /// `tokio::time::sleep(delay).await` makes this test fail:
    /// `.expect("supervise_driver must return promptly on shutdown, not
    /// after the 10s backoff")` panics on the `Err` from
    /// `tokio::time::timeout(Duration::from_millis(500), handle)`, i.e. a
    /// `Elapsed` timeout error, because the spawned task is still sleeping
    /// out its 10s backoff instead of observing shutdown. Recompiled and
    /// re-run to confirm that exact panic, then reverted.
    #[tokio::test]
    async fn shutdown_stops_supervise_driver_promptly_mid_backoff() {
        let route = Arc::new(RouteHandle::new(1.0, 500, 8));
        let call_count = Arc::new(AtomicUsize::new(0));
        // Always fails, so the loop is guaranteed to be sitting in the
        // backoff sleep (not mid-attempt) shortly after start.
        let attempt = flaky_attempt(usize::MAX, call_count);
        // A backoff far larger than the shutdown-stops-it assertion window
        // below: if shutdown didn't cancel the sleep, the timeout on the
        // join would fire first and this test would fail.
        let backoff = Backoff::new(Duration::from_secs(10), Duration::from_secs(30), 2.0);
        let (shutdown_tx, shutdown_rx) = watch::channel(false);

        let handle = tokio::spawn(supervise_driver(
            attempt,
            route,
            backoff,
            "test-route".to_string(),
            shutdown_rx,
        ));

        // Give the loop a moment to fail its first attempt and enter the
        // (10s) backoff sleep.
        tokio::time::sleep(Duration::from_millis(20)).await;
        shutdown_tx.send(true).unwrap();

        tokio::time::timeout(Duration::from_millis(500), handle)
            .await
            .expect("supervise_driver must return promptly on shutdown, not after the 10s backoff")
            .expect("supervise_driver task did not panic");
    }
}
