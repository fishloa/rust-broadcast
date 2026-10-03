//! DASH `.mpd` Media Presentation Description **parser** — ISO/IEC 23009-1 —
//! the structural inverse of [`crate::dash`]'s `DashPackager` writer.
//!
//! `transmux` demuxes remote DASH presentations (issue #758, DASH-pull
//! ingest) by first fetching an MPD and resolving the segment URLs it
//! describes; this module is the piece that reads that MPD text. Like the
//! writer, it reads XML with a [`quick_xml::Reader`] pull loop (so, like the
//! writer, it needs the `std` feature), scoped to exactly the MPD subset the
//! writer emits plus what real-world isoff-live MPDs use. Malformed XML (bad
//! nesting, undefined entity references, unterminated constructs) is a
//! structured [`DashParseError`], never a panic.
//!
//! # Structure parsed (ISO/IEC 23009-1:2014)
//!
//! - **MPD** (§5.3.1) — [`Mpd`]: `profiles`, `type` ([`MpdType`]),
//!   `mediaPresentationDuration`/`minimumUpdatePeriod`/
//!   `availabilityStartTime`/`timeShiftBufferDepth` (§5.3.1.2 Table 3), one or
//!   more `Period`.
//! - **Period** (§5.3.2) — [`Period`]: `id`, `start`, `duration`, an optional
//!   `BaseURL`, its `AdaptationSet`s.
//! - **`AdaptationSet`** (§5.3.3) — [`AdaptationSet`]: `mimeType`,
//!   `contentType`, an optional set-level `SegmentTemplate`, its
//!   `Representation`s.
//! - **`Representation`** (§5.3.5) — [`Representation`]: `id`, `bandwidth`,
//!   `codecs`, geometry/audio attributes, its own `SegmentTemplate` (merged
//!   with its parents' — see [`Mpd::parse`]'s inheritance note).
//!
//! # `SegmentTemplate` and `BaseURL` inheritance (§5.3.9.1, §5.3.9.2)
//!
//! Segment information is hierarchical across `Period` >
//! `AdaptationSet` > `Representation`, and a lower level overrides **only the
//! attributes it declares** — the rest are inherited. The standard's Annex G
//! examples lean on this heavily (G.13 gives each `Representation` a
//! `SegmentTemplate` carrying only `@initialization` and inherits
//! `@media`/`@timescale`/`@duration`/`@startNumber` from the
//! `AdaptationSet`), so whole-element replacement is not enough. Every
//! [`Representation::segment_template`] this parser returns is that resolved,
//! effective template. `@duration` and `SegmentTimeline` are exclusive
//! (§5.3.9.4.4), so a child that introduces a timeline does not also inherit
//! the parent's `@duration`.
//!
//! `BaseURL` (§5.3.9.2) is inherited over the same chain, and each level's
//! value is reported as [`Mpd::base_url`]/[`Period::base_url`]/
//! [`AdaptationSet::base_url`]/[`Representation::base_url`]. Several
//! `BaseURL` children of one element are *alternates* consulted in order
//! (§5.6.5), so the first non-empty one is kept. [`Mpd::resolve_segment_url`]
//! applies RFC 3986 §5
//! reference resolution down that chain (`url::Url::join`), via
//! [`crate::base_url`].
//! - **`SegmentTemplate`** (§5.3.9.4.4) — [`SegmentTemplate`]: `timescale`,
//!   `initialization`/`media` templates, `startNumber`,
//!   `presentationTimeOffset`, either a nominal `duration` (`$Number$`
//!   addressing) or a child **`SegmentTimeline`** (§5.3.9.6) —
//!   [`SegmentTimeline`] / [`S`] — of `<S t= d= r=>` runs (`$Time$`
//!   addressing).
//!
//! Elements outside this subset (`ProgramInformation`, `ServiceDescription`,
//! `Role`, `ContentProtection`, `SegmentList`, `SegmentBase`, …) are tolerated
//! — skipped as opaque subtrees — rather than rejected: a `Representation`
//! that only carries `SegmentList`/`SegmentBase` addressing simply ends up
//! with `segment_template: None` (unsupported in this v1, not a parse
//! failure). Malformed/truncated XML never panics; every failure path returns
//! [`DashParseError`].
//!
//! # Segment-URL resolution
//!
//! [`SegmentTemplate::resolve`] substitutes `$RepresentationID$`/`$Number$`/
//! `$Time$`/`$Bandwidth$` (with optional `%0Nd` width, and `$$` → `$`,
//! §5.3.9.4.4 Table 16) into an `initialization`/`media` template string.
//! [`SegmentTimeline::enumerate`] expands a timeline's `<S>` runs (repeating
//! each `r+1` times, accumulating `t`, §5.3.9.6) into the `(number, time)`
//! sequence a caller walks to build every media segment URL in order;
//! [`SegmentTemplate::number_sequence`] does the equivalent for `$Number$`
//! addressing with a constant nominal `@duration` (no timeline).

use alloc::string::{String, ToString};
use alloc::vec::Vec;
use core::fmt;
use core::str::FromStr;
use core::time::Duration;

use crate::xml_chars::{char_data, is_xml_char};
use quick_xml::Reader;
use quick_xml::XmlVersion;
use quick_xml::errors::IllFormedError;
use quick_xml::events::{BytesStart, Event};
use quick_xml::name::QName;

// ---------------------------------------------------------------------------
// Error
// ---------------------------------------------------------------------------

/// Errors returned while parsing an MPD (ISO/IEC 23009-1) document.
///
/// Distinct from [`crate::Error`] (like
/// [`FlvError`](crate::flv::FlvError)/[`RtmpError`](crate::rtmp::RtmpError)) —
/// this parser never panics on malformed or truncated input; every failure
/// path returns one of these variants instead.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum DashParseError {
    /// The input ended before a well-formed document was found (e.g. an
    /// unclosed root element, or no `MPD` element at all).
    UnexpectedEof,
    /// A `<...>` tag, `<!--...-->` comment, `<?...?>` declaration, or
    /// `<!...>` markup declaration was never closed.
    UnterminatedTag {
        /// Byte offset (into the input) where the unterminated construct began.
        pos: usize,
    },
    /// An attribute inside a start tag was not well-formed
    /// (`name="value"`/`name='value'`, ISO/IEC 23009-1 §5.3.1 following XML
    /// 1.0 §3.1).
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
    /// An attribute's value could not be parsed as the type it must carry.
    InvalidAttributeValue {
        /// The element's name.
        element: &'static str,
        /// The attribute's name.
        attr: &'static str,
        /// The raw (unparsable) value.
        value: String,
    },
    /// An `xs:duration` string (§5.3.1.2, W3C XML Schema Part 2 §3.2.6) could
    /// not be parsed by [`parse_iso8601_duration`].
    InvalidDuration {
        /// The raw (unparsable) value.
        value: String,
    },
    /// A `SegmentTimeline` would exceed the cap on total segments (remote
    /// alloc-DoS defense — an untrusted MPD specifying unbounded `<S r="...">`.
    TimelineTooLong {
        /// The segment count (or a hint of it) that breached the cap.
        count_hint: u64,
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

impl fmt::Display for DashParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            DashParseError::UnexpectedEof => {
                write!(f, "unexpected end of input while parsing MPD XML")
            }
            DashParseError::UnterminatedTag { pos } => {
                write!(
                    f,
                    "unterminated XML tag/comment/declaration at byte offset {pos}"
                )
            }
            DashParseError::MalformedAttribute { pos } => {
                write!(f, "malformed XML attribute near byte offset {pos}")
            }
            DashParseError::UnexpectedElement { expected, found } => {
                if found.is_empty() {
                    write!(f, "expected element <{expected}>, found none")
                } else {
                    write!(f, "expected element <{expected}>, found <{found}>")
                }
            }
            DashParseError::MissingAttribute { element, attr } => {
                write!(f, "<{element}> is missing required attribute @{attr}")
            }
            DashParseError::InvalidAttributeValue {
                element,
                attr,
                value,
            } => write!(f, "<{element}>@{attr} has invalid value {value:?}"),
            DashParseError::InvalidDuration { value } => {
                write!(f, "invalid xs:duration {value:?}")
            }
            DashParseError::TimelineTooLong { count_hint } => {
                write!(
                    f,
                    "SegmentTimeline exceeded max segment count ({count_hint} > {})",
                    MAX_TIMELINE_SEGMENTS
                )
            }
            DashParseError::MismatchedEndTag { expected, found } => {
                if found.is_empty() {
                    write!(f, "expected closing tag </{expected}>, found none")
                } else {
                    write!(f, "expected closing tag </{expected}>, found </{found}>")
                }
            }
            DashParseError::Xml { pos, message } => {
                write!(f, "XML error at byte offset {pos}: {message}")
            }
        }
    }
}

#[cfg(feature = "std")]
impl std::error::Error for DashParseError {}

/// Crate-local result alias for this module.
type Result<T> = core::result::Result<T, DashParseError>;

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
fn xml_error(reader: &XmlReader<'_>, err: &quick_xml::Error) -> DashParseError {
    use quick_xml::Error as E;
    let pos = usize::try_from(reader.buffer_position()).unwrap_or(usize::MAX);
    match err {
        E::Syntax(_) => DashParseError::UnterminatedTag { pos },
        E::IllFormed(IllFormedError::MismatchedEndTag { expected, found }) => {
            DashParseError::MismatchedEndTag {
                expected: local_part(expected),
                found: local_part(found),
            }
        }
        _ => DashParseError::Xml {
            pos,
            message: err.to_string(),
        },
    }
}

/// A well-formedness error described by `message` at the reader's position.
fn xml_message(reader: &XmlReader<'_>, message: String) -> DashParseError {
    DashParseError::Xml {
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
        let attr = attr.map_err(|_| DashParseError::MalformedAttribute { pos })?;
        let value = attr
            .normalized_value(XmlVersion::Implicit1_0)
            .map_err(|_| DashParseError::MalformedAttribute { pos })?;
        if !value.chars().all(is_xml_char) {
            return Err(DashParseError::MalformedAttribute { pos });
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
            None => return Err(DashParseError::UnexpectedEof),
        }
    }
    Ok(())
}

/// The character data of an already-open element `name` (entity and character
/// references resolved by quick-xml, CDATA included), consuming its end tag.
/// `None` for a self-closing element or one holding nested markup (the
/// nested subtree is skipped — callers only model text-only elements).
fn text_content(
    reader: &mut XmlReader<'_>,
    name: &'static str,
    self_closing: bool,
) -> Result<Option<String>> {
    if self_closing {
        return Ok(None);
    }
    let mut text = String::new();
    let mut nested = false;
    loop {
        let event = read_event(reader)?;
        if let Some(piece) = char_data(&event).map_err(|message| xml_message(reader, message))? {
            text.push_str(&piece);
            continue;
        }
        match event {
            Event::Start(_) => {
                nested = true;
                skip_element(reader)?;
            }
            Event::Empty(_) => nested = true,
            Event::End(e) => {
                let found = e.local_name().into_inner();
                if found != name {
                    return Err(DashParseError::MismatchedEndTag {
                        expected: name.to_string(),
                        found: found.to_string(),
                    });
                }
                return Ok(if nested { None } else { Some(text) });
            }
            Event::Eof => return Err(DashParseError::UnexpectedEof),
            _ => {}
        }
    }
}

// ---------------------------------------------------------------------------
// Defaults (ISO/IEC 23009-1 §5.3.9.4.4/.6)
// ---------------------------------------------------------------------------

/// `SegmentTemplate@timescale` default (§5.3.9.2.2) — ticks per second when
/// the attribute is absent.
const DEFAULT_TIMESCALE: u64 = 1;
/// `SegmentTemplate@startNumber` default (§5.3.9.4.4) — the first `$Number$`
/// value when the attribute is absent.
const DEFAULT_START_NUMBER: u64 = 1;
/// `SegmentTemplate@presentationTimeOffset` default (§5.3.9.2.2).
const DEFAULT_PRESENTATION_TIME_OFFSET: u64 = 0;
/// `S@r` default (§5.3.9.6.2) — a run of exactly one segment (no repeats).
const DEFAULT_REPEAT: i64 = 0;

// ---------------------------------------------------------------------------
// Unbounded-input caps (remote alloc-DoS defense)
// ---------------------------------------------------------------------------

/// Cap on total segments in a `SegmentTimeline` enumeration. A hostile MPD
/// specifying a huge `<S r="...">` repeat count would allocate unboundedly
/// otherwise. 100,000 segments is generous (a 2-second segment window spanning
/// ~55 hours of live content), while still protecting against allocation DoS.
pub const MAX_TIMELINE_SEGMENTS: usize = 100_000;

/// Cap on the `%0Nd` zero-padding width in a `$Number$` / `$Time$` /
/// `$Bandwidth$` substitution. A u64 in decimal has at most 20 digits; any
/// wider padding is meaningless and a hostile `$Number%9999999999d$` in the
/// `@media` template would allocate / loop unboundedly. This cap prevents that
/// alloc-DoS while preserving all valid use.
pub const MAX_FORMAT_WIDTH: usize = 20;

// ---------------------------------------------------------------------------
// Model
// ---------------------------------------------------------------------------

/// `MPD@type` (ISO/IEC 23009-1 §5.3.1.2 Table 3).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[non_exhaustive]
pub enum MpdType {
    /// VOD — the presentation's Periods are fixed and never change.
    #[default]
    Static,
    /// Live — the presentation may still be extended with new segments/Periods.
    Dynamic,
}

impl MpdType {
    /// The spec-token label for this `MPD@type` value.
    pub fn name(&self) -> &'static str {
        match self {
            MpdType::Static => "static",
            MpdType::Dynamic => "dynamic",
        }
    }
}

broadcast_common::impl_spec_display!(MpdType);

/// A parsed MPD document (ISO/IEC 23009-1 §5.3.1) — the root of
/// [`Mpd::parse`]'s output, and the structural inverse of
/// [`crate::dash::DashPackager`]'s rendered XML output.
#[derive(Debug, Clone, PartialEq)]
pub struct Mpd {
    /// `MPD@profiles` (§5.3.1.2).
    pub profiles: String,
    /// `MPD@type` (§5.3.1.2 Table 3); `Static` when the attribute is absent
    /// (the spec default).
    pub mpd_type: MpdType,
    /// `MPD@mediaPresentationDuration` (VOD only, §5.3.1.2 Table 3).
    pub media_presentation_duration: Option<Duration>,
    /// `MPD@minimumUpdatePeriod` (live only, §5.3.1.2 Table 3).
    pub minimum_update_period: Option<Duration>,
    /// `MPD@availabilityStartTime` (live only, §5.3.1.2 Table 3) — kept as the
    /// raw ISO-8601 UTC string (no wall-clock parsing in this `no_std` crate).
    pub availability_start_time: Option<String>,
    /// `MPD@timeShiftBufferDepth` (live only, §5.3.1.2 Table 3).
    pub time_shift_buffer_depth: Option<Duration>,
    /// The first non-empty `BaseURL` child declared at the MPD level
    /// (§5.3.9.2), or `None`. This is the outermost element of the
    /// `BaseURL` chain a segment URL resolves through (MPD > Period >
    /// AdaptationSet > Representation).
    pub base_url: Option<String>,
    /// The document's `Period` elements, in document order.
    pub periods: Vec<Period>,
}

/// A `Period` element (ISO/IEC 23009-1 §5.3.2).
#[derive(Debug, Clone, PartialEq)]
pub struct Period {
    /// `Period@id`.
    pub id: Option<String>,
    /// `Period@start` (§5.3.2.2).
    pub start: Option<Duration>,
    /// `Period@duration` (§5.3.2.2).
    pub duration: Option<Duration>,
    /// The first `BaseURL` child declared at this level (§5.3.9.2), or `None`.
    pub base_url: Option<String>,
    /// The Period's `AdaptationSet` elements, in document order.
    pub adaptation_sets: Vec<AdaptationSet>,
}

/// An `AdaptationSet` element (ISO/IEC 23009-1 §5.3.3).
#[derive(Debug, Clone, PartialEq)]
pub struct AdaptationSet {
    /// `AdaptationSet@mimeType` (§5.3.3.2), when the AdaptationSet itself
    /// carries one (real-world manifests often only carry it per-Representation).
    pub mime_type: Option<String>,
    /// `AdaptationSet@contentType` (§5.3.3.2, e.g. `"video"`/`"audio"`).
    pub content_type: Option<String>,
    /// The first `BaseURL` child declared at this level (§5.3.9.2), or `None`.
    pub base_url: Option<String>,
    /// The AdaptationSet-level `SegmentTemplate`, if declared directly here
    /// (§5.3.9.1 — SegmentTemplate is inheritable down to `Representation`;
    /// see [`Mpd::parse`]'s inheritance note for how that's resolved onto
    /// each [`Representation::segment_template`]). This is the *effective*
    /// template for the level: its own attributes merged over the
    /// `Period`-level ones it inherits from.
    pub segment_template: Option<SegmentTemplate>,
    /// The set's `Representation` elements, in document order.
    pub representations: Vec<Representation>,
}

/// A `Representation` element (ISO/IEC 23009-1 §5.3.5).
#[derive(Debug, Clone, PartialEq)]
pub struct Representation {
    /// `Representation@id` (required, §5.3.5.2).
    pub id: String,
    /// `Representation@bandwidth` in bits/second (required, §5.3.5.2).
    pub bandwidth: u64,
    /// `Representation@codecs` (RFC 6381).
    pub codecs: Option<String>,
    /// `Representation@width`, video only.
    pub width: Option<u32>,
    /// `Representation@height`, video only.
    pub height: Option<u32>,
    /// `Representation@frameRate` (`num/den` or integer string), video only.
    pub frame_rate: Option<String>,
    /// `Representation@audioSamplingRate`, audio only.
    pub audio_sampling_rate: Option<u32>,
    /// `Representation@mimeType` (§5.3.7.2).
    pub mime_type: Option<String>,
    /// The first `BaseURL` child declared at this level (§5.3.9.2), or `None`.
    /// Only the level's own `BaseURL` is reported; resolving it against the
    /// enclosing levels' is the caller's job (the module's scope is a single
    /// level per element).
    pub base_url: Option<String>,
    /// This Representation's effective `SegmentTemplate`: its own child
    /// element merged, attribute by attribute, over its `AdaptationSet`'s (in
    /// turn merged over the `Period`'s) — see [`Mpd::parse`]'s inheritance
    /// note. `None` if no level declared one — e.g. a Representation
    /// addressed only by `SegmentList`/`SegmentBase`, which this v1 parser
    /// does not resolve (tolerated, not an error: see the module docs).
    pub segment_template: Option<SegmentTemplate>,
}

/// A `SegmentTemplate` element (ISO/IEC 23009-1 §5.3.9.4.4).
#[derive(Debug, Clone, PartialEq)]
pub struct SegmentTemplate {
    /// `@timescale` — ticks per second (default 1, §5.3.9.2.2).
    pub timescale: u64,
    /// `@initialization` template (contains `$RepresentationID$`).
    pub initialization: Option<String>,
    /// `@media` template (contains `$RepresentationID$` plus `$Number$` or
    /// `$Time$`).
    pub media: Option<String>,
    /// `@startNumber` (default 1, §5.3.9.4.4).
    pub start_number: u64,
    /// `@duration` — the nominal per-segment duration for `$Number$`
    /// addressing (§5.3.9.4.4 L1688); `None` under `$Time$`/`SegmentTimeline`
    /// addressing (the two are mutually exclusive, §5.3.9.4.4 L1628).
    pub duration: Option<u64>,
    /// `@presentationTimeOffset` (default 0, §5.3.9.2.2).
    pub presentation_time_offset: u64,
    /// The child `SegmentTimeline`, under `$Time$` addressing.
    pub timeline: Option<SegmentTimeline>,
}

impl SegmentTemplate {
    /// Enumerate `$Number$` addressing **without** a `SegmentTimeline` —
    /// `count` consecutive segment numbers starting at [`Self::start_number`]
    /// (§5.3.9.4.4 L1688: segment N starts at `(N - startNumber) * @duration`).
    /// The caller supplies `count` (derived from the Period/Representation's
    /// total duration and [`Self::duration`] — presentation-level arithmetic
    /// this module doesn't perform).
    pub fn number_sequence(&self, count: usize) -> Vec<u64> {
        (0..count as u64)
            .map(|i| self.start_number.saturating_add(i))
            .collect()
    }

    /// Substitute `$RepresentationID$`/`$Number$`/`$Time$`/`$Bandwidth$`
    /// (each optionally with a `%0Nd` zero-padded width) plus `$$` → `$`
    /// (ISO/IEC 23009-1 §5.3.9.4.4 Table 16) into a `template` string (an
    /// [`Self::initialization`] or [`Self::media`] value).
    ///
    /// A dynamic identifier whose value was not supplied (e.g. `$Time$` when
    /// `time` is `None`) is emitted **literally** (`$Time$`) rather than
    /// silently dropped, so a caller misuse is visible in the resolved URL
    /// instead of producing a subtly wrong one. Unrecognized identifiers are
    /// likewise passed through literally.
    pub fn resolve(
        template: &str,
        representation_id: &str,
        number: Option<u64>,
        time: Option<u64>,
        bandwidth: Option<u64>,
    ) -> String {
        let mut out = String::with_capacity(template.len());
        let mut rest = template;
        while let Some(dollar) = rest.find('$') {
            out.push_str(&rest[..dollar]);
            let tail = &rest[dollar + 1..];
            if let Some(after_escape) = tail.strip_prefix('$') {
                out.push('$');
                rest = after_escape;
                continue;
            }
            match tail.find('$') {
                Some(end) => {
                    let ident = &tail[..end];
                    let (name, width) = match ident.split_once('%') {
                        Some((n, fmt)) => (
                            n,
                            fmt.strip_suffix('d').and_then(|w| w.parse::<usize>().ok()),
                        ),
                        None => (ident, None),
                    };
                    match name {
                        "RepresentationID" => out.push_str(representation_id),
                        "Number" => push_numeric(&mut out, number, width, ident),
                        "Time" => push_numeric(&mut out, time, width, ident),
                        "Bandwidth" => push_numeric(&mut out, bandwidth, width, ident),
                        _ => {
                            out.push('$');
                            out.push_str(ident);
                            out.push('$');
                        }
                    }
                    rest = &tail[end + 1..];
                }
                None => {
                    // Unterminated identifier: emit the '$' literally and keep
                    // scanning the remainder as plain text.
                    out.push('$');
                    rest = tail;
                }
            }
        }
        out.push_str(rest);
        out
    }
}

/// Push a resolved `$Number$`/`$Time$`/`$Bandwidth$` value (zero-padded to
/// `width` if given), or the identifier literally (`$ident$`) if `value` is
/// `None`.
fn push_numeric(out: &mut String, value: Option<u64>, width: Option<usize>, ident: &str) {
    match value {
        Some(v) => match width {
            Some(w) => out.push_str(&format_width(v, w)),
            None => {
                out.push_str(&v.to_string());
            }
        },
        None => {
            out.push('$');
            out.push_str(ident);
            out.push('$');
        }
    }
}

/// Format `n` zero-padded to at least `width` decimal digits. The width is
/// clamped to [`MAX_FORMAT_WIDTH`] to defend against unbounded allocation from
/// maliciously large `%0Nd` directives in the MPD's template strings.
fn format_width(n: u64, width: usize) -> String {
    let width = width.min(MAX_FORMAT_WIDTH);
    let digits = n.to_string();
    if digits.len() >= width {
        digits
    } else {
        let mut out = String::with_capacity(width);
        for _ in 0..(width - digits.len()) {
            out.push('0');
        }
        out.push_str(&digits);
        out
    }
}

/// A `SegmentTimeline` element (ISO/IEC 23009-1 §5.3.9.6) — an explicit list
/// of segment-duration runs, for `$Time$` addressing.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct SegmentTimeline {
    /// The `<S>` entries, in document order.
    pub segments: Vec<S>,
}

impl SegmentTimeline {
    /// Expand every `<S>` run into `(number, time)` pairs in presentation
    /// order (ISO/IEC 23009-1 §5.3.9.6). `number` starts at `start_number`
    /// (the enclosing [`SegmentTemplate::start_number`]) and increments by
    /// one per segment; `time` is each segment's start time in the
    /// representation's `@timescale` ticks — explicit via [`S::t`] when
    /// present, else the spec-default derivation (previous segment's
    /// `t + d`, §5.3.9.6.2 L1791), starting from 0 if the very first `S`
    /// omits `@t`.
    ///
    /// A negative [`S::r`] (`-1`, meaning "repeat until the next `S`'s `@t`
    /// or the end of the Period", §5.3.9.6.2) cannot be resolved without that
    /// external context; it is tolerated as a single occurrence (not a
    /// panic/error) rather than looping unboundedly.
    ///
    /// Returns an error if the total segment count (summed across all `S`
    /// entries, counting each `r+1` repetition) would exceed
    /// [`MAX_TIMELINE_SEGMENTS`], defending against remote alloc-DoS attacks
    /// via hostile MPDs with unbounded `<S r="...">` values.
    pub fn enumerate(&self, start_number: u64) -> Result<Vec<(u64, u64)>> {
        let mut out = Vec::new();
        let mut number = start_number;
        let mut time: u64 = 0;
        let mut total_segments: u64 = 0;
        for s in &self.segments {
            if let Some(t) = s.t {
                time = t;
            }
            let repeats: u64 = if s.r < 0 {
                1
            } else {
                (s.r as u64).saturating_add(1)
            };
            // Guard the accumulation: if this S's repeats would exceed the cap
            // on its own, or the accumulated total would, return an error.
            if repeats > MAX_TIMELINE_SEGMENTS as u64 {
                return Err(DashParseError::TimelineTooLong {
                    count_hint: repeats,
                });
            }
            total_segments = total_segments.saturating_add(repeats);
            if total_segments as usize > MAX_TIMELINE_SEGMENTS {
                return Err(DashParseError::TimelineTooLong {
                    count_hint: total_segments,
                });
            }
            for _ in 0..repeats {
                out.push((number, time));
                number = number.saturating_add(1);
                time = time.saturating_add(s.d);
            }
        }
        Ok(out)
    }
}

/// One `<S>` run entry inside a `SegmentTimeline` (ISO/IEC 23009-1 §5.3.9.6).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct S {
    /// `@t` — this run's explicit start time, in the representation's
    /// `@timescale` ticks. Only the first `<S>` is required to carry it
    /// (§5.3.9.6.2 L1791); later runs derive it from the previous run.
    pub t: Option<u64>,
    /// `@d` — this run's segment duration, in `@timescale` ticks (required).
    pub d: u64,
    /// `@r` — repeat count: this run has `r + 1` segments of duration `@d`
    /// (default 0, i.e. one segment). `-1` means "repeat until the next
    /// `S`'s `@t` or Period end" (§5.3.9.6.2) — see
    /// [`SegmentTimeline::enumerate`]'s handling.
    pub r: i64,
}

// ---------------------------------------------------------------------------
// xs:duration
// ---------------------------------------------------------------------------

/// Seconds in a day (used by this module's unit tests).
#[cfg(test)]
const SECONDS_PER_DAY: u64 = 24 * 60 * 60;
/// Seconds in an hour (used by this module's unit tests).
#[cfg(test)]
const SECONDS_PER_HOUR: u64 = 60 * 60;

/// Parse an `xs:duration` string (W3C XML Schema 1.1 Part 2 §3.3.6, as used by
/// every duration-valued MPD attribute — §5.3.1.2/§5.3.2.2/§5.3.9.2.2): the
/// `PnYnMnDTnHnMnS` lexical form, e.g. `PT1H2M3.5S`, `PT4S`, `PT0S`, `P1DT2H`.
///
/// Lexical rules (checked here, before `jiff` converts the value): the `P`
/// and every designator (`Y M D T H S`) are UPPER case; each of the
/// year/month/day/hour/minute numbers is digits only, in that order, each at
/// most once; only the seconds may carry a fraction, and it needs a digit on
/// both sides of the `.` (`PT.5S` and `PT5.S` are invalid); at least one
/// component must be present, and `T` must be followed by one. A fraction
/// longer than nine digits is valid XSD and is TRUNCATED to nanoseconds.
/// Surrounding whitespace is trimmed. Weeks (`P1W`), a sign on a component, a
/// comma decimal separator and fractions on hours/minutes are not xs:duration
/// and are rejected.
///
/// The value is converted by `jiff`'s ISO 8601 span parser with a day counting
/// as 24 hours. Every input outside that is a [`DashParseError::InvalidDuration`]
/// (never a panic): a lexically valid but unrepresentable value — calendar
/// `nY`/`nM` (years/months are ambiguous without a reference date), a negative
/// duration (`-PT1S`, valid XSD but [`Duration`] is unsigned), or a magnitude
/// beyond `jiff`'s span limits (`PT999999999H`).
pub fn parse_iso8601_duration(s: &str) -> Result<Duration> {
    let trimmed = s.trim();
    let invalid = || DashParseError::InvalidDuration {
        value: trimmed.to_string(),
    };
    let normalised = normalise_xs_duration(trimmed).ok_or_else(invalid)?;
    let span = jiff::fmt::temporal::SpanParser::new()
        .parse_span(normalised.as_str())
        .map_err(|_| invalid())?;
    // Days count as 24 h; years and months are calendar units that need a
    // reference date, so `to_duration` refuses them.
    let signed = span
        .to_duration(jiff::SpanRelativeTo::days_are_24_hours())
        .map_err(|_| invalid())?;
    Duration::try_from(signed).map_err(|_| invalid())
}

/// Validate `s` against the xs:duration lexical grammar (see
/// [`parse_iso8601_duration`]) and return it with the seconds fraction
/// truncated to nine digits; `None` if it is not in the lexical space or is
/// negative (a leading `-` is lexically valid, but `Duration` cannot hold it).
fn normalise_xs_duration(s: &str) -> Option<String> {
    let rest = s.strip_prefix('P')?;
    let (date_part, time_part) = match rest.split_once('T') {
        Some((d, t)) => (d, Some(t)),
        None => (rest, None),
    };
    let mut components = 0usize;
    let mut out = String::from("P");
    // Date part: [nY][nM][nD], in order, digits only.
    let mut remaining = date_part;
    for designator in ['Y', 'M', 'D'] {
        if let Some((digits, tail)) = remaining.split_once(designator) {
            if digits.is_empty() || !digits.bytes().all(|b| b.is_ascii_digit()) {
                return None;
            }
            out.push_str(digits);
            out.push(designator);
            remaining = tail;
            components += 1;
        }
    }
    if !remaining.is_empty() {
        return None;
    }
    if let Some(time) = time_part {
        out.push('T');
        let mut remaining = time;
        let mut time_components = 0usize;
        for designator in ['H', 'M'] {
            if let Some((digits, tail)) = remaining.split_once(designator) {
                if digits.is_empty() || !digits.bytes().all(|b| b.is_ascii_digit()) {
                    return None;
                }
                out.push_str(digits);
                out.push(designator);
                remaining = tail;
                time_components += 1;
            }
        }
        if let Some((number, tail)) = remaining.split_once('S') {
            if !tail.is_empty() {
                return None;
            }
            let (whole, frac) = match number.split_once('.') {
                Some((w, f)) => (w, Some(f)),
                None => (number, None),
            };
            if whole.is_empty() || !whole.bytes().all(|b| b.is_ascii_digit()) {
                return None;
            }
            out.push_str(whole);
            if let Some(frac) = frac {
                if frac.is_empty() || !frac.bytes().all(|b| b.is_ascii_digit()) {
                    return None;
                }
                out.push('.');
                // Valid XSD allows any number of digits; Duration holds nanoseconds.
                out.push_str(&frac[..frac.len().min(9)]);
            }
            out.push('S');
            time_components += 1;
        } else if !remaining.is_empty() {
            return None;
        }
        if time_components == 0 {
            return None;
        }
        components += time_components;
    }
    (components > 0).then_some(out)
}

// ---------------------------------------------------------------------------
// DASH-specific attribute helpers (XML parsing is in the `xml` module)
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
        .ok_or(DashParseError::MissingAttribute { element, attr: key })
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
                .map_err(|_| DashParseError::InvalidAttributeValue {
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
    let v = attr(attrs, key).ok_or(DashParseError::MissingAttribute { element, attr: key })?;
    v.trim()
        .parse::<T>()
        .map_err(|_| DashParseError::InvalidAttributeValue {
            element,
            attr: key,
            value: v.to_string(),
        })
}

fn parse_duration_attr(attrs: &[(String, String)], key: &str) -> Result<Option<Duration>> {
    match attr(attrs, key) {
        Some(v) => Ok(Some(parse_iso8601_duration(v)?)),
        None => Ok(None),
    }
}

// ---------------------------------------------------------------------------
// Parsing
// ---------------------------------------------------------------------------

impl Mpd {
    /// The `BaseURL` chain for `representation` of `adaptation_set` in
    /// `period`, outermost first: MPD, then Period, then AdaptationSet, then
    /// Representation (ISO/IEC 23009-1 §5.6.5 — a `BaseURL` is inherited down
    /// the same hierarchy as segment information).
    ///
    /// Empty entries are dropped, since an empty `BaseURL` contributes nothing
    /// to a chain and would otherwise resolve a later entry against itself.
    pub fn base_url_chain(
        &self,
        period: &Period,
        adaptation_set: &AdaptationSet,
        representation: &Representation,
    ) -> Vec<String> {
        [
            &self.base_url,
            &period.base_url,
            &adaptation_set.base_url,
            &representation.base_url,
        ]
        .into_iter()
        .flatten()
        .filter(|value| !value.is_empty())
        .cloned()
        .collect()
    }

    /// Resolve a segment reference against a Representation's `BaseURL` chain
    /// and the MPD's own location, by RFC 3986 §5.2 reference resolution
    /// (`url::Url::join`) applied to each level in turn; see
    /// [`crate::base_url`].
    ///
    /// `mpd_url` is the MPD's own URL — its source URL, or
    /// `Url::from_file_path` for a file — and `None` for an in-memory MPD, in
    /// which case a result that stays relative is returned relative.
    /// `reference` is typically a resolved `SegmentTemplate`'s
    /// `initialization`/`media` URL, which may itself be relative.
    ///
    /// `None` means the reference or a `BaseURL` carries a control character
    /// or whitespace (a CR/LF inside a URL would let a manifest smuggle a
    /// second request line into anything that later writes an HTTP request from
    /// it) or does not parse.
    pub fn resolve_segment_url(
        &self,
        mpd_url: Option<&url::Url>,
        period: &Period,
        adaptation_set: &AdaptationSet,
        representation: &Representation,
        reference: &str,
    ) -> Option<String> {
        crate::base_url::resolve_chain(
            mpd_url,
            &self.base_url_chain(period, adaptation_set, representation),
            reference,
        )
    }

    /// Parse an MPD document (ISO/IEC 23009-1 §5.3) into this structural
    /// model — the inverse of [`crate::dash::DashPackager`]'s rendered XML
    /// output.
    ///
    /// # `SegmentTemplate` inheritance
    ///
    /// `SegmentTemplate` is an inheritable property along the
    /// `Period` > `AdaptationSet` > `Representation` chain, and a lower level
    /// overrides only the attributes it actually declares (§5.3.9.1). This
    /// parser resolves that inheritance eagerly: each level's element is
    /// merged, attribute by attribute, over the level above it, and
    /// [`AdaptationSet::segment_template`] / [`Representation::segment_template`]
    /// are the *effective* templates a caller should use — regardless of which
    /// element declared which attribute in the source XML. An attribute no
    /// level declares keeps its spec default (§5.3.9.2.2 `@timescale`/
    /// `@presentationTimeOffset`, §5.3.9.4.4 `@startNumber`), and a
    /// `SegmentTimeline` is taken from the lowest level that carries one.
    pub fn parse(xml: &str) -> Result<Mpd> {
        const EL: &str = "MPD";
        let mut xml_reader = new_reader(xml);
        let reader = &mut xml_reader;

        let (mpd_attrs, mpd_self_closing) = match next_tag(reader)? {
            Some(Tag::Open {
                name,
                attrs,
                self_closing,
            }) if name == "MPD" => (attrs, self_closing),
            Some(Tag::Open { name, .. }) => {
                return Err(DashParseError::UnexpectedElement {
                    expected: EL,
                    found: name.to_string(),
                });
            }
            Some(Tag::Close { .. }) => {
                return Err(DashParseError::UnexpectedElement {
                    expected: EL,
                    found: String::new(),
                });
            }
            None => return Err(DashParseError::UnexpectedEof),
        };

        let profiles = required_attr_owned(&mpd_attrs, "profiles", EL)?;
        let mpd_type = match attr(&mpd_attrs, "type") {
            Some("dynamic") => MpdType::Dynamic,
            _ => MpdType::Static,
        };
        let media_presentation_duration =
            parse_duration_attr(&mpd_attrs, "mediaPresentationDuration")?;
        let minimum_update_period = parse_duration_attr(&mpd_attrs, "minimumUpdatePeriod")?;
        let availability_start_time = attr_owned(&mpd_attrs, "availabilityStartTime");
        let time_shift_buffer_depth = parse_duration_attr(&mpd_attrs, "timeShiftBufferDepth")?;

        let mut base_url = None;
        let mut periods = Vec::new();
        if !mpd_self_closing {
            loop {
                match next_tag(reader)? {
                    Some(Tag::Open {
                        name,
                        attrs,
                        self_closing,
                    }) if name == "BaseURL" => {
                        let found = parse_base_url(reader, &attrs, self_closing)?;
                        keep_first_base_url(&mut base_url, found);
                    }
                    Some(Tag::Open {
                        name,
                        attrs,
                        self_closing,
                    }) if name == "Period" => {
                        periods.push(parse_period(reader, attrs, self_closing)?)
                    }
                    Some(Tag::Open { self_closing, .. }) => {
                        if !self_closing {
                            skip_element(reader)?;
                        }
                    }
                    Some(Tag::Close { name }) => {
                        if name != EL {
                            return Err(DashParseError::MismatchedEndTag {
                                expected: EL.to_string(),
                                found: name.to_string(),
                            });
                        }
                        break;
                    }
                    None => return Err(DashParseError::UnexpectedEof),
                }
            }
        }

        Ok(Mpd {
            profiles,
            mpd_type,
            media_presentation_duration,
            minimum_update_period,
            availability_start_time,
            time_shift_buffer_depth,
            base_url,
            periods,
        })
    }
}

/// The per-level pieces a `SegmentTemplate` contributes before it is merged
/// with its parent level — see [`merge_templates`].
#[derive(Debug, Clone)]
struct TemplateLayer {
    attrs: LayerTemplate,
    timeline: Option<SegmentTimeline>,
}

fn parse_period<'a>(
    reader: &mut XmlReader<'a>,
    attrs: Vec<(String, String)>,
    self_closing: bool,
) -> Result<Period> {
    const EL: &str = "Period";
    let id = attr_owned(&attrs, "id");
    let start = parse_duration_attr(&attrs, "start")?;
    let duration = parse_duration_attr(&attrs, "duration")?;

    let mut base_url = None;
    let mut template: Option<TemplateLayer> = None;
    let mut adaptation_sets = Vec::new();
    if !self_closing {
        loop {
            match next_tag(reader)? {
                Some(Tag::Open {
                    name,
                    attrs,
                    self_closing,
                }) if name == "BaseURL" => {
                    let found = parse_base_url(reader, &attrs, self_closing)?;
                    keep_first_base_url(&mut base_url, found);
                }
                Some(Tag::Open {
                    name,
                    attrs,
                    self_closing,
                }) if name == "SegmentTemplate" => {
                    template = Some(parse_segment_template_layer(reader, attrs, self_closing)?)
                }
                Some(Tag::Open {
                    name,
                    attrs,
                    self_closing,
                }) if name == "AdaptationSet" => {
                    let parent = template
                        .as_ref()
                        .map(|t| merge_templates(None, t.attrs.clone(), t.timeline.clone()));
                    adaptation_sets.push(parse_adaptation_set(
                        reader,
                        attrs,
                        self_closing,
                        parent.as_ref(),
                    )?);
                }
                Some(Tag::Open { self_closing, .. }) => {
                    if !self_closing {
                        skip_element(reader)?;
                    }
                }
                Some(Tag::Close { name }) => {
                    if name != EL {
                        return Err(DashParseError::MismatchedEndTag {
                            expected: EL.to_string(),
                            found: name.to_string(),
                        });
                    }
                    break;
                }
                None => return Err(DashParseError::UnexpectedEof),
            }
        }
    }

    Ok(Period {
        id,
        start,
        duration,
        base_url,
        adaptation_sets,
    })
}

/// Parse a `BaseURL` element's text content (§5.3.9.2 `BaseURLType`): the
/// element's character data with surrounding whitespace trimmed. Nested
/// elements are skipped (the type allows none), and an element with no text
/// yields `None`.
fn parse_base_url<'a>(
    reader: &mut XmlReader<'a>,
    attrs: &[(String, String)],
    self_closing: bool,
) -> Result<Option<String>> {
    let _ = attrs;
    let text = text_content(reader, "BaseURL", self_closing)?;
    Ok(text.map(|t| t.trim().to_string()).filter(|t| !t.is_empty()))
}

/// Fold one parsed `BaseURL` into a level's value, keeping the **first
/// non-empty** one.
///
/// ISO/IEC 23009-1 §5.6.5: several `BaseURL` children of one element are
/// *alternates* for the same level, consulted in order — not successive
/// overrides of one another. So a later `BaseURL` never replaces an earlier
/// one, and an empty element (which `parse_base_url` reports as `None`) cannot
/// clear a value an earlier sibling established.
fn keep_first_base_url(slot: &mut Option<String>, found: Option<String>) {
    if slot.is_none() {
        *slot = found;
    }
}

fn parse_adaptation_set<'a>(
    reader: &mut XmlReader<'a>,
    attrs: Vec<(String, String)>,
    self_closing: bool,
    parent: Option<&SegmentTemplate>,
) -> Result<AdaptationSet> {
    const EL: &str = "AdaptationSet";
    let mime_type = attr_owned(&attrs, "mimeType");
    let content_type = attr_owned(&attrs, "contentType");
    let mut base_url = None;

    // Each Representation is collected with its *own* raw template layer (if it
    // declared one) and no inheritance applied yet: a Representation's child
    // `SegmentTemplate` is itself a layer in the hierarchy (§5.3.9.1), so it
    // contributes its attributes rather than replacing the set's.
    let mut own_layer: Option<TemplateLayer> = None;
    let mut parsed: Vec<(Representation, Option<TemplateLayer>)> = Vec::new();
    if !self_closing {
        loop {
            match next_tag(reader)? {
                Some(Tag::Open {
                    name,
                    attrs,
                    self_closing,
                }) if name == "BaseURL" => {
                    let found = parse_base_url(reader, &attrs, self_closing)?;
                    keep_first_base_url(&mut base_url, found);
                }
                Some(Tag::Open {
                    name,
                    attrs,
                    self_closing,
                }) if name == "SegmentTemplate" => {
                    own_layer = Some(parse_segment_template_layer(reader, attrs, self_closing)?)
                }
                Some(Tag::Open {
                    name,
                    attrs,
                    self_closing,
                }) if name == "Representation" => {
                    parsed.push(parse_representation(reader, attrs, self_closing)?)
                }
                Some(Tag::Open { self_closing, .. }) => {
                    if !self_closing {
                        skip_element(reader)?;
                    }
                }
                Some(Tag::Close { name }) => {
                    if name != EL {
                        return Err(DashParseError::MismatchedEndTag {
                            expected: EL.to_string(),
                            found: name.to_string(),
                        });
                    }
                    break;
                }
                None => return Err(DashParseError::UnexpectedEof),
            }
        }
    }

    // This set's effective template: its own layer over the parent (Period)
    // level's, which was resolved before this element was parsed.
    let effective = match own_layer {
        Some(layer) => Some(merge_templates(parent, layer.attrs, layer.timeline)),
        None => parent.cloned(),
    };

    let mut representations = Vec::with_capacity(parsed.len());
    for (mut repr, layer) in parsed {
        repr.segment_template = match (effective.as_ref(), layer) {
            (Some(parent), Some(child)) => {
                Some(merge_templates(Some(parent), child.attrs, child.timeline))
            }
            (Some(parent), None) => Some(parent.clone()),
            (None, Some(child)) => Some(to_template(child)),
            (None, None) => None,
        };
        representations.push(repr);
    }
    let segment_template = effective;

    Ok(AdaptationSet {
        mime_type,
        content_type,
        base_url,
        segment_template,
        representations,
    })
}

fn parse_representation<'a>(
    reader: &mut XmlReader<'a>,
    attrs: Vec<(String, String)>,
    self_closing: bool,
) -> Result<(Representation, Option<TemplateLayer>)> {
    const EL: &str = "Representation";
    let id = required_attr_owned(&attrs, "id", EL)?;
    let bandwidth: u64 = required_attr_parse(&attrs, "bandwidth", EL)?;
    let codecs = attr_owned(&attrs, "codecs");
    let mime_type = attr_owned(&attrs, "mimeType");
    let width: Option<u32> = parse_attr(&attrs, "width", EL)?;
    let height: Option<u32> = parse_attr(&attrs, "height", EL)?;
    let frame_rate = attr_owned(&attrs, "frameRate");
    let audio_sampling_rate: Option<u32> = parse_attr(&attrs, "audioSamplingRate", EL)?;

    let mut base_url = None;
    let mut pending: Option<TemplateLayer> = None;
    if !self_closing {
        loop {
            match next_tag(reader)? {
                Some(Tag::Open {
                    name,
                    attrs,
                    self_closing,
                }) if name == "BaseURL" => {
                    let found = parse_base_url(reader, &attrs, self_closing)?;
                    keep_first_base_url(&mut base_url, found);
                }
                Some(Tag::Open {
                    name,
                    attrs,
                    self_closing,
                }) if name == "SegmentTemplate" => {
                    pending = Some(parse_segment_template_layer(reader, attrs, self_closing)?)
                }
                Some(Tag::Open { self_closing, .. }) => {
                    if !self_closing {
                        skip_element(reader)?;
                    }
                }
                Some(Tag::Close { name }) => {
                    if name != EL {
                        return Err(DashParseError::MismatchedEndTag {
                            expected: EL.to_string(),
                            found: name.to_string(),
                        });
                    }
                    break;
                }
                None => return Err(DashParseError::UnexpectedEof),
            }
        }
    }

    Ok((
        Representation {
            id,
            bandwidth,
            codecs,
            width,
            height,
            frame_rate,
            audio_sampling_rate,
            mime_type,
            base_url,
            segment_template: None,
        },
        pending,
    ))
}

/// Resolve a level's raw template parts with no parent: the two defaultless
/// template strings keep their own values (or stay absent).
fn to_template(layer: TemplateLayer) -> SegmentTemplate {
    merge_templates(None, layer.attrs, layer.timeline)
}

/// Split a `SegmentTemplate` element's attributes into the raw `Option`s the
/// inheritance model needs (ISO/IEC 23009-1 §5.3.9.1).
///
/// Every attribute is kept as declared — an absent one is `None`, *not* its
/// spec default. The distinction matters: §5.3.9.1 makes a lower-level
/// `SegmentTemplate` override "only the attribute it specifies", and the
/// standard's own worked examples rely on that (Annex G.13 gives each
/// Representation a `SegmentTemplate` carrying only `@initialization` and
/// inheriting `@media`/`@timescale`/`@duration`/`@startNumber` from the
/// AdaptationSet). Filling in a default here would make such a child silently
/// reset every parent attribute it did not restate.
fn split_template_attrs(attrs: &[(String, String)]) -> Result<LayerTemplate> {
    const EL: &str = "SegmentTemplate";
    Ok(LayerTemplate {
        timescale: parse_attr(attrs, "timescale", EL)?,
        initialization: attr_owned(attrs, "initialization"),
        media: attr_owned(attrs, "media"),
        start_number: parse_attr(attrs, "startNumber", EL)?,
        duration: parse_attr(attrs, "duration", EL)?,
        presentation_time_offset: parse_attr(attrs, "presentationTimeOffset", EL)?,
    })
}

/// One level's `SegmentTemplate` attributes as the element declared them, before
/// inheritance from the level above is resolved. Every field is `None` when the
/// attribute was absent, so [`merge_templates`] can tell "not restated" from
/// "restated as the default".
#[derive(Debug, Clone, PartialEq, Eq, Default)]
struct LayerTemplate {
    timescale: Option<u64>,
    initialization: Option<String>,
    media: Option<String>,
    start_number: Option<u64>,
    duration: Option<u64>,
    presentation_time_offset: Option<u64>,
}

/// Build a level's effective `SegmentTemplate` by merging this level's
/// attributes over the parent level's, one attribute at a time (ISO/IEC
/// 23009-1 §5.3.9.1: segment information "is hierarchical ... a lower level
/// overrides only the attributes it specifies").
///
/// An attribute this level did not declare takes the parent's value, or its
/// spec default when no level declared one. The `SegmentTimeline` child follows
/// the same rule element-wise: the lowest level that declares one wins.
fn merge_templates(
    parent: Option<&SegmentTemplate>,
    own: LayerTemplate,
    own_timeline: Option<SegmentTimeline>,
) -> SegmentTemplate {
    let inherit_u64 = |own: Option<u64>, get: fn(&SegmentTemplate) -> u64, default: u64| {
        own.or_else(|| parent.map(get)).unwrap_or(default)
    };
    // `@duration` and `SegmentTimeline` are mutually exclusive (§5.3.9.4.4:
    // "Either @duration ... or a SegmentTimeline element shall be present, but
    // not both"), and that rule binds the *effective* template in both
    // directions:
    //
    // - A child that introduces a timeline while the parent declared
    //   `@duration` is choosing `$Time$` addressing, so the parent's nominal
    //   duration must not be inherited alongside it.
    // - A child that declares its own `@duration` is choosing `$Number$`
    //   addressing, so the parent's timeline must not be inherited alongside
    //   *it* (the direction that was missing: the timeline used to be taken
    //   unconditionally from the parent, silently cancelling the child's own
    //   `@duration`).
    //
    // Each element is therefore inherited only when this level declared neither
    // of the two, or declared the same one.
    let own_has_timeline = own_timeline.is_some();
    let timeline = if own.duration.is_some() {
        // This level chose `$Number$`; the parent's `$Time$` timeline does not
        // apply.
        own_timeline
    } else {
        own_timeline.or_else(|| parent.and_then(|p| p.timeline.clone()))
    };
    let duration = if own_has_timeline {
        // This level chose `$Time$`; the parent's `$Number$` duration does not
        // apply.
        None
    } else {
        own.duration.or_else(|| parent.and_then(|p| p.duration))
    };
    SegmentTemplate {
        timescale: inherit_u64(own.timescale, |p| p.timescale, DEFAULT_TIMESCALE),
        initialization: own
            .initialization
            .or_else(|| parent.and_then(|p| p.initialization.clone())),
        media: own.media.or_else(|| parent.and_then(|p| p.media.clone())),
        start_number: inherit_u64(own.start_number, |p| p.start_number, DEFAULT_START_NUMBER),
        duration,
        presentation_time_offset: inherit_u64(
            own.presentation_time_offset,
            |p| p.presentation_time_offset,
            DEFAULT_PRESENTATION_TIME_OFFSET,
        ),
        timeline,
    }
}

/// Parse a `SegmentTemplate` element whose effective values still depend on a
/// parent level (`Period`/`AdaptationSet`), consuming the whole element.
fn parse_segment_template_layer(
    reader: &mut XmlReader<'_>,
    attrs: Vec<(String, String)>,
    self_closing: bool,
) -> Result<TemplateLayer> {
    let attrs = split_template_attrs(&attrs)?;
    let timeline = parse_segment_template_body(reader, self_closing)?;
    Ok(TemplateLayer { attrs, timeline })
}

/// Parse the `SegmentTimeline` child of a `SegmentTemplate` element (or its
/// absence), consuming the element's body up to its end tag.
fn parse_segment_template_body(
    reader: &mut XmlReader<'_>,
    self_closing: bool,
) -> Result<Option<SegmentTimeline>> {
    const EL: &str = "SegmentTemplate";
    let mut timeline = None;
    if !self_closing {
        loop {
            match next_tag(reader)? {
                Some(Tag::Open {
                    name, self_closing, ..
                }) if name == "SegmentTimeline" => {
                    timeline = Some(parse_segment_timeline(reader, self_closing)?)
                }
                Some(Tag::Open { self_closing, .. }) => {
                    if !self_closing {
                        skip_element(reader)?;
                    }
                }
                Some(Tag::Close { name }) => {
                    if name != EL {
                        return Err(DashParseError::MismatchedEndTag {
                            expected: EL.to_string(),
                            found: name.to_string(),
                        });
                    }
                    break;
                }
                None => return Err(DashParseError::UnexpectedEof),
            }
        }
    }
    Ok(timeline)
}

fn parse_segment_timeline(
    reader: &mut XmlReader<'_>,
    self_closing: bool,
) -> Result<SegmentTimeline> {
    const EL: &str = "SegmentTimeline";
    let mut segments = Vec::new();
    if !self_closing {
        loop {
            match next_tag(reader)? {
                Some(Tag::Open {
                    name,
                    attrs,
                    self_closing,
                }) if name == "S" => {
                    let t: Option<u64> = parse_attr(&attrs, "t", "S")?;
                    let d: u64 = required_attr_parse(&attrs, "d", "S")?;
                    let r: i64 = parse_attr(&attrs, "r", "S")?.unwrap_or(DEFAULT_REPEAT);
                    segments.push(S { t, d, r });
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
                        return Err(DashParseError::MismatchedEndTag {
                            expected: EL.to_string(),
                            found: name.to_string(),
                        });
                    }
                    break;
                }
                None => return Err(DashParseError::UnexpectedEof),
            }
        }
    }
    Ok(SegmentTimeline { segments })
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::vec;

    // -- quick-xml pull plumbing ---------------------------------------------

    /// The five XML specials in one string.
    const SPECIALS: &str = "a&b<c>d\"e'f";

    fn first_open_attrs(xml: &str) -> Vec<(String, String)> {
        let mut reader = new_reader(xml);
        loop {
            match next_tag(&mut reader).expect("tokenize") {
                Some(Tag::Open { attrs, .. }) => return attrs,
                Some(_) => continue,
                None => panic!("no start tag"),
            }
        }
    }

    /// r04-W46: a `>` inside a quoted attribute value is legal raw (XML 1.0
    /// §2.4 requires only `<` and `&` to be escaped there) and must not
    /// terminate the start tag.
    #[test]
    fn greater_than_inside_attribute_value_is_not_a_tag_end() {
        let attrs = first_open_attrs(r#"<SegmentTemplate media="a?x=1&amp;y=2>3" id="v"/>"#);
        assert_eq!(
            attrs,
            vec![
                ("media".to_string(), "a?x=1&y=2>3".to_string()),
                ("id".to_string(), "v".to_string()),
            ]
        );
        let attrs = first_open_attrs("<X a='>' b='z'/>");
        assert_eq!(attrs[0].1, ">");
    }

    /// Numeric character references and the five named entities resolve in
    /// attribute values.
    #[test]
    fn character_and_named_references_in_attribute_values() {
        let attrs = first_open_attrs(r#"<X media="a?x=1&#38;n=$Number$" hex="&#x26;"/>"#);
        assert_eq!(attrs[0].1, "a?x=1&n=$Number$");
        assert_eq!(attrs[1].1, "&");
        let attrs = first_open_attrs(r#"<X a="&lt;&gt;&amp;&quot;&apos;"/>"#);
        assert_eq!(attrs[0].1, "<>&\"'");
    }

    /// An undefined entity in an attribute value is a structured error (the
    /// hand-rolled tokenizer passed it through verbatim).
    #[test]
    fn undefined_entity_in_attribute_is_a_structured_error() {
        let mut reader = new_reader(r#"<X b="&nope;"/>"#);
        assert!(matches!(
            next_tag(&mut reader),
            Err(DashParseError::MalformedAttribute { .. })
        ));
    }

    /// Hostile input: unterminated constructs are structured errors, never a
    /// panic.
    #[test]
    fn unterminated_constructs_are_errors_not_panics() {
        for bad in [
            "<X",
            r#"<X a="unterminated"#,
            "<X a='1'",
            "<!-- never closed",
            "<![CDATA[ never",
            "<?pi",
        ] {
            let mut reader = new_reader(bad);
            let mut outcome = Ok(Some(()));
            for _ in 0..4 {
                outcome = next_tag(&mut reader).map(|t| t.map(|_| ()));
                if !matches!(outcome, Ok(Some(()))) {
                    break;
                }
            }
            assert!(
                matches!(outcome, Err(_) | Ok(None)),
                "{bad:?} must not panic"
            );
        }
        let mut reader = new_reader("<X");
        assert!(matches!(
            next_tag(&mut reader),
            Err(DashParseError::UnterminatedTag { .. })
        ));
    }

    /// A mismatched end tag is reported with both names.
    #[test]
    fn mismatched_end_tag_names_both_elements() {
        let mut reader = new_reader("<A><B></A>");
        let mut err = None;
        for _ in 0..4 {
            match next_tag(&mut reader) {
                Ok(Some(_)) => {}
                Ok(None) => break,
                Err(e) => {
                    err = Some(e);
                    break;
                }
            }
        }
        assert!(matches!(
            err,
            Some(DashParseError::MismatchedEndTag { ref expected, ref found })
                if expected == "B" && found == "A"
        ));
    }

    /// Element text resolves entities (named + numeric) and CDATA; the old
    /// tokenizer's 20k-entity linearity guard, ported: a long run of `&amp;`
    /// decodes in one pass.
    #[test]
    fn text_content_resolves_references_and_is_linear() {
        let xml = format!("<BaseURL>{}</BaseURL>", "&amp;".repeat(20_000));
        let mut reader = new_reader(&xml);
        assert!(matches!(next_tag(&mut reader), Ok(Some(Tag::Open { .. }))));
        let text = text_content(&mut reader, "BaseURL", false)
            .unwrap()
            .unwrap();
        assert_eq!(text, "&".repeat(20_000));

        let mut reader = new_reader("<B>a&#65;&#x42;&lt;<![CDATA[<&>]]></B>");
        next_tag(&mut reader).unwrap();
        assert_eq!(
            text_content(&mut reader, "B", false).unwrap().as_deref(),
            Some("aAB<<&>")
        );
    }

    /// An undefined entity or an out-of-range character reference in element
    /// text is an error, never a panic; nested markup is skipped.
    #[test]
    fn bad_references_and_nested_markup_in_text() {
        for bad in [
            "<B>&nope;</B>",
            "<B>&#0;</B>",
            "<B>&#xD800;</B>",
            "<B>&</B>",
        ] {
            let mut reader = new_reader(bad);
            next_tag(&mut reader).unwrap();
            assert!(text_content(&mut reader, "B", false).is_err(), "{bad}");
        }
        let mut reader = new_reader("<B>x<c>y</c></B><Next/>");
        next_tag(&mut reader).unwrap();
        assert_eq!(text_content(&mut reader, "B", false).unwrap(), None);
        assert!(matches!(
            next_tag(&mut reader),
            Ok(Some(Tag::Open { ref name, .. })) if name == "Next"
        ));
    }

    /// XML 1.0 §2.2: a control character is rejected, literally or through a
    /// character reference, in element text and in attribute values.
    #[test]
    fn control_characters_are_rejected() {
        for bad in [
            "<B>&#x1;</B>",
            "<B>&#1;</B>",
            "<B>\u{1}</B>",
            "<B>&#x0;</B>",
        ] {
            let mut reader = new_reader(bad);
            next_tag(&mut reader).unwrap();
            assert!(text_content(&mut reader, "B", false).is_err(), "{bad:?}");
        }
        for bad in [r#"<X a="&#x1;"/>"#, "<X a=\"\u{1}\"/>"] {
            let mut reader = new_reader(bad);
            assert!(
                matches!(
                    next_tag(&mut reader),
                    Err(DashParseError::MalformedAttribute { .. })
                ),
                "{bad:?}"
            );
        }
        let mut reader = new_reader("<B>a&#9;&#10;b</B>");
        next_tag(&mut reader).unwrap();
        assert_eq!(
            text_content(&mut reader, "B", false).unwrap().as_deref(),
            Some("a\t\nb")
        );
    }

    /// Character data inside an element the parser skips (or between elements)
    /// is validated exactly like modelled text.
    #[test]
    fn invalid_character_data_in_skipped_content_is_rejected() {
        for bad in ["&nope;", "&#1;", "&#x1;", "\u{1}", "<![CDATA[\u{1}]]>"] {
            let skipped =
                format!(r#"<MPD profiles="p"><Foo><Deep>{bad}</Deep></Foo><Period/></MPD>"#);
            assert!(Mpd::parse(&skipped).is_err(), "skipped subtree {bad:?}");
            let between = format!(r#"<MPD profiles="p">{bad}<Period/></MPD>"#);
            assert!(Mpd::parse(&between).is_err(), "between elements {bad:?}");
            let base = format!(r#"<MPD profiles="p"><BaseURL>a<x>{bad}</x></BaseURL></MPD>"#);
            assert!(Mpd::parse(&base).is_err(), "nested in BaseURL {bad:?}");
        }
    }

    /// `SPECIALS` survives an attribute round trip through the escaped form.
    #[test]
    fn specials_round_trip_through_attribute_escaping() {
        let xml = format!("<X a=\"{}\"/>", quick_xml::escape::escape(SPECIALS));
        assert_eq!(first_open_attrs(&xml)[0].1, SPECIALS);
    }

    // -- tokenizer / model -----------------------------------------------

    const SMALL_MPD: &str = r#"<?xml version="1.0" encoding="utf-8"?>
<MPD xmlns="urn:mpeg:dash:schema:mpd:2011" profiles="urn:mpeg:dash:profile:isoff-live:2011" type="static" mediaPresentationDuration="PT3.0S">
  <Period id="0" start="PT0.0S">
    <AdaptationSet contentType="video">
      <Representation id="v0" mimeType="video/mp4" codecs="avc1.4d400d" bandwidth="58141" width="320" height="240">
        <SegmentTemplate timescale="90000" initialization="init-stream$RepresentationID$.m4s" media="chunk-stream$RepresentationID$-$Number%05d$.m4s" startNumber="1">
          <SegmentTimeline>
            <S t="2070" d="90000" r="2" />
          </SegmentTimeline>
        </SegmentTemplate>
      </Representation>
    </AdaptationSet>
  </Period>
</MPD>"#;

    #[test]
    fn parses_small_mpd_structure() {
        let mpd = Mpd::parse(SMALL_MPD).expect("parse");
        assert_eq!(mpd.profiles, "urn:mpeg:dash:profile:isoff-live:2011");
        assert_eq!(mpd.mpd_type, MpdType::Static);
        assert_eq!(mpd.media_presentation_duration, Some(Duration::new(3, 0)));
        assert_eq!(mpd.periods.len(), 1);

        let period = &mpd.periods[0];
        assert_eq!(period.id.as_deref(), Some("0"));
        assert_eq!(period.start, Some(Duration::new(0, 0)));
        assert_eq!(period.adaptation_sets.len(), 1);

        let set = &period.adaptation_sets[0];
        assert_eq!(set.content_type.as_deref(), Some("video"));
        assert_eq!(set.representations.len(), 1);

        let repr = &set.representations[0];
        assert_eq!(repr.id, "v0");
        assert_eq!(repr.bandwidth, 58141);
        assert_eq!(repr.codecs.as_deref(), Some("avc1.4d400d"));
        assert_eq!(repr.width, Some(320));
        assert_eq!(repr.height, Some(240));

        let st = repr.segment_template.as_ref().expect("segment template");
        assert_eq!(st.timescale, 90000);
        assert_eq!(
            st.initialization.as_deref(),
            Some("init-stream$RepresentationID$.m4s")
        );
        assert_eq!(
            st.media.as_deref(),
            Some("chunk-stream$RepresentationID$-$Number%05d$.m4s")
        );
        assert_eq!(st.start_number, 1);

        let timeline = st.timeline.as_ref().expect("timeline");
        assert_eq!(
            timeline.segments,
            vec![S {
                t: Some(2070),
                d: 90000,
                r: 2
            }]
        );
    }

    #[test]
    fn dynamic_type_and_defaults() {
        let xml = r#"<MPD profiles="p" type="dynamic"><Period><AdaptationSet><Representation id="0" bandwidth="1"/></AdaptationSet></Period></MPD>"#;
        let mpd = Mpd::parse(xml).expect("parse");
        assert_eq!(mpd.mpd_type, MpdType::Dynamic);
        assert_eq!(mpd.media_presentation_duration, None);
        assert_eq!(mpd.periods[0].adaptation_sets[0].representations[0].id, "0");
        assert_eq!(
            mpd.periods[0].adaptation_sets[0].representations[0].bandwidth,
            1
        );
    }

    #[test]
    fn missing_type_defaults_to_static() {
        let xml = r#"<MPD profiles="p"><Period/></MPD>"#;
        let mpd = Mpd::parse(xml).expect("parse");
        assert_eq!(mpd.mpd_type, MpdType::Static);
        assert_eq!(mpd.periods.len(), 1);
        assert!(mpd.periods[0].adaptation_sets.is_empty());
    }

    #[test]
    fn segment_template_inherited_from_adaptation_set() {
        let xml = r#"<MPD profiles="p">
            <Period>
                <AdaptationSet contentType="video">
                    <SegmentTemplate timescale="1000" media="chunk-$Number$.m4s" startNumber="1"/>
                    <Representation id="0" bandwidth="1"/>
                    <Representation id="1" bandwidth="2">
                        <SegmentTemplate timescale="2000" media="own-$Number$.m4s" startNumber="5"/>
                    </Representation>
                </AdaptationSet>
            </Period>
        </MPD>"#;
        let mpd = Mpd::parse(xml).expect("parse");
        let set = &mpd.periods[0].adaptation_sets[0];
        assert_eq!(
            set.segment_template.as_ref().unwrap().timescale,
            1000,
            "AdaptationSet-level template retained"
        );

        let r0 = &set.representations[0];
        let r0_st = r0.segment_template.as_ref().expect("inherited template");
        assert_eq!(r0_st.timescale, 1000, "inherited from AdaptationSet");
        assert_eq!(r0_st.media.as_deref(), Some("chunk-$Number$.m4s"));

        let r1 = &set.representations[1];
        let r1_st = r1.segment_template.as_ref().expect("own template");
        assert_eq!(r1_st.timescale, 2000, "own template wins over inherited");
        assert_eq!(r1_st.start_number, 5);
    }

    #[test]
    fn segment_template_merges_attribute_by_attribute_across_three_levels() {
        // ISO/IEC 23009-1 §5.3.9.1: segment information is hierarchical, and a
        // lower level overrides only the attributes it declares. Structurally
        // this is the standard's Annex G.13 shape (Representation templates
        // carrying a single attribute, inheriting the rest), with the Period
        // level added on top (G.12's shape).
        let xml = r#"<MPD profiles="p">
            <Period>
                <SegmentTemplate timescale="90000" media="p-$Number$.m4s">
                    <SegmentTimeline><S t="0" d="90000" r="2"/></SegmentTimeline>
                </SegmentTemplate>
                <AdaptationSet contentType="video">
                    <SegmentTemplate startNumber="7"/>
                    <Representation id="0" bandwidth="1">
                        <SegmentTemplate media="$RepresentationID$-$Time$.m4s"/>
                    </Representation>
                    <Representation id="1" bandwidth="2"/>
                </AdaptationSet>
            </Period>
        </MPD>"#;
        let mpd = Mpd::parse(xml).expect("parse");
        let set = &mpd.periods[0].adaptation_sets[0];

        let set_st = set.segment_template.as_ref().expect("set template");
        assert_eq!(set_st.timescale, 90000, "inherited from Period");
        assert_eq!(set_st.start_number, 7, "declared by the AdaptationSet");
        assert_eq!(set_st.media.as_deref(), Some("p-$Number$.m4s"));
        assert!(set_st.timeline.is_some(), "timeline inherited from Period");

        let r0 = set.representations[0]
            .segment_template
            .as_ref()
            .expect("effective template");
        assert_eq!(r0.timescale, 90000, "inherited up the whole chain");
        assert_eq!(r0.start_number, 7);
        assert_eq!(
            r0.media.as_deref(),
            Some("$RepresentationID$-$Time$.m4s"),
            "the Representation's own @media wins"
        );
        assert!(
            r0.timeline.is_some(),
            "the Representation inherits the Period's timeline"
        );

        let r1 = set.representations[1]
            .segment_template
            .as_ref()
            .expect("effective template");
        assert_eq!(r1.media.as_deref(), Some("p-$Number$.m4s"));
        assert_eq!(r1.start_number, 7);
        assert_eq!(r1.timescale, 90000);
    }

    #[test]
    fn base_url_is_reported_per_level() {
        let xml = r#"<MPD profiles="p">
            <Period>
                <BaseURL>
                    https://cdn.example.com/vod/
                </BaseURL>
                <AdaptationSet contentType="video">
                    <Representation id="0" bandwidth="1">
                        <BaseURL>rep/</BaseURL>
                    </Representation>
                </AdaptationSet>
            </Period>
        </MPD>"#;
        let mpd = Mpd::parse(xml).expect("parse");
        let period = &mpd.periods[0];
        assert_eq!(
            period.base_url.as_deref(),
            Some("https://cdn.example.com/vod/"),
            "surrounding whitespace is trimmed"
        );
        assert!(period.adaptation_sets[0].base_url.is_none());
        assert_eq!(
            period.adaptation_sets[0].representations[0]
                .base_url
                .as_deref(),
            Some("rep/")
        );
    }

    #[test]
    fn first_non_empty_base_url_wins_per_level() {
        // §5.6.5: several BaseURL children of one element are alternates,
        // consulted in order — the first is the one used. An empty later one
        // must not clear it, and a leading empty one must not swallow the
        // expression either.
        let xml = r#"<MPD profiles="p">
            <BaseURL>https://mpd.example.com/</BaseURL>
            <Period>
                <BaseURL></BaseURL>
                <BaseURL>period-a/</BaseURL>
                <BaseURL>period-b/</BaseURL>
                <AdaptationSet contentType="video">
                    <Representation id="0" bandwidth="1"/>
                </AdaptationSet>
            </Period>
        </MPD>"#;
        let mpd = Mpd::parse(xml).expect("parse");
        assert_eq!(
            mpd.base_url.as_deref(),
            Some("https://mpd.example.com/"),
            "the MPD-level BaseURL is read"
        );
        assert_eq!(
            mpd.periods[0].base_url.as_deref(),
            Some("period-a/"),
            "the first non-empty sibling wins; an empty one is skipped and a              later one does not override"
        );
    }

    #[test]
    fn child_duration_over_parent_timeline_is_also_exclusive() {
        // The reverse of the case above: the AdaptationSet addresses by
        // `$Time$` (a SegmentTimeline), while the Representation declares
        // `@duration`. §5.3.9.4.4 forbids a template carrying both, so the
        // Representation's `@duration` means `$Number$` addressing and it must
        // NOT inherit the parent's timeline.
        let xml = r#"<MPD profiles="p">
            <Period>
                <AdaptationSet contentType="video">
                    <SegmentTemplate media="t-$Time$.m4s" timescale="90000">
                        <SegmentTimeline><S t="0" d="90000" r="2"/></SegmentTimeline>
                    </SegmentTemplate>
                    <Representation id="0" bandwidth="1">
                        <SegmentTemplate media="n-$Number$.m4s" duration="45000"/>
                    </Representation>
                </AdaptationSet>
            </Period>
        </MPD>"#;
        let mpd = Mpd::parse(xml).expect("parse");
        let set = &mpd.periods[0].adaptation_sets[0];
        let set_st = set.segment_template.as_ref().expect("set template");
        assert!(set_st.timeline.is_some(), "the set addresses by $Time$");
        assert_eq!(set_st.duration, None);

        let r = set.representations[0]
            .segment_template
            .as_ref()
            .expect("effective template");
        assert_eq!(
            r.duration,
            Some(45000),
            "the Representation's own @duration is kept"
        );
        assert!(
            r.timeline.is_none(),
            "a template carrying @duration must not also inherit the parent's              SegmentTimeline; the two addressing modes are exclusive"
        );
        // Everything else still inherits.
        assert_eq!(r.timescale, 90000);
        assert_eq!(r.media.as_deref(), Some("n-$Number$.m4s"));
    }

    #[test]
    fn segment_timeline_and_duration_are_mutually_exclusive() {
        // §5.3.9.4.4: "@duration ... or a SegmentTimeline ... but not both".
        // A Representation that introduces a SegmentTimeline while the
        // AdaptationSet declared @duration must not end up with both.
        let xml = r#"<MPD profiles="p">
            <Period>
                <AdaptationSet contentType="video">
                    <SegmentTemplate media="c-$Number$.m4s" duration="90000" timescale="90000"/>
                    <Representation id="0" bandwidth="1">
                        <SegmentTemplate media="t-$Time$.m4s">
                            <SegmentTimeline><S t="0" d="90000" r="2"/></SegmentTimeline>
                        </SegmentTemplate>
                    </Representation>
                </AdaptationSet>
            </Period>
        </MPD>"#;
        let mpd = Mpd::parse(xml).expect("parse");
        let set = &mpd.periods[0].adaptation_sets[0];
        let set_st = set.segment_template.as_ref().expect("set template");
        assert_eq!(set_st.duration, Some(90000), "the set's own @duration");
        assert!(set_st.timeline.is_none());

        let r = set.representations[0].segment_template.as_ref().unwrap();
        assert!(r.timeline.is_some(), "the child's timeline");
        assert_eq!(
            r.duration, None,
            "a template carrying a SegmentTimeline must not also carry the              parent's @duration; the two addressing modes are exclusive"
        );
        // The rest still inherits.
        assert_eq!(r.timescale, 90000);
    }

    #[test]
    fn representation_without_any_template_is_none() {
        let xml = r#"<MPD profiles="p"><Period><AdaptationSet><Representation id="0" bandwidth="1"/></AdaptationSet></Period></MPD>"#;
        let mpd = Mpd::parse(xml).expect("parse");
        assert!(
            mpd.periods[0].adaptation_sets[0].representations[0]
                .segment_template
                .is_none()
        );
    }

    #[test]
    fn tolerates_unknown_elements() {
        let xml = r#"<MPD profiles="p">
            <ProgramInformation></ProgramInformation>
            <ServiceDescription id="0"></ServiceDescription>
            <Period>
                <AdaptationSet>
                    <Role schemeIdUri="urn:mpeg:dash:role:2011" value="main"/>
                    <Representation id="0" bandwidth="1">
                        <AudioChannelConfiguration schemeIdUri="x" value="2"/>
                    </Representation>
                </AdaptationSet>
            </Period>
        </MPD>"#;
        let mpd = Mpd::parse(xml).expect("parse should tolerate unmodeled elements");
        assert_eq!(mpd.periods.len(), 1);
        assert_eq!(mpd.periods[0].adaptation_sets[0].representations[0].id, "0");
    }

    #[test]
    fn entity_unescape_in_attribute_values() {
        let xml = r#"<MPD profiles="a &amp; b &lt;x&gt;"><Period/></MPD>"#;
        let mpd = Mpd::parse(xml).expect("parse");
        assert_eq!(mpd.profiles, "a & b <x>");
    }

    // -- malformed / truncated input: must error, never panic ------------

    #[test]
    fn unterminated_tag_is_error_not_panic() {
        let err = Mpd::parse("<MPD profiles=\"p\"").unwrap_err();
        assert!(matches!(
            err,
            DashParseError::UnterminatedTag { .. } | DashParseError::UnexpectedEof
        ));
    }

    #[test]
    fn unclosed_attribute_quote_is_error_not_panic() {
        let err = Mpd::parse(r#"<MPD profiles="p type="static"><Period/></MPD>"#).unwrap_err();
        // Whatever the specific classification, it must be a structured
        // error, not a panic (the test itself not panicking is the proof).
        let _ = err;
    }

    #[test]
    fn truncated_after_declaration_is_error() {
        let err = Mpd::parse("<?xml version=\"1.0\"?>").unwrap_err();
        assert_eq!(err, DashParseError::UnexpectedEof);
    }

    #[test]
    fn wrong_root_element_is_error() {
        let err = Mpd::parse("<NotAnMpd/>").unwrap_err();
        assert!(matches!(err, DashParseError::UnexpectedElement { .. }));
    }

    #[test]
    fn missing_required_attribute_is_error() {
        // Representation without @bandwidth.
        let xml = r#"<MPD profiles="p"><Period><AdaptationSet><Representation id="0"/></AdaptationSet></Period></MPD>"#;
        let err = Mpd::parse(xml).unwrap_err();
        assert!(matches!(
            err,
            DashParseError::MissingAttribute {
                element: "Representation",
                attr: "bandwidth"
            }
        ));
    }

    #[test]
    fn empty_input_is_error() {
        let err = Mpd::parse("").unwrap_err();
        assert_eq!(err, DashParseError::UnexpectedEof);
    }

    // -- xs:duration -------------------------------------------------------

    #[test]
    fn iso8601_duration_hours_minutes_fractional_seconds() {
        assert_eq!(
            parse_iso8601_duration("PT1H2M3.5S").unwrap(),
            Duration::new(3723, 500_000_000)
        );
    }

    #[test]
    fn iso8601_duration_seconds_only() {
        assert_eq!(parse_iso8601_duration("PT4S").unwrap(), Duration::new(4, 0));
    }

    #[test]
    fn iso8601_duration_zero() {
        assert_eq!(parse_iso8601_duration("PT0S").unwrap(), Duration::new(0, 0));
    }

    #[test]
    fn iso8601_duration_days_and_hours() {
        assert_eq!(
            parse_iso8601_duration("P1DT2H").unwrap(),
            Duration::new(SECONDS_PER_DAY + 2 * SECONDS_PER_HOUR, 0)
        );
    }

    #[test]
    fn iso8601_duration_writer_tenths_form() {
        // The exact form `DashPackager`'s writer emits (xs_duration_tenths).
        assert_eq!(
            parse_iso8601_duration("PT2.0S").unwrap(),
            Duration::new(2, 0)
        );
    }

    #[test]
    fn iso8601_duration_rejects_bare_p() {
        assert!(parse_iso8601_duration("P").is_err());
    }

    #[test]
    fn iso8601_duration_rejects_missing_prefix() {
        assert!(parse_iso8601_duration("1H2M3S").is_err());
    }

    #[test]
    fn iso8601_duration_rejects_calendar_months() {
        // `nM` in the date part (calendar months) is deliberately unsupported.
        assert!(parse_iso8601_duration("P1M").is_err());
    }

    // -- template resolution ------------------------------------------------

    #[test]
    fn resolve_representation_id() {
        assert_eq!(
            SegmentTemplate::resolve("init-$RepresentationID$.m4s", "7", None, None, None),
            "init-7.m4s"
        );
    }

    #[test]
    fn resolve_number_with_width() {
        assert_eq!(
            SegmentTemplate::resolve(
                "chunk-$RepresentationID$-$Number%05d$.m4s",
                "0",
                Some(42),
                None,
                None
            ),
            "chunk-0-00042.m4s"
        );
    }

    #[test]
    fn resolve_number_without_width() {
        assert_eq!(
            SegmentTemplate::resolve("chunk-$Number$.m4s", "0", Some(42), None, None),
            "chunk-42.m4s"
        );
    }

    #[test]
    fn resolve_time() {
        assert_eq!(
            SegmentTemplate::resolve("chunk-$Time$.m4s", "0", None, Some(2070), None),
            "chunk-2070.m4s"
        );
    }

    #[test]
    fn resolve_bandwidth_with_width() {
        assert_eq!(
            SegmentTemplate::resolve("seg-$Bandwidth%08d$.m4s", "0", None, None, Some(58141)),
            "seg-00058141.m4s"
        );
    }

    #[test]
    fn resolve_dollar_escape() {
        assert_eq!(
            SegmentTemplate::resolve("literal-$$-$Number$", "0", Some(1), None, None),
            "literal-$-1"
        );
    }

    #[test]
    fn resolve_missing_value_emitted_literally() {
        assert_eq!(
            SegmentTemplate::resolve("chunk-$Time$.m4s", "0", Some(1), None, None),
            "chunk-$Time$.m4s"
        );
    }

    #[test]
    fn resolve_unknown_identifier_passthrough() {
        assert_eq!(
            SegmentTemplate::resolve("$Unknown$-x", "0", None, None, None),
            "$Unknown$-x"
        );
    }

    #[test]
    fn number_sequence_from_start_number() {
        let st = SegmentTemplate {
            timescale: 1,
            initialization: None,
            media: None,
            start_number: 5,
            duration: Some(1000),
            presentation_time_offset: 0,
            timeline: None,
        };
        assert_eq!(st.number_sequence(3), vec![5, 6, 7]);
    }

    // -- SegmentTimeline enumeration -----------------------------------------

    #[test]
    fn enumerate_expands_repeats() {
        // Matches the real fixture's video SegmentTemplate: one S with r=2
        // (three total segments of duration 90000, starting at t=2070).
        let timeline = SegmentTimeline {
            segments: vec![S {
                t: Some(2070),
                d: 90000,
                r: 2,
            }],
        };
        assert_eq!(
            timeline.enumerate(1).expect("enumerate"),
            vec![(1, 2070), (2, 92070), (3, 182070)]
        );
    }

    #[test]
    fn enumerate_multiple_s_entries_accumulate_time() {
        // Matches the real fixture's audio SegmentTemplate: four distinct
        // durations, only the first carrying an explicit @t.
        let timeline = SegmentTimeline {
            segments: vec![
                S {
                    t: Some(0),
                    d: 41984,
                    r: 0,
                },
                S {
                    t: None,
                    d: 44032,
                    r: 0,
                },
                S {
                    t: None,
                    d: 45056,
                    r: 0,
                },
                S {
                    t: None,
                    d: 3072,
                    r: 0,
                },
            ],
        };
        assert_eq!(
            timeline.enumerate(1).expect("enumerate"),
            vec![(1, 0), (2, 41984), (3, 86016), (4, 131072)]
        );
    }

    #[test]
    fn enumerate_negative_r_tolerated_as_single_segment() {
        let timeline = SegmentTimeline {
            segments: vec![S {
                t: Some(0),
                d: 1000,
                r: -1,
            }],
        };
        assert_eq!(timeline.enumerate(1).expect("enumerate"), vec![(1, 0)]);
    }

    /// Gap 4 fast unit test (#738): format_width must clamp to MAX_FORMAT_WIDTH
    /// instantly, not attempt huge allocations. Test via the public
    /// SegmentTemplate::resolve API with a hostile width and verify the result
    /// is small.
    #[test]
    fn segment_template_resolve_clamps_format_width_instantly() {
        // A hostile template with an impossibly large format width. The
        // resolve must clamp to MAX_FORMAT_WIDTH (20) and return immediately
        // without allocation or looping.
        let resolved =
            SegmentTemplate::resolve("chunk-$Number%9999999999d$.m4s", "r0", Some(42), None, None);

        // The number 42 zero-padded to at most 20 digits is "00000000000000000042".
        // The full resolved string should be small.
        assert!(
            resolved.len() < 100,
            "resolved string must be small (clamped width): {resolved}"
        );
        assert!(
            resolved.contains("42"),
            "number must appear in resolved string: {resolved}"
        );
        assert_eq!(
            resolved, "chunk-00000000000000000042.m4s",
            "exact padding to 20 digits (MAX_FORMAT_WIDTH)"
        );
    }
}
