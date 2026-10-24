//! Error type returned by playlist parsing.

use alloc::string::String;
use thiserror::Error;

/// Crate-wide result alias.
pub type Result<T> = core::result::Result<T, Error>;

/// Error variants that [`crate::MediaPlaylist::parse`]/[`crate::MasterPlaylist::parse`]
/// can return.
///
/// No longer `Eq` (only `PartialEq`): [`Error::InvalidDecimalSeconds`]/
/// [`Error::InvalidSignedDecimalSeconds`] carry the offending `f64`, which
/// has no `Eq` impl (NaN != NaN).
#[derive(Debug, Error, PartialEq)]
#[non_exhaustive]
pub enum Error {
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
