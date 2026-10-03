//! Microsoft Smooth Streaming ([MS-SSTR]) **client manifest** parser +
//! `CodecPrivateData` codec glue — the structural inverse of
//! [`crate::smooth::SmoothPackager`]'s writer.
//!
//! Smooth-pull ingest (issue #759, T1) needs to fetch a remote client
//! Manifest and resolve the fragment URLs it describes, exactly as
//! [`crate::dash_parse`] does for DASH (issue #758). Like that parser, this
//! one reads XML with the shared [`quick_xml::Reader`]-based tokenizer
//! (crate-private, `std` feature) rather than writing a second one.
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
//! - `FourCC="AACL"`/`"AACH"`: the `CodecPrivateData` bytes ARE the
//!   `AudioSpecificConfig`, carried straight into `CodecConfig::Aac` via
//!   [`crate::rtp_sdp::aac_config_from_asc_bytes`]; when absent, an ASC is
//!   synthesised from `SamplingRate`/`Channels`.
//!
//! [`track_spec_from_quality_level`] dispatches on `QualityLevel@FourCC`
//! (case-insensitively) and rejects any other token (a Dolby `EC-3`/`AC-3`
//! audio level, an `H265` video level, a private one) with
//! `Error::UnsupportedCodec`: those carry their own CodecPrivateData shape, and
//! parsing Dolby bytes as an AudioSpecificConfig would silently yield a garbage
//! AAC track.

use alloc::string::{String, ToString};
use alloc::vec::Vec;
use core::fmt;
use core::str::FromStr;

use crate::aac_asc::{ChannelConfiguration, SamplingFrequencyIndex};
use crate::annexb::iter_annexb_nals;
use crate::error::{Error as CrateError, Result as CrateResult};
use crate::nal::{NalCodec, nal_unit_type};
use crate::nalu_types::{AvcPps, AvcSps};
use crate::pipeline::{CodecConfig, TrackSpec};
use crate::rtp_sdp::{aac_config_from_asc_bytes, avc_config_from_sps_pps};
use crate::smooth::{FOURCC_AACH, FOURCC_AACL, FOURCC_AVC1, FOURCC_H264, SMOOTH_TIMESCALE};
use crate::xml_chars::{char_data, is_xml_char};
use quick_xml::Reader;
use quick_xml::XmlVersion;
use quick_xml::errors::IllFormedError;
use quick_xml::events::{BytesStart, Event};
use quick_xml::name::QName;

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
        expected: String,
        /// The element name actually found in the closing tag.
        found: String,
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

// ---------------------------------------------------------------------------
// quick-xml pull plumbing
// ---------------------------------------------------------------------------

/// A `quick-xml` reader over the manifest text.
type XmlReader<'a> = Reader<&'a [u8]>;

/// What a parse loop sees: the start of an element (with its decoded
/// attributes) or the end of one. Text, comments, processing instructions, the
/// XML declaration and DOCTYPE are consumed by [`next_tag`] and never surface.
enum Tag {
    /// `<Name a="b">` or the self-closing `<Name a="b"/>`; `name` is the local
    /// name (namespace prefix stripped), attribute names keep any prefix.
    Open {
        name: String,
        attrs: Vec<(String, String)>,
        self_closing: bool,
    },
    /// `</Name>`.
    Close { name: String },
}

fn new_reader(xml: &str) -> XmlReader<'_> {
    Reader::from_str(xml)
}

/// Map a quick-xml error at the reader's current position.
fn xml_error(reader: &XmlReader<'_>, err: &quick_xml::Error) -> SmoothParseError {
    use quick_xml::Error as E;
    let pos = usize::try_from(reader.buffer_position()).unwrap_or(usize::MAX);
    match err {
        E::Syntax(_) => SmoothParseError::UnterminatedTag { pos },
        E::IllFormed(IllFormedError::MismatchedEndTag { expected, found }) => {
            SmoothParseError::MismatchedEndTag {
                expected: local_part(expected),
                found: local_part(found),
            }
        }
        _ => SmoothParseError::Xml {
            pos,
            message: err.to_string(),
        },
    }
}

/// A well-formedness error described by `message` at the reader's position.
fn xml_message(reader: &XmlReader<'_>, message: String) -> SmoothParseError {
    SmoothParseError::Xml {
        pos: usize::try_from(reader.buffer_position()).unwrap_or(usize::MAX),
        message,
    }
}

/// The local part of a (possibly prefixed) qualified name.
fn local_part(qname: &str) -> String {
    QName(qname).local_name().into_inner().to_string()
}

fn read_event<'a>(reader: &mut XmlReader<'a>) -> Result<Event<'a>> {
    match reader.read_event() {
        Ok(event) => Ok(event),
        Err(e) => Err(xml_error(reader, &e)),
    }
}

/// Decode a start tag's attributes (values unescaped and normalized by
/// quick-xml).
fn read_attrs(reader: &XmlReader<'_>, e: &BytesStart<'_>) -> Result<Vec<(String, String)>> {
    let pos = usize::try_from(reader.buffer_position()).unwrap_or(usize::MAX);
    let mut attrs = Vec::new();
    for attr in e.attributes() {
        let attr = attr.map_err(|_| SmoothParseError::MalformedAttribute { pos })?;
        let value = attr
            .normalized_value(XmlVersion::Implicit1_0)
            .map_err(|_| SmoothParseError::MalformedAttribute { pos })?;
        if !value.chars().all(is_xml_char) {
            return Err(SmoothParseError::MalformedAttribute { pos });
        }
        attrs.push((attr.key.as_ref().to_string(), value.into_owned()));
    }
    Ok(attrs)
}

/// The next element start/end, or `Ok(None)` at end of input.
fn next_tag(reader: &mut XmlReader<'_>) -> Result<Option<Tag>> {
    loop {
        let event = read_event(reader)?;
        // Character data between elements (and inside skipped subtrees) is
        // validated exactly like the modelled path.
        if char_data(&event)
            .map_err(|message| xml_message(reader, message))?
            .is_some()
        {
            continue;
        }
        match event {
            Event::Start(e) => {
                return Ok(Some(Tag::Open {
                    name: e.local_name().into_inner().to_string(),
                    attrs: read_attrs(reader, &e)?,
                    self_closing: false,
                }));
            }
            Event::Empty(e) => {
                return Ok(Some(Tag::Open {
                    name: e.local_name().into_inner().to_string(),
                    attrs: read_attrs(reader, &e)?,
                    self_closing: true,
                }));
            }
            Event::End(e) => {
                return Ok(Some(Tag::Close {
                    name: e.local_name().into_inner().to_string(),
                }));
            }
            Event::Eof => return Ok(None),
            _ => {}
        }
    }
}

/// Skip an already-open element's subtree, up to and including its matching
/// end tag.
fn skip_element(reader: &mut XmlReader<'_>) -> Result<()> {
    let mut depth: usize = 1;
    while depth > 0 {
        match next_tag(reader)? {
            Some(Tag::Open { self_closing, .. }) => {
                if !self_closing {
                    depth += 1;
                }
            }
            Some(Tag::Close { .. }) => depth -= 1,
            None => return Err(SmoothParseError::UnexpectedEof),
        }
    }
    Ok(())
}

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
    if !bytes.len().is_multiple_of(2) {
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
// Attribute helpers (XML parsing is in the `xml` module)
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
        let mut xml_reader = new_reader(xml);
        let reader = &mut xml_reader;

        let (attrs, self_closing) = match next_tag(reader)? {
            Some(Tag::Open {
                name,
                attrs,
                self_closing,
            }) if name == "SmoothStreamingMedia" => (attrs, self_closing),
            Some(Tag::Open { name, .. }) => {
                return Err(SmoothParseError::UnexpectedElement {
                    expected: EL,
                    found: name.to_string(),
                });
            }
            Some(Tag::Close { .. }) => {
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
                match next_tag(reader)? {
                    Some(Tag::Open {
                        name,
                        attrs,
                        self_closing,
                    }) if name == "StreamIndex" => {
                        streams.push(parse_stream_index(reader, attrs, self_closing)?)
                    }
                    Some(Tag::Open { self_closing, .. }) => {
                        if !self_closing {
                            skip_element(reader)?;
                        }
                    }
                    Some(Tag::Close { name }) => {
                        if name != EL {
                            return Err(SmoothParseError::MismatchedEndTag {
                                expected: EL.to_string(),
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
    reader: &mut XmlReader<'_>,
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
            match next_tag(reader)? {
                Some(Tag::Open {
                    name,
                    attrs,
                    self_closing,
                }) if name == "QualityLevel" => {
                    qualities.push(parse_quality_level(&attrs)?);
                    if !self_closing {
                        skip_element(reader)?;
                    }
                }
                Some(Tag::Open {
                    name,
                    attrs,
                    self_closing,
                }) if name == "c" => {
                    let t: Option<u64> = parse_attr(&attrs, "t", "c")?;
                    let d: Option<u64> = parse_attr(&attrs, "d", "c")?;
                    let r: Option<u32> = parse_attr(&attrs, "r", "c")?;
                    chunks_list.push(C { t, d, r });
                    if !self_closing {
                        skip_element(reader)?;
                    }
                }
                Some(Tag::Open { self_closing, .. }) => {
                    if !self_closing {
                        skip_element(reader)?;
                    }
                }
                Some(Tag::Close { name }) => {
                    if name != EL {
                        return Err(SmoothParseError::MismatchedEndTag {
                            expected: EL.to_string(),
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
/// - [`StreamType::Video`]: `FourCC` `H264` (a.k.a. `AVC1`, [MS-SSTR]
///   §2.2.2.5) — splits the Annex-B `CodecPrivateData` into SPS/PPS NAL units
///   and builds an `avcC` via [`crate::rtp_sdp::avc_config_from_sps_pps`];
///   geometry prefers the SPS-decoded coded dimensions (authoritative),
///   falling back to the `QualityLevel`'s `MaxWidth`/`MaxHeight` if the SPS
///   doesn't decode.
/// - [`StreamType::Audio`]: `FourCC` `AACL` or `AACH` — the `CodecPrivateData`
///   bytes ARE the `AudioSpecificConfig`, carried via
///   [`crate::rtp_sdp::aac_config_from_asc_bytes`]. When it is absent (allowed
///   by [MS-SSTR]), the ASC is synthesised from the `QualityLevel`'s
///   `SamplingRate`/`Channels`.
/// - [`StreamType::Text`]: not carriable in this crate's ISOBMFF/fMP4 mux
///   path — returns [`crate::Error::UnsupportedCodec`].
///
/// `FourCC` matching is case-insensitive (real manifests use `AACL` and
/// `aacl`); any unrecognised token is [`crate::Error::UnsupportedCodec`].
pub fn track_spec_from_quality_level(
    track_id: u32,
    timescale: u32,
    stream_type: StreamType,
    quality: &QualityLevel,
) -> CrateResult<TrackSpec> {
    match stream_type {
        StreamType::Video => {
            // [MS-SSTR] §2.2.2.5 names the video FourCC `H264`, a.k.a. `AVC1`.
            if !is_fourcc(&quality.four_cc, FOURCC_H264)
                && !is_fourcc(&quality.four_cc, FOURCC_AVC1)
            {
                return Err(CrateError::UnsupportedCodec {
                    codec: "Smooth video FourCC (only H264/AVC1 is synthesised)",
                });
            }
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
                .map(|info| {
                    Ok::<_, CrateError>((
                        u16::try_from(info.width).map_err(|_| CrateError::InvalidValue {
                            field: "SPS sps_pic_width_max_in_luma_samples",
                            value: u64::from(info.width),
                            reason: "does not fit the IR's 16-bit dimension field",
                        })?,
                        u16::try_from(info.height).map_err(|_| CrateError::InvalidValue {
                            field: "SPS sps_pic_height_max_in_luma_samples",
                            value: u64::from(info.height),
                            reason: "does not fit the IR's 16-bit dimension field",
                        })?,
                    ))
                })
                .transpose()?
                .unwrap_or((
                    checked_u16(quality.width, "QualityLevel@MaxWidth")?,
                    checked_u16(quality.height, "QualityLevel@MaxHeight")?,
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
            // [MS-SSTR] §2.2.2.5 names the audio FourCC `AACL` (AAC-LC) and
            // `AACH` (HE-AAC). A Dolby FourCC (`EC-3`/`AC-3`) instead carries
            // Dolby `CodecPrivateData`, which "parses" as an ASC (any ≥2-byte
            // buffer does) and would become a garbage `CodecConfig::Aac` track
            // with a nonsense rate/channel count — reject it.
            // HE-AAC (`AACH`) carries an AAC-LC core too, so both FourCCs
            // synthesise the same core ASC (see `synthesise_asc`).
            if !is_fourcc(&quality.four_cc, FOURCC_AACL)
                && !is_fourcc(&quality.four_cc, FOURCC_AACH)
            {
                return Err(CrateError::UnsupportedCodec {
                    codec: "Smooth audio FourCC (only AACL/AACH is synthesised)",
                });
            }
            // No `CodecPrivateData`: [MS-SSTR] allows the config to be derived
            // from the quality level's own attributes, so build the ASC.
            let asc = if quality.codec_private_data.is_empty() {
                synthesise_asc(AOT_AAC_LC, quality)?
            } else {
                quality.codec_private_data.clone()
            };
            let config = aac_config_from_asc_bytes(asc)?;
            Ok(TrackSpec::new(track_id, timescale, config))
        }
        StreamType::Text => Err(CrateError::UnsupportedCodec {
            codec: "Smooth text (timed-text) stream",
        }),
    }
}

/// Case-insensitive `FourCC` comparison ([MS-SSTR] `QualityLevel@FourCC`).
fn is_fourcc(value: &str, expected: &str) -> bool {
    value.eq_ignore_ascii_case(expected)
}

/// Convert an optional `QualityLevel` dimension to the IR's `u16`, rejecting an
/// over-range value rather than truncating it (the `#997` class).
fn checked_u16(value: Option<u32>, field: &'static str) -> CrateResult<u16> {
    match value {
        Some(v) => u16::try_from(v).map_err(|_| CrateError::InvalidValue {
            field,
            value: u64::from(v),
            reason: "does not fit the IR's 16-bit dimension field",
        }),
        None => Ok(0),
    }
}

/// Synthesise an `AudioSpecificConfig` from a `QualityLevel`'s
/// `SamplingRate`/`Channels` attributes (no `CodecPrivateData`).
///
/// ISO/IEC 14496-3 §1.6.2.1 `AudioSpecificConfig()` is
/// `audioObjectType(5)` | `samplingFrequencyIndex(4)` | `channelConfiguration(4)`,
/// each packed MSB-first into the leading bytes:
/// - `audioObjectType`: 2 (AAC-LC) — the **core** object type.
/// - `samplingFrequencyIndex`: the shared ISO/IEC 14496-3 Table 1.10 index for
///   `SamplingRate`. A rate that is not a Table 1.10 entry is
///   [`crate::Error::UnsupportedCodec`]: this synthesiser writes only the
///   two-byte core form, so it cannot express the `samplingFrequencyIndex == 0xF`
///   explicit-24-bit-rate escape (that would need the rate appended after the
///   index), and writing the escape index alone would describe a config with no
///   sample rate.
/// - `channelConfiguration`: `Channels` mapped through Table 1.19 (1, 2, 3, 4,
///   5, 6, 8); a count with no defined mapping is
///   [`crate::Error::UnsupportedCodec`].
///
/// **HE-AAC (`AACH`)**: only the core AAC-LC config is written. The hierarchical
/// explicit-signalling form of §1.6.2.1 (`extensionAudioObjectType = 5` +
/// `extensionSamplingFrequencyIndex` + the core AOT) is not synthesised, so the
/// SBR extension stays **implicit** — a decoder derives it from the
/// backward-compatible sync extension in the stream itself. That is a legal
/// carriage (§1.6.2.1 allows implicit signalling) and honest about what these
/// two bytes say; it does mean the `esds` reports the core rate, not twice it.
fn synthesise_asc(aot: u8, quality: &QualityLevel) -> CrateResult<Vec<u8>> {
    let rate = quality.sampling_rate.ok_or(CrateError::UnsupportedCodec {
        codec: "Smooth AAC QualityLevel with no SamplingRate and no CodecPrivateData",
    })?;
    let channels = quality.channels.ok_or(CrateError::UnsupportedCodec {
        codec: "Smooth AAC QualityLevel with no Channels and no CodecPrivateData",
    })?;
    let sf_index = frequency_index_for(rate).ok_or(CrateError::UnsupportedCodec {
        codec: "Smooth AAC SamplingRate is not an ISO/IEC 14496-3 Table 1.10 entry",
    })?;
    let channel_raw = channel_configuration_for(channels)?;

    // audioObjectType(5) | samplingFrequencyIndex(4) | channelConfiguration(4)
    // → 13 bits → 2 bytes, the remaining 3 bits zero. Only the core AAC-LC
    // object type is written (see the doc above).
    let mut out = alloc::vec![0u8; 2];
    out[0] = (aot << 3) | (sf_index >> 1);
    out[1] = ((sf_index & 0x01) << 7) | (channel_raw << 3);
    Ok(out)
}

/// The `samplingFrequencyIndex` for a rate, from the crate's single copy of
/// ISO/IEC 14496-3 Table 1.10 (`SamplingFrequencyIndex::raw`).
///
/// A rate absent from Table 1.10 is `None`: this synthesiser writes only the
/// two-byte core ASC (`audioObjectType`/`samplingFrequencyIndex`/
/// `channelConfiguration`), so it cannot express the 24-bit explicit-rate
/// escape (`samplingFrequencyIndex == 0xF`) without also appending that value.
/// Returning `None` and rejecting is honest; silently writing the escape index
/// with no rate would describe a config with no sample rate at all.
fn frequency_index_for(rate: u32) -> Option<u8> {
    SAMPLING_FREQUENCY_INDICES
        .iter()
        .find(|c| c.table_hz() == Some(rate))
        .map(|c| c.raw())
}
/// The `audioObjectType` for AAC-LC (ISO/IEC 14496-3 Table 1.18).
const AOT_AAC_LC: u8 = 2;

/// Every non-reserved `SamplingFrequencyIndex`, in Table 1.10 order.
const SAMPLING_FREQUENCY_INDICES: [SamplingFrequencyIndex; 12] = [
    SamplingFrequencyIndex::Fs96000,
    SamplingFrequencyIndex::Fs88200,
    SamplingFrequencyIndex::Fs64000,
    SamplingFrequencyIndex::Fs48000,
    SamplingFrequencyIndex::Fs44100,
    SamplingFrequencyIndex::Fs32000,
    SamplingFrequencyIndex::Fs24000,
    SamplingFrequencyIndex::Fs22050,
    SamplingFrequencyIndex::Fs16000,
    SamplingFrequencyIndex::Fs12000,
    SamplingFrequencyIndex::Fs8000,
    SamplingFrequencyIndex::Fs7350,
];

/// Map a channel count to `channelConfiguration` (ISO/IEC 14496-3 Table 1.19):
/// 1→mono, 2→stereo, 3→`Ch3`, 4→`Ch4`, 5→`Ch5`, 6→`Ch5_1` (5.1), 8→`Ch7_1`
/// (7.1: 3 front + 2 side + 2 back + LFE). Table 1.19 has no 7-channel
/// configuration — `Ch7_1` is eight — so 7 is rejected, not mis-mapped. Any
/// other count is [`crate::Error::UnsupportedCodec`] rather than a wrong
/// mapping.
fn channel_configuration_for(channels: u16) -> CrateResult<u8> {
    match channels {
        1 => Ok(ChannelConfiguration::Mono.raw()),
        2 => Ok(ChannelConfiguration::Stereo.raw()),
        3 => Ok(ChannelConfiguration::Ch3.raw()),
        4 => Ok(ChannelConfiguration::Ch4.raw()),
        5 => Ok(ChannelConfiguration::Ch5.raw()),
        6 => Ok(ChannelConfiguration::Ch5_1.raw()),
        8 => Ok(ChannelConfiguration::Ch7_1.raw()),
        _ => Err(CrateError::UnsupportedCodec {
            codec: "Smooth AAC channel count with no Table 1.19 configuration",
        }),
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    /// Character data inside a skipped subtree (e.g. `<Protection>`) or between
    /// elements is validated like modelled text.
    #[test]
    fn invalid_character_data_in_skipped_content_is_rejected() {
        for bad in ["&nope;", "&#1;", "&#x1;", "\u{1}", "<![CDATA[\u{1}]]>"] {
            let skipped = format!(
                "<SmoothStreamingMedia><Protection><ProtectionHeader>{bad}</ProtectionHeader></Protection></SmoothStreamingMedia>"
            );
            assert!(SmoothManifest::parse(&skipped).is_err(), "skipped {bad:?}");
            let between = format!(
                "<SmoothStreamingMedia>{bad}<StreamIndex Type=\"video\"/></SmoothStreamingMedia>"
            );
            assert!(SmoothManifest::parse(&between).is_err(), "between {bad:?}");
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
    fn track_spec_from_quality_level_audio_dolby_fourcc_rejected() {
        // `FourCC="EC-3"` carries Dolby (E-)AC-3 CodecPrivateData, not an
        // AudioSpecificConfig — this must be rejected, not silently parsed as
        // an AAC track. The gate is the FourCC itself (the bytes are
        // deliberately a plausible ASC, so only the FourCC check stops it).
        let quality = QualityLevel {
            index: 0,
            bitrate: 128_000,
            four_cc: "EC-3".to_string(),
            codec_private_data: alloc::vec![0x10, 0x3D],
            width: None,
            height: None,
            sampling_rate: Some(48_000),
            channels: Some(6),
            bits_per_sample: Some(16),
            packet_size: None,
            audio_tag: None,
        };
        match track_spec_from_quality_level(2, 48_000, StreamType::Audio, &quality) {
            Err(CrateError::UnsupportedCodec { codec }) => {
                assert_eq!(codec, "Smooth audio FourCC (only AACL/AACH is synthesised)");
            }
            other => panic!("expected UnsupportedCodec for EC-3, got {other:?}"),
        }
        let quality = QualityLevel {
            four_cc: "AC-3".to_string(),
            ..quality
        };
        assert!(matches!(
            track_spec_from_quality_level(2, 48_000, StreamType::Audio, &quality),
            Err(CrateError::UnsupportedCodec { .. })
        ));
    }

    #[test]
    fn track_spec_from_quality_level_video_non_h264_fourcc_rejected() {
        // The video path builds an `avcC` from Annex-B SPS/PPS; a non-H264
        // FourCC (HEVC `H265`) must not take that path.
        let quality = QualityLevel {
            index: 0,
            bitrate: 1_000_000,
            four_cc: "H265".to_string(),
            codec_private_data: alloc::vec![0x00, 0x00, 0x00, 0x01, 0x67, 0x42, 0xC0, 0x1E, 0xAB],
            width: Some(640),
            height: Some(360),
            sampling_rate: None,
            channels: None,
            bits_per_sample: None,
            packet_size: None,
            audio_tag: None,
        };
        match track_spec_from_quality_level(1, 90_000, StreamType::Video, &quality) {
            Err(CrateError::UnsupportedCodec { codec }) => {
                assert_eq!(codec, "Smooth video FourCC (only H264/AVC1 is synthesised)");
            }
            other => panic!("expected UnsupportedCodec for H265, got {other:?}"),
        }
    }

    /// `FourCC` matching is case-insensitive ([MS-SSTR] §2.2.2.5 names `H264`
    /// a.k.a. `AVC1`, and manifest authors are inconsistent about case).
    #[test]
    fn four_cc_matching_is_case_insensitive() {
        // Video `H264` / `AVC1` / lowercase.
        for cc in ["H264", "h264", "AVC1", "avc1"] {
            let quality = QualityLevel {
                index: 0,
                bitrate: 500_000,
                four_cc: cc.to_string(),
                codec_private_data: alloc::vec![
                    0x00, 0x00, 0x00, 0x01, 0x67, 0x42, 0xC0, 0x1E, 0xAB, 0x00, 0x00, 0x00, 0x01,
                    0x68, 0xCE, 0x3C, 0x80,
                ],
                width: Some(640),
                height: Some(360),
                sampling_rate: None,
                channels: None,
                bits_per_sample: None,
                packet_size: None,
                audio_tag: None,
            };
            assert!(
                track_spec_from_quality_level(1, 90_000, StreamType::Video, &quality).is_ok(),
                "video FourCC {cc} must be accepted"
            );
        }
        // Audio `AACL` / `AACH` / lowercase.
        for cc in ["AACL", "aacl", "AACH", "aach"] {
            let quality = QualityLevel {
                index: 0,
                bitrate: 128_000,
                four_cc: cc.to_string(),
                codec_private_data: alloc::vec![0x12, 0x10],
                width: None,
                height: None,
                sampling_rate: Some(44_100),
                channels: Some(2),
                bits_per_sample: Some(16),
                packet_size: None,
                audio_tag: Some(255),
            };
            assert!(
                track_spec_from_quality_level(2, 44_100, StreamType::Audio, &quality).is_ok(),
                "audio FourCC {cc} must be accepted"
            );
        }
    }

    /// An `AACL` `QualityLevel` with no `CodecPrivateData` synthesises an ASC
    /// from `SamplingRate`/`Channels` (ISO/IEC 14496-3 §1.6.2.1). For AAC-LC
    /// 44 100 Hz stereo the ASC is the literal `12 10`
    /// (`audioObjectType` 2, `samplingFrequencyIndex` 4, `channelConfiguration`
    /// 2), cross-checked against the crate's own ASC parser.
    #[test]
    fn empty_codec_private_data_synthesises_aac_lc_asc() {
        let quality = QualityLevel {
            index: 0,
            bitrate: 128_000,
            four_cc: "AACL".to_string(),
            codec_private_data: Vec::new(),
            width: None,
            height: None,
            sampling_rate: Some(44_100),
            channels: Some(2),
            bits_per_sample: Some(16),
            packet_size: None,
            audio_tag: Some(255),
        };
        // The synthesiser's literals.
        assert_eq!(
            synthesise_asc(AOT_AAC_LC, &quality).unwrap(),
            alloc::vec![0x12, 0x10]
        );
        // And the crate's own parser agrees about what those bytes mean.
        let asc =
            <crate::aac_asc::AudioSpecificConfig as broadcast_common::Parse>::parse(&[0x12, 0x10])
                .unwrap();
        assert_eq!(asc.sampling_frequency_index.raw(), 4);
        assert_eq!(asc.channel_configuration.raw(), 2);

        let spec = track_spec_from_quality_level(2, 44_100, StreamType::Audio, &quality)
            .expect("empty CodecPrivateData must synthesise an ASC");
        let CodecConfig::Aac {
            sample_rate,
            channel_count,
            esds,
            ..
        } = spec.config
        else {
            panic!("expected CodecConfig::Aac");
        };
        assert_eq!((sample_rate, channel_count), (44_100, 2));
        let dsi = esds
            .es_descriptor
            .decoder_config
            .as_ref()
            .and_then(|d| d.decoder_specific_info.as_ref())
            .expect("esds carries a DecoderSpecificInfo");
        assert_eq!(
            dsi.data.as_ref(),
            [0x12u8, 0x10],
            "the esds DecoderSpecificInfo must be the literal AAC-LC ASC 12 10"
        );
    }

    /// `AACH` (HE-AAC) with no `CodecPrivateData` synthesises the plain AAC-LC
    /// **core** ASC — the SBR extension is implicit, derived from the stream's
    /// backward-compatible signalling rather than the two config bytes — so the
    /// bytes equal the `AACL` case and `heaac_signaling()` reports no explicit
    /// SBR.
    #[test]
    fn empty_codec_private_data_synthesises_core_asc_for_aach() {
        let quality = QualityLevel {
            index: 0,
            bitrate: 64_000,
            four_cc: "AACH".to_string(),
            codec_private_data: Vec::new(),
            width: None,
            height: None,
            sampling_rate: Some(44_100),
            channels: Some(2),
            bits_per_sample: Some(16),
            packet_size: None,
            audio_tag: Some(255),
        };
        // Through the real entry point, so the AOT the caller picks is what
        // actually lands in the esds.
        let spec = track_spec_from_quality_level(3, 44_100, StreamType::Audio, &quality)
            .expect("AACH must synthesise a core ASC");
        let CodecConfig::Aac {
            sample_rate,
            channel_count,
            esds,
            ..
        } = spec.config
        else {
            panic!("expected CodecConfig::Aac");
        };
        assert_eq!((sample_rate, channel_count), (44_100, 2));
        let dsi = esds
            .es_descriptor
            .decoder_config
            .as_ref()
            .and_then(|d| d.decoder_specific_info.as_ref())
            .expect("esds carries a DecoderSpecificInfo");
        assert_eq!(
            dsi.data.as_ref(),
            [0x12u8, 0x10],
            "the esds DecoderSpecificInfo must be the core AAC-LC ASC"
        );
        let asc =
            <crate::aac_asc::AudioSpecificConfig as broadcast_common::Parse>::parse(&dsi.data)
                .unwrap();
        assert_eq!(asc.audio_object_type.raw(), AOT_AAC_LC);
        let he = asc.heaac_signaling();
        assert!(!he.sbr_present, "SBR is implicit, not explicitly signalled");
        assert!(!he.ps_present);
    }

    /// Every Table 1.19 channel count synthesises its literal ASC bytes (48 kHz
    /// AAC-LC: `audioObjectType` 2, `samplingFrequencyIndex` 3): 1 → `11 88`,
    /// 2 → `11 90`, 6 → `11 B0`, 8 → `11 B8`. `Ch7_1` is **eight** channels, so
    /// a 7-channel level is rejected, not mapped onto it.
    #[test]
    fn channel_configurations_synthesise_literal_asc_bytes() {
        let base = QualityLevel {
            index: 0,
            bitrate: 128_000,
            four_cc: "AACL".to_string(),
            codec_private_data: Vec::new(),
            width: None,
            height: None,
            sampling_rate: Some(48_000),
            channels: Some(2),
            bits_per_sample: Some(16),
            packet_size: None,
            audio_tag: Some(255),
        };
        for (channels, expected) in [
            (1u16, [0x11u8, 0x88]),
            (2, [0x11, 0x90]),
            (6, [0x11, 0xB0]),
            (8, [0x11, 0xB8]),
        ] {
            let q = QualityLevel {
                channels: Some(channels),
                ..base.clone()
            };
            assert_eq!(
                synthesise_asc(AOT_AAC_LC, &q).unwrap(),
                expected,
                "channels={channels}"
            );
            // The crate's own parser agrees about the channel count.
            let asc =
                <crate::aac_asc::AudioSpecificConfig as broadcast_common::Parse>::parse(&expected)
                    .unwrap();
            assert_eq!(asc.channel_configuration.channel_count(), Some(channels));
        }
        // 7 has no Table 1.19 configuration (Ch7_1 is 8).
        let seven = QualityLevel {
            channels: Some(7),
            ..base
        };
        assert!(matches!(
            synthesise_asc(AOT_AAC_LC, &seven),
            Err(CrateError::UnsupportedCodec { .. })
        ));
    }

    /// A rate that is not an ISO/IEC 14496-3 Table 1.10 entry (e.g. 44 000 Hz)
    /// and a channel count with no Table 1.19 mapping are `UnsupportedCodec`,
    /// never a wrong rate/count.
    #[test]
    fn unsupported_rate_or_channels_are_rejected_not_guessed() {
        let base = QualityLevel {
            index: 0,
            bitrate: 128_000,
            four_cc: "AACL".to_string(),
            codec_private_data: Vec::new(),
            width: None,
            height: None,
            sampling_rate: Some(44_100),
            channels: Some(2),
            bits_per_sample: Some(16),
            packet_size: None,
            audio_tag: Some(255),
        };
        let odd_rate = QualityLevel {
            sampling_rate: Some(44_000),
            ..base.clone()
        };
        assert!(matches!(
            synthesise_asc(AOT_AAC_LC, &odd_rate),
            Err(CrateError::UnsupportedCodec { .. })
        ));
        let odd_channels = QualityLevel {
            channels: Some(9),
            ..base.clone()
        };
        assert!(matches!(
            synthesise_asc(AOT_AAC_LC, &odd_channels),
            Err(CrateError::UnsupportedCodec { .. })
        ));
        // Missing attributes are also errors, not defaults.
        let no_rate = QualityLevel {
            sampling_rate: None,
            ..base.clone()
        };
        assert!(synthesise_asc(AOT_AAC_LC, &no_rate).is_err());
        let no_channels = QualityLevel {
            channels: None,
            ..base
        };
        assert!(synthesise_asc(AOT_AAC_LC, &no_channels).is_err());
    }

    /// The SPS-decoded geometry is authoritative, so an SPS whose coded width
    /// exceeds `u16::MAX` is `Error::InvalidValue` (not `InvalidInput`, and not
    /// a wrapped dimension) — the same class the `MaxWidth` fallback rejects.
    ///
    /// The SPS bytes are the one from `fixtures/flv/oversize-dims.flv`
    /// (`pic_width_in_mbs_minus1 = 4095` → width 65 536, height 48).
    #[test]
    fn oversized_sps_dimensions_are_invalid_value() {
        let quality = QualityLevel {
            index: 0,
            bitrate: 1,
            four_cc: "H264".to_string(),
            // Annex-B SPS (type 7) with the oversize dimensions.
            codec_private_data: alloc::vec![
                0x00, 0x00, 0x00, 0x01, 0x67, 0x42, 0x00, 0x1F, 0xF4, 0x00, 0x08, 0x00, 0x38, 0x80,
            ],
            width: None,
            height: None,
            sampling_rate: None,
            channels: None,
            bits_per_sample: None,
            packet_size: None,
            audio_tag: None,
        };
        // The SPS really does decode to 65 536 (so the error below is the
        // over-range check, not a decode failure falling back to `None`).
        let decoded = crate::sps::decode_avc_sps(&quality.codec_private_data[4..])
            .expect("the oversize SPS decodes");
        assert_eq!(decoded.width, 65_536);
        match track_spec_from_quality_level(1, 90_000, StreamType::Video, &quality) {
            Err(CrateError::InvalidValue {
                field: "SPS sps_pic_width_max_in_luma_samples",
                value: 65_536,
                ..
            }) => {}
            other => panic!("expected an SPS width overflow InvalidValue, got {other:?}"),
        }
    }

    /// A `QualityLevel` dimension above `u16::MAX` is rejected, not truncated
    /// (the `#997` class), and this holds when the SPS does not decode (the
    /// attribute fallback path).
    #[test]
    fn oversized_quality_level_dimensions_are_rejected() {
        let quality = QualityLevel {
            index: 0,
            bitrate: 1,
            four_cc: "H264".to_string(),
            // A present-but-undecodable SPS (type 7, garbage body): it passes
            // `avc_config_from_sps_pps`'s ≥4-byte check but `decode()` fails, so
            // the MaxWidth/MaxHeight fallback path runs.
            codec_private_data: alloc::vec![0x00, 0x00, 0x00, 0x01, 0x67, 0xFF, 0xFF, 0xFF, 0xFF,],
            width: Some(70_000),
            height: Some(360),
            sampling_rate: None,
            channels: None,
            bits_per_sample: None,
            packet_size: None,
            audio_tag: None,
        };
        match track_spec_from_quality_level(1, 90_000, StreamType::Video, &quality) {
            Err(CrateError::InvalidValue {
                field: "QualityLevel@MaxWidth",
                value: 70_000,
                ..
            }) => {}
            other => panic!("expected a MaxWidth overflow error, got {other:?}"),
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
