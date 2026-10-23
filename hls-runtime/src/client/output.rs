//! Output events — [`Output`] out of [`crate::client::HlsClient::next_output`].

use alloc::string::String;
use alloc::sync::Arc;
use alloc::vec::Vec;
use broadcast_hls::MediaType;
use transmux::TrackId;

use transmux::DisplayInfo;
use transmux::Sample;
use transmux::TrackSpec;

/// Policy for choosing a variant from a multivariant (master) playlist.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct VariantPolicy {
    /// Maximum allowed `BANDWIDTH` in bits per second.
    pub max_bandwidth: Option<u32>,
    /// Maximum allowed resolution height in pixels.
    pub max_height: Option<u32>,
    /// Allowed codec prefixes (e.g. `avc1`, `mp4a`): every comma-separated
    /// `CODECS` entry must have one of these as a prefix.
    pub supported_codecs: Option<Vec<String>>,
}

/// Information about one variant stream in a master playlist.
#[derive(Debug, Clone, PartialEq)]
pub struct VariantInfo {
    /// Resolved absolute URL of the variant's media playlist.
    pub url: String,
    /// `BANDWIDTH` in bits per second.
    pub bandwidth: u32,
    /// `AVERAGE-BANDWIDTH`, if present.
    pub average_bandwidth: Option<u32>,
    /// `RESOLUTION` as `(width, height)`, if present.
    pub resolution: Option<(u32, u32)>,
    /// `CODECS` attribute, if present.
    pub codecs: Option<String>,
    /// `AUDIO` group id, if any.
    pub audio_group: Option<String>,
    /// `SUBTITLES` group id, if any.
    pub subtitles_group: Option<String>,
}

/// Information about one `#EXT-X-MEDIA` rendition in a master playlist.
#[derive(Debug, Clone, PartialEq)]
pub struct RenditionInfo {
    /// Rendition type.
    pub media_type: MediaType,
    /// `GROUP-ID`.
    pub group_id: String,
    /// `NAME`.
    pub name: String,
    /// `LANGUAGE`, if present.
    pub language: Option<String>,
    /// `DEFAULT` flag.
    pub default: bool,
    /// `AUTOSELECT` flag.
    pub autoselect: bool,
    /// Resolved absolute URI, or `None` when the rendition is carried inside
    /// the variant's own segments (e.g. closed captions).
    pub url: Option<String>,
}

/// The result of selecting a variant from a master playlist.
#[derive(Debug, Clone, PartialEq)]
pub struct MasterSelection {
    /// Variants in playlist order.
    pub variants: Vec<VariantInfo>,
    /// Index of the chosen variant in [`Self::variants`].
    pub chosen: usize,
    /// Renditions exposed by the master playlist. Renditions with a `url` are
    /// not fetched by this client; create a second `HlsClient::new(rendition.url)`
    /// to play one.
    pub renditions: Vec<RenditionInfo>,
}

/// One unit of decoded output, drained in order via
/// [`crate::client::HlsClient::next_output`].
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
        track_id: TrackId,
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
    /// The full current track set for the stream. Emitted before `Init` on the
    /// classic TS-segment path and after the first `Init` on the fMP4 path.
    Tracks(Vec<TrackSpec>),
    /// A track's display state changed (SPS geometry etc.).
    DisplayChanged {
        /// Track ID.
        track_id: TrackId,
        /// PTS of the affected access unit, if known.
        pts: Option<i64>,
        /// The full new display state, shared with the following sample.
        display: Arc<DisplayInfo>,
    },
    /// A master (multivariant) playlist was parsed and a variant selected.
    /// The caller may inspect [`MasterSelection`] and, for renditions with a
    /// `url`, create a second client via `HlsClient::new(rendition.url)`.
    Master(MasterSelection),
}
