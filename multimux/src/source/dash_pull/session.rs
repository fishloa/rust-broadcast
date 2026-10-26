//! The sans-IO DASH-pull [`IngestSession`]: a driven
//! [`dash_runtime::client::DashClient`] behind the media-plane ingress traits.
//!
//! The core owns everything DASH: MPD parsing, addressing, the UTC clock
//! ladder, refresh, the live join, availability gating, failure
//! classification and track selection. This session owns what is multimux's:
//! mapping the core's outputs onto [`SessionEvent`]s (stitching the sample
//! timeline across Periods, see [`super::stitch`]), the policy for what ends a
//! connection attempt (the route supervisor reconnects), and the retry
//! directives the drive loop turns into backoff delays. It never reads a
//! clock and never sleeps: every instant arrives through
//! [`DashIngestSession::set_now`] or [`Stage::feed`].

use std::collections::VecDeque;

use broadcast_common::{Demand, Stage, Timestamp};
use dash_runtime::client::{
    Action, DashClient, DashConfig, DropCause, Error as CoreError, Failure, FailureClass,
    FatalKind, Notice, Output, PullLimits, ResourceId, ResponseMeta, UtcId, UtcResponse,
};
use dash_runtime::error::MpdError;
use dash_runtime::selection::DropReason;
use media_plane::ingress::{IngestSession, ProgramId, SessionEvent};
use media_plane::trunk::RetentionClass;
use url::Url;

use super::stitch::Stitcher;
use crate::error::{MultimuxError, Result};

/// The one program a DASH presentation publishes: every selected Period and
/// AdaptationSet plays on a single timeline.
pub const PROGRAM: ProgramId = ProgramId(0);

/// Consecutive unusable MPD answers (refresh failures and rejected bodies)
/// after which the attempt ends and the supervisor reconnects.
const MAX_MPD_FAILURES: u32 = 8;
/// Consecutive failures of one segment before a static presentation fails the
/// attempt and a live one skips the segment.
const MAX_SEGMENT_FAILURES: u32 = 4;
/// Consecutive failures of one init or index resource before the attempt ends.
const MAX_INIT_FAILURES: u32 = 4;
/// Segment requests kept outstanding by the core, a share of
/// [`crate::source::MAX_INFLIGHT_FETCHES`] that leaves room for the MPD, the
/// inits and the `UTCTiming` requests.
const SEGMENT_WINDOW: usize = crate::source::MAX_INFLIGHT_FETCHES / 2;
const NANOS_PER_MICRO: u64 = 1_000;

/// The [`DashConfig`] multimux runs the core with. `allow_cross_origin` is the
/// route's opt-in: off, every URL an MPD makes the server request (`BaseURL`,
/// segments, `UTCTiming`) must have the MPD's own origin, because a server-side
/// pull must not fetch whatever address a remote manifest names.
#[must_use]
pub fn dash_config(allow_cross_origin: bool) -> DashConfig {
    let mut config = DashConfig::default();
    config.cross_origin_urls = allow_cross_origin;
    config
}

/// The shared pull limits multimux runs the core with.
#[must_use]
pub fn pull_limits() -> PullLimits {
    let mut limits = PullLimits::default();
    limits.max_segment_bytes = crate::source::MAX_HTTP_BODY_BYTES;
    limits.max_inflight_segments = SEGMENT_WINDOW;
    limits.max_segment_failures = MAX_SEGMENT_FAILURES;
    limits.max_init_failures = MAX_INIT_FAILURES;
    limits
}

/// What a response or failure is about.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
#[non_exhaustive]
pub enum FetchTarget {
    /// The MPD, first fetch or refresh.
    Mpd,
    /// A `UTCTiming` request.
    Utc(UtcId),
    /// An init, index or media segment.
    Resource(ResourceId),
}

/// One thing the drive loop feeds the session.
#[derive(Debug)]
#[non_exhaustive]
pub enum DashInput<'a> {
    /// The MPD body with what the loop measured and read.
    Mpd {
        /// The body.
        body: &'a [u8],
        /// Exchange instants and headers.
        meta: ResponseMeta<'a>,
    },
    /// The MPD request was answered `304`.
    MpdUnchanged {
        /// Exchange instants.
        meta: ResponseMeta<'a>,
    },
    /// An init, index or media segment body.
    Resource {
        /// Which one.
        id: ResourceId,
        /// The body, already sliced to the requested range.
        body: &'a [u8],
    },
    /// A `UTCTiming` answer.
    Utc {
        /// Which request.
        id: UtcId,
        /// The answer.
        response: UtcResponse<'a>,
    },
    /// A fetch failed.
    Failed {
        /// Which fetch.
        target: FetchTarget,
        /// How.
        failure: Failure,
    },
}

/// What the drive loop retries after a backoff.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub(super) enum RetryTarget {
    Mpd,
    Resource(ResourceId),
}

/// A retry the core asked for; `attempt` counts consecutive failures of the
/// resource, from 1.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct Retry {
    pub(super) target: RetryTarget,
    pub(super) attempt: u32,
}

/// The sans-IO DASH-pull [`IngestSession`]: a driven `DashClient`, no socket.
pub struct DashIngestSession {
    client: DashClient,
    now_us: u64,
    pending_events: VecDeque<SessionEvent>,
    retries: VecDeque<Retry>,
    stitch: Stitcher,
    established: bool,
    announced: bool,
    mpd_accepted: bool,
    mpd_rejections: u32,
    encryption_seen: bool,
    ended: bool,
    failed: Option<MultimuxError>,
}

impl DashIngestSession {
    /// A session for `mpd_url` with multimux's defaults (cross-origin URLs
    /// refused). Performs no I/O.
    #[must_use]
    pub fn new(mpd_url: Url) -> Self {
        Self::with_config(mpd_url, dash_config(false), pull_limits())
    }

    /// A session with an explicit core configuration.
    #[must_use]
    pub fn with_config(mpd_url: Url, dash: DashConfig, limits: PullLimits) -> Self {
        Self {
            client: DashClient::new(mpd_url.as_str(), dash, limits),
            now_us: 0,
            pending_events: VecDeque::new(),
            retries: VecDeque::new(),
            stitch: Stitcher::default(),
            established: false,
            announced: false,
            mpd_accepted: false,
            mpd_rejections: 0,
            encryption_seen: false,
            ended: false,
            failed: None,
        }
    }

    /// Advance the session's view of the monotonic clock, microseconds since
    /// the source opened. Never moves backwards.
    pub fn set_now(&mut self, now_us: u64) {
        self.now_us = self.now_us.max(now_us);
    }

    /// At the monotonic instant `now_us` the host's UTC clock read `unix_us`.
    pub fn set_wall_clock(&mut self, now_us: u64, unix_us: u64) {
        self.set_now(now_us);
        self.client.set_wall_clock(now_us, unix_us);
        self.drain_outputs();
    }

    /// True once a static presentation was played to its end.
    #[must_use]
    pub fn ended(&self) -> bool {
        self.ended
    }

    /// True once the core or the failure policy ended the attempt; the next
    /// [`Stage::finish`] reports why.
    #[must_use]
    pub fn failed(&self) -> bool {
        self.failed.is_some()
    }

    /// True once an MPD has been accepted.
    #[must_use]
    pub fn mpd_accepted(&self) -> bool {
        self.mpd_accepted
    }

    /// Whether the core still waits for the answer to `id`.
    #[must_use]
    pub fn awaits(&self, id: ResourceId) -> bool {
        self.client.awaits(id)
    }

    /// Ask the core for the fetch it wants retried; call once the backoff has
    /// elapsed.
    pub(super) fn retry(&mut self, target: RetryTarget) {
        match target {
            RetryTarget::Mpd => self.client.retry_mpd(self.now_us),
            RetryTarget::Resource(id) => {
                self.client.retry(self.now_us, id);
            }
        }
    }

    /// The retries the core asked for since the last call.
    pub(super) fn take_retries(&mut self) -> Vec<Retry> {
        self.retries.drain(..).collect()
    }

    fn take_failed(&mut self) -> Result<()> {
        self.failed.take().map_or(Ok(()), Err)
    }

    fn fail(&mut self, error: MultimuxError) {
        self.failed.get_or_insert(error);
    }

    fn queue_retry(&mut self, target: RetryTarget, class: FailureClass) {
        if let FailureClass::Retry { attempt, .. } = class {
            self.retries.push_back(Retry { target, attempt });
        }
    }

    fn on_mpd(&mut self, body: &[u8], meta: &ResponseMeta<'_>) {
        match self.client.on_mpd_with(self.now_us, body, meta) {
            Ok(()) => {
                self.mpd_rejections = 0;
                self.mpd_accepted = true;
                if !self.established {
                    self.established = true;
                    self.pending_events.push_back(SessionEvent::Established);
                }
            }
            Err(CoreError::Mpd(error)) => self.mpd_rejected(&error),
            Err(_) => {}
        }
    }

    fn mpd_rejected(&mut self, error: &MpdError) {
        self.mpd_rejections = self.mpd_rejections.saturating_add(1);
        if !self.mpd_accepted || self.mpd_rejections >= MAX_MPD_FAILURES {
            self.fail(MultimuxError::Connect {
                reason: format!("dash-pull: mpd rejected: {error}"),
            });
            return;
        }
        self.retries.push_back(Retry {
            target: RetryTarget::Mpd,
            attempt: self.mpd_rejections,
        });
    }

    fn on_resource(&mut self, id: ResourceId, body: &[u8]) {
        let result = match id {
            ResourceId::Init { .. } => self.client.on_init(self.now_us, id, body),
            ResourceId::Index { .. } => self.client.on_index(self.now_us, id, body),
            _ => self.client.on_segment(self.now_us, id, body),
        };
        if let Err(CoreError::Rejected { id, class, reason }) = result {
            tracing::warn!(?id, reason = reason.name(), "dash-pull: body rejected");
            self.queue_retry(RetryTarget::Resource(id), class);
        }
    }

    fn on_failed(&mut self, target: FetchTarget, failure: Failure) {
        match target {
            FetchTarget::Mpd => {
                let class = self.client.on_mpd_failure(self.now_us, failure);
                if let FailureClass::Retry { attempt, .. } = class {
                    if attempt >= MAX_MPD_FAILURES {
                        self.fail(MultimuxError::Connect {
                            reason: format!("dash-pull: mpd refresh failed {attempt} times"),
                        });
                    } else {
                        self.queue_retry(RetryTarget::Mpd, class);
                    }
                }
            }
            FetchTarget::Utc(id) => {
                let _ = self.client.on_utc_failure(self.now_us, id, failure);
            }
            FetchTarget::Resource(id) => {
                if let Ok(class) = self.client.on_failure(self.now_us, id, failure) {
                    self.queue_retry(RetryTarget::Resource(id), class);
                }
            }
        }
    }

    fn drain_outputs(&mut self) {
        while let Some(output) = self.client.next_output() {
            self.map_output(output);
        }
    }

    fn map_output(&mut self, output: Output) {
        match output {
            Output::Tracks(specs) => {
                if specs.is_empty() {
                    return;
                }
                self.stitch.set_tracks(&specs);
                let event = if self.announced {
                    SessionEvent::TracksChanged {
                        program: PROGRAM,
                        tracks: specs,
                    }
                } else {
                    self.announced = true;
                    SessionEvent::NewProgram {
                        program: PROGRAM,
                        tracks: specs,
                    }
                };
                self.pending_events.push_back(event);
            }
            Output::Samples {
                track_id,
                mut samples,
            } => {
                self.stitch.apply(track_id, &mut samples);
                for sample in samples {
                    self.pending_events.push_back(SessionEvent::Sample {
                        program: PROGRAM,
                        track_id,
                        retention: RetentionClass::Timed,
                        sample,
                    });
                }
            }
            Output::Discontinuity => self.stitch.discontinuity(),
            Output::TrackDropped {
                period, set, cause, ..
            } => {
                if cause == DropCause::Selection(DropReason::UnsupportedEncryption) {
                    self.encryption_seen = true;
                }
                tracing::warn!(
                    period,
                    set,
                    cause = cause.name(),
                    "dash-pull: track not played"
                );
            }
            Output::TrackSkipped { fourcc, reason, .. } => {
                tracing::warn!(%fourcc, %reason, "dash-pull: track skipped");
            }
            Output::FragmentDamaged { skipped, .. } => {
                tracing::warn!(skipped, "dash-pull: damaged fragments skipped");
            }
            Output::Notice(Notice::FellBehind) => {
                tracing::warn!(
                    "dash-pull: fell behind the live window, continuing at its oldest segment"
                );
            }
            Output::Notice(notice) => tracing::info!(notice = notice.name(), "dash-pull"),
            Output::ClockSynced {
                source,
                uncertainty_us,
            } => tracing::info!(%source, uncertainty_us, "dash-pull: UTC clock"),
            Output::SegmentLost { id, status } => {
                tracing::warn!(?id, ?status, "dash-pull: segment given up on");
                metrics::counter!(crate::prometheus::PULL_FRAGMENT_ABANDONED_TOTAL).increment(1);
            }
            Output::Fatal(kind) => {
                let error = self.fatal_error(&kind);
                self.fail(error);
            }
            Output::EndOfStream => self.ended = true,
            _ => {}
        }
    }

    fn fatal_error(&self, kind: &FatalKind) -> MultimuxError {
        match kind {
            FatalKind::MpdInvalid { reason } if self.encryption_seen => MultimuxError::Encrypted {
                reason: format!(
                    "dash-pull: no Representation can be played ({reason}); the content is protected by ContentProtection and is not decrypted"
                ),
            },
            FatalKind::ClockUnavailable => MultimuxError::Connect {
                reason: "dash-pull: a live MPD needs a UTC clock and none is available (no UTCTiming answer, no host clock, no response Date)".into(),
            },
            other => MultimuxError::Connect {
                reason: format!("dash-pull: presentation cannot continue: {other:?}"),
            },
        }
    }
}

impl Stage for DashIngestSession {
    type In<'a> = DashInput<'a>;
    type Out = SessionEvent;
    type Error = MultimuxError;

    fn feed(&mut self, input: DashInput<'_>, now: Timestamp) -> Result<()> {
        self.set_now(now.as_nanos() / NANOS_PER_MICRO);
        match input {
            DashInput::Mpd { body, meta } => self.on_mpd(body, &meta),
            DashInput::MpdUnchanged { meta } => {
                let _ = self.client.on_mpd_unchanged(self.now_us, &meta);
                self.mpd_rejections = 0;
            }
            DashInput::Resource { id, body } => self.on_resource(id, body),
            DashInput::Utc { id, response } => {
                let _ = self.client.on_utc_response(self.now_us, id, &response);
            }
            DashInput::Failed { target, failure } => self.on_failed(target, failure),
        }
        self.drain_outputs();
        self.take_failed()
    }

    fn poll(&mut self) -> Option<SessionEvent> {
        self.pending_events.pop_front()
    }

    fn finish(&mut self) -> Result<()> {
        self.drain_outputs();
        self.take_failed()
    }

    fn next_deadline(&self) -> Option<Timestamp> {
        if !self.pending_events.is_empty() || self.failed.is_some() {
            return Some(Timestamp::from_nanos(
                self.now_us.saturating_mul(NANOS_PER_MICRO),
            ));
        }
        self.client
            .next_wake()
            .map(|us| Timestamp::from_nanos(us.saturating_mul(NANOS_PER_MICRO)))
    }

    fn on_deadline(&mut self, now: Timestamp) {
        self.set_now(now.as_nanos() / NANOS_PER_MICRO);
        self.drain_outputs();
    }

    fn demand(&self) -> Demand {
        Demand::new(crate::source::MAX_TS_READ)
    }
}

impl IngestSession for DashIngestSession {
    type Request = Action;

    fn poll_transmit(&mut self) -> Option<Action> {
        let action = self.client.poll(self.now_us);
        self.drain_outputs();
        action
    }
}
