//! Base64 codec (RFC 4648), wrapping the `base64` crate.
//!
//! Two decoders are provided so callers can keep their existing padding
//! policy: [`base64_decode_strict`] requires canonical padding, while
//! [`base64_decode_lenient`] tolerates missing padding and trailing bits.

use alloc::string::String;
use alloc::vec::Vec;

/// Base64-encode `data` (RFC 4648 §4, with `=` padding).
pub fn base64_encode(data: &[u8]) -> String {
    use base64::Engine as _;
    base64::engine::general_purpose::STANDARD.encode(data)
}

/// Strictly base64-decode `text`: canonical padding is required.
pub fn base64_decode_strict(s: &str) -> Result<Vec<u8>, base64::DecodeError> {
    use base64::Engine as _;
    base64::engine::general_purpose::STANDARD.decode(s)
}

/// Leniently base64-decode `text`: padding is optional and trailing bits are
/// tolerated.
pub fn base64_decode_lenient(s: &str) -> Result<Vec<u8>, base64::DecodeError> {
    use base64::Engine as _;
    const LENIENT: base64::engine::GeneralPurpose = base64::engine::GeneralPurpose::new(
        &base64::alphabet::STANDARD,
        base64::engine::general_purpose::GeneralPurposeConfig::new()
            .with_decode_padding_mode(base64::engine::DecodePaddingMode::Indifferent)
            .with_decode_allow_trailing_bits(true),
    );
    LENIENT.decode(s)
}

#[cfg(test)]
mod tests {
    use super::*;

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
}
