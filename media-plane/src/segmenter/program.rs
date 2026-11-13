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

/// Configuration for one [`ProgramSegmenter`], replaceable on route reload.
#[derive(Clone, Debug)]
pub struct SegmenterConfig {
    pub container: transmux::Container,
    pub ts_cut_policy: transmux::SegmentCutPolicy,
    pub ts_idr_fallback: TsHlsIdrFallback,
    pub target_duration_secs: f64,
    pub part_target_ms: u32,
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
            part_target_ms: 250,
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
    pub fn with_part_target_ms(mut self, v: u32) -> Self {
        self.part_target_ms = v;
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
    PartTargetMs(u32),
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
        part_target_ms: u32,
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
        _part_target_ms: u32,
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
struct ReadySegment {
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
enum AnySegmenter {
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
    fn part_target_ms(&self) -> Option<u32> {
        match self {
            AnySegmenter::Fmp4(seg) => Some(seg.part_target_ms()),
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
enum TrackChange {
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
struct GapState {
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
    cursor: SampleCursor,
    /// The last [`Self::pump`] stopped at its per-call item budget with samples still queued.
    pub backlogged: bool,
    /// Per track whose declared timescale differs from the `Trunk`'s (an
    /// `audio_profiles` HE-AAC override moves it to the output rate): the
    /// `(trunk, declared)` timescales its samples are re-expressed between
    /// on their way into the segmenter.
    timescale_moves: HashMap<TrackId, (u32, u32)>,
    segment_writer: SegmentWriter,
    seg: AnySegmenter,
    /// The next segment published opens a replacement run, so it carries
    /// `EXT-X-DISCONTINUITY` (RFC 8216bis 6.2.1: a change of timing or
    /// encoding is signalled with it).
    mark_next_discontinuous: bool,
    gap: Option<GapState>,
    /// Cumulative nanoseconds of segment duration published so far by this
    /// segmenter — mirrors `crate::route::RouteHandle`'s own
    /// `next_timeline_ns`, kept per-program here since each program's
    /// timeline is independent.
    next_timeline_ns: u64,
    /// Last-seen [`Trunk::track_generation`] — compared each pump to detect
    /// a mid-stream track-set change (issue #781). When the trunk's
    /// generation advances past this, [`Self::apply_track_change`] admits
    /// new tracks into (or rebuilds) the segmenter.
    track_generation: u64,
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
    pending: VecDeque<PendingPublish>,
    /// When the entry currently at the front of `pending` first became
    /// stuck — `None` whenever `pending` is empty. Compared against
    /// [`PENDING_PUBLISH_MAX_WAIT`] in [`Self::drain_pending`].
    pending_since: Option<Instant>,
    /// The mapping from the trunk's internal segment timeline to the
    /// source's own media (90 kHz) clock — see [`MediaAnchor`]. Reported to
    /// the trunk via [`SegmentWriter::note_segment_start`] so
    /// `Trunk::events_in_segment` can attribute inband SCTE-35 events to the
    /// right segment (issue #1083, W9). `None` until the first timed sample.
    anchor: Option<MediaAnchor>,
    /// An anchor to apply at the NEXT segment boundary, not now: a
    /// discontinuity detected mid-segment must not retro-stamp the segment
    /// already being filled with the post-jump clock (item 5). Set while
    /// samples of the in-progress segment are still arriving; consumed by
    /// [`Self::queue_segment`] once that segment closes, so the following
    /// segment is anchored to the first post-jump sample.
    pending_anchor: Option<MediaAnchor>,
    /// Per-track timeline-jump detector (broadcast-client semantics), moved
    /// to transmux.
    timeline: transmux::JumpDetector,
    /// Wall-clock origin for delivery times passed to the detector.
    ingest_origin: Instant,
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
    idr_wait: transmux::IdrWait,
    /// The wait ran out and the route cuts on non-IDR sync samples
    /// (`ts_hls_idr_fallback` permitting).
    idr_fallback_active: bool,
    /// Every sample since the first non-IDR sync sample, kept while the gate
    /// waits for an IDR: when the wait runs out the stream is replayed from
    /// that sync sample, so a stream whose only random-access points came
    /// before the bound still yields its first segment.
    idr_hold: VecDeque<(TrackId, Sample)>,
    idr_hold_bytes: usize,
    /// Separate audio renditions (issue #1428 slice 2b): one follower per
    /// mapped audio track, empty unless the route opts into split audio.
    pub followers: Vec<AudioFollower>,
    /// WebVTT subtitle renditions built from teletext subtitle pages (issue
    /// #1428): one follower per declared page.
    pub subtitle_followers: Vec<SubtitleFollower>,
    /// Duration of each recent leader segment, for sizing gap placeholders.
    leader_durations: LeaderDurations,
    /// Per-track caption detectors (`None` for a track whose codec has no
    /// caption carriage), built lazily and dropped on a track-set change.
    caption_detectors: HashMap<TrackId, Option<CaptionDetector>>,
    /// Every caption stream this segmenter has seen, across track changes.
    captions_seen: BTreeSet<DetectedCaption>,
    captions_declared: Vec<AtscCaptionService>,
    /// `captions_seen` has grown, or the scan window has ended, since the
    /// state was last handed to the route.
    captions_unreported: bool,
    /// First DTS seen per track the caption scan covers.
    caption_scan_started: HashMap<TrackId, i64>,
    /// A full [`CAPTION_SCAN_WINDOW_SECS`] of video has been examined; an
    /// empty `captions_seen` then means the route declares no captions.
    captions_scan_complete: bool,
    /// The leader's `PART-TARGET` last handed to the route's origin.
    part_target_published: Option<u32>,
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
struct MediaAnchor {
    source_90k: u64,
    timeline_ns: u64,
}

/// Start-gate safety valve (issue #1425): the gate drops leading video samples
/// until the first sync sample; if a video track flags no sync sample at all
/// within this many video samples (about 30 s at 60 fps) the gate opens
/// anyway (with a warning) instead of dropping the program forever.
const START_GATE_MAX_VIDEO_DROPPED: u64 = 1800;

/// Cursor items one [`ProgramSegmenter::pump`] call processes before it returns, leaving the rest
/// queued for the next call, so the caller can yield to the runtime between batches.
const PUMP_BATCH_ITEMS: usize = 512;

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

impl ProgramSegmenter {
    /// Start this run's media timeline where `route_handle` says `program`'s
    /// serving begins it (a no-op for the first run on a route): call once,
    /// right after [`Self::try_new`] and the program's publication, before
    /// the first [`Self::pump`].
    pub fn continue_after_reconnect(&mut self, media_start_ns: u64) {
        if media_start_ns != 0 {
            self.continue_media_timeline(media_start_ns);
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
    pub fn try_new(trunk: &Arc<Trunk>, config: SegmenterConfig) -> (Option<Self>, Vec<SegmenterEvent>) {
        Self::try_new_with_siblings(trunk, config, &())
    }

    pub fn try_new_with_siblings(
        trunk: &Arc<Trunk>,
        config: SegmenterConfig,
        siblings: &dyn SiblingTrunks,
    ) -> (Option<Self>, Vec<SegmenterEvent>) {
        let target_duration_secs = config.target_duration_secs;
        let part_target_ms = config.part_target_ms;
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
                    part_target_ms,
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
            siblings.build_audio_followers(trunk, &mapped, target_duration_secs, part_target_ms);
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
    fn publish_without_blocking(&mut self, item: PendingPublish) {
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
    fn drain_pending(&mut self) {
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

    /// [`Self::pump`] for at most [`PUMP_BATCH_ITEMS`] queued items, leaving the rest queued and
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
        let Some(ms) = self.seg.part_target_ms() else {
            return;
        };
        if self.part_target_published != Some(ms) {
            events.push(SegmenterEvent::PartTargetMs(ms));
            self.part_target_published = Some(ms);
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
    fn apply_track_change(
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
                        self.config.part_target_ms,
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
                                self.config.part_target_ms,
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
    fn publish_ready(&mut self) -> (usize, usize) {
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
    fn queue_segment(&mut self, segment: ReadySegment) {
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
    fn publish_in_stream_order(
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
