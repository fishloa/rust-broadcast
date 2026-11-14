//! Microsoft Smooth Streaming (MS-SSTR) **client Manifest** — typed model,
//! parse and serialize.
//!
//! See `private/specs-verified/mpeg/transmux/smooth/ms-sstr.md` for the spec
//! transcription this module cites throughout (client Manifest §2.2.2.x, live
//! attributes §2.2.2.1, `c` timeline §2.2.2.6, URL token substitution
//! §2.2.4.1).
//!
//! # Structure ([MS-SSTR])
//!
//! - **`SmoothStreamingMedia`** (§2.2.2.1) — [`SmoothManifest`]:
//!   `MajorVersion`/`MinorVersion`, `TimeScale` (default [`SMOOTH_TIMESCALE`]),
//!   `Duration`, the live-only `IsLive`/`LookAheadFragmentCount`/
//!   `DVRWindowLength`, its `Protection`/`ProtectionHeader` ([`Protection`])
//!   and `StreamIndex` children.
//! - **`StreamIndex`** (§2.2.2.3) — [`StreamIndex`]: `Type` ([`StreamType`]),
//!   `Name`, `Subtype`, `Chunks`, `QualityLevels`, `TimeScale`, `Url` (the
//!   fragment-URL template), its `QualityLevel` and `c` children.
//! - **`QualityLevel`** (§2.2.2.5) — [`QualityLevel`]: `Index`, `Bitrate`,
//!   `FourCC`, `CodecPrivateData` (hex-decoded here), geometry/audio
//!   attributes.
//! - **`c`** (§2.2.2.6, `StreamFragmentElement`) — [`C`]: `n`/`t`/`d`/`r`. See
//!   [`StreamIndex::enumerate_chunks`] for the bounded `r`-expansion.
//!
//! # Bounded input (remote alloc-DoS defense)
//!
//! A client Manifest is fetched from an untrusted remote server. Two places
//! could otherwise drive unbounded allocation:
//! - a `c@r` repeat count — [`StreamIndex::enumerate_chunks`] errors
//!   ([`SmoothParseError::ChunkRunTooLong`]) rather than expanding past
//!   [`MAX_CHUNK_RUN`];
//! - a `QualityLevel@CodecPrivateData` hex string — [`hex_decode`] errors
//!   ([`SmoothParseError::CodecPrivateDataTooLong`]) rather than allocating
//!   past [`MAX_CODEC_PRIVATE_DATA_HEX_LEN`], checked *before* any decode
//!   allocation.
//!
//! The document itself is read under [`broadcast_common::xml_chars::XmlLimits`]
//! (see [`default_limits`]). No malformed/truncated/adversarial XML panics:
//! every failure path returns a [`SmoothParseError`], never an
//! `unwrap`/`expect`/`panic!` on parsed input.
//!
//! # Serializing
//!
//! [`SmoothManifest::to_xml`] builds the document from the typed model alone —
//! nothing is echoed from the parsed source, and nothing is added to it: one
//! `c` element per [`C`] in [`StreamIndex::chunks_list`] carrying exactly the
//! attributes that entry sets (`n`/`t`/`d`/`r`, each only when `Some`). A
//! model therefore re-parses to itself. A writer that numbers every chunk or
//! writes a `t` + `r` run as explicit start times does so in the model it
//! builds (as `transmux`'s `SmoothPackager` does), not here.

use alloc::string::{String, ToString};
use alloc::vec::Vec;
use core::fmt;
use core::str::FromStr;

use broadcast_common::timescale::Timescale;
use broadcast_common::xml::{Elem, Namespace, XmlError, read_tree};
use broadcast_common::xml_chars::{
    LimitExceeded, NO_ATTRS, XmlLimit, XmlLimits, XmlWriter, close_element as close,
    empty_element as empty, finish_document, new_writer, open_element as open, text_element,
    write_declaration,
};

// ---------------------------------------------------------------------------
// Error
// ---------------------------------------------------------------------------

/// Errors returned while parsing an MS-SSTR client Manifest.
///
/// The parser never panics on malformed or truncated input — every failure
/// path returns one of these variants instead.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum SmoothParseError {
    /// The root element (or an expected child) was not the element name the
    /// grammar requires at that position.
    UnexpectedElement {
        /// The element name required at this position.
        expected: &'static str,
        /// The element name actually found (empty if none was found at all).
        found: String,
    },
    /// A required attribute was absent from an element.
    MissingAttribute {
        /// The element's name.
        element: &'static str,
        /// The missing attribute's name.
        attr: &'static str,
    },
    /// An attribute's value could not be parsed as the type it must carry
    /// (or, for `StreamIndex@Type`, was not one of the known tokens).
    InvalidAttributeValue {
        /// The element's name.
        element: &'static str,
        /// The attribute's name.
        attr: &'static str,
        /// The raw (unparsable) value.
        value: String,
    },
    /// A `c@r` repeat run would exceed the cap on total expanded chunks
    /// (remote alloc-DoS defense — an untrusted Manifest specifying an
    /// unbounded `<c r="...">`).
    ChunkRunTooLong {
        /// The chunk count (or a hint of it) that breached the cap.
        count_hint: u64,
    },
    /// A `c@r` repeat count of zero, which would stand for zero fragments
    /// (MS-SSTR §2.2.2.6: `r` is one-based).
    InvalidRepeatCount {
        /// The offending value.
        r: u32,
    },
    /// A `c` entry whose duration cannot be derived: no explicit `@d`, no
    /// following `c` carrying an explicit `@t`, and no manifest `Duration`
    /// to take it from (MS-SSTR §2.2.2.6 start-time coding).
    MissingDuration {
        /// Index of the offending `c` entry.
        index: usize,
    },
    /// A `c` entry whose derived duration would be negative: the following
    /// entry's `@t` is not after this entry's `t` (MS-SSTR §2.2.2.6).
    NonMonotonicTime {
        /// Index of the offending `c` entry.
        index: usize,
    },
    /// A `c` entry with no `@d` and `@r > 1` whose span to the next entry's
    /// `@t` is not a whole multiple of `r` ticks, so its `r` fragments cannot
    /// share one duration (MS-SSTR §2.2.2.6).
    IndivisibleRun {
        /// Index of the offending `c` entry.
        index: usize,
    },
    /// A `QualityLevel@CodecPrivateData` hex string exceeded
    /// [`MAX_CODEC_PRIVATE_DATA_HEX_LEN`] (remote alloc-DoS defense), checked
    /// before any decode allocation.
    CodecPrivateDataTooLong {
        /// The hex string's length in bytes.
        len: usize,
    },
    /// A `CodecPrivateData` value was not valid hex (odd length, or a
    /// non-hex-digit byte).
    InvalidHex {
        /// The offending raw value.
        value: String,
    },
    /// The manifest exceeds a hostile-input bound
    /// ([`broadcast_common::xml_chars::XmlLimits`] defaults: 4 MiB, depth 32,
    /// 100 000 elements, 8 KiB per attribute value, 256 attributes per
    /// element).
    TooLarge {
        /// The bound that was exceeded.
        limit: XmlLimit,
        /// Its ceiling.
        max: usize,
    },
    /// Any other XML well-formedness error reported by `quick-xml` (bad
    /// entity or character reference, mismatched nesting detected by the
    /// reader, invalid encoding, …).
    Xml {
        /// Byte offset (into the input) where the reader stopped.
        pos: usize,
        /// `quick-xml`'s description of the problem.
        message: String,
    },
}

impl fmt::Display for SmoothParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            SmoothParseError::UnexpectedElement { expected, found } => {
                if found.is_empty() {
                    write!(f, "expected element <{expected}>, found none")
                } else {
                    write!(f, "expected element <{expected}>, found <{found}>")
                }
            }
            SmoothParseError::MissingAttribute { element, attr } => {
                write!(f, "<{element}> is missing required attribute @{attr}")
            }
            SmoothParseError::InvalidAttributeValue {
                element,
                attr,
                value,
            } => write!(f, "<{element}>@{attr} has invalid value {value:?}"),
            SmoothParseError::InvalidRepeatCount { r } => {
                write!(f, "<c>@r has invalid value {r} (must be >= 1)")
            }
            SmoothParseError::MissingDuration { index } => {
                write!(f, "<c> at index {index} duration cannot be derived")
            }
            SmoothParseError::IndivisibleRun { index } => {
                write!(f, "<c> at index {index} run does not divide evenly")
            }
            SmoothParseError::NonMonotonicTime { index } => {
                write!(f, "<c> at index {index} has non-monotonic time")
            }
            SmoothParseError::ChunkRunTooLong { count_hint } => {
                write!(
                    f,
                    "c@r repeat run exceeded max chunk count ({count_hint} > {})",
                    MAX_CHUNK_RUN
                )
            }
            SmoothParseError::CodecPrivateDataTooLong { len } => {
                write!(
                    f,
                    "CodecPrivateData hex length {len} exceeds max {}",
                    MAX_CODEC_PRIVATE_DATA_HEX_LEN
                )
            }
            SmoothParseError::InvalidHex { value } => {
                write!(f, "invalid CodecPrivateData hex {value:?}")
            }
            SmoothParseError::TooLarge { limit, max } => {
                write!(f, "manifest exceeds the {limit} limit of {max}")
            }
            SmoothParseError::Xml { pos, message } => {
                write!(f, "XML error at byte offset {pos}: {message}")
            }
        }
    }
}

#[cfg(feature = "std")]
impl std::error::Error for SmoothParseError {}

/// Crate-local result alias for this module.
type Result<T> = core::result::Result<T, SmoothParseError>;

/// Namespace classes the Smooth reader distinguishes: no namespace (the only one
/// whose elements and attributes are the manifest's own) and any other.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Ns {
    Smooth,
    Foreign,
}

impl Namespace for Ns {
    const TARGET: Self = Ns::Smooth;
    const NONE: Self = Ns::Smooth;

    fn classify(_uri: &str) -> Self {
        Ns::Foreign
    }
}

impl From<broadcast_common::xml_chars::XmlCharError> for SmoothParseError {
    /// A character-data or namespace failure, at the position the caller
    /// that raised it reports (the pull reader's own offset).
    fn from(e: broadcast_common::xml_chars::XmlCharError) -> Self {
        Self::Xml {
            pos: 0,
            message: e.to_string(),
        }
    }
}

impl From<XmlError> for SmoothParseError {
    fn from(e: XmlError) -> Self {
        match e {
            XmlError::NotWellFormed { position, message } => Self::Xml {
                pos: position,
                message,
            },
            XmlError::WrongRoot { found } => Self::UnexpectedElement {
                expected: ELEMENT_ROOT,
                found,
            },
            XmlError::Limit(LimitExceeded { limit, max }) => Self::TooLarge { limit, max },
            XmlError::InvalidUtf8 { valid_up_to } => Self::Xml {
                pos: valid_up_to,
                message: String::from("invalid UTF-8"),
            },
            _ => Self::Xml {
                pos: 0,
                message: String::from("unreadable document"),
            },
        }
    }
}

/// The input bounds [`SmoothManifest::parse`] applies: the shared
/// [`XmlLimits`] defaults with the attribute ceiling raised to twice
/// [`MAX_CODEC_PRIVATE_DATA_HEX_LEN`] so that parser's own typed cap (not the
/// generic one) is what rejects an oversized `CodecPrivateData`. Real Smooth
/// manifests are a few KiB; the 4 MiB byte bound covers a multi-hour live
/// manifest with 100 000 chunk entries.
#[must_use]
pub fn default_limits() -> XmlLimits {
    let mut limits = XmlLimits::default();
    limits.max_attribute_bytes = 2 * MAX_CODEC_PRIVATE_DATA_HEX_LEN;
    limits
}

// ---------------------------------------------------------------------------
// Unbounded-input caps (remote alloc-DoS defense)
// ---------------------------------------------------------------------------

/// Cap on total chunks expanded from a `StreamIndex`'s `c@r` repeat runs: a
/// hostile Manifest specifying a huge `<c r="...">` would allocate unboundedly
/// otherwise. 100,000 chunks is generous while still protecting against
/// allocation DoS.
pub const MAX_CHUNK_RUN: usize = 100_000;

/// Cap on a `QualityLevel@CodecPrivateData` hex string's length, checked
/// before any decode allocation. 65,536 hex characters (32 KiB decoded) is
/// far beyond any real H.264 SPS+PPS or AAC AudioSpecificConfig (each at most
/// a few hundred bytes), while still protecting against a hostile Manifest's
/// unbounded `CodecPrivateData` allocating without limit.
pub const MAX_CODEC_PRIVATE_DATA_HEX_LEN: usize = 65_536;

// ---------------------------------------------------------------------------
// Manifest wire constants
// ---------------------------------------------------------------------------

/// The Smooth default manifest time scale — 10 MHz (100 ns ticks),
/// `SmoothStreamingMedia@TimeScale` (§2.2.2.1).
pub const SMOOTH_TIMESCALE: u64 = SMOOTH_DEFAULT_TIMESCALE.as_u64();

/// The Smooth default manifest time scale as a [`Timescale`]: the value an
/// absent `SmoothStreamingMedia@TimeScale` resolves to.
pub const SMOOTH_DEFAULT_TIMESCALE: Timescale = Timescale::from_nonzero(10_000_000);

/// `SmoothStreamingMedia@MajorVersion` a constructed manifest carries (2).
pub const DEFAULT_MAJOR_VERSION: u32 = 2;
/// `SmoothStreamingMedia@MinorVersion` a constructed manifest carries (0).
pub const DEFAULT_MINOR_VERSION: u32 = 0;

/// The inner encoding the declaration names.
const XML_ENCODING: &str = "utf-8";

const ELEMENT_ROOT: &str = "SmoothStreamingMedia";
const ELEMENT_PROTECTION: &str = "Protection";
const ELEMENT_PROTECTION_HEADER: &str = "ProtectionHeader";
const ELEMENT_STREAM_INDEX: &str = "StreamIndex";
const ELEMENT_QUALITY_LEVEL: &str = "QualityLevel";
const ELEMENT_C: &str = "c";

const ATTR_MAJOR_VERSION: &str = "MajorVersion";
const ATTR_MINOR_VERSION: &str = "MinorVersion";
const ATTR_DURATION: &str = "Duration";
const ATTR_TIMESCALE: &str = "TimeScale";
const ATTR_IS_LIVE: &str = "IsLive";
const ATTR_LOOK_AHEAD_FRAGMENT_COUNT: &str = "LookAheadFragmentCount";
const ATTR_DVR_WINDOW_LENGTH: &str = "DVRWindowLength";
const ATTR_TYPE: &str = "Type";
const ATTR_NAME: &str = "Name";
const ATTR_SUBTYPE: &str = "Subtype";
const ATTR_CHUNKS: &str = "Chunks";
const ATTR_QUALITY_LEVELS: &str = "QualityLevels";
const ATTR_URL: &str = "Url";
const ATTR_N: &str = "n";
const ATTR_INDEX: &str = "Index";
const ATTR_BITRATE: &str = "Bitrate";
const ATTR_FOUR_CC: &str = "FourCC";
const ATTR_CODEC_PRIVATE_DATA: &str = "CodecPrivateData";
const ATTR_MAX_WIDTH: &str = "MaxWidth";
const ATTR_MAX_HEIGHT: &str = "MaxHeight";
const ATTR_SAMPLING_RATE: &str = "SamplingRate";
const ATTR_CHANNELS: &str = "Channels";
const ATTR_BITS_PER_SAMPLE: &str = "BitsPerSample";
const ATTR_PACKET_SIZE: &str = "PacketSize";
const ATTR_AUDIO_TAG: &str = "AudioTag";
const ATTR_T: &str = "t";
const ATTR_D: &str = "d";
const ATTR_R: &str = "r";

/// The `IsLive` attribute's true token (§2.2.2.1).
const IS_LIVE_TRUE: &str = "TRUE";

// ---------------------------------------------------------------------------
// StreamType
// ---------------------------------------------------------------------------

/// `StreamIndex@Type` (§2.2.2.3), the media kind of a stream.
///
/// The token set is closed: a parser rejects any other `Type`, and callers
/// match every variant, so a new token is a deliberate breaking change rather
/// than something a `#[non_exhaustive]` wildcard arm would hide.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StreamType {
    /// `Type="video"`.
    Video,
    /// `Type="audio"`.
    Audio,
    /// `Type="text"` (timed text / subtitles).
    Text,
}

impl StreamType {
    /// The [MS-SSTR] `StreamIndex@Type` token.
    pub fn name(&self) -> &'static str {
        match self {
            StreamType::Video => "video",
            StreamType::Audio => "audio",
            StreamType::Text => "text",
        }
    }
}

broadcast_common::impl_spec_display!(StreamType);

// ---------------------------------------------------------------------------
// Model
// ---------------------------------------------------------------------------

/// A parsed MS-SSTR client Manifest (`SmoothStreamingMedia`, §2.2.2.1).
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub struct SmoothManifest {
    /// `SmoothStreamingMedia@MajorVersion` (default 2).
    pub major_version: u32,
    /// `SmoothStreamingMedia@MinorVersion` (default 0).
    pub minor_version: u32,
    /// `SmoothStreamingMedia@TimeScale` (default [`SMOOTH_TIMESCALE`]).
    pub timescale: Timescale,
    /// `SmoothStreamingMedia@Duration`, in `timescale` ticks.
    pub duration: Option<u64>,
    /// `SmoothStreamingMedia@IsLive` (default `false`).
    pub is_live: bool,
    /// `SmoothStreamingMedia@LookAheadFragmentCount` (live only).
    pub look_ahead_fragment_count: Option<u32>,
    /// `SmoothStreamingMedia@DVRWindowLength`, in `timescale` ticks (live
    /// only; absent/0 means an unbounded DVR window).
    pub dvr_window_length: Option<u64>,
    /// `SmoothStreamingMedia@Protection` (§2.2.2.2) — a `ProtectionHeader`
    /// element whose text is the PlayReady (`MMP`/`WRMHEADER`) header, when
    /// the presentation is protected. `None` when the manifest declares none.
    pub protection: Option<Protection>,
    /// The document's `StreamIndex` elements, in document order.
    pub streams: Vec<StreamIndex>,
}

/// The `Protection` element (`SmoothStreamingMedia@Protection`, §2.2.2.2).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
#[non_exhaustive]
pub struct Protection {
    /// The `ProtectionHeader` element's text (the opaque
    /// `WRMHEADER`/PlayReady header), as the document carried it.
    pub header: Option<String>,
}

/// A `StreamIndex` element (§2.2.2.3).
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub struct StreamIndex {
    /// `StreamIndex@Type` (required).
    pub stream_type: StreamType,
    /// `StreamIndex@Name`.
    pub name: Option<String>,
    /// `StreamIndex@Subtype`.
    pub subtype: Option<String>,
    /// `StreamIndex@Chunks` — the advertised fragment count.
    pub chunks: Option<u32>,
    /// `StreamIndex@TimeScale`, overriding the manifest-level `TimeScale` for
    /// this stream, when present.
    pub timescale: Option<Timescale>,
    /// `StreamIndex@QualityLevels` — the advertised quality-level count.
    pub quality_levels: Option<u32>,
    /// `StreamIndex@Url` — the fragment-URL template (e.g.
    /// `QualityLevels({bitrate})/Fragments(video={start time})`), resolved
    /// per-fragment via [`StreamIndex::resolve_fragment_url`].
    pub url: String,
    /// The stream's `QualityLevel` children, in document order.
    pub qualities: Vec<QualityLevel>,
    /// The stream's `c` (`StreamFragmentElement`) children, in document
    /// order — expand via [`StreamIndex::enumerate_chunks`].
    pub chunks_list: Vec<C>,
}

/// A `QualityLevel` element (§2.2.2.5).
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct QualityLevel {
    /// `QualityLevel@Index`.
    pub index: u32,
    /// `QualityLevel@Bitrate`, in bits/second.
    pub bitrate: u64,
    /// `QualityLevel@FourCC` (e.g. `H264` for video, `AACL` for AAC-LC audio).
    pub four_cc: String,
    /// `QualityLevel@CodecPrivateData`, hex-decoded to bytes (bounded, see
    /// [`MAX_CODEC_PRIVATE_DATA_HEX_LEN`]).
    pub codec_private_data: Vec<u8>,
    /// `QualityLevel@MaxWidth`, video only.
    pub width: Option<u32>,
    /// `QualityLevel@MaxHeight`, video only.
    pub height: Option<u32>,
    /// `QualityLevel@SamplingRate`, audio only.
    pub sampling_rate: Option<u32>,
    /// `QualityLevel@Channels`, audio only.
    pub channels: Option<u16>,
    /// `QualityLevel@BitsPerSample`, audio only.
    pub bits_per_sample: Option<u16>,
    /// `QualityLevel@PacketSize`, audio only.
    pub packet_size: Option<u32>,
    /// `QualityLevel@AudioTag`, audio only (255 = raw AAC).
    pub audio_tag: Option<u32>,
}

impl QualityLevel {
    /// A quality level of `bitrate` bits/second in `four_cc`, with the
    /// `CodecPrivateData` bytes the stream's fragments were encoded with.
    #[must_use]
    pub fn new(four_cc: impl Into<String>, bitrate: u64, codec_private_data: Vec<u8>) -> Self {
        Self {
            index: 0,
            bitrate,
            four_cc: four_cc.into(),
            codec_private_data,
            width: None,
            height: None,
            sampling_rate: None,
            channels: None,
            bits_per_sample: None,
            packet_size: None,
            audio_tag: None,
        }
    }

    /// Set `QualityLevel@Index`.
    #[must_use]
    pub fn with_index(mut self, index: u32) -> Self {
        self.index = index;
        self
    }

    /// Clear `QualityLevel@PacketSize` — the attribute a writer leaves out
    /// when it describes each packet implicitly (one access unit).
    #[must_use]
    pub fn without_packet_size(mut self) -> Self {
        self.packet_size = None;
        self
    }

    /// Set the video geometry (`QualityLevel@MaxWidth`/`@MaxHeight`).
    #[must_use]
    pub fn with_size(mut self, width: u32, height: u32) -> Self {
        self.width = Some(width);
        self.height = Some(height);
        self
    }

    /// Set the audio sample parameters: `QualityLevel@SamplingRate`,
    /// `@Channels`, `@BitsPerSample`, `@PacketSize` and `@AudioTag`.
    #[must_use]
    pub fn with_audio(
        mut self,
        sampling_rate: u32,
        channels: u16,
        bits_per_sample: u16,
        packet_size: u32,
        audio_tag: u32,
    ) -> Self {
        self.sampling_rate = Some(sampling_rate);
        self.channels = Some(channels);
        self.bits_per_sample = Some(bits_per_sample);
        self.packet_size = Some(packet_size);
        self.audio_tag = Some(audio_tag);
        self
    }
}

/// One `c` (`StreamFragmentElement`) entry (§2.2.2.6).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct C {
    /// `@n` — the fragment number within its `StreamIndex`, written only when
    /// present (a manifest may number none, some or all of its `c` entries).
    pub n: Option<u32>,
    /// `@t` — this entry's explicit absolute start time, in the enclosing
    /// stream's `TimeScale` ticks. Only the first `<c>` typically carries it
    /// explicitly; later entries derive it by accumulating `d` (see
    /// [`StreamIndex::enumerate_chunks`]).
    pub t: Option<u64>,
    /// `@d` — this entry's fragment duration, in `TimeScale` ticks.
    pub d: Option<u64>,
    /// `@r` — repeat count: this entry represents `r` fragments of duration
    /// `d` (default: one fragment, no repeat). `r = 0` is invalid. See
    /// [`StreamIndex::enumerate_chunks`]'s bounded expansion.
    pub r: Option<u32>,
}

impl C {
    /// A `c` entry of duration `d` (in the stream's `TimeScale` ticks) with no
    /// explicit start time and no repeat — the second and later entries of a
    /// fragment timeline.
    #[must_use]
    pub fn new(d: u64) -> Self {
        Self {
            n: None,
            t: None,
            d: Some(d),
            r: None,
        }
    }

    /// This entry with an explicit start time `t` (§2.2.2.6), which resets the
    /// accumulated timeline.
    #[must_use]
    pub fn at(mut self, t: u64) -> Self {
        self.t = Some(t);
        self
    }

    /// This entry numbered `n` (`@n`), the fragment number a writer numbers its
    /// timeline with.
    #[must_use]
    pub fn numbered(mut self, n: u32) -> Self {
        self.n = Some(n);
        self
    }

    /// This entry with `@r`, the repeat count: `r` fragments of duration
    /// `d`.
    #[must_use]
    pub fn repeated(mut self, r: u32) -> Self {
        self.r = Some(r);
        self
    }
}

// ---------------------------------------------------------------------------
// Bounded helpers
// ---------------------------------------------------------------------------

/// `{bitrate}` token in a `StreamIndex@Url` fragment-URL template (§2.2.4.1).
pub const TOKEN_BITRATE: &str = "{bitrate}";
/// `{start time}` token in a `StreamIndex@Url` fragment-URL template (§2.2.4.1).
pub const TOKEN_START_TIME: &str = "{start time}";

impl SmoothManifest {
    /// A VOD manifest at the default [`SMOOTH_TIMESCALE`] with the given
    /// streams and no duration; see [`SmoothManifest::with_duration`].
    #[must_use]
    pub fn new(streams: Vec<StreamIndex>) -> Self {
        Self {
            streams,
            ..Self::default()
        }
    }

    /// [`SmoothManifest::new`] with `SmoothStreamingMedia@Duration` set.
    #[must_use]
    pub fn with_duration(mut self, duration: u64) -> Self {
        self.duration = Some(duration);
        self
    }

    /// Make this a live (DVR) manifest: `IsLive="TRUE"`, a DVR window of
    /// `dvr_window_length` ticks and the server's look-ahead count (§2.2.2.1).
    #[must_use]
    pub fn live(mut self, dvr_window_length: u64, look_ahead_fragment_count: u32) -> Self {
        self.is_live = true;
        self.dvr_window_length = Some(dvr_window_length);
        self.look_ahead_fragment_count = Some(look_ahead_fragment_count);
        self
    }

    /// Attach a `Protection` header (§2.2.2.2) with `header` as its
    /// `ProtectionHeader` text.
    ///
    /// The text is opaque: the parsed model keeps the `WRMHEADER` a manifest
    /// carried, so a round trip of a document that also carried a
    /// `SystemID` (which the parser does not retain) differs only by that
    /// attribute.
    #[must_use]
    pub fn with_protection(mut self, header: impl Into<String>) -> Self {
        self.protection = Some(Protection {
            header: Some(header.into()),
        });
        self
    }
}

impl Default for SmoothManifest {
    fn default() -> Self {
        Self {
            major_version: DEFAULT_MAJOR_VERSION,
            minor_version: DEFAULT_MINOR_VERSION,
            timescale: SMOOTH_DEFAULT_TIMESCALE,
            duration: None,
            is_live: false,
            look_ahead_fragment_count: None,
            dvr_window_length: None,
            protection: None,
            streams: Vec::new(),
        }
    }
}

impl StreamIndex {
    /// A stream of `stream_type` whose fragments are addressed by the
    /// `url` template and served at the given `qualities`; attributes the
    /// caller does not need are left unset.
    #[must_use]
    pub fn new(
        stream_type: StreamType,
        url: impl Into<String>,
        qualities: Vec<QualityLevel>,
    ) -> Self {
        Self {
            stream_type,
            name: None,
            subtype: None,
            chunks: None,
            quality_levels: None,
            timescale: None,
            url: url.into(),
            qualities,
            chunks_list: Vec::new(),
        }
    }

    /// Set `StreamIndex@Name`, the attribute a second stream of the same
    /// `Type` needs for its fragment URLs to stay distinct.
    #[must_use]
    pub fn with_name(mut self, name: impl Into<String>) -> Self {
        self.name = Some(name.into());
        self
    }

    /// Set `StreamIndex@Subtype` (required on a text stream, §2.2.2.3).
    #[must_use]
    pub fn with_subtype(mut self, subtype: impl Into<String>) -> Self {
        self.subtype = Some(subtype.into());
        self
    }

    /// Set `StreamIndex@Chunks` and `StreamIndex@QualityLevels`, the advertised
    /// fragment and quality counts.
    #[must_use]
    pub fn with_counts(mut self, chunks: u32, quality_levels: u32) -> Self {
        self.chunks = Some(chunks);
        self.quality_levels = Some(quality_levels);
        self
    }

    /// Set `StreamIndex@TimeScale`, overriding the manifest-level one.
    #[must_use]
    pub fn with_timescale(mut self, timescale: Timescale) -> Self {
        self.timescale = Some(timescale);
        self
    }

    /// Set the `c` timeline.
    #[must_use]
    pub fn with_chunks(mut self, chunks: Vec<C>) -> Self {
        self.chunks_list = chunks;
        self
    }

    /// Whether `other` denotes the **same** stream as `self` across a manifest refresh.
    ///
    /// Identity is `Type` + `Name` + `Url` template (MS-SSTR 2.2.3, the attributes that name a
    /// stream); matching on `Type` alone collapses two audio `StreamIndex` elements onto the
    /// first, which then adopts the other's URL template and timeline.
    #[must_use]
    pub fn same_stream(&self, other: &StreamIndex) -> bool {
        self.stream_type == other.stream_type && self.name == other.name && self.url == other.url
    }

    /// Expand every `c` entry's `@r` repeat run into `(t, d)` pairs in
    /// presentation order (§2.2.2.6): `t` accumulates (explicit on an entry
    /// resets it, otherwise it continues from the previous entry's `t + d`),
    /// each entry contributing `r + 1` occurrences of its own `d`.
    ///
    /// A `c` without `@d` takes `(next.t - t) / r` as the duration of each of
    /// its `r` fragments (§2.2.2.6: a run shares one duration), and
    /// [`SmoothParseError::IndivisibleRun`] when that is not exact.
    ///
    /// An explicit `t` earlier than the end of the preceding entry (a repeat
    /// or a step back) is [`SmoothParseError::NonMonotonicTime`]; a later one
    /// is a gap and is kept.
    ///
    /// Returns [`SmoothParseError::ChunkRunTooLong`] if the total chunk count
    /// (summed across every `c`, counting each `r + 1` repetition) would
    /// exceed [`MAX_CHUNK_RUN`], defending against remote alloc-DoS attacks
    /// via a hostile Manifest with an unbounded `<c r="...">`.
    pub fn enumerate_chunks(&self) -> Result<Vec<(u64, u64)>> {
        let mut out = Vec::new();
        let mut t: u64 = 0;
        let mut total: u64 = 0;
        let list = &self.chunks_list;
        for (i, c) in list.iter().enumerate() {
            if let Some(explicit_t) = c.t {
                if explicit_t < t {
                    return Err(SmoothParseError::NonMonotonicTime { index: i });
                }
                t = explicit_t;
            }
            let repeats: u64 = match c.r {
                None => 1,
                Some(0) => {
                    return Err(SmoothParseError::InvalidRepeatCount { r: 0 });
                }
                Some(r) => u64::from(r),
            };
            let d = match c.d {
                Some(d) => d,
                None => {
                    let next_t = list.get(i + 1).and_then(|n| n.t);
                    match next_t {
                        Some(nt) => {
                            if nt < t {
                                return Err(SmoothParseError::NonMonotonicTime { index: i });
                            }
                            let span = nt - t;
                            if !span.is_multiple_of(repeats) {
                                return Err(SmoothParseError::IndivisibleRun { index: i });
                            }
                            span / repeats
                        }
                        None => {
                            return Err(SmoothParseError::MissingDuration { index: i });
                        }
                    }
                }
            };
            if repeats > MAX_CHUNK_RUN as u64 {
                return Err(SmoothParseError::ChunkRunTooLong {
                    count_hint: repeats,
                });
            }
            total = total.saturating_add(repeats);
            if total as usize > MAX_CHUNK_RUN {
                return Err(SmoothParseError::ChunkRunTooLong { count_hint: total });
            }
            for _ in 0..repeats {
                out.push((t, d));
                t = t.saturating_add(d);
            }
        }
        Ok(out)
    }

    /// Resolve this stream's `Url` fragment-URL template (§2.2.4.1) by
    /// literal substitution of the [`TOKEN_BITRATE`]/[`TOKEN_START_TIME`]
    /// tokens with the given quality's bitrate and a fragment's start time
    /// (the first element of one of [`Self::enumerate_chunks`]'s pairs).
    pub fn resolve_fragment_url(&self, bitrate: u64, start_time: u64) -> String {
        self.url
            .replace(TOKEN_BITRATE, &bitrate.to_string())
            .replace(TOKEN_START_TIME, &start_time.to_string())
    }
}

/// Hex-decode a `CodecPrivateData` attribute value into bytes, bounded by
/// [`MAX_CODEC_PRIVATE_DATA_HEX_LEN`] (checked *before* any decode
/// allocation, defending against a hostile Manifest's unbounded
/// `CodecPrivateData`) and erroring on odd length or a non-hex-digit byte
/// rather than panicking. `CodecPrivateData` is upper-case hex on the wire;
/// decoding is case-insensitive.
pub fn hex_decode(s: &str) -> Result<Vec<u8>> {
    if s.len() > MAX_CODEC_PRIVATE_DATA_HEX_LEN {
        return Err(SmoothParseError::CodecPrivateDataTooLong { len: s.len() });
    }
    broadcast_common::hex::hex_decode(s).ok_or_else(|| SmoothParseError::InvalidHex {
        value: s.to_string(),
    })
}

// ---------------------------------------------------------------------------
// Attribute helpers (XML parsing is in the `xml` module)
// ---------------------------------------------------------------------------

// ---------------------------------------------------------------------------
// Parsing
// ---------------------------------------------------------------------------

impl SmoothManifest {
    /// Parse a client Manifest document (§2.2.2) into this structural model.
    pub fn parse(xml: &str) -> Result<SmoothManifest> {
        Self::parse_with_limits(xml, default_limits())
    }

    /// [`SmoothManifest::parse`] under caller-chosen input bounds; a document
    /// past one fails with [`SmoothParseError::TooLarge`].
    pub fn parse_with_limits(xml: &str, limits: XmlLimits) -> Result<SmoothManifest> {
        const EL: &str = "SmoothStreamingMedia";
        let root = read_tree::<Ns>(xml.as_bytes(), ELEMENT_ROOT, &limits)?;

        let major_version: u32 = parse_attr(&root, "MajorVersion", EL)?.unwrap_or(2);
        let minor_version: u32 = parse_attr(&root, "MinorVersion", EL)?.unwrap_or(0);
        let timescale = parse_timescale(&root, EL)?.unwrap_or(SMOOTH_DEFAULT_TIMESCALE);
        let duration: Option<u64> = parse_attr(&root, "Duration", EL)?;
        let is_live = root
            .attr(ATTR_IS_LIVE)
            .is_some_and(|v| v.eq_ignore_ascii_case(IS_LIVE_TRUE));
        let look_ahead_fragment_count: Option<u32> =
            parse_attr(&root, "LookAheadFragmentCount", EL)?;
        let dvr_window_length: Option<u64> = parse_attr(&root, "DVRWindowLength", EL)?;

        let mut protection: Option<Protection> = None;
        let mut streams = Vec::new();
        for child in root.children.iter().filter(|c| c.ns == Ns::Smooth) {
            if child.name == ELEMENT_STREAM_INDEX {
                streams.push(parse_stream_index(child)?);
            } else if child.name == ELEMENT_PROTECTION && protection.is_none() {
                protection = Some(parse_protection(child)?);
            }
        }

        Ok(SmoothManifest {
            major_version,
            minor_version,
            timescale,
            duration,
            is_live,
            look_ahead_fragment_count,
            dvr_window_length,
            protection,
            streams,
        })
    }
}

fn attr<'a>(element: &'a Elem<Ns>, key: &str) -> Option<&'a str> {
    element.attr(key)
}

fn required_attr_owned(
    element: &Elem<Ns>,
    key: &'static str,
    name: &'static str,
) -> Result<String> {
    attr(element, key)
        .map(String::from)
        .ok_or(SmoothParseError::MissingAttribute {
            element: name,
            attr: key,
        })
}

fn parse_attr<T: FromStr>(
    element: &Elem<Ns>,
    key: &'static str,
    name: &'static str,
) -> Result<Option<T>> {
    match attr(element, key) {
        Some(v) => {
            v.trim()
                .parse::<T>()
                .map(Some)
                .map_err(|_| SmoothParseError::InvalidAttributeValue {
                    element: name,
                    attr: key,
                    value: v.to_string(),
                })
        }
        None => Ok(None),
    }
}

/// `TimeScale`: absent is `Ok(None)`; a present value that is zero, wider than
/// 32 bits or not a number is [`SmoothParseError::InvalidAttributeValue`].
fn parse_timescale(element: &Elem<Ns>, name: &'static str) -> Result<Option<Timescale>> {
    let Some(raw) = attr(element, ATTR_TIMESCALE) else {
        return Ok(None);
    };
    raw.trim()
        .parse::<u32>()
        .ok()
        .and_then(Timescale::new)
        .map(Some)
        .ok_or_else(|| SmoothParseError::InvalidAttributeValue {
            element: name,
            attr: ATTR_TIMESCALE,
            value: raw.to_string(),
        })
}

fn required_attr_parse<T: FromStr>(
    element: &Elem<Ns>,
    key: &'static str,
    name: &'static str,
) -> Result<T> {
    let v = attr(element, key).ok_or(SmoothParseError::MissingAttribute {
        element: name,
        attr: key,
    })?;
    v.trim()
        .parse::<T>()
        .map_err(|_| SmoothParseError::InvalidAttributeValue {
            element: name,
            attr: key,
            value: v.to_string(),
        })
}

fn parse_protection(element: &Elem<Ns>) -> Result<Protection> {
    let mut protection = Protection::default();
    for child in element.children.iter().filter(|c| c.ns == Ns::Smooth) {
        if child.name == ELEMENT_PROTECTION_HEADER && protection.header.is_none() {
            if !child.children.is_empty() {
                return Err(SmoothParseError::Xml {
                    pos: 0,
                    message: String::from("child element inside a text-only element"),
                });
            }
            protection.header = Some(child.text().to_string());
        }
    }
    Ok(protection)
}

fn parse_stream_index(element: &Elem<Ns>) -> Result<StreamIndex> {
    const EL: &str = "StreamIndex";
    let type_str = required_attr_owned(element, "Type", EL)?;
    let stream_type = match type_str.as_str() {
        "video" => StreamType::Video,
        "audio" => StreamType::Audio,
        "text" => StreamType::Text,
        _ => {
            return Err(SmoothParseError::InvalidAttributeValue {
                element: EL,
                attr: "Type",
                value: type_str,
            });
        }
    };
    let name = attr(element, "Name").map(String::from);
    let subtype = attr(element, "Subtype").map(String::from);
    let chunks: Option<u32> = parse_attr(element, "Chunks", EL)?;
    let quality_levels: Option<u32> = parse_attr(element, "QualityLevels", EL)?;
    let timescale = parse_timescale(element, EL)?;
    let url = required_attr_owned(element, "Url", EL)?;

    let mut qualities = Vec::new();
    let mut chunks_list = Vec::new();
    for child in element.children.iter().filter(|c| c.ns == Ns::Smooth) {
        if child.name == "QualityLevel" {
            qualities.push(parse_quality_level(child)?);
        } else if child.name == ELEMENT_C {
            let n: Option<u32> = parse_attr(child, ATTR_N, ELEMENT_C)?;
            let t: Option<u64> = parse_attr(child, "t", "c")?;
            let d: Option<u64> = parse_attr(child, "d", "c")?;
            let r: Option<u32> = parse_attr(child, "r", "c")?;
            chunks_list.push(C { n, t, d, r });
        }
    }

    Ok(StreamIndex {
        stream_type,
        name,
        subtype,
        chunks,
        quality_levels,
        timescale,
        url,
        qualities,
        chunks_list,
    })
}

fn parse_quality_level(element: &Elem<Ns>) -> Result<QualityLevel> {
    const EL: &str = "QualityLevel";
    let index: u32 = parse_attr(element, "Index", EL)?.unwrap_or(0);
    let bitrate: u64 = required_attr_parse(element, "Bitrate", EL)?;
    let four_cc = required_attr_owned(element, "FourCC", EL)?;
    let codec_private_data = match attr(element, "CodecPrivateData") {
        Some(hex) => hex_decode(hex)?,
        None => Vec::new(),
    };
    let width: Option<u32> = parse_attr(element, "MaxWidth", EL)?;
    let height: Option<u32> = parse_attr(element, "MaxHeight", EL)?;
    let sampling_rate: Option<u32> = parse_attr(element, "SamplingRate", EL)?;
    let channels: Option<u16> = parse_attr(element, "Channels", EL)?;
    let bits_per_sample: Option<u16> = parse_attr(element, "BitsPerSample", EL)?;
    let packet_size: Option<u32> = parse_attr(element, "PacketSize", EL)?;
    let audio_tag: Option<u32> = parse_attr(element, "AudioTag", EL)?;

    Ok(QualityLevel {
        index,
        bitrate,
        four_cc,
        codec_private_data,
        width,
        height,
        sampling_rate,
        channels,
        bits_per_sample,
        packet_size,
        audio_tag,
    })
}

// ---------------------------------------------------------------------------
// Serialization
// ---------------------------------------------------------------------------

/// One attribute of a start tag.
type Attribute<'a> = (&'a str, String);

impl SmoothManifest {
    /// Render this model as an MS-SSTR client Manifest document (§2.2.2).
    ///
    /// Every element and attribute the model holds is written, built from the
    /// typed model alone — nothing is echoed from a parsed source. Values are
    /// escaped by `quick-xml`, so any attribute and the `ProtectionHeader`
    /// text may carry any XML 1.0 character.
    ///
    /// The rendered document re-parses to an equal model, except where the
    /// parser normalises rather than stores: a `c` carrying both `t` and `r`
    /// (the `t` resumes the timeline and the run is shortened to the same
    /// fragments), a duplicate attribute, an attribute or child element the
    /// model has no field for (dropped), foreign-namespace content (dropped),
    /// and `IsLive` spelled other than `"TRUE"`/`"FALSE"` (re-read `false`).
    #[must_use]
    pub fn to_xml(&self) -> String {
        let mut writer = new_writer();
        let w = &mut writer;
        write_declaration(w, XML_ENCODING);
        open(w, ELEMENT_ROOT, &self.root_attributes());
        if let Some(protection) = &self.protection {
            write_protection(w, protection);
        }
        for stream in &self.streams {
            write_stream_index(w, stream);
        }
        close(w, ELEMENT_ROOT);
        finish_document(writer)
    }

    fn root_attributes(&self) -> Vec<Attribute<'_>> {
        let mut attrs = alloc::vec![
            (ATTR_MAJOR_VERSION, self.major_version.to_string()),
            (ATTR_MINOR_VERSION, self.minor_version.to_string()),
        ];
        if let Some(duration) = self.duration {
            attrs.push((ATTR_DURATION, duration.to_string()));
        }
        attrs.push((ATTR_TIMESCALE, self.timescale.get().to_string()));
        if self.is_live {
            attrs.push((ATTR_IS_LIVE, IS_LIVE_TRUE.to_string()));
        }
        if let Some(count) = self.look_ahead_fragment_count {
            attrs.push((ATTR_LOOK_AHEAD_FRAGMENT_COUNT, count.to_string()));
        }
        if let Some(window) = self.dvr_window_length {
            attrs.push((ATTR_DVR_WINDOW_LENGTH, window.to_string()));
        }
        attrs
    }
}

impl StreamIndex {
    fn attributes(&self) -> Vec<Attribute<'_>> {
        let mut attrs = alloc::vec![(ATTR_TYPE, self.stream_type.name().to_string())];
        if let Some(name) = &self.name {
            attrs.push((ATTR_NAME, name.clone()));
        }
        if let Some(subtype) = &self.subtype {
            attrs.push((ATTR_SUBTYPE, subtype.clone()));
        }
        if let Some(chunks) = self.chunks {
            attrs.push((ATTR_CHUNKS, chunks.to_string()));
        }
        if let Some(count) = self.quality_levels {
            attrs.push((ATTR_QUALITY_LEVELS, count.to_string()));
        }
        if let Some(timescale) = self.timescale {
            attrs.push((ATTR_TIMESCALE, timescale.get().to_string()));
        }
        attrs.push((ATTR_URL, self.url.clone()));
        attrs
    }
}

impl QualityLevel {
    fn attributes(&self) -> Vec<Attribute<'_>> {
        let mut attrs = alloc::vec![
            (ATTR_INDEX, self.index.to_string()),
            (ATTR_BITRATE, self.bitrate.to_string()),
            (ATTR_FOUR_CC, self.four_cc.clone()),
        ];
        if let Some(width) = self.width {
            attrs.push((ATTR_MAX_WIDTH, width.to_string()));
        }
        if let Some(height) = self.height {
            attrs.push((ATTR_MAX_HEIGHT, height.to_string()));
        }
        if let Some(rate) = self.sampling_rate {
            attrs.push((ATTR_SAMPLING_RATE, rate.to_string()));
        }
        if let Some(channels) = self.channels {
            attrs.push((ATTR_CHANNELS, channels.to_string()));
        }
        if let Some(bits) = self.bits_per_sample {
            attrs.push((ATTR_BITS_PER_SAMPLE, bits.to_string()));
        }
        if let Some(size) = self.packet_size {
            attrs.push((ATTR_PACKET_SIZE, size.to_string()));
        }
        if let Some(tag) = self.audio_tag {
            attrs.push((ATTR_AUDIO_TAG, tag.to_string()));
        }
        if !self.codec_private_data.is_empty() {
            attrs.push((
                ATTR_CODEC_PRIVATE_DATA,
                broadcast_common::hex::hex_encode(&self.codec_private_data).to_uppercase(),
            ));
        }
        attrs
    }
}

impl C {
    fn attributes(&self) -> Vec<Attribute<'static>> {
        let mut attrs = Vec::new();
        if let Some(n) = self.n {
            attrs.push((ATTR_N, n.to_string()));
        }
        if let Some(t) = self.t {
            attrs.push((ATTR_T, t.to_string()));
        }
        if let Some(d) = self.d {
            attrs.push((ATTR_D, d.to_string()));
        }
        if let Some(r) = self.r {
            attrs.push((ATTR_R, r.to_string()));
        }
        attrs
    }
}

fn write_protection(w: &mut XmlWriter, protection: &Protection) {
    match &protection.header {
        Some(header) => {
            open(w, ELEMENT_PROTECTION, NO_ATTRS);
            text_element(w, ELEMENT_PROTECTION_HEADER, header);
            close(w, ELEMENT_PROTECTION);
        }
        None => empty(w, ELEMENT_PROTECTION, NO_ATTRS),
    }
}

fn write_stream_index(w: &mut XmlWriter, stream: &StreamIndex) {
    open(w, ELEMENT_STREAM_INDEX, &stream.attributes());
    for quality in &stream.qualities {
        empty(w, ELEMENT_QUALITY_LEVEL, &quality.attributes());
    }
    for chunk in &stream.chunks_list {
        empty(w, ELEMENT_C, &chunk.attributes());
    }
    close(w, ELEMENT_STREAM_INDEX);
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::format;

    /// Character data inside a skipped subtree (e.g. `<Protection>`) or between
    /// elements is validated like modelled text.
    #[test]
    fn invalid_character_data_in_skipped_content_is_rejected() {
        for bad in ["&nope;", "&#1;", "&#x1;", "\u{1}", "<![CDATA[\u{1}]]>"] {
            let skipped = format!(
                "<SmoothStreamingMedia><Protection><ProtectionHeader>{bad}</ProtectionHeader></Protection></SmoothStreamingMedia>"
            );
            assert!(
                matches!(
                    SmoothManifest::parse(&skipped),
                    Err(SmoothParseError::Xml { .. })
                ),
                "skipped {bad:?}"
            );
            let between = format!(
                "<SmoothStreamingMedia>{bad}<StreamIndex Type=\"video\"/></SmoothStreamingMedia>"
            );
            assert!(
                matches!(
                    SmoothManifest::parse(&between),
                    Err(SmoothParseError::Xml { .. })
                ),
                "between {bad:?}"
            );
        }
    }

    const SMALL_MANIFEST: &str = r#"<?xml version="1.0" encoding="utf-8"?>
<SmoothStreamingMedia MajorVersion="2" MinorVersion="0" Duration="30000000" TimeScale="10000000">
  <StreamIndex Type="video" Subtype="" Chunks="2" QualityLevels="1" Url="QualityLevels({bitrate})/Fragments(video={start time})">
    <QualityLevel Index="0" Bitrate="500000" FourCC="H264" MaxWidth="640" MaxHeight="360" CodecPrivateData="000000016742C01EAB0000000168CE3C80"/>
    <c n="0" t="0" d="20000000"/>
    <c n="1" d="10000000"/>
  </StreamIndex>
  <StreamIndex Type="audio" Subtype="" Chunks="1" QualityLevels="1" Url="QualityLevels({bitrate})/Fragments(audio={start time})">
    <QualityLevel Index="0" Bitrate="128000" FourCC="AACL" SamplingRate="44100" Channels="2" BitsPerSample="16" AudioTag="255" CodecPrivateData="1210"/>
    <c d="30000000"/>
  </StreamIndex>
</SmoothStreamingMedia>"#;

    #[test]
    fn parses_small_manifest_structure() {
        let m = SmoothManifest::parse(SMALL_MANIFEST).expect("parse");
        assert_eq!(m.major_version, 2);
        assert_eq!(m.minor_version, 0);
        assert_eq!(m.timescale, Timescale::from_nonzero(10_000_000));
        assert_eq!(m.duration, Some(30_000_000));
        assert!(!m.is_live);
        assert_eq!(m.look_ahead_fragment_count, None);
        assert_eq!(m.streams.len(), 2);

        let video = &m.streams[0];
        assert_eq!(video.stream_type, StreamType::Video);
        assert_eq!(video.chunks, Some(2));
        assert_eq!(
            video.url,
            "QualityLevels({bitrate})/Fragments(video={start time})"
        );
        assert_eq!(video.qualities.len(), 1);
        let vq = &video.qualities[0];
        assert_eq!(vq.four_cc, "H264");
        assert_eq!(vq.bitrate, 500_000);
        assert_eq!(vq.width, Some(640));
        assert_eq!(vq.height, Some(360));
        assert_eq!(
            vq.codec_private_data,
            alloc::vec![
                0x00, 0x00, 0x00, 0x01, 0x67, 0x42, 0xC0, 0x1E, 0xAB, 0x00, 0x00, 0x00, 0x01, 0x68,
                0xCE, 0x3C, 0x80
            ]
        );
        assert_eq!(video.chunks_list.len(), 2);

        let audio = &m.streams[1];
        assert_eq!(audio.stream_type, StreamType::Audio);
        let aq = &audio.qualities[0];
        assert_eq!(aq.four_cc, "AACL");
        assert_eq!(aq.sampling_rate, Some(44100));
        assert_eq!(aq.channels, Some(2));
        assert_eq!(aq.audio_tag, Some(255));
        assert_eq!(aq.codec_private_data, alloc::vec![0x12, 0x10]);
    }

    #[test]
    fn live_manifest_shape_parses() {
        let xml = r#"<SmoothStreamingMedia MajorVersion="2" MinorVersion="0" TimeScale="10000000" IsLive="TRUE" LookAheadFragmentCount="2" DVRWindowLength="600000000">
            <StreamIndex Type="video" Url="QualityLevels({bitrate})/Fragments(video={start time})">
                <QualityLevel Index="0" Bitrate="1" FourCC="H264"/>
            </StreamIndex>
        </SmoothStreamingMedia>"#;
        let m = SmoothManifest::parse(xml).expect("parse live manifest");
        assert!(m.is_live);
        assert_eq!(m.look_ahead_fragment_count, Some(2));
        assert_eq!(m.dvr_window_length, Some(600_000_000));
    }

    #[test]
    fn missing_is_live_defaults_false() {
        let xml = r#"<SmoothStreamingMedia MajorVersion="2" MinorVersion="0" TimeScale="10000000">
        </SmoothStreamingMedia>"#;
        let m = SmoothManifest::parse(xml).expect("parse");
        assert!(!m.is_live);
    }

    // -- chunk enumeration ---------------------------------------------------

    #[test]
    fn enumerate_chunks_accumulates_and_expands_r() {
        let si = StreamIndex {
            stream_type: StreamType::Video,
            name: None,
            subtype: None,
            chunks: None,
            quality_levels: None,
            timescale: None,
            url: String::new(),
            qualities: Vec::new(),
            chunks_list: alloc::vec![
                C {
                    n: None,
                    t: Some(0),
                    d: Some(1000),
                    r: Some(2),
                },
                C {
                    n: None,
                    t: None,
                    d: Some(500),
                    r: None,
                },
            ],
        };
        let chunks = si.enumerate_chunks().expect("enumerate");
        assert_eq!(chunks, alloc::vec![(0, 1000), (1000, 1000), (2000, 500)]);
    }

    #[test]
    fn a1_repeat_r_is_one_based() {
        let xml = r#"<SmoothStreamingMedia MajorVersion="2" MinorVersion="0" TimeScale="100"><StreamIndex Type="video" Url="u"><QualityLevel Index="0" Bitrate="1" FourCC="H264"/><c t="0" d="100" r="3"/></StreamIndex></SmoothStreamingMedia>"#;
        let m = SmoothManifest::parse(xml).unwrap();
        assert_eq!(
            m.streams[0].enumerate_chunks().unwrap(),
            alloc::vec![(0, 100), (100, 100), (200, 100)]
        );
    }

    #[test]
    fn a1_repeat_r_zero_is_invalid() {
        let xml = r#"<SmoothStreamingMedia MajorVersion="2" MinorVersion="0" TimeScale="100"><StreamIndex Type="video" Url="u"><QualityLevel Index="0" Bitrate="1" FourCC="H264"/><c t="0" d="100" r="0"/></StreamIndex></SmoothStreamingMedia>"#;
        let m = SmoothManifest::parse(xml).unwrap();
        assert!(matches!(
            m.streams[0].enumerate_chunks().unwrap_err(),
            SmoothParseError::InvalidRepeatCount { r: 0 }
        ));
    }

    #[test]
    fn a2_derives_duration_from_next_t() {
        let xml = r#"<SmoothStreamingMedia MajorVersion="2" MinorVersion="0" TimeScale="100"><StreamIndex Type="video" Url="u"><QualityLevel Index="0" Bitrate="1" FourCC="H264"/><c t="0"/><c t="100"/><c t="200" d="100"/></StreamIndex></SmoothStreamingMedia>"#;
        let m = SmoothManifest::parse(xml).unwrap();
        assert_eq!(
            m.streams[0].enumerate_chunks().unwrap(),
            alloc::vec![(0, 100), (100, 100), (200, 100)]
        );
    }

    #[test]
    fn a2_missing_duration_reports_index() {
        let xml = r#"<SmoothStreamingMedia MajorVersion="2" MinorVersion="0" TimeScale="100"><StreamIndex Type="video" Url="u"><QualityLevel Index="0" Bitrate="1" FourCC="H264"/><c t="0"/></StreamIndex></SmoothStreamingMedia>"#;
        let m = SmoothManifest::parse(xml).unwrap();
        assert!(matches!(
            m.streams[0].enumerate_chunks().unwrap_err(),
            SmoothParseError::MissingDuration { index: 0 }
        ));
    }

    #[test]
    fn a2_non_monotonic_time_reports_error() {
        let xml = r#"<SmoothStreamingMedia MajorVersion="2" MinorVersion="0" TimeScale="100"><StreamIndex Type="video" Url="u"><QualityLevel Index="0" Bitrate="1" FourCC="H264"/><c t="100"/><c t="50" d="10"/></StreamIndex></SmoothStreamingMedia>"#;
        let m = SmoothManifest::parse(xml).unwrap();
        assert!(matches!(
            m.streams[0].enumerate_chunks().unwrap_err(),
            SmoothParseError::NonMonotonicTime { .. }
        ));
    }

    fn single_stream(chunks: &str) -> StreamIndex {
        let xml = alloc::format!(
            r#"<SmoothStreamingMedia MajorVersion="2" MinorVersion="0" TimeScale="100"><StreamIndex Type="video" Url="u"><QualityLevel Index="0" Bitrate="1" FourCC="H264"/>{chunks}</StreamIndex></SmoothStreamingMedia>"#
        );
        SmoothManifest::parse(&xml).unwrap().streams.remove(0)
    }

    #[test]
    fn explicit_t_that_repeats_or_goes_back_is_non_monotonic_with_d_present() {
        for chunks in [
            r#"<c t="0" d="100"/><c t="0" d="100"/>"#,
            r#"<c t="0" d="100"/><c t="50" d="100"/>"#,
            r#"<c t="200" d="100"/><c t="100" d="100"/>"#,
        ] {
            assert_eq!(
                single_stream(chunks).enumerate_chunks().unwrap_err(),
                SmoothParseError::NonMonotonicTime { index: 1 },
                "{chunks}"
            );
        }
    }

    #[test]
    fn explicit_t_after_a_gap_is_accepted() {
        assert_eq!(
            single_stream(r#"<c t="0" d="100"/><c t="300" d="100"/>"#)
                .enumerate_chunks()
                .unwrap(),
            alloc::vec![(0, 100), (300, 100)]
        );
    }

    #[test]
    fn absent_d_with_repeat_divides_the_run_between_explicit_times() {
        assert_eq!(
            single_stream(r#"<c t="0" r="3"/><c t="300" d="100"/>"#)
                .enumerate_chunks()
                .unwrap(),
            alloc::vec![(0, 100), (100, 100), (200, 100), (300, 100)]
        );
    }

    #[test]
    fn absent_d_with_repeat_that_does_not_divide_exactly_is_an_error() {
        assert_eq!(
            single_stream(r#"<c t="0" r="3"/><c t="301" d="100"/>"#)
                .enumerate_chunks()
                .unwrap_err(),
            SmoothParseError::IndivisibleRun { index: 0 }
        );
    }

    #[test]
    fn a3_protection_header_text() {
        for (body, expected) in [
            (r#"abc&amp;def"#, "abc&def"),
            (r#"ab<!--c-->de"#, "abde"),
            (r#"ab<![CDATA[<x>]]>cd"#, "ab<x>cd"),
            (r#"a&#13;b"#, "a\rb"),
        ] {
            let xml = format!(
                r#"<SmoothStreamingMedia><Protection><ProtectionHeader SystemID="x">{body}</ProtectionHeader></Protection></SmoothStreamingMedia>"#
            );
            let m = SmoothManifest::parse(&xml).unwrap();
            assert_eq!(m.protection.unwrap().header.unwrap(), expected);
        }
        let m = SmoothManifest::new(vec![]).with_protection("a&b\r\nc");
        let xml = m.to_xml();
        let m2 = SmoothManifest::parse(&xml).unwrap();
        assert_eq!(m2.protection.unwrap().header.unwrap(), "a&b\r\nc");
    }

    #[test]
    fn enumerate_chunks_matches_fixture_timeline() {
        let m = SmoothManifest::parse(SMALL_MANIFEST).unwrap();
        let chunks = m.streams[0].enumerate_chunks().unwrap();
        assert_eq!(
            chunks,
            alloc::vec![(0, 20_000_000), (20_000_000, 10_000_000)]
        );
    }

    // -- URL resolution -------------------------------------------------------

    #[test]
    fn resolve_fragment_url_substitutes_tokens() {
        let m = SmoothManifest::parse(SMALL_MANIFEST).unwrap();
        let video = &m.streams[0];
        let resolved = video.resolve_fragment_url(500_000, 20_000_000);
        assert_eq!(resolved, "QualityLevels(500000)/Fragments(video=20000000)");
    }

    // -- DoS caps: must bite --------------------------------------------------

    #[test]
    fn chunk_run_cap_bites_without_huge_alloc() {
        let xml = r#"<SmoothStreamingMedia MajorVersion="2" MinorVersion="0" TimeScale="10000000">
            <StreamIndex Type="video" Url="Fragments(video={start time})">
                <QualityLevel Index="0" Bitrate="1" FourCC="H264"/>
                <c d="1" r="4000000000"/>
            </StreamIndex>
        </SmoothStreamingMedia>"#;
        let m = SmoothManifest::parse(xml).expect("XML shape itself parses");
        let err = m.streams[0].enumerate_chunks().unwrap_err();
        assert!(matches!(err, SmoothParseError::ChunkRunTooLong { .. }));
    }

    #[test]
    fn codec_private_data_hex_cap_bites() {
        // One character past the cap must be rejected before any decode
        // allocation is attempted.
        let huge_hex: String = "AB".repeat(MAX_CODEC_PRIVATE_DATA_HEX_LEN / 2 + 1);
        let err = hex_decode(&huge_hex).unwrap_err();
        assert!(matches!(
            err,
            SmoothParseError::CodecPrivateDataTooLong { .. }
        ));
    }

    #[test]
    fn codec_private_data_hex_cap_bites_via_manifest_parse() {
        let huge_hex: String = "AB".repeat(MAX_CODEC_PRIVATE_DATA_HEX_LEN / 2 + 1);
        let xml = alloc::format!(
            r#"<SmoothStreamingMedia MajorVersion="2" MinorVersion="0" TimeScale="10000000">
                <StreamIndex Type="video" Url="Fragments(video={{start time}})">
                    <QualityLevel Index="0" Bitrate="1" FourCC="H264" CodecPrivateData="{huge_hex}"/>
                </StreamIndex>
            </SmoothStreamingMedia>"#
        );
        let err = SmoothManifest::parse(&xml).unwrap_err();
        assert!(matches!(
            err,
            SmoothParseError::CodecPrivateDataTooLong { .. }
        ));
    }

    // -- hex_decode -----------------------------------------------------------

    #[test]
    fn hex_decode_round_trips() {
        assert_eq!(
            hex_decode("0001ABff").unwrap(),
            alloc::vec![0x00, 0x01, 0xAB, 0xFF]
        );
    }

    #[test]
    fn hex_decode_rejects_odd_length() {
        assert!(matches!(
            hex_decode("ABC"),
            Err(SmoothParseError::InvalidHex { .. })
        ));
    }

    #[test]
    fn hex_decode_rejects_non_hex() {
        assert!(matches!(
            hex_decode("ZZ"),
            Err(SmoothParseError::InvalidHex { .. })
        ));
    }

    #[test]
    fn hex_decode_empty_is_empty() {
        assert_eq!(hex_decode("").unwrap(), Vec::<u8>::new());
    }

    // -- malformed / truncated input: must error, never panic ----------------

    #[test]
    fn unterminated_tag_is_error_not_panic() {
        let err = SmoothManifest::parse("<SmoothStreamingMedia MajorVersion=\"2\"").unwrap_err();
        assert!(matches!(err, SmoothParseError::Xml { .. }));
    }

    #[test]
    fn wrong_root_element_is_error() {
        let err = SmoothManifest::parse("<NotSmooth/>").unwrap_err();
        assert!(matches!(err, SmoothParseError::UnexpectedElement { .. }));
    }

    #[test]
    fn empty_input_is_error() {
        assert!(matches!(
            SmoothManifest::parse("").unwrap_err(),
            SmoothParseError::Xml { .. }
        ));
    }

    #[test]
    fn missing_required_attribute_is_error() {
        // QualityLevel without @Bitrate.
        let xml = r#"<SmoothStreamingMedia MajorVersion="2" MinorVersion="0" TimeScale="10000000">
            <StreamIndex Type="video" Url="x">
                <QualityLevel Index="0" FourCC="H264"/>
            </StreamIndex>
        </SmoothStreamingMedia>"#;
        let err = SmoothManifest::parse(xml).unwrap_err();
        assert!(matches!(
            err,
            SmoothParseError::MissingAttribute {
                element: "QualityLevel",
                attr: "Bitrate"
            }
        ));
    }

    #[test]
    fn unknown_stream_type_is_error() {
        let xml = r#"<SmoothStreamingMedia MajorVersion="2" MinorVersion="0" TimeScale="10000000">
            <StreamIndex Type="bogus" Url="x"/>
        </SmoothStreamingMedia>"#;
        let err = SmoothManifest::parse(xml).unwrap_err();
        assert!(matches!(
            err,
            SmoothParseError::InvalidAttributeValue {
                element: "StreamIndex",
                attr: "Type",
                ..
            }
        ));
    }

    #[test]
    fn absent_time_scale_resolves_to_the_smooth_default() {
        let m = SmoothManifest::parse(
            "<SmoothStreamingMedia><StreamIndex Type=\"video\" Url=\"u\"/></SmoothStreamingMedia>",
        )
        .expect("parse");
        assert_eq!(m.timescale, SMOOTH_DEFAULT_TIMESCALE);
        assert_eq!(m.timescale.get(), 10_000_000);
        assert_eq!(m.streams[0].timescale, None);
    }

    #[test]
    fn present_zero_time_scale_is_a_typed_error() {
        let root = "<SmoothStreamingMedia TimeScale=\"0\"/>";
        assert_eq!(
            SmoothManifest::parse(root),
            Err(SmoothParseError::InvalidAttributeValue {
                element: "SmoothStreamingMedia",
                attr: "TimeScale",
                value: "0".to_string(),
            })
        );
        let stream = "<SmoothStreamingMedia><StreamIndex Type=\"video\" Url=\"u\" TimeScale=\"0\"/></SmoothStreamingMedia>";
        assert_eq!(
            SmoothManifest::parse(stream),
            Err(SmoothParseError::InvalidAttributeValue {
                element: "StreamIndex",
                attr: "TimeScale",
                value: "0".to_string(),
            })
        );
    }

    #[test]
    fn truncated_garbage_shapes_never_panic() {
        let inputs = [
            "<SmoothStreamingMedia",
            "<SmoothStreamingMedia MajorVersion=",
            "<SmoothStreamingMedia><StreamIndex",
            "<SmoothStreamingMedia><StreamIndex Type=\"video\"><QualityLevel",
            "<SmoothStreamingMedia></OtherTag>",
            "not xml at all",
            "<<<<<<<",
        ];
        for input in inputs {
            // The only assertion is that this doesn't panic; any Result is fine.
            let _ = SmoothManifest::parse(input);
        }
    }
}
