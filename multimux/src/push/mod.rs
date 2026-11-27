//! Push output driver (issue #744) — relay/restream ingested media to a
//! **downstream** server (an SRT Listener, RTMP publish, or RTSP
//! ANNOUNCE/RECORD) rather than serving it over HTTP.
//!
//! The first push protocol in the workspace, this module introduces the push
//! driver infrastructure the RTMP and RTSP transports also build on
//! (`PushTransport` now has three implementors: SRT, RTMP, RTSP):
//!
//! - [`PushTransport`] — the async trait a concrete push protocol implements
//!   (`connect`, `send`, `write_message`, optional `setup`, `close`) — the
//!   byte/socket layer.
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
//!   ring and hand every drained item, connect/setup outcome and flush result
//!   to a [`media_plane::push_session::PushSession`], which owns every
//!   decision; `drive_push` performs the awaits (connect, setup, the bounded
//!   flush, backoff), pushes the queued wire messages out over the real
//!   transport every iteration, and reconnects on failure with
//!   [`ReconnectPolicy`](crate::config::ReconnectPolicy) backoff.
//!
//! This is the *reverse* of the ingest path: instead of demuxing inbound media
//! and publishing samples into a `Trunk`, a push driver subscribes to a
//! `Trunk`'s samples and muxes them outbound. Each transport picks its own
//! wire container through [`PushTransport::media_muxer`]: SRT and RTSP keep a
//! [`TsMuxStage`] and emit whole 188-byte TS packets, RTMP keeps an
//! `RtmpMuxStage` and emits FLV `VIDEODATA`/`AUDIODATA` payloads framed into
//! RTMP messages (issue #934) — every push path frames one sample at a time and
//! never builds a whole batch.
//!
//! The supervisor/origin wiring (spawning a push task per configured route)
//! is live: `crate::origin::spawn_push_outputs` spawns one [`drive_push`]
//! task per `srt_push`/`rtmp_push`/`rtsp_push` output on a route
//! (`src/origin/mod.rs`), and `crate::origin::admin` starts/stops those tasks
//! as routes are added/removed at runtime.

pub(crate) mod egress;
pub mod rtmp;
pub mod rtsp;
pub mod srt;

use std::sync::Arc;
use std::sync::atomic::{AtomicU32, AtomicU64, Ordering};
use std::time::{Duration, Instant};

use media_plane::push_session::{
    FlushResult, PushDecision, PushErrorClass, PushErrorKind, PushSession, ReconnectCause,
};
use media_plane::trunk::Trunk;
use tokio::time::sleep;
use tokio_util::sync::CancellationToken;

use egress::FlushOutcome;
pub use egress::PushTransportEgress;

use crate::config::PushFormat;
use broadcast_common::Stage;
use broadcast_common::stage::Timestamp;
use transmux::ir::{Sample, TrackSpec};
use transmux::mux_stage::{MuxInput, MuxOutput};
use transmux::{DEFAULT_PSI_INTERVAL, TrackId, TsMuxStage};

/// Longest wait for a trunk wake-up before the push loop re-checks its queued
/// flush and renegotiation work, so a lost wake cannot stall an idle source.
const TRUNK_WAKE_CAP: Duration = Duration::from_millis(250);

pub use rtmp::{RtmpTransport, RtmpTransportConfig};
pub use rtsp::{RtspPushError, RtspTransport, RtspTransportConfig};
pub(crate) use srt::validate_srt_url;
pub use srt::{SrtTransport, SrtTransportConfig};

/// Error from the push egress's framing/write path — distinguishes a muxing
/// failure (not a connection problem: `drive_push` drops the sample without
/// forcing a reconnect) from a transport-level send/protocol failure
/// (`drive_push` reconnects).
///
/// Generic over no transport type — every [`PushTransport::Error`] already
/// requires `std::error::Error + Send + Sync + 'static` (the trait bound),
/// so it boxes into `Transport` uniformly regardless of which transport
/// raised it.
#[non_exhaustive]
#[derive(Debug, thiserror::Error)]
pub enum SendMediaError {
    /// Muxing/framing the sample failed — no connection to blame, so it is
    /// dropped without forcing a reconnect.
    #[error("push mux failed: {0}")]
    Mux(#[from] MuxError),
    /// The transport's own send/protocol call failed.
    #[error("push send failed: {0}")]
    Transport(Box<dyn std::error::Error + Send + Sync>),
}

impl PushErrorKind for SendMediaError {
    fn kind(&self) -> PushErrorClass {
        match self {
            SendMediaError::Mux(_) => PushErrorClass::DropItem,
            SendMediaError::Transport(_) => PushErrorClass::Reconnect,
        }
    }
}

/// Why framing a sample failed: the muxer's own error, or a track/codec the
/// transport cannot carry at all.
#[non_exhaustive]
#[derive(Debug, thiserror::Error)]
pub enum MuxError {
    /// The TS muxer refused the sample.
    #[error(transparent)]
    Package(#[from] transmux::Error),
    /// The transport's own write failed.
    #[error(transparent)]
    Transport(Box<dyn std::error::Error + Send + Sync>),
    /// The FLV framing an RTMP push needs refused the batch.
    #[error(transparent)]
    Flv(#[from] transmux::FlvError),
    /// The RTMP framing an RTMP push needs refused the batch.
    #[error(transparent)]
    Rtmp(#[from] transmux::RtmpFlvError),
    /// The transport carries none of the offered codecs.
    #[error("no track is carriable on this transport")]
    NoCarriableTrack,
    /// The transport has no per-connection muxer to frame a sample through.
    #[error("the transport has no per-connection muxer")]
    NoMuxer,
}

/// One per-connection sample muxer, as a transport hands it out.
///
/// Both per-sample muxers the push transports own — a TS push's
/// [`TsMuxStage`] and RTMP's [`transmux::RtmpMuxStage`] — speak
/// [`MuxInput`]/[`MuxOutput`], so this trait is their one object-safe view:
/// [`PushTransport::media_muxer`] returns it and
/// [`egress::PushTransportEgress`] is its one driver (announce the negotiated
/// track set, feed one drained sample at a time, flush at an idle boundary).
///
/// The state lives and dies with the transport, so a reconnect (a new
/// transport, a new [`PushTransportEgress`]) starts a fresh stream with fresh
/// PSI, continuity counters and chunk-stream state.
pub trait MediaMuxer {
    /// Announce (or update) the track set this connection carries; an
    /// identical set is a no-op.
    ///
    /// # Errors
    /// The muxer's own error when the set cannot be added to its stream.
    fn set_tracks(&mut self, tracks: &[TrackSpec]) -> Result<(), SendMediaError>;

    /// Mux one drained sample, returning every wire message it completed.
    ///
    /// # Errors
    /// The muxer's own error.
    fn encode_sample(
        &mut self,
        track: TrackId,
        sample: &Sample,
    ) -> Result<Vec<bytes::Bytes>, SendMediaError>;

    /// Emit every message the muxer still holds because the stream went idle
    /// or ended (an interleave window, or one pending sample per track).
    ///
    /// # Errors
    /// The muxer's own error.
    fn flush(&mut self) -> Result<Vec<bytes::Bytes>, SendMediaError>;

    /// The source timeline broke before the next sample: frame the transport's
    /// own in-stream marker (a TS `discontinuity_indicator`, an RTMP timestamp
    /// rebase) and return every wire message that completes.
    ///
    /// # Errors
    /// The muxer's own error.
    fn discontinuity(&mut self) -> Result<Vec<bytes::Bytes>, SendMediaError>;
}

/// The per-connection MPEG-2 TS muxer a TS push transport feeds one sample at
/// a time (issue #1024, Phase 4): one [`TsMuxStage`] carrying continuity
/// counters, the PCR and the PSI schedule across samples, and the track set it
/// last announced.
pub(crate) struct TsPushMux {
    stage: TsMuxStage,
    announced: Vec<TrackSpec>,
}

impl TsPushMux {
    fn new() -> Self {
        Self {
            stage: TsMuxStage::new(Some(DEFAULT_PSI_INTERVAL)),
            announced: Vec::new(),
        }
    }

    fn take_ready(&mut self) -> Vec<bytes::Bytes> {
        let mut out = Vec::new();
        while let Some(item) = self.stage.poll() {
            if let MuxOutput::Bytes(chunk) = item {
                out.push(chunk);
            }
        }
        out
    }
}

impl MediaMuxer for TsPushMux {
    fn set_tracks(&mut self, tracks: &[TrackSpec]) -> Result<(), SendMediaError> {
        if self.announced == tracks {
            return Ok(());
        }
        for spec in tracks {
            self.stage
                .feed(MuxInput::Track(Box::new(spec.clone())), Timestamp::ZERO)
                .map_err(MuxError::Package)?;
        }
        self.announced = tracks.to_vec();
        Ok(())
    }

    /// Feed one sample and collect every whole TS packet the stage emits.
    fn encode_sample(
        &mut self,
        track: TrackId,
        sample: &Sample,
    ) -> Result<Vec<bytes::Bytes>, SendMediaError> {
        self.stage
            .feed(
                MuxInput::Sample {
                    track,
                    sample: sample.clone(),
                },
                Timestamp::ZERO,
            )
            .map_err(MuxError::Package)?;
        Ok(self.take_ready())
    }

    /// Close the unit being built now: emit the packets the stage still holds
    /// (its interleave window waits for the next PCR-carrying sample, so an
    /// idle stream would otherwise leave the last few frames queued).
    fn flush(&mut self) -> Result<Vec<bytes::Bytes>, SendMediaError> {
        self.stage
            .feed(MuxInput::Flush, Timestamp::ZERO)
            .map_err(MuxError::Package)?;
        Ok(self.take_ready())
    }

    /// Mark the next packet of every carried PID with the adaptation field's
    /// `discontinuity_indicator` (ISO/IEC 13818-1 §2.4.3.5).
    fn discontinuity(&mut self) -> Result<Vec<bytes::Bytes>, SendMediaError> {
        self.stage
            .feed(MuxInput::Discontinuity { track: None }, Timestamp::ZERO)
            .map_err(MuxError::Package)?;
        Ok(self.take_ready())
    }
}

/// A concrete push transport: the wire protocol half of a push output.
///
/// Mirrors the ingest side's *socket-as-handle* shape (e.g.
/// [`srt_runtime::io::SrtSocket`]) — [`connect`](Self::connect) dials out to
/// the downstream server, [`media_muxer`](Self::media_muxer) hands out the
/// connection's per-sample muxer, and [`close`](Self::close) tears the
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
    /// The low-level "write these bytes" primitive.
    /// **Its contract is "frame `data`
    /// as this transport's own idea of one payload"**, not "write these
    /// bytes verbatim" — for SRT/RTSP that's the same thing (there is no
    /// separate framing step), but RTMP's override always wraps `data` in a
    /// *new* `send_video` chunk-stream message (issue #934: distinct typed
    /// messages per elementary stream), so it must never be called with
    /// bytes the connection muxer already framed — see
    /// [`write_message`](Self::write_message) for that case.
    async fn send(&mut self, data: &[u8]) -> Result<(), Self::Error>;

    /// Write one already wire-ready message **verbatim** — no framing, no
    /// encoding, just bytes to the socket (issue #942). This is what
    /// [`egress::PushTransportEgress::flush_transmit`] calls to actually
    /// transmit what [`media_muxer`](Self::media_muxer) produced, and it is
    /// deliberately a *different* method from [`send`](Self::send): for
    /// SRT/RTSP the default forwarding to `send` is correct (their `send` is
    /// already a raw write, no framing side effect), but RTMP's `send` is
    /// not — its whole job is to wrap `data` in a fresh chunk-stream
    /// message, which would double-frame a message the muxer already
    /// produced (RTMP overrides this instead).
    async fn write_message(&mut self, message: &[u8]) -> Result<(), Self::Error> {
        self.send(message).await
    }

    /// Optional session setup before the first `send` — e.g.
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
    /// `true`: MPEG-2 TS carries essentially any `transmux::CodecConfig`
    /// SRT/RTSP push hand it, opaque PES included — RTMP is the one transport
    /// whose wire container (FLV) has a real, narrow codec set and overrides
    /// this.
    fn supports_codec(&self, _config: &transmux::CodecConfig) -> bool {
        true
    }

    /// The per-connection muxer this transport frames with: a `TsPushMux` for
    /// the TS pushes (SRT, RTSP), `Some` once `setup` created it for RTMP.
    /// `None` means the connection has no muxer yet (`setup` has not run), so
    /// there is nothing to frame a sample through.
    ///
    /// [`egress::PushTransportEgress`] is the one driver: it announces the
    /// negotiated track set, feeds one drained sample at a time and flushes at
    /// an idle boundary, so no push path builds a whole batch.
    fn media_muxer(&mut self) -> Option<&mut dyn MediaMuxer> {
        None
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

/// How long a push connection must stay up before its end counts as a
/// healthy session: the workspace's one value, shared with every other
/// reconnect (issue #2471).
pub const HEALTHY_CONNECTION: Duration = Duration::from_secs(30);

/// A zero schedule (empty steps, zero cap) an impossible schedule falls back
/// to: the one `RetrySchedule::from_steps` never rejects (a zero cap is
/// always under the schedule's ceiling).
fn zero_schedule(seed: u64) -> broadcast_common::backoff::RetrySchedule {
    broadcast_common::backoff::RetrySchedule::from_steps(&[], Duration::ZERO, seed)
        .expect("an empty zero-cap schedule is always accepted")
}

/// The reconnect engine: [`ReconnectState`] + the attempt budget + the
/// workspace's one reconnect schedule (`broadcast_common::backoff::RetrySchedule`,
/// issue #2471 — the same rule the origin supervisor uses).
#[derive(Debug, Clone)]
pub struct ReconnectEngine {
    state: ReconnectState,
    attempt: u32,
    schedule: broadcast_common::backoff::RetrySchedule,
    policy: crate::config::ReconnectPolicy,
    /// The instant the current connection came up, for the healthy-connection
    /// rule (a connection that dropped a moment later keeps counting up).
    connected_at: Option<Instant>,
    /// A zero policy waits nothing at all, not one millisecond: a push that
    /// has asked for no backoff should not pay a timer wait on every retry.
    backoff_disabled: bool,
}

impl ReconnectEngine {
    /// Start in [`ReconnectState::Ready`] (nothing to reconnect to yet).
    ///
    /// A policy whose schedule the shared
    /// [`RetrySchedule`](broadcast_common::backoff::RetrySchedule) rejects (a cap
    /// past its ceiling) falls back to a zero schedule — never on a validated
    /// config, so this stays infallible rather than panicking or returning an
    /// error the push loop has nowhere to route.
    pub fn new(policy: crate::config::ReconnectPolicy) -> Self {
        let schedule = policy
            .backoff
            .schedule(crate::source::seed_entropy())
            .unwrap_or_else(|_| zero_schedule(crate::source::seed_entropy()));
        let backoff_disabled =
            policy.backoff.steps.iter().all(|d| d.is_zero()) && policy.backoff.cap.is_zero();
        Self {
            state: ReconnectState::Ready,
            attempt: 0,
            schedule,
            policy,
            connected_at: None,
            backoff_disabled,
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

    /// A connection was established. The attempt counter is not reset here:
    /// it restarts only once the connection has stayed up for
    /// [`HEALTHY_CONNECTION`] (see [`Self::on_disconnect`]).
    pub fn on_connect(&mut self) {
        self.on_connect_at(Instant::now());
    }

    fn on_connect_at(&mut self, now: Instant) {
        self.connected_at = Some(now);
        self.state = ReconnectState::Ready;
    }

    /// The connection dropped (or a dial-out failed) — enter backoff,
    /// increment the attempt counter, and go permanently [`Failed`](ReconnectState::Failed) once
    /// `max_attempts` is exceeded.
    ///
    /// Backoff follows the policy's schedule (its step table, then the cap)
    /// and jittered.
    ///
    /// A connection that had stayed up for [`HEALTHY_CONNECTION`] first
    /// restarts the series from its floor.
    pub fn on_disconnect(&mut self) {
        self.on_disconnect_at(Instant::now());
    }

    fn on_disconnect_at(&mut self, now: Instant) {
        let healthy = self
            .connected_at
            .take()
            .is_some_and(|since| now.saturating_duration_since(since) >= HEALTHY_CONNECTION);
        let wait = if self.backoff_disabled {
            Duration::ZERO
        } else if healthy {
            self.attempt = 0;
            self.schedule.delay_for_attempt(0)
        } else {
            let attempt = self.attempt;
            self.schedule.delay_for_attempt(attempt)
        };
        self.attempt = self.attempt.saturating_add(1);
        if self.policy.exhausted(self.attempt) {
            self.state = ReconnectState::Failed;
            return;
        }
        self.state = ReconnectState::Backoff {
            resume_at: now + wait,
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
/// `NegotiationOutcome::Accepted` carries whenever its `TrackSelection`
/// names fewer tracks than were proposed — the structured replacement for
/// `push::rtmp::RtmpTransport`'s old `warned_refused_tracks` flag (issue
/// #942): the selection itself is the "which tracks, and how many were
/// excluded" fact, so there is nothing left to track privately inside the
/// transport.
fn log_partial_selection(url: &str, carried: usize, proposed: usize) {
    if carried < proposed {
        tracing::warn!(
            url = %url,
            carried,
            proposed,
            "push output cannot carry every track in this program; excluded tracks are \
             dropped from this push"
        );
    }
}

fn close_connected<T: PushTransport>(session: &mut PushSession<PushTransportEgress<T>>) {
    if let Some(egress) = session.egress_mut() {
        egress.transport_mut().close();
    }
}

fn retire_egress<T: PushTransport>(mut egress: PushTransportEgress<T>, written: &mut u64) {
    *written = written.saturating_add(egress.bytes_sent());
    egress.transport_mut().close();
}

fn flush_result(outcome: &FlushOutcome) -> FlushResult {
    match outcome {
        FlushOutcome::Flushed => FlushResult::Flushed,
        FlushOutcome::Cancelled => FlushResult::Cancelled,
        FlushOutcome::TimedOut => FlushResult::TimedOut,
        FlushOutcome::Failed(_) => FlushResult::Failed,
    }
}

fn publish_metrics<T: PushTransport>(
    metrics: PushMetrics,
    session: &mut PushSession<PushTransportEgress<T>>,
    written: u64,
) -> PushMetrics {
    let counters = session.counters();
    metrics
        .samples_sent
        .store(counters.samples_sent, Ordering::Relaxed);
    metrics
        .samples_dropped
        .store(counters.samples_dropped, Ordering::Relaxed);
    metrics
        .reconnect_count
        .store(counters.connects, Ordering::Relaxed);
    let connected = session.egress_mut().map_or(0, |egress| egress.bytes_sent());
    metrics
        .bytes_sent
        .store(written.saturating_add(connected), Ordering::Relaxed);
    metrics
}

/// The main push loop: subscribe to `trunk`'s sample ring and push its
/// samples to the downstream server over `T`, reconnecting on failure per
/// `reconnect` (issue #744).
///
/// Issue #942: routes every sample through a [`PushTransportEgress<T>`]
/// (`media_plane::PushEgress` over `T`) rather than muxing/sending directly.
/// Every decision is the [`PushSession`]'s (`media_plane::push_session`): this
/// loop only performs the awaits it cannot — connect, setup, the bounded
/// flush, backoff — and logs/redacts/measures what the decisions report.
///
/// Returns the [`PushMetrics`] accumulated for the lifetime of this task.
/// Cancellation is cooperative via `cancel` — the loop checks it between
/// drain steps (never mid-`send`).
#[cfg(test)]
static DRIVE_PUSH_TURNS: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

pub async fn drive_push<T: PushTransport>(
    trunk: Arc<Trunk>,
    url: String,
    config: T::Config,
    _format: PushFormat,
    reconnect: crate::config::ReconnectPolicy,
    cancel: CancellationToken,
) -> PushMetrics {
    // The destination as it may appear in a log line: a push URL carries
    // credentials in its userinfo and a stream key in its path/query, none of
    // which may reach the logs (audit T14, #1142).
    let log_url = crate::redact::redact_destination(&url);
    // Transport errors may echo what they were asked to dial; scrub every
    // secret derived from the URL out of their text before logging.
    let scrub = |err: &dyn std::fmt::Display| {
        crate::redact::scrub_destination_secrets(&err.to_string(), &url)
    };
    /// The longest one flush of queued messages may take before the push is
    /// treated as failed and reconnected. A transport write can block on a
    /// live peer that stopped consuming (SRT's `send` waits for flow-window
    /// room), which must neither wedge the task nor outlive a cancel.
    const PUSH_FLUSH_TIMEOUT: Duration = Duration::from_secs(10);

    let metrics = PushMetrics::default();
    let mut session: PushSession<PushTransportEgress<T>> = PushSession::new();
    let mut engine = ReconnectEngine::new(reconnect);
    let mut cursor = trunk.subscribe();
    // Bytes egresses the session has handed back wrote before they were
    // closed; the connected egress's own count is added on top.
    let mut written_bytes = 0u64;
    // `Trunk` samples are already `transmux::Sample`s; the track set we mux
    // against comes straight from the trunk. `_format` is unused here — it
    // only ever selects TS (the only implemented `PushFormat`); `Mp4`/`Mkv`
    // are rejected at config-validate time (`crate::config::Route::
    // validate_standalone`), so by the time a push task runs, `_format` is
    // guaranteed to be `Ts`.
    let _ = _format;

    loop {
        #[cfg(test)]
        DRIVE_PUSH_TURNS.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        if cancel.is_cancelled() {
            close_connected(&mut session);
            return publish_metrics(metrics, &mut session, written_bytes);
        }

        // Wait for samples, or a bounded wake-up, before draining — the
        // 250 ms cap keeps cancellation / track-set changes noticed. Both
        // arms are real `.await`s so this never parks the runtime worker, and
        // the whole wait is raced against `cancel` so a shutdown returns
        // promptly rather than after the next tick (defect 5).
        tokio::select! {
            () = cancel.cancelled() => {
                close_connected(&mut session);
                return publish_metrics(metrics, &mut session, written_bytes);
            }
            () = async {
                match trunk.listen() {
                    Some(listener) => {
                        let _ = tokio::time::timeout(TRUNK_WAKE_CAP, listener).await;
                    }
                    // No free waiter slot: park until one frees — never a
                    // fixed 50 ms sleep-poll (B10b). Still capped at the same
                    // 250 ms as the `Some` arm so a slot that frees between
                    // `listen()` returning `None` and `waiter_slot_freed()`
                    // registering (or a lost wake for any reason) cannot stall
                    // the push — and its queued flush/renegotiate — forever on
                    // an idle source.
                    None => {
                        if let Some(freed) = trunk.waiter_slot_freed() {
                            let _ = tokio::time::timeout(TRUNK_WAKE_CAP, freed).await;
                        }
                    }
                }
            } => {}
        }

        while let Some(item) = cursor.poll() {
            match session.on_item(&item) {
                PushDecision::Continue => {}
                PushDecision::Renegotiated { carried, proposed } => {
                    log_partial_selection(&log_url, carried, proposed);
                }
                PushDecision::RenegotiateRefused { reason } => {
                    tracing::warn!(
                        url = %log_url,
                        reason,
                        "push renegotiate refused; continuing on the previous track \
                         selection"
                    );
                }
                PushDecision::MuxDropped(error) => {
                    tracing::error!(error = %error, "push mux failed; dropping sample");
                }
                PushDecision::Reconnect { egress, cause } => {
                    match &cause {
                        ReconnectCause::Send(err) => {
                            tracing::warn!(url = %log_url, error = %scrub(err), "push send failed; reconnecting");
                        }
                        ReconnectCause::Renegotiate(err) => {
                            tracing::warn!(url = %log_url, error = %scrub(err), "push renegotiate failed; closing");
                        }
                        _ => {}
                    }
                    retire_egress(egress, &mut written_bytes);
                    engine.on_disconnect();
                }
                _ => {}
            }
        }

        if engine.is_failed() {
            // Drained the cursor above; give up permanently.
            if let PushDecision::Stop { egress } = session.on_give_up()
                && let Some(egress) = egress
            {
                retire_egress(egress, &mut written_bytes);
            }
            return publish_metrics(metrics, &mut session, written_bytes);
        }

        // Connect / reconnect when due and not already connected.
        if engine.should_connect()
            && !session.is_connected()
            && session.pending_setup_mut().is_none()
        {
            let connect_result = tokio::select! {
                () = cancel.cancelled() => {
                    // Cancelled mid-connect (defect 5): do not wait out a
                    // transport connect that may block for its own timeout.
                    close_connected(&mut session);
                    return publish_metrics(metrics, &mut session, written_bytes);
                }
                result = T::connect(&url, &config) => result,
            };
            match connect_result {
                Ok(conn) => {
                    let (generation, tracks_now) = trunk.tracks_with_generation();
                    match session.on_connected(
                        PushTransportEgress::new(conn),
                        generation,
                        &tracks_now,
                    ) {
                        PushDecision::NeedsSetup => {
                            let setup = match session.pending_setup_mut() {
                                Some(pending) => {
                                    let selected = pending.selected_tracks().to_vec();
                                    Some(pending.transport_mut().setup(&selected).await)
                                }
                                None => None,
                            };
                            match setup {
                                Some(Ok(())) => {
                                    if let PushDecision::Connected { carried, proposed } =
                                        session.on_setup(true)
                                    {
                                        log_partial_selection(&log_url, carried, proposed);
                                        engine.on_connect();
                                    }
                                }
                                Some(Err(err)) => {
                                    tracing::warn!(url = %log_url, error = %scrub(&err), "push setup failed; closing");
                                    if let PushDecision::Reconnect { egress, .. } =
                                        session.on_setup(false)
                                    {
                                        retire_egress(egress, &mut written_bytes);
                                        engine.on_disconnect();
                                    }
                                }
                                None => {}
                            }
                        }
                        PushDecision::Backoff { egress, cause } => {
                            if let Some(err) = cause {
                                tracing::warn!(url = %log_url, error = %scrub(&err), "push negotiate failed; backing off");
                            }
                            retire_egress(egress, &mut written_bytes);
                            engine.on_disconnect();
                        }
                        PushDecision::Redundant { egress } => {
                            retire_egress(egress, &mut written_bytes);
                        }
                        _ => {}
                    }
                }
                Err(e) => {
                    tracing::warn!(url = %log_url, error = %scrub(&e), "push connect failed; backing off");
                    engine.on_disconnect();
                }
            }
        }

        // Flush whatever `send` queued this iteration (the async half —
        // see `PushTransportEgress::flush_transmit`'s own doc); otherwise
        // sleep out the backoff (cancellation-aware).
        if let Some(egress) = session.egress_mut() {
            // Emit the packets the per-connection muxer still holds: a muxer
            // keeps one interleave window, so the tail of an idle stream would
            // otherwise wait for the next sample.
            if let Err(error) = egress.flush_mux().await {
                tracing::error!(error = %error, "push mux flush failed; continuing");
            }
            let outcome = egress
                .flush_transmit_bounded(&cancel, PUSH_FLUSH_TIMEOUT)
                .await;
            match &outcome {
                FlushOutcome::Failed(err) => {
                    tracing::warn!(url = %log_url, error = %scrub(err), "push send failed while flushing; reconnecting");
                }
                FlushOutcome::TimedOut => {
                    tracing::warn!(url = %log_url, timeout = ?PUSH_FLUSH_TIMEOUT, "push send stalled; reconnecting");
                }
                _ => {}
            }
            match session.on_flush(flush_result(&outcome)) {
                PushDecision::Reconnect { egress, .. } => {
                    retire_egress(egress, &mut written_bytes);
                    engine.on_disconnect();
                }
                PushDecision::Stop { egress } => {
                    if let Some(egress) = egress {
                        retire_egress(egress, &mut written_bytes);
                    }
                    return publish_metrics(metrics, &mut session, written_bytes);
                }
                _ => {}
            }
        } else {
            let wait = engine.time_until_retry();
            if !wait.is_zero() {
                // Cancel-aware backoff (defect 5): a shutdown during the wait
                // must return promptly, not sit out the whole retry interval.
                tokio::select! {
                    () = cancel.cancelled() => {
                        close_connected(&mut session);
                        return publish_metrics(metrics, &mut session, written_bytes);
                    }
                    () = sleep(wait) => {}
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::ReconnectPolicy;
    use media_plane::trunk::TrunkConfig;

    fn policy(max_attempts: Option<std::num::NonZeroU32>) -> ReconnectPolicy {
        let mut policy = ReconnectPolicy::default();
        if let Some(max) = max_attempts {
            policy = policy.with_max_attempts(max);
        }
        policy
    }

    /// A [`PushTransport`] whose `connect` always fails immediately (no real
    /// I/O, no `.await` suspension) — used to keep `drive_push`'s loop body
    /// free of any *other* async wait, so the test below isolates exactly
    /// the `trunk.listen() == None` branch.
    struct AlwaysFailsTransport;

    #[derive(Debug, thiserror::Error)]
    #[error("connect always fails")]
    struct AlwaysFailsError;

    #[async_trait::async_trait]
    impl PushTransport for AlwaysFailsTransport {
        type Config = ();
        type Error = AlwaysFailsError;

        async fn connect(_url: &str, _config: &Self::Config) -> Result<Self, Self::Error> {
            Err(AlwaysFailsError)
        }

        async fn send(&mut self, _data: &[u8]) -> Result<(), Self::Error> {
            Ok(())
        }

        fn close(&mut self) {}
    }

    fn nz(n: usize) -> std::num::NonZeroUsize {
        std::num::NonZeroUsize::new(n).unwrap()
    }

    /// With every `Trunk` listener slot already held, `drive_push` must not
    /// starve sibling tasks on a `current_thread` runtime.
    ///
    /// A sole waiter slot is held for the whole test so `trunk.listen()`
    /// always returns `None` inside `drive_push`; `AlwaysFailsTransport`
    /// keeps every other step in the loop free of a real `.await`
    /// suspension (zero backoff means the retry-wait is skipped too), so
    /// the only possible yield point is the `None` branch itself.
    ///
    /// Runs under `start_paused` with NO real-time sleep: virtual time only
    /// advances when the runtime is otherwise idle, so the sibling `yield_now`
    /// spin below can only make progress if `drive_push` actually returns
    /// `Poll::Pending` at the `None` branch. Cancelling afterwards returns the
    /// push.
    ///
    /// PRE-FIX FAILURE OBSERVED: `sibling_progress.load() == 0` — the
    /// counter task was never scheduled even once, because the old code's
    /// `if let Some(listener) = trunk.listen() { .. }` is a no-op when
    /// `listen()` returns `None`, and every other step (failed connect, zero
    /// backoff) resolves without a real `.await` suspension, so `drive_push`'s
    /// task never returns `Poll::Pending` and the `current_thread` executor
    /// never gets to run the sibling task.
    #[tokio::test(start_paused = true, flavor = "current_thread")]
    async fn drive_push_yields_when_no_listener_slot_is_free() {
        let trunk = Trunk::new(
            TrunkConfig::default()
                .with_timed_capacity(nz(1))
                .with_sparse_capacity(nz(1))
                .with_segment_capacity(nz(1))
                .with_event_capacity(nz(1))
                .with_part_capacity(nz(1))
                .with_waiter_capacity(nz(1)),
        );
        let _held_slot = trunk.listen().expect("the sole waiter slot is free here");

        let sibling_progress = Arc::new(AtomicU64::new(0));
        let sibling_progress_task = Arc::clone(&sibling_progress);
        let sibling = tokio::spawn(async move {
            loop {
                sibling_progress_task.fetch_add(1, Ordering::Relaxed);
                tokio::task::yield_now().await;
            }
        });

        let cancel = CancellationToken::new();
        let reconnect = ReconnectPolicy::doubling(Duration::ZERO, Duration::ZERO);
        let push = tokio::spawn(drive_push::<AlwaysFailsTransport>(
            trunk,
            "push://unreachable".to_string(),
            (),
            PushFormat::Ts,
            reconnect,
            cancel.clone(),
        ));

        // Let both tasks be polled repeatedly. Virtual time does NOT advance
        // here, so a `drive_push` that never yields leaves the sibling at 0.
        for _ in 0..50 {
            tokio::task::yield_now().await;
        }
        assert!(
            sibling_progress.load(Ordering::Relaxed) > 10,
            "sibling task starved while every listener slot was held: {}",
            sibling_progress.load(Ordering::Relaxed)
        );

        cancel.cancel();
        sibling.abort();
        let _ = tokio::time::timeout(Duration::from_secs(2), push).await;
    }

    /// The default policy's step table doubles (1s, 2s, 4s, 8s, 16s) then caps
    /// at 30 s. The band holds across every attempt, and the saturated tail
    /// stays jittered in `[cap*5/6, cap]` rather than collapsing to the cap.
    #[test]
    fn backoff_doubles_each_attempt_capped_at_max() {
        let eng = ReconnectEngine::new(policy(None));
        let schedule = eng.policy.backoff.schedule(0x5EED).unwrap();
        let raw = |attempt: u32| {
            eng.policy.backoff.cap.min(
                eng.policy
                    .backoff
                    .steps
                    .get(attempt as usize)
                    .copied()
                    .unwrap_or(eng.policy.backoff.cap),
            )
        };
        for attempt in 0..40u32 {
            assert!(
                raw(attempt) >= Duration::from_secs(1) && raw(attempt) <= Duration::from_secs(30),
                "attempt {attempt}: {:?} escaped [1s, 30s]",
                raw(attempt)
            );
            assert!(schedule.delay_for_attempt(attempt) <= Duration::from_secs(30));
        }
        // At the cap the delay stays jittered in [25s, 30s], never a single
        // constant (a jitter re-clamped to the cap would collapse it to 30s).
        let capped = broadcast_common::backoff::RetrySchedule::new(
            Duration::from_secs(1),
            Duration::from_secs(30),
            2.0,
            0x5EED,
        )
        .unwrap()
        .with_offset(1);
        let mut distinct = std::collections::HashSet::new();
        for _ in 0..40 {
            let d = capped.delay_for_attempt(20);
            assert!(
                d >= Duration::from_millis(25_000) && d <= Duration::from_millis(30_000),
                "capped backoff {d:?} must stay jittered in [25s, 30s]"
            );
            distinct.insert(d.as_nanos());
        }
        assert!(distinct.len() > 1, "zero jitter at the cap");
    }

    #[test]
    fn max_attempts_triggers_failed() {
        let mut eng = ReconnectEngine::new(policy(std::num::NonZeroU32::new(3)));
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
    fn on_connect_returns_a_failed_engine_to_ready() {
        let mut eng = ReconnectEngine::new(policy(std::num::NonZeroU32::new(2)));
        eng.on_disconnect();
        eng.on_disconnect();
        assert!(eng.is_failed());
        eng.on_connect();
        assert!(!eng.is_failed());
        assert!(eng.should_connect());
    }

    /// A connection that stayed up for [`HEALTHY_CONNECTION`] first resets
    /// the attempt counter (the same rule the origin supervisor follows, from
    /// the same shared timer), so a series of short outages after a healthy
    /// session always waits the first step again.
    #[test]
    fn a_connection_that_stayed_up_restarts_the_backoff_series() {
        let mut eng = ReconnectEngine::new(policy(None));
        let healthy = Instant::now();
        eng.on_connect_at(healthy);
        eng.on_disconnect_at(healthy + HEALTHY_CONNECTION);
        assert_eq!(
            eng.attempt, 1,
            "a healthy connection starts the series over"
        );
        let ReconnectState::Backoff { resume_at } = eng.state else {
            panic!("a disconnect must arm a wait: {:?}", eng.state);
        };
        assert!(resume_at > healthy, "a wait is armed: {resume_at:?}");
    }

    /// A connection that drops at once is not healthy: the attempt counter
    /// keeps climbing, which is what a broken destination must do.
    #[test]
    fn a_connection_that_drops_at_once_keeps_counting() {
        let mut eng = ReconnectEngine::new(policy(std::num::NonZeroU32::new(3)));
        let mut at = Instant::now();
        for expected in 1..=3u32 {
            at += Duration::from_millis(1);
            eng.on_connect_at(at);
            at += Duration::from_millis(1);
            eng.on_disconnect_at(at);
            assert_eq!(eng.attempt, expected);
        }
        assert!(eng.is_failed(), "the attempt budget ran out");
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

    /// A destination that refuses every dial is re-dialled only after the
    /// backoff has been slept out: with a live sample stream keeping every
    /// loop wake-up immediate, the loop turns over a handful of times
    /// between two dials instead of spinning for the whole backoff (the
    /// engine still refuses to *dial* early, so dial spacing alone cannot
    /// see the difference).
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_failing_push_sleeps_out_its_backoff() {
        use std::sync::Mutex;
        use std::sync::atomic::Ordering;

        static DIALS: Mutex<Vec<(std::time::Instant, u64)>> = Mutex::new(Vec::new());

        struct CountingFailsTransport;

        #[derive(Debug, thiserror::Error)]
        #[error("connect always fails")]
        struct CountingFailsError;

        #[async_trait::async_trait]
        impl PushTransport for CountingFailsTransport {
            type Config = ();
            type Error = CountingFailsError;

            async fn connect(_url: &str, _config: &Self::Config) -> Result<Self, Self::Error> {
                DIALS
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner)
                    .push((
                        std::time::Instant::now(),
                        DRIVE_PUSH_TURNS.load(Ordering::Relaxed),
                    ));
                Err(CountingFailsError)
            }

            async fn send(&mut self, _data: &[u8]) -> Result<(), Self::Error> {
                Ok(())
            }

            fn close(&mut self) {}
        }

        DIALS
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clear();
        DRIVE_PUSH_TURNS.store(0, Ordering::Relaxed);
        let trunk = Trunk::new(TrunkConfig::default().with_segment_capacity(nz(64)));
        // A live sample stream: every `trunk.listen()` resolves at once, so
        // the loop's own 250 ms wake-up cap is not what paces it.
        let publish_cancel = CancellationToken::new();
        let publisher = {
            let trunk = Arc::clone(&trunk);
            let publish_cancel = publish_cancel.clone();
            tokio::spawn(async move {
                let writer = trunk.segment_writer().expect("segment writer");
                let mut seq = 1u32;
                while !publish_cancel.is_cancelled() {
                    if writer
                        .publish_segment(media_plane::trunk::SegmentEntry::new(
                            bytes::Bytes::from(vec![0u8; 8]),
                            media_plane::ids::SegmentSeq::new(seq),
                            Duration::from_millis(5),
                            broadcast_common::Timestamp::from_nanos(u64::from(seq) * 5_000_000),
                            false,
                        ))
                        .is_err()
                    {
                        break;
                    }
                    seq = seq.saturating_add(1);
                    tokio::time::sleep(Duration::from_millis(5)).await;
                }
            })
        };

        let cancel = CancellationToken::new();
        let reconnect =
            ReconnectPolicy::doubling(Duration::from_millis(300), Duration::from_secs(30));
        let run = tokio::spawn(drive_push::<CountingFailsTransport>(
            Arc::clone(&trunk),
            "srt://127.0.0.1:1/live".to_string(),
            (),
            PushFormat::Ts,
            reconnect,
            cancel.clone(),
        ));

        let deadline = std::time::Instant::now() + Duration::from_secs(10);
        loop {
            let dials = DIALS
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .len();
            if dials >= 2 {
                break;
            }
            assert!(
                std::time::Instant::now() < deadline,
                "the push engine did not re-dial within the guard"
            );
            tokio::task::yield_now().await;
        }
        let (gap, turns) = {
            let dials = DIALS
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            (
                dials[1].0.duration_since(dials[0].0),
                dials[1].1.saturating_sub(dials[0].1),
            )
        };
        cancel.cancel();
        tokio::time::timeout(Duration::from_millis(100), run)
            .await
            .expect("a cancel during the backoff must return promptly")
            .expect("driver task");
        publish_cancel.cancel();
        publisher.abort();

        assert!(
            gap >= Duration::from_millis(250),
            "the two dials were {gap:?} apart, inside the 300 ms backoff floor"
        );
        assert!(
            turns <= 10,
            "the loop turned {turns} times between two dials while sleeping out a 300 ms \
             backoff (a skipped wait spins instead of sleeping)"
        );
    }

    /// A connection whose connect-time negotiation fails (a trunk with no
    /// carriable track) is closed before the backoff, not dropped unclosed:
    /// the session hands the egress back in `PushDecision::Backoff`.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_failed_negotiation_closes_the_connection() {
        static DIALS: AtomicU64 = AtomicU64::new(0);
        static CLOSES: AtomicU64 = AtomicU64::new(0);

        struct ConnectsTransport;

        #[derive(Debug, thiserror::Error)]
        #[error("never fails")]
        struct NeverFails;

        #[async_trait::async_trait]
        impl PushTransport for ConnectsTransport {
            type Config = ();
            type Error = NeverFails;

            async fn connect(_url: &str, _config: &Self::Config) -> Result<Self, Self::Error> {
                DIALS.fetch_add(1, Ordering::Relaxed);
                Ok(ConnectsTransport)
            }

            async fn send(&mut self, _data: &[u8]) -> Result<(), Self::Error> {
                Ok(())
            }

            fn close(&mut self) {
                CLOSES.fetch_add(1, Ordering::Relaxed);
            }
        }

        let trunk = Trunk::new(TrunkConfig::default());
        let cancel = CancellationToken::new();
        let reconnect =
            ReconnectPolicy::doubling(Duration::from_millis(10), Duration::from_millis(10));
        let run = tokio::spawn(drive_push::<ConnectsTransport>(
            trunk,
            "srt://127.0.0.1:1/live".to_string(),
            (),
            PushFormat::Ts,
            reconnect,
            cancel.clone(),
        ));

        let deadline = std::time::Instant::now() + Duration::from_secs(10);
        while DIALS.load(Ordering::Relaxed) < 3 {
            assert!(
                std::time::Instant::now() < deadline,
                "the push did not re-dial within the guard"
            );
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
        cancel.cancel();
        tokio::time::timeout(Duration::from_secs(2), run)
            .await
            .expect("cancel returns promptly")
            .expect("driver task");

        assert!(
            CLOSES.load(Ordering::Relaxed) >= 2,
            "{} dials but {} closes: a connection that failed negotiation was dropped unclosed",
            DIALS.load(Ordering::Relaxed),
            CLOSES.load(Ordering::Relaxed)
        );
    }
    /// A partial selection is reported once, with the carried and proposed
    /// counts — the structured successor to the deleted per-transport
    /// refused-track warning.
    #[test]
    fn a_partial_selection_is_logged_once_with_the_excluded_count() {
        #[derive(Clone, Default)]
        struct Capture(std::sync::Arc<std::sync::Mutex<String>>);

        impl tracing::Subscriber for Capture {
            fn enabled(&self, _: &tracing::Metadata<'_>) -> bool {
                true
            }
            fn new_span(&self, _: &tracing::span::Attributes<'_>) -> tracing::span::Id {
                tracing::span::Id::from_u64(1)
            }
            fn record(&self, _: &tracing::span::Id, _: &tracing::span::Record<'_>) {}
            fn record_follows_from(&self, _: &tracing::span::Id, _: &tracing::span::Id) {}
            fn enter(&self, _: &tracing::span::Id) {}
            fn exit(&self, _: &tracing::span::Id) {}
            fn event(&self, event: &tracing::Event<'_>) {
                struct Fields<'a>(&'a mut String);
                impl tracing::field::Visit for Fields<'_> {
                    fn record_debug(
                        &mut self,
                        field: &tracing::field::Field,
                        value: &dyn std::fmt::Debug,
                    ) {
                        use std::fmt::Write as _;
                        let _ = write!(self.0, "{}={value:?} ", field.name());
                    }
                }
                let mut line = String::new();
                event.record(&mut Fields(&mut line));
                let mut buffer = self
                    .0
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner);
                buffer.push_str(&line);
            }
        }

        let capture = Capture::default();
        let buffer = capture.0.clone();
        tracing::subscriber::with_default(capture, || {
            log_partial_selection("push://x", 1, 4);
            log_partial_selection("push://x", 4, 4);
        });
        let logs = buffer
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone();
        assert_eq!(
            logs.matches("cannot carry every track").count(),
            1,
            "the exclusion is reported once: {logs}"
        );
        assert!(logs.contains("carried=1 "), "{logs}");
        assert!(logs.contains("proposed=4 "), "{logs}");
    }
}
