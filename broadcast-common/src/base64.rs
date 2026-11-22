//! Base64 codec (RFC 4648), wrapping the `base64` crate.
//!
//! Two decoders are provided so callers can keep their existing padding
//! policy: [`base64_decode_strict`] requires canonical padding, while
//! [`base64_decode_lenient`] tolerates missing padding and trailing bits.

use alloc::string::String;
use alloc::vec::Vec;

/// Why a base64 decode refused its input.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum Base64DecodeError {
    /// A byte outside the alphabet, or misplaced padding or trailing input.
    #[error("invalid base64 byte {byte:#04x} at offset {offset}")]
    InvalidByte {
        /// Byte offset in the input.
        offset: usize,
        /// That byte.
        byte: u8,
    },
    /// The input holds a number of alphabet symbols no encoding produces.
    #[error("invalid base64 length: {symbols} symbol(s)")]
    InvalidLength {
        /// Alphabet symbols in the input.
        symbols: usize,
    },
    /// The last symbol carries non-zero bits beyond the encoded data.
    #[error("invalid base64 last symbol {symbol:#04x} at offset {offset}")]
    InvalidLastSymbol {
        /// Byte offset in the input.
        offset: usize,
        /// That symbol.
        symbol: u8,
    },
    /// Padding is absent where required, or present where refused.
    #[error("invalid base64 padding")]
    InvalidPadding,
}

impl Base64DecodeError {
    /// The reason's stable label, without the offending input.
    pub fn name(&self) -> &'static str {
        match self {
            Self::InvalidByte { .. } => "invalid_byte",
            Self::InvalidLength { .. } => "invalid_length",
            Self::InvalidLastSymbol { .. } => "invalid_last_symbol",
            Self::InvalidPadding => "invalid_padding",
        }
    }
}

fn decode_error(error: ::base64::DecodeError) -> Base64DecodeError {
    match error {
        ::base64::DecodeError::InvalidByte(offset, byte) => {
            Base64DecodeError::InvalidByte { offset, byte }
        }
        ::base64::DecodeError::InvalidLength(symbols) => {
            Base64DecodeError::InvalidLength { symbols }
        }
        ::base64::DecodeError::InvalidLastSymbol { offset, symbol, .. } => {
            Base64DecodeError::InvalidLastSymbol { offset, symbol }
        }
        ::base64::DecodeError::InvalidPadding => Base64DecodeError::InvalidPadding,
    }
}

/// Base64-encode `data` (RFC 4648 §4, with `=` padding).
pub fn base64_encode(data: &[u8]) -> String {
    use base64::Engine as _;
    base64::engine::general_purpose::STANDARD.encode(data)
}

/// Strictly base64-decode `text`: canonical padding is required.
pub fn base64_decode_strict(s: &str) -> Result<Vec<u8>, Base64DecodeError> {
    use base64::Engine as _;
    base64::engine::general_purpose::STANDARD
        .decode(s)
        .map_err(decode_error)
}

/// Leniently base64-decode `text`: padding is optional and trailing bits are
/// tolerated.
pub fn base64_decode_lenient(s: &str) -> Result<Vec<u8>, Base64DecodeError> {
    use base64::Engine as _;
    const LENIENT: base64::engine::GeneralPurpose = base64::engine::GeneralPurpose::new(
        &base64::alphabet::STANDARD,
        base64::engine::general_purpose::GeneralPurposeConfig::new()
            .with_decode_padding_mode(base64::engine::DecodePaddingMode::Indifferent)
            .with_decode_allow_trailing_bits(true),
    );
    LENIENT.decode(s).map_err(decode_error)
}

/// Base64-encode `data` with the URL-safe alphabet and no padding (RFC 4648
/// §5).
pub fn base64_url_encode_nopad(data: &[u8]) -> String {
    use base64::Engine as _;
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(data)
}

/// Decode URL-safe, unpadded base64 (RFC 4648 §5); padding is refused.
pub fn base64_url_decode_nopad(s: &str) -> Result<Vec<u8>, Base64DecodeError> {
    use base64::Engine as _;
    base64::engine::general_purpose::URL_SAFE_NO_PAD
        .decode(s)
        .map_err(decode_error)
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::string::ToString;

    #[test]
    fn decode_errors_are_broadcast_commons_own_type() {
        assert_eq!(
            base64_decode_strict("a!=="),
            Err(Base64DecodeError::InvalidByte {
                offset: 1,
                byte: b'!'
            })
        );
        assert_eq!(
            base64_decode_strict("aGVsbG8"),
            Err(Base64DecodeError::InvalidPadding)
        );
        assert_eq!(
            base64_decode_lenient("aGVsbG8hZ"),
            Err(Base64DecodeError::InvalidLength { symbols: 9 })
        );
        assert_eq!(
            base64_decode_strict("Zm9="),
            Err(Base64DecodeError::InvalidLastSymbol {
                offset: 2,
                symbol: b'9'
            })
        );
        assert_eq!(
            Base64DecodeError::InvalidByte {
                offset: 1,
                byte: b'!'
            }
            .to_string(),
            "invalid base64 byte 0x21 at offset 1"
        );
        assert_eq!(Base64DecodeError::InvalidPadding.name(), "invalid_padding");
    }

    #[test]
    fn round_trips() {
        let data = b"hello";
        let enc = base64_encode(data);
        assert_eq!(base64_decode_strict(&enc).unwrap(), data);
    }

    #[test]
    fn strict_vs_lenient_on_padding_less_input() {
        // "aGVsbG8" is "hello" without padding.
        assert!(base64_decode_strict("aGVsbG8").is_err());
        assert_eq!(base64_decode_lenient("aGVsbG8").unwrap(), b"hello");
    }

    #[test]
    fn url_safe_no_pad_uses_dash_underscore_and_drops_padding() {
        assert_eq!(base64_url_encode_nopad(&[0xfb, 0xff, 0xfe]), "-__-");
        assert_eq!(base64_url_encode_nopad(b"fo"), "Zm8");
        assert_eq!(base64_encode(&[0xfb, 0xff, 0xfe]), "+//+");
        assert_eq!(base64_url_decode_nopad("-__-").unwrap(), [0xfb, 0xff, 0xfe]);
        assert!(base64_url_decode_nopad("Zm8=").is_err());
        assert!(base64_url_decode_nopad("+//+").is_err());
    }
}
