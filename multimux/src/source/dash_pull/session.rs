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

#[cfg(test)]
mod tests {
    use super::*;
    use broadcast_common::Unpackage;
    use dash_runtime::selection::DropReason;
    use transmux::media::Fmp4Demux;
    use transmux::{Sample, TrackId, TrackSpec};

    const AST_UNIX_SECS: i64 = 1_767_225_600;
    const MICROS: u64 = 1_000_000;

    fn ts(now_us: u64) -> Timestamp {
        Timestamp::from_nanos(now_us * NANOS_PER_MICRO)
    }

    fn session() -> DashIngestSession {
        DashIngestSession::new(Url::parse("http://origin.test/live.mpd").unwrap())
    }

    fn spec(id: u32, timescale: u32) -> TrackSpec {
        let init = std::fs::read(test_fixtures::path("dash/init-stream1.m4s")).expect("init");
        let mut spec = Fmp4Demux::new()
            .unpackage(init.as_slice())
            .expect("demux init")
            .tracks
            .remove(0)
            .spec;
        spec.track_id = TrackId::new(id);
        spec.timescale = timescale;
        spec
    }

    fn sample(dts: i64) -> Sample {
        Sample::new(vec![0u8], Some(dts), Some(dts), Some(1_000), true)
    }

    fn events(s: &mut DashIngestSession) -> Vec<SessionEvent> {
        std::iter::from_fn(|| s.poll()).collect()
    }

    fn live_mpd() -> String {
        r#"<?xml version="1.0"?>
<MPD xmlns="urn:mpeg:dash:schema:mpd:2011" type="dynamic" availabilityStartTime="2026-01-01T00:00:00Z" minimumUpdatePeriod="PT1S" timeShiftBufferDepth="PT30S" suggestedPresentationDelay="PT2S" maxSegmentDuration="PT2S" profiles="urn:mpeg:dash:profile:isoff-live:2011">
<Period id="0" start="PT0S"><AdaptationSet mimeType="video/mp4"><Representation id="0" codecs="avc1.4d400d" bandwidth="1000" width="320" height="240"><SegmentTemplate timescale="1000" duration="2000" startNumber="1" initialization="init.m4s" media="seg-$Number$.m4s"/></Representation></AdaptationSet></Period>
</MPD>"#
            .to_owned()
    }

    fn host_utc_us() -> u64 {
        u64::try_from(AST_UNIX_SECS + 100).unwrap() * MICROS
    }

    /// A session that accepted a live MPD at `now` 0 and is waiting for its
    /// first refresh.
    fn live_session() -> DashIngestSession {
        let mut s = session();
        s.set_wall_clock(0, host_utc_us());
        assert!(matches!(s.poll_transmit(), Some(Action::FetchMpd { .. })));
        let mpd = live_mpd();
        s.feed(
            DashInput::Mpd {
                body: mpd.as_bytes(),
                meta: ResponseMeta::at(0),
            },
            ts(0),
        )
        .expect("a good MPD is accepted");
        s
    }

    /// Advance to the next MPD request the core makes, answering nothing else.
    fn next_mpd_request(s: &mut DashIngestSession, now_us: &mut u64) {
        for _ in 0..64 {
            while let Some(action) = s.poll_transmit() {
                if matches!(action, Action::FetchMpd { .. }) {
                    return;
                }
            }
            events(s);
            let wake = s.next_deadline().expect("the core wants to refresh");
            *now_us = (*now_us).max(wake.as_nanos() / NANOS_PER_MICRO);
            s.set_now(*now_us);
        }
        panic!("the core never asked for an MPD refresh");
    }

    #[test]
    fn established_is_queued_with_the_first_accepted_mpd_and_only_then() {
        let mut s = session();
        assert!(events(&mut s).is_empty());
        let mpd = live_mpd();
        s.set_wall_clock(0, host_utc_us());
        let _ = s.poll_transmit();
        s.feed(
            DashInput::Mpd {
                body: mpd.as_bytes(),
                meta: ResponseMeta::at(0),
            },
            ts(0),
        )
        .unwrap();
        let got = events(&mut s);
        assert!(
            matches!(got.as_slice(), [SessionEvent::Established]),
            "{got:?}"
        );
        assert!(s.mpd_accepted());
    }

    #[test]
    fn a_first_mpd_that_does_not_parse_fails_the_attempt() {
        let mut s = session();
        let _ = s.poll_transmit();
        let err = s
            .feed(
                DashInput::Mpd {
                    body: b"<not-an-mpd",
                    meta: ResponseMeta::at(0),
                },
                ts(0),
            )
            .expect_err("there is nothing to keep playing");
        assert!(err.to_string().contains("mpd rejected"), "{err}");
        assert!(!s.mpd_accepted());
    }

    #[test]
    fn rejected_refreshes_are_retried_until_the_eighth_in_a_row() {
        let mut s = live_session();
        let mut now = 0u64;
        for attempt in 1..MAX_MPD_FAILURES {
            next_mpd_request(&mut s, &mut now);
            s.feed(
                DashInput::Mpd {
                    body: b"<garbage",
                    meta: ResponseMeta::at(now),
                },
                ts(now),
            )
            .unwrap_or_else(|e| panic!("rejection {attempt} must not end the attempt: {e}"));
            let retries = s.take_retries();
            assert_eq!(
                retries,
                [Retry {
                    target: RetryTarget::Mpd,
                    attempt
                }]
            );
            s.retry(RetryTarget::Mpd);
        }
        next_mpd_request(&mut s, &mut now);
        let err = s
            .feed(
                DashInput::Mpd {
                    body: b"<garbage",
                    meta: ResponseMeta::at(now),
                },
                ts(now),
            )
            .expect_err("the eighth rejection in a row ends the attempt");
        assert!(err.to_string().contains("mpd rejected"), "{err}");
    }

    #[test]
    fn a_good_mpd_between_rejections_resets_the_count() {
        let mut s = live_session();
        let mut now = 0u64;
        let good = live_mpd();
        for _ in 0..MAX_MPD_FAILURES * 2 {
            next_mpd_request(&mut s, &mut now);
            let body: &[u8] = if s.mpd_rejections % 3 == 2 {
                good.as_bytes()
            } else {
                b"<garbage"
            };
            s.feed(
                DashInput::Mpd {
                    body,
                    meta: ResponseMeta::at(now),
                },
                ts(now),
            )
            .expect("never eight in a row");
            s.take_retries();
            s.retry(RetryTarget::Mpd);
        }
    }

    #[test]
    fn failed_mpd_refreshes_are_retried_until_the_eighth_in_a_row() {
        let mut s = live_session();
        let mut now = 0u64;
        for attempt in 1..MAX_MPD_FAILURES {
            next_mpd_request(&mut s, &mut now);
            s.feed(
                DashInput::Failed {
                    target: FetchTarget::Mpd,
                    failure: Failure::Status(503),
                },
                ts(now),
            )
            .unwrap_or_else(|e| panic!("failure {attempt} must not end the attempt: {e}"));
            assert_eq!(
                s.take_retries(),
                [Retry {
                    target: RetryTarget::Mpd,
                    attempt
                }]
            );
            s.retry(RetryTarget::Mpd);
        }
        next_mpd_request(&mut s, &mut now);
        let err = s
            .feed(
                DashInput::Failed {
                    target: FetchTarget::Mpd,
                    failure: Failure::Timeout,
                },
                ts(now),
            )
            .expect_err("the eighth failure in a row ends the attempt");
        assert!(err.to_string().contains("refresh failed"), "{err}");
    }

    #[test]
    fn the_session_clock_never_moves_backwards() {
        let mut s = session();
        s.set_now(5_000);
        s.set_now(1_000);
        assert_eq!(s.now_us, 5_000);
        s.feed(
            DashInput::Failed {
                target: FetchTarget::Mpd,
                failure: Failure::Transport,
            },
            ts(2_000),
        )
        .ok();
        assert_eq!(s.now_us, 5_000);
    }

    #[test]
    fn the_first_track_set_is_a_new_program_and_later_ones_replace_it() {
        let mut s = session();
        s.map_output(Output::Tracks(Vec::new()));
        assert!(
            events(&mut s).is_empty(),
            "an empty track set announces nothing"
        );
        s.map_output(Output::Tracks(vec![spec(1, 1000)]));
        s.map_output(Output::Tracks(vec![spec(1, 1000), spec(2, 48_000)]));
        let got = events(&mut s);
        assert!(
            matches!(
                got.as_slice(),
                [
                    SessionEvent::NewProgram { program, tracks: first },
                    SessionEvent::TracksChanged { program: again, tracks: second },
                ] if *program == PROGRAM && *again == PROGRAM && first.len() == 1 && second.len() == 2
            ),
            "{got:?}"
        );
    }

    #[test]
    fn samples_publish_as_timed_samples_of_the_one_program() {
        let mut s = session();
        s.map_output(Output::Tracks(vec![spec(1, 1000)]));
        s.map_output(Output::Samples {
            track_id: TrackId::new(1),
            samples: vec![sample(0), sample(1_000)],
        });
        let got = events(&mut s);
        let samples: Vec<_> = got
            .iter()
            .filter_map(|e| match e {
                SessionEvent::Sample {
                    program,
                    track_id,
                    retention,
                    sample,
                } => Some((*program, *track_id, *retention, sample.dts)),
                _ => None,
            })
            .collect();
        assert_eq!(
            samples,
            [
                (PROGRAM, TrackId::new(1), RetentionClass::Timed, Some(0)),
                (PROGRAM, TrackId::new(1), RetentionClass::Timed, Some(1_000)),
            ]
        );
    }

    #[test]
    fn a_discontinuity_output_stitches_the_samples_that_follow() {
        let mut s = session();
        s.map_output(Output::Tracks(vec![spec(1, 1000)]));
        s.map_output(Output::Samples {
            track_id: TrackId::new(1),
            samples: vec![sample(0)],
        });
        s.map_output(Output::Discontinuity);
        s.map_output(Output::Samples {
            track_id: TrackId::new(1),
            samples: vec![sample(0)],
        });
        let dts: Vec<_> = events(&mut s)
            .into_iter()
            .filter_map(|e| match e {
                SessionEvent::Sample { sample, .. } => sample.dts,
                _ => None,
            })
            .collect();
        assert_eq!(dts, [0, 1_000]);
    }

    #[test]
    fn a_fatal_output_fails_the_next_feed_and_finish_reports_it_once() {
        let mut s = session();
        s.map_output(Output::Fatal(FatalKind::ClockUnavailable));
        assert!(s.failed());
        let err = s.finish().expect_err("fatal");
        assert!(err.to_string().contains("UTC clock"), "{err}");
        assert!(s.finish().is_ok(), "reported once");
        assert!(!s.failed());
    }

    #[test]
    fn protected_content_that_leaves_nothing_to_play_is_an_encrypted_error() {
        let mut s = session();
        s.map_output(Output::TrackDropped {
            period: 0,
            set: 0,
            rep: None,
            cause: DropCause::Selection(DropReason::UnsupportedEncryption),
        });
        s.map_output(Output::Fatal(FatalKind::MpdInvalid {
            reason: "no_playable_representation",
        }));
        assert!(
            matches!(s.finish(), Err(MultimuxError::Encrypted { .. })),
            "protected content is reported as such"
        );
        let mut plain = session();
        plain.map_output(Output::Fatal(FatalKind::MpdInvalid {
            reason: "no_playable_representation",
        }));
        assert!(matches!(plain.finish(), Err(MultimuxError::Connect { .. })));
    }

    #[test]
    fn the_end_of_the_stream_is_reported_once_the_core_says_so() {
        let mut s = session();
        assert!(!s.ended());
        s.map_output(Output::EndOfStream);
        assert!(s.ended());
    }

    #[test]
    fn the_session_has_something_to_drain_while_events_wait() {
        let mut s = session();
        assert!(s.next_deadline().is_none());
        s.map_output(Output::Tracks(vec![spec(1, 1000)]));
        s.set_now(7_000);
        assert_eq!(s.next_deadline(), Some(ts(7_000)));
        events(&mut s);
        assert!(s.next_deadline().is_none());
    }

    #[test]
    fn a_segment_failure_queues_a_retry_naming_its_attempt() {
        let mut s = live_session();
        let mut now = 0u64;
        // Let the core request a segment, then fail it.
        let mut segment = None;
        for _ in 0..64 {
            match s.poll_transmit() {
                Some(Action::FetchSegment { id, .. }) => {
                    segment = Some(id);
                    break;
                }
                Some(Action::FetchInit { id, .. }) => {
                    let init = std::fs::read(test_fixtures::path("dash/init-stream0.m4s")).unwrap();
                    s.feed(DashInput::Resource { id, body: &init }, ts(now))
                        .ok();
                }
                Some(_) => {}
                None => {
                    events(&mut s);
                    let wake = s.next_deadline().expect("a wake");
                    now = now.max(wake.as_nanos() / NANOS_PER_MICRO);
                    s.set_now(now);
                }
            }
        }
        let id = segment.expect("the core requested a segment");
        s.feed(
            DashInput::Failed {
                target: FetchTarget::Resource(id),
                failure: Failure::Status(500),
            },
            ts(now),
        )
        .unwrap();
        assert_eq!(
            s.take_retries(),
            [Retry {
                target: RetryTarget::Resource(id),
                attempt: 1
            }]
        );
    }
}
