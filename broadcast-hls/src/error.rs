//! Error type returned by playlist parsing.

use alloc::string::String;
use thiserror::Error;

/// Crate-wide result alias.
pub type Result<T> = core::result::Result<T, Error>;

/// Why a client-defined (`X-`) attribute name was rejected
/// (RFC 8216bis §4.2, §4.4.5.1).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum ClientAttrNameError {
    /// The name does not start with `X-`.
    MissingPrefix,
    /// Nothing follows the `X-` prefix.
    EmptyName,
    /// The name ends with `-`.
    TrailingHyphen,
    /// A character is outside the §4.2 `AttributeName` set.
    InvalidCharacter,
}

impl ClientAttrNameError {
    /// Stable lowercase label.
    pub fn name(&self) -> &'static str {
        match self {
            Self::MissingPrefix => "missing_prefix",
            Self::EmptyName => "empty_name",
            Self::TrailingHyphen => "trailing_hyphen",
            Self::InvalidCharacter => "invalid_character",
        }
    }
}

broadcast_core::impl_spec_display!(ClientAttrNameError);

/// Why a [`crate::CustomTagName`] was rejected (RFC 8216bis §4.1).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum CustomTagNameError {
    /// The name does not start with `EXT-`.
    MissingPrefix,
    /// Nothing follows the `EXT-` prefix.
    EmptyName,
    /// The name ends with `-`.
    TrailingHyphen,
    /// A character is outside the §4.1 tag set.
    InvalidCharacter,
    /// The name is one this crate parses as a standard tag.
    StandardTag,
}

impl CustomTagNameError {
    /// Stable lowercase label.
    pub fn name(&self) -> &'static str {
        match self {
            Self::MissingPrefix => "missing_prefix",
            Self::EmptyName => "empty_name",
            Self::TrailingHyphen => "trailing_hyphen",
            Self::InvalidCharacter => "invalid_character",
            Self::StandardTag => "standard_tag",
        }
    }
}

broadcast_core::impl_spec_display!(CustomTagNameError);

/// Why an attribute-list text was rejected by the strict wire scanner
/// (RFC 8216bis §4.2).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum AttributeListError {
    /// The list is empty.
    Empty,
    /// A comma has no attribute after it.
    TrailingComma,
    /// An attribute has no `=`.
    MissingEquals,
    /// The name is empty or not a legal `AttributeName`.
    InvalidName,
    /// The value is empty.
    EmptyValue,
    /// A quoted-string is not terminated.
    UnterminatedQuote,
    /// Characters follow a closing quote.
    TrailingCharacters,
    /// Whitespace appears outside a quoted-string.
    Whitespace,
    /// A bare value contains a `"`.
    QuoteInBareValue,
    /// A value contains a C0 control character or DEL.
    ControlCharacter,
}

impl AttributeListError {
    /// Stable lowercase label.
    pub fn name(&self) -> &'static str {
        match self {
            Self::Empty => "empty",
            Self::TrailingComma => "trailing_comma",
            Self::MissingEquals => "missing_equals",
            Self::InvalidName => "invalid_name",
            Self::EmptyValue => "empty_value",
            Self::UnterminatedQuote => "unterminated_quote",
            Self::TrailingCharacters => "trailing_characters",
            Self::Whitespace => "whitespace",
            Self::QuoteInBareValue => "quote_in_bare_value",
            Self::ControlCharacter => "control_character",
        }
    }
}

broadcast_core::impl_spec_display!(AttributeListError);

/// Why an `EXT-X-DATERANGE` was rejected (RFC 8216bis §4.4.5.1).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum DateRangeError {
    /// `ID` is empty.
    EmptyId,
    /// `END-ON-NEXT` is present without `CLASS`.
    EndOnNextWithoutClass,
    /// `END-ON-NEXT` is present with `DURATION`.
    EndOnNextWithDuration,
    /// `END-ON-NEXT` is present with `END-DATE`.
    EndOnNextWithEndDate,
    /// `END-DATE` is before `START-DATE`.
    EndBeforeStart,
    /// `END-DATE` is not `START-DATE` plus `DURATION`.
    EndDateDurationMismatch,
    /// A known attribute has the wrong RFC 8216bis §4.2 value kind.
    WrongAttributeKind,
    /// A `DURATION`/`PLANNED-DURATION` is not a decimal-floating-point.
    InvalidDurationValue,
    /// A `CUE` is not a list of `PRE`/`POST`/`ONCE`.
    InvalidCue,
    /// A `CUE` names both `PRE` and `POST`.
    CuePreAndPost,
    /// A `CUE` repeats a trigger.
    DuplicateCueTrigger,
    /// An `X-` attribute value is not one of the three allowed kinds.
    InvalidClientAttributeValue,
    /// An `SCTE35-CMD`/`-OUT`/`-IN` value is not a hexadecimal-sequence.
    InvalidScte35Value,
    /// `END-ON-NEXT` is not exactly `YES`.
    EndOnNextNotYes,
}

impl DateRangeError {
    /// Stable lowercase label.
    pub fn name(&self) -> &'static str {
        match self {
            Self::EmptyId => "empty_id",
            Self::EndOnNextWithoutClass => "end_on_next_without_class",
            Self::EndOnNextWithDuration => "end_on_next_with_duration",
            Self::EndOnNextWithEndDate => "end_on_next_with_end_date",
            Self::EndBeforeStart => "end_before_start",
            Self::EndDateDurationMismatch => "end_date_duration_mismatch",
            Self::WrongAttributeKind => "wrong_attribute_kind",
            Self::InvalidDurationValue => "invalid_duration_value",
            Self::InvalidCue => "invalid_cue",
            Self::CuePreAndPost => "cue_pre_and_post",
            Self::DuplicateCueTrigger => "duplicate_cue_trigger",
            Self::InvalidClientAttributeValue => "invalid_client_attribute_value",
            Self::InvalidScte35Value => "invalid_scte35_value",
            Self::EndOnNextNotYes => "end_on_next_not_yes",
        }
    }
}

broadcast_core::impl_spec_display!(DateRangeError);

/// Error variants that [`crate::MediaPlaylist::parse`]/[`crate::MasterPlaylist::parse`]
/// can return.
///
/// No longer `Eq` (only `PartialEq`): [`Error::InvalidDecimalSeconds`]/
/// [`Error::InvalidSignedDecimalSeconds`] carry the offending `f64`, which
/// has no `Eq` impl (NaN != NaN).
#[derive(Debug, Error, PartialEq, Clone)]
#[non_exhaustive]
pub enum Error {
    /// A playlist exceeded a [`crate::limits::Limits`] bound (audit 20
    /// finding 41, issue #2852).
    #[error("{what} is {len}, over the limit of {max}")]
    TooLarge {
        /// What exceeded the bound (e.g. `"playlist bytes"`).
        what: &'static str,
        /// The observed size.
        len: usize,
        /// The bound it exceeded.
        max: usize,
    },

    /// An HLS playlist (`.m3u8`, RFC 8216bis) tag could not be parsed —
    /// [`crate::MediaPlaylist::parse`]/[`crate::MasterPlaylist::parse`]'s
    /// `to_m3u8()` renderers' symmetric inverse. Unrecognized tags are
    /// ignored (forward-compat); this variant is only returned for a
    /// *known* tag whose required attribute is missing or whose value
    /// fails to parse.
    #[error("hls parse (line {line_no}): {reason}\n  {line}")]
    HlsParse {
        /// 1-based line number within the input playlist text.
        line_no: usize,
        /// The offending line, verbatim.
        line: String,
        /// Human-readable explanation.
        reason: String,
    },

    /// An attribute-list value cannot be represented as the requested
    /// [`crate::AttrValue`] kind (issue #1045 T12): a quoted-string
    /// containing `"`, CR or LF (RFC 8216 §4.2 forbids all three), or a
    /// bare enumerated-string/decimal value containing a character that
    /// would require quoting (`,`, `"`, CR, LF, or whitespace). Rejected at
    /// construction rather than silently mangled (e.g. percent-encoded) or
    /// rendered raw — cf. #1129.
    #[error("attribute value {value:?} is not valid as {kind}")]
    InvalidAttrValue {
        /// The value that failed validation.
        value: String,
        /// Which kind was requested: `"a quoted-string"` or `"a bare
        /// value"`.
        kind: &'static str,
    },

    /// [`crate::DecimalSeconds::new`] rejected a NaN, infinite, or negative
    /// value (issue #1140 T12/BH-W4): RFC 8216bis §4.2 decimal-floating-point
    /// is non-negative and finite. Rejected at construction rather than
    /// silently clamped, so a bad value can never reach a renderer.
    #[error("{0} is not a finite, non-negative decimal-floating-point number of seconds")]
    InvalidDecimalSeconds(f64),

    /// [`crate::SignedDecimalSeconds::new`] rejected a NaN or infinite value
    /// (issue #1140 T12/BH-W4) — the one signed decimal-floating-point field
    /// (`TIME-OFFSET`) still requires finiteness.
    #[error("{0} is not a finite decimal-floating-point number of seconds")]
    InvalidSignedDecimalSeconds(f64),

    /// A model field rendered as an RFC 8216 §4.2 quoted-string — a
    /// quoted-string attribute value (`CODECS`, `KEYFORMAT`, `SERVER-URI`,
    /// `VALUE`, …) — contains `"`, CR or LF, all three forbidden there
    /// (audit BH-W7, issue #1111). Rejected at construction rather than
    /// emitted raw: a value carrying a newline injects a tag line into the
    /// rendered playlist, and one carrying `"` breaks out of the attribute
    /// list.
    #[error("{what} {value:?} contains {bad_char:?}, forbidden in an HLS quoted-string")]
    InvalidQuotedString {
        /// Which field rejected the value (e.g. `"segment URI"`).
        what: String,
        /// The offending character: `"`, CR or LF.
        bad_char: char,
        /// The value that failed validation.
        value: String,
    },

    /// A model field rendered as a bare URI line or as the URI of a
    /// URI-bearing attribute (segment/part/map/variant URI) contains `"`,
    /// CR or LF (audit BH-W7, issue #1111). Same injection consequence as
    /// [`Error::InvalidQuotedString`]; a separate variant so a caller can
    /// tell which field class failed.
    #[error("URI {value:?} ({what}) contains {bad_char:?}, forbidden in an HLS URI")]
    InvalidUri {
        /// Which URI rejected the value (e.g. `"segment URI"`).
        what: String,
        /// The offending character: `"`, CR or LF.
        bad_char: char,
        /// The value that failed validation.
        value: String,
    },

    /// A verbatim tag line (`extra_tags`/`pre_tags`) is empty or does not
    /// start with `#EXT`, so it would render as a URI line or a terminator
    /// instead of a tag (audit 20 finding 51, issue #2862).
    #[error("{what} {value:?} is not an #EXT tag line")]
    InvalidTagLine {
        /// Which field rejected the value (e.g. `"segment pre-tag"`).
        what: String,
        /// The offending line.
        value: String,
    },

    /// A `ClientAttrName` is not an `X-` prefixed name built from the
    /// RFC 8216bis §4.2 `AttributeName` character set (`[A-Z]`, `[0-9]`, `-`).
    #[error("client attribute name {name:?} is invalid: {reason}")]
    InvalidClientAttrName {
        /// The offending name.
        name: String,
        /// Why it was rejected.
        reason: ClientAttrNameError,
    },

    /// A `CustomTagName` is not an `EXT-` prefixed name built from the
    /// RFC 8216bis §4.1 tag character set (`[A-Z]`, `[0-9]`, `-`).
    #[error("custom tag name {name:?} is invalid: {reason}")]
    InvalidCustomTagName {
        /// The offending name.
        name: String,
        /// Why it was rejected.
        reason: CustomTagNameError,
    },

    /// An attribute-list text is outside the RFC 8216bis §4.2 grammar.
    #[error("attribute list {list:?} is invalid: {reason}")]
    InvalidAttributeList {
        /// The offending list text.
        list: String,
        /// Why it was rejected.
        reason: AttributeListError,
    },

    /// An attribute-list name is not built from the RFC 8216bis §4.2
    /// `AttributeName` character set (`[A-Z]`, `[0-9]`, `-`).
    #[error("attribute name {name:?} is not a legal RFC 8216bis AttributeName")]
    InvalidAttributeName {
        /// The offending name.
        name: String,
    },

    /// An `EXT-X-DATERANGE` violates a rule RFC 8216bis §4.4.5.1 states
    /// locally (one this crate can check without another tag).
    #[error("EXT-X-DATERANGE {id:?} is invalid: {reason}")]
    InvalidDateRange {
        /// The range's `ID`.
        id: String,
        /// Why it was rejected.
        reason: DateRangeError,
    },

    /// [`crate::MediaPlaylist::render_segment`] was asked for a segment
    /// index the playlist does not have.
    #[error("segment index {index} is out of range for a playlist of {len} segments")]
    SegmentIndexOutOfRange {
        /// The requested index.
        index: usize,
        /// The playlist's segment count.
        len: usize,
    },

    /// An `#EXT-X-KEY` whose `METHOD` is not `NONE` carries no `URI`
    /// (RFC 8216 §4.3.2.4: `URI` is REQUIRED unless `METHOD` is `NONE`).
    #[error("EXT-X-KEY with METHOD={method} has no URI")]
    KeyWithoutUri {
        /// The method that needs a URI.
        method: crate::EncryptionMethod,
    },

    /// A segment is covered only by `#EXT-X-KEY` tags of a `KEYFORMAT` other
    /// than `identity`, which this crate cannot resolve to a key file.
    #[error("no identity key applies; unsupported KEYFORMAT {keyformat:?}")]
    UnsupportedKeyFormat {
        /// The non-identity `KEYFORMAT` that applies.
        keyformat: String,
    },

    /// A `CLOSED-CAPTIONS` `INSTREAM-ID` is not `CC1`..`CC4` or
    /// `SERVICE1`..`SERVICE63` in canonical form (RFC 8216bis §4.4.6.1:
    /// `"CC"` + one digit 1-4, or `"SERVICE"` + decimal 1-63 with no sign and
    /// no leading zero).
    #[error("invalid CLOSED-CAPTIONS INSTREAM-ID {value:?}")]
    InvalidInstreamId {
        /// The offending value.
        value: String,
    },

    /// An `INSTREAM-ID` form does not match the rendition `TYPE`: `CC`/
    /// `SERVICE` forms are only valid on `CLOSED-CAPTIONS`, and `Other` only on
    /// the other types (RFC 8216bis §4.4.6.1).
    #[error("INSTREAM-ID {value:?} is not valid on TYPE={media_type}")]
    InstreamIdTypeMismatch {
        /// The rendition `TYPE`.
        media_type: crate::MediaType,
        /// The rendered `INSTREAM-ID`.
        value: String,
    },

    /// A Rendition Group has more than one member with `DEFAULT=YES`
    /// (RFC 8216bis §4.4.6.1.1).
    #[error("group {group_id:?} of TYPE={media_type} has more than one DEFAULT=YES member")]
    DuplicateDefaultInGroup {
        /// The group's `TYPE`.
        media_type: crate::MediaType,
        /// The group's `GROUP-ID`.
        group_id: String,
    },

    /// Two members of one Rendition Group share a `NAME`
    /// (RFC 8216bis §4.4.6.1.1).
    #[error("group {group_id:?} of TYPE={media_type} has more than one member named {name:?}")]
    DuplicateNameInGroup {
        /// The group's `TYPE`.
        media_type: crate::MediaType,
        /// The group's `GROUP-ID`.
        group_id: String,
        /// The repeated `NAME`.
        name: String,
    },

    /// An enumerated-string `YES`/`NO` attribute carries any other value;
    /// enumerated-strings are case-sensitive (RFC 8216bis §4.2).
    #[error("attribute {attribute} must be exactly YES or NO, got {value:?}")]
    InvalidYesNo {
        /// The attribute name.
        attribute: &'static str,
        /// The offending value.
        value: String,
    },

    /// An `IV` is not a `0x`/`0X`-prefixed 128-bit hexadecimal-sequence
    /// (RFC 8216bis §4.2, §4.4.4.4).
    #[error("IV {value:?} is not a 0x-prefixed 128-bit hexadecimal-sequence")]
    InvalidIv {
        /// The offending value.
        value: String,
    },

    /// A quoted-string attribute value has no closing `"` (RFC 8216bis §4.2).
    #[error("unterminated quoted-string in attribute list {list:?}")]
    UnterminatedQuotedString {
        /// The attribute list text.
        list: String,
    },

    /// An `AttributeName` appears more than once in one attribute list
    /// (RFC 8216bis §4.2: "MUST NOT appear more than once").
    #[error("duplicate attribute {name:?}")]
    DuplicateAttribute {
        /// The repeated attribute name.
        name: String,
    },
}
