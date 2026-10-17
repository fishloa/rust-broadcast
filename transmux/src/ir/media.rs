//! The neutral hub IR — [`Media`], [`PcrSample`].
//!
//! Moved out of `transmux/src/media.rs` (media plane step 2a, no-op): same
//! types, same fields, same impls.

use alloc::string::String;
use alloc::vec::Vec;
use mpeg_ts::pid::Pid;

use super::{DegradationRecord, InputDegradation, Track};

/// One PCR observation from a TS adaptation field (ISO/IEC 13818-1 §2.4.3.4).
///
/// `#[non_exhaustive]`: construct with [`PcrSample::new`] — a future field
/// (e.g. the `pcr` vs `opcr` distinction of §2.4.3.5) should be additive, not
/// a major bump.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub struct PcrSample {
    /// `program_clock_reference` as a 27 MHz value (`base * 300 + extension`).
    pub pcr_27mhz: u64,
    /// PID the PCR was carried on.
    pub pid: Pid,
    /// 0-based index of the 188-byte packet in the demuxed input.
    pub packet_index: u64,
    /// The adaptation field's `discontinuity_indicator` (§2.4.3.5).
    pub discontinuity: bool,
}

impl PcrSample {
    /// Create a PCR observation with every field explicit.
    pub fn new(pcr_27mhz: u64, pid: Pid, packet_index: u64, discontinuity: bool) -> Self {
        Self {
            pcr_27mhz,
            pid,
            packet_index,
            discontinuity,
        }
    }
}

/// One track present in the source container that a demuxer could not model
/// into a [`Track`] — an unrecognised sample entry/codec (a QuickTime hint
/// track, a chapter/text track, `c608`/`c708`, GoPro `gpmd`, or any other
/// FourCC this crate has no [`crate::pipeline::CodecConfig`] reconstruction
/// for), or a structurally malformed `trak`.
///
/// DEMUX = lenient but loud (media plane step-2 fix wave 1, B2/B3): such a
/// track is skipped rather than failing the whole file, but it is never
/// silent — [`Fmp4Demux`](crate::media::Fmp4Demux) and
/// [`ProgressiveDemux`](crate::progressive_demux::ProgressiveDemux) both
/// record one of these per skipped track in [`Media::skipped`].
///
/// `#[non_exhaustive]`: construct with [`SkippedTrack::new`].
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct SkippedTrack {
    /// Best-effort sample-entry FourCC, decoded lossily as text. A
    /// placeholder (`"unknown"`) when the `trak` was too malformed to even
    /// reach its `stsd` entry.
    pub fourcc: String,
    /// Human-readable reason (the underlying `Error`'s `Display` text).
    pub reason: String,
}

impl SkippedTrack {
    /// Record a skipped track from its sample-entry FourCC and a
    /// human-readable reason.
    pub fn new(fourcc: String, reason: String) -> Self {
        Self { fourcc, reason }
    }
}

/// The media intermediate representation: a set of elementary [`Track`]s.
///
/// This is the hub's neutral form. [`Unpackage`](broadcast_common::Unpackage)
/// impls (e.g. [`Fmp4Demux`](crate::media::Fmp4Demux)) produce a `Media`;
/// [`Package`](broadcast_common::Package) impls (e.g.
/// [`CmafMux`](crate::media::CmafMux), [`HlsPackager`](crate::media::HlsPackager))
/// consume one.
///
/// `#[non_exhaustive]`: this is the hub's central type and its field set is
/// still growing — `pcr` and `skipped` were each added after the initial
/// release, and each addition was a breaking change that need not have been
/// one. Construct with [`Media::new`] and set further fields by assignment;
/// destructuring or matching from another crate needs a trailing `..`.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct Media {
    /// Elementary tracks, in the order they appear in the source movie.
    pub tracks: Vec<Track>,
    /// Movie timescale (`mvhd.timescale`), preserved for lossless re-muxing.
    pub movie_timescale: u32,
    /// PCR timeline recovered from the source, in wire order
    /// ([`PcrSample`], ISO/IEC 13818-1 §2.4.3.4). Empty for every demuxer that
    /// does not read a TS adaptation field (i.e. every non-[`TsDemux`](crate::ts_demux::TsDemux) source).
    pub pcr: Vec<PcrSample>,
    /// Tracks the demuxer found in the source container but could not model
    /// (media plane step-2 fix wave 1, B2/B3) — always empty unless the
    /// producing demuxer actually skipped something; see [`SkippedTrack`].
    pub skipped: Vec<SkippedTrack>,
    /// Damage a one-shot demuxer skipped over instead of failing the whole
    /// input (audit T16, #1144): one entry per skipped container unit (see
    /// [`InputDegradation::MalformedUnit`]), in input order. For a TS input it
    /// also carries every `InputDegraded` the streaming core raised
    /// ([`InputDegradation::SyncLoss`], [`InputDegradation::TruncatedPacket`],
    /// transport errors, continuity gaps). Empty for a clean input and for
    /// every demuxer that never resynchronises.
    pub degraded: Vec<InputDegradation>,
    /// Where each [`degraded`](Self::degraded) entry was observed. For a TS
    /// input this is parallel to `degraded` (same length, same order:
    /// `degraded_at[i].kind == degraded[i]`) and carries the PID and packet
    /// index, so a consumer can ignore damage on PIDs it does not keep (a
    /// DSM-CC carousel, say) and act on damage to its tracks. Empty for
    /// every demuxer whose container has no PID/packet provenance.
    pub degraded_at: Vec<DegradationRecord>,
}

impl Media {
    /// Create a `Media` from tracks and a movie timescale, with an empty PCR
    /// timeline and no skipped tracks.
    pub fn new(tracks: Vec<Track>, movie_timescale: u32) -> Self {
        Self {
            tracks,
            movie_timescale,
            pcr: Vec::new(),
            skipped: Vec::new(),
            degraded: Vec::new(),
            degraded_at: Vec::new(),
        }
    }

    /// Attach a PCR timeline, returning `self` (builder style).
    pub fn with_pcr(mut self, pcr: Vec<PcrSample>) -> Self {
        self.pcr = pcr;
        self
    }
}
