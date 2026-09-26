//! The EN 50221 `length_field` — the ASN.1-style length used by every PDU at
//! the Transport, Session and Application layers — ETSI EN 50221 §7, Table 1
//! (PDF p. 11).
//!
//! Coding:
//! - the first (MSB) bit is the `size_indicator`;
//! - `size_indicator == 0`: the data-field length is the remaining 7 bits — any
//!   length `0..=127` fits in one byte;
//! - `size_indicator == 1`: the remaining 7 bits (`length_field_size`) code how
//!   many subsequent bytes carry the length, MSB-first. The spec caps any length
//!   at 65535, i.e. at most three length bytes.
//!
//! The indefinite-length ASN.1 form is NOT used.

use crate::error::{Error, Result};

/// The `size_indicator` bit (the MSB of the first `length_field` byte): set when
/// the field is the multi-byte form.
pub const SIZE_INDICATOR_MASK: u8 = 0x80;

/// Decode a `length_field` from the front of `bytes`.
///
/// Returns `(length_value, header_len)` where `header_len` is the number of
/// bytes the `length_field` itself occupied (so the body starts at
/// `bytes[header_len..]`).
pub fn decode(bytes: &[u8]) -> Result<(usize, usize)> {
    let first = *bytes
        .first()
        .ok_or(Error::InvalidLength("empty length_field"))?;
    if first & SIZE_INDICATOR_MASK == 0 {
        // Short form: 7-bit length in this byte.
        return Ok(((first & 0x7F) as usize, 1));
    }
    // Long form: low 7 bits = number of subsequent length bytes.
    let n = (first & 0x7F) as usize;
    if n == 0 {
        // size_indicator==1 with zero following bytes is the indefinite form,
        // which the spec forbids.
        return Err(Error::InvalidLength("indefinite length form not allowed"));
    }
    if n > 2 {
        // "Any length up to 65535 can be encoded by three bytes" (module doc)
        // means three bytes TOTAL: the size-indicator byte plus at most 2
        // length bytes — not 2 subsequent to a size-indicator that itself
        // implies a 4-byte total. `encode_into` never emits `n > 2` (it caps
        // values at 0xFFFF, [`Error::LengthTooLarge`] beyond that), so
        // accepting a wider form here would parse a value `encode_into`
        // could never reproduce.
        return Err(Error::InvalidLength("length_field_size exceeds 2 bytes"));
    }
    if bytes.len() < 1 + n {
        return Err(Error::BufferTooShort {
            need: 1 + n,
            have: bytes.len(),
            what: "length_field (long form)",
        });
    }
    let mut value = 0usize;
    for &b in &bytes[1..1 + n] {
        value = (value << 8) | b as usize;
    }
    // Non-minimal long forms (e.g. `value = 5` sent as the 2-byte long form
    // rather than the 1-byte short form) are deliberately still accepted:
    // real hardware captures use them (`dvb-ci-runtime`'s
    // `decodes_long_form_length_profile_reply`, issue #337, `82 00 09` for
    // value 9) — a decoder that rejected them would reject real CAM
    // traffic. `parse -> serialize` of a *higher-level* object (which
    // stores only the decoded value, not the original form) then legally
    // re-encodes it minimally; that is a value-preserving, not
    // byte-preserving, round trip for this one low-level field, same as
    // any lenient-parse/canonical-serialize codec.
    Ok((value, 1 + n))
}

/// Number of bytes [`encode_into`] will write for `value`.
#[must_use]
pub fn encoded_len(value: usize) -> usize {
    if value < 0x80 {
        1
    } else if value <= 0xFF {
        2
    } else if value <= 0xFFFF {
        3
    } else {
        // >65535: caller is expected to reject via encode_into; report the
        // widest legal form so serialized_len stays an upper bound.
        4
    }
}

/// Encode `value` as a `length_field` into `buf`, returning the bytes written.
///
/// Uses the short form for `0..=127`, otherwise the minimal long form. Values
/// above 65535 are rejected ([`Error::LengthTooLarge`]) per the spec cap.
pub fn encode_into(value: usize, buf: &mut [u8]) -> Result<usize> {
    let need = encoded_len(value);
    if value > 0xFFFF {
        return Err(Error::LengthTooLarge(value));
    }
    if buf.len() < need {
        return Err(Error::OutputBufferTooSmall {
            need,
            have: buf.len(),
        });
    }
    if value < 0x80 {
        buf[0] = value as u8;
    } else if value <= 0xFF {
        buf[0] = SIZE_INDICATOR_MASK | 1;
        buf[1] = value as u8;
    } else {
        buf[0] = SIZE_INDICATOR_MASK | 2;
        buf[1] = (value >> 8) as u8;
        buf[2] = value as u8;
    }
    Ok(need)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn short_form_round_trip() {
        for v in [0usize, 1, 0x7F] {
            let mut buf = [0u8; 4];
            let n = encode_into(v, &mut buf).unwrap();
            assert_eq!(n, 1);
            assert_eq!(encoded_len(v), 1);
            let (decoded, hdr) = decode(&buf).unwrap();
            assert_eq!((decoded, hdr), (v, 1));
        }
    }

    #[test]
    fn two_byte_form_round_trip() {
        for v in [0x80usize, 0xFF] {
            let mut buf = [0u8; 4];
            let n = encode_into(v, &mut buf).unwrap();
            assert_eq!(n, 2);
            assert_eq!(buf[0], SIZE_INDICATOR_MASK | 1);
            let (decoded, hdr) = decode(&buf).unwrap();
            assert_eq!((decoded, hdr), (v, 2));
        }
    }

    #[test]
    fn three_byte_form_round_trip() {
        for v in [0x100usize, 0x1234, 0xFFFF] {
            let mut buf = [0u8; 4];
            let n = encode_into(v, &mut buf).unwrap();
            assert_eq!(n, 3);
            assert_eq!(buf[0], SIZE_INDICATOR_MASK | 2);
            let (decoded, hdr) = decode(&buf).unwrap();
            assert_eq!((decoded, hdr), (v, 3));
        }
    }

    #[test]
    fn rejects_oversize() {
        let mut buf = [0u8; 4];
        assert!(matches!(
            encode_into(0x1_0000, &mut buf),
            Err(Error::LengthTooLarge(0x1_0000))
        ));
    }

    #[test]
    fn rejects_indefinite_and_wide() {
        assert!(decode(&[0x80]).is_err()); // indefinite form
        assert!(decode(&[0x84, 0, 0, 0, 0]).is_err()); // 4 length bytes
        assert!(decode(&[]).is_err());
        assert!(decode(&[0x82, 0x12]).is_err()); // truncated long form
    }

    /// r10-W-4: `encode_into` never produces a 3-subsequent-byte long form
    /// (it caps at `n = 2`, `LengthTooLarge` beyond `0xFFFF`), so a decode
    /// that accepts one parses a value `encode_into` cannot reproduce —
    /// pre-fix, `decode` accepted `n` up to 3.
    #[test]
    fn rejects_three_length_bytes_form() {
        // size_indicator=1, length_field_size=3, value = 0x000100 = 256.
        assert!(decode(&[0x83, 0x00, 0x01, 0x00]).is_err());
    }

    /// A non-minimal long form (`encode_into` would have used a shorter
    /// form for the same value) must still decode — real hardware sends
    /// these (see `dvb-ci-runtime`'s `decodes_long_form_length_profile_reply`,
    /// issue #337: `82 00 09` for value 9, which `encode_into` would encode
    /// as the 1-byte short form `09`). Rejecting them would reject real CAM
    /// traffic; only the *value* round-trips, not necessarily these exact
    /// bytes, matching any lenient-parse/canonical-serialize codec.
    #[test]
    fn non_minimal_long_forms_still_decode() {
        assert_eq!(decode(&[0x81, 0x05]).unwrap(), (5, 2));
        assert_eq!(decode(&[0x82, 0x00, 0x09]).unwrap(), (9, 3));
        assert_eq!(decode(&[0x81, 0x80]).unwrap(), (0x80, 2));
        assert_eq!(decode(&[0x82, 0x01, 0x00]).unwrap(), (0x100, 3));
    }

    #[test]
    fn mutating_a_byte_changes_decode() {
        let mut buf = [0u8; 4];
        encode_into(0x1234, &mut buf).unwrap();
        let (a, _) = decode(&buf).unwrap();
        buf[2] ^= 0xFF;
        let (b, _) = decode(&buf).unwrap();
        assert_ne!(a, b);
    }
}
