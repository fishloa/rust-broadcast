//! Output events — [`Output`] out of [`crate::LlHlsClient::next_output`].

use alloc::vec::Vec;

use transmux::Sample;

/// One unit of decoded output, drained in order via
/// [`crate::LlHlsClient::next_output`].
///
/// The ordering contract: exactly one [`Output::Init`] is emitted before any
/// [`Output::Samples`] for a given track, and `Samples` are emitted in decode
/// order with no gap and no repeat — a fetched Part's samples are emitted
/// exactly once, whether it arrived as a standalone Part fetch or was
/// coalesced into its parent's closed-segment view (see the crate docs'
/// "Dedup" section).
#[derive(Debug, Clone)]
#[non_exhaustive]
pub enum Output {
    /// The Media Initialization Section bytes (`ftyp`+`moov`, RFC 8216bis
    /// §4.4.4.5's `EXT-X-MAP` resource) — hand to the decoder once before any
    /// `Samples`. Re-emitted only when the playlist's `EXT-X-MAP` changes
    /// (e.g. across an `EXT-X-DISCONTINUITY`).
    Init(Vec<u8>),
    /// Newly available coded samples (access units) for one track, in decode
    /// order, demuxed from a fetched Part or Segment resource via
    /// `transmux::Fmp4Demux`.
    Samples {
        /// Track ID, matching the `moov`'s `trak.tkhd.track_ID` (ISO/IEC
        /// 14496-12 §8.3.2) carried by the preceding [`Output::Init`].
        track_id: u32,
        /// The samples, in decode order.
        samples: Vec<Sample>,
    },
    /// `EXT-X-DISCONTINUITY` (RFC 8216 §4.3.4.3): the encoding, timestamps,
    /// tracks, or codec parameters may have changed between the previously
    /// emitted samples and the ones that follow this marker. Forwarded
    /// verbatim so the sink can flush/reset its decoder pipeline.
    Discontinuity,
    /// The playlist reached `#EXT-X-ENDLIST` (RFC 8216 §4.3.3.4) with no
    /// further open segment or preload hint outstanding — playback is
    /// complete, no more output will ever be produced.
    EndOfStream,
}
