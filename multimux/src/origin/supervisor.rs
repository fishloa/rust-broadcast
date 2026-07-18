//! Per-route ingest supervisor: connect the source, drive it through
//! [`run_pipeline`], and — on connect failure, pipeline error, or source
//! end-of-stream — reconnect with capped exponential backoff, forever, until
//! shutdown fires.
//!
//! Before this module, `origin::serve` spawned a one-shot per-route task:
//! connect once, run the pipeline once, and on any failure just `eprintln!`
//! and let the task die for good — after which the HTTP origin kept serving
//! the frozen last playlist as `200 OK` forever. `supervise` replaces that
//! one-shot task with a loop, and keeps [`MediaStore::health`] in sync so a
//! client/output can (eventually) see that a route stopped producing new
//! media rather than silently going stale.
//!
//! [`supervise`] is `#[tracing::instrument]`ed with the route name as a
//! `tracing` span field, so every event it emits (connect success/failure,
//! pipeline stop, backoff) is attributed to its route without repeating the
//! name in every message. Never logs the source URL/credentials — see
//! [`supervise`]'s own doc comment.
//!
//! Reconnecting needs to re-run the "connect" step, so it's abstracted
//! behind [`SourceConnector`] rather than baked in as a one-shot
//! `Result<Session>` — this is also what makes the loop unit-testable
//! without a real RTSP server (see the mock connectors in this module's
//! tests).

use std::future::Future;
use std::sync::Arc;
use std::time::Duration;

use tokio::sync::watch;

use crate::pipeline::{SampleSource, run_pipeline};
use crate::store::{HealthState, MediaStore};

/// Production default backoff: starts at 500 ms, doubles, caps at 30 s.
const DEFAULT_BACKOFF_MIN: Duration = Duration::from_millis(500);
const DEFAULT_BACKOFF_MAX: Duration = Duration::from_secs(30);
const DEFAULT_BACKOFF_FACTOR: f64 = 2.0;

/// Abstracts a route's "connect to the source" step so [`supervise`] can
/// re-run it on every reconnect attempt, and so the loop is testable
/// without a real RTSP server.
///
/// `#[allow(async_fn_in_trait)]`-equivalent: this trait spells its method as
/// `-> impl Future<..> + Send` (RPITIT) rather than `async fn`, precisely so
/// the `Send` bound the supervisor's `tokio::spawn` needs is part of the
/// trait contract, not left implicit like [`SampleSource`] (which is
/// internal-only and always instantiated at concrete `Send` types).
pub trait SourceConnector: Send + Sync + 'static {
    /// The session type this connector yields on success — the pipeline's
    /// [`SampleSource`].
    type Source: SampleSource + Send;

    /// Attempt one connection. Called again for every reconnect.
    fn connect(&self) -> impl Future<Output = crate::Result<Self::Source>> + Send;
}

impl SourceConnector for crate::source::rtsp::RtspSource {
    type Source = crate::source::rtsp::RtspSession;

    async fn connect(&self) -> crate::Result<Self::Source> {
        crate::source::rtsp::RtspSource::connect(self).await
    }
}

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
/// alongside every `store.set_health(..)` in [`supervise`] — this is the one
/// place that has both the route's name (for the label) and every health
/// transition, since [`MediaStore`] itself doesn't carry its own route name.
fn record_route_up(name: &str, state: HealthState) {
    let up = if matches!(state, HealthState::Live) {
        1.0
    } else {
        0.0
    };
    metrics::gauge!(crate::prometheus::ROUTE_UP, "route" => name.to_string()).set(up);
}

/// Bumps [`crate::prometheus::SOURCE_RECONNECTS_TOTAL`] for `name`: called
/// every time [`supervise`]'s loop is about to retry after losing its
/// connection (a failed `connect()`, or a `run_pipeline` that returned after
/// having been live) — i.e. every transition into [`HealthState::Reconnecting`].
fn record_reconnect(name: &str) {
    metrics::counter!(crate::prometheus::SOURCE_RECONNECTS_TOTAL, "route" => name.to_string())
        .increment(1);
}

/// Runs one route's supervised ingest loop until `shutdown` fires:
///
/// ```text
/// set_health(Connecting)
/// loop {
///     match connector.connect().await {
///         Ok(source) => {
///             set_health(Live); backoff.reset();
///             run_pipeline(..).await;   // returns on EOF/err
///             set_health(Reconnecting)
///         }
///         Err(e) => { warn!(e); set_health(Reconnecting) }
///     }
///     if shutdown fired { break }
///     sleep(backoff.next()) // cancellable by shutdown
/// }
/// ```
///
/// `name` is used only in log lines (never the source URL/credentials —
/// callers must pass a connector that never surfaces those, which
/// [`crate::source::rtsp::RtspSource`] already ensures by stripping userinfo
/// before it ever reaches an error message).
///
/// This never gives up permanently: a source going away (camera reboot,
/// network blip) always retries as [`HealthState::Reconnecting`], since
/// sources like cameras come back. [`HealthState::Failed`] is reserved for
/// an unrecoverable error class future callers may want to distinguish; the
/// loop here does not currently produce it.
#[tracing::instrument(
    name = "route",
    skip(connector, store, target_duration_secs, part_target_ms, backoff, name, shutdown),
    fields(route = %name)
)]
pub async fn supervise<C: SourceConnector>(
    connector: C,
    store: Arc<MediaStore>,
    target_duration_secs: f64,
    part_target_ms: u32,
    mut backoff: Backoff,
    name: String,
    mut shutdown: watch::Receiver<bool>,
) {
    tracing::info!("connecting");
    store.set_health(HealthState::Connecting);
    record_route_up(&name, HealthState::Connecting);
    let mut attempt: u64 = 0;

    loop {
        if *shutdown.borrow() {
            break;
        }

        match connector.connect().await {
            Ok(source) => {
                attempt = 0;
                tracing::info!("connected, ingest live");
                store.set_health(HealthState::Live);
                record_route_up(&name, HealthState::Live);
                backoff.reset();
                if let Err(e) = run_pipeline(
                    store.clone(),
                    target_duration_secs,
                    part_target_ms,
                    source,
                    &name,
                )
                .await
                {
                    tracing::warn!(error = %e, "pipeline stopped");
                }
                store.set_health(HealthState::Reconnecting);
                record_route_up(&name, HealthState::Reconnecting);
                record_reconnect(&name);
            }
            Err(e) => {
                attempt += 1;
                tracing::warn!(error = %e, attempt, "failed to connect");
                store.set_health(HealthState::Reconnecting);
                record_route_up(&name, HealthState::Reconnecting);
                record_reconnect(&name);
            }
        }

        if *shutdown.borrow() {
            break;
        }

        let delay = backoff.next();
        tracing::warn!(
            delay_ms = delay.as_millis() as u64,
            attempt,
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
    use crate::pipeline::MockSource;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use transmux::avc_config_from_sprop;
    use transmux::pipeline::{CodecConfig, Sample, TrackSpec};

    const SPROP: &str = "Z0IAKeKQFAe2AtwEBAaQeJEV,aM48gA==";
    const VIDEO_TIMESCALE: u32 = 90_000;
    const FRAME_DUR: u32 = VIDEO_TIMESCALE / 30;

    fn video_track_spec() -> TrackSpec {
        let config = avc_config_from_sprop(SPROP).expect("valid sprop");
        TrackSpec::new(
            1,
            VIDEO_TIMESCALE,
            CodecConfig::Avc {
                config,
                width: 0,
                height: 0,
            },
        )
    }

    /// A handful of batches that, fed through the real segmenter, produce at
    /// least an init segment (so tests can assert media actually landed in
    /// the store, not just that health flipped).
    fn sample_batches(n: u32) -> Vec<Vec<(u32, Sample)>> {
        (0..n)
            .map(|i| {
                let data = vec![0xABu8.wrapping_add(i as u8); 32];
                let sample = Sample::new(data, FRAME_DUR, i == 0, 0);
                vec![(1u32, sample)]
            })
            .collect()
    }

    /// Tiny backoff for tests: keeps the whole suite fast regardless of how
    /// many reconnect cycles a test drives through.
    fn tiny_backoff() -> Backoff {
        Backoff::new(Duration::from_millis(1), Duration::from_millis(20), 2.0)
    }

    /// A connector that fails connect `fail_times` times, then always
    /// succeeds, yielding a fresh [`MockSource`] (cloned batches) on every
    /// successful connect.
    struct FlakyConnector {
        fail_times: usize,
        connect_count: Arc<AtomicUsize>,
        specs: Vec<TrackSpec>,
        batches: Vec<Vec<(u32, Sample)>>,
    }

    impl SourceConnector for FlakyConnector {
        type Source = MockSource;

        async fn connect(&self) -> crate::Result<MockSource> {
            let attempt = self.connect_count.fetch_add(1, Ordering::SeqCst);
            if attempt < self.fail_times {
                return Err(crate::MultimuxError::Connect {
                    reason: "flaky connector: simulated failure".into(),
                });
            }
            Ok(MockSource::new(self.specs.clone(), self.batches.clone()))
        }
    }

    /// A [`SampleSource`] that yields one batch every `delay` (a real,
    /// short `tokio::time::sleep`), then ends — standing in for a live
    /// stream that trickles samples in over wall-clock time rather than
    /// delivering everything in one synchronous burst like [`MockSource`].
    ///
    /// Needed specifically so a test can observe the supervisor sitting in
    /// `HealthState::Live` for a real (if brief) span: a source with no
    /// genuine await point completes an entire connect -> pipeline ->
    /// reconnect cycle without ever yielding to the runtime, so a
    /// cooperatively-scheduled observer task could never actually witness
    /// the `Live` state in between.
    struct PacedSource {
        specs: Vec<TrackSpec>,
        batches: std::vec::IntoIter<Vec<(u32, Sample)>>,
        delay: Duration,
    }

    impl SampleSource for PacedSource {
        fn track_specs(&self) -> Vec<TrackSpec> {
            self.specs.clone()
        }

        async fn next_samples(&mut self) -> crate::Result<Option<Vec<(u32, Sample)>>> {
            tokio::time::sleep(self.delay).await;
            Ok(self.batches.next())
        }
    }

    /// Like [`FlakyConnector`], but yields a [`PacedSource`] on success
    /// instead of an instantaneous [`MockSource`].
    struct PacedFlakyConnector {
        fail_times: usize,
        connect_count: Arc<AtomicUsize>,
        specs: Vec<TrackSpec>,
        batches: Vec<Vec<(u32, Sample)>>,
        delay: Duration,
    }

    impl SourceConnector for PacedFlakyConnector {
        type Source = PacedSource;

        async fn connect(&self) -> crate::Result<PacedSource> {
            let attempt = self.connect_count.fetch_add(1, Ordering::SeqCst);
            if attempt < self.fail_times {
                return Err(crate::MultimuxError::Connect {
                    reason: "flaky connector: simulated failure".into(),
                });
            }
            Ok(PacedSource {
                specs: self.specs.clone(),
                batches: self.batches.clone().into_iter(),
                delay: self.delay,
            })
        }
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

    /// Biting test 1: a connector that fails once then succeeds must still
    /// bring the route up — health cycles through `Connecting`/
    /// `Reconnecting` and settles on `Live`, with real samples landing in
    /// the store. Reverting the supervise loop to the old one-shot
    /// (connect-once-then-die) breaks this: the route would stay dead after
    /// the first failure and health would never leave `Reconnecting`.
    #[tokio::test]
    async fn reconnects_after_connect_failure_and_reaches_live() {
        let store = Arc::new(MediaStore::new(1.0, 500, 8));
        let connect_count = Arc::new(AtomicUsize::new(0));
        // Paced (not instantaneous) so the observer below has a real
        // wall-clock window in which to catch `Live` — see `PacedSource`'s
        // doc comment for why an instant source can't be observed this way.
        let connector = PacedFlakyConnector {
            fail_times: 1,
            connect_count: connect_count.clone(),
            specs: vec![video_track_spec()],
            batches: sample_batches(50),
            delay: Duration::from_millis(2),
        };
        let (shutdown_tx, shutdown_rx) = watch::channel(false);

        let handle = tokio::spawn(supervise(
            connector,
            store.clone(),
            1.0,
            500,
            tiny_backoff(),
            "test-route".to_string(),
            shutdown_rx,
        ));

        let reached_live = wait_until(Duration::from_secs(2), || {
            store.health() == HealthState::Live && store.init_bytes().is_some()
        })
        .await;
        assert!(
            reached_live,
            "route must recover to Live after one connect failure"
        );
        assert!(
            connect_count.load(Ordering::SeqCst) >= 2,
            "connector must have been retried after the first failure"
        );

        shutdown_tx.send(true).unwrap();
        tokio::time::timeout(Duration::from_secs(1), handle)
            .await
            .expect("supervise returns promptly on shutdown")
            .expect("supervise task did not panic");
    }

    /// Biting test 2: a source that ends (`next_samples` -> `Ok(None)`,
    /// i.e. `MockSource`'s batches exhausted) must be treated as a
    /// recoverable disconnect, not a terminal state — the supervisor
    /// reconnects (calling the connector again) rather than exiting.
    /// Reverting to the one-shot task breaks this: `connect_count` would
    /// stay at 1 forever.
    #[tokio::test]
    async fn reconnects_after_source_eof() {
        let store = Arc::new(MediaStore::new(1.0, 500, 8));
        let connect_count = Arc::new(AtomicUsize::new(0));
        let connector = FlakyConnector {
            fail_times: 0,
            connect_count: connect_count.clone(),
            specs: vec![video_track_spec()],
            // A short, finite batch list: MockSource reports EOF once
            // exhausted, ending run_pipeline and forcing a reconnect.
            batches: sample_batches(3),
        };
        let (shutdown_tx, shutdown_rx) = watch::channel(false);

        let handle = tokio::spawn(supervise(
            connector,
            store.clone(),
            1.0,
            500,
            tiny_backoff(),
            "test-route".to_string(),
            shutdown_rx,
        ));

        let reconnected = wait_until(Duration::from_secs(2), || {
            connect_count.load(Ordering::SeqCst) >= 2
        })
        .await;
        assert!(
            reconnected,
            "connector must be called again after source EOF"
        );

        shutdown_tx.send(true).unwrap();
        tokio::time::timeout(Duration::from_secs(1), handle)
            .await
            .expect("supervise returns promptly on shutdown")
            .expect("supervise task did not panic");
    }

    /// Biting test 4: firing shutdown must stop the loop promptly even
    /// mid-backoff, well under the (deliberately large, relative to the
    /// test) backoff cap — proving the sleep is cancellable, not a plain
    /// `tokio::time::sleep` the loop blindly awaits to completion.
    #[tokio::test]
    async fn shutdown_stops_the_loop_promptly() {
        let store = Arc::new(MediaStore::new(1.0, 500, 8));
        let connect_count = Arc::new(AtomicUsize::new(0));
        // Always fails, so the loop is guaranteed to be sitting in the
        // backoff sleep (not mid-pipeline) shortly after start.
        let connector = FlakyConnector {
            fail_times: usize::MAX,
            connect_count,
            specs: vec![video_track_spec()],
            batches: Vec::new(),
        };
        // A backoff far larger than the shutdown-stops-it assertion window
        // below: if shutdown didn't cancel the sleep, the timeout on the
        // join would fire first and this test would fail.
        let backoff = Backoff::new(Duration::from_secs(10), Duration::from_secs(30), 2.0);
        let (shutdown_tx, shutdown_rx) = watch::channel(false);

        let handle = tokio::spawn(supervise(
            connector,
            store,
            1.0,
            500,
            backoff,
            "test-route".to_string(),
            shutdown_rx,
        ));

        // Give the loop a moment to fail its first connect and enter the
        // (10s) backoff sleep.
        tokio::time::sleep(Duration::from_millis(20)).await;
        shutdown_tx.send(true).unwrap();

        tokio::time::timeout(Duration::from_millis(500), handle)
            .await
            .expect("supervise must return promptly on shutdown, not after the 10s backoff")
            .expect("supervise task did not panic");
    }
}
