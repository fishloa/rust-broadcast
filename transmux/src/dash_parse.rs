//! DASH `.mpd` Media Presentation Description **parser** — ISO/IEC 23009-1 —
//! the structural inverse of [`crate::dash`]'s `DashPackager` writer.
//!
//! `transmux` demuxes remote DASH presentations (issue #758, DASH-pull
//! ingest) by first fetching an MPD and resolving the segment URLs it
//! describes; this module is the piece that reads that MPD text. Like the
//! writer, it is dependency-free: a small hand-rolled XML tokenizer
//! (`no_std` `alloc`, no external XML crate), scoped to exactly the MPD
//! subset the writer emits plus what real-world isoff-live MPDs use.
//!
//! # Structure parsed (ISO/IEC 23009-1:2014)
//!
//! - **MPD** (§5.3.1) — [`Mpd`]: `profiles`, `type` ([`MpdType`]),
//!   `mediaPresentationDuration`/`minimumUpdatePeriod`/
//!   `availabilityStartTime`/`timeShiftBufferDepth` (§5.3.1.2 Table 3), one or
//!   more `Period`.
//! - **Period** (§5.3.2) — [`Period`]: `id`, `start`, `duration`, its
//!   `AdaptationSet`s.
//! - **`AdaptationSet`** (§5.3.3) — [`AdaptationSet`]: `mimeType`,
//!   `contentType`, an optional set-level `SegmentTemplate`, its
//!   `Representation`s.
//! - **`Representation`** (§5.3.5) — [`Representation`]: `id`, `bandwidth`,
//!   `codecs`, geometry/audio attributes, its own `SegmentTemplate` (falling
//!   back to the `AdaptationSet`'s — see [`Mpd::parse`]'s inheritance note).
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

use crate::xml_parse::{XmlError, XmlEvent, XmlTokenizer, skip_element};

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
        expected: &'static str,
        /// The element name actually found in the closing tag.
        found: String,
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
        }
    }
}

#[cfg(feature = "std")]
impl std::error::Error for DashParseError {}

impl From<XmlError> for DashParseError {
    fn from(err: XmlError) -> Self {
        match err {
            XmlError::UnexpectedEof => DashParseError::UnexpectedEof,
            XmlError::UnterminatedTag { pos } => DashParseError::UnterminatedTag { pos },
            XmlError::MalformedAttribute { pos } => DashParseError::MalformedAttribute { pos },
            XmlError::MismatchedEndTag { expected, found } => {
                DashParseError::MismatchedEndTag { expected, found }
            }
        }
    }
}

/// Crate-local result alias for this module.
type Result<T> = core::result::Result<T, DashParseError>;

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
    /// The AdaptationSet-level `SegmentTemplate`, if declared directly here
    /// (§5.3.9.1 — SegmentTemplate is inheritable down to `Representation`;
    /// see [`Mpd::parse`]'s inheritance note for how that's resolved onto
    /// each [`Representation::segment_template`]).
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
    /// This Representation's effective `SegmentTemplate`: its own child
    /// element if present, else its `AdaptationSet`'s (see [`Mpd::parse`]'s
    /// inheritance note). `None` if neither declared one — e.g. a
    /// Representation addressed only by `SegmentList`/`SegmentBase`, which
    /// this v1 parser does not resolve (tolerated, not an error: see the
    /// module docs).
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

/// Seconds in a day, for the `nD` component of an `xs:duration`.
const SECONDS_PER_DAY: u64 = 24 * 60 * 60;
/// Seconds in an hour, for the `nH` component.
const SECONDS_PER_HOUR: u64 = 60 * 60;
/// Seconds in a minute, for the `nM` (time-part) component.
const SECONDS_PER_MINUTE: u64 = 60;
/// `Duration`'s subsecond field is nanoseconds; a fractional-seconds string is
/// padded/truncated to this many digits.
const NANOSECOND_DIGITS: usize = 9;

/// Parse an `xs:duration` string (W3C XML Schema Part 2 §3.2.6, as used by
/// every duration-valued MPD attribute — §5.3.1.2/§5.3.2.2/§5.3.9.2.2): the
/// `PnDTnHnMnS` form, e.g. `PT1H2M3.5S`, `PT4S`, `PT0S`, `P1DT2H`.
///
/// Only a plain day count (`nD`) is supported for the date part — calendar
/// `nY`/`nM` (years/months) are ambiguous without a reference date and are
/// not used by [`crate::dash::DashPackager`]'s writer or any DASH profile
/// this crate targets; such input is rejected with [`DashParseError::InvalidDuration`]
/// rather than guessed at.
pub fn parse_iso8601_duration(s: &str) -> Result<Duration> {
    let trimmed = s.trim();
    let invalid = || DashParseError::InvalidDuration {
        value: trimmed.to_string(),
    };
    let rest = trimmed.strip_prefix('P').ok_or_else(invalid)?;
    let (date_part, time_part) = match rest.find('T') {
        Some(idx) => (&rest[..idx], Some(&rest[idx + 1..])),
        None => (rest, None),
    };
    if date_part.is_empty() && time_part.is_none() {
        return Err(invalid()); // bare "P"
    }

    let mut total_secs: u64 = 0;

    if !date_part.is_empty() {
        let days_str = date_part.strip_suffix('D').ok_or_else(invalid)?;
        let days: u64 = days_str.parse().map_err(|_| invalid())?;
        total_secs = total_secs.saturating_add(days.saturating_mul(SECONDS_PER_DAY));
    }

    let mut nanos: u32 = 0;

    if let Some(time_part) = time_part {
        if time_part.is_empty() {
            return Err(invalid()); // bare "T" with nothing after it
        }
        let mut remaining = time_part;
        if let Some(idx) = remaining.find('H') {
            let n: u64 = remaining[..idx].parse().map_err(|_| invalid())?;
            total_secs = total_secs.saturating_add(n.saturating_mul(SECONDS_PER_HOUR));
            remaining = &remaining[idx + 1..];
        }
        if let Some(idx) = remaining.find('M') {
            let n: u64 = remaining[..idx].parse().map_err(|_| invalid())?;
            total_secs = total_secs.saturating_add(n.saturating_mul(SECONDS_PER_MINUTE));
            remaining = &remaining[idx + 1..];
        }
        if let Some(idx) = remaining.find('S') {
            let secs_str = &remaining[..idx];
            let (whole, frac) = match secs_str.split_once('.') {
                Some((w, f)) => (w, Some(f)),
                None => (secs_str, None),
            };
            let whole_secs: u64 = if whole.is_empty() {
                0
            } else {
                whole.parse().map_err(|_| invalid())?
            };
            total_secs = total_secs.saturating_add(whole_secs);
            if let Some(frac) = frac {
                nanos = parse_fraction_nanos(frac).map_err(|_| invalid())?;
            }
            remaining = &remaining[idx + 1..];
        }
        if !remaining.is_empty() {
            return Err(invalid()); // trailing garbage after S/M/H
        }
    }

    Ok(Duration::new(total_secs, nanos))
}

/// Parse a fractional-seconds digit string (`"5"`, `"500"`, …) into
/// nanoseconds, padding/truncating to [`NANOSECOND_DIGITS`] digits.
fn parse_fraction_nanos(frac: &str) -> core::result::Result<u32, ()> {
    if frac.is_empty() || !frac.bytes().all(|b| b.is_ascii_digit()) {
        return Err(());
    }
    let mut digits = String::with_capacity(NANOSECOND_DIGITS);
    digits.push_str(frac);
    while digits.len() < NANOSECOND_DIGITS {
        digits.push('0');
    }
    digits.truncate(NANOSECOND_DIGITS);
    digits.parse::<u32>().map_err(|_| ())
}

// ---------------------------------------------------------------------------
// DASH-specific attribute helpers (XML parsing is in xml_parse module)
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
    /// Parse an MPD document (ISO/IEC 23009-1 §5.3) into this structural
    /// model — the inverse of [`crate::dash::DashPackager`]'s rendered XML
    /// output.
    ///
    /// # `SegmentTemplate` inheritance
    ///
    /// `SegmentTemplate` is an inheritable property along the
    /// `Period` > `AdaptationSet` > `Representation` chain (§5.3.9.1). This
    /// parser resolves that inheritance eagerly: after parsing each
    /// `AdaptationSet`, every `Representation` whose own `segment_template`
    /// is `None` is given a clone of the `AdaptationSet`-level one (if any) —
    /// so [`Representation::segment_template`] is always the *effective*
    /// template a caller should use, regardless of which element actually
    /// declared it in the source XML.
    pub fn parse(xml: &str) -> Result<Mpd> {
        const EL: &str = "MPD";
        let mut tok = XmlTokenizer::new(xml);

        let (mpd_attrs, mpd_self_closing) = match tok.next_event()? {
            Some(XmlEvent::Start {
                name: "MPD",
                attrs,
                self_closing,
            }) => (attrs, self_closing),
            Some(XmlEvent::Start { name, .. }) => {
                return Err(DashParseError::UnexpectedElement {
                    expected: EL,
                    found: name.to_string(),
                });
            }
            Some(XmlEvent::End { .. }) => {
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

        let mut periods = Vec::new();
        if !mpd_self_closing {
            loop {
                match tok.next_event()? {
                    Some(XmlEvent::Start {
                        name: "Period",
                        attrs,
                        self_closing,
                    }) => periods.push(parse_period(&mut tok, attrs, self_closing)?),
                    Some(XmlEvent::Start { self_closing, .. }) => {
                        if !self_closing {
                            skip_element(&mut tok)?;
                        }
                    }
                    Some(XmlEvent::End { name }) => {
                        if name != EL {
                            return Err(DashParseError::MismatchedEndTag {
                                expected: EL,
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
            periods,
        })
    }
}

fn parse_period(
    tok: &mut XmlTokenizer<'_>,
    attrs: Vec<(String, String)>,
    self_closing: bool,
) -> Result<Period> {
    const EL: &str = "Period";
    let id = attr_owned(&attrs, "id");
    let start = parse_duration_attr(&attrs, "start")?;
    let duration = parse_duration_attr(&attrs, "duration")?;

    let mut adaptation_sets = Vec::new();
    if !self_closing {
        loop {
            match tok.next_event()? {
                Some(XmlEvent::Start {
                    name: "AdaptationSet",
                    attrs,
                    self_closing,
                }) => adaptation_sets.push(parse_adaptation_set(tok, attrs, self_closing)?),
                Some(XmlEvent::Start { self_closing, .. }) => {
                    if !self_closing {
                        skip_element(tok)?;
                    }
                }
                Some(XmlEvent::End { name }) => {
                    if name != EL {
                        return Err(DashParseError::MismatchedEndTag {
                            expected: EL,
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
        adaptation_sets,
    })
}

fn parse_adaptation_set(
    tok: &mut XmlTokenizer<'_>,
    attrs: Vec<(String, String)>,
    self_closing: bool,
) -> Result<AdaptationSet> {
    const EL: &str = "AdaptationSet";
    let mime_type = attr_owned(&attrs, "mimeType");
    let content_type = attr_owned(&attrs, "contentType");

    let mut segment_template = None;
    let mut representations = Vec::new();
    if !self_closing {
        loop {
            match tok.next_event()? {
                Some(XmlEvent::Start {
                    name: "SegmentTemplate",
                    attrs,
                    self_closing,
                }) => segment_template = Some(parse_segment_template(tok, attrs, self_closing)?),
                Some(XmlEvent::Start {
                    name: "Representation",
                    attrs,
                    self_closing,
                }) => representations.push(parse_representation(tok, attrs, self_closing)?),
                Some(XmlEvent::Start { self_closing, .. }) => {
                    if !self_closing {
                        skip_element(tok)?;
                    }
                }
                Some(XmlEvent::End { name }) => {
                    if name != EL {
                        return Err(DashParseError::MismatchedEndTag {
                            expected: EL,
                            found: name.to_string(),
                        });
                    }
                    break;
                }
                None => return Err(DashParseError::UnexpectedEof),
            }
        }
    }

    // Inherit the AdaptationSet-level SegmentTemplate onto any Representation
    // that didn't declare its own (see `Mpd::parse`'s inheritance note).
    if let Some(inherited) = &segment_template {
        for r in &mut representations {
            if r.segment_template.is_none() {
                r.segment_template = Some(inherited.clone());
            }
        }
    }

    Ok(AdaptationSet {
        mime_type,
        content_type,
        segment_template,
        representations,
    })
}

fn parse_representation(
    tok: &mut XmlTokenizer<'_>,
    attrs: Vec<(String, String)>,
    self_closing: bool,
) -> Result<Representation> {
    const EL: &str = "Representation";
    let id = required_attr_owned(&attrs, "id", EL)?;
    let bandwidth: u64 = required_attr_parse(&attrs, "bandwidth", EL)?;
    let codecs = attr_owned(&attrs, "codecs");
    let mime_type = attr_owned(&attrs, "mimeType");
    let width: Option<u32> = parse_attr(&attrs, "width", EL)?;
    let height: Option<u32> = parse_attr(&attrs, "height", EL)?;
    let frame_rate = attr_owned(&attrs, "frameRate");
    let audio_sampling_rate: Option<u32> = parse_attr(&attrs, "audioSamplingRate", EL)?;

    let mut segment_template = None;
    if !self_closing {
        loop {
            match tok.next_event()? {
                Some(XmlEvent::Start {
                    name: "SegmentTemplate",
                    attrs,
                    self_closing,
                }) => segment_template = Some(parse_segment_template(tok, attrs, self_closing)?),
                Some(XmlEvent::Start { self_closing, .. }) => {
                    if !self_closing {
                        skip_element(tok)?;
                    }
                }
                Some(XmlEvent::End { name }) => {
                    if name != EL {
                        return Err(DashParseError::MismatchedEndTag {
                            expected: EL,
                            found: name.to_string(),
                        });
                    }
                    break;
                }
                None => return Err(DashParseError::UnexpectedEof),
            }
        }
    }

    Ok(Representation {
        id,
        bandwidth,
        codecs,
        width,
        height,
        frame_rate,
        audio_sampling_rate,
        mime_type,
        segment_template,
    })
}

fn parse_segment_template(
    tok: &mut XmlTokenizer<'_>,
    attrs: Vec<(String, String)>,
    self_closing: bool,
) -> Result<SegmentTemplate> {
    const EL: &str = "SegmentTemplate";
    let timescale: u64 = parse_attr(&attrs, "timescale", EL)?.unwrap_or(DEFAULT_TIMESCALE);
    let initialization = attr_owned(&attrs, "initialization");
    let media = attr_owned(&attrs, "media");
    let start_number: u64 = parse_attr(&attrs, "startNumber", EL)?.unwrap_or(DEFAULT_START_NUMBER);
    let duration: Option<u64> = parse_attr(&attrs, "duration", EL)?;
    let presentation_time_offset: u64 = parse_attr(&attrs, "presentationTimeOffset", EL)?
        .unwrap_or(DEFAULT_PRESENTATION_TIME_OFFSET);

    let mut timeline = None;
    if !self_closing {
        loop {
            match tok.next_event()? {
                Some(XmlEvent::Start {
                    name: "SegmentTimeline",
                    self_closing,
                    ..
                }) => timeline = Some(parse_segment_timeline(tok, self_closing)?),
                Some(XmlEvent::Start { self_closing, .. }) => {
                    if !self_closing {
                        skip_element(tok)?;
                    }
                }
                Some(XmlEvent::End { name }) => {
                    if name != EL {
                        return Err(DashParseError::MismatchedEndTag {
                            expected: EL,
                            found: name.to_string(),
                        });
                    }
                    break;
                }
                None => return Err(DashParseError::UnexpectedEof),
            }
        }
    }

    Ok(SegmentTemplate {
        timescale,
        initialization,
        media,
        start_number,
        duration,
        presentation_time_offset,
        timeline,
    })
}

fn parse_segment_timeline(
    tok: &mut XmlTokenizer<'_>,
    self_closing: bool,
) -> Result<SegmentTimeline> {
    const EL: &str = "SegmentTimeline";
    let mut segments = Vec::new();
    if !self_closing {
        loop {
            match tok.next_event()? {
                Some(XmlEvent::Start {
                    name: "S",
                    attrs,
                    self_closing,
                }) => {
                    let t: Option<u64> = parse_attr(&attrs, "t", "S")?;
                    let d: u64 = required_attr_parse(&attrs, "d", "S")?;
                    let r: i64 = parse_attr(&attrs, "r", "S")?.unwrap_or(DEFAULT_REPEAT);
                    segments.push(S { t, d, r });
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
                        return Err(DashParseError::MismatchedEndTag {
                            expected: EL,
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
}
