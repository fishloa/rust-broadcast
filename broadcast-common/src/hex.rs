//! Hexadecimal encoding of raw byte fields.
//!
//! Wire formats across this workspace render opaque byte fields as hex text —
//! an HLS `#EXT-X-KEY:KEYID=0x…` attribute, an SDP `fmtp` codec-config
//! parameter, a Smooth Streaming `QualityLevel@CodecPrivateData`. The encoder
//! is identical in every one of them, so it lives here rather than being
//! recopied per crate.
//!
//! Only the *encoder* is shared. Decoding needs an error type, and each
//! consumer's is its own (and their input-validation policies genuinely
//! differ — e.g. a manifest parser caps input length where a local helper
//! does not), so decoders stay with their callers.

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
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(data.len() * 2);
    for &b in data {
        out.push(HEX[(b >> 4) as usize] as char);
        out.push(HEX[(b & 0x0F) as usize] as char);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::format;
    use alloc::vec::Vec;

    #[test]
    fn encodes_lowercase_zero_padded() {
        assert_eq!(hex_encode(&[0xDE, 0xAD, 0xBE, 0xEF]), "deadbeef");
        // A byte below 0x10 must still occupy two characters.
        assert_eq!(hex_encode(&[0x00, 0x01, 0x0A]), "00010a");
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
