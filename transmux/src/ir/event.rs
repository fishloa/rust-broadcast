//! [`DemuxEvent`] — the streaming-demux event vocabulary, and
//! [`EventProvenance`], its container-native "where did this come from"
//! sidecar.
//!
//! Moved out of `transmux/src/ts_demux.rs` (media plane step 2e): that module
//! was the *only* place this type was defined, but it is not TS-only —
//! [`crate::flv_stream::StreamingFlvDemux`] emits it too (`use
//! crate::ts_demux::DemuxEvent` was the tell). Two variants were TS-specific
//! in a supposedly neutral enum: `Discontinuity { pid: u16 }` hardcoded an
//! MPEG-2 TS PID, and `Pcr(PcrSample)` wrapped the whole TS-shaped
//! [`crate::ir::PcrSample`] (`pid` + `packet_index` + `discontinuity` bool)
//! directly. Both are folded into a provenance-carrying shape here: the
//! concept (a discontinuity was observed; a clock reference was observed)
//! stays in the primary variant, and the TS-only *identity* detail (which PID,
//! which packet) moves into [`EventProvenance`] — `None`/absent for a
//! container that has no such identity (FLV has no PID concept at all).

use super::sample::Sample;
use super::track::TrackSpec;

/// Container-native identity for a [`DemuxEvent`] that is not itself part of
/// the neutral vocabulary — e.g. the MPEG-2 TS PID a
/// [`DemuxEvent::Discontinuity`] / [`DemuxEvent::ClockReference`] was observed
/// on. Every field is `Option`: `None` means "this container has no
/// equivalent", never a fabricated value — a demuxer with no PID concept
/// (FLV, RTP) simply never populates `pid`.
#[non_exhaustive]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct EventProvenance {
    /// The MPEG-2 TS PID this event was observed on (ISO/IEC 13818-1
    /// §2.4.3.2), when the source container is TS.
    pub pid: Option<u16>,
    /// 0-based index of the source packet the event was observed at, in
    /// whatever unit the container demuxes in (e.g. the 188-byte TS packet
    /// index), when the container has one.
    pub packet_index: Option<u64>,
}

/// Why a [`DemuxEvent::Discontinuity`] was raised. `#[non_exhaustive]`: a
/// future discontinuity source (e.g. issue #778's continuity-counter gap)
/// adds a variant, not a breaking change.
#[non_exhaustive]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DiscontinuityKind {
    /// An explicit MPEG-2 TS adaptation-field `discontinuity_indicator`
    /// (ISO/IEC 13818-1 §2.4.3.5) — the source signalled the gap itself.
    /// Driven strictly by that bit, nothing else (issue #778 keeps a
    /// continuity-counter gap on the same PID a separate, undecided kind, not
    /// folded into this one).
    Signalled,
    /// A live audio track's frame-exact dts/pts anchor drifted from the wire
    /// PES clock past the re-anchor threshold and was re-anchored — a genuine
    /// gap (splice, encoder restart), not the 90 kHz/sample-rate rounding
    /// drift a real muxer accrues by construction, which the anchor absorbs
    /// silently. See `ts_demux`'s `audio_discontinuity_threshold_90k` for the
    /// threshold and its derivation.
    TimelineReanchored,
    /// A per-PID buffer cap was exceeded and the in-flight payload was
    /// dropped to keep memory bounded (e.g. [`crate::ts_demux::StreamingTsDemux`]'s
    /// `MAX_PES_BUFFER_BYTES` — a stalled PES that never sees a fresh
    /// `payload_unit_start`).
    BudgetExceeded {
        /// Bytes discarded when the cap tripped.
        bytes: u64,
    },
}

impl DiscontinuityKind {
    /// A short, stable label for this kind — used by `Display`.
    pub fn name(&self) -> &'static str {
        match self {
            DiscontinuityKind::Signalled => "signalled",
            DiscontinuityKind::TimelineReanchored => "timeline reanchored",
            DiscontinuityKind::BudgetExceeded { .. } => "budget exceeded",
        }
    }
}

broadcast_common::impl_spec_display!(DiscontinuityKind);

/// Why a [`DemuxEvent::TrackAbandoned`] was raised. `#[non_exhaustive]`: a
/// future abandonment cause adds a variant, not a breaking change.
#[non_exhaustive]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AbandonReason {
    /// The track's codec config never became recoverable before end of
    /// input (e.g. a PMT-listed H.264 PID whose SPS/PPS never arrived) —
    /// [`DemuxEvent::TrackAdded`] never fired for it.
    ConfigUnrecoverable,
    /// A per-PID probe/parked backlog byte cap was exceeded before the
    /// track's config resolved (e.g. [`crate::ts_demux::StreamingTsDemux`]'s
    /// `MAX_PROBE_BACKLOG_BYTES`) — permanently abandoned, never promoted to
    /// live.
    BudgetExceeded,
}

impl AbandonReason {
    /// A short, stable label for this reason — used by `Display`.
    pub fn name(&self) -> &'static str {
        match self {
            AbandonReason::ConfigUnrecoverable => "config unrecoverable",
            AbandonReason::BudgetExceeded => "budget exceeded",
        }
    }
}

broadcast_common::impl_spec_display!(AbandonReason);

/// A demux event, drained incrementally from a streaming demuxer (issue #555:
/// [`crate::ts_demux::StreamingTsDemux`]; issue #738:
/// [`crate::flv_stream::StreamingFlvDemux`]).
///
/// This is deliberately **not** a universal cross-crate event enum: it is the
/// demux family's own vocabulary (every variant is something a container
/// *demuxer* can observe), not shared with the segmenter/packetiser families,
/// which name their own `Out` types (media plane step 2e, spec §6 "Drive
/// shape, not vocabulary").
///
/// # Event order within one drain
///
/// Observation order **per emission class, not wire order across classes**:
/// [`DemuxEvent::ClockReference`], [`DemuxEvent::Discontinuity`], and
/// section-carried [`DemuxEvent::Sample`]s are emitted at observation time
/// (`LiveKind::Section` pushes immediately, no lookahead), while PES-carried
/// [`DemuxEvent::Sample`]s lag by one access unit (`advance_one_behind`). A
/// consumer projecting a sparse/section event onto the media timeline must
/// anchor it on the most recent [`DemuxEvent::ClockReference`], never on the
/// position of a neighbouring PES-carried [`DemuxEvent::Sample`].
///
/// # Removal semantics (issue #774)
///
/// No [`DemuxEvent::Sample`] for a removed `track_id` is ever queued after
/// that track's [`DemuxEvent::TrackRemoved`] — the demuxer flushes or drops
/// that PID's pending payload first. Removal tracks the **declared** set
/// only (a PMT version change that no longer lists a PID) — it is never
/// synthesised from a silence timeout or any other absence heuristic.
#[non_exhaustive]
#[derive(Debug, Clone)]
pub enum DemuxEvent {
    /// New track discovered. The codec config is fully recovered by the time
    /// this fires — an opaque [`crate::pipeline::CodecConfig::Data`] track
    /// fires on its very first access unit, since its config needs no
    /// in-band header at all.
    TrackAdded(TrackSpec),
    /// An existing track's PMT-derived metadata changed — its
    /// `es_info_descriptors` (e.g. a corrected `ISO_639_language_descriptor`)
    /// or a reclassified `stream_type` — independent of codec config
    /// recovery, which stays single-shot and permanent (issue #774). Carries
    /// the track's full, current [`TrackSpec`] (same `track_id`, unchanged
    /// `config`, updated `es_info_descriptors`).
    TrackUpdated(TrackSpec),
    /// A previously-declared track is no longer listed in the source's track
    /// declaration (e.g. a PMT version change that drops a PID). Only ever
    /// raised for a track that had already fired [`DemuxEvent::TrackAdded`]
    /// (i.e. a real `track_id` exists) — see the type-level "Removal
    /// semantics" note above.
    #[non_exhaustive]
    TrackRemoved {
        /// The removed track's ID (matches a prior [`DemuxEvent::TrackAdded`]).
        track_id: u32,
        /// Container-native identity for this observation (e.g. the TS PID),
        /// when the container has one.
        provenance: EventProvenance,
    },
    /// A track's config recovery (or backlog budget) was permanently
    /// abandoned — it will never fire [`DemuxEvent::TrackAdded`]. `track_id`
    /// is `None`: abandonment always happens *before* a track_id would be
    /// assigned (config recovery — and therefore promotion — never
    /// completed), never fabricated for a track a consumer has already seen.
    #[non_exhaustive]
    TrackAbandoned {
        /// Always `None` today (see the field doc) — carried as `Option` so
        /// a future abandonment path that *does* have an assigned track_id
        /// is not a breaking change.
        track_id: Option<u32>,
        /// Why this track was abandoned.
        reason: AbandonReason,
        /// Container-native identity for this observation (e.g. the TS PID),
        /// when the container has one.
        provenance: EventProvenance,
    },
    /// A completed access unit / audio frame, with absolute per-sample
    /// `dts`/`pts` (issue #556 semantics; media plane step 2c: absolute
    /// rather than carried in a separate `SourceTiming`).
    #[non_exhaustive]
    Sample {
        /// The owning track's ID (matches a prior [`DemuxEvent::TrackAdded`]).
        track_id: u32,
        /// The coded sample.
        sample: Sample,
    },
    /// A clock reference observed in the source container (e.g. an MPEG-2 TS
    /// PCR, ISO/IEC 13818-1 §2.4.3.4/§2.4.3.5). A container with no such
    /// concept (FLV, RTP) never emits this.
    #[non_exhaustive]
    ClockReference {
        /// The clock value, in the container's native clock rate (`clock_hz`).
        ticks: u64,
        /// The rate, in Hz, `ticks` is expressed in (27 MHz for an MPEG-2 TS
        /// PCR).
        clock_hz: u32,
        /// `true` when this observation follows a signalled discontinuity in
        /// the source clock (e.g. the same TS packet's adaptation-field
        /// `discontinuity_indicator`, §2.4.3.5).
        discontinuous: bool,
        /// Container-native identity for this observation (e.g. the TS PID
        /// carrying it), when the container has one.
        provenance: EventProvenance,
    },
    /// A discontinuity indicator observed on the source stream (e.g. an
    /// MPEG-2 TS adaptation-field `discontinuity_indicator`, ISO/IEC
    /// 13818-1 §2.4.3.5), independent of whether that same observation also
    /// carried a [`DemuxEvent::ClockReference`].
    #[non_exhaustive]
    Discontinuity {
        /// The track this discontinuity was observed on, when the carrying
        /// PID/stream had already resolved to one at the time it fired.
        /// `None` before that resolution (never fabricated) — a discontinuity
        /// can legitimately be observed on a PID whose track isn't known yet.
        track: Option<u32>,
        /// What kind of discontinuity this is.
        kind: DiscontinuityKind,
        /// Container-native identity for this observation (e.g. the TS PID),
        /// when the container has one.
        provenance: EventProvenance,
    },
    /// Every currently-known declared track has resolved: none is still
    /// pending config recovery. By the time this fires,
    /// [`DemuxEvent::TrackAdded`] has already been (or is about to be, in the
    /// same event batch) emitted for every track known so far — the signal a
    /// consumer building a multi-track segmenter needs to know it is safe to
    /// construct (or has learned) the full track set, rather than building
    /// video-only at the first video keyframe and silently missing a
    /// later-resolving audio track.
    ///
    /// This means "the declared track set is stable", which requires the
    /// container to *have* an up-front track declaration in the first place
    /// (MPEG-2 TS: the PMT). A container without one — FLV/RTMP, whose
    /// `TypeFlags` header bits are informational only and not trusted even by
    /// the one-shot [`crate::flv::FlvDemux`] — legitimately never emits this;
    /// that asymmetry is for the media plane's ingress layer to handle
    /// explicitly (e.g. gating on the first [`DemuxEvent::Sample`] instead),
    /// not something this event can paper over by pretending every container
    /// has a track-count declaration.
    ///
    /// `generation` is a monotonic counter bumped once per applied track-set
    /// change (add/update/remove — issue #774): it is *not* a PID count, so a
    /// removal immediately followed by an addition that returns the known-PID
    /// count to a previously-seen value still re-arms this event (the count
    /// itself is never a reliable de-dup key — see `ts_demux.rs`'s own
    /// regression test for the failure mode this replaced).
    #[non_exhaustive]
    TracksResolved {
        /// The track-set generation this event confirms is fully resolved.
        generation: u32,
    },
}

impl DemuxEvent {
    /// Construct a [`DemuxEvent::ClockReference`] — the only way to build
    /// this variant from outside this crate, since it is `#[non_exhaustive]`
    /// (issue #774 media plane reshape) so a future field (e.g. a
    /// wall-clock/UTC anchor) is not a breaking change.
    pub fn clock_reference(
        ticks: u64,
        clock_hz: u32,
        discontinuous: bool,
        provenance: EventProvenance,
    ) -> Self {
        DemuxEvent::ClockReference {
            ticks,
            clock_hz,
            discontinuous,
            provenance,
        }
    }

    /// Construct a [`DemuxEvent::Discontinuity`] — see
    /// [`Self::clock_reference`] for why this variant needs a constructor.
    pub fn discontinuity(
        track: Option<u32>,
        kind: DiscontinuityKind,
        provenance: EventProvenance,
    ) -> Self {
        DemuxEvent::Discontinuity {
            track,
            kind,
            provenance,
        }
    }

    /// Construct a [`DemuxEvent::TrackRemoved`] — see
    /// [`Self::clock_reference`] for why this variant needs a constructor.
    pub fn track_removed(track_id: u32, provenance: EventProvenance) -> Self {
        DemuxEvent::TrackRemoved {
            track_id,
            provenance,
        }
    }

    /// Construct a [`DemuxEvent::TrackAbandoned`] — see
    /// [`Self::clock_reference`] for why this variant needs a constructor.
    pub fn track_abandoned(
        track_id: Option<u32>,
        reason: AbandonReason,
        provenance: EventProvenance,
    ) -> Self {
        DemuxEvent::TrackAbandoned {
            track_id,
            reason,
            provenance,
        }
    }

    /// Construct a [`DemuxEvent::Sample`] — see [`Self::clock_reference`] for
    /// why this variant needs a constructor.
    pub fn sample(track_id: u32, sample: Sample) -> Self {
        DemuxEvent::Sample { track_id, sample }
    }

    /// Construct a [`DemuxEvent::TracksResolved`] — see
    /// [`Self::clock_reference`] for why this variant needs a constructor.
    pub fn tracks_resolved(generation: u32) -> Self {
        DemuxEvent::TracksResolved { generation }
    }
}
