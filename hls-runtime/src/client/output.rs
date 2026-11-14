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
#[non_exhaustive]
pub struct VariantPolicy {
    /// Maximum allowed `BANDWIDTH` in bits per second.
    pub max_bandwidth: Option<u32>,
    /// Maximum allowed resolution height in pixels.
    pub max_height: Option<u32>,
    /// Allowed codec prefixes (e.g. `avc1`, `mp4a`): every comma-separated
    /// `CODECS` entry must have one of these as a prefix.
    pub supported_codecs: Option<Vec<String>>,
}

impl VariantPolicy {
    /// Set the maximum allowed `BANDWIDTH` in bits per second.
    #[must_use]
    pub fn with_max_bandwidth(mut self, max_bandwidth: u32) -> Self {
        self.max_bandwidth = Some(max_bandwidth);
        self
    }

    /// Set the maximum allowed resolution height in pixels.
    #[must_use]
    pub fn with_max_height(mut self, max_height: u32) -> Self {
        self.max_height = Some(max_height);
        self
    }

    /// Set the allowed codec prefixes.
    #[must_use]
    pub fn with_supported_codecs(mut self, supported_codecs: Vec<String>) -> Self {
        self.supported_codecs = Some(supported_codecs);
        self
    }
}

/// Information about one variant stream in a master playlist.
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub struct VariantInfo {
    /// The URI the manifest declares for it: an absolute HLS media playlists' URL, a DASH
    /// Representation's Initialization Segment URL, or empty for a protocol whose variant has
    /// no URI of its own (Smooth, whose fragments live under the manifest's URL).
    pub uri: String,
    /// Its peak bandwidth in bits per second.
    ///
    /// [`u64`] because the protocols that report one do not share a width: an HLS
    /// `BANDWIDTH` and a DASH `@bandwidth` are a `u32` and a `u64` (both range-checked where
    /// they parse), and every value is carried as given rather than clamped into the narrower.
    pub bandwidth: u64,
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

impl VariantInfo {
    /// Build a variant description from its required URI and bandwidth.
    #[must_use]
    pub fn new(uri: String, bandwidth: u64) -> Self {
        Self {
            uri,
            bandwidth,
            average_bandwidth: None,
            resolution: None,
            codecs: None,
            audio_group: None,
            subtitles_group: None,
        }
    }

    /// Attach `AVE-RAGE-BANDWIDTH`.
    #[must_use]
    pub fn with_average_bandwidth(mut self, average_bandwidth: u32) -> Self {
        self.average_bandwidth = Some(average_bandwidth);
        self
    }

    /// Attach `RESOLUTION`.
    #[must_use]
    pub fn with_resolution(mut self, resolution: (u32, u32)) -> Self {
        self.resolution = Some(resolution);
        self
    }

    /// Attach `CODECS`.
    #[must_use]
    pub fn with_codecs(mut self, codecs: String) -> Self {
        self.codecs = Some(codecs);
        self
    }

    /// Attach the `AUDIO` group id.
    #[must_use]
    pub fn with_audio_group(mut self, audio_group: String) -> Self {
        self.audio_group = Some(audio_group);
        self
    }

    /// Attach the `SUBTITLES` group id.
    #[must_use]
    pub fn with_subtitles_group(mut self, subtitles_group: String) -> Self {
        self.subtitles_group = Some(subtitles_group);
        self
    }
}

/// Information about one `#EXT-X-MEDIA` rendition in a master playlist.
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
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

impl RenditionInfo {
    /// Build a rendition description from its required type, group id, and
    /// name.
    #[must_use]
    pub fn new(media_type: MediaType, group_id: String, name: String) -> Self {
        Self {
            media_type,
            group_id,
            name,
            language: None,
            default: false,
            autoselect: false,
            url: None,
        }
    }

    /// Attach `LANGUAGE`.
    #[must_use]
    pub fn with_language(mut self, language: String) -> Self {
        self.language = Some(language);
        self
    }

    /// Set the `DEFAULT` flag.
    #[must_use]
    pub fn with_default(mut self, default: bool) -> Self {
        self.default = default;
        self
    }

    /// Set the `AUTOSELECT` flag.
    #[must_use]
    pub fn with_autoselect(mut self, autoselect: bool) -> Self {
        self.autoselect = autoselect;
        self
    }

    /// Attach the resolved absolute URI.
    #[must_use]
    pub fn with_url(mut self, url: String) -> Self {
        self.url = Some(url);
        self
    }
}

/// The result of selecting a variant from a master playlist.
#[derive(Debug, Clone, PartialEq)]
pub struct MasterSelection {
    /// Variants in playlist order.
    pub variants: Vec<VariantInfo>,
    /// Index of the chosen variant in [`Self::variants`].
    pub chosen: usize,
    /// Renditions exposed by the master playlist. Renditions with a `url` are
    /// not fetched by this client; create a second `HlsClient::new(hls_runtime::client::HlsClientConfig::new(rendition.url))`
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
        /// 14496-12:2026 §8.3.2) carried by the preceding [`Output::Init`].
        track_id: TrackId,
        /// The samples, in decode order.
        samples: Vec<Sample>,
    },
    /// `EXT-X-DISCONTINUITY` (RFC 8216 §4.3.2.3): the encoding, timestamps,
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
    /// A PMT-listed elementary stream of a TS segment was left out of the track set, the
    /// synthesized init and the samples because it cannot be carried in fragmented MP4
    /// ([`transmux::CodecConfig::is_muxable_in_bmff`] is false: opaque data such as SCTE-35,
    /// ID3 or private PES, or a subtitle without a sample entry). Emitted each time the demuxer
    /// (re)announces such a track, so a consumer that reports it must deduplicate by
    /// `track_id`. A TS-segment stream with no muxable track at all produces only these.
    TrackDropped {
        /// The dropped track's id.
        track_id: TrackId,
        /// The PMT `stream_type` (ISO/IEC 13818-1 Table 2-34) when the track is opaque data.
        stream_type: Option<u8>,
    },
    /// A track of an fMP4 Media Initialization Section was left out of the
    /// track set and the samples because the demuxer could not model it: an
    /// encrypted sample entry (`encv`, `enca`, ... — this client decrypts only
    /// AES-128 segments, never CENC/CBCS sample encryption) or an unrecognised
    /// codec. Emitted once per track and init, so a consumer that reports it
    /// deduplicates by `track_id` across inits. A stream whose every track is
    /// skipped produces only these, followed by an empty [`Output::Tracks`].
    TrackSkipped {
        /// The skipped `trak`'s `tkhd.track_ID`, `None` when the demuxer could
        /// not name it.
        track_id: Option<TrackId>,
        /// The track's sample-entry four-character code (`encv`, `enca`, ...).
        fourcc: String,
        /// The demuxer's reason.
        reason: String,
    },
    /// The demuxer skipped one or more `moof` fragments of a Part/Segment
    /// resource — unparsable, or addressing sample data outside the resource
    /// (an absolute `tfhd` base-data-offset, which only holds when the
    /// segment sits at that offset of the whole file; CMAF uses
    /// default-base-is-moof). The samples of the other fragments are still
    /// emitted; those of the skipped ones are lost.
    FragmentDamaged {
        /// The resource that held the skipped fragments.
        id: crate::client::ResourceId,
        /// How many fragments (or per-track fragment runs) were skipped.
        skipped: usize,
    },
    /// A master (multivariant) playlist was parsed and a variant selected.
    /// The caller may inspect [`MasterSelection`] and, for renditions with a
    /// `url`, create a second client via `HlsClient::new(hls_runtime::client::HlsClientConfig::new(rendition.url))`.
    Master(MasterSelection),
}
