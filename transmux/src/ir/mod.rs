//! The transmux media intermediate representation (IR).
//!
//! This is the neutral hub form every demuxer produces and every mux stage or
//! manifest renderer consumes:
//!
//! - [`TrackSpec`] — one elementary track's identity/codec config
//!   ([`CodecConfig`]), with optional CENC/CBCS track crypto defaults
//!   ([`TrackProtection`]); per-sample IVs live on [`Sample::protection`].
//! - [`Sample`] — one coded access unit: data bytes, absolute optional
//!   `dts`/`pts` (media plane step 2c), optional duration, [`SampleFlags`],
//!   and optional debug-only [`Provenance`].
//! - [`CodecConfig`] — per-track codec configuration (decoder config record +
//!   geometry, or the opaque [`CodecConfig::Data`] carriage for PMT-listed
//!   streams this crate does not decode — see [`DataCarriage`]).
//! - [`FragmentTrackData`] — one track's samples for a single media segment,
//!   the input shape [`crate::pipeline::build_media_segment`] consumes.
//! - [`DisplayGeometry`] — a video track's display geometry (coded size and
//!   the SPS VUI sample aspect ratio, `broadcast_bitstream::facts::AspectRatioIdc`), carried on
//!   [`TrackSpec::display`] (design: `docs/display-geometry/DESIGN.md`).
//! - [`DemuxEvent`] — the streaming-demux event vocabulary (media plane step
//!   2e), drained from [`crate::ts_demux::StreamingTsDemux`] /
//!   [`crate::flv_stream::StreamingFlvDemux`]; [`EventProvenance`] carries the
//!   container-native identity (e.g. a TS PID) a neutral event needs to omit
//!   from its primary fields.
//!
//! # Invariants
//!
//! - `Sample::timing` (`.dts`)/`Sample::timing` (`.pts`) are stored *absolutely*, in the track's
//!   media timescale (media plane step 2c) — `None` only for section-carried
//!   tracks (SCTE-35/DSM-CC/private sections), which have no timestamp at
//!   all, never fabricated. A track's start decode time is its first sample's
//!   `dts` when the track is timed — the DTS it sits at on the presentation
//!   timeline (maps onto the fragment `tfdt` `baseMediaDecodeTime`,
//!   ISO/IEC 14496-12:2026 §8.8.12).
//! - 33-bit (TS) / 32-bit (RTP) rollover is unwrapped once, at the demux edge
//!   (`crate::ts_demux`, `crate::rtp`) — never re-derived downstream.
//! - Samples within a track are always in decode order.
//! - A sample's [`SampleProtection`] carries its own IV and subsample map;
//!   [`TrackSpec::protection`], when present, must state the scheme and `tenc`
//!   those samples were protected under.
//!
//! This module is a pure type consolidation (media plane step 2a): the
//! packagers/depackagers built on this IR ([`Mp4Demux`](crate::mp4_demux::Mp4Demux),
//! [`CmafMuxStage`](crate::media::CmafMuxStage), [`HlsPackager`](crate::media::HlsPackager))
//! and the segment-authoring machinery ([`crate::pipeline::build_init_segment`] /
//! [`crate::pipeline::build_media_segment`]) stay in their existing modules —
//! only the data types moved.

mod audio_meta;
mod codec;
mod display;
mod event;
mod ids;
mod media;
mod sample;
mod track;

pub use audio_meta::{
    AudioLanguage, AudioMetadata, AudioRole, AudioType, EditorialClassification, MixType,
    SupplementaryAudio,
};
pub(crate) use codec::AUDIO_SAMPLE_SIZE_BITS;
pub use codec::{CodecConfig, DataCarriage, DataIdentity, SubtitleFormat, SubtitleSampleEntry};
pub use display::{
    ActiveFormat, AfdPersistence, DisplayChangeSet, DisplayGeometry, DisplayInfo,
    INFERRED_COLOUR_CODE, INFERRED_FULL_RANGE, INFERRED_VIDEO_FORMAT, MPEG2_INFERRED_COLOUR_CODE,
    UserDataCarriage,
};
pub use event::{
    AbandonReason, DegradationRecord, DemuxEvent, DiscontinuityKind, EventProvenance,
    InputDegradation,
};
pub use ids::{ProgramNumber, TrackId};
pub use media::{PcrSample, SkippedTrack};
pub use sample::{
    FragmentTrackData, Provenance, Sample, SampleFlags, SampleProtection, SampleTiming,
};
pub use track::{ConstantIvSenc, TrackProtection, TrackSpec};
