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
}
