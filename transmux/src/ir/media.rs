//! The neutral hub IR — [`Media`], [`PcrSample`].
//!
//! Moved out of `transmux/src/media.rs` (media plane step 2a, no-op): same
//! types, same fields, same impls.

use alloc::vec::Vec;

use super::Track;

/// One PCR observation from a TS adaptation field (ISO/IEC 13818-1 §2.4.3.4).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PcrSample {
    /// `program_clock_reference` as a 27 MHz value (`base * 300 + extension`).
    pub pcr_27mhz: u64,
    /// PID the PCR was carried on.
    pub pid: u16,
    /// 0-based index of the 188-byte packet in the demuxed input.
    pub packet_index: u64,
    /// The adaptation field's `discontinuity_indicator` (§2.4.3.5).
    pub discontinuity: bool,
}

/// The media intermediate representation: a set of elementary [`Track`]s.
///
/// This is the hub's neutral form. [`Unpackage`](broadcast_common::Unpackage)
/// impls (e.g. [`Fmp4Demux`](crate::media::Fmp4Demux)) produce a `Media`;
/// [`Package`](broadcast_common::Package) impls (e.g.
/// [`CmafMux`](crate::media::CmafMux), [`HlsPackager`](crate::media::HlsPackager))
/// consume one.
#[derive(Debug, Clone)]
pub struct Media {
    /// Elementary tracks, in the order they appear in the source movie.
    pub tracks: Vec<Track>,
    /// Movie timescale (`mvhd.timescale`), preserved for lossless re-muxing.
    pub movie_timescale: u32,
    /// PCR timeline recovered from the source, in wire order
    /// ([`PcrSample`], ISO/IEC 13818-1 §2.4.3.4). Empty for every demuxer that
    /// does not read a TS adaptation field (i.e. every non-[`TsDemux`](crate::ts_demux::TsDemux) source).
    pub pcr: Vec<PcrSample>,
}

impl Media {
    /// Create a `Media` from tracks and a movie timescale, with an empty PCR
    /// timeline.
    pub fn new(tracks: Vec<Track>, movie_timescale: u32) -> Self {
        Self {
            tracks,
            movie_timescale,
            pcr: Vec::new(),
        }
    }

    /// Attach a PCR timeline, returning `self` (builder style).
    pub fn with_pcr(mut self, pcr: Vec<PcrSample>) -> Self {
        self.pcr = pcr;
        self
    }
}
