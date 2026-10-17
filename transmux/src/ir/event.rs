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
use crate::TrackId;
use mpeg_ts::pid::Pid;

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
    pub pid: Option<Pid>,
    /// 0-based index of the source packet the event was observed at, in
    /// whatever unit the container demuxes in (e.g. the 188-byte TS packet
    /// index), when the container has one.
    pub packet_index: Option<u64>,
    /// The MPEG-2 TS `program_number` (ISO/IEC 13818-1 §2.4.4.8, PAT/PMT)
    /// the event is attributed to, when the container has programs and the
    /// attribution is known. Today only [`DemuxEvent::ClockReference`] sets
    /// it: the program whose PMT names the carrying PID as its `PCR_PID`.
    /// `None` means "not attributable (yet)" — never a fabricated value; see
    /// [`DemuxEvent::ClockReference`] for the pre-PMT behaviour.
    pub program_number: Option<u16>,
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

/// What degraded the input signal — a condition the source container reported
/// that the demuxer cannot repair (issue #778). `#[non_exhaustive]`: a future
/// degradation source (e.g. RTP sequence-number gap) adds a variant, not a
/// breaking change.
#[non_exhaustive]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InputDegradation {
    /// The `transport_error_indicator` bit was set in the TS header
    /// (ISO/IEC 13818-1 §2.4.3.2) — the demodulator could not correct this
    /// packet.
    TransportError,
    /// A continuity-counter fault on one PID (ISO/IEC 13818-1 §2.4.3.3; ETSI
    /// TR 101 290 1.4), counted per packet on payload-bearing non-null
    /// packets only: a counter jump, a same-counter packet that is not a
    /// byte-identical repeat (PCR field excepted), or an identical repeat
    /// beyond the ONE duplicate the spec allows (`got` is then the last
    /// counter, one entry per excess packet). Excluded: the one legal
    /// duplicate and signalled discontinuities (`discontinuity_indicator` in
    /// the adaptation field). `expected` is the *last-observed* CC + 1 mod 16;
    /// `got` is the wire value that arrived. The entry names no PID itself:
    /// see [`Media::degraded_at`](crate::ir::Media::degraded_at) /
    /// [`DemuxEvent::InputDegraded`]'s `provenance`.
    ContinuityGap {
        /// The expected counter (`last_cc + 1 & 0x0F`).
        expected: u8,
        /// The counter value that arrived on the wire.
        got: u8,
    },
    /// An H.264/H.265 NAL that carries a parameter-set header (SPS/PPS, or
    /// VPS) was **rejected** as unusable for a decoder configuration and
    /// ignored (issue #1171): it was empty or longer than the `u16` length
    /// field of an `avcC`/`hvcC` record (ISO/IEC 14496-15 §5.3.3.1.2 /
    /// §8.3.3.1.2), had a wrong NAL header, or was an SPS that does not parse.
    /// The track's current config is unchanged and no `TrackAdded` /
    /// `TrackUpdated` is raised for it. One event per rejected NAL.
    InvalidParameterSet,
    /// A whole container unit (a Program Stream pack, an RTP packet, an fMP4
    /// `moof` fragment) failed to parse and was **skipped**; the demuxer
    /// resynchronised on the next unit and carried on instead of failing the
    /// whole input (audit T16, #1144). `position` is the unit's byte offset
    /// for byte-stream inputs (PS pack, fMP4 box) and its index in the packet
    /// list for packet-list inputs (batch RTP).
    MalformedUnit {
        /// Byte offset / packet index of the skipped unit (see variant docs).
        position: u64,
    },
    /// A byte-stream TS input **lost sync** (ISO/IEC 13818-1 §2.4.3.2): a
    /// packet's `sync_byte` was not `0x47` (or garbage sat between packets), so
    /// `dropped_bytes` bytes starting at byte `offset` were discarded until the
    /// resynchroniser re-locked on a run of sync bytes. Whatever those bytes
    /// carried (PES payload, a PSI section) is lost. One event per lost-sync
    /// span, never for junk before the first lock (a capture that starts
    /// mid-packet). `provenance.packet_index` is the index of the next intact
    /// packet.
    SyncLoss {
        /// Byte offset of the first discarded byte, from the start of the input.
        offset: u64,
        /// Number of bytes discarded before sync was re-acquired.
        dropped_bytes: u64,
    },
    /// The TS input **ended on a truncated packet**: `dropped_bytes` (fewer than
    /// a whole packet) trailing bytes starting at `offset` were discarded.
    TruncatedPacket {
        /// Byte offset of the first discarded byte, from the start of the input.
        offset: u64,
        /// Number of trailing bytes discarded.
        dropped_bytes: u64,
    },
}

impl InputDegradation {
    /// A short, stable label for this degradation — used by `Display`.
    pub fn name(&self) -> &'static str {
        match self {
            InputDegradation::TransportError => "transport error",
            InputDegradation::ContinuityGap { .. } => "continuity gap",
            InputDegradation::InvalidParameterSet => "invalid parameter set",
            InputDegradation::MalformedUnit { .. } => "malformed unit",
            InputDegradation::SyncLoss { .. } => "sync loss",
            InputDegradation::TruncatedPacket { .. } => "truncated packet",
        }
    }
}

broadcast_common::impl_spec_display!(InputDegradation);

/// One [`DemuxEvent::InputDegraded`] as kept on [`Media::degraded_at`](crate::ir::Media::degraded_at):
/// the [`InputDegradation`] plus *where* it was observed, so a consumer can
/// tell damage on an unused data PID (a DSM-CC carousel) from damage on a
/// track it keeps. `#[non_exhaustive]`: build one with [`DegradationRecord::new`].
#[non_exhaustive]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DegradationRecord {
    /// What degraded.
    pub kind: InputDegradation,
    /// The track the carrying PID maps to. On a [`Media::degraded_at`](crate::ir::Media::degraded_at)
    /// record from the one-shot TS demuxer this is resolved against the final
    /// [`Media::tracks`](crate::ir::Media::tracks) (match on
    /// `spec.source_pid == provenance.pid`), so damage seen before a Data /
    /// DSM-CC PID was announced still names its track (issue #1348). `None`
    /// means the PID never became a track, or the damage is stream-wide
    /// (`provenance.pid == None`, e.g. sync loss); never fabricated. The live
    /// [`DemuxEvent::InputDegraded::track`] is only what had resolved when the
    /// event fired, so it may be `None` for a PID that later becomes a track.
    pub track: Option<TrackId>,
    /// The TS PID and packet index the event was observed at.
    pub provenance: EventProvenance,
}

impl DegradationRecord {
    /// Build a record from the fields of a [`DemuxEvent::InputDegraded`].
    pub fn new(
        kind: InputDegradation,
        track: Option<TrackId>,
        provenance: EventProvenance,
    ) -> Self {
        Self {
            kind,
            track,
            provenance,
        }
    }
}

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
    /// An existing track's metadata changed in place. Carries the track's
    /// full, current [`TrackSpec`] — always the same `track_id`, so a consumer
    /// that replaces its stored spec keyed by `track_id` keeps its timeline.
    ///
    /// Two sources raise it today:
    /// - TS (issue #774): the PMT-derived metadata changed — its
    ///   `es_info_descriptors` (e.g. a corrected `ISO_639_language_descriptor`)
    ///   — with the codec config unchanged, since PMT-declared config recovery
    ///   stays single-shot and permanent.
    /// - FLV (r04-W13): the publisher re-sent the track's sequence header
    ///   (`AVCPacketType`/`AACPacketType` == 0), which encoders do routinely
    ///   after a resolution, profile, sample-rate or channel-count change. Here
    ///   `config` **does** change, and a consumer must treat it as "reload the
    ///   init segment / rebuild the sample-entry" — the previous config described
    ///   a stream that is no longer being sent.
    TrackUpdated(TrackSpec),
    /// A previously-declared track is no longer listed in the source's track
    /// declaration (e.g. a PMT version change that drops a PID). Only ever
    /// raised for a track that had already fired [`DemuxEvent::TrackAdded`]
    /// (i.e. a real `track_id` exists) — see the type-level "Removal
    /// semantics" note above.
    #[non_exhaustive]
    TrackRemoved {
        /// The removed track's ID (matches a prior [`DemuxEvent::TrackAdded`]).
        track_id: TrackId,
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
        track_id: Option<TrackId>,
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
        track_id: TrackId,
        /// The coded sample.
        sample: Sample,
    },
    /// A clock reference observed in the source container (e.g. an MPEG-2 TS
    /// PCR, ISO/IEC 13818-1 §2.4.3.4/§2.4.3.5). A container with no such
    /// concept (FLV, RTP) never emits this.
    ///
    /// **TS attribution (issue #1354).** A program's clock is the PCR carried
    /// on the `PCR_PID` its PMT names (§2.4.4.8-9, Table 2-33; §2.4.3.5: the PCR
    /// fields are "carried in the adaptation field of the transport stream
    /// packets with a PID value equal to the PCR_PID"), and a multiplex has one
    /// independent clock per program. So once the PMTs the PAT announces are
    /// all known, a PCR is emitted **only** from a PID that some program names
    /// as its `PCR_PID`, with `provenance.program_number` set to that
    /// program (one event per program when several share a PCR_PID, so a
    /// per-program consumer never has to guess; dedupe on `provenance.pid`
    /// for a PID-level view). A PCR on any other PID is dropped. **Before**
    /// that point (PAT not seen, or an announced PMT still pending) a PCR on
    /// a PID no applied PMT claims is emitted with `program_number: None`
    /// rather than held: holding would lose the first PMT-interval of clock
    /// samples, and a PID whose program has not been described yet cannot be
    /// ruled out as a PCR_PID. `None` therefore means "not yet attributable",
    /// never "no program".
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
        track: Option<TrackId>,
        /// What kind of discontinuity this is.
        kind: DiscontinuityKind,
        /// Container-native identity for this observation (e.g. the TS PID),
        /// when the container has one.
        provenance: EventProvenance,
    },
    /// A signal from the source container that the input degraded in a way
    /// the demuxer cannot repair — e.g. an MPEG-2 TS transport-error
    /// indicator (`tei`) or a continuity-counter gap (issue #778). A
    /// consumer repackaging a lossy UDP multicast can expose this as an
    /// operational metric; it is the difference between a diagnosable fault
    /// and a mystery.
    #[non_exhaustive]
    InputDegraded {
        /// The track this degradation was observed on, when the carrying
        /// PID/stream had already resolved to one at the time it fired.
        /// `None` before that resolution (never fabricated) — degradation
        /// can legitimately be observed on a PID whose track isn't known yet.
        track: Option<TrackId>,
        /// What degraded.
        kind: InputDegradation,
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
    /// A PID that the demuxer has *stepped over* — a non-video probe that saw
    /// 512 access units without resolving, or a declared PID that has sent no
    /// packet at all for 8192 TS packets — counts as resolved for this signal:
    /// it means "nothing more is being waited on", not "every declared PID has a
    /// track". Such a PID is not abandoned: if it does resolve later it is
    /// announced with a [`DemuxEvent::TrackAdded`] (a later id, its backlog
    /// replayed) and this event fires again, so a consumer that keys a
    /// track-set change off it sees the late track.
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
        track: Option<TrackId>,
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
    pub fn track_removed(track_id: TrackId, provenance: EventProvenance) -> Self {
        DemuxEvent::TrackRemoved {
            track_id,
            provenance,
        }
    }

    /// Construct a [`DemuxEvent::TrackAbandoned`] — see
    /// [`Self::clock_reference`] for why this variant needs a constructor.
    pub fn track_abandoned(
        track_id: Option<TrackId>,
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
    pub fn sample(track_id: TrackId, sample: Sample) -> Self {
        DemuxEvent::Sample { track_id, sample }
    }

    /// Construct a [`DemuxEvent::InputDegraded`] — see
    /// [`Self::clock_reference`] for why this variant needs a constructor.
    pub fn input_degraded(
        track: Option<TrackId>,
        kind: InputDegradation,
        provenance: EventProvenance,
    ) -> Self {
        DemuxEvent::InputDegraded {
            track,
            kind,
            provenance,
        }
    }

    /// Construct a [`DemuxEvent::TracksResolved`] — see
    /// [`Self::clock_reference`] for why this variant needs a constructor.
    pub fn tracks_resolved(generation: u32) -> Self {
        DemuxEvent::TracksResolved { generation }
    }
}
