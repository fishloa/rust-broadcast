//! The transmux media intermediate representation (IR).
//!
//! This is the neutral hub form every demuxer produces and every muxer
//! consumes (the transmux side of the `broadcast-common` container-mux
//! vocabulary, [`broadcast_common::Unpackage`] / [`broadcast_common::Package`]):
//!
//! - [`Media`] — a set of elementary [`Track`]s, plus the movie timescale and
//!   an optional recovered PCR timeline ([`PcrSample`]).
//! - [`Track`] — one elementary track: its identity/codec config
//!   ([`TrackSpec`] wrapping [`CodecConfig`]) plus its decode-ordered
//!   [`Sample`]s, an absolute decode-time anchor, and optional CENC/CBCS
//!   crypto metadata ([`TrackEncryption`]).
//! - [`Sample`] — one coded access unit: data bytes, duration, sync flag,
//!   composition offset, and optional [`SourceTiming`] recovered from the
//!   source container.
//! - [`CodecConfig`] — per-track codec configuration (decoder config record +
//!   geometry, or the opaque [`CodecConfig::Data`] carriage for PMT-listed
//!   streams this crate does not decode — see [`DataCarriage`]).
//! - [`FragmentTrackData`] — one track's samples for a single media segment,
//!   the input shape [`crate::pipeline::build_media_segment`] consumes.
//!
//! # Invariants
//!
//! - [`Sample`] timing is stored *relatively*: a sample's DTS is the running
//!   sum of preceding [`Sample::duration`]s within its track; PTS is
//!   `DTS + composition_offset`. [`Track::start_decode_time`] is the missing
//!   absolute anchor — the DTS the track's first sample sits at on the
//!   presentation timeline (maps onto the fragment `tfdt`
//!   `baseMediaDecodeTime`, ISO/IEC 14496-12:2015 §8.8.12).
//! - Samples within a [`Track`] are always in decode order.
//! - [`Track::encryption`], when present, must have exactly one
//!   [`crate::cenc::SampleEncryptionEntry`] per [`Track::samples`] entry.
//!
//! This module is a pure type consolidation (media plane step 2a): the
//! packagers/depackagers built on this IR ([`Fmp4Demux`](crate::media::Fmp4Demux),
//! [`CmafMux`](crate::media::CmafMux), [`HlsPackager`](crate::media::HlsPackager))
//! and the segment-authoring machinery ([`crate::pipeline::build_init_segment`] /
//! [`crate::pipeline::build_media_segment`]) stay in their existing modules —
//! only the data types moved.

mod codec;
mod media;
mod sample;
mod track;

pub use codec::{CodecConfig, DataCarriage};
pub use media::{Media, PcrSample};
pub use sample::{FragmentTrackData, Sample, SourceTiming};
pub use track::{Track, TrackEncryption, TrackSpec};
