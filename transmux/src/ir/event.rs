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
use super::track::Track;

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

/// A demux event, drained incrementally from a streaming demuxer (issue #555:
/// [`crate::ts_demux::StreamingTsDemux`]; issue #738:
/// [`crate::flv_stream::StreamingFlvDemux`]).
///
/// This is deliberately **not** a universal cross-crate event enum: it is the
/// demux family's own vocabulary (every variant is something a container
/// *demuxer* can observe), not shared with the segmenter/packetiser families,
/// which name their own `Out` types (media plane step 2e, spec §6 "Drive
/// shape, not vocabulary").
// `TrackAdded(Track)` is deliberately the large variant: a `Track` carries a
// full `TrackSpec` and its (usually empty, for a just-added track) sample
// buffer. Boxing it would change every `DemuxEvent::TrackAdded(track)` call
// site across this crate and downstream (`multimux`) to `Box<Track>` for a
// negligible win — `TrackAdded` fires once per track, not once per sample,
// so the size difference from `ClockReference`/`Discontinuity` (which fire
// far more often, and stay small on purpose) is intentional, not an
// oversight this lint should gate on.
#[allow(clippy::large_enum_variant)]
#[non_exhaustive]
#[derive(Debug, Clone)]
pub enum DemuxEvent {
    /// New track discovered. The codec config is fully recovered by the time
    /// this fires — an opaque [`crate::pipeline::CodecConfig::Data`] track
    /// fires on its very first access unit, since its config needs no
    /// in-band header at all.
    TrackAdded(Track),
    /// A completed access unit / audio frame, with absolute per-sample
    /// `dts`/`pts` (issue #556 semantics; media plane step 2c: absolute
    /// rather than carried in a separate `SourceTiming`).
    Sample {
        /// The owning track's ID (matches a prior [`DemuxEvent::TrackAdded`]).
        track_id: u32,
        /// The coded sample.
        sample: Sample,
    },
    /// A clock reference observed in the source container (e.g. an MPEG-2 TS
    /// PCR, ISO/IEC 13818-1 §2.4.3.4/§2.4.3.5). A container with no such
    /// concept (FLV, RTP) never emits this.
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
    Discontinuity {
        /// The track this discontinuity was observed on, when the carrying
        /// PID/stream had already resolved to one at the time it fired.
        /// `None` before that resolution (never fabricated) — a discontinuity
        /// can legitimately be observed on a PID whose track isn't known yet.
        track: Option<u32>,
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
    TracksResolved,
}
