//! Productionises the segmenter gap `media_plane::ts_session`'s own test proved closed
//! (issue #805 task 2b): task 2 wired all eight `media_plane`-ported inputs
//! (rtsp/rtp/ts_udp/ts_http/srt/hls_pull/dash_pull/smooth_pull) so they
//! *ingest* — each publishes its driver-minted [`Trunk`] into
//! `crate::route::RouteHandle`'s program registry
//! (`crate::source::report_driver_progress`), and egress resolves through
//! that registry. But nothing turned the raw samples the driver publishes
//! into segments/parts: the eight inputs wrote samples via the driver's
//! `TrunkWriter` and nothing ever read them back out, so a driver-backed
//! route's LL-HLS/DASH playlists came back empty — ingest was observable,
//! playback was not.
//!
//! `ProgramSegmenter` is the missing stage: one per announced
//! [`ProgramId`], subscribing a [`SampleCursor`] to that program's
//! driver-minted `Trunk`, feeding a [`transmux::ll_hls::LlHlsSegmenter`], and
//! publishing the resulting parts/segments back into **the same `Trunk`**
//! via [`Trunk::segment_writer`] — never a second `Trunk`, and never copying
//! samples between trunks (decided in
//! `docs/superpowers/specs/2026-07-26-media-plane-architecture.md` §8: the
//! ring-group split, `Trunk::segment_writer`, is exactly what makes holding
//! both the driver's sample writer and this segmenter's segment writer live
//! simultaneously on one `Trunk` legal). `drive_program_segmenters` is the
//! per-iteration driver every `run_*` entry point calls, mirroring
//! `report_driver_progress`'s own shape: build a segmenter the first time a
//! program's tracks have landed, then pump every already-built segmenter's
//! cursor.
//!
//! **Issue #887**: `ProgramSegmenter` feeds one of *two* segmenter kinds —
//! [`transmux::ll_hls::LlHlsSegmenter`] (fMP4/CMAF, LL-HLS parts) or
//! [`transmux::ts_hls::StreamingTsHlsSegmenter`] (classic whole-`.ts`
//! segments, no parts) — selected per-program by that program's route's
//! configured `crate::route::RouteHandle::container()` (see `AnySegmenter`,
//! below). A route's container is fixed at route-construction time
//! (`crate::config::OutputKind::TsHls` is mutually exclusive with
//! `llhls`/`dash`/`ll_dash` on the same route, enforced by
//! `crate::config::Route::validate_standalone`), so every program on one
//! route always takes the same branch.
//!
//! Both are `pub(crate)` (issue #805 task 6 narrowed them back from `pub`):
//! the supported extension surface for a `crate::registry::SchemeRegistry`
//! `Custom` input factory is now the single `crate::source::advance_route`
//! facade over `report_driver_progress` + this module's own
//! `drive_program_segmenters`, not either call directly — a factory that
//! called them separately (in the wrong order, or one without the other)
//! could silently ingest with nothing ever becoming servable, the exact
//! footgun `advance_route` exists to remove. See `examples/custom_scheme.rs`.
//!
//! # Init bytes and `RouteHandle::set_init`
//!
//! The fMP4 init segment has no home in a `Trunk` (no ring holds it — see
//! `hls_runtime::server::engine`'s own module doc, "the one thing that
//! genuinely cannot come from the `Trunk` alone"); it lives inside the
//! route's `hls_runtime::server::HlsOrigin` instead.
//! `RouteHandle::publish_program` (crate-private) builds that program's
//! `ProgramServing` bundle (`HlsOrigin`+`DashState`) the first time its
//! `Trunk` is published — see that method's own doc — so calling
//! `crate::route::RouteHandle::set_init` here, *after* this program's
//! `Trunk` has already been published into the registry, lands the init
//! bytes in the same bundle every driver-backed `run_*` reads from.
//! Init is pushed for **every** program, not just `SPTS_PROGRAM_ID` — DVR
//! recording needs it for all programs (issue #906).
//!
//! # Why segmenting is per-program, not per-route
//!
//! An MPTS route carries several programs on one `IngestDriver`; each has
//! its own driver-minted `Trunk` (see `media_plane::ingress`'s own docs) and
//! must be segmented independently — `drive_program_segmenters` iterates
//! every program `driver.programs()` reports, not just one. Every program
//! gets its own `crate::route::RouteHandle` registry entry (issue #805 task
//! 6: per-program serving state), so every program's `Trunk` carries its own
//! real segments/parts, individually resolvable exactly like the
//! `media_plane::ts_session` test proves and `crate::route`'s own
//! `two_programs_serve_distinct_media` test proves end to end. *Selecting*
//! one from an incoming HTTP request (as opposed to segmenting/serving it at
//! all) is the addressing question `crate::route`'s own module doc records
//! options for.

use std::collections::{BTreeSet, HashMap, VecDeque};
use std::sync::Arc;
use std::time::{Duration, Instant};

use broadcast_common::clock33::PTS_HZ;
use broadcast_common::timescale::{Rounding, rescale_u64};
use transmux::TrackId;

use crate::ingress::ProgramId;
use crate::publish_queue::{PendingPublish, drain_publish_queue, part_entry};
use crate::trunk::{SampleCursor, SampleCursorItem, SegmentEntry, SegmentWriter, Trunk};
use broadcast_common::Timestamp;
use broadcast_common::bcp47_language;
use dvb_si::descriptors::atsc_caption_service::AtscCaptionService;
use transmux::Container;
use transmux::SegmentMeta;
use transmux::ll_hls::{LlHlsSegmenter, PartCutReady, PartInfo};
use transmux::pipeline::{CodecConfig, Sample, TrackSpec};
use transmux::ts_hls::StreamingTsHlsSegmenter;
use transmux::{CaptionDetector, DetectedCaption, declared_caption_services};

use crate::ids::SegmentSeq;
use crate::segmenter::audio::AudioFollower;

use crate::segmenter::audio::{LEADER_DURATION_HISTORY, LeaderDurations, leader_tracks};
use crate::segmenter::subtitle::{LeaderPart, SubtitleFollower};
use broadcast_common::units::NANOS_PER_SEC;
use broadcast_hls::InstreamId;

/// Configuration for one [`ProgramSegmenter`], replaceable on route reload:
/// start from [`SegmenterConfig::default`] and set what differs with the
/// `with_*` setters.
#[derive(Clone, Debug)]
#[non_exhaustive]
pub struct SegmenterConfig {
    pub container: transmux::Container,
    pub ts_cut_policy: transmux::SegmentCutPolicy,
    pub ts_idr_fallback: TsHlsIdrFallback,
    pub target_duration_secs: f64,
    pub part_target: Duration,
    pub window_segments_cap: std::num::NonZeroUsize,
    pub route_name: String,
    pub audio_rendition_sources: Vec<transmux::audio_profile::AudioTrackSelector>,
    pub audio_profiles: Vec<transmux::audio_profile::AudioProfileOverride>,
    pub subtitles_enabled: bool,
    pub trunk_config: crate::trunk::TrunkConfig,
}

impl Default for SegmenterConfig {
    fn default() -> Self {
        Self {
            container: transmux::Container::default(),
            ts_cut_policy: transmux::SegmentCutPolicy::default(),
            ts_idr_fallback: TsHlsIdrFallback::default(),
            target_duration_secs: 1.0,
            part_target: Duration::from_millis(250),
            window_segments_cap: std::num::NonZeroUsize::MIN.saturating_add(7),
            route_name: "unknown".to_string(),
            audio_rendition_sources: Vec::new(),
            audio_profiles: Vec::new(),
            subtitles_enabled: true,
            trunk_config: crate::trunk::TrunkConfig::default(),
        }
    }
}

impl SegmenterConfig {
    pub fn with_container(mut self, v: transmux::Container) -> Self {
        self.container = v;
        self
    }
    pub fn with_ts_cut_policy(mut self, v: transmux::SegmentCutPolicy) -> Self {
        self.ts_cut_policy = v;
        self
    }
    pub fn with_ts_idr_fallback(mut self, v: TsHlsIdrFallback) -> Self {
        self.ts_idr_fallback = v;
        self
    }
    pub fn with_target_duration_secs(mut self, v: f64) -> Self {
        self.target_duration_secs = v;
        self
    }
    pub fn with_part_target(mut self, v: Duration) -> Self {
        self.part_target = v;
        self
    }
    pub fn with_window_segments_cap(mut self, v: std::num::NonZeroUsize) -> Self {
        self.window_segments_cap = v;
        self
    }
    pub fn with_route_name(mut self, v: impl Into<String>) -> Self {
        self.route_name = v.into();
        self
    }
    pub fn with_audio_rendition_sources(
        mut self,
        v: Vec<transmux::audio_profile::AudioTrackSelector>,
    ) -> Self {
        self.audio_rendition_sources = v;
        self
    }
    pub fn with_audio_profiles(
        mut self,
        v: Vec<transmux::audio_profile::AudioProfileOverride>,
    ) -> Self {
        self.audio_profiles = v;
        self
    }
    pub fn with_subtitles(mut self, v: bool) -> Self {
        self.subtitles_enabled = v;
        self
    }
    pub fn with_trunk_config(mut self, v: crate::trunk::TrunkConfig) -> Self {
        self.trunk_config = v;
        self
    }

    pub fn apply_declared_audio_profiles(
        &self,
        tracks: Vec<transmux::pipeline::TrackSpec>,
    ) -> Vec<transmux::pipeline::TrackSpec> {
        let applied = transmux::audio_profile::apply_overrides(&self.audio_profiles, tracks);
        for declared in &applied.unmatched {
            tracing::warn!(route = %self.route_name, selector = %declared.track, "audio_profiles entry matches no audio track; ignoring until one appears");
        }
        for (declared, error) in &applied.refused {
            tracing::warn!(route = %self.route_name, selector = %declared.track, profile = %declared.audio_profile, %error, "audio_profiles entry cannot be applied to this track; leaving it as the source states it");
        }
        applied.tracks
    }

    pub fn resolve_audio_sources(
        &self,
        tracks: &[transmux::pipeline::TrackSpec],
    ) -> Vec<(usize, transmux::pipeline::TrackSpec)> {
        crate::segmenter::audio::resolve_sources(
            &self.route_name,
            &self.audio_rendition_sources,
            tracks,
        )
    }
}

#[derive(Debug, Clone)]
#[non_exhaustive]
pub enum SegmenterEvent {
    TrackSpecs(Vec<transmux::pipeline::TrackSpec>),
    TracksActive(bool),
    BeginTrackSpecs {
        first_segment: SegmentSeq,
        specs: Vec<transmux::pipeline::TrackSpec>,
    },
    Init(Vec<u8>),
    InitFrom {
        init: Vec<u8>,
        first_segment: SegmentSeq,
    },
    PartTarget(Duration),
    ClosedCaptions {
        detected: Vec<InstreamId>,
        languages: Vec<(InstreamId, String)>,
    },
    TsIdrMissing(TsHlsIdrStatus),
    OutputFailed(String),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[non_exhaustive]
pub enum TsHlsIdrFallback {
    #[default]
    Auto,
    Targets(u32),
    Never,
}

impl TsHlsIdrFallback {
    pub fn falls_back(&self) -> bool {
        !matches!(self, TsHlsIdrFallback::Never)
    }
    pub fn bound(&self, target_duration_secs: f64) -> std::time::Duration {
        const IDR_FALLBACK_TARGETS: u32 = 3;
        const IDR_FALLBACK_MIN_SECS: f64 = 6.0;
        let secs = match self {
            TsHlsIdrFallback::Targets(n) => f64::from(*n) * target_duration_secs,
            TsHlsIdrFallback::Auto | TsHlsIdrFallback::Never => {
                (f64::from(IDR_FALLBACK_TARGETS) * target_duration_secs).max(IDR_FALLBACK_MIN_SECS)
            }
        };
        std::time::Duration::from_secs_f64(secs)
    }
    pub fn name(&self) -> &'static str {
        match self {
            TsHlsIdrFallback::Auto => "auto",
            TsHlsIdrFallback::Targets(_) => "targets",
            TsHlsIdrFallback::Never => "never",
        }
    }
}

broadcast_common::impl_spec_display!(TsHlsIdrFallback);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum TsHlsIdrStatus {
    FellBack,
    Awaiting,
}

impl TsHlsIdrStatus {
    pub fn name(&self) -> &'static str {
        match self {
            TsHlsIdrStatus::FellBack => "fell_back",
            TsHlsIdrStatus::Awaiting => "awaiting",
        }
    }
}

broadcast_common::impl_spec_display!(TsHlsIdrStatus);

pub trait SiblingTrunks {
    /// Build audio follower renditions for `trunk`. The only registration
    /// call besides [`Self::build_subtitle_followers`]: the host builds the
    /// sibling trunks (see [`crate::trunk::Trunk`]) before the segmenter
    /// runs, because the master playlist depends on that ordering.
    fn build_audio_followers(
        &self,
        trunk: &crate::trunk::Trunk,
        mapped: &[(usize, transmux::pipeline::TrackSpec)],
        target_duration_secs: f64,
        part_target: Duration,
    ) -> Vec<crate::segmenter::audio::AudioFollower>;
    fn build_subtitle_followers(
        &self,
        trunk: &crate::trunk::Trunk,
        tracks: &[transmux::pipeline::TrackSpec],
    ) -> Vec<crate::segmenter::subtitle::SubtitleFollower>;
}

impl SiblingTrunks for () {
    fn build_audio_followers(
        &self,
        _trunk: &crate::trunk::Trunk,
        _mapped: &[(usize, transmux::pipeline::TrackSpec)],
        _target_duration_secs: f64,
        _part_target: Duration,
    ) -> Vec<crate::segmenter::audio::AudioFollower> {
        Vec::new()
    }
    fn build_subtitle_followers(
        &self,
        _trunk: &crate::trunk::Trunk,
        _tracks: &[transmux::pipeline::TrackSpec],
    ) -> Vec<crate::segmenter::subtitle::SubtitleFollower> {
        Vec::new()
    }
}

/// One closed segment, normalised across the two segmenter kinds
/// [`AnySegmenter`] wraps — [`transmux::ll_hls::SegmentInfo`] (fMP4) and
/// [`transmux::ts_hls::TsSegment`] (classic TS) carry the same information
/// under different field names/numbering conventions (see
/// [`AnySegmenter::take_ready_segments`]), so [`ProgramSegmenter::publish_ready`]
/// works against this one shape regardless of which segmenter produced it.
pub(crate) struct ReadySegment {
    bytes: Vec<u8>,
    /// 1-based, matching [`media_plane::trunk::SegmentEntry::sequence_number`]'s
    /// own convention (which in turn mirrors
    /// [`transmux::ll_hls::SegmentInfo::segment_seq`]) — a
    /// [`transmux::ts_hls::TsSegment::sequence`] is 0-based, so
    /// [`AnySegmenter::take_ready_segments`] adds one when normalising it.
    segment_seq: SegmentSeq,
    duration: f64,
    discontinuous: bool,
}

impl From<transmux::ll_hls::SegmentInfo> for ReadySegment {
    fn from(info: transmux::ll_hls::SegmentInfo) -> Self {
        ReadySegment {
            bytes: info.bytes,
            segment_seq: SegmentSeq::new(info.segment_seq),
            duration: info.duration,
            // The fMP4 segmenter never itself flags a discontinuity (see
            // `ProgramSegmenter::publish_ready`'s pre-#887 hardcoded `false` —
            // preserved here, not a behaviour change).
            discontinuous: false,
        }
    }
}

impl From<transmux::ts_hls::TsSegment> for ReadySegment {
    fn from(seg: transmux::ts_hls::TsSegment) -> Self {
        ReadySegment {
            bytes: seg.bytes,
            segment_seq: SegmentSeq::new(
                u32::try_from(seg.sequence)
                    .unwrap_or(u32::MAX)
                    .saturating_add(1),
            ),
            duration: seg.duration,
            discontinuous: seg.discontinuous,
        }
    }
}

/// The two segmenter kinds a [`ProgramSegmenter`] can drive, selected by the
/// route's configured [`Container`] (`crate::route::RouteHandle::container`,
/// issue #887): [`Container::Fmp4`] feeds an [`LlHlsSegmenter`] (parts +
/// segments), [`Container::MpegTs`] feeds a [`StreamingTsHlsSegmenter`]
/// (whole self-initialising `.ts` segments only — no parts, no init segment;
/// see that type's own module doc). Kept as one enum (rather than
/// `ProgramSegmenter` being generic) so `drive_program_segmenters` and its
/// `HashMap<ProgramId, ProgramSegmenter>` stay a single concrete type
/// regardless of which container any given route is configured for.
pub(crate) enum AnySegmenter {
    Fmp4(LlHlsSegmenter),
    Ts(StreamingTsHlsSegmenter),
}

impl AnySegmenter {
    fn set_start_time_ns(&mut self, start_ns: u64) {
        match self {
            AnySegmenter::Fmp4(seg) => seg.set_start_time_ns(start_ns),
            AnySegmenter::Ts(seg) => seg.set_start_time_ns(start_ns),
        }
    }

    fn push(&mut self, track_id: TrackId, sample: Sample) -> transmux::Result<()> {
        match self {
            AnySegmenter::Fmp4(seg) => seg.push(track_id, sample),
            AnySegmenter::Ts(seg) => seg.push(track_id, sample),
        }
    }

    /// Finalize any trailing buffered partial segment — `LlHlsSegmenter::flush`
    /// for fMP4, `StreamingTsHlsSegmenter::finish` for classic TS (same job,
    /// different name: see that method's own doc for why it is not named
    /// `flush` there).
    fn part_target(&self) -> Option<Duration> {
        match self {
            AnySegmenter::Fmp4(seg) => Some(Duration::from_millis(u64::from(seg.part_target_ms()))),
            AnySegmenter::Ts(_) => None,
        }
    }

    fn finish(&mut self) -> transmux::Result<()> {
        match self {
            AnySegmenter::Fmp4(seg) => seg.flush(),
            AnySegmenter::Ts(seg) => seg.finish(),
        }
    }

    /// The init segment bytes, fMP4 only — `Container::MpegTs`'s classic TS
    /// segments are self-initialising (in-band PAT/PMT at the head of every
    /// segment; see [`StreamingTsHlsSegmenter`]'s own doc), so this is always
    /// `None` for [`AnySegmenter::Ts`] — mirroring
    /// `hls_runtime::server::HlsOrigin::set_init`'s documented no-op under
    /// [`Container::MpegTs`].
    fn init_segment(&self) -> Option<Vec<u8>> {
        match self {
            AnySegmenter::Fmp4(seg) => match seg.init_segment() {
                Ok(bytes) => Some(bytes),
                Err(e) => {
                    tracing::warn!(error = %e, "driver-backed segmenter init segment build failed");
                    None
                }
            },
            AnySegmenter::Ts(_) => None,
        }
    }

    /// LL-HLS partial segments — always empty for [`AnySegmenter::Ts`], which
    /// has no partial-segment concept at all (see [`AnySegmenter`]'s own doc).
    fn record_part_cuts(&mut self) {
        if let AnySegmenter::Fmp4(seg) = self {
            seg.record_part_cuts();
        }
    }

    fn take_ready_part_cuts(&mut self) -> Vec<PartCutReady> {
        match self {
            AnySegmenter::Fmp4(seg) => seg.take_ready_part_cuts(),
            AnySegmenter::Ts(_) => Vec::new(),
        }
    }

    fn take_ready_parts(&mut self) -> Vec<PartInfo> {
        match self {
            AnySegmenter::Fmp4(seg) => seg.take_ready_parts(),
            AnySegmenter::Ts(_) => Vec::new(),
        }
    }

    /// Closed segments, normalised to [`ReadySegment`] — see that type's own
    /// doc for the fMP4/TS field-mapping differences this bridges.
    fn take_ready_segments(&mut self) -> Vec<ReadySegment> {
        match self {
            AnySegmenter::Fmp4(seg) => seg
                .take_ready_segments()
                .into_iter()
                .map(Into::into)
                .collect(),
            AnySegmenter::Ts(seg) => seg.take_ready().into_iter().map(Into::into).collect(),
        }
    }

    /// Admit a new track into a running segmenter — issue #781: a mid-stream
    /// PMT change adds an elementary stream.
    ///
    /// For [`Container::MpegTs`], [`StreamingTsHlsSegmenter::add_track`]
    /// admits the new track in place; the next muxed `.ts` segment will carry
    /// it in its PMT.
    ///
    /// For [`Container::Fmp4`], [`LlHlsSegmenter`] has no `add_track` (its
    /// track set is frozen at construction) — returns `false` to signal the
    /// caller must rebuild the segmenter at a segment boundary (when no
    /// samples are buffered mid-segment).
    ///
    /// Returns `true` if the track was admitted in place, `false` if a
    /// rebuild is required.
    fn add_track(&mut self, spec: &TrackSpec) -> bool {
        match self {
            AnySegmenter::Fmp4(_) => false,
            AnySegmenter::Ts(seg) => match seg.add_track(spec.clone()) {
                Ok(()) => {
                    tracing::info!(
                        track_id = %spec.track_id,
                        "admitted new track into running TS-HLS segmenter"
                    );
                    true
                }
                Err(e) => {
                    tracing::warn!(
                        error = %e,
                        track_id = %spec.track_id,
                        "failed to admit new track into TS-HLS segmenter"
                    );
                    false
                }
            },
        }
    }

    /// Apply a changed track spec in place (issue #1425). fMP4 cannot, and
    /// reports `Unknown` so the caller keeps its rebuild path.
    fn update_track(
        &mut self,
        spec: &TrackSpec,
    ) -> Result<transmux::UpdateTrackOutcome, transmux::Error> {
        match self {
            AnySegmenter::Fmp4(_) => Ok(transmux::UpdateTrackOutcome::Unknown),
            AnySegmenter::Ts(seg) => match seg.update_track(spec.clone()) {
                Ok(outcome) => {
                    if matches!(outcome, transmux::UpdateTrackOutcome::Changed) {
                        tracing::info!(
                            track_id = %spec.track_id,
                            "applied changed track config to running TS-HLS segmenter"
                        );
                    }
                    Ok(outcome)
                }
                Err(e) => {
                    tracing::warn!(
                        error = %e,
                        track_id = %spec.track_id,
                        "failed to apply changed track config to TS-HLS segmenter"
                    );
                    Err(e)
                }
            },
        }
    }
}

/// One program's segmenting state: a [`SampleCursor`] on its driver-minted
/// [`Trunk`], an [`LlHlsSegmenter`] fed from it, and the *same* `Trunk`'s own
/// [`SegmentWriter`] the resulting parts/segments are published back
/// through.
///
/// `pub(crate)` (not `pub`, issue #805 task 6): the one caller outside this
/// module, `crate::source::advance_route`, is the facade a
/// `crate::registry::SchemeRegistry` `Custom` factory calls instead of
/// touching this type or [`drive_program_segmenters`] directly — see that
/// facade's own doc.
/// What [`ProgramSegmenter::apply_track_change`] did with a track-set change.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum TrackChange {
    /// Admitted in place: the init segment is unchanged.
    Admitted,
    /// The segmenter was rebuilt; the new init serves from this segment.
    Rebuilt { first_segment: SegmentSeq },
    /// The set carries no media the container can mux (empty, or data only).
    NoMuxableMedia,
    /// The change could not be applied; the previous segmenter stays.
    Refused,
}

/// The program has no muxable media: its current segment was closed, a gap
/// placeholder published, and every sample is dropped until a track set the
/// container can mux arrives.
#[derive(Debug, Clone, Copy)]
pub(crate) struct GapState {
    resume_segment: SegmentSeq,
}

/// [`SegmentWriter`] the resulting parts/segments are published back
/// through.
///
/// The per-program fMP4/TS segmenter a driver-backed route runs.
///
/// Public (issue #1083) so an embedder or a test can drive the exact
/// production segmenting path — [`Self::try_new`] → push samples into the
/// program's `Trunk` → [`Self::pump`] → [`Self::flush`] — rather than
/// reimplementing it. Most callers should use the
/// `multimux::source::segment::drive_program_segmenters` facade instead.
pub struct ProgramSegmenter {
    pub config: SegmenterConfig,
    pub parts_produced: usize,
    pub segments_produced: usize,
    pub non_idr_ts_cuts: u64,
    pub(crate) cursor: SampleCursor,
    /// The last [`Self::pump`] stopped at its per-call item budget with samples still queued.
    pub backlogged: bool,
    /// Per track whose declared timescale differs from the `Trunk`'s (an
    /// `audio_profiles` HE-AAC override moves it to the output rate): the
    /// `(trunk, declared)` timescales its samples are re-expressed between
    /// on their way into the segmenter.
    pub(crate) timescale_moves: HashMap<TrackId, (u32, u32)>,
    pub(crate) segment_writer: SegmentWriter,
    pub(crate) seg: AnySegmenter,
    /// The next segment published opens a replacement run, so it carries
    /// `EXT-X-DISCONTINUITY` (RFC 8216bis 6.2.1: a change of timing or
    /// encoding is signalled with it).
    pub(crate) mark_next_discontinuous: bool,
    pub(crate) gap: Option<GapState>,
    /// Cumulative nanoseconds of segment duration published so far by this
    /// segmenter — mirrors `crate::route::RouteHandle`'s own
    /// `next_timeline_ns`, kept per-program here since each program's
    /// timeline is independent.
    pub(crate) next_timeline_ns: u64,
    /// Last-seen [`Trunk::track_generation`] — compared each pump to detect
    /// a mid-stream track-set change (issue #781). When the trunk's
    /// generation advances past this, [`Self::apply_track_change`] admits
    /// new tracks into (or rebuilds) the segmenter.
    pub(crate) track_generation: u64,
    /// The track set the samples being drained were published under: the
    /// trunk's set at construction, replaced at each in-band
    /// [`SampleCursorItem::TracksChanged`]. Never read from the trunk, which
    /// may already be ahead of the cursor.
    pub published_tracks: Arc<[TrackSpec]>,
    /// Set when an fMP4 rebuild re-armed [`Self::start_gate_open`] mid-stream:
    /// the gate then holds back only video, so audio that is not part of the
    /// change keeps flowing.
    gate_video_only: bool,
    /// Parts/segments this segmenter could not hand to `segment_writer` on a
    /// previous call (a segment's `ArchiveOverrun::StallIngest` pin had not
    /// caught up yet), oldest first — see
    /// [`Self::publish_without_blocking`]/[`Self::drain_pending`] for why
    /// this exists: `pump`/`flush` run synchronously on a shared tokio
    /// worker thread (via `drive_program_segmenters`, called directly from
    /// every ingest source's async task loop), so this segmenter must never
    /// call the blocking `SegmentWriter::publish_segment` — a stalled DVR
    /// pin would then park that worker thread for up to
    /// `media_plane::trunk::SegmentWriter`'s bounded wait (issue #1082's T7
    /// finding). Bounded by [`PENDING_PUBLISH_QUEUE_CAP`].
    pub(crate) pending: VecDeque<PendingPublish>,
    /// When the entry currently at the front of `pending` first became
    /// stuck — `None` whenever `pending` is empty. Compared against
    /// [`PENDING_PUBLISH_MAX_WAIT`] in [`Self::drain_pending`].
    pub(crate) pending_since: Option<Instant>,
    /// The mapping from the trunk's internal segment timeline to the
    /// source's own media (90 kHz) clock — see [`MediaAnchor`]. Reported to
    /// the trunk via [`SegmentWriter::note_segment_start`] so
    /// `Trunk::events_in_segment` can attribute inband SCTE-35 events to the
    /// right segment (issue #1083, W9). `None` until the first timed sample.
    pub(crate) anchor: Option<MediaAnchor>,
    /// An anchor to apply at the NEXT segment boundary, not now: a
    /// discontinuity detected mid-segment must not retro-stamp the segment
    /// already being filled with the post-jump clock (item 5). Set while
    /// samples of the in-progress segment are still arriving; consumed by
    /// [`Self::queue_segment`] once that segment closes, so the following
    /// segment is anchored to the first post-jump sample.
    pub(crate) pending_anchor: Option<MediaAnchor>,
    /// Per-track timeline-jump detector (broadcast-client semantics), moved
    /// to transmux.
    pub(crate) timeline: transmux::JumpDetector,
    /// Wall-clock origin for delivery times passed to the detector.
    pub(crate) ingest_origin: Instant,
    /// Start-gate (Apple authoring 7.4 MUST: segments start on a sync/IDR
    /// picture, issue #1425): for a program whose track set contains a video
    /// track, drop every timed sample until the first sync sample of a video
    /// track has been seen. `true` once opened.
    pub(crate) start_gate_open: bool,
    /// Number of timed samples dropped by the start-gate so far.
    pub(crate) start_gate_dropped: u64,
    /// Of those, how many were on a video track.
    pub(crate) start_gate_video_dropped: u64,
    /// Whether the one-shot `tracing::info!` on gate open has fired.
    pub(crate) start_gate_opened_logged: bool,
    /// How long the start gate has dropped IDR-less video while an `idr_only`
    /// classic-TS route waits for its first IDR.
    pub(crate) idr_wait: transmux::IdrWait,
    /// The wait ran out and the route cuts on non-IDR sync samples
    /// (`ts_hls_idr_fallback` permitting).
    pub(crate) idr_fallback_active: bool,
    /// Every sample since the first non-IDR sync sample, kept while the gate
    /// waits for an IDR: when the wait runs out the stream is replayed from
    /// that sync sample, so a stream whose only random-access points came
    /// before the bound still yields its first segment.
    pub(crate) idr_hold: VecDeque<(TrackId, Sample)>,
    pub(crate) idr_hold_bytes: usize,
    /// Separate audio renditions (issue #1428 slice 2b): one follower per
    /// mapped audio track, empty unless the route opts into split audio.
    pub followers: Vec<AudioFollower>,
    /// WebVTT subtitle renditions built from teletext subtitle pages (issue
    /// #1428): one follower per declared page.
    pub subtitle_followers: Vec<SubtitleFollower>,
    /// Duration of each recent leader segment, for sizing gap placeholders.
    pub(crate) leader_durations: LeaderDurations,
    /// Per-track caption detectors (`None` for a track whose codec has no
    /// caption carriage), built lazily and dropped on a track-set change.
    pub(crate) caption_detectors: HashMap<TrackId, Option<CaptionDetector>>,
    /// Every caption stream this segmenter has seen, across track changes.
    pub(crate) captions_seen: BTreeSet<DetectedCaption>,
    pub(crate) captions_declared: Vec<AtscCaptionService>,
    /// `captions_seen` has grown, or the scan window has ended, since the
    /// state was last handed to the route.
    pub(crate) captions_unreported: bool,
    /// First DTS seen per track the caption scan covers.
    pub(crate) caption_scan_started: HashMap<TrackId, i64>,
    /// A full [`CAPTION_SCAN_WINDOW_SECS`] of video has been examined; an
    /// empty `captions_seen` then means the route declares no captions.
    pub(crate) captions_scan_complete: bool,
    /// The leader's `PART-TARGET` last handed to the route's origin.
    pub(crate) part_target_published: Option<Duration>,
}

/// Most sample payload bytes the IDR-fallback hold keeps (32 MiB). Past it the
/// hold is discarded and restarts at the next sync sample, so a stream with
/// no usable random-access point cannot grow it without bound.
/// Media time of video examined for captions before a route that has found
/// none declares `CLOSED-CAPTIONS=NONE`; a later detection replaces it.
const CAPTION_SCAN_WINDOW_SECS: i64 = 10;
const IDR_HOLD_MAX_BYTES: usize = 32 * 1024 * 1024;

/// Maps this segmenter's internal per-segment timeline
/// ([`SegmentEntry::timeline_position`], which starts at 0 for a fresh
/// segmenter and advances by segment durations) onto the **source's** own
/// 90 kHz media clock — the same clock SCTE-35 events are anchored on.
///
/// `source_90k` is the rescaled PTS of a real sample, and `timeline_ns` the
/// internal timeline position at the moment that sample was seen. Every
/// segment's start is then `source_90k + (segment.timeline_position -
/// timeline_ns)`, which is exact for any segment (no per-segment rounding
/// accumulation — the delta is computed once from real values). A PTS
/// discontinuity (or a track change) re-anchors: the anchor is re-taken from
/// the first sample after it, so a jump in the source clock does not smear
/// every later boundary (issue #1083, E).
#[derive(Debug, Clone, Copy)]
pub(crate) struct MediaAnchor {
    source_90k: u64,
    timeline_ns: u64,
}

/// Start-gate safety valve (issue #1425): the gate drops leading video samples
/// until the first sync sample; if a video track flags no sync sample at all
/// within this many video samples (about 30 s at 60 fps) the gate opens
/// anyway (with a warning) instead of dropping the program forever.
pub(crate) const START_GATE_MAX_VIDEO_DROPPED: u64 = 1800;

/// Cursor items one [`ProgramSegmenter::pump`] call processes before it returns, leaving the rest
/// queued for the next call, so the caller can yield to the runtime between batches.
pub const PUMP_BATCH_ITEMS: usize = 512;

/// Rolling-window depth [`StreamingTsHlsSegmenter::new`] is given —
/// irrelevant to a driver-backed route in practice (its own `.playlist()`
/// method, the only consumer of this window, is never called here: the
/// served playlist is rendered from the `Trunk`'s own segment log via
/// `crate::route::ProgramServing`'s `HlsOrigin`, exactly like the fMP4 path).
/// Any positive value works; matches this route's own advertised-window
/// depth (`crate::route::RouteHandle::window_segments_cap`) so memory use is
/// at least bounded consistently with everything else per-route.
fn ts_segmenter_window(config: &SegmenterConfig) -> usize {
    config.window_segments_cap.get()
}

/// Refuse a track a classic MPEG-TS HLS segment may not carry: Apple HLS
/// Authoring Spec 1.5 allows only AVC video in TS, so HEVC and AV1 fail the
/// output rather than producing an invalid stream.
///
/// Fail `route_handle`'s output on the first track [`ts_hls_track_check`]
/// refuses, logging once. `true` when every track is acceptable.
fn ts_hls_tracks_acceptable<'a>(
    config: &SegmenterConfig,
    events: &mut Vec<SegmenterEvent>,
    configs: impl IntoIterator<Item = &'a CodecConfig>,
) -> bool {
    for cfg in configs {
        let (codec, reason) = match cfg {
            transmux::pipeline::CodecConfig::Hevc { .. } => {
                ("HEVC", "HEVC is not allowed in MPEG-TS HLS segments")
            }
            transmux::pipeline::CodecConfig::Av1 { .. } => {
                ("AV1", "AV1 is not allowed in MPEG-TS HLS segments")
            }
            _ => continue,
        };
        let _ = codec;
        events.push(SegmenterEvent::OutputFailed(format!(
            "ts_hls output cannot carry {codec}: {reason}"
        )));
        tracing::error!(route = %config.route_name, "ts_hls output failed: {}", reason);
        return false;
    }
    true
}

/// Whether `sample` of a video track may open the start gate: a sync sample,
/// or under a classic-TS route cutting on IDR only
/// ([`transmux::SegmentCutPolicy::IdrOnly`]) a sample carrying an IDR slice,
/// so the media-time anchor is pinned on the picture the first segment starts
/// with rather than on dropped leading pictures.
fn gate_opens_on(
    config: &SegmenterConfig,
    codec_config: Option<&CodecConfig>,
    sample: &Sample,
    idr_fallback_active: bool,
) -> bool {
    let idr_only = matches!(config.container, Container::MpegTs)
        && config.ts_cut_policy == transmux::SegmentCutPolicy::IdrOnly
        && !idr_fallback_active;
    let codec = match codec_config {
        Some(CodecConfig::Avc { .. }) => Some(transmux::NalCodec::Avc),
        Some(CodecConfig::Hevc { .. }) => Some(transmux::NalCodec::Hevc),
        Some(CodecConfig::Vvc { .. }) => Some(transmux::NalCodec::Vvc),
        _ => None,
    };
    match (idr_only, codec) {
        (true, Some(codec)) => transmux::access_unit_is_idr(codec, &sample.data),
        _ => sample.flags.is_sync,
    }
}

/// A part target as whole milliseconds (the LL-HLS segmenter's own unit).
fn part_target_millis(part_target: Duration) -> u32 {
    u32::try_from(part_target.as_millis()).unwrap_or(u32::MAX)
}

impl ProgramSegmenter {
    /// Start this run's media timeline where `route_handle` says `program`'s
    /// serving begins it (a no-op for the first run on a route): call once,
    /// right after [`Self::try_new`] and the program's publication, before
    /// the first [`Self::pump`].
    pub fn continue_after_reconnect(&mut self, media_start: Duration) {
        if !media_start.is_zero() {
            let start_ns = u64::try_from(media_start.as_nanos()).unwrap_or(u64::MAX);
            self.continue_media_timeline(start_ns);
        }
    }

    /// Start this run's media timeline at `start_ns` instead of zero: the
    /// leader's and every follower's decode clock and the segment timeline.
    pub(crate) fn continue_media_timeline(&mut self, start_ns: u64) {
        if start_ns == 0 {
            return;
        }
        self.seg.set_start_time_ns(start_ns);
        self.next_timeline_ns = start_ns;
        self.mark_next_discontinuous = true;
        for follower in &mut self.followers {
            follower.set_start_time_ns(start_ns);
        }
        for follower in &mut self.subtitle_followers {
            follower.set_start_time_ns(start_ns);
        }
    }

    /// Attempt to start segmenting `trunk`. `None` if its track set hasn't
    /// landed yet (a `NewProgram` announcement with samples pending is
    /// unusual but not impossible — retried on the next call), if its
    /// segment writer has already been taken (must never happen for a
    /// freshly-observed per-program `Trunk`, but defensive rather than
    /// panicking on a driver/caller bug), or if the underlying segmenter
    /// constructor ([`LlHlsSegmenter::with_part_target`] for
    /// [`Container::Fmp4`], [`StreamingTsHlsSegmenter::new`] for
    /// [`Container::MpegTs`]) itself rejects the track set (logged, not
    /// propagated — a segmentation failure on one program must not tear down
    /// the whole ingest session; see `drive_program_segmenters`'s own doc).
    pub fn try_new(
        trunk: &Arc<Trunk>,
        config: SegmenterConfig,
    ) -> (Option<Self>, Vec<SegmenterEvent>) {
        Self::try_new_with_siblings(trunk, config, &())
    }

    pub fn try_new_with_siblings(
        trunk: &Arc<Trunk>,
        config: SegmenterConfig,
        siblings: &dyn SiblingTrunks,
    ) -> (Option<Self>, Vec<SegmenterEvent>) {
        let target_duration_secs = config.target_duration_secs;
        let part_target = config.part_target;
        let (cursor, track_generation, published_tracks) =
            trunk.subscribe_from_backlog_with_tracks();
        let tracks = config.apply_declared_audio_profiles(published_tracks.to_vec());
        if tracks.is_empty() {
            return (None, Vec::new());
        }
        let timescale_moves = timescale_moves(&published_tracks, &tracks);
        let segment_writer = match trunk.segment_writer() {
            Some(w) => w,
            None => return (None, Vec::new()),
        };
        // Seeded from `next_sequence_number` rather than the plain
        // `with_part_target` constructor's implicit `1`: this `Trunk` may
        // already have accepted segments from an earlier `SegmentWriter`
        // that was dropped and re-issued (`media_plane` issue #1082) —
        // `publish_segment` now rejects a `sequence_number` that is not
        // strictly greater than the last one accepted, so this segmenter
        // must resume from there, not renumber from 1 again. For a fresh
        // `Trunk` this is exactly `1`, matching `with_part_target`'s own
        // default.
        let next_seq = segment_writer.next_sequence_number();
        let mapped = if matches!(config.container, Container::Fmp4) {
            config.resolve_audio_sources(&tracks)
        } else {
            Vec::new()
        };
        let mut seg = match config.container {
            Container::Fmp4 => {
                let muxable = leader_tracks(&tracks, &mapped);
                if muxable.is_empty() {
                    return (None, Vec::new());
                }
                match LlHlsSegmenter::with_part_target_at(
                    muxable,
                    transmux::VIDEO_CLOCK_RATE,
                    target_duration_secs,
                    part_target_millis(part_target),
                    next_seq.value(),
                    next_seq.value(),
                ) {
                    Ok(seg) => AnySegmenter::Fmp4(seg),
                    Err(e) => {
                        tracing::warn!(error = %e, "driver-backed fMP4 segmenter build failed");
                        return (None, Vec::new());
                    }
                }
            }
            Container::MpegTs => {
                // Whole seconds, clamped >= 1 (mirrors
                // `StreamingTsHlsSegmenter::new`'s own clamp) — the target
                // duration is configured as `f64` seconds route-wide
                // (`crate::config::Config::target_duration_secs`), but the
                // classic-TS segmenter's cut rule is integer-second, per
                // `transmux::ts_hls`'s own module doc ("no_std-friendly").
                let target_secs = target_duration_secs.round().max(1.0) as u32;
                // Seeded from `next_seq` (see this function's own doc above,
                // on the Fmp4 arm) — a `StreamingTsHlsSegmenter::new` built
                // from the plain, unseeded constructor always numbers its
                // first cut segment `0`; on a `Trunk` that already holds
                // segments (or after an `apply_track_change` rebuild) that
                // is not strictly greater than the last one accepted, so
                // every segment this segmenter ever cuts would be rejected
                // as `NonMonotonicSequenceNumber` and dropped forever
                // (`drain_pending`'s `Err(TryPublishSegmentError::NonMonotonic(_))`
                // arm), silently taking the whole route's TS-HLS output dark.
                let mut events = Vec::new();
                if !ts_hls_tracks_acceptable(&config, &mut events, tracks.iter().map(|t| &t.config))
                {
                    return (None, events);
                }
                match StreamingTsHlsSegmenter::with_start_sequence(
                    tracks.to_vec(),
                    target_secs,
                    ts_segmenter_window(&config),
                    u64::from(next_seq.value().saturating_sub(1)),
                ) {
                    Ok(seg) => AnySegmenter::Ts(seg.with_cut_policy(config.ts_cut_policy)),
                    Err(e) => {
                        tracing::warn!(error = %e, "driver-backed TS-HLS segmenter build failed");
                        return (None, Vec::new());
                    }
                }
            }
            // `Container` is `#[non_exhaustive]`: a future container variant
            // this segmenter has no branch for is refused (logged, not
            // propagated — see this function's own doc), not silently
            // defaulted into one of the two existing branches.
            other => {
                tracing::warn!(
                    ?other,
                    "driver-backed segmenter: no segmenter implementation for this container"
                );
                return (None, Vec::new());
            }
        };
        let followers =
            siblings.build_audio_followers(trunk, &mapped, target_duration_secs, part_target);
        let subtitle_followers = if matches!(config.container, Container::Fmp4) {
            siblings.build_subtitle_followers(trunk, &tracks)
        } else {
            Vec::new()
        };
        if !followers.is_empty() || !subtitle_followers.is_empty() {
            seg.record_part_cuts();
        }
        let mut ps = ProgramSegmenter {
            config: config.clone(),
            parts_produced: 0,
            segments_produced: 0,
            non_idr_ts_cuts: 0,
            backlogged: false,
            timescale_moves,
            // `subscribe_from_backlog`, not `subscribe` (issue #808): the
            // driver's own `feed` call that announced this program's
            // `NewProgram` routinely carries its first samples too (a
            // single MPEG-TS feed batch commonly holds the PMT *and* the
            // first PES packets) — those samples are already sitting in
            // the ring by the time this constructor runs, and a
            // live-tail `subscribe()` cursor would never see them (the
            // opening IDR risks landing in that skipped batch). Replaying
            // whatever backlog the ring still retains is exactly this
            // consumer's shape: it exists to segment everything a program
            // ever publishes, not just what arrives after it happens to be
            // built.
            cursor,
            segment_writer,
            seg,
            mark_next_discontinuous: false,
            gap: None,
            next_timeline_ns: 0,
            track_generation,
            published_tracks,
            gate_video_only: false,
            pending: VecDeque::new(),
            pending_since: None,
            anchor: None,
            pending_anchor: None,
            start_gate_open: false,
            start_gate_dropped: 0,
            start_gate_video_dropped: 0,
            start_gate_opened_logged: false,
            idr_wait: transmux::IdrWait::new(config.ts_idr_fallback.bound(target_duration_secs)),
            idr_fallback_active: false,
            idr_hold: VecDeque::new(),
            idr_hold_bytes: 0,
            followers,
            subtitle_followers,
            leader_durations: LeaderDurations::new(),
            caption_detectors: HashMap::new(),
            captions_seen: BTreeSet::new(),
            captions_declared: Vec::new(),
            captions_unreported: false,
            caption_scan_started: HashMap::new(),
            captions_scan_complete: false,
            part_target_published: None,
            timeline: transmux::JumpDetector::new(Some(transmux::TimelineJumpConfig::default())),
            ingest_origin: Instant::now(),
        };
        ps.timeline.on_tracks(&ps.published_tracks);
        (Some(ps), Vec::new())
    }

    /// Queue `item` (behind anything already queued from an earlier call,
    /// so order is preserved) and immediately try to drain the queue — see
    /// [`Self::drain_pending`]. Never blocks.
    pub(crate) fn publish_without_blocking(&mut self, item: PendingPublish) {
        self.pending.push_back(item);
        self.drain_pending();
    }

    /// Drain [`Self::pending`] from the front for as long as items keep
    /// going through, in order — a live part always succeeds
    /// ([`SegmentWriter::publish_part`] never blocks or rejects), so only a
    /// segment can actually get stuck (its `ArchiveOverrun::StallIngest` pin
    /// not yet caught up); when one does, this stops rather than letting a
    /// later queued item (another segment, or a part that belongs after it)
    /// jump ahead of it.
    ///
    /// Called on every call to [`Self::pump`]/[`Self::flush`] — not only
    /// when this segmenter has something new to publish — so a backlog left
    /// over from an earlier call keeps getting retried even on a tick that
    /// produces nothing new, and the tail is drained (with this same
    /// deadline/expiry rule) at end of stream rather than lost.
    ///
    /// While the front is stuck, tracks how long via [`Self::pending_since`]
    /// and, once [`PENDING_PUBLISH_MAX_WAIT`] elapses **or**
    /// [`PENDING_PUBLISH_QUEUE_CAP`] is reached (memory must stay bounded
    /// even before the deadline, for a pin that never advances at all),
    /// force-expires the blocking pin via
    /// [`SegmentWriter::expire_stalled_pins`] and retries immediately —
    /// applying, non-blockingly, the exact bound
    /// `SegmentWriter::publish_segment`'s own internal blocking wait uses.
    pub(crate) fn drain_pending(&mut self) {
        drain_publish_queue(
            &mut self.segment_writer,
            &mut self.pending,
            &mut self.pending_since,
        );
    }

    /// The init segment bytes — built once, at construction, stable for the
    /// life of the segmenter (mirrors [`LlHlsSegmenter::init_segment`]'s own
    /// doc). Always `None` under [`Container::MpegTs`].
    /// Callers publish it with the host's `set_init` once the program is
    /// registered.
    pub fn init_segment(&self) -> Option<Vec<u8>> {
        self.seg.init_segment()
    }

    /// Drain every sample this program's cursor has observed since the last
    /// call, push it through the segmenter, and publish whatever parts/
    /// segments that produced. Returns `(parts_published, segments_published)`
    /// this call — issue #809: `drive_program_segmenters` sums these across
    /// every segmenter to drive `crate::prometheus::PARTS_PRODUCED_TOTAL`/
    /// `SEGMENTS_PRODUCED_TOTAL`.
    ///
    /// A track-set change (issue #781) arrives in-band as
    /// [`SampleCursorItem::TracksChanged`], ordered against the samples: every
    /// sample before it is segmented under the previous track set, every one
    /// after under the new. The segmenter admits new tracks in place or, for
    /// fMP4 (which has no in-place `add_track`), flushes the old segmenter and
    /// rebuilds it at that boundary; the rebuild gates only video until the new
    /// level's first sync sample, so audio keeps flowing.
    pub fn pump(
        &mut self,
        program: ProgramId,
        trunk: &Trunk,
        events: &mut Vec<SegmenterEvent>,
        siblings: &dyn SiblingTrunks,
    ) -> (usize, usize) {
        let mut totals = (0, 0);
        loop {
            let (parts, segments) = self.pump_batch(program, trunk, events, siblings);
            totals.0 += parts;
            totals.1 += segments;
            if !self.backlogged {
                return totals;
            }
        }
    }

    /// [`Self::pump`] for at most `PUMP_BATCH_ITEMS` queued items, leaving the rest queued and
    /// setting `backlogged` so the caller can yield to the runtime before the next batch.
    pub fn pump_batch(
        &mut self,
        program: ProgramId,
        trunk: &Trunk,
        events: &mut Vec<SegmenterEvent>,
        siblings: &dyn SiblingTrunks,
    ) -> (usize, usize) {
        let mut drained = 0usize;
        self.backlogged = false;
        loop {
            if drained == PUMP_BATCH_ITEMS {
                self.backlogged = true;
                break;
            }
            let Some(item) = self.cursor.poll() else {
                break;
            };
            drained += 1;
            if let SampleCursorItem::TracksChanged { generation, tracks }
            | SampleCursorItem::TracksLagged {
                generation, tracks, ..
            } = item
            {
                if generation > self.track_generation {
                    let previous_tracks = std::mem::replace(&mut self.published_tracks, tracks);
                    let previous_moves = self.timescale_moves.clone();
                    match self.apply_track_change(program, trunk, events, siblings, generation) {
                        TrackChange::Admitted => {
                            events.push(SegmenterEvent::TrackSpecs(self.published_tracks.to_vec()));
                            events.push(SegmenterEvent::TracksActive(true));
                        }
                        TrackChange::Rebuilt { first_segment } => {
                            events.push(SegmenterEvent::BeginTrackSpecs {
                                first_segment,
                                specs: self.published_tracks.to_vec(),
                            });
                            events.push(SegmenterEvent::TracksActive(true));
                        }
                        TrackChange::NoMuxableMedia => {
                            self.published_tracks = previous_tracks;
                            self.timescale_moves = previous_moves;
                            self.enter_gap(program, trunk, events, siblings);
                        }
                        TrackChange::Refused => {
                            self.published_tracks = previous_tracks;
                            self.timescale_moves = previous_moves;
                        }
                    }
                    self.track_generation = generation;
                    self.timeline.on_tracks(&self.published_tracks);
                    self.caption_detectors.clear();
                }
            } else if let SampleCursorItem::Timed { track_id, sample } = item {
                if self.gap.is_some() {
                    continue;
                }
                if self
                    .subtitle_followers
                    .iter()
                    .any(|f| f.source_track_id == track_id)
                {
                    for follower in self
                        .subtitle_followers
                        .iter_mut()
                        .filter(|f| f.source_track_id == track_id)
                    {
                        follower.push(&sample);
                    }
                    continue;
                }
                self.observe_captions(track_id, &sample.data, sample.dts);
                // Start-gate (Apple authoring 7.4 MUST: segments start on a
                // sync/IDR picture, issue #1425): for a program whose track set
                // contains a video track, drop every timed sample until the
                // first sync sample of a video track has been seen. Runs
                // BEFORE the media-anchor block so dropped samples never pin
                // timeline 0.
                let tracks = Arc::clone(&self.published_tracks);
                let has_video_track = tracks.iter().any(|t| t.config.is_video());
                let passes_video_only_gate = self.gate_video_only
                    && !tracks
                        .iter()
                        .find(|t| t.track_id == track_id)
                        .is_some_and(|t| t.config.is_video());
                if has_video_track && !self.start_gate_open && !passes_video_only_gate {
                    let is_video_sample = tracks
                        .iter()
                        .find(|t| t.track_id == track_id)
                        .map(|t| t.config.is_video())
                        .unwrap_or(false);
                    let opens_gate = is_video_sample
                        && gate_opens_on(
                            &self.config,
                            tracks
                                .iter()
                                .find(|t| t.track_id == track_id)
                                .map(|t| &t.config),
                            &sample,
                            self.idr_fallback_active,
                        );
                    // Safety valve: a video track that NEVER flags a sync
                    // sample (a broken or non-keyframe-signalling source)
                    // must not be dropped forever — after
                    // `START_GATE_MAX_VIDEO_DROPPED` video samples (about 30 s
                    // at 60 fps) open the gate anyway, loudly, and let the
                    // segmenter's own cut rules apply as they did before the
                    // gate existed.
                    let valve = is_video_sample
                        && self.start_gate_video_dropped >= START_GATE_MAX_VIDEO_DROPPED;
                    if valve {
                        tracing::warn!(
                            program = program.0,
                            dropped = self.start_gate_dropped,
                            "start gate: no video sync sample after {} video samples; \
                             opening the gate on a non-sync sample",
                            START_GATE_MAX_VIDEO_DROPPED
                        );
                    } else if !opens_gate {
                        self.start_gate_dropped += 1;
                        if is_video_sample {
                            self.start_gate_video_dropped += 1;
                            let timescale = tracks
                                .iter()
                                .find(|t| t.track_id == track_id)
                                .map_or(0, |t| t.timescale);
                            let waiting = self.idr_wait_applies();
                            let tripped =
                                waiting && self.idr_wait.observe_dropped(&sample, timescale);
                            if waiting && self.config.ts_idr_fallback.falls_back() {
                                self.hold_for_fallback(is_video_sample, track_id, sample);
                            }
                            if tripped {
                                self.idr_wait_ran_out(program, events);
                                if !self.idr_hold.is_empty() {
                                    self.start_gate_open = true;
                                    tracing::info!(
                                        program = program.0,
                                        dropped = self.start_gate_dropped,
                                        replayed = self.idr_hold.len(),
                                        "start gate opened on the first non-IDR sync sample after the IDR wait ran out"
                                    );
                                    self.start_gate_opened_logged = true;
                                    self.idr_hold_bytes = 0;
                                    for (held_track, held) in std::mem::take(&mut self.idr_hold) {
                                        self.ingest_sample(held_track, held);
                                    }
                                }
                            }
                        } else if self.idr_wait_applies()
                            && self.config.ts_idr_fallback.falls_back()
                        {
                            self.hold_for_fallback(false, track_id, sample);
                        }
                        continue;
                    }
                    self.start_gate_open = true;
                    self.gate_video_only = false;
                    self.idr_hold.clear();
                    self.idr_hold_bytes = 0;
                    if !self.start_gate_opened_logged {
                        tracing::info!(
                            program = program.0,
                            dropped = self.start_gate_dropped,
                            video_dropped = self.start_gate_video_dropped,
                            "start gate opened on first video sync sample"
                        );
                        self.start_gate_opened_logged = true;
                    }
                }
                self.ingest_sample(track_id, sample);
            } else if let SampleCursorItem::Lagged { skipped } = item {
                tracing::warn!(
                    program = program.0,
                    skipped,
                    "segmenter fell behind the trunk's timed ring; those samples are lost"
                );
            }
            // `Sparse`/`Degraded` items: a section-carried (sparse) track has
            // no place in either segmenter's media segment (fMP4 or classic
            // TS) and `Degraded` is cursor bookkeeping, not media — nothing
            // to feed the segmenter with either way, mirroring `media_plane::ts_session`'s
            // own reference test, which only ever matches
            // `SampleCursorItem::Timed`.
        }
        if self.captions_unreported {
            let languages = self
                .captions_seen
                .iter()
                .filter_map(|caption| {
                    let iso = caption.declared_language(&self.captions_declared)?;
                    Some((caption.instream_id(), bcp47_language(&iso)?))
                })
                .collect();
            let detected_vec: Vec<InstreamId> = self
                .captions_seen
                .iter()
                .map(DetectedCaption::instream_id)
                .collect();
            events.push(SegmenterEvent::ClosedCaptions {
                detected: detected_vec,
                languages,
            });
            self.captions_unreported = false;
        }
        self.publish_part_target(program, events);
        let counts = self.publish_ready();
        self.parts_produced += counts.0;
        self.segments_produced += counts.1;
        counts
    }

    fn publish_part_target(&mut self, _program: ProgramId, events: &mut Vec<SegmenterEvent>) {
        let Some(part_target) = self.seg.part_target() else {
            return;
        };
        if self.part_target_published != Some(part_target) {
            events.push(SegmenterEvent::PartTarget(part_target));
            self.part_target_published = Some(part_target);
        }
    }

    fn observe_captions(&mut self, track_id: TrackId, data: &[u8], dts: Option<i64>) {
        let declared = &mut self.captions_declared;
        let tracks = &self.published_tracks;
        let detector = self.caption_detectors.entry(track_id).or_insert_with(|| {
            let spec = tracks.iter().find(|t| t.track_id == track_id)?;
            let detector = CaptionDetector::for_config(&spec.config)?;
            *declared = declared_caption_services(&spec.es_info_descriptors);
            Some(detector)
        });
        let Some(detector) = detector else {
            return;
        };
        if detector.observe(data) {
            self.captions_seen.extend(detector.detected());
            self.captions_unreported = true;
        }
        if self.captions_scan_complete {
            return;
        }
        let timescale = tracks
            .iter()
            .find(|t| t.track_id == track_id)
            .map(|t| t.timescale);
        let (Some(dts), Some(timescale)) = (dts, timescale) else {
            return;
        };
        let started = *self.caption_scan_started.entry(track_id).or_insert(dts);
        let window_ticks = i64::from(timescale) * CAPTION_SCAN_WINDOW_SECS;
        if dts.saturating_sub(started) >= window_ticks {
            self.captions_scan_complete = true;
            self.captions_unreported = true;
        }
    }

    /// Anchor the media timeline on `sample` and push it through the
    /// segmenter.
    fn ingest_sample(&mut self, track_id: TrackId, sample: Sample) {
        if let Some(follower) = self
            .followers
            .iter_mut()
            .find(|f| f.source_track_id == track_id)
        {
            let sample = in_declared_timescale(&self.timescale_moves, track_id, sample);
            follower.push(sample, &self.leader_durations);
            return;
        }
        // Keep the internal-timeline -> source-90kHz mapping anchored
        // (issue #1083, E): anchor on the first timed sample, and
        // re-anchor on a genuine source-clock discontinuity so a jump
        // in the source PTS is not smeared across every later segment
        // boundary. This is what makes `note_segment_start` (below)
        // report the source clock the SCTE-35 events are on, not the
        // segmenter's own restart-from-zero timeline.
        if let Some(pts) = sample.pts
            && let Some(timescale) = self
                .published_tracks
                .iter()
                .find(|t| t.track_id == track_id)
                .map(|t| t.timescale)
            && timescale > 0
        {
            let now_us =
                u64::try_from(self.ingest_origin.elapsed().as_micros()).unwrap_or(u64::MAX);
            let re_anchor = match self.anchor {
                None => true,
                Some(_) => self.timeline.is_jump(track_id, &sample, None, now_us),
            };
            self.timeline.record(track_id, &sample, None, now_us);
            if re_anchor {
                let fresh = MediaAnchor {
                    source_90k: rescale_to_90k(pts, timescale),
                    timeline_ns: self.next_timeline_ns,
                };
                if self.anchor.is_none() {
                    // No anchor at all yet: this IS the first
                    // segment's clock, so it applies immediately.
                    self.anchor = Some(fresh);
                } else {
                    // A discontinuity mid-stream. The segment being
                    // filled right now must keep the OLD anchor (its
                    // start really was before the jump); the new one
                    // takes effect at the next segment boundary
                    // (item 5).
                    self.pending_anchor = Some(fresh);
                }
            }
        }
        let sample = in_declared_timescale(&self.timescale_moves, track_id, sample);
        if let Err(e) = self.seg.push(track_id, sample) {
            tracing::warn!(error = %e, "driver-backed segmenter push failed");
        }
    }

    /// Keep `sample` for the IDR-fallback replay: collection starts at the
    /// first non-IDR sync sample of a video track and takes every sample after
    /// it, so the replay opens on the oldest random-access point the wait saw.
    fn hold_for_fallback(&mut self, is_video: bool, track_id: TrackId, sample: Sample) {
        if self.idr_hold.is_empty() && !(is_video && sample.flags.is_sync) {
            return;
        }
        self.idr_hold_bytes = self.idr_hold_bytes.saturating_add(sample.data.len());
        self.idr_hold.push_back((track_id, sample));
        if self.idr_hold_bytes > IDR_HOLD_MAX_BYTES {
            self.idr_hold.clear();
            self.idr_hold_bytes = 0;
        }
    }

    /// Whether this route is a classic-TS output cutting on IDR only, so a
    /// stream without IDR pictures is a stall rather than normal operation.
    fn idr_wait_applies(&self) -> bool {
        matches!(self.config.container, Container::MpegTs)
            && self.config.ts_cut_policy == transmux::SegmentCutPolicy::IdrOnly
            && !self.idr_fallback_active
    }

    /// The IDR wait passed its bound: fall back to non-IDR sync samples when
    /// the route allows it, and tell the operator either way (once per route:
    /// log, `multimux_ts_hls_non_idr_total`, admin `ts_hls_idr`).
    fn idr_wait_ran_out(&mut self, _program: ProgramId, events: &mut Vec<SegmenterEvent>) {
        let falls_back = self.config.ts_idr_fallback.falls_back();
        let status = if falls_back {
            TsHlsIdrStatus::FellBack
        } else {
            TsHlsIdrStatus::Awaiting
        };
        if falls_back {
            self.idr_fallback_active = true;
            if let AnySegmenter::Ts(seg) = &mut self.seg {
                seg.fall_back_to_any_rap();
            }
        }
        self.non_idr_ts_cuts += 1;
        events.push(SegmenterEvent::TsIdrMissing(status));
    }

    /// Apply a mid-stream track-set change detected via
    /// [`Trunk::track_generation`] advancing past `self.track_generation`
    /// (issue #781).
    ///
    /// For [`Container::MpegTs`], [`StreamingTsHlsSegmenter::add_track`]
    /// admits new tracks into the running segmenter in place — the next
    /// muxed segment carries them. Removals are handled by the trunk alone
    /// (samples stop arriving for the removed track).
    ///
    /// For [`Container::Fmp4`], [`LlHlsSegmenter`] has no `add_track` so
    /// the segmenter is rebuilt from the trunk's current track set. Rebuilding
    /// must NOT reset segment numbering or the media sequence — those live
    /// in the `Trunk`'s `SegmentLog`, which is never touched here (only the
    /// `LlHlsSegmenter`'s internal `next_seq`/`current_segment` state gets
    /// fresh, but since the `Trunk` already has the published segments, the
    /// served playlist sees no discontinuity). The rebuild runs only when
    /// no samples are buffered (i.e. at a segment boundary); if the previous
    /// segmenter still holds buffered samples, they are flushed first via
    /// [`Self::pump`]'s normal drain→cut cycle, and the generation change
    /// is detected on the NEXT call (post-boundary).
    pub(crate) fn apply_track_change(
        &mut self,
        _program: ProgramId,
        trunk: &Trunk,
        events: &mut Vec<SegmenterEvent>,
        siblings: &dyn SiblingTrunks,
        generation: u64,
    ) -> TrackChange {
        let new_tracks = self
            .config
            .apply_declared_audio_profiles(self.published_tracks.to_vec());
        self.timescale_moves = timescale_moves(&self.published_tracks, &new_tracks);
        tracing::info!(
            generation,
            track_count = new_tracks.len(),
            "mid-stream track-set change detected — admitting into segmenter"
        );

        // TS adds each new track individually; fMP4 rebuilds below with the complete set.
        let is_ts = matches!(self.config.container, Container::MpegTs);
        let new_specs: Vec<TrackSpec> = if is_ts {
            new_tracks
                .iter()
                .filter(|t| {
                    ts_hls_tracks_acceptable(&self.config, events, std::iter::once(&t.config))
                })
                .cloned()
                .collect()
        } else {
            Vec::new()
        };

        if is_ts && new_specs.is_empty() {
            tracing::warn!("mid-stream track-set change left no TS-muxable track — entering a gap");
            return TrackChange::NoMuxableMedia;
        }

        let mut any_admitted = false;
        for spec in new_specs.iter().filter(|_| self.gap.is_none()) {
            match self.seg.update_track(spec) {
                Ok(transmux::UpdateTrackOutcome::Unchanged) => {}
                Ok(transmux::UpdateTrackOutcome::Unknown) => {
                    any_admitted |= self.seg.add_track(spec);
                }
                Ok(_) => any_admitted = true,
                Err(_) => {
                    any_admitted |= self.seg.add_track(spec);
                }
            }
        }

        if matches!(self.config.container, Container::Fmp4) || !any_admitted {
            let mut first_segment = None;
            // Rebuild the segmenter from the trunk's current complete
            // track set. The previous segmenter's buffered samples have
            // already been flushed by `pump` (which calls this before
            // draining the cursor, and the previous drain already cut any
            // due segment). The new segmenter starts fresh with the
            // updated track set.
            match self.config.container {
                Container::Fmp4 => {
                    // Only the tracks an fMP4 segmenter can carry, exactly as
                    // `try_new` filters them: a Data/Subtitle PID joining a
                    // DVB source mid-stream (a routine PMT version bump) must
                    // not make the rebuild fail (audit r07-C7, #1083).
                    let mapped = self.config.resolve_audio_sources(&new_tracks);
                    let muxable = leader_tracks(&new_tracks, &mapped);
                    if muxable.is_empty() {
                        tracing::warn!(
                            "mid-stream track-set change left no fMP4-muxable track — \
                             entering a gap"
                        );
                        return TrackChange::NoMuxableMedia;
                    }
                    // Flush the old segmenter in place, drain its output and
                    // read its next sequence numbers — the replacement
                    // resumes from there so segment numbering is strictly
                    // monotonic (issue #781 BLOCKER). The old segmenter stays
                    // installed until the replacement is built, so a failed
                    // rebuild leaves a working fMP4 segmenter, never a
                    // placeholder of another container.
                    let AnySegmenter::Fmp4(old_seg) = &mut self.seg else {
                        return TrackChange::Refused;
                    };
                    if let Err(e) = old_seg.flush() {
                        tracing::warn!(error = %e, "old segmenter flush on rebuild failed");
                    }
                    // Stream order, not "all parts then all segments" — see
                    // `Self::publish_in_stream_order`'s own doc for why the
                    // latter can publish a later segment's opening part
                    // ahead of an earlier segment's own close.
                    let parts = old_seg.take_ready_parts();
                    let segments: Vec<ReadySegment> = old_seg
                        .take_ready_segments()
                        .into_iter()
                        .map(Into::into)
                        .collect();
                    let (next_seq, mut current_seg) = old_seg.next_sequence_numbers();
                    if let Some(gap) = self.gap {
                        current_seg = current_seg.max(gap.resume_segment.value());
                    }
                    let part_cuts = old_seg.take_ready_part_cuts();
                    self.note_leader_segments(&segments);
                    let leader_parts = self.leader_parts(&part_cuts, &parts);
                    let last_flushed = segments.iter().map(|s| s.segment_seq.value()).max();
                    self.publish_in_stream_order(parts, segments);
                    self.feed_followers(part_cuts, &leader_parts);
                    self.finish_followers();
                    let last_queued = self
                        .last_queued_segment(trunk)
                        .into_iter()
                        .chain(last_flushed)
                        .max();
                    let new_init_first_segment =
                        SegmentSeq::new(last_queued.unwrap_or(0)).saturating_next();
                    first_segment = Some(new_init_first_segment);

                    match LlHlsSegmenter::with_part_target_at(
                        muxable,
                        transmux::VIDEO_CLOCK_RATE,
                        self.config.target_duration_secs,
                        part_target_millis(self.config.part_target),
                        next_seq,
                        current_seg,
                    ) {
                        Ok(seg) => {
                            if let Ok(init) = seg.init_segment() {
                                events.push(SegmenterEvent::InitFrom {
                                    init,
                                    first_segment: new_init_first_segment,
                                });
                            }
                            self.seg = AnySegmenter::Fmp4(seg);
                            self.part_target_published = None;
                            self.followers.clear();
                            self.followers = siblings.build_audio_followers(
                                trunk,
                                &mapped,
                                self.config.target_duration_secs,
                                self.config.part_target,
                            );
                            self.subtitle_followers.clear();
                            self.subtitle_followers =
                                siblings.build_subtitle_followers(trunk, &new_tracks);
                            if !self.followers.is_empty() || !self.subtitle_followers.is_empty() {
                                self.seg.record_part_cuts();
                            }
                            // Re-arm the start-gate for the rebuilt track set
                            // (issue #1425): a fresh segmenter must not emit a
                            // segment until the first video sync sample again.
                            self.start_gate_open = false;
                            self.gate_video_only = true;
                            self.start_gate_dropped = 0;
                            self.start_gate_video_dropped = 0;
                            self.start_gate_opened_logged = false;
                            tracing::info!(
                                "fMP4 segmenter rebuilt with updated track set for \
                                 mid-stream addition (resuming seq={next_seq} seg={current_seg})"
                            );
                        }
                        Err(e) => {
                            tracing::warn!(
                                error = %e,
                                "fMP4 segmenter rebuild failed — keeping the current \
                                 segmenter, track change dropped"
                            );
                            return TrackChange::Refused;
                        }
                    }
                }
                Container::MpegTs => {
                    // TS segmenter: the rebuild path. Should only be reached
                    // if add_track failed on every new track (unusual).
                    //
                    // Seeded from `self.segment_writer.next_sequence_number()`
                    // — same reasoning as `try_new`'s own Ts arm: the plain
                    // `StreamingTsHlsSegmenter::new` always numbers its first
                    // cut segment `0`, which this `Trunk` (already holding
                    // segments from the *old* segmenter this rebuild is
                    // replacing) rejects as non-monotonic forever after.
                    let mut next_seq = self.segment_writer.next_sequence_number();
                    if let Some(gap) = self.gap {
                        next_seq = next_seq.max(gap.resume_segment);
                    }
                    match StreamingTsHlsSegmenter::with_start_sequence(
                        new_tracks.to_vec(),
                        self.config.target_duration_secs.round().max(1.0) as u32,
                        ts_segmenter_window(&self.config),
                        u64::from(next_seq.value().saturating_sub(1)),
                    ) {
                        Ok(seg) => {
                            let mut seg = seg.with_cut_policy(self.config.ts_cut_policy);
                            if self.idr_fallback_active {
                                seg.fall_back_to_any_rap();
                            }
                            let replaced = std::mem::replace(&mut self.seg, AnySegmenter::Ts(seg));
                            if let AnySegmenter::Ts(mut old) = replaced {
                                if let Err(e) = old.finish() {
                                    tracing::warn!(error = %e, "old TS segmenter finish on rebuild failed");
                                }
                                let segments: Vec<ReadySegment> =
                                    old.take_ready().into_iter().map(Into::into).collect();
                                self.publish_in_stream_order(Vec::new(), segments);
                            }
                            // Re-arm the start-gate for the rebuilt track set
                            // (issue #1425).
                            self.start_gate_open = false;
                            self.start_gate_dropped = 0;
                            self.start_gate_video_dropped = 0;
                            self.start_gate_opened_logged = false;
                            self.idr_wait = transmux::IdrWait::new(self.idr_wait.bound());
                            self.idr_hold.clear();
                            self.idr_hold_bytes = 0;
                            tracing::info!(
                                "TS segmenter rebuilt with updated track set for \
                                 mid-stream addition"
                            );
                        }
                        Err(e) => {
                            tracing::warn!(
                                error = %e,
                                "TS segmenter rebuild failed — track addition dropped"
                            );
                            return TrackChange::Refused;
                        }
                    }
                }
                // `#[non_exhaustive]`: a future container has no rebuild
                // path — log and carry on.
                _ => {
                    tracing::warn!(
                        "mid-stream track-set change on an unsupported container — \
                         new tracks will not be served"
                    );
                    return TrackChange::Refused;
                }
            }
            if self.gap.take().is_some() {
                self.mark_next_discontinuous = true;
            }
            return first_segment.map_or(TrackChange::Admitted, |first_segment| {
                TrackChange::Rebuilt { first_segment }
            });
        }
        TrackChange::Admitted
    }

    fn last_queued_segment(&self, trunk: &Trunk) -> Option<u32> {
        self.pending
            .iter()
            .filter_map(|p| match p {
                PendingPublish::Segment(entry) => Some(entry.sequence_number.value()),
                _ => None,
            })
            .chain(trunk.last_closed_segment().map(SegmentSeq::value))
            .max()
    }

    fn enter_gap(
        &mut self,
        _program: ProgramId,
        trunk: &Trunk,
        events: &mut Vec<SegmenterEvent>,
        _siblings: &dyn SiblingTrunks,
    ) {
        events.push(SegmenterEvent::TracksActive(false));
        if self.gap.is_some() {
            return;
        }
        self.flush();
        let seq = SegmentSeq::new(self.last_queued_segment(trunk).unwrap_or(0)).saturating_next();
        let duration = Duration::from_secs_f64(self.config.target_duration_secs);
        let start_ns = self.next_timeline_ns;
        self.next_timeline_ns = self
            .next_timeline_ns
            .saturating_add(u64::try_from(duration.as_nanos()).unwrap_or(u64::MAX));
        if !self.followers.is_empty() || !self.subtitle_followers.is_empty() {
            self.leader_durations
                .insert(seq.value(), duration.as_secs_f64());
        }
        self.publish_without_blocking(PendingPublish::Segment(SegmentEntry::new(
            Vec::new(),
            seq,
            duration,
            Timestamp::from_nanos(start_ns),
            SegmentMeta {
                discontinuous: false,
            },
        )));
        self.gap = Some(GapState {
            resume_segment: seq.saturating_next(),
        });
    }

    /// Finalize any trailing buffered partial segment — called once the
    /// driver reaches a terminal health state, so a
    /// disconnecting driver-backed route doesn't silently drop its last
    /// partial segment the way the pre-flush `run_pipeline` regression once
    /// did (see that module's `eos_flush_emits_buffered_tail_segment` test).
    /// Returns `(parts_published, segments_published)` this call — see
    /// [`Self::pump`].
    pub fn parts_produced(&self) -> usize {
        self.parts_produced
    }
    pub fn segments_produced(&self) -> usize {
        self.segments_produced
    }
    pub fn non_idr_ts_cuts(&self) -> u64 {
        self.non_idr_ts_cuts
    }

    pub fn flush(&mut self) -> (usize, usize) {
        if let Err(e) = self.seg.finish() {
            tracing::warn!(error = %e, "driver-backed segmenter flush failed");
        }
        let counts = self.publish_ready();
        self.parts_produced += counts.0;
        self.segments_produced += counts.1;
        self.finish_followers();
        counts
    }

    fn finish_followers(&mut self) {
        for follower in &mut self.followers {
            follower.finish(&self.leader_durations);
        }
        for follower in &mut self.subtitle_followers {
            follower.finish(&self.leader_durations);
        }
    }

    /// Retries [`Self::pending`] first (issue #1082 T7: this must happen on
    /// every call — `pump`'s and `flush`'s own docs — not only when there is
    /// something new to publish, so a backlog left over from an earlier
    /// call keeps draining even on an otherwise-idle tick, and the tail is
    /// drained rather than lost at end of stream), then queues whatever
    /// this segmenter has newly finished producing, in stream order — see
    /// [`Self::publish_in_stream_order`].
    pub(crate) fn publish_ready(&mut self) -> (usize, usize) {
        self.drain_pending();
        let parts = self.seg.take_ready_parts();
        let segments = self.seg.take_ready_segments();
        let part_cuts = self.seg.take_ready_part_cuts();
        self.note_leader_segments(&segments);
        let leader_parts = self.leader_parts(&part_cuts, &parts);
        let counts = self.publish_in_stream_order(parts, segments);
        self.feed_followers(part_cuts, &leader_parts);
        counts
    }

    fn note_leader_segments(&mut self, segments: &[ReadySegment]) {
        if self.followers.is_empty() && self.subtitle_followers.is_empty() {
            return;
        }
        for segment in segments {
            self.leader_durations
                .insert(segment.segment_seq.value(), segment.duration);
        }
        while self.leader_durations.len() > LEADER_DURATION_HISTORY {
            self.leader_durations.pop_first();
        }
    }

    /// The cuts paired with the duration the leader's own part has, for the
    /// subtitle followers: their parts carry the video playlist's durations.
    fn leader_parts(&self, cuts: &[PartCutReady], parts: &[PartInfo]) -> Vec<LeaderPart> {
        if self.subtitle_followers.is_empty() {
            return Vec::new();
        }
        cuts.iter()
            .filter_map(|cut| {
                let part = parts
                    .iter()
                    .find(|p| p.segment_seq == cut.msn && p.part_index == cut.part_index)?;
                Some(LeaderPart {
                    cut: *cut,
                    duration_secs: part.duration,
                })
            })
            .collect()
    }

    fn feed_followers(&mut self, cuts: Vec<PartCutReady>, leader_parts: &[LeaderPart]) {
        for follower in &mut self.subtitle_followers {
            follower.on_part_cuts(leader_parts, &self.leader_durations);
        }
        if cuts.is_empty() {
            return;
        }
        for follower in &mut self.followers {
            follower.on_part_cuts(cuts.clone(), &self.leader_durations);
        }
    }

    /// Queue one part, converting it to the `Trunk`'s [`PartEntry`] shape.
    fn queue_part(&mut self, part: PartInfo) {
        self.publish_without_blocking(PendingPublish::Part(part_entry(part)));
    }

    /// Queue one closed segment, converting it to the `Trunk`'s
    /// [`SegmentEntry`] shape and advancing [`Self::next_timeline_ns`] by
    /// its duration.
    pub(crate) fn queue_segment(&mut self, segment: ReadySegment) {
        let duration = Duration::from_secs_f64(segment.duration);
        let start_ns = self.next_timeline_ns;
        // Tell the trunk where this segment starts on the media (90 kHz)
        // clock, so `Trunk::events_in_segment` can attribute inband SCTE-35
        // events to it (issue #1083, W9 — no production caller of
        // `note_segment_start` existed before this). The start is derived
        // from the real anchor plus this segment's exact internal-timeline
        // offset — no per-segment rounding accumulation (issue #1083, E).
        if let Some(anchor) = self.anchor {
            let delta_ns = self.next_timeline_ns.saturating_sub(anchor.timeline_ns);
            let delta_90k = rescale_u64(delta_ns, NANOS_PER_SEC, PTS_HZ, Rounding::Floor);
            let start_90k = anchor.source_90k.saturating_add(delta_90k);
            self.segment_writer
                .note_segment_start(segment.segment_seq, timed_metadata::MediaTime(start_90k));
        }
        // A discontinuity recorded while this segment was filling becomes
        // effective for the NEXT one (item 5): its `timeline_ns` is this
        // segment's end, so the next segment's start maps exactly onto the
        // post-jump source clock.
        if let Some(mut pending) = self.pending_anchor.take() {
            pending.timeline_ns = self
                .next_timeline_ns
                .saturating_add(u64::try_from(duration.as_nanos()).unwrap_or(u64::MAX));
            self.anchor = Some(pending);
        }
        self.next_timeline_ns = self
            .next_timeline_ns
            .saturating_add(u64::try_from(duration.as_nanos()).unwrap_or(u64::MAX));
        let discontinuous =
            segment.discontinuous || std::mem::take(&mut self.mark_next_discontinuous);
        self.publish_without_blocking(PendingPublish::Segment(SegmentEntry::new(
            segment.bytes,
            segment.segment_seq,
            duration,
            Timestamp::from_nanos(start_ns),
            SegmentMeta { discontinuous },
        )));
    }

    /// Queue `parts` and `segments` interleaved by `segment_seq`, in the
    /// order they actually occurred on the wire — parts of segment N, then
    /// segment N's own close, then parts of segment N+1, and so on — rather
    /// than every ready part followed by every ready segment.
    ///
    /// The latter (this method's predecessor) queues `take_ready_parts()`'s
    /// *entire* batch before touching a single entry of
    /// `take_ready_segments()`. When a call's batch spans a segment
    /// boundary — the segmenter has both cut segment N *and* already
    /// produced segment N+1's opening part before this method next runs —
    /// that ordering publishes part (N+1, 0) into the `Trunk`'s live-part
    /// log (through [`Self::publish_without_blocking`], which never waits
    /// for anything ahead of it in [`Self::pending`]) before segment N's own
    /// [`SegmentEntry`] has even been queued, let alone published. A part
    /// ring reader that assumes "no part of segment N+1 precedes segment
    /// N's close" (the shape [`media_plane::trunk::Trunk::parts_in_segment`]
    /// used to rely on) then loses segment N's still-open parts entirely —
    /// and, in LL-HLS terms, the just-closing segment's own
    /// `#EXT-X-PART` tags silently vanish from the playlist.
    ///
    /// `parts` and `segments` are each already in production order
    /// (ascending `segment_seq`, and ascending `part_index` within a
    /// `segment_seq`) — a merge by `segment_seq` (a part with `segment_seq`
    /// `<=` the next unqueued segment's `segment_seq` goes first, so a
    /// segment's own trailing parts are queued immediately before its
    /// close) recovers the true interleaving without needing any additional
    /// timestamp.
    pub(crate) fn publish_in_stream_order(
        &mut self,
        parts: Vec<PartInfo>,
        segments: Vec<ReadySegment>,
    ) -> (usize, usize) {
        let mut parts = parts.into_iter().peekable();
        let mut segments = segments.into_iter().peekable();
        let mut parts_published = 0usize;
        let mut segments_published = 0usize;
        loop {
            let take_part = match (parts.peek(), segments.peek()) {
                (Some(part), Some(segment)) => part.segment_seq <= segment.segment_seq.value(),
                (Some(_), None) => true,
                (None, Some(_)) => false,
                (None, None) => break,
            };
            if take_part {
                self.queue_part(parts.next().expect("peeked Some above"));
                parts_published += 1;
            } else {
                self.queue_segment(segments.next().expect("peeked Some above"));
                segments_published += 1;
            }
        }
        (parts_published, segments_published)
    }
}

fn in_declared_timescale(
    moves: &HashMap<TrackId, (u32, u32)>,
    track_id: TrackId,
    sample: Sample,
) -> Sample {
    match moves.get(&track_id) {
        Some(&(from, to)) => sample.rescale_timescale(from, to),
        None => sample,
    }
}

fn timescale_moves(
    published: &[TrackSpec],
    declared: &[TrackSpec],
) -> HashMap<TrackId, (u32, u32)> {
    declared
        .iter()
        .filter_map(|d| {
            let from = published
                .iter()
                .find(|t| t.track_id == d.track_id)?
                .timescale;
            (from != d.timescale && from > 0).then_some((d.track_id, (from, d.timescale)))
        })
        .collect()
}

/// Rescale a timestamp in `timescale` ticks to the trunk event ring's fixed
/// 90 kHz clock (issue #1083, W9). Rounds down, saturating — a value that
/// overflows `u64` is clamped, never wrapped.
fn rescale_to_90k(pts: i64, timescale: u32) -> u64 {
    // A negative PTS (a pre-roll/PTS-base offset a demuxer may produce) is
    // clamped to 0: the event ring has no representation for a time before
    // zero, and a segment starting there simply begins the timeline.
    rescale_u64(
        u64::try_from(pts).unwrap_or(0),
        timescale,
        PTS_HZ,
        Rounding::Floor,
    )
}

#[cfg(test)]
mod segmenter_tests {
    use super::*;
    use crate::ids::PartIndex;
    use crate::publish_queue::{PENDING_PUBLISH_MAX_WAIT, PENDING_PUBLISH_QUEUE_CAP};
    use crate::trunk::{
        ArchiveOverrun, EventAnchor, PartEntry, RetentionClass, SegmentCursorItem, TrunkConfig,
    };
    use timed_metadata::MediaTime;
    use transmux::pipeline::CodecConfig;

    pub(super) const FRAME_DUR: u32 = transmux::VIDEO_CLOCK_RATE / 30;
    pub(super) const GOP: u32 = 30;

    pub(super) fn nz(n: usize) -> std::num::NonZeroUsize {
        std::num::NonZeroUsize::new(n).expect("test capacity must be non-zero")
    }

    pub(super) fn sample_at(i: u32, is_sync: bool) -> Sample {
        let nal = [0x65u8, 0xAAu8.wrapping_add(i as u8), (i % 256) as u8];
        let mut data = (nal.len() as u32).to_be_bytes().to_vec();
        data.extend_from_slice(&nal);
        Sample::new(
            data,
            Some(i64::from(i) * i64::from(FRAME_DUR)),
            Some(i64::from(i) * i64::from(FRAME_DUR)),
            Some(FRAME_DUR),
            is_sync,
        )
    }

    /// A real H.264 video track at 90 kHz, as `media_plane::ts_session`'s
    /// test support builds it.
    pub(super) fn track_spec(track_id: TrackId) -> TrackSpec {
        crate::ts_session::test_support::track_spec(track_id)
    }

    /// A Data track (a DVB SCTE-35/teletext PID) that cannot ride in fMP4.
    pub(super) fn data_track_spec(track_id: TrackId) -> TrackSpec {
        TrackSpec::new(
            track_id,
            90_000,
            CodecConfig::Data {
                stream_type: 0x86,
                descriptors: Vec::new(),
                carriage: transmux::pipeline::DataCarriage::Sections,
            },
        )
    }

    pub(super) fn fake_ready_entry(seq: u32) -> SegmentEntry {
        SegmentEntry::new(
            bytes::Bytes::from(vec![seq as u8; 4]),
            SegmentSeq::new(seq),
            Duration::from_secs(1),
            Timestamp::from_nanos(u64::from(seq)),
            SegmentMeta {
                discontinuous: false,
            },
        )
    }

    /// A helper for these low-level tests: build a `ProgramSegmenter`
    /// directly (bypassing `try_new`'s real TS/fMP4 pipeline) over a
    /// caller-supplied `Trunk`, so a test can control the `Trunk`'s segment
    /// capacity and pins precisely.
    pub(super) fn bare_segmenter(trunk: &Arc<Trunk>) -> ProgramSegmenter {
        let segment_writer = trunk.segment_writer().expect("first take succeeds");
        let seg = AnySegmenter::Ts(
            StreamingTsHlsSegmenter::new(vec![track_spec(TrackId::new(1))], 1, 8)
                .expect("valid placeholder TS segmenter"),
        );
        let mut ps = ProgramSegmenter {
            config: SegmenterConfig::default(),
            parts_produced: 0,
            segments_produced: 0,
            non_idr_ts_cuts: 0,
            backlogged: false,
            timescale_moves: HashMap::new(),
            cursor: trunk.subscribe_from_backlog(),
            segment_writer,
            seg,
            mark_next_discontinuous: false,
            gap: None,
            next_timeline_ns: 0,
            track_generation: trunk.track_generation(),
            published_tracks: trunk.tracks(),
            gate_video_only: false,
            pending: VecDeque::new(),
            pending_since: None,
            anchor: None,
            pending_anchor: None,
            start_gate_open: false,
            start_gate_dropped: 0,
            start_gate_video_dropped: 0,
            start_gate_opened_logged: false,
            idr_wait: transmux::IdrWait::new(std::time::Duration::from_secs(6)),
            idr_fallback_active: false,
            idr_hold: VecDeque::new(),
            idr_hold_bytes: 0,
            followers: Vec::new(),
            subtitle_followers: Vec::new(),
            leader_durations: LeaderDurations::new(),
            caption_detectors: HashMap::new(),
            captions_seen: BTreeSet::new(),
            captions_declared: Vec::new(),
            captions_unreported: false,
            caption_scan_started: HashMap::new(),
            captions_scan_complete: false,
            part_target_published: None,
            timeline: transmux::JumpDetector::new(Some(transmux::TimelineJumpConfig::default())),
            ingest_origin: Instant::now(),
        };
        ps.timeline.on_tracks(&ps.published_tracks);
        ps
    }

    /// Pre-populate `trunk`'s segment log with segments `1..=5`, as if
    /// published by an earlier `SegmentWriter`.
    pub(super) fn pre_seed_five_segments(trunk: &Arc<Trunk>) {
        let seeding_writer = trunk.segment_writer().expect("first take succeeds");
        for seq in 1..=5u32 {
            seeding_writer
                .publish_segment(fake_ready_entry(seq))
                .expect("pre-seed accepted");
        }
    }

    pub(super) fn pending_sequence_numbers(segmenter: &ProgramSegmenter) -> Vec<u32> {
        segmenter
            .pending
            .iter()
            .map(|item| match item {
                PendingPublish::Segment(e) => e.sequence_number.value(),
                PendingPublish::Part(p) => p.segment_number.value(),
            })
            .collect()
    }

    pub(super) const NO_SIBLINGS: () = ();

    /// Build a segmenter for `trunk` with the route-like config a test wants.
    pub(super) fn segmenter_for(trunk: &Arc<Trunk>, config: SegmenterConfig) -> ProgramSegmenter {
        let (segmenter, events) =
            ProgramSegmenter::try_new_with_siblings(trunk, config, &NO_SIBLINGS);
        assert!(
            events.is_empty(),
            "a healthy build emits no constructor events"
        );
        segmenter.expect("segmenter builds")
    }

    /// A trunk sized like the start-gate tests want (a 64-frame sample ring)
    /// carrying one real H.264 video track.
    pub(super) fn video_trunk() -> (Arc<Trunk>, crate::trunk::TrunkWriter) {
        let trunk = Trunk::new(
            TrunkConfig::default()
                .with_timed_capacity(nz(64))
                .with_sparse_capacity(nz(64))
                .with_segment_capacity(nz(16))
                .with_event_capacity(nz(8))
                .with_part_capacity(nz(8)),
        );
        let writer = trunk.writer().expect("writer");
        writer.set_tracks(vec![track_spec(TrackId::new(1))]);
        (trunk, writer)
    }

    /// The AAC track spec of the public `h264-aac.ts` capture, renumbered.
    pub(super) fn aac_track_spec(track_id: TrackId) -> TrackSpec {
        use broadcast_common::mux::Unpackage;
        let ts = test_fixtures::read("ts/h264-aac.ts");
        let media = transmux::TsDemux::new().unpackage(&ts).expect("demux TS");
        let mut spec = media
            .tracks
            .iter()
            .find(|t| matches!(t.spec.config, CodecConfig::Aac { .. }))
            .expect("fixture has an AAC track")
            .spec
            .clone();
        spec.track_id = track_id;
        spec
    }

    pub(super) fn fmp4_config() -> SegmenterConfig {
        SegmenterConfig::default()
            .with_container(Container::Fmp4)
            .with_target_duration_secs(1.0)
            .with_part_target(Duration::from_millis(250))
    }

    pub(super) fn ts_config() -> SegmenterConfig {
        SegmenterConfig::default()
            .with_container(Container::MpegTs)
            .with_target_duration_secs(1.0)
            .with_part_target(Duration::from_millis(250))
    }

    #[test]
    pub(super) fn a_zero_timescale_rescales_without_dividing_by_zero() {
        assert_eq!(rescale_to_90k(7, 0), 7 * 90_000);
    }

    /// Issue #1083 W9: the production segmenter must call
    /// [`SegmentWriter::note_segment_start`], so the trunk's per-segment
    /// event attribution (`Trunk::events_in_segment`) is non-empty for a
    /// segment a real SCTE-35 event falls inside. Before this fix nothing in
    /// production ever called `note_segment_start`, so the DASH `emsg`
    /// injection path (`crate::origin::resource::inject_segment_events`) saw
    /// an empty event list for every real segment.
    ///
    /// Drives the real `ProgramSegmenter::pump` (the production entry point
    /// `drive_program_segmenters` calls, where the segment start is
    /// recorded), not a direct `seg.push` — the latter would bypass exactly
    /// the code this fixes.
    ///
    /// Biting test: delete the `self.segment_writer.note_segment_start(...)`
    /// call from `queue_segment` and this test's `events_in_segment` walk
    /// finds nothing, failing the assertion.
    #[test]
    pub(super) fn program_segmenter_notes_each_segments_start_media_time() {
        // A bare trunk with the real video track (90000 ticks/s == the event
        // ring's own 90 kHz clock).
        // A sample ring large enough to retain every frame (the segmenter
        // subscribes from the backlog; a small ring would evict the head).
        let trunk = Trunk::new(
            TrunkConfig::default()
                .with_timed_capacity(nz(512))
                .with_sparse_capacity(nz(16))
                .with_segment_capacity(nz(8))
                .with_event_capacity(nz(16))
                .with_part_capacity(nz(16)),
        );
        let writer = trunk.writer().expect("first trunk writer");
        writer.set_tracks(vec![track_spec(TrackId::new(1))]);

        // A real SCTE-35 splice_insert (event_id 2002) at pts 0.
        let splice_bytes = test_fixtures::read("scte35/splice-insert-2002.bin");
        let mut timeline = timed_metadata::Timeline::new();
        let ev = timeline.push_scte35(&splice_bytes).unwrap();
        writer.publish_event(ev, EventAnchor::Media(MediaTime(0)));

        // Write enough frames into the trunk to cut a few real segments (a
        // sync frame every 45 frames == 0.5 s, target 1.0 s).
        for i in 0..200u32 {
            writer.publish(
                TrackId::new(1),
                RetentionClass::Timed,
                sample_at(i, i.is_multiple_of(45)),
            );
        }

        let mut segmenter = segmenter_for(&trunk, fmp4_config());
        // `pump` drains the backlog cursor and publishes whatever segments
        // that produced — the production path.
        let mut events = Vec::new();
        segmenter.pump(ProgramId(0), &trunk, &mut events, &NO_SIBLINGS);
        segmenter.flush();

        let last = trunk
            .last_closed_segment()
            .expect("the segmenter must have closed at least one segment");
        let attributed = (1..=last.value()).any(|seq| {
            trunk
                .events_in_segment(SegmentSeq::new(seq))
                .iter()
                .any(|e| {
                    matches!(
                        e.event.source,
                        timed_metadata::event::SourcePayload::Scte35 { .. }
                    )
                })
        });
        assert!(
            attributed,
            "a published SCTE-35 event must be attributable to a segment;  \
            `note_segment_start` was never called by the production segmenter before W9  \
            (closed segments: 1..={last})"
        );
    }

    /// **Review item 1, `try_new`'s `Container::MpegTs` arm.** Building a
    /// TS-container segmenter against a `Trunk` that already holds segments
    /// (e.g. a `SegmentWriter` dropped and re-issued — `media_plane` issue
    /// #1082) must resume numbering from `SegmentWriter::next_sequence_number()`,
    /// not renumber from 1 — a segmenter seeded from 1 has every segment it
    /// cuts rejected as non-monotonic and dropped.
    #[test]
    pub(super) fn try_new_ts_arm_seeds_from_an_already_populated_trunk() {
        let trunk = Trunk::new(
            TrunkConfig::default()
                .with_timed_capacity(nz(64))
                .with_sparse_capacity(nz(64))
                .with_segment_capacity(nz(16))
                .with_event_capacity(nz(8))
                .with_part_capacity(nz(8)),
        );
        trunk
            .writer()
            .expect("first take succeeds")
            .set_tracks(vec![track_spec(TrackId::new(1))]);
        pre_seed_five_segments(&trunk);

        let mut segmenter = segmenter_for(&trunk, ts_config());

        // Push samples through a sync boundary past the 1s target so the
        // segmenter cuts at least one real segment.
        for i in 0..90u32 {
            segmenter
                .seg
                .push(TrackId::new(1), sample_at(i, i.is_multiple_of(45)))
                .expect("push");
        }
        segmenter.publish_ready();

        assert_eq!(
            trunk.last_closed_segment(),
            Some(SegmentSeq::new(6)),
            "the first segment a freshly-built TS segmenter cuts against an \
             already-populated Trunk must resume from next_sequence_number(), \
             not renumber from 1"
        );
    }

    /// **Review item 1's rebuild arm.** A rebuilt TS segmenter's first cut
    /// segment must resume from `next_sequence_number()`, not renumber from 1.
    #[test]
    pub(super) fn apply_track_change_ts_rebuild_seeds_from_next_sequence_number() {
        let trunk = Trunk::new(
            TrunkConfig::default()
                .with_timed_capacity(nz(64))
                .with_sparse_capacity(nz(64))
                .with_segment_capacity(nz(16))
                .with_event_capacity(nz(8))
                .with_part_capacity(nz(8)),
        );
        trunk
            .writer()
            .expect("first take succeeds")
            .set_tracks(vec![track_spec(TrackId::new(1))]);
        pre_seed_five_segments(&trunk);

        // `bare_segmenter` already builds its placeholder Ts segmenter over
        // track 1 — so re-announcing that same track_id below fails
        // `add_track`'s duplicate check on the (only) entry, forcing the
        // rebuild path.
        let mut segmenter = bare_segmenter(&trunk);
        // The placeholder segmenter `bare_segmenter` builds is a TS one, so
        // the rebuild it triggers below takes `apply_track_change`'s own
        // `Container::MpegTs` arm.
        segmenter.config = ts_config();

        segmenter.published_tracks = trunk.tracks();
        let mut events = Vec::new();
        segmenter.apply_track_change(
            ProgramId(0),
            &trunk,
            &mut events,
            &NO_SIBLINGS,
            trunk.track_generation(),
        );

        for i in 0..90u32 {
            segmenter
                .seg
                .push(TrackId::new(1), sample_at(i, i.is_multiple_of(45)))
                .expect("push");
        }
        segmenter.publish_ready();

        assert_eq!(
            trunk.last_closed_segment(),
            Some(SegmentSeq::new(6)),
            "the rebuilt TS segmenter's first cut segment must resume from \
             next_sequence_number(), not renumber from 1"
        );
    }

    /// Push ~3 s of 25 fps video (a keyframe every 25 frames) and publish what
    /// closes.
    pub(super) fn push_gop_run(segmenter: &mut ProgramSegmenter, first_frame: u32) {
        for i in first_frame..first_frame + 75 {
            segmenter
                .seg
                .push(TrackId::new(1), sample_at(i, i.is_multiple_of(25)))
                .expect("push");
        }
        segmenter.publish_ready();
    }

    /// Audit r07-C7 (#1083): a Data/Subtitle track joining mid-stream must
    /// neither panic the rebuild nor leave an MPEG-TS placeholder installed
    /// on an fMP4 route (which would publish TS bytes as `seg-*.m4s`). The
    /// fMP4 segmenter is rebuilt over the muxable tracks only and numbering
    /// continues.
    #[test]
    pub(super) fn fmp4_rebuild_with_a_data_track_keeps_fmp4_and_numbering() {
        let trunk = Trunk::new(
            TrunkConfig::default()
                .with_timed_capacity(nz(64))
                .with_sparse_capacity(nz(64))
                .with_segment_capacity(nz(16))
                .with_event_capacity(nz(8))
                .with_part_capacity(nz(64)),
        );
        let writer = trunk.writer().expect("first take succeeds");
        writer.set_tracks(vec![track_spec(TrackId::new(1))]);
        let mut segmenter = segmenter_for(&trunk, fmp4_config());
        let mut cursor = trunk.subscribe_segments();
        push_gop_run(&mut segmenter, 0);
        let before = trunk.last_closed_segment().expect("a segment closed");

        // A routine PMT version bump adds a Data PID.
        writer.set_tracks(vec![
            track_spec(TrackId::new(1)),
            data_track_spec(TrackId::new(2)),
        ]);
        segmenter.published_tracks = trunk.tracks();
        let mut events = Vec::new();
        segmenter.apply_track_change(
            ProgramId(0),
            &trunk,
            &mut events,
            &NO_SIBLINGS,
            trunk.track_generation(),
        );
        let AnySegmenter::Fmp4(rebuilt) = &segmenter.seg else {
            panic!("an fMP4 route must keep producing fMP4 after a track change");
        };
        // The rebuilt segmenter carries only the muxable tracks, so its init
        // segment is buildable (a Data track makes it `UnmuxableDataTrack`).
        rebuilt
            .init_segment()
            .expect("rebuilt segmenter must be able to build its init segment");

        push_gop_run(&mut segmenter, 75);
        let after = trunk.last_closed_segment().expect("segments closed");
        assert!(
            after > before,
            "numbering must continue: {before} -> {after}"
        );
        let mut checked = 0;
        while let Some(item) = cursor.poll() {
            if let SegmentCursorItem::Segment(entry) = item {
                assert_ne!(
                    entry.bytes[0], TS_SYNC_BYTE,
                    "segment {} is MPEG-TS bytes on an fMP4 route",
                    entry.sequence_number
                );
                assert!(
                    matches!(&entry.bytes[4..8], STYP_BYTES | MOOF_BYTES),
                    "segment {} does not start with an fMP4 box: {:02x?}",
                    entry.sequence_number,
                    &entry.bytes[..8]
                );
                checked += 1;
            }
        }
        assert!(
            checked >= 2,
            "expected segments from both runs, saw {checked}"
        );
    }

    pub(super) const TS_SYNC_BYTE: u8 = 0x47;
    pub(super) const STYP_BYTES: &[u8] = b"styp";
    pub(super) const MOOF_BYTES: &[u8] = b"moof";

    /// The effective track set a refused change leaves behind: the segmenter
    /// keeps producing fMP4 and its published set does not grow.
    #[test]
    pub(super) fn fmp4_rebuild_with_no_muxable_track_keeps_the_current_segmenter() {
        let trunk = Trunk::new(
            TrunkConfig::default()
                .with_timed_capacity(nz(64))
                .with_sparse_capacity(nz(64))
                .with_segment_capacity(nz(16))
                .with_event_capacity(nz(8))
                .with_part_capacity(nz(64)),
        );
        let writer = trunk.writer().expect("first take succeeds");
        writer.set_tracks(vec![track_spec(TrackId::new(1))]);
        let mut segmenter = segmenter_for(&trunk, fmp4_config());

        writer.set_tracks(vec![data_track_spec(TrackId::new(2))]);
        let mut events = Vec::new();
        segmenter.pump(ProgramId(0), &trunk, &mut events, &NO_SIBLINGS);
        assert!(matches!(segmenter.seg, AnySegmenter::Fmp4(_)));
        assert!(
            events
                .iter()
                .any(|e| matches!(e, SegmenterEvent::TracksActive(false))),
            "a data-only change must announce no active tracks: {events:?}"
        );
        assert_eq!(
            segmenter
                .published_tracks
                .iter()
                .map(|t| t.track_id)
                .collect::<Vec<_>>(),
            vec![TrackId::new(1)],
            "a refused change must leave the published set the installed segmenter carries"
        );

        // And an empty set (every track removed).
        writer.set_tracks(Vec::new());
        segmenter.published_tracks = trunk.tracks();
        let mut events = Vec::new();
        segmenter.apply_track_change(
            ProgramId(0),
            &trunk,
            &mut events,
            &NO_SIBLINGS,
            trunk.track_generation(),
        );
        assert!(matches!(segmenter.seg, AnySegmenter::Fmp4(_)));
    }

    /// The in-band marker, not the trunk's generation, decides when the
    /// announced track specs move.
    #[test]
    pub(super) fn route_track_specs_follow_the_in_band_marker_not_the_trunk_generation() {
        let trunk = Trunk::new(
            TrunkConfig::default()
                .with_timed_capacity(nz(512))
                .with_sparse_capacity(nz(16))
                .with_segment_capacity(nz(8))
                .with_event_capacity(nz(16))
                .with_part_capacity(nz(16)),
        );
        let writer = trunk.writer().expect("first take succeeds");
        writer.set_tracks(vec![track_spec(TrackId::new(1))]);
        let mut segmenter = segmenter_for(&trunk, ts_config());
        for i in 0..10u32 {
            writer.publish(
                TrackId::new(1),
                RetentionClass::Timed,
                sample_at(i, i.is_multiple_of(45)),
            );
        }
        let mut changed = track_spec(TrackId::new(1));
        changed.timescale = 45_000;
        writer.set_tracks(vec![changed.clone()]);
        assert_eq!(trunk.track_generation(), 2);
        let mut events = Vec::new();
        segmenter.pump(ProgramId(0), &trunk, &mut events, &NO_SIBLINGS);
        assert!(
            events
                .iter()
                .any(|e| matches!(e, SegmenterEvent::TrackSpecs(_))),
            "the in-band marker must announce the new specs: {events:?}"
        );
        assert_eq!(segmenter.published_tracks[0].timescale, 45_000);
    }

    /// Item 5 (unit): a discontinuity recorded while a segment is in progress
    /// must NOT change that segment's start — it is promoted to the NEXT
    /// segment boundary. Driven directly on the anchor state, since an fMP4
    /// segmenter cannot be made to keep cutting after a jump large enough to
    /// be detected as a discontinuity.
    ///
    /// Biting test: assign `self.anchor = Some(fresh)` unconditionally (the
    /// pre-fix behaviour) and the in-progress segment's start changes.
    #[test]
    fn a_pending_anchor_does_not_change_the_in_progress_segment() {
        let trunk = Trunk::new(
            TrunkConfig::default()
                .with_timed_capacity(nz(64))
                .with_sparse_capacity(nz(16))
                .with_segment_capacity(nz(8))
                .with_event_capacity(nz(16))
                .with_part_capacity(nz(16)),
        );
        trunk
            .writer()
            .expect("writer")
            .set_tracks(vec![track_spec(TrackId::new(1))]);
        let mut segmenter = segmenter_for(&trunk, fmp4_config());

        // Establish the initial anchor for segment 1.
        segmenter.anchor = Some(MediaAnchor {
            source_90k: 0,
            timeline_ns: 0,
        });
        // A discontinuity observed mid-segment-1: it must NOT retro-stamp
        // segment 1.
        segmenter.pending_anchor = Some(MediaAnchor {
            source_90k: 900_090,
            timeline_ns: 0,
        });
        assert_eq!(
            segmenter.anchor.map(|a| a.source_90k),
            Some(0),
            "the in-progress segment's anchor must be unchanged by a pending one"
        );

        // Close segment 1 (duration 1 s == 90000 ticks) -> the pending anchor
        // is promoted, offset by that segment's duration.
        segmenter.queue_segment(ReadySegment {
            bytes: vec![0u8; 8],
            segment_seq: SegmentSeq::new(1),
            duration: 1.0,
            discontinuous: false,
        });
        let promoted = segmenter.anchor.expect("anchor promoted");
        assert_eq!(
            promoted.source_90k, 900_090,
            "the NEXT segment must be anchored to the post-jump clock"
        );
        assert_eq!(
            promoted.timeline_ns, 1_000_000_000,
            "the promoted anchor's timeline position is the end of the closed segment (1 s in ns)"
        );
        assert!(
            segmenter.pending_anchor.is_none(),
            "the pending anchor is consumed, not re-applied"
        );
    }

    /// Issue #1428: an AAC config change (sample rate or channel count) is a
    /// discontinuity trigger on a classic-TS route; an unchanged AAC
    /// re-announce is not.
    #[test]
    fn apply_track_change_ts_aac_config_change_arms_discontinuity() {
        let video = track_spec(TrackId::new(1));
        let audio = aac_track_spec(TrackId::new(2));

        let unchanged = ts_playlist_after_respec(
            vec![video.clone(), audio.clone()],
            vec![video.clone(), audio.clone()],
        );
        assert!(
            !unchanged.contains("#EXT-X-DISCONTINUITY"),
            "unchanged AAC re-announce must not arm a discontinuity: {unchanged}"
        );

        let mut rate = audio.clone();
        if let CodecConfig::Aac { sample_rate, .. } = &mut rate.config {
            *sample_rate = if *sample_rate == 48_000 {
                44_100
            } else {
                48_000
            };
        }
        let pl = ts_playlist_after_respec(
            vec![video.clone(), audio.clone()],
            vec![video.clone(), rate],
        );
        assert!(
            pl.contains("#EXT-X-DISCONTINUITY"),
            "AAC sample_rate change must arm a discontinuity: {pl}"
        );

        let mut chans = audio.clone();
        if let CodecConfig::Aac { channel_count, .. } = &mut chans.config {
            *channel_count = if *channel_count == 2 { 1 } else { 2 };
        }
        let pl = ts_playlist_after_respec(vec![video.clone(), audio], vec![video, chans]);
        assert!(
            pl.contains("#EXT-X-DISCONTINUITY"),
            "AAC channel_count change must arm a discontinuity: {pl}"
        );
    }

    /// Push a first run, apply `respec` as a track change, push a second run,
    /// and return the rebuilt TS segmenter's rendered playlist.
    fn ts_playlist_after_respec(initial: Vec<TrackSpec>, respec: Vec<TrackSpec>) -> String {
        let trunk = Trunk::new(
            TrunkConfig::default()
                .with_timed_capacity(nz(64))
                .with_sparse_capacity(nz(64))
                .with_segment_capacity(nz(16))
                .with_event_capacity(nz(8))
                .with_part_capacity(nz(8)),
        );
        let writer = trunk.writer().expect("first take succeeds");
        writer.set_tracks(initial);
        let mut segmenter = segmenter_for(&trunk, ts_config());
        for i in 0..45u32 {
            segmenter
                .seg
                .push(TrackId::new(1), sample_at(i, i.is_multiple_of(45)))
                .expect("push");
        }
        segmenter.publish_ready();
        writer.set_tracks(respec);
        segmenter.published_tracks = trunk.tracks();
        let mut events = Vec::new();
        segmenter.apply_track_change(
            ProgramId(0),
            &trunk,
            &mut events,
            &NO_SIBLINGS,
            trunk.track_generation(),
        );
        for i in 45..90u32 {
            segmenter
                .seg
                .push(TrackId::new(1), sample_at(i, i.is_multiple_of(45)))
                .expect("push");
        }
        segmenter.publish_ready();
        let AnySegmenter::Ts(ts) = &segmenter.seg else {
            panic!("ts")
        };
        ts.playlist().expect("playlist")
    }

    /// A changed spec must arm a discontinuity on the rebuilt TS segmenter;
    /// an unchanged re-announce must not.
    #[test]
    pub(super) fn apply_track_change_ts_changed_spec_arms_discontinuity() {
        let trunk = Trunk::new(
            TrunkConfig::default()
                .with_timed_capacity(nz(64))
                .with_sparse_capacity(nz(64))
                .with_segment_capacity(nz(16))
                .with_event_capacity(nz(8))
                .with_part_capacity(nz(8)),
        );
        let writer = trunk.writer().expect("first take succeeds");
        writer.set_tracks(vec![track_spec(TrackId::new(1))]);
        let mut segmenter = segmenter_for(&trunk, ts_config());

        // Push a first run so a segment closes.
        for i in 0..45u32 {
            segmenter
                .seg
                .push(TrackId::new(1), sample_at(i, i.is_multiple_of(45)))
                .expect("push");
        }
        segmenter.publish_ready();

        // Changed spec: bump the timescale (a discontinuity trigger).
        let mut changed = track_spec(TrackId::new(1));
        changed.timescale = 45_000;
        writer.set_tracks(vec![changed.clone()]);
        segmenter.published_tracks = trunk.tracks();
        let mut events = Vec::new();
        segmenter.apply_track_change(
            ProgramId(0),
            &trunk,
            &mut events,
            &NO_SIBLINGS,
            trunk.track_generation(),
        );

        for i in 45..90u32 {
            segmenter
                .seg
                .push(TrackId::new(1), sample_at(i, i.is_multiple_of(45)))
                .expect("push");
        }
        segmenter.publish_ready();

        let AnySegmenter::Ts(ts) = &segmenter.seg else {
            panic!("ts")
        };
        let pl = ts.playlist().expect("playlist");
        assert!(
            pl.contains("#EXT-X-DISCONTINUITY"),
            "changed spec must arm a discontinuity: {pl}"
        );

        // Unchanged re-announce: no discontinuity armed.
        let trunk2 = Trunk::new(
            TrunkConfig::default()
                .with_timed_capacity(nz(64))
                .with_sparse_capacity(nz(64))
                .with_segment_capacity(nz(16))
                .with_event_capacity(nz(8))
                .with_part_capacity(nz(8)),
        );
        let writer2 = trunk2.writer().expect("first take succeeds");
        writer2.set_tracks(vec![track_spec(TrackId::new(1))]);
        let mut segmenter2 = segmenter_for(&trunk2, ts_config());
        for i in 0..45u32 {
            segmenter2
                .seg
                .push(TrackId::new(1), sample_at(i, i.is_multiple_of(45)))
                .expect("push");
        }
        segmenter2.publish_ready();
        writer2.set_tracks(vec![track_spec(TrackId::new(1))]);
        segmenter2.published_tracks = trunk2.tracks();
        let mut events2 = Vec::new();
        segmenter2.apply_track_change(
            ProgramId(0),
            &trunk2,
            &mut events2,
            &NO_SIBLINGS,
            trunk2.track_generation(),
        );
        for i in 45..90u32 {
            segmenter2
                .seg
                .push(TrackId::new(1), sample_at(i, i.is_multiple_of(45)))
                .expect("push");
        }
        segmenter2.publish_ready();
        let AnySegmenter::Ts(ts2) = &segmenter2.seg else {
            panic!("ts")
        };
        let pl2 = ts2.playlist().expect("playlist");
        assert!(
            !pl2.contains("#EXT-X-DISCONTINUITY"),
            "unchanged re-announce must not arm a discontinuity: {pl2}"
        );
    }

    /// `ProgramSegmenter::publish_without_blocking` must never call the
    /// blocking `SegmentWriter::publish_segment` — `pump`/`flush` run
    /// synchronously on whatever thread the owning async ingest task is
    /// scheduled on (a shared tokio worker in production), so a stalled
    /// `ArchiveOverrun::StallIngest` DVR pin blocking that call would park
    /// the worker thread for up to `media_plane`'s bounded wait.
    #[test]
    pub(super) fn publish_without_blocking_never_blocks_the_calling_thread() {
        // Segment-log capacity 1, and a `StallIngest` pin this test never
        // drains: every publish past the first would need to block under
        // the pre-fix, direct `publish_segment` call. Only 5 entries — well
        // under `PENDING_PUBLISH_QUEUE_CAP` — so the queue-depth expiry
        // (tested separately below) does not also fire here.
        let trunk = Trunk::new(
            TrunkConfig::default()
                .with_timed_capacity(nz(8))
                .with_sparse_capacity(nz(8))
                .with_segment_capacity(nz(1))
                .with_event_capacity(nz(8))
                .with_part_capacity(nz(8)),
        );
        let _pinned_cursor = trunk.pin_segments(ArchiveOverrun::StallIngest);
        let mut segmenter = bare_segmenter(&trunk);

        let (done_tx, done_rx) = std::sync::mpsc::channel();
        let handle = std::thread::spawn(move || {
            for seq in 1..=5u32 {
                segmenter.publish_without_blocking(PendingPublish::Segment(fake_ready_entry(seq)));
            }
            done_tx.send(segmenter).expect("send back for inspection");
        });

        let segmenter = done_rx.recv_timeout(Duration::from_millis(500)).expect(
            "publish_without_blocking must never block — the pinned \
                 cursor above is never drained in this test",
        );
        handle.join().unwrap();

        // Nothing was silently dropped: whatever could not go through yet
        // is queued, in order, for the next call to retry.
        assert_eq!(
            pending_sequence_numbers(&segmenter),
            (2..=5).collect::<Vec<_>>(),
            "the queue must hold exactly the entries that could not be \
             published yet, in order — seq 1 alone fit before capacity was reached"
        );
    }

    #[test]
    pub(super) fn pending_queue_is_capped_even_before_the_deadline_elapses() {
        let trunk = Trunk::new(
            TrunkConfig::default()
                .with_timed_capacity(nz(8))
                .with_sparse_capacity(nz(8))
                .with_segment_capacity(nz(1))
                .with_event_capacity(nz(8))
                .with_part_capacity(nz(8)),
        );
        let mut stall = trunk.pin_segments(ArchiveOverrun::StallIngest);
        let mut segmenter = bare_segmenter(&trunk);

        // Publish well past the cap; the never-drained `stall` pin blocks
        // every eviction past the first.
        for seq in 1..=(PENDING_PUBLISH_QUEUE_CAP as u32 + 20) {
            segmenter.publish_without_blocking(PendingPublish::Segment(fake_ready_entry(seq)));
        }

        assert!(
            segmenter.pending.len() <= PENDING_PUBLISH_QUEUE_CAP,
            "queue length {} exceeded the cap {}",
            segmenter.pending.len(),
            PENDING_PUBLISH_QUEUE_CAP
        );
        // The stalled pin must actually have been terminated (live output
        // unfroze), not merely have its backlog silently truncated.
        assert!(
            matches!(stall.poll(), Some(SegmentCursorItem::Terminated)),
            "the stalled StallIngest pin must be terminated once the queue cap is hit"
        );
    }

    #[test]
    pub(super) fn queue_cap_counts_stuck_segments_not_queued_parts() {
        let trunk = Trunk::new(
            TrunkConfig::default()
                .with_timed_capacity(nz(8))
                .with_sparse_capacity(nz(8))
                .with_segment_capacity(nz(1))
                .with_event_capacity(nz(64))
                .with_part_capacity(nz(64)),
        );
        let mut stall = trunk.pin_segments(ArchiveOverrun::StallIngest);
        let mut segmenter = bare_segmenter(&trunk);

        // Seg 1 fills the (capacity-1) segment log with no eviction needed,
        // so it publishes immediately. Seg 2 needs to evict seg 1, which
        // the never-drained `stall` pin (still sitting on seg 1) blocks —
        // this is the one that actually gets stuck.
        segmenter.publish_without_blocking(PendingPublish::Segment(fake_ready_entry(1)));
        segmenter.publish_without_blocking(PendingPublish::Segment(fake_ready_entry(2)));

        // ...then far more parts than the (item-count) cap used to allow —
        // simulating an LL-HLS route's part cadence while that one segment
        // stays stuck. Every one of these queues up behind the stuck
        // segment (parts never jump ahead — see `publish_in_stream_order`'s
        // own doc) rather than draining.
        for i in 0..(PENDING_PUBLISH_QUEUE_CAP as u32 * 4) {
            segmenter.publish_without_blocking(PendingPublish::Part(PartEntry::new(
                bytes::Bytes::from_static(b"x"),
                SegmentSeq::new(2),
                PartIndex::new(i),
                Duration::from_millis(200),
                i == 0,
            )));
        }

        // `stall`'s cursor legitimately has real backlog to read (seg 1) —
        // the assertion is that it was never force-expired (`Terminated`),
        // not that `poll()` returns nothing at all.
        assert!(
            !matches!(stall.poll(), Some(SegmentCursorItem::Terminated)),
            "a single stuck segment behind a flood of queued parts must not \
             force-expire the pin — only PENDING_PUBLISH_QUEUE_CAP SEGMENTS \
             (or the 30s deadline) may do that"
        );
    }

    #[test]
    pub(super) fn stuck_segment_under_the_deadline_is_not_expired() {
        let trunk = Trunk::new(
            TrunkConfig::default()
                .with_timed_capacity(nz(8))
                .with_sparse_capacity(nz(8))
                .with_segment_capacity(nz(1))
                .with_event_capacity(nz(8))
                .with_part_capacity(nz(8)),
        );
        let mut stall = trunk.pin_segments(ArchiveOverrun::StallIngest);
        let mut segmenter = bare_segmenter(&trunk);

        // Seg 1 fills the (capacity-1) log with no eviction needed; seg 2
        // needs to evict it, which the never-drained `stall` pin blocks.
        segmenter.publish_without_blocking(PendingPublish::Segment(fake_ready_entry(1)));
        segmenter.publish_without_blocking(PendingPublish::Segment(fake_ready_entry(2)));
        assert_eq!(
            segmenter.pending.len(),
            1,
            "seg 2 must be stuck, not dropped"
        );

        // A 3s-old stall — comfortably under PENDING_PUBLISH_MAX_WAIT (30s).
        segmenter.pending_since = Some(Instant::now() - Duration::from_secs(3));
        segmenter.drain_pending();

        // `stall`'s cursor legitimately has real backlog to read (seg 1) —
        // the assertion is that it was never force-expired (`Terminated`),
        // not that `poll()` returns nothing at all.
        assert!(
            !matches!(stall.poll(), Some(SegmentCursorItem::Terminated)),
            "a 3s stall must not force-expire the pin — PENDING_PUBLISH_MAX_WAIT is 30s"
        );
        assert_eq!(
            segmenter.pending.len(),
            1,
            "the segment must still be queued, waiting for the pin to advance"
        );
    }

    #[test]
    pub(super) fn stuck_segment_past_the_deadline_is_expired() {
        let trunk = Trunk::new(
            TrunkConfig::default()
                .with_timed_capacity(nz(8))
                .with_sparse_capacity(nz(8))
                .with_segment_capacity(nz(1))
                .with_event_capacity(nz(8))
                .with_part_capacity(nz(8)),
        );
        let mut stall = trunk.pin_segments(ArchiveOverrun::StallIngest);
        let mut segmenter = bare_segmenter(&trunk);

        segmenter.publish_without_blocking(PendingPublish::Segment(fake_ready_entry(1)));
        segmenter.publish_without_blocking(PendingPublish::Segment(fake_ready_entry(2)));

        segmenter.pending_since =
            Some(Instant::now() - PENDING_PUBLISH_MAX_WAIT - Duration::from_secs(1));
        segmenter.drain_pending();

        assert!(
            matches!(stall.poll(), Some(SegmentCursorItem::Terminated)),
            "a stall past PENDING_PUBLISH_MAX_WAIT must force-expire the pin"
        );
        assert_eq!(
            segmenter.pending.len(),
            0,
            "the segment must have been retried and accepted once the pin was expired"
        );
    }

    #[test]
    pub(super) fn pending_since_resets_when_the_front_item_publishes_not_only_when_the_queue_empties()
     {
        let trunk = Trunk::new(
            TrunkConfig::default()
                .with_timed_capacity(nz(8))
                .with_sparse_capacity(nz(8))
                .with_segment_capacity(nz(1))
                .with_event_capacity(nz(8))
                .with_part_capacity(nz(8)),
        );
        let mut stall = trunk.pin_segments(ArchiveOverrun::StallIngest);
        let mut segmenter = bare_segmenter(&trunk);

        // Seg 1 fills the (capacity-1) log immediately. Seg 2 needs to
        // evict it, which the pin (not yet having consumed seg 1) blocks.
        segmenter.publish_without_blocking(PendingPublish::Segment(fake_ready_entry(1)));
        segmenter.publish_without_blocking(PendingPublish::Segment(fake_ready_entry(2)));
        // Seg 3 queues up behind the still-stuck seg 2 — never even
        // attempted yet, since `drain_pending` stops at the stuck front.
        segmenter.publish_without_blocking(PendingPublish::Segment(fake_ready_entry(3)));
        assert_eq!(
            pending_sequence_numbers(&segmenter),
            vec![2, 3],
            "seg 2 stuck, seg 3 queued behind it"
        );

        // Make seg 2's stall LOOK already past the deadline — simulating
        // that it has been stuck for a long time before finally resolving.
        segmenter.pending_since =
            Some(Instant::now() - PENDING_PUBLISH_MAX_WAIT - Duration::from_secs(1));

        // The pin genuinely advances now: it consumes seg 1, so evicting it
        // for seg 2 is no longer blocked.
        assert!(
            matches!(stall.poll(), Some(SegmentCursorItem::Segment(_))),
            "the pin must read real backlog (seg 1) here, not Terminated"
        );
        segmenter.drain_pending();

        // The decisive check: seg 3's own stall (the pin still has not
        // consumed seg 2, so evicting it for seg 3 is freshly blocked) has
        // been underway for 0 real seconds and must not be force-expired on
        // the strength of seg 2's old, already-resolved stall timestamp.
        // Pre-fix, it IS force-expired here — which (as an observable side
        // effect) also lets seg 3 itself slip through immediately once the
        // pin gives up, leaving `pending` empty instead of `[3]`.
        assert!(
            !matches!(stall.poll(), Some(SegmentCursorItem::Terminated)),
            "seg 3's stall just started (0s old) — it must not be force-expired on \
             the strength of seg 2's old, already-resolved stall timestamp"
        );
        assert_eq!(
            pending_sequence_numbers(&segmenter),
            vec![3],
            "seg 2 must have been accepted once the pin advanced past it, and seg 3 \
             must still be waiting (genuinely stuck, not force-expired through)"
        );
    }

    #[test]
    pub(super) fn stream_order_keeps_a_later_segments_part_from_landing_ahead_of_a_stuck_segment() {
        let trunk = Trunk::new(
            TrunkConfig::default()
                .with_timed_capacity(nz(8))
                .with_sparse_capacity(nz(8))
                .with_segment_capacity(nz(1))
                .with_event_capacity(nz(64))
                .with_part_capacity(nz(64)),
        );
        let _stall = trunk.pin_segments(ArchiveOverrun::StallIngest);
        let mut segmenter = bare_segmenter(&trunk);

        // Segment 0 fills the (capacity-1) log; the pin (created before it,
        // and never drained) has not consumed it, so evicting it for
        // segment 1 is blocked.
        segmenter.publish_without_blocking(PendingPublish::Segment(fake_ready_entry(0)));

        let parts = vec![
            PartInfo {
                bytes: vec![0u8; 4],
                duration: 0.2,
                independent: true,
                segment_seq: 1,
                part_index: 0,
            },
            PartInfo {
                bytes: vec![0u8; 4],
                duration: 0.2,
                independent: false,
                segment_seq: 1,
                part_index: 1,
            },
            // Segment 2's opening part — already produced (the segmenter
            // has started buffering the next segment) even though segment
            // 1 has not been queued yet in this same call.
            PartInfo {
                bytes: vec![0u8; 4],
                duration: 0.2,
                independent: true,
                segment_seq: 2,
                part_index: 0,
            },
        ];
        let segments = vec![ReadySegment {
            bytes: vec![1u8; 4],
            segment_seq: SegmentSeq::new(1),
            duration: 1.0,
            discontinuous: false,
        }];

        segmenter.publish_in_stream_order(parts, segments);

        // Segment 1's own two parts must have gone through — they were
        // never behind anything stuck.
        assert!(
            trunk
                .part_bytes(SegmentSeq::new(1), PartIndex::new(0))
                .is_some()
                && trunk
                    .part_bytes(SegmentSeq::new(1), PartIndex::new(1))
                    .is_some(),
            "segment 1's own parts must reach the Trunk — nothing was ever stuck ahead of them"
        );
        // Segment 1 itself must be stuck (queued, not yet in the Trunk) —
        // the last CLOSED segment is still segment 0, the one that filled
        // capacity.
        assert_eq!(
            trunk.last_closed_segment(),
            Some(SegmentSeq::new(0)),
            "segment 1 must still be stuck behind the un-consumed StallIngest pin"
        );
        // THE INVARIANT: segment 2's opening part must NOT have bypassed
        // the still-stuck segment 1 — it must be queued behind it, in
        // stream order, not delivered early.
        assert!(
            trunk
                .part_bytes(SegmentSeq::new(2), PartIndex::new(0))
                .is_none(),
            "segment 2's opening part must not land in the Trunk ahead of the \
             still-stuck segment 1's own close"
        );
    }

    /// One segment per second: 30 fps frames with a sync sample every 30.
    pub(super) fn gapless_pts(i: u32) -> i64 {
        i64::from(i) * i64::from(FRAME_DUR)
    }

    pub(super) fn on_90k(pts: i64) -> u64 {
        u64::try_from(pts).expect("test clocks stay positive")
    }

    pub(super) struct JumpRig {
        trunk: Arc<Trunk>,
        writer: crate::trunk::TrunkWriter,
        segmenter: ProgramSegmenter,
    }

    impl JumpRig {
        fn new() -> Self {
            let trunk = Trunk::new(
                TrunkConfig::default()
                    .with_timed_capacity(nz(256))
                    .with_sparse_capacity(nz(64))
                    .with_segment_capacity(nz(16))
                    .with_event_capacity(nz(8))
                    .with_part_capacity(nz(8)),
            );
            let writer = trunk.writer().expect("writer");
            writer.set_tracks(vec![track_spec(TrackId::new(1))]);
            let segmenter = segmenter_for(&trunk, fmp4_config());
            Self {
                trunk,
                writer,
                segmenter,
            }
        }

        /// Push `count` frames from frame index `first`, with a sync sample
        /// every [`GOP`] frames, through the production `pump` path.
        fn push_frames(&mut self, first: u32, count: u32, pts_at: impl Fn(u32) -> i64) {
            for i in first..first + count {
                let pts = pts_at(i);
                let mut sample = sample_at(i, i.is_multiple_of(GOP));
                sample.pts = Some(pts);
                sample.dts = Some(pts);
                self.writer
                    .publish(TrackId::new(1), RetentionClass::Timed, sample);
                let mut events = Vec::new();
                self.segmenter
                    .pump(ProgramId(0), &self.trunk, &mut events, &NO_SIBLINGS);
            }
        }

        /// The reported start of segments `1..=last` on the source clock.
        fn source_starts(&mut self, last: u32) -> Vec<Option<u64>> {
            self.segmenter.flush();
            (1..=last)
                .map(|seq| {
                    self.trunk
                        .segment_start_time(SegmentSeq::new(seq))
                        .map(|t| t.0)
                })
                .collect()
        }
    }

    /// (a) A video track whose PTS steps back 2 s mid-stream re-anchors: the
    /// first segment after the step starts at the stepped-back source clock,
    /// not at the pre-step clock continued forward.
    #[test]
    pub(super) fn a_backward_pts_step_reanchors_the_segments_source_clock() {
        let mut rig = JumpRig::new();
        rig.push_frames(0, 60, gapless_pts);
        let back = 2 * i64::from(transmux::VIDEO_CLOCK_RATE);
        rig.push_frames(60, 60, |i| gapless_pts(i) - back);

        assert_eq!(
            rig.source_starts(3),
            vec![Some(0), Some(90_000), Some(on_90k(gapless_pts(60) - back))],
            "the segment after a 2 s backward PTS step must restart from the stepped-back \
             source clock (frame 60 PTS minus 2 s), not continue the pre-step line at 180000"
        );
    }

    /// (b) A forward PTS step re-anchors to the jumped-to clock.
    #[test]
    pub(super) fn a_forward_pts_step_reanchors_the_segments_source_clock() {
        let mut rig = JumpRig::new();
        rig.push_frames(0, 60, gapless_pts);
        let forward = 8 * i64::from(transmux::VIDEO_CLOCK_RATE);
        rig.push_frames(60, 60, |i| gapless_pts(i) + forward);

        assert_eq!(
            rig.source_starts(3),
            vec![
                Some(0),
                Some(90_000),
                Some(on_90k(gapless_pts(60) + forward))
            ],
            "the segment after an 8 s forward PTS step must restart from the jumped-to source \
             clock (frame 60 PTS plus 8 s)"
        );
    }

    /// (c) B-frame reordering is not a jump: the starts follow the first
    /// picture of each segment and stay 90 000 apart.
    #[test]
    pub(super) fn b_frame_reordering_does_not_reanchor() {
        let mut rig = JumpRig::new();
        let reordered = |i: u32| -> i64 {
            let display_offset = match i % 3 {
                0 => 1,
                1 => -2,
                _ => -1,
            };
            gapless_pts(i) + i64::from(display_offset * 1_000)
        };
        rig.push_frames(0, 60, reordered);
        rig.push_frames(60, 60, reordered);

        assert_eq!(
            rig.source_starts(3),
            vec![Some(1_000), Some(91_000), Some(181_000)],
            "B-frame reordering must not re-anchor: the starts follow the first picture of each \
             segment (frame 0, 30, 60 at +1 ms per the reorder) and stay 90000 apart"
        );
    }
}

#[cfg(test)]
mod segmenter_gate_tests {
    use super::segmenter_tests::*;
    use super::*;
    use crate::trunk::{RetentionClass, TrunkConfig};
    use transmux::pipeline::CodecConfig;

    #[test]
    fn start_gate_drops_leading_non_sync_video() {
        let (trunk, writer) = video_trunk();
        let mut segmenter = segmenter_for(&trunk, fmp4_config());
        for i in 0..4u32 {
            writer.publish(TrackId::new(1), RetentionClass::Timed, sample_at(i, i == 3));
        }
        segmenter.pump(ProgramId(0), &trunk, &mut Vec::new(), &NO_SIBLINGS);
        assert_eq!(segmenter.start_gate_dropped, 3);
        assert_eq!(segmenter.start_gate_video_dropped, 3);
        assert!(segmenter.start_gate_open);
    }

    /// The gate runs BEFORE the media-anchor block (issue #1425): the first
    /// segment's reported start is the first KEPT (sync) sample's PTS on the
    /// source 90 kHz clock, not a dropped leading sample's. Biting: with the
    /// gate placed after the anchor block timeline 0 pins to sample 0 and the
    /// start comes out `3 * FRAME_DUR` early.
    #[test]
    fn start_gate_anchor_pins_to_the_first_kept_sync_sample() {
        let (trunk, writer) = video_trunk();
        let mut segmenter = segmenter_for(&trunk, fmp4_config());
        // Source clock starts 10 s in; samples 0..3 are mid-GOP, 3 is the
        // first sync, then a sync every 45 frames (1 s) so segment 1 closes
        // when sample 48 arrives.
        let offset = 10 * i64::from(transmux::VIDEO_CLOCK_RATE);
        let at = |i: u32, is_sync: bool| {
            let mut s = sample_at(i, is_sync);
            s.pts = Some(offset + i64::from(i) * i64::from(FRAME_DUR));
            s.dts = s.pts;
            s
        };
        for i in 0..100u32 {
            let is_sync = i == 3 || (i > 3 && (i - 3).is_multiple_of(45));
            writer.publish(TrackId::new(1), RetentionClass::Timed, at(i, is_sync));
            // Pump as we go: the sample ring is bounded (64), so an unpumped
            // burst of 100 would evict the leading samples before the gate
            // ever saw them.
            if i % 16 == 15 {
                segmenter.pump(ProgramId(0), &trunk, &mut Vec::new(), &NO_SIBLINGS);
            }
        }
        segmenter.pump(ProgramId(0), &trunk, &mut Vec::new(), &NO_SIBLINGS);
        assert_eq!(segmenter.start_gate_dropped, 3);
        assert!(
            trunk.last_closed_segment().is_some(),
            "segment 1 must have closed"
        );
        let first_kept_pts_90k = u64::try_from(offset + 3 * i64::from(FRAME_DUR)).unwrap();
        assert_eq!(
            trunk.segment_start_time(SegmentSeq::new(1)),
            Some(timed_metadata::MediaTime(first_kept_pts_90k)),
            "first segment starts at the first kept sync sample's PTS"
        );
    }

    #[test]
    fn start_gate_safety_valve_opens_without_any_sync_sample() {
        let (trunk, writer) = video_trunk();
        let mut segmenter = segmenter_for(&trunk, fmp4_config());
        let total = u32::try_from(START_GATE_MAX_VIDEO_DROPPED).unwrap() + 3;
        for i in 0..total {
            writer.publish(TrackId::new(1), RetentionClass::Timed, sample_at(i, false));
            if i % 16 == 15 {
                segmenter.pump(ProgramId(0), &trunk, &mut Vec::new(), &NO_SIBLINGS);
            }
        }
        segmenter.pump(ProgramId(0), &trunk, &mut Vec::new(), &NO_SIBLINGS);
        assert!(segmenter.start_gate_open, "valve must open the gate");
        assert_eq!(
            segmenter.start_gate_video_dropped, START_GATE_MAX_VIDEO_DROPPED,
            "exactly the valve's worth of video samples were dropped first"
        );
    }

    #[test]
    fn start_gate_no_drop_when_starts_on_sync() {
        let (trunk, writer) = video_trunk();
        let mut segmenter = segmenter_for(&trunk, fmp4_config());
        for i in 0..4u32 {
            writer.publish(TrackId::new(1), RetentionClass::Timed, sample_at(i, true));
        }
        segmenter.pump(ProgramId(0), &trunk, &mut Vec::new(), &NO_SIBLINGS);
        assert_eq!(segmenter.start_gate_dropped, 0);
        assert!(segmenter.start_gate_open);
    }

    #[test]
    fn start_gate_audio_only_drops_nothing() {
        let trunk = Trunk::new(
            TrunkConfig::default()
                .with_timed_capacity(nz(64))
                .with_sparse_capacity(nz(64))
                .with_segment_capacity(nz(16))
                .with_event_capacity(nz(8))
                .with_part_capacity(nz(8)),
        );
        let writer = trunk.writer().expect("writer");
        writer.set_tracks(vec![TrackSpec::new(
            TrackId::new(2),
            48000,
            CodecConfig::Vorbis {
                codec_private: Vec::new(),
                channels: 2,
                sample_rate: 48000,
            },
        )]);
        let mut segmenter = segmenter_for(&trunk, ts_config());
        for i in 0..4u32 {
            writer.publish(TrackId::new(2), RetentionClass::Timed, sample_at(i, false));
        }
        segmenter.pump(ProgramId(0), &trunk, &mut Vec::new(), &NO_SIBLINGS);
        assert_eq!(segmenter.start_gate_dropped, 0);
    }

    #[test]
    fn start_gate_video_plus_audio_drops_audio_before_first_video_sync() {
        let trunk = Trunk::new(
            TrunkConfig::default()
                .with_timed_capacity(nz(64))
                .with_sparse_capacity(nz(64))
                .with_segment_capacity(nz(16))
                .with_event_capacity(nz(8))
                .with_part_capacity(nz(8)),
        );
        let writer = trunk.writer().expect("writer");
        writer.set_tracks(vec![
            track_spec(TrackId::new(1)),
            data_track_spec(TrackId::new(2)),
        ]);
        let mut segmenter = segmenter_for(&trunk, fmp4_config());
        for i in 0..3u32 {
            writer.publish(TrackId::new(1), RetentionClass::Timed, sample_at(i, false));
            let s = sample_at(i, false);
            writer.publish(TrackId::new(2), RetentionClass::Timed, s);
        }
        writer.publish(TrackId::new(1), RetentionClass::Timed, sample_at(3, true));
        // audio after the sync is kept
        writer.publish(TrackId::new(2), RetentionClass::Timed, sample_at(4, false));
        segmenter.pump(ProgramId(0), &trunk, &mut Vec::new(), &NO_SIBLINGS);
        assert_eq!(segmenter.start_gate_dropped, 6);
        assert_eq!(segmenter.start_gate_video_dropped, 3);
        assert!(segmenter.start_gate_open);
    }

    const GEN1_MARK: u8 = 0xDD;
    const GEN2_MARK: u8 = 0xEE;

    fn marked_sample(i: u32, is_sync: bool, mark: u8) -> Sample {
        let nal = [0x65u8, mark, (i % 256) as u8];
        let mut data = (nal.len() as u32).to_be_bytes().to_vec();
        data.extend_from_slice(&nal);
        Sample::new(
            data,
            Some(i64::from(i) * i64::from(FRAME_DUR)),
            Some(i64::from(i) * i64::from(FRAME_DUR)),
            Some(FRAME_DUR),
            is_sync,
        )
    }

    fn marks_in(bytes: &[u8]) -> (bool, bool) {
        let has = |mark: u8| bytes.windows(2).any(|w| w == [0x65, mark]);
        (has(GEN1_MARK), has(GEN2_MARK))
    }

    /// **A9-4.** Samples published before `set_tracks` are muxed under the
    /// old track set even when the segmenter first runs after both the
    /// samples and the change are already in the ring.
    #[test]
    fn samples_published_before_a_track_change_are_not_muxed_under_the_new_set() {
        let trunk = Trunk::new(
            TrunkConfig::default()
                .with_timed_capacity(nz(256))
                .with_sparse_capacity(nz(64))
                .with_segment_capacity(nz(16))
                .with_event_capacity(nz(8))
                .with_part_capacity(nz(64)),
        );
        let writer = trunk.writer().expect("writer");
        writer.set_tracks(vec![track_spec(TrackId::new(1))]);
        let mut segmenter = segmenter_for(&trunk, fmp4_config());
        let mut segments = trunk.subscribe_segments();

        for i in 0..45u32 {
            let sample = marked_sample(i, i == 0, GEN1_MARK);
            writer.publish(TrackId::new(1), RetentionClass::Timed, sample);
        }
        writer.set_tracks(vec![
            track_spec(TrackId::new(1)),
            data_track_spec(TrackId::new(2)),
        ]);
        for i in 45..120u32 {
            let sample = marked_sample(i, i % 30 == 0, GEN2_MARK);
            writer.publish(TrackId::new(1), RetentionClass::Timed, sample);
        }
        segmenter.pump(ProgramId(0), &trunk, &mut Vec::new(), &NO_SIBLINGS);

        let mut checked = 0;
        let mut saw = (false, false);
        while let Some(item) = segments.poll() {
            if let crate::trunk::SegmentCursorItem::Segment(entry) = item {
                let marks = marks_in(&entry.bytes);
                assert!(
                    !(marks.0 && marks.1),
                    "segment {} mixes samples of both track sets",
                    entry.sequence_number
                );
                saw = (saw.0 || marks.0, saw.1 || marks.1);
                checked += 1;
            }
        }
        assert!(
            checked >= 2,
            "expected segments from both sets, saw {checked}"
        );
        assert_eq!(saw, (true, true), "both generations must reach a segment");
    }

    /// A video-only spec change must not re-arm the start gate for audio: the
    /// gate holds back video only.
    #[test]
    fn audio_keeps_flowing_across_a_video_only_track_change() {
        let trunk = Trunk::new(
            TrunkConfig::default()
                .with_timed_capacity(nz(256))
                .with_sparse_capacity(nz(64))
                .with_segment_capacity(nz(16))
                .with_event_capacity(nz(8))
                .with_part_capacity(nz(64)),
        );
        let writer = trunk.writer().expect("writer");
        let video = track_spec(TrackId::new(1));
        let audio = aac_track_spec(TrackId::new(2));
        writer.set_tracks(vec![video.clone(), audio.clone()]);
        let mut segmenter = segmenter_for(&trunk, fmp4_config());
        writer.publish(
            TrackId::new(1),
            RetentionClass::Timed,
            marked_sample(0, true, GEN1_MARK),
        );
        segmenter.pump(ProgramId(0), &trunk, &mut Vec::new(), &NO_SIBLINGS);
        assert!(segmenter.start_gate_open);

        let mut changed = video.clone();
        if let CodecConfig::Avc { width, .. } = &mut changed.config {
            *width = 1280;
        }
        writer.set_tracks(vec![changed, audio]);
        for i in 0..3u32 {
            writer.publish(TrackId::new(2), RetentionClass::Timed, sample_at(i, true));
        }
        writer.publish(
            TrackId::new(1),
            RetentionClass::Timed,
            marked_sample(1, true, GEN2_MARK),
        );
        segmenter.pump(ProgramId(0), &trunk, &mut Vec::new(), &NO_SIBLINGS);
        assert_eq!(
            segmenter.start_gate_dropped, 0,
            "audio must not be dropped by the video start gate after a video-only change"
        );
        assert!(segmenter.start_gate_open);
    }
}
