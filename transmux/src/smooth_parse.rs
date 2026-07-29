//! Microsoft Smooth Streaming ([MS-SSTR]) **client manifest** parser +
//! `CodecPrivateData` codec glue — the structural inverse of
//! [`crate::smooth::SmoothPackager`]'s writer.
//!
//! Smooth-pull ingest (issue #759, T1) needs to fetch a remote client
//! Manifest and resolve the fragment URLs it describes, exactly as
//! [`crate::dash_parse`] does for DASH (issue #758). Like that parser, this
//! one is dependency-free: it reuses the shared hand-rolled `xml_parse`
//! tokenizer (`no_std` `alloc`, crate-private) rather than writing a second
//! one.
//!
//! See [`transmux/docs/smooth/ms-sstr.md`](../../docs/smooth/ms-sstr.md) for
//! the full spec transcription this module cites throughout (client Manifest
//! §2.2.2.x, live attributes §2.2.2.1, `c` timeline §2.2.2.6, URL token
//! substitution §2.2.4.1).
//!
//! # Structure parsed ([MS-SSTR])
//!
//! - **`SmoothStreamingMedia`** (§2.2.2.1) — [`SmoothManifest`]:
//!   `MajorVersion`/`MinorVersion`, `TimeScale` (default
//!   [`crate::smooth::SMOOTH_TIMESCALE`]), `Duration`, the live-only
//!   `IsLive`/`LookAheadFragmentCount`/`DVRWindowLength`, its `StreamIndex`
//!   children.
//! - **`StreamIndex`** (§2.2.2.3) — [`StreamIndex`]: `Type` ([`StreamType`]),
//!   `Name`, `Subtype`, `Chunks`, `TimeScale`, `Url` (the fragment-URL
//!   template), its `QualityLevel` and `c` children.
//! - **`QualityLevel`** (§2.2.2.5) — [`QualityLevel`]: `Index`, `Bitrate`,
//!   `FourCC`, `CodecPrivateData` (hex-decoded here), geometry/audio
//!   attributes.
//! - **`c`** (§2.2.2.6, `StreamFragmentElement`) — [`C`]: `t`/`d`/`r`. See
//!   [`StreamIndex::enumerate_chunks`] for the bounded `r`-expansion.
//!
//! # Bounded input (remote alloc-DoS defense)
//!
//! A client Manifest is fetched from an untrusted remote server. Two places
//! could otherwise drive unbounded allocation, mirroring the defenses #758
//! was forced into by review:
//! - a `c@r` repeat count — [`StreamIndex::enumerate_chunks`] errors
//!   ([`SmoothParseError::ChunkRunTooLong`]) rather than expanding past
//!   [`MAX_CHUNK_RUN`];
//! - a `QualityLevel@CodecPrivateData` hex string — [`hex_decode`] errors
//!   ([`SmoothParseError::CodecPrivateDataTooLong`]) rather than allocating
//!   past [`MAX_CODEC_PRIVATE_DATA_HEX_LEN`], checked *before* any decode
//!   allocation.
//!
//! No malformed/truncated/adversarial XML panics: every failure path returns
//! a [`SmoothParseError`], never an `unwrap`/`expect`/`panic!` on parsed
//! input.
//!
//! # Init-segment synthesis (the big delta vs DASH)
//!
//! Smooth has **no** bootstrapping init segment — a `QualityLevel`'s
//! `CodecPrivateData` IS the codec config. [`track_spec_from_quality_level`]
//! builds the [`crate::pipeline::TrackSpec`] a caller feeds to
//! [`crate::pipeline::build_init_segment`] so [`crate::media::Fmp4Demux`]
//! (which hard-requires a `moov`) can then absorb the Smooth fragment stream:
//! - `FourCC="H264"`: the Annex-B-framed `CodecPrivateData` (start-code
//!   delimited SPS+PPS) is split with [`crate::annexb::iter_annexb_nals`] and
//!   classified/assembled into an `avcC` via
//!   [`crate::rtp_sdp::avc_config_from_sps_pps`] (no SPS-parsing duplication).
//! - `FourCC="AACL"`: the `CodecPrivateData` bytes ARE the
//!   `AudioSpecificConfig`, carried straight into `CodecConfig::Aac` via
//!   [`crate::rtp_sdp::aac_config_from_asc_bytes`].

use alloc::string::{String, ToString};
use alloc::vec::Vec;
use core::fmt;
use core::str::FromStr;

use crate::annexb::iter_annexb_nals;
use crate::error::{Error as CrateError, Result as CrateResult};
use crate::nal::{NalCodec, nal_unit_type};
use crate::nalu_types::{AvcPps, AvcSps};
use crate::pipeline::{CodecConfig, TrackSpec};
use crate::rtp_sdp::{aac_config_from_asc_bytes, avc_config_from_sps_pps};
use crate::smooth::SMOOTH_TIMESCALE;
use crate::xml_parse::{XmlError, XmlEvent, XmlTokenizer, skip_element};

// ---------------------------------------------------------------------------
// Error
// ---------------------------------------------------------------------------

/// Errors returned while parsing an MS-SSTR client Manifest.
///
/// Distinct from [`crate::Error`] and from [`crate::dash_parse::DashParseError`]
/// (though structurally a near-mirror of the latter) — this parser never
/// panics on malformed or truncated input; every failure path returns one of
/// these variants instead.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum SmoothParseError {
    /// The input ended before a well-formed document was found (e.g. an
    /// unclosed root element, or no `SmoothStreamingMedia` element at all).
    UnexpectedEof,
    /// A `<...>` tag, `<!--...-->` comment, `<?...?>` declaration, or
    /// `<!...>` markup declaration was never closed.
    UnterminatedTag {
        /// Byte offset (into the input) where the unterminated construct began.
        pos: usize,
    },
    /// An attribute inside a start tag was not well-formed
    /// (`name="value"`/`name='value'`, XML 1.0 §3.1).
    MalformedAttribute {
        /// Byte offset (into the input) of the offending attribute.
        pos: usize,
    },
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
    /// An end tag's name does not match the element currently open — a
    /// malformed nesting that would silently truncate the structure.
    MismatchedEndTag {
        /// The element name expected to close.
        expected: &'static str,
        /// The element name actually found in the closing tag.
        found: String,
    },
}

impl fmt::Display for SmoothParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            SmoothParseError::UnexpectedEof => {
                write!(
                    f,
                    "unexpected end of input while parsing Smooth manifest XML"
                )
            }
            SmoothParseError::UnterminatedTag { pos } => {
                write!(
                    f,
                    "unterminated XML tag/comment/declaration at byte offset {pos}"
                )
            }
            SmoothParseError::MalformedAttribute { pos } => {
                write!(f, "malformed XML attribute near byte offset {pos}")
            }
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
            SmoothParseError::MismatchedEndTag { expected, found } => {
                if found.is_empty() {
                    write!(f, "expected closing tag </{expected}>, found none")
                } else {
                    write!(f, "expected closing tag </{expected}>, found </{found}>")
                }
            }
        }
    }
}

#[cfg(feature = "std")]
impl std::error::Error for SmoothParseError {}

impl From<XmlError> for SmoothParseError {
    fn from(err: XmlError) -> Self {
        match err {
            XmlError::UnexpectedEof => SmoothParseError::UnexpectedEof,
            XmlError::UnterminatedTag { pos } => SmoothParseError::UnterminatedTag { pos },
            XmlError::MalformedAttribute { pos } => SmoothParseError::MalformedAttribute { pos },
            XmlError::MismatchedEndTag { expected, found } => {
                SmoothParseError::MismatchedEndTag { expected, found }
            }
        }
    }
}

/// Crate-local result alias for this module.
type Result<T> = core::result::Result<T, SmoothParseError>;

// ---------------------------------------------------------------------------
// Unbounded-input caps (remote alloc-DoS defense)
// ---------------------------------------------------------------------------

/// Cap on total chunks expanded from a `StreamIndex`'s `c@r` repeat runs.
/// Mirrors [`crate::dash_parse::MAX_TIMELINE_SEGMENTS`]: a hostile Manifest
/// specifying a huge `<c r="...">` would allocate unboundedly otherwise.
/// 100,000 chunks is generous while still protecting against allocation DoS.
pub const MAX_CHUNK_RUN: usize = 100_000;

/// Cap on a `QualityLevel@CodecPrivateData` hex string's length, checked
/// before any decode allocation. 65,536 hex characters (32 KiB decoded) is
/// far beyond any real H.264 SPS+PPS or AAC AudioSpecificConfig (each at most
/// a few hundred bytes), while still protecting against a hostile Manifest's
/// unbounded `CodecPrivateData` allocating without limit.
pub const MAX_CODEC_PRIVATE_DATA_HEX_LEN: usize = 65_536;

// ---------------------------------------------------------------------------
// StreamType
// ---------------------------------------------------------------------------

/// `StreamIndex@Type` (§2.2.2.3).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
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

/// A parsed MS-SSTR client Manifest (`SmoothStreamingMedia`, §2.2.2.1) — the
/// structural inverse of [`crate::smooth::SmoothPackager`]'s rendered XML
/// output.
#[derive(Debug, Clone, PartialEq)]
pub struct SmoothManifest {
    /// `SmoothStreamingMedia@MajorVersion` (default 2).
    pub major_version: u32,
    /// `SmoothStreamingMedia@MinorVersion` (default 0).
    pub minor_version: u32,
    /// `SmoothStreamingMedia@TimeScale` (default
    /// [`crate::smooth::SMOOTH_TIMESCALE`]).
    pub timescale: u64,
    /// `SmoothStreamingMedia@Duration`, in `timescale` ticks.
    pub duration: Option<u64>,
    /// `SmoothStreamingMedia@IsLive` (default `false`).
    pub is_live: bool,
    /// `SmoothStreamingMedia@LookAheadFragmentCount` (live only).
    pub look_ahead_fragment_count: Option<u32>,
    /// `SmoothStreamingMedia@DVRWindowLength`, in `timescale` ticks (live
    /// only; absent/0 means an unbounded DVR window).
    pub dvr_window_length: Option<u64>,
    /// The document's `StreamIndex` elements, in document order.
    pub streams: Vec<StreamIndex>,
}

/// A `StreamIndex` element (§2.2.2.3).
#[derive(Debug, Clone, PartialEq)]
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
    pub timescale: Option<u64>,
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
pub struct QualityLevel {
    /// `QualityLevel@Index`.
    pub index: u32,
    /// `QualityLevel@Bitrate`, in bits/second.
    pub bitrate: u64,
    /// `QualityLevel@FourCC` (e.g. [`crate::smooth::FOURCC_H264`] /
    /// [`crate::smooth::FOURCC_AACL`]).
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

/// One `c` (`StreamFragmentElement`) entry (§2.2.2.6).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct C {
    /// `@t` — this entry's explicit absolute start time, in the enclosing
    /// stream's `TimeScale` ticks. Only the first `<c>` typically carries it
    /// explicitly; later entries derive it by accumulating `d` (see
    /// [`StreamIndex::enumerate_chunks`]).
    pub t: Option<u64>,
    /// `@d` — this entry's fragment duration, in `TimeScale` ticks.
    pub d: Option<u64>,
    /// `@r` — repeat count: this entry represents `r + 1` fragments of
    /// duration `d` (default: one fragment, no repeat). See
    /// [`StreamIndex::enumerate_chunks`]'s bounded expansion.
    pub r: Option<u32>,
}

// ---------------------------------------------------------------------------
// Bounded helpers
// ---------------------------------------------------------------------------

/// `{bitrate}` token in a `StreamIndex@Url` fragment-URL template (§2.2.4.1).
pub const TOKEN_BITRATE: &str = "{bitrate}";
/// `{start time}` token in a `StreamIndex@Url` fragment-URL template (§2.2.4.1).
pub const TOKEN_START_TIME: &str = "{start time}";

impl StreamIndex {
    /// Expand every `c` entry's `@r` repeat run into `(t, d)` pairs in
    /// presentation order (§2.2.2.6): `t` accumulates (explicit on an entry
    /// resets it, otherwise it continues from the previous entry's `t + d`),
    /// each entry contributing `r + 1` occurrences of its own `d`.
    ///
    /// Returns [`SmoothParseError::ChunkRunTooLong`] if the total chunk count
    /// (summed across every `c`, counting each `r + 1` repetition) would
    /// exceed [`MAX_CHUNK_RUN`], defending against remote alloc-DoS attacks
    /// via a hostile Manifest with an unbounded `<c r="...">`.
    pub fn enumerate_chunks(&self) -> Result<Vec<(u64, u64)>> {
        let mut out = Vec::new();
        let mut t: u64 = 0;
        let mut total: u64 = 0;
        for c in &self.chunks_list {
            if let Some(explicit_t) = c.t {
                t = explicit_t;
            }
            let d = c.d.unwrap_or(0);
            let repeats: u64 = c.r.map(|r| u64::from(r).saturating_add(1)).unwrap_or(1);
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

/// Hex-decode a `CodecPrivateData` attribute value into bytes (the inverse of
/// [`crate::smooth`]'s internal `hex_upper` writer helper), bounded by
/// [`MAX_CODEC_PRIVATE_DATA_HEX_LEN`] (checked *before* any decode
/// allocation, defending against a hostile Manifest's unbounded
/// `CodecPrivateData`) and erroring on odd length or a non-hex-digit byte
/// rather than panicking.
pub fn hex_decode(s: &str) -> Result<Vec<u8>> {
    if s.len() > MAX_CODEC_PRIVATE_DATA_HEX_LEN {
        return Err(SmoothParseError::CodecPrivateDataTooLong { len: s.len() });
    }
    if s.is_empty() {
        return Ok(Vec::new());
    }
    let bytes = s.as_bytes();
    if bytes.len() % 2 != 0 {
        return Err(SmoothParseError::InvalidHex {
            value: s.to_string(),
        });
    }
    let mut out = Vec::with_capacity(bytes.len() / 2);
    let mut i = 0usize;
    while i < bytes.len() {
        let hi = hex_nibble(bytes[i]).ok_or_else(|| SmoothParseError::InvalidHex {
            value: s.to_string(),
        })?;
        let lo = hex_nibble(bytes[i + 1]).ok_or_else(|| SmoothParseError::InvalidHex {
            value: s.to_string(),
        })?;
        out.push((hi << 4) | lo);
        i += 2;
    }
    Ok(out)
}

fn hex_nibble(b: u8) -> Option<u8> {
    match b {
        b'0'..=b'9' => Some(b - b'0'),
        b'A'..=b'F' => Some(b - b'A' + 10),
        b'a'..=b'f' => Some(b - b'a' + 10),
        _ => None,
    }
}

// ---------------------------------------------------------------------------
// Attribute helpers (XML parsing is in the xml_parse module)
// ---------------------------------------------------------------------------

fn attr<'a>(attrs: &'a [(String, String)], key: &str) -> Option<&'a str> {
    attrs
        .iter()
        .find(|(k, _)| k == key)
        .map(|(_, v)| v.as_str())
}

fn attr_owned(attrs: &[(String, String)], key: &str) -> Option<String> {
    attr(attrs, key).map(String::from)
}

fn required_attr_owned(
    attrs: &[(String, String)],
    key: &'static str,
    element: &'static str,
) -> Result<String> {
    attr(attrs, key)
        .map(String::from)
        .ok_or(SmoothParseError::MissingAttribute { element, attr: key })
}

fn parse_attr<T: FromStr>(
    attrs: &[(String, String)],
    key: &'static str,
    element: &'static str,
) -> Result<Option<T>> {
    match attr(attrs, key) {
        Some(v) => {
            v.trim()
                .parse::<T>()
                .map(Some)
                .map_err(|_| SmoothParseError::InvalidAttributeValue {
                    element,
                    attr: key,
                    value: v.to_string(),
                })
        }
        None => Ok(None),
    }
}

fn required_attr_parse<T: FromStr>(
    attrs: &[(String, String)],
    key: &'static str,
    element: &'static str,
) -> Result<T> {
    let v = attr(attrs, key).ok_or(SmoothParseError::MissingAttribute { element, attr: key })?;
    v.trim()
        .parse::<T>()
        .map_err(|_| SmoothParseError::InvalidAttributeValue {
            element,
            attr: key,
            value: v.to_string(),
        })
}

// ---------------------------------------------------------------------------
// Parsing
// ---------------------------------------------------------------------------

impl SmoothManifest {
    /// Parse a client Manifest document (§2.2.2) into this structural model —
    /// the inverse of [`crate::smooth::SmoothPackager`]'s rendered XML
    /// output.
    pub fn parse(xml: &str) -> Result<SmoothManifest> {
        const EL: &str = "SmoothStreamingMedia";
        let mut tok = XmlTokenizer::new(xml);

        let (attrs, self_closing) = match tok.next_event()? {
            Some(XmlEvent::Start {
                name: "SmoothStreamingMedia",
                attrs,
                self_closing,
            }) => (attrs, self_closing),
            Some(XmlEvent::Start { name, .. }) => {
                return Err(SmoothParseError::UnexpectedElement {
                    expected: EL,
                    found: name.to_string(),
                });
            }
            Some(XmlEvent::End { .. }) => {
                return Err(SmoothParseError::UnexpectedElement {
                    expected: EL,
                    found: String::new(),
                });
            }
            None => return Err(SmoothParseError::UnexpectedEof),
        };

        let major_version: u32 = parse_attr(&attrs, "MajorVersion", EL)?.unwrap_or(2);
        let minor_version: u32 = parse_attr(&attrs, "MinorVersion", EL)?.unwrap_or(0);
        let timescale: u64 = parse_attr(&attrs, "TimeScale", EL)?.unwrap_or(SMOOTH_TIMESCALE);
        let duration: Option<u64> = parse_attr(&attrs, "Duration", EL)?;
        let is_live = attr(&attrs, "IsLive").is_some_and(|v| v.eq_ignore_ascii_case("true"));
        let look_ahead_fragment_count: Option<u32> =
            parse_attr(&attrs, "LookAheadFragmentCount", EL)?;
        let dvr_window_length: Option<u64> = parse_attr(&attrs, "DVRWindowLength", EL)?;

        let mut streams = Vec::new();
        if !self_closing {
            loop {
                match tok.next_event()? {
                    Some(XmlEvent::Start {
                        name: "StreamIndex",
                        attrs,
                        self_closing,
                    }) => streams.push(parse_stream_index(&mut tok, attrs, self_closing)?),
                    Some(XmlEvent::Start { self_closing, .. }) => {
                        if !self_closing {
                            skip_element(&mut tok)?;
                        }
                    }
                    Some(XmlEvent::End { name }) => {
                        if name != EL {
                            return Err(SmoothParseError::MismatchedEndTag {
                                expected: EL,
                                found: name.to_string(),
                            });
                        }
                        break;
                    }
                    None => return Err(SmoothParseError::UnexpectedEof),
                }
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
            streams,
        })
    }
}

fn parse_stream_index(
    tok: &mut XmlTokenizer<'_>,
    attrs: Vec<(String, String)>,
    self_closing: bool,
) -> Result<StreamIndex> {
    const EL: &str = "StreamIndex";
    let type_str = required_attr_owned(&attrs, "Type", EL)?;
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
    let name = attr_owned(&attrs, "Name");
    let subtype = attr_owned(&attrs, "Subtype");
    let chunks: Option<u32> = parse_attr(&attrs, "Chunks", EL)?;
    let timescale: Option<u64> = parse_attr(&attrs, "TimeScale", EL)?;
    let url = required_attr_owned(&attrs, "Url", EL)?;

    let mut qualities = Vec::new();
    let mut chunks_list = Vec::new();
    if !self_closing {
        loop {
            match tok.next_event()? {
                Some(XmlEvent::Start {
                    name: "QualityLevel",
                    attrs,
                    self_closing,
                }) => {
                    qualities.push(parse_quality_level(&attrs)?);
                    if !self_closing {
                        skip_element(tok)?;
                    }
                }
                Some(XmlEvent::Start {
                    name: "c",
                    attrs,
                    self_closing,
                }) => {
                    let t: Option<u64> = parse_attr(&attrs, "t", "c")?;
                    let d: Option<u64> = parse_attr(&attrs, "d", "c")?;
                    let r: Option<u32> = parse_attr(&attrs, "r", "c")?;
                    chunks_list.push(C { t, d, r });
                    if !self_closing {
                        skip_element(tok)?;
                    }
                }
                Some(XmlEvent::Start { self_closing, .. }) => {
                    if !self_closing {
                        skip_element(tok)?;
                    }
                }
                Some(XmlEvent::End { name }) => {
                    if name != EL {
                        return Err(SmoothParseError::MismatchedEndTag {
                            expected: EL,
                            found: name.to_string(),
                        });
                    }
                    break;
                }
                None => return Err(SmoothParseError::UnexpectedEof),
            }
        }
    }

    Ok(StreamIndex {
        stream_type,
        name,
        subtype,
        chunks,
        timescale,
        url,
        qualities,
        chunks_list,
    })
}

fn parse_quality_level(attrs: &[(String, String)]) -> Result<QualityLevel> {
    const EL: &str = "QualityLevel";
    let index: u32 = parse_attr(attrs, "Index", EL)?.unwrap_or(0);
    let bitrate: u64 = required_attr_parse(attrs, "Bitrate", EL)?;
    let four_cc = required_attr_owned(attrs, "FourCC", EL)?;
    let codec_private_data = match attr(attrs, "CodecPrivateData") {
        Some(hex) => hex_decode(hex)?,
        None => Vec::new(),
    };
    let width: Option<u32> = parse_attr(attrs, "MaxWidth", EL)?;
    let height: Option<u32> = parse_attr(attrs, "MaxHeight", EL)?;
    let sampling_rate: Option<u32> = parse_attr(attrs, "SamplingRate", EL)?;
    let channels: Option<u16> = parse_attr(attrs, "Channels", EL)?;
    let bits_per_sample: Option<u16> = parse_attr(attrs, "BitsPerSample", EL)?;
    let packet_size: Option<u32> = parse_attr(attrs, "PacketSize", EL)?;
    let audio_tag: Option<u32> = parse_attr(attrs, "AudioTag", EL)?;

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
// Codec glue — init-segment synthesis from CodecPrivateData
// ---------------------------------------------------------------------------

/// H.264 `nal_unit_type` for a sequence parameter set (SPS) —
/// ITU-T H.264 §7.4.1 Table 7-1.
const AVC_NAL_SPS: u8 = 7;
/// H.264 `nal_unit_type` for a picture parameter set (PPS).
const AVC_NAL_PPS: u8 = 8;

/// Build a [`TrackSpec`] (feed to [`crate::pipeline::build_init_segment`]) from
/// a `QualityLevel`'s `CodecPrivateData`, keyed by the enclosing
/// `StreamIndex`'s [`StreamType`] — the init-segment synthesis Smooth needs
/// in place of a bootstrapping init segment (see the module docs).
///
/// - [`StreamType::Video`]: splits the Annex-B `CodecPrivateData` into
///   SPS/PPS NAL units and builds an `avcC` via
///   [`crate::rtp_sdp::avc_config_from_sps_pps`]; geometry prefers the
///   SPS-decoded coded dimensions (authoritative), falling back to the
///   `QualityLevel`'s `MaxWidth`/`MaxHeight` if the SPS doesn't decode.
/// - [`StreamType::Audio`]: the `CodecPrivateData` bytes ARE the
///   `AudioSpecificConfig`, carried via
///   [`crate::rtp_sdp::aac_config_from_asc_bytes`].
/// - [`StreamType::Text`]: not carriable in this crate's ISOBMFF/fMP4 mux
///   path — returns [`crate::Error::UnsupportedCodec`].
pub fn track_spec_from_quality_level(
    track_id: u32,
    timescale: u32,
    stream_type: StreamType,
    quality: &QualityLevel,
) -> CrateResult<TrackSpec> {
    match stream_type {
        StreamType::Video => {
            let mut sps: Vec<AvcSps> = Vec::new();
            let mut pps: Vec<AvcPps> = Vec::new();
            for nal in iter_annexb_nals(&quality.codec_private_data) {
                match nal_unit_type(NalCodec::Avc, nal) {
                    Some(AVC_NAL_SPS) => sps.push(AvcSps(nal.to_vec())),
                    Some(AVC_NAL_PPS) => pps.push(AvcPps(nal.to_vec())),
                    _ => {}
                }
            }
            let config = avc_config_from_sps_pps(sps, pps)?;
            let (width, height) = config
                .config
                .sps
                .first()
                .and_then(|s| s.decode().ok())
                .map(|info| (info.width as u16, info.height as u16))
                .unwrap_or((
                    quality.width.unwrap_or(0) as u16,
                    quality.height.unwrap_or(0) as u16,
                ));
            Ok(TrackSpec::new(
                track_id,
                timescale,
                CodecConfig::Avc {
                    config,
                    width,
                    height,
                },
            ))
        }
        StreamType::Audio => {
            let config = aac_config_from_asc_bytes(quality.codec_private_data.clone())?;
            Ok(TrackSpec::new(track_id, timescale, config))
        }
        StreamType::Text => Err(CrateError::UnsupportedCodec {
            codec: "Smooth text (timed-text) stream",
        }),
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

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
        assert_eq!(m.timescale, 10_000_000);
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
            timescale: None,
            url: String::new(),
            qualities: Vec::new(),
            chunks_list: alloc::vec![
                C {
                    t: Some(0),
                    d: Some(1000),
                    r: Some(2),
                },
                C {
                    t: None,
                    d: Some(500),
                    r: None,
                },
            ],
        };
        let chunks = si.enumerate_chunks().expect("enumerate");
        assert_eq!(
            chunks,
            alloc::vec![(0, 1000), (1000, 1000), (2000, 1000), (3000, 500)]
        );
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
        assert!(hex_decode("ABC").is_err());
    }

    #[test]
    fn hex_decode_rejects_non_hex() {
        assert!(hex_decode("ZZ").is_err());
    }

    #[test]
    fn hex_decode_empty_is_empty() {
        assert_eq!(hex_decode("").unwrap(), Vec::<u8>::new());
    }

    // -- malformed / truncated input: must error, never panic ----------------

    #[test]
    fn unterminated_tag_is_error_not_panic() {
        let err = SmoothManifest::parse("<SmoothStreamingMedia MajorVersion=\"2\"").unwrap_err();
        assert!(matches!(
            err,
            SmoothParseError::UnterminatedTag { .. } | SmoothParseError::UnexpectedEof
        ));
    }

    #[test]
    fn wrong_root_element_is_error() {
        let err = SmoothManifest::parse("<NotSmooth/>").unwrap_err();
        assert!(matches!(err, SmoothParseError::UnexpectedElement { .. }));
    }

    #[test]
    fn empty_input_is_error() {
        assert_eq!(
            SmoothManifest::parse("").unwrap_err(),
            SmoothParseError::UnexpectedEof
        );
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

    // -- codec glue ------------------------------------------------------------

    #[test]
    fn track_spec_from_quality_level_video_avc() {
        // Real-shaped (if tiny) SPS/PPS: type 7 (SPS) then type 8 (PPS),
        // Annex-B start-code delimited, matching the writer's CodecPrivateData shape.
        let cpd = alloc::vec![
            0x00, 0x00, 0x00, 0x01, 0x67, 0x42, 0xC0, 0x1E, 0xAB, 0x00, 0x00, 0x00, 0x01, 0x68,
            0xCE, 0x3C, 0x80,
        ];
        let quality = QualityLevel {
            index: 0,
            bitrate: 500_000,
            four_cc: "H264".to_string(),
            codec_private_data: cpd,
            width: Some(640),
            height: Some(360),
            sampling_rate: None,
            channels: None,
            bits_per_sample: None,
            packet_size: None,
            audio_tag: None,
        };
        let spec = track_spec_from_quality_level(1, 90_000, StreamType::Video, &quality)
            .expect("build video TrackSpec");
        match spec.config {
            CodecConfig::Avc { config, .. } => {
                assert_eq!(config.config.sps.len(), 1);
                assert_eq!(config.config.pps.len(), 1);
                assert_eq!(
                    config.config.sps[0].0,
                    alloc::vec![0x67, 0x42, 0xC0, 0x1E, 0xAB]
                );
                assert_eq!(config.config.pps[0].0, alloc::vec![0x68, 0xCE, 0x3C, 0x80]);
            }
            _ => panic!("expected CodecConfig::Avc"),
        }
    }

    #[test]
    fn track_spec_from_quality_level_audio_aac() {
        let quality = QualityLevel {
            index: 0,
            bitrate: 128_000,
            four_cc: "AACL".to_string(),
            codec_private_data: alloc::vec![0x12, 0x10],
            width: None,
            height: None,
            sampling_rate: Some(44100),
            channels: Some(2),
            bits_per_sample: Some(16),
            packet_size: None,
            audio_tag: Some(255),
        };
        let spec = track_spec_from_quality_level(2, 44_100, StreamType::Audio, &quality)
            .expect("build audio TrackSpec");
        match spec.config {
            CodecConfig::Aac {
                sample_rate,
                channel_count,
                ..
            } => {
                assert_eq!(sample_rate, 44100);
                assert_eq!(channel_count, 2);
            }
            _ => panic!("expected CodecConfig::Aac"),
        }
    }

    #[test]
    fn track_spec_from_quality_level_text_unsupported() {
        let quality = QualityLevel {
            index: 0,
            bitrate: 1,
            four_cc: "TTML".to_string(),
            codec_private_data: Vec::new(),
            width: None,
            height: None,
            sampling_rate: None,
            channels: None,
            bits_per_sample: None,
            packet_size: None,
            audio_tag: None,
        };
        assert!(track_spec_from_quality_level(3, 1000, StreamType::Text, &quality).is_err());
    }

    #[test]
    fn track_spec_from_quality_level_video_no_sps_errors_not_panics() {
        let quality = QualityLevel {
            index: 0,
            bitrate: 1,
            four_cc: "H264".to_string(),
            codec_private_data: Vec::new(),
            width: None,
            height: None,
            sampling_rate: None,
            channels: None,
            bits_per_sample: None,
            packet_size: None,
            audio_tag: None,
        };
        assert!(track_spec_from_quality_level(1, 90_000, StreamType::Video, &quality).is_err());
    }
}
