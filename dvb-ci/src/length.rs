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

/// Widest long form accepted on the wire: `length_field_size` of 4 (one
/// size-indicator byte + four length octets). Anything wider is not a length
/// this codec can carry (values are capped at [`MAX_LENGTH`]).
pub const MAX_LENGTH_OCTETS: u8 = 4;

/// The largest value a `length_field` may carry (EN 50221 Table 1: "any length
/// up to 65535").
pub const MAX_LENGTH: usize = 0xFFFF;

/// How a `length_field` is — or is to be — coded on the wire.
///
/// A CAM may code a length in a longer form than necessary (`82 00 09` for the
/// value 9, captured raw on hardware, dvb-ci-runtime #337). `parse` is tolerant
/// and records the coding here; `serialize` re-emits exactly that coding, so a
/// parsed APDU/SPDU/TPDU round-trips byte-for-byte. A value built by hand uses
/// [`LengthForm::Minimal`] (the [`Default`]): the shortest legal form.
///
/// `parse` only ever produces [`LengthForm::Long`] for a *non-minimal* coding,
/// so a parsed object equals the same object built with the default form
/// whenever the wire used the minimal coding.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub enum LengthForm {
    /// The shortest legal coding for the value: the 1-byte short form for
    /// `0..=127`, otherwise the long form with the fewest length octets.
    #[default]
    Minimal,
    /// A non-minimal long form (`size_indicator = 1`) with exactly `octets`
    /// length octets (`1..=`[`MAX_LENGTH_OCTETS`]), e.g. `82 00 09`.
    Long {
        /// `length_field_size`: the number of length octets that follow the
        /// size-indicator byte.
        octets: u8,
    },
}

impl LengthForm {
    /// `true` for [`LengthForm::Minimal`] (used to skip the field in serde).
    #[must_use]
    pub const fn is_minimal(&self) -> bool {
        matches!(self, Self::Minimal)
    }

    /// Short label (issue #204 convention).
    #[must_use]
    pub fn name(&self) -> &'static str {
        match self {
            Self::Minimal => "minimal",
            Self::Long { .. } => "long",
        }
    }
}
broadcast_common::impl_spec_display!(LengthForm);

/// Decode a `length_field` from the front of `bytes`.
///
/// Returns `(length_value, header_len)` where `header_len` is the number of
/// bytes the `length_field` itself occupied (so the body starts at
/// `bytes[header_len..]`). Use [`decode_form`] to also learn how it was coded.
pub fn decode(bytes: &[u8]) -> Result<(usize, usize)> {
    let (value, hdr, _) = decode_form(bytes)?;
    Ok((value, hdr))
}

/// Like [`decode`], also returning the [`LengthForm`] the field was coded in
/// (so a serializer can reproduce it).
///
/// Accepts the short form and any long form of 1 to [`MAX_LENGTH_OCTETS`]
/// length octets whose value is at most [`MAX_LENGTH`], including non-minimal
/// ones (real CAMs send them). Rejects the indefinite form (`0x80`), a
/// `length_field_size` above [`MAX_LENGTH_OCTETS`], a value above
/// [`MAX_LENGTH`], and a truncated field.
pub fn decode_form(bytes: &[u8]) -> Result<(usize, usize, LengthForm)> {
    let first = *bytes
        .first()
        .ok_or(Error::InvalidLength("empty length_field"))?;
    if first & SIZE_INDICATOR_MASK == 0 {
        // Short form: 7-bit length in this byte.
        return Ok(((first & 0x7F) as usize, 1, LengthForm::Minimal));
    }
    // Long form: low 7 bits = number of subsequent length bytes.
    let n = (first & 0x7F) as usize;
    if n == 0 {
        // size_indicator==1 with zero following bytes is the indefinite form,
        // which the spec forbids.
        return Err(Error::InvalidLength("indefinite length form not allowed"));
    }
    if n > MAX_LENGTH_OCTETS as usize {
        return Err(Error::InvalidLength(
            "length_field_size exceeds 4 length octets",
        ));
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
    if value > MAX_LENGTH {
        return Err(Error::LengthTooLarge(value));
    }
    // Record the coding only when it is NOT the minimal one, so a parsed object
    // compares equal to one built with the default form whenever the wire used
    // the shortest legal coding.
    let form = if n == minimal_octets(value) && value >= 0x80 {
        LengthForm::Minimal
    } else {
        LengthForm::Long { octets: n as u8 }
    };
    Ok((value, 1 + n, form))
}

/// Length octets of the minimal long form of `value` (`value >= 0x80`).
const fn minimal_octets(value: usize) -> usize {
    if value <= 0xFF { 1 } else { 2 }
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

/// Number of bytes a `length_field` for `value` occupies when coded in `form`.
///
/// A [`LengthForm::Long`] whose octets cannot hold `value` (or exceed
/// [`MAX_LENGTH_OCTETS`]) is an error at encode time; here it reports the
/// requested size so `serialized_len` stays consistent with what a successful
/// serialize writes.
#[must_use]
pub fn encoded_len_with(value: usize, form: LengthForm) -> usize {
    match form {
        LengthForm::Minimal => encoded_len(value),
        LengthForm::Long { octets } => 1 + octets as usize,
    }
}

/// Encode `value` as a `length_field` coded in `form`, returning the bytes
/// written. [`LengthForm::Minimal`] is exactly [`encode_into`]; a
/// [`LengthForm::Long`] writes the size-indicator byte and `octets` big-endian
/// length octets (so a parsed non-minimal field is reproduced byte-for-byte).
///
/// Errors with [`Error::LengthTooLarge`] when `value > `[`MAX_LENGTH`] and with
/// [`Error::InvalidLength`] when `octets` is `0`, above [`MAX_LENGTH_OCTETS`],
/// or too few to hold `value`.
pub fn encode_with(value: usize, form: LengthForm, buf: &mut [u8]) -> Result<usize> {
    let LengthForm::Long { octets } = form else {
        return encode_into(value, buf);
    };
    if value > MAX_LENGTH {
        return Err(Error::LengthTooLarge(value));
    }
    if octets == 0 || octets > MAX_LENGTH_OCTETS {
        return Err(Error::InvalidLength("length_field_size must be 1..=4"));
    }
    let n = octets as usize;
    if n < 4 && value >> (8 * n) != 0 {
        return Err(Error::InvalidLength(
            "length value does not fit the recorded length_field_size",
        ));
    }
    let need = 1 + n;
    if buf.len() < need {
        return Err(Error::OutputBufferTooSmall {
            need,
            have: buf.len(),
        });
    }
    buf[0] = SIZE_INDICATOR_MASK | octets;
    for (i, slot) in buf[1..need].iter_mut().enumerate() {
        let shift = 8 * (n - 1 - i);
        *slot = (value >> shift) as u8;
    }
    Ok(need)
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
        assert!(decode(&[0x85, 0, 0, 0, 0, 0]).is_err()); // 5 length bytes
        assert!(decode(&[]).is_err());
        assert!(decode(&[0x82, 0x12]).is_err()); // truncated long form
    }

    /// r10-W-4: every wire form of a `length_field` is captured by
    /// `decode_form` and reproduced byte-for-byte by `encode_with`: the
    /// 1-octet short form, `0x81`, `0x82` (minimal and non-minimal) and
    /// `0x83`/`0x84` (always non-minimal for a value capped at `0xFFFF`).
    #[test]
    fn every_form_round_trips_byte_exactly() {
        let cases: &[(&[u8], usize, LengthForm)] = &[
            (&[0x05], 5, LengthForm::Minimal),
            (&[0x7F], 127, LengthForm::Minimal),
            (&[0x81, 0x80], 0x80, LengthForm::Minimal),
            (&[0x81, 0xFF], 0xFF, LengthForm::Minimal),
            (&[0x82, 0x01, 0x00], 0x100, LengthForm::Minimal),
            (&[0x82, 0xFF, 0xFF], 0xFFFF, LengthForm::Minimal),
            // Non-minimal: recorded and reproduced.
            (&[0x81, 0x05], 5, LengthForm::Long { octets: 1 }),
            (&[0x81, 0x00], 0, LengthForm::Long { octets: 1 }),
            // dvb-ci-runtime #337, captured raw on hardware.
            (&[0x82, 0x00, 0x09], 9, LengthForm::Long { octets: 2 }),
            (&[0x82, 0x00, 0x80], 0x80, LengthForm::Long { octets: 2 }),
            (
                &[0x83, 0x00, 0x01, 0x00],
                0x100,
                LengthForm::Long { octets: 3 },
            ),
            (&[0x83, 0x00, 0x00, 0x09], 9, LengthForm::Long { octets: 3 }),
            (
                &[0x84, 0x00, 0x00, 0x00, 0x09],
                9,
                LengthForm::Long { octets: 4 },
            ),
        ];
        for &(wire, value, form) in cases {
            let (v, hdr, f) = decode_form(wire).unwrap();
            assert_eq!((v, hdr, f), (value, wire.len(), form), "{wire:02X?}");
            let mut buf = [0u8; 8];
            let n = encode_with(value, form, &mut buf).unwrap();
            assert_eq!(&buf[..n], wire, "re-encode {wire:02X?}");
            assert_eq!(encoded_len_with(value, form), wire.len());
        }
    }

    /// The default form is the minimal coding (what `encode_into` writes).
    #[test]
    fn minimal_form_matches_encode_into() {
        for v in [0usize, 1, 0x7F, 0x80, 0xFF, 0x100, 0x1234, 0xFFFF] {
            let (mut a, mut b) = ([0u8; 4], [0u8; 4]);
            let na = encode_into(v, &mut a).unwrap();
            let nb = encode_with(v, LengthForm::default(), &mut b).unwrap();
            assert_eq!(a[..na], b[..nb]);
        }
    }

    #[test]
    fn encode_with_rejects_a_form_that_cannot_hold_the_value() {
        let mut buf = [0u8; 8];
        // 0x100 needs 2 octets.
        assert!(encode_with(0x100, LengthForm::Long { octets: 1 }, &mut buf).is_err());
        assert!(encode_with(5, LengthForm::Long { octets: 0 }, &mut buf).is_err());
        assert!(encode_with(5, LengthForm::Long { octets: 5 }, &mut buf).is_err());
        assert!(encode_with(0x1_0000, LengthForm::Long { octets: 4 }, &mut buf).is_err());
    }

    #[test]
    fn rejects_a_value_above_the_spec_cap() {
        // 83 01 00 00 = 0x10000 > 65535.
        assert!(matches!(
            decode_form(&[0x83, 0x01, 0x00, 0x00]),
            Err(Error::LengthTooLarge(0x1_0000))
        ));
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
