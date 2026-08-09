//! Push output driver (issue #744) — relay/restream ingested media to a
//! **downstream** server (an SRT Listener, RTMP publish, or RTSP
//! ANNOUNCE/RECORD) rather than serving it over HTTP.
//!
//! The first push protocol in the workspace, this module introduces the push
//! driver infrastructure the RTMP and RTSP transports also build on
//! (`PushTransport` now has three implementors: SRT, RTMP, RTSP):
//!
//! - [`PushTransport`] — the async trait a concrete push protocol implements
//!   (`connect`, `send`, `write_message`, `send_media`, optional `setup`,
//!   `close`) — the byte/socket layer.
//! - [`egress::PushTransportEgress`] — [`media_plane::PushEgress`] over a
//!   [`PushTransport`] (issue #942): negotiates which tracks the wire
//!   container can carry and muxes drained samples into wire messages,
//!   sans-IO — see that module's own doc for why this is a composed layer,
//!   not a duplicate of `PushTransport`.
//! - [`ReconnectEngine`] — the exponential-backoff reconnect FSM shared by
//!   every push transport.
//! - [`PushMetrics`] — the counters a push task keeps (samples sent/dropped,
//!   bytes sent, reconnects) for observability.
//! - [`drive_push`] — the async main loop: subscribe to a [`Trunk`]'s sample
//!   ring, negotiate/renegotiate and feed each drained item to a
//!   [`egress::PushTransportEgress`], flushing its queued wire messages out
//!   over the real transport every iteration, reconnecting on failure with
//!   [`ReconnectPolicy`](crate::config::ReconnectPolicy) backoff.
//!
//! This is the *reverse* of the ingest path: instead of demuxing inbound media
//! and publishing samples into a `Trunk`, a push driver subscribes to a
//! `Trunk`'s samples and muxes them outbound. Each transport picks its own
//! wire container via [`PushTransport::encode_media`]: SRT and RTSP mux with
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

mod egress;
mod rtmp;
mod rtsp;
mod srt;

use std::sync::Arc;
use std::sync::atomic::{AtomicU32, AtomicU64, Ordering};
use std::time::{Duration, Instant};

use media_plane::egress::{NegotiationOutcome, PushEgress, TrackSelection};
use media_plane::trunk::{SampleCursorItem, Trunk};
use tokio::time::sleep;
use tokio_util::sync::CancellationToken;

pub use egress::PushTransportEgress;

use crate::config::PushFormat;
use broadcast_common::Package;
use transmux::TsMux;
use transmux::ir::{Media, TrackSpec};

pub use rtmp::{RtmpTransport, RtmpTransportConfig};
pub use rtsp::{RtspTransport, RtspTransportConfig};
pub use srt::{SrtTransport, SrtTransportConfig};

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
    /// one opaque MPEG-2 TS blob per batch). **Its contract is "frame `data`
    /// as this transport's own idea of one payload"**, not "write these
    /// bytes verbatim" — for SRT/RTSP that's the same thing (there is no
    /// separate framing step), but RTMP's override always wraps `data` in a
    /// *new* `send_video` chunk-stream message (issue #934: distinct typed
    /// messages per elementary stream), so it must never be called with
    /// bytes [`encode_media`](Self::encode_media) already framed — see
    /// [`write_message`](Self::write_message) for that case. A transport
    /// whose wire format isn't "one blob" overrides `send_media` instead
    /// and may leave this unused by `drive_push`, but it must still exist
    /// so tests/other code can push a raw payload directly.
    async fn send(&mut self, data: &[u8]) -> Result<(), Self::Error>;

    /// Write one already wire-ready message **verbatim** — no framing, no
    /// encoding, just bytes to the socket (issue #942). This is what
    /// [`egress::PushTransportEgress::flush_transmit`] calls to actually
    /// transmit what [`encode_media`](Self::encode_media) queued, and it is
    /// deliberately a *different* method from [`send`](Self::send): for
    /// SRT/RTSP the default forwarding to `send` is correct (their `send` is
    /// already a raw write, no framing side effect), but RTMP's `send` is
    /// not — its whole job is to wrap `data` in a fresh chunk-stream
    /// message, which would double-frame a message
    /// [`encode_media`](Self::encode_media) already produced (RTMP
    /// overrides this instead).
    async fn write_message(&mut self, message: &[u8]) -> Result<(), Self::Error> {
        self.send(message).await
    }

    /// Optional session setup before the first `send`/`send_media` — e.g.
    /// RTMP's `connect`/`createStream`/`publish` (issue #934: also where the
    /// AVC/AAC sequence headers + `onMetaData` are sent, once), or RTSP's
    /// `ANNOUNCE`/`RECORD`.
    async fn setup(&mut self, _tracks: &[TrackSpec]) -> Result<(), Self::Error> {
        Ok(())
    }

    /// Whether this transport's wire container can carry `config` at all
    /// (issue #942: the structured vocabulary [`PushTransportEgress`]'s
    /// `negotiate`/`renegotiate` need to refuse a track truthfully,
    /// replacing the ad-hoc warn-once-then-silently-drop logic
    /// `push::rtmp::RtmpTransport` used to carry on its own). Default
    /// `true`: MPEG-2 TS (the default [`send_media`](Self::send_media))
    /// carries essentially any `transmux::CodecConfig` SRT/RTSP push hand
    /// it, opaque PES included — RTMP is the one transport whose wire
    /// container (FLV) has a real, narrow codec set and overrides this.
    fn supports_codec(&self, _config: &transmux::CodecConfig) -> bool {
        true
    }

    /// Encode one drained batch into this transport's native wire messages,
    /// performing **no I/O** (issue #942) — the sans-IO half of
    /// [`send_media`](Self::send_media). [`egress::PushTransportEgress::send`]
    /// is why this exists as a method a caller can invoke separately from
    /// an actual write: its trait, [`media_plane::PushEgress`], is
    /// deliberately synchronous (mirrors
    /// [`media_plane::ingress::IngestSession::poll_transmit`]'s sans-IO
    /// shape — see that adapter's module doc), so it cannot `.await` a
    /// socket write itself; it calls this instead, queues the resulting
    /// messages, and hands them to an external async driver through
    /// [`media_plane::PushEgress::poll_transmit`].
    ///
    /// Default: one message, the whole batch MPEG-2 TS-muxed via [`TsMux`]
    /// — exactly [`send_media`](Self::send_media)'s own default encode step,
    /// just without the `.await` that used to immediately follow it. RTMP
    /// overrides this (issue #934/#942): RTMP messages carry FLV
    /// `VIDEODATA`/`AUDIODATA` bodies, not TS, so it splits `media` into
    /// per-frame, chunk-stream-framed messages via `rtmp_runtime::client::
    /// ClientSession::send_video`/`send_audio` — themselves sans-IO calls,
    /// so this really is a clean split, not a workaround.
    fn encode_media(&mut self, media: &Media) -> Result<Vec<bytes::Bytes>, SendMediaError> {
        let bytes = TsMux::new()
            .package(media)
            .map_err(|e| SendMediaError::Mux(e.to_string()))?;
        Ok(vec![bytes::Bytes::from(bytes)])
    }

    /// Mux one drained batch of samples into this transport's native wire
    /// format and send it: [`encode_media`](Self::encode_media) (sans-IO),
    /// then [`send`](Self::send) (I/O) for each resulting message — what
    /// SRT and RTSP push both want via the default `encode_media` (one TS
    /// message per batch). RTMP overrides both methods (issue #934): see
    /// [`encode_media`](Self::encode_media)'s own doc.
    ///
    /// Returns the number of payload bytes actually sent, for
    /// [`PushMetrics::bytes_sent`].
    async fn send_media(&mut self, media: &Media) -> Result<u64, SendMediaError> {
        let messages = self.encode_media(media)?;
        let mut total = 0u64;
        for message in &messages {
            self.send(message)
                .await
                .map_err(|e| SendMediaError::Transport(Box::new(e)))?;
            total += message.len() as u64;
        }
        Ok(total)
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
        if let Some(max) = self.policy.max_attempts
            && self.attempt >= max
        {
            self.state = ReconnectState::Failed;
            return;
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

/// Logs the "some tracks excluded" fact a `negotiate`/`renegotiate`
/// [`NegotiationOutcome::Accepted`] carries whenever its [`TrackSelection`]
/// names fewer tracks than were proposed — the structured replacement for
/// `push::rtmp::RtmpTransport`'s old `warned_refused_tracks` flag (issue
/// #942): the selection itself is the "which tracks, and how many were
/// excluded" fact, so there is nothing left to track privately inside the
/// transport.
fn log_partial_selection(url: &str, proposed: usize, selection: &TrackSelection) {
    if selection.track_ids.len() < proposed {
        tracing::warn!(
            url,
            carried = selection.track_ids.len(),
            proposed,
            "push output cannot carry every track in this program; excluded tracks are \
             dropped from this push"
        );
    }
}

/// The main push loop: subscribe to `trunk`'s sample ring and push its
/// samples to the downstream server over `T`, reconnecting on failure per
/// `reconnect` (issue #744).
///
/// Issue #942: routes every sample through a [`PushTransportEgress<T>`]
/// (`media_plane::PushEgress` over `T`) rather than muxing/sending directly.
/// `negotiate` runs once per connection (replacing `RtmpTransport::setup`'s
/// old ad-hoc all-or-nothing codec check); `renegotiate` runs whenever
/// [`Trunk::track_generation`] changes mid-connection (issue #781's shape,
/// on the push side — nothing before issue #942 detected this at all).
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
    /// The generic [`NegotiationOutcome::Error`] reason when a proposed
    /// track set has no track this push output's wire format can carry at
    /// all — deliberately transport-agnostic (this function is generic over
    /// `T`); the *specific* reason (e.g. RTMP's "no AVC video or AAC audio
    /// track…") still reaches the log line below via `Display`.
    const UNSATISFIABLE: &str = "no track this output's container format can carry";

    let metrics = PushMetrics::default();
    let mut egress: Option<PushTransportEgress<T>> = None;
    let mut engine = ReconnectEngine::new(reconnect);
    let mut cursor = trunk.subscribe();
    // The `Trunk::track_generation` last negotiated/renegotiated against —
    // `None` until the first successful `negotiate`, so the very first
    // connection always renegotiates-on-connect rather than skipping it.
    let mut negotiated_generation: Option<u64> = None;
    // `Trunk` samples are already `transmux::Sample`s; the track set we mux
    // against comes straight from the trunk. `_format` is unused here — it
    // only ever selects TS (the only implemented `PushFormat`); `Mp4`/`Mkv`
    // are rejected at config-validate time (`crate::config::Route::
    // validate_standalone`), so by the time a push task runs, `_format` is
    // guaranteed to be `Ts`.
    let _ = _format;

    loop {
        if cancel.is_cancelled() {
            if let Some(e) = egress.as_mut() {
                e.transport_mut().close();
            }
            return metrics;
        }

        // Wait for samples, or a bounded wake-up, before draining — the
        // 250 ms cap keeps cancellation / track-set changes noticed.
        if let Some(listener) = trunk.listen() {
            listener.wait_deadline(Instant::now() + Duration::from_millis(250));
        }

        // Renegotiate if the track set changed since the last successful
        // negotiate/renegotiate on this connection (issue #781's shape).
        if let Some(e) = egress.as_mut() {
            let generation = trunk.track_generation();
            if negotiated_generation != Some(generation) {
                let tracks_now = trunk.tracks();
                match e.renegotiate(&tracks_now) {
                    NegotiationOutcome::Accepted(sel) => {
                        log_partial_selection(&url, tracks_now.len(), &sel);
                        negotiated_generation = Some(generation);
                    }
                    NegotiationOutcome::Refused { reason } => {
                        // Truthful, in-band refusal (issue #781): keep
                        // running on whichever selection was last accepted
                        // — exactly `NegotiationOutcome::Refused`'s own
                        // documented contract — rather than silently
                        // adopting the change or tearing the connection
                        // down over it.
                        tracing::warn!(
                            url,
                            reason,
                            "push renegotiate refused; continuing on the previous track \
                             selection"
                        );
                        negotiated_generation = Some(generation);
                    }
                    NegotiationOutcome::Error(err) => {
                        tracing::warn!(url, error = %err, "push renegotiate failed; closing");
                        e.transport_mut().close();
                        egress = None;
                        negotiated_generation = None;
                        engine.on_disconnect();
                    }
                    // `NegotiationOutcome` is `#[non_exhaustive]` (defined in
                    // `media_plane`): a future variant this loop has no
                    // reaction to yet is treated as "nothing changed" —
                    // safe by construction, since it means neither
                    // `Accepted` nor `Refused` nor `Error` fired, so the
                    // previous selection (whatever it was) keeps running
                    // unmodified, exactly like an unrecognized
                    // `AcceptOutcome` elsewhere in this crate.
                    _ => {}
                }
            }
        }

        // Drain the cursor, feeding each item straight to the egress (sans-IO
        // — see `PushTransportEgress`'s module doc) when connected.
        let mut dropped_while_disconnected = 0u64;
        while let Some(item) = cursor.poll() {
            if let SampleCursorItem::Lagged { skipped } | SampleCursorItem::Degraded { skipped } =
                &item
            {
                metrics
                    .samples_dropped
                    .fetch_add(*skipped, Ordering::Relaxed);
            }
            let Some(e) = egress.as_mut() else {
                if matches!(
                    item,
                    SampleCursorItem::Timed { .. } | SampleCursorItem::Sparse { .. }
                ) {
                    dropped_while_disconnected += 1;
                }
                continue;
            };
            match e.send(&item) {
                Ok(()) => {
                    if matches!(
                        item,
                        SampleCursorItem::Timed { .. } | SampleCursorItem::Sparse { .. }
                    ) {
                        metrics.samples_sent.fetch_add(1, Ordering::Relaxed);
                    }
                }
                Err(SendMediaError::Mux(msg)) => {
                    tracing::error!(error = %msg, "push mux failed; dropping sample");
                    metrics.samples_dropped.fetch_add(1, Ordering::Relaxed);
                }
                Err(SendMediaError::Transport(err)) => {
                    tracing::warn!(url, error = %err, "push send failed; reconnecting");
                    e.transport_mut().close();
                    egress = None;
                    negotiated_generation = None;
                    engine.on_disconnect();
                }
            }
        }
        if dropped_while_disconnected > 0 {
            metrics
                .samples_dropped
                .fetch_add(dropped_while_disconnected, Ordering::Relaxed);
        }

        if engine.is_failed() {
            // Drained the cursor above; give up permanently.
            return metrics;
        }

        // Connect / reconnect when due and not already connected.
        if engine.should_connect() && egress.is_none() {
            match T::connect(&url, &config).await {
                Ok(conn) => {
                    let mut e = PushTransportEgress::new(conn, UNSATISFIABLE);
                    let tracks_now = trunk.tracks();
                    let generation = trunk.track_generation();
                    match e.negotiate(&tracks_now) {
                        NegotiationOutcome::Accepted(sel) => {
                            log_partial_selection(&url, tracks_now.len(), &sel);
                            let selected = e.selected_tracks().to_vec();
                            if let Err(err) = e.transport_mut().setup(&selected).await {
                                tracing::warn!(url, error = %err, "push setup failed; closing");
                                e.transport_mut().close();
                                engine.on_disconnect();
                            } else {
                                engine.on_connect();
                                metrics.reconnect_count.fetch_add(1, Ordering::Relaxed);
                                negotiated_generation = Some(generation);
                                egress = Some(e);
                            }
                        }
                        NegotiationOutcome::Error(err) => {
                            tracing::warn!(url, error = %err, "push negotiate failed; backing off");
                            engine.on_disconnect();
                        }
                        // `NegotiationOutcome` is `#[non_exhaustive]`; a
                        // first `negotiate` never returns `Refused` today
                        // (there is no prior selection yet to fall back on
                        // — see the trait's own doc), so a future variant
                        // here is treated the same as a hard failure.
                        _ => {
                            engine.on_disconnect();
                        }
                    }
                }
                Err(e) => {
                    tracing::warn!(url, error = %e, "push connect failed; backing off");
                    engine.on_disconnect();
                }
            }
        }

        // Flush whatever `send` queued this iteration (the async half —
        // see `PushTransportEgress::flush_transmit`'s own doc); otherwise
        // sleep out the backoff (cancellation-aware).
        if let Some(e) = egress.as_mut() {
            if let Err(SendMediaError::Transport(err)) = e.flush_transmit().await {
                tracing::warn!(url, error = %err, "push send failed while flushing; reconnecting");
                e.transport_mut().close();
                egress = None;
                negotiated_generation = None;
                engine.on_disconnect();
            }
        } else {
            let wait = engine.time_until_retry();
            if !wait.is_zero() {
                sleep(wait).await;
            }
        }
    }
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
