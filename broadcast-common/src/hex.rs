//! Hexadecimal encoding of raw byte fields.
//!
//! Wire formats across this workspace render opaque byte fields as hex text —
//! an HLS `#EXT-X-KEY:KEYID=0x…` attribute, an SDP `fmtp` codec-config
//! parameter, a Smooth Streaming `QualityLevel@CodecPrivateData`. The encoder
//! is identical in every one of them, so it lives here rather than being
//! recopied per crate.
//!
//! The encoder delegates to the `hex` crate. Decoding needs an error type, and
//! each consumer's is its own (and their input-validation policies genuinely
//! differ — e.g. a manifest parser caps input length where a local helper
//! does not), so decoders stay with their callers. [`hex_decode`] is the
//! decoder for the surfaces that must accept either case (Smooth's
//! `CodecPrivateData` is upper-case on the wire but not universally so).

use alloc::string::String;

/// Hex-encode `data` as lowercase ASCII (two characters per byte).
///
/// The output is exactly `2 * data.len()` characters, zero-padded per byte,
/// with no separators and no `0x` prefix — the form every wire format in this
/// workspace uses.
///
/// ```
/// use broadcast_common::hex::hex_encode;
///
/// assert_eq!(hex_encode(&[0x00, 0x0f, 0xff]), "000fff");
/// assert_eq!(hex_encode(&[]), "");
/// ```
pub fn hex_encode(data: &[u8]) -> String {
    ::hex::encode(data)
}

/// Hex-encode `data` as uppercase ASCII (two characters per byte).
pub fn hex_encode_upper(data: &[u8]) -> String {
    ::hex::encode_upper(data)
}

/// Hex-decode `text` (either case, an even number of digits), or `None` when
/// it is not hex of even length.
///
/// ```
/// use broadcast_common::hex::hex_decode;
///
/// assert_eq!(hex_decode("00Ff"), Some(vec![0x00, 0xff]));
/// assert_eq!(hex_decode("0"), None);
/// assert_eq!(hex_decode("zz"), None);
/// ```
#[must_use]
pub fn hex_decode(text: &str) -> Option<alloc::vec::Vec<u8>> {
    ::hex::decode(text).ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::format;
    use alloc::vec::Vec;

    #[test]
    fn encodes_uppercase_zero_padded() {
        assert_eq!(hex_encode_upper(&[0xDE, 0xAD, 0xBE, 0xEF]), "DEADBEEF");
        assert_eq!(hex_encode_upper(&[]), "");
    }

    #[test]
    fn encodes_lowercase_zero_padded() {
        assert_eq!(hex_encode(&[0xDE, 0xAD, 0xBE, 0xEF]), "deadbeef");
        // A byte below 0x10 must still occupy two characters.
        assert_eq!(hex_encode(&[0x00, 0x01, 0x0A]), "00010a");
    }

    #[test]
    fn decodes_either_case_and_rejects_odd_or_non_hex() {
        assert_eq!(hex_decode("00Ff12"), Some(alloc::vec![0x00, 0xff, 0x12]));
        assert_eq!(hex_decode(""), Some(alloc::vec::Vec::new()));
        assert_eq!(hex_decode("0"), None);
        assert_eq!(hex_decode("zz"), None);
    }

    #[test]
    fn empty_input_is_empty_output() {
        assert_eq!(hex_encode(&[]), "");
    }

    #[test]
    fn output_is_always_two_chars_per_byte() {
        let all: Vec<u8> = (0..=255u8).collect();
        let encoded = hex_encode(&all);
        assert_eq!(encoded.len(), all.len() * 2);
        // Every byte value round-trips to the same text `format!("{:02x}")`
        // produces — the property callers actually depend on.
        let expected: String = all.iter().map(|b| format!("{b:02x}")).collect();
        assert_eq!(encoded, expected);
    }
}
