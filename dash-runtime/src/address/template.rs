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
