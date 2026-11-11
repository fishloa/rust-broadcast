//! `SegmentTemplate` identifier scanning and substitution (ISO/IEC 23009-1
//! 5.3.9.4.4, Table 21).
//!
//! The scan is strict: a `$` that does not enclose a known identifier makes
//! the whole template invalid, which the spec resolves by dropping the
//! containing Representation (5.3.9.4.4 step 2). Format tags are `%0<width>d`
//! only. Substitution itself is `transmux::SegmentTemplate::resolve`, so the
//! two crates cannot disagree about widths or `$$`.

use broadcast_common::impl_spec_display;
use transmux::dash_parse::MAX_FORMAT_WIDTH;

const IDENT_RESERVED: &str = "RepresentationID";
const IDENT_NUMBER: &str = "Number";
const IDENT_TIME: &str = "Time";
const IDENT_BANDWIDTH: &str = "Bandwidth";
const IDENT_SUB_NUMBER: &str = "SubNumber";
const TAG_ZERO_FLAG: char = '0';
const TAG_CONVERSION: char = 'd';
const DELIMITER: char = '$';
const TAG_START: char = '%';

/// Which identifiers a template contains.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[non_exhaustive]
pub struct TemplateUse {
    /// `$RepresentationID$`.
    pub representation_id: bool,
    /// `$Number$`.
    pub number: bool,
    /// `$Time$`.
    pub time: bool,
    /// `$Bandwidth$`.
    pub bandwidth: bool,
    /// `$SubNumber$` (segment sequences; unsupported).
    pub sub_number: bool,
}

/// Why a template is invalid.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum TemplateFault {
    /// A `$` with no closing `$`.
    UnterminatedIdentifier,
    /// `$name$` where `name` is not in Table 21.
    UnknownIdentifier,
    /// A format tag on `$RepresentationID$`, which takes none.
    FormatOnRepresentationId,
    /// A format tag that is not `%0<width>d`.
    BadFormatTag,
    /// A width above the supported maximum.
    WidthTooLarge,
    /// `$Number$` and `$Time$` in one template.
    NumberAndTime,
    /// A media template with neither `$Number$` nor `$Time$`.
    MissingNumberOrTime,
    /// An `@initialization` template containing `$Number$` or `$Time$`.
    DynamicIdentifierInInitialization,
}

impl TemplateFault {
    /// Stable snake_case label.
    #[must_use]
    pub fn name(&self) -> &'static str {
        match self {
            Self::UnterminatedIdentifier => "unterminated_identifier",
            Self::UnknownIdentifier => "unknown_identifier",
            Self::FormatOnRepresentationId => "format_on_representation_id",
            Self::BadFormatTag => "bad_format_tag",
            Self::WidthTooLarge => "width_too_large",
            Self::NumberAndTime => "number_and_time",
            Self::MissingNumberOrTime => "missing_number_or_time",
            Self::DynamicIdentifierInInitialization => "dynamic_identifier_in_initialization",
        }
    }
}
impl_spec_display!(TemplateFault);

fn check_tag(tag: &str) -> Result<(), TemplateFault> {
    let inner = tag
        .strip_suffix(TAG_CONVERSION)
        .ok_or(TemplateFault::BadFormatTag)?;
    let digits = inner
        .strip_prefix(TAG_ZERO_FLAG)
        .ok_or(TemplateFault::BadFormatTag)?;
    if !digits.bytes().all(|b| b.is_ascii_digit()) {
        return Err(TemplateFault::BadFormatTag);
    }
    if !digits.is_empty() {
        let width: usize = digits.parse().map_err(|_| TemplateFault::WidthTooLarge)?;
        if width > MAX_FORMAT_WIDTH {
            return Err(TemplateFault::WidthTooLarge);
        }
    }
    Ok(())
}

/// Scan `template` and report the identifiers it uses.
///
/// # Errors
/// [`TemplateFault`] for anything 5.3.9.4.4 leaves undefined.
pub fn scan(template: &str) -> Result<TemplateUse, TemplateFault> {
    let mut used = TemplateUse::default();
    let mut rest = template;
    while let Some(at) = rest.find(DELIMITER) {
        let tail = &rest[at + 1..];
        if let Some(after) = tail.strip_prefix(DELIMITER) {
            rest = after;
            continue;
        }
        let end = tail
            .find(DELIMITER)
            .ok_or(TemplateFault::UnterminatedIdentifier)?;
        let ident = &tail[..end];
        let (name, tag) = match ident.split_once(TAG_START) {
            Some((n, t)) => (n, Some(format!("{TAG_START}{t}"))),
            None => (ident, None),
        };
        match name {
            IDENT_RESERVED => {
                if tag.is_some() {
                    return Err(TemplateFault::FormatOnRepresentationId);
                }
                used.representation_id = true;
            }
            IDENT_NUMBER | IDENT_TIME | IDENT_BANDWIDTH | IDENT_SUB_NUMBER => {
                if let Some(t) = &tag {
                    check_tag(&t[TAG_START.len_utf8()..])?;
                }
                match name {
                    IDENT_NUMBER => used.number = true,
                    IDENT_TIME => used.time = true,
                    IDENT_BANDWIDTH => used.bandwidth = true,
                    _ => used.sub_number = true,
                }
            }
            _ => return Err(TemplateFault::UnknownIdentifier),
        }
        rest = &tail[end + 1..];
    }
    Ok(used)
}

/// Validate a media/index template: exactly one of `$Number$`/`$Time$`.
///
/// # Errors
/// [`TemplateFault`].
pub fn scan_media(template: &str) -> Result<TemplateUse, TemplateFault> {
    let used = scan(template)?;
    if used.number && used.time {
        return Err(TemplateFault::NumberAndTime);
    }
    if !used.number && !used.time {
        return Err(TemplateFault::MissingNumberOrTime);
    }
    Ok(used)
}

/// Validate an initialization template: neither `$Number$` nor `$Time$`.
///
/// # Errors
/// [`TemplateFault`].
pub fn scan_initialization(template: &str) -> Result<TemplateUse, TemplateFault> {
    let used = scan(template)?;
    if used.number || used.time {
        return Err(TemplateFault::DynamicIdentifierInInitialization);
    }
    Ok(used)
}

/// Substitute the identifiers of an already scanned template.
#[must_use]
pub fn substitute(
    template: &str,
    representation_id: &str,
    number: Option<u64>,
    time: Option<u64>,
    bandwidth: u64,
) -> String {
    transmux::SegmentTemplate::resolve(template, representation_id, number, time, Some(bandwidth))
}

/// A template split into literals and Table 21 identifiers, with each
/// identifier's format width.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Part<'a> {
    Literal(&'a str),
    Escape,
    RepresentationId,
    Number(Option<usize>),
    Time(Option<usize>),
    Bandwidth(Option<usize>),
}

fn number_width(ident: &str) -> Option<usize> {
    let (_, digits) = ident.split_once(TAG_START)?;
    let digits = digits.strip_suffix(TAG_CONVERSION).unwrap_or(digits);
    digits.parse().ok().map(|w: usize| w.min(MAX_FORMAT_WIDTH))
}

fn parts(template: &str) -> Result<Vec<Part<'_>>, TemplateFault> {
    scan(template)?;
    let mut out: Vec<Part<'_>> = Vec::new();
    let mut rest = template;
    while let Some(at) = rest.find(DELIMITER) {
        if at > 0 {
            out.push(Part::Literal(&rest[..at]));
        }
        let tail = &rest[at + 1..];
        if let Some(after) = tail.strip_prefix(DELIMITER) {
            out.push(Part::Escape);
            rest = after;
            continue;
        }
        let end = tail
            .find(DELIMITER)
            .ok_or(TemplateFault::UnterminatedIdentifier)?;
        let ident = &tail[..end];
        let name = match ident.split_once(TAG_START) {
            Some((n, _)) => n,
            None => ident,
        };
        out.push(match name {
            IDENT_RESERVED => Part::RepresentationId,
            IDENT_NUMBER => Part::Number(number_width(ident)),
            IDENT_TIME => Part::Time(number_width(ident)),
            IDENT_BANDWIDTH => Part::Bandwidth(number_width(ident)),
            _ => return Err(TemplateFault::UnknownIdentifier),
        });
        rest = &tail[end + 1..];
    }
    if !rest.is_empty() {
        out.push(Part::Literal(rest));
    }
    Ok(out)
}

/// The identifiers a URL was matched against a template: the exact inverse of
/// [`substitute`].
#[derive(Debug, Clone, PartialEq, Eq, Default)]
#[non_exhaustive]
pub struct TemplateMatch {
    /// The matched `$RepresentationID$` value (empty if the template carried
    /// none).
    pub representation_id: String,
    /// The matched `$Number$` value, if present.
    pub number: Option<u64>,
    /// The matched `$Time$` value, if present.
    pub time: Option<u64>,
    /// The matched `$Bandwidth$` value, if present.
    pub bandwidth: Option<u64>,
}

/// Recover the identifiers that [`substitute`] would have rendered as `url`.
///
/// `known_representation_ids` supplies the values `$RepresentationID$` may
/// take; they are tried longest-first and the whole `url` must match. Returns
/// `None` when the template is invalid (any [`TemplateFault`]), when a
/// `$SubNumber$` makes the reading undefined, or when the reading is not
/// unique.
#[must_use]
pub fn match_url(
    template: &str,
    url: &str,
    known_representation_ids: &[&str],
) -> Option<TemplateMatch> {
    let parts = parts(template).ok()?;
    let mut ids: Vec<&str> = known_representation_ids.to_vec();
    ids.sort_by(|a, b| b.len().cmp(&a.len()).then_with(|| a.cmp(b)));
    ids.dedup();
    let mut solutions: Vec<TemplateMatch> = Vec::new();
    let search = Search {
        parts: &parts,
        ids: &ids,
        url,
    };
    walk(&search, 0, 0, Reading::default(), &mut solutions);
    if solutions.len() == 1 {
        solutions.pop()
    } else {
        None
    }
}

#[derive(Debug, Clone, Default)]
struct Reading {
    matched: TemplateMatch,
    id: bool,
    number: bool,
    time: bool,
    bandwidth: bool,
}

#[derive(Clone, Copy)]
struct Search<'s, 'p> {
    parts: &'s [Part<'p>],
    ids: &'s [&'s str],
    url: &'s str,
}

fn walk(
    s: &Search<'_, '_>,
    pi: usize,
    pos: usize,
    current: Reading,
    solutions: &mut Vec<TemplateMatch>,
) {
    let Search {
        parts, ids, url, ..
    } = *s;
    if solutions.len() >= 2 {
        return;
    }
    let Some(part) = parts.get(pi) else {
        if pos == url.len() {
            solutions.push(current.matched);
        }
        return;
    };
    let rest = &url[pos..];
    match part {
        Part::Literal(lit) => {
            if let Some(after) = rest.strip_prefix(*lit) {
                let at = url.len() - after.len();
                walk(s, pi + 1, at, current, solutions);
            }
        }
        Part::Escape => {
            if let Some(after) = rest.strip_prefix(DELIMITER) {
                let at = url.len() - after.len();
                walk(s, pi + 1, at, current, solutions);
            }
        }
        Part::RepresentationId => {
            for id in ids {
                let Some(after) = rest.strip_prefix(*id) else {
                    continue;
                };
                if current.id && current.matched.representation_id != *id {
                    continue;
                }
                let mut next = current.clone();
                next.id = true;
                next.matched.representation_id = (*id).to_owned();
                walk(s, pi + 1, url.len() - after.len(), next, solutions);
                if solutions.len() >= 2 {
                    return;
                }
            }
        }
        Part::Number(width) => numeric(s, pi, pos, *width, &current, solutions, Field::Number),
        Part::Time(width) => numeric(s, pi, pos, *width, &current, solutions, Field::Time),
        Part::Bandwidth(width) => {
            numeric(s, pi, pos, *width, &current, solutions, Field::Bandwidth)
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Field {
    Number,
    Time,
    Bandwidth,
}

impl Reading {
    fn set(&mut self, field: Field, value: u64) -> Option<&mut Self> {
        let slot = match field {
            Field::Number => &mut self.matched.number,
            Field::Time => &mut self.matched.time,
            Field::Bandwidth => &mut self.matched.bandwidth,
        };
        match *slot {
            Some(seen) if seen != value => return None,
            _ => *slot = Some(value),
        }
        match field {
            Field::Number => self.number = true,
            Field::Time => self.time = true,
            Field::Bandwidth => self.bandwidth = true,
        }
        Some(self)
    }
}

fn digit_run(rest: &str) -> usize {
    rest.find(|c: char| !c.is_ascii_digit())
        .unwrap_or(rest.len())
}

#[must_use]
pub fn format_width(n: u64, width: usize) -> String {
    let width = width.clamp(1, MAX_FORMAT_WIDTH);
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

fn numeric(
    s: &Search<'_, '_>,
    pi: usize,
    pos: usize,
    width: Option<usize>,
    current: &Reading,
    solutions: &mut Vec<TemplateMatch>,
    field: Field,
) {
    let rest = &s.url[pos..];
    let max_len = digit_run(rest);
    let w = width.map_or(1, |w| w.clamp(1, MAX_FORMAT_WIDTH));
    let min_len = w.min(max_len).max(1);
    if max_len == 0 || min_len > max_len {
        return;
    }
    for len in (min_len..=max_len).rev() {
        let Some(sub) = rest.get(..len) else {
            continue;
        };
        let Ok(value) = sub.parse::<u64>() else {
            continue;
        };
        if format_width(value, w) != sub {
            continue;
        }
        let mut next = current.clone();
        if next.set(field, value).is_none() {
            continue;
        }
        walk(s, pi + 1, pos + len, next, solutions);
        if solutions.len() >= 2 {
            return;
        }
    }
}
