//! Push output driver (issue #744) — relay/restream ingested media to a
//! **downstream** server (an SRT Listener, RTMP publish, or RTSP
//! ANNOUNCE/RECORD) rather than serving it over HTTP.
//!
//! The first push protocol in the workspace, this module introduces the push
//! driver infrastructure the RTMP and RTSP transports also build on
//! (`PushTransport` now has three implementors: SRT, RTMP, RTSP):
//!
//! - [`PushTransport`] — the async trait a concrete push protocol implements
//!   (`connect`, `send`, `send_media`, optional `setup`, `close`).
//! - [`ReconnectEngine`] — the exponential-backoff reconnect FSM shared by
//!   every push transport.
//! - [`PushMetrics`] — the counters a push task keeps (samples sent/dropped,
//!   bytes sent, reconnects) for observability.
//! - [`drive_push`] — the async main loop: subscribe to a [`Trunk`]'s sample
//!   ring and hand the drained batch to [`PushTransport::send_media`],
//!   reconnecting on failure with [`ReconnectPolicy`](crate::config::ReconnectPolicy)
//!   backoff.
//!
//! This is the *reverse* of the ingest path: instead of demuxing inbound media
//! and publishing samples into a `Trunk`, a push driver subscribes to a
//! `Trunk`'s samples and muxes them outbound. Each transport picks its own
//! wire container via [`PushTransport::send_media`]: SRT and RTSP mux with
//! `transmux::TsMux` (`broadcast_common::Package`) and ship one opaque TS
//! blob per batch (the trait's default); RTMP instead splits samples into
//! FLV-framed messages (`transmux::flv_frame_payloads`) and ships them via
//! `send_video`/`send_audio` (issue #934) — RTMP carries FLV
//! `VIDEODATA`/`AUDIODATA` payloads, not MPEG-2 TS.
//!
//! The supervisor/origin wiring (spawning a push task per configured route)
//! is live: `crate::origin::spawn_push_outputs` spawns one [`drive_push`]
//! task per `srt_push`/`rtmp_push`/`rtsp_push` output on a route
//! (`src/origin/mod.rs`), and `crate::origin::admin` starts/stops those tasks
//! as routes are added/removed at runtime.

mod rtmp;
mod rtsp;
mod srt;

use std::sync::Arc;
use std::sync::atomic::{AtomicU32, AtomicU64, Ordering};
use std::time::{Duration, Instant};

use media_plane::trunk::{SampleCursorItem, Trunk};
use tokio::time::sleep;
use tokio_util::sync::CancellationToken;

use crate::config::PushFormat;
use broadcast_common::Package;
use transmux::TsMux;
use transmux::ir::{Media, Sample, Track, TrackSpec};

pub use rtmp::{RtmpTransport, RtmpTransportConfig};
pub use rtsp::{RtspTransport, RtspTransportConfig};
pub use srt::{SrtTransport, SrtTransportConfig};

/// MPEG-2 TS `stream_type` for PES private data — ISO/IEC 13818-1 Table 2-34.
/// The opaque `CodecConfig::Data` fallback for a drained sample whose `track_id`
/// has no matching spec in the trunk's track set.
const STREAM_TYPE_PRIVATE: u8 = 0x06;

/// Error from [`PushTransport::send_media`] — distinguishes a muxing failure
/// (not a connection problem: `drive_push` drops the batch without forcing a
/// reconnect, matching the pre-#934 `TsMux::package` handling) from a
/// transport-level send/protocol failure (`drive_push` reconnects).
///
/// Generic over no transport type — every [`PushTransport::Error`] already
/// requires `std::error::Error + Send + Sync + 'static` (the trait bound),
/// so it boxes into `Transport` uniformly regardless of which transport
/// raised it.
#[non_exhaustive]
#[derive(Debug, thiserror::Error)]
pub enum SendMediaError {
    /// Muxing/framing the batch failed — no connection to blame, so the
    /// batch is dropped without forcing a reconnect.
    #[error("push mux failed: {0}")]
    Mux(String),
    /// The transport's own send/protocol call failed.
    #[error("push send failed: {0}")]
    Transport(Box<dyn std::error::Error + Send + Sync>),
}

/// A concrete push transport: the wire protocol half of a push output.
///
/// Mirrors the ingest side's *socket-as-handle* shape (e.g.
/// [`srt_runtime::io::SrtSocket`]) — [`connect`](Self::connect) dials out to
/// the downstream server, [`send_media`](Self::send_media) muxes and pushes
/// one drained batch of samples, and [`close`](Self::close) tears the
/// connection down.
#[async_trait::async_trait]
pub trait PushTransport: Send + 'static {
    /// The transport's per-connection configuration
    /// (e.g. [`SrtTransportConfig`] for SRT).
    type Config: Send + Sync + Clone;
    /// The transport's connection/send error type.
    type Error: std::error::Error + Send + Sync + 'static;

    /// Dial out to the downstream server at `url`, using `config`.
    async fn connect(url: &str, config: &Self::Config) -> Result<Self, Self::Error>
    where
        Self: Sized;

    /// Push one application payload to the downstream server.
    ///
    /// The low-level "write these bytes" primitive — used by
    /// [`send_media`](Self::send_media)'s default implementation (SRT/RTSP:
    /// one opaque MPEG-2 TS blob per batch). A transport whose wire format
    /// isn't "one blob" (RTMP, issue #934: distinct typed messages per
    /// elementary stream) overrides `send_media` instead and may leave this
    /// unused by `drive_push`, but it must still exist so tests/other code
    /// can push a raw payload directly.
    async fn send(&mut self, data: &[u8]) -> Result<(), Self::Error>;

    /// Optional session setup before the first `send`/`send_media` — e.g.
    /// RTMP's `connect`/`createStream`/`publish` (issue #934: also where the
    /// AVC/AAC sequence headers + `onMetaData` are sent, once), or RTSP's
    /// `ANNOUNCE`/`RECORD`.
    async fn setup(&mut self, _tracks: &[TrackSpec]) -> Result<(), Self::Error> {
        Ok(())
    }

    /// Mux one drained batch of samples into this transport's native wire
    /// format and send it. Default: MPEG-2 TS via [`TsMux`], shipped as one
    /// opaque payload through [`send`](Self::send) — what SRT and RTSP push
    /// both want. RTMP overrides this (issue #934): RTMP messages carry FLV
    /// `VIDEODATA`/`AUDIODATA` bodies, not TS, so it splits `media` into
    /// per-frame payloads (`transmux::flv_frame_payloads`) and dispatches
    /// each through `send_video`/`send_audio` directly, bypassing `send`
    /// and `TsMux` entirely.
    ///
    /// Returns the number of payload bytes actually sent, for
    /// [`PushMetrics::bytes_sent`].
    async fn send_media(&mut self, media: &Media) -> Result<u64, SendMediaError> {
        let bytes = TsMux::new()
            .package(media)
            .map_err(|e| SendMediaError::Mux(e.to_string()))?;
        self.send(&bytes)
            .await
            .map_err(|e| SendMediaError::Transport(Box::new(e)))?;
        Ok(bytes.len() as u64)
    }

    /// Tear this transport down (free the handle/abort its driver task).
    fn close(&mut self);
}

/// The exponential-backoff reconnect state machine shared by every push
/// transport.
///
/// - [`Ready`](Self::Ready) — connected (or never connected), push normally.
/// - [`Backoff`](Self::Backoff) — disconnected; waiting until `resume_at`
///   before dialing out again.
/// - [`Failed`](Self::Failed) — gave up permanently after exceeding
///   [`ReconnectPolicy::max_attempts`](crate::config::ReconnectPolicy::max_attempts).
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum ReconnectState {
    /// Connected (or never had a connection to tear down yet).
    Ready,
    /// Disconnected; do not dial again until `resume_at`.
    Backoff { resume_at: Instant },
    /// Gave up permanently (exceeded `max_attempts`).
    Failed,
}

/// The reconnect engine: [`ReconnectState`] + the attempt counter + the
/// backoff/attempt budget that drives it.
#[derive(Debug, Clone)]
pub struct ReconnectEngine {
    state: ReconnectState,
    attempt: u32,
    policy: crate::config::ReconnectPolicy,
}

impl ReconnectEngine {
    /// Start in [`ReconnectState::Ready`] (nothing to reconnect to yet).
    pub fn new(policy: crate::config::ReconnectPolicy) -> Self {
        Self {
            state: ReconnectState::Ready,
            attempt: 0,
            policy,
        }
    }

    /// Whether a dial-out should happen now.
    pub fn should_connect(&self) -> bool {
        match self.state {
            ReconnectState::Ready => true,
            ReconnectState::Failed => false,
            ReconnectState::Backoff { resume_at } => Instant::now() >= resume_at,
        }
    }

    /// A connection attempt started successfully — reset the attempt counter.
    pub fn on_connect(&mut self) {
        self.attempt = 0;
        self.state = ReconnectState::Ready;
    }

    /// The connection dropped (or a dial-out failed) — enter backoff,
    /// increment the attempt counter, and go permanently [`Failed`](ReconnectState::Failed) once
    /// `max_attempts` is exceeded.
    ///
    /// Backoff follows the doubling series 1s, 2s, 4s, … (each retry
    /// doubles), capped at [`ReconnectPolicy::max_backoff_ms`](crate::config::ReconnectPolicy::max_backoff_ms) — the `attempt`
    /// counter is 1-based, so the `n`-th disconnect waits
    /// [`ReconnectPolicy::backoff_for(n - 1)`](crate::config::ReconnectPolicy::backoff_for).
    pub fn on_disconnect(&mut self) {
        self.attempt = self.attempt.saturating_add(1);
        if let Some(max) = self.policy.max_attempts {
            if self.attempt >= max {
                self.state = ReconnectState::Failed;
                return;
            }
        }
        let backoff = self.policy.backoff_for(self.attempt.saturating_sub(1));
        self.state = ReconnectState::Backoff {
            resume_at: Instant::now() + backoff,
        };
    }

    /// Whether the engine has given up permanently.
    pub fn is_failed(&self) -> bool {
        self.state == ReconnectState::Failed
    }

    /// Time until the next dial-out, or [`Duration::ZERO`] if a dial-out
    /// should happen immediately / the engine is ready.
    pub fn time_until_retry(&self) -> Duration {
        match self.state {
            ReconnectState::Ready | ReconnectState::Failed => Duration::ZERO,
            ReconnectState::Backoff { resume_at } => {
                resume_at.saturating_duration_since(Instant::now())
            }
        }
    }

    /// The current engine state.
    pub fn state(&self) -> &ReconnectState {
        &self.state
    }
}

/// The counters a push task keeps, for observability/health.
#[derive(Debug, Default)]
pub struct PushMetrics {
    samples_sent: AtomicU64,
    samples_dropped: AtomicU64,
    bytes_sent: AtomicU64,
    reconnect_count: AtomicU32,
}

impl PushMetrics {
    /// Total samples successfully pushed downstream.
    pub fn samples_sent(&self) -> u64 {
        self.samples_sent.load(Ordering::Relaxed)
    }
    /// Total samples dropped while disconnected / backed off.
    pub fn samples_dropped(&self) -> u64 {
        self.samples_dropped.load(Ordering::Relaxed)
    }
    /// Total payload bytes successfully pushed downstream.
    pub fn bytes_sent(&self) -> u64 {
        self.bytes_sent.load(Ordering::Relaxed)
    }
    /// Total successful reconnects.
    pub fn reconnect_count(&self) -> u32 {
        self.reconnect_count.load(Ordering::Relaxed)
    }
}

/// The main push loop: subscribe to `trunk`'s sample ring and push its
/// samples to the downstream server over `T`, reconnecting on failure per
/// `reconnect` (issue #744).
///
/// Returns the [`PushMetrics`] accumulated for the lifetime of this task.
/// Cancellation is cooperative via `cancel` — the loop checks it between
/// drain steps (never mid-`send`).
pub async fn drive_push<T: PushTransport>(
    trunk: Arc<Trunk>,
    url: String,
    config: T::Config,
    _format: PushFormat,
    reconnect: crate::config::ReconnectPolicy,
    cancel: CancellationToken,
) -> PushMetrics {
    let metrics = PushMetrics::default();
    let mut transport: Option<T> = None;
    let mut engine = ReconnectEngine::new(reconnect);
    let mut cursor = trunk.subscribe();
    // `Trunk` samples are already `transmux::Sample`s; the track set we mux
    // against comes straight from the trunk. `_format` is unused here — it
    // only ever selects TS (the only implemented `PushFormat`); `Mp4`/`Mkv`
    // are rejected at config-validate time (`crate::config::Route::
    // validate_standalone`), so by the time a push task runs, `_format` is
    // guaranteed to be `Ts`.
    let _ = _format;

    loop {
        if cancel.is_cancelled() {
            if let Some(t) = transport.as_mut() {
                t.close();
            }
            return metrics;
        }

        // Wait for samples, or a bounded wake-up, before draining — the
        // 250 ms cap keeps cancellation / track-set changes noticed.
        if let Some(listener) = trunk.listen() {
            listener.wait_deadline(Instant::now() + Duration::from_millis(250));
        }

        // Drain the cursor, accumulating samples per track_id.
        let mut drained: Vec<(u32, Vec<Sample>)> = Vec::new();
        while let Some(item) = cursor.poll() {
            match item {
                SampleCursorItem::Timed { track_id, sample }
                | SampleCursorItem::Sparse { track_id, sample } => {
                    match drained.iter_mut().find(|(id, _)| *id == track_id) {
                        Some((_, samples)) => samples.push(sample),
                        None => drained.push((track_id, vec![sample])),
                    }
                }
                SampleCursorItem::Lagged { skipped } | SampleCursorItem::Degraded { skipped } => {
                    metrics
                        .samples_dropped
                        .fetch_add(skipped, Ordering::Relaxed);
                }
                // `#[non_exhaustive]`: future cursor items are ignored.
                _ => {}
            }
        }

        if engine.is_failed() {
            // Drained the cursor above; give up permanently.
            return metrics;
        }

        // Connect / reconnect when due and not already connected.
        if engine.should_connect() && transport.is_none() {
            match T::connect(&url, &config).await {
                Ok(mut conn) => {
                    engine.on_connect();
                    metrics.reconnect_count.fetch_add(1, Ordering::Relaxed);
                    let tracks = trunk.tracks();
                    if let Err(e) = conn.setup(&tracks).await {
                        tracing::warn!(%url, error = %e, "push setup failed; closing");
                        conn.close();
                        engine.on_disconnect();
                        continue;
                    }
                    transport = Some(conn);
                }
                Err(e) => {
                    tracing::warn!(%url, error = %e, "push connect failed; backing off");
                    engine.on_disconnect();
                }
            }
        }

        // Push the drained batch when connected; otherwise sleep out the
        // backoff (cancellation-aware) and drop the batch.
        if drained.is_empty() {
            continue;
        }
        if transport.is_some() {
            let media = media_from_samples(&trunk.tracks(), &drained);
            match transport.as_mut().unwrap().send_media(&media).await {
                Ok(sent_bytes) => {
                    metrics.bytes_sent.fetch_add(sent_bytes, Ordering::Relaxed);
                    let sent: u64 = drained.iter().map(|(_, s)| s.len() as u64).sum();
                    metrics.samples_sent.fetch_add(sent, Ordering::Relaxed);
                }
                Err(SendMediaError::Mux(msg)) => {
                    tracing::error!(error = %msg, "push mux failed; dropping batch");
                    let dropped: u64 = drained.iter().map(|(_, s)| s.len() as u64).sum();
                    metrics
                        .samples_dropped
                        .fetch_add(dropped, Ordering::Relaxed);
                }
                Err(SendMediaError::Transport(e)) => {
                    tracing::warn!(%url, error = %e, "push send failed; reconnecting");
                    // Close so `should_connect` dials out afresh.
                    transport.as_mut().unwrap().close();
                    transport = None;
                    engine.on_disconnect();
                }
            }
        } else {
            // Backoff (or connect still pending): count the batch dropped and
            // sleep until the next dial-out is due.
            let dropped: u64 = drained.iter().map(|(_, s)| s.len() as u64).sum();
            metrics
                .samples_dropped
                .fetch_add(dropped, Ordering::Relaxed);
            let wait = engine.time_until_retry();
            if !wait.is_zero() {
                sleep(wait).await;
            }
        }
    }
}

/// Build a [`Media`] from the drained samples, pairing each `track_id`'s
/// samples with its spec from `tracks` (the trunk's current track set).
fn media_from_samples(tracks: &[TrackSpec], drained: &[(u32, Vec<Sample>)]) -> Media {
    let mut built: Vec<Track> = Vec::new();
    for (id, samples) in drained {
        let spec = tracks
            .iter()
            .find(|s| s.track_id == *id)
            .cloned()
            .unwrap_or_else(|| {
                // Opaque PES carriage for a track with no matching spec — the TS
                // mux path can carry these verbatim (`CodecConfig::is_muxable_in_bmff`
                // only gates the *fMP4* path).
                TrackSpec::new(
                    *id,
                    90_000,
                    transmux::CodecConfig::Data {
                        stream_type: STREAM_TYPE_PRIVATE, // ISO/IEC 13818-1 Table 2-34: PES private data
                        descriptors: Vec::new(),
                        carriage: transmux::ir::DataCarriage::Pes,
                    },
                )
            });
        built.push(Track::new(spec, samples.clone()));
    }
    let timescale = tracks.iter().next().map(|t| t.timescale).unwrap_or(90_000);
    Media::new(built, timescale)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::ReconnectPolicy;

    fn policy(max_attempts: Option<u32>) -> ReconnectPolicy {
        ReconnectPolicy {
            initial_backoff_ms: 1_000,
            max_backoff_ms: 30_000,
            max_attempts,
        }
    }

    #[test]
    fn backoff_doubles_each_attempt_capped_at_max() {
        let eng = ReconnectEngine::new(policy(None));
        // backoff_for(attempt) is the doubling series from index 0:
        // 1s, 2s, 4s, ... (the n-th disconnect waits backoff_for(n - 1)).
        assert_eq!(eng.policy.backoff_for(0), Duration::from_millis(1_000));
        assert_eq!(eng.policy.backoff_for(1), Duration::from_millis(2_000));
        assert_eq!(eng.policy.backoff_for(2), Duration::from_millis(4_000));
        // Capped at max_backoff_ms (30s here).
        assert_eq!(eng.policy.backoff_for(20), Duration::from_millis(30_000));
        assert_eq!(eng.policy.backoff_for(30), Duration::from_millis(30_000));
    }

    #[test]
    fn max_attempts_triggers_failed() {
        let mut eng = ReconnectEngine::new(policy(Some(3)));
        assert!(!eng.is_failed());
        for _ in 0..3 {
            eng.on_disconnect();
        }
        assert!(
            eng.is_failed(),
            "exceeding max_attempts must fail the engine"
        );
    }

    #[test]
    fn on_connect_resets_attempt_counter() {
        let mut eng = ReconnectEngine::new(policy(Some(2)));
        eng.on_disconnect();
        eng.on_disconnect();
        assert!(eng.is_failed());
        eng.on_connect();
        assert!(!eng.is_failed());
        assert!(eng.should_connect());
    }

    #[test]
    fn should_connect_respects_backoff_timing() {
        let mut eng = ReconnectEngine::new(policy(None));
        assert!(eng.should_connect());
        eng.on_disconnect();
        // In backoff, not yet resumed.
        assert!(eng.time_until_retry() > Duration::ZERO);
        assert!(!eng.should_connect() || eng.time_until_retry().is_zero());
        // Never failed.
        assert!(!eng.is_failed());
    }
}
