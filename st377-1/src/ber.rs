//! BER-encoded KLV Length (ISO/IEC 8825-1 Basic Encoding Rules, as
//! constrained for MXF by `docs/st377-1.md` §6.3.4).
//!
//! - **Short form**: one byte, high bit clear (`0x00`-`0x7F`) — the length
//!   itself.
//! - **Long form**: first byte's high bit set; low 7 bits = the number of
//!   following big-endian length bytes. MXF forbids the "unspecified
//!   length" token (`0x80` alone, i.e. long-form claiming zero following
//!   bytes) and caps the encoding at 9 bytes total (1 header + up to 8
//!   length bytes) — `docs/st377-1.md` §6.3.4.

use crate::error::{Error, Result};

/// The BER length-field width to serialize a value's length with (issue
/// #1047 / audit MX-C1). `docs/st377-1.md` §6.3.4 permits any valid BER
/// form for a length token; real MXF encoders routinely write a longer,
/// fixed-width long form (rather than the shortest one that fits) so a
/// pack can be rewritten in place later without shifting every later
/// absolute offset. Every type that owns a top-level Key-Length-Value
/// envelope (`KlvItem`, `PartitionPack`, `PrimerPack`, `RandomIndexPack`,
/// `LocalSet`) stores one of these, set to `Fixed` from the on-wire width
/// on parse so `serialize_into` reproduces it exactly, and defaulting to
/// `Minimal` for a freshly built value.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[non_exhaustive]
pub enum BerLength {
    /// Always (re)compute the canonical minimal-width encoding for
    /// whatever length is being serialized.
    #[default]
    Minimal,
    /// Reproduce this specific on-wire total token width in bytes: `1` is
    /// short form; `2..=9` is long form with `width - 1` following length
    /// bytes.
    Fixed(core::num::NonZeroU8),
}

impl BerLength {
    /// Build a [`Self::Fixed`] from a length token's total consumed width
    /// (as [`decode_ber_length`] returns it) — always in range since a
    /// successful decode consumes at least 1 and at most 9 bytes.
    #[must_use]
    pub(crate) fn fixed_from_consumed(consumed: usize) -> Self {
        Self::Fixed(core::num::NonZeroU8::new(consumed as u8).unwrap_or(
            // Unreachable in practice (decode_ber_length never returns 0 or
            // >9), but never panics even if that invariant is ever loosened.
            core::num::NonZeroU8::MIN,
        ))
    }
}

/// Decode a BER length token at the start of `bytes`.
///
/// Returns `(length, bytes_consumed_by_the_length_token_itself)`. Does not
/// touch the value bytes that follow.
pub fn decode_ber_length(bytes: &[u8]) -> Result<(u64, usize)> {
    let first = *bytes.first().ok_or(Error::BufferTooShort {
        need: 1,
        have: 0,
        what: "BER length first byte",
    })?;

    if first & 0x80 == 0 {
        // Short form: value is the byte itself.
        return Ok((u64::from(first), 1));
    }

    let following = usize::from(first & 0x7F);
    if following == 0 {
        // 0x80 alone: reserved "unspecified length" — forbidden in MXF.
        return Err(Error::BerIndefiniteLength);
    }
    if following > 8 {
        return Err(Error::BerLengthTooLong { bytes: following });
    }
    if bytes.len() < 1 + following {
        return Err(Error::BufferTooShort {
            need: 1 + following,
            have: bytes.len(),
            what: "BER long-form length bytes",
        });
    }

    let mut value: u64 = 0;
    for &b in &bytes[1..1 + following] {
        value = (value << 8) | u64::from(b);
    }
    Ok((value, 1 + following))
}

/// Number of bytes [`encode_ber_length`] will write for `len`, using the
/// canonical minimal encoding (short form when `len <= 127`, otherwise the
/// shortest long form that fits).
#[must_use]
pub fn ber_length_size(len: u64) -> usize {
    if len <= 0x7F {
        1
    } else {
        let bytes_needed = (64 - len.leading_zeros()).div_ceil(8) as usize;
        1 + bytes_needed
    }
}

/// Encode `len` as a canonical minimal-form BER length into `buf`. Returns
/// the number of bytes written (always [`ber_length_size`]`(len)`).
pub fn encode_ber_length(len: u64, buf: &mut [u8]) -> Result<usize> {
    let size = ber_length_size(len);
    if buf.len() < size {
        return Err(Error::BufferTooShort {
            need: size,
            have: buf.len(),
            what: "BER length output",
        });
    }
    if len <= 0x7F {
        buf[0] = len as u8;
    } else {
        let following = size - 1;
        buf[0] = 0x80 | (following as u8);
        let be = len.to_be_bytes();
        buf[1..size].copy_from_slice(&be[8 - following..]);
    }
    Ok(size)
}

/// The number of bytes [`encode_ber_length_as`] will write for `len` under
/// `width` — [`ber_length_size`]`(len)` for [`BerLength::Minimal`], or the
/// fixed width itself for [`BerLength::Fixed`] (issue #1047).
#[must_use]
pub fn ber_length_size_for(len: u64, width: BerLength) -> usize {
    match width {
        BerLength::Minimal => ber_length_size(len),
        BerLength::Fixed(n) => usize::from(n.get()),
    }
}

/// Encode `len` as a BER length into `buf`, using `width`'s specific
/// on-wire form rather than always recomputing the canonical minimal one
/// (issue #1047 / audit MX-C1 — preserves a real encoder's fixed-width
/// long-form length token across parse -> serialize). Returns the number
/// of bytes written (always [`ber_length_size_for`]`(len, width)`).
///
/// Errors (in addition to `BufferTooShort`, checked first): `Fixed(1)`
/// (short form) when `len > 0x7F`, or `Fixed(n)` (long form, `n - 1`
/// length bytes) when `len` doesn't fit in `n - 1` bytes — both
/// [`Error::BerLengthTooLong`], since the requested fixed width cannot
/// represent this length at all (never produced by parsing a
/// well-formed file; only reachable by building/mutating a value by hand
/// into an inconsistent state).
pub fn encode_ber_length_as(len: u64, width: BerLength, buf: &mut [u8]) -> Result<usize> {
    match width {
        BerLength::Minimal => encode_ber_length(len, buf),
        BerLength::Fixed(n) => {
            let size = usize::from(n.get());
            if buf.len() < size {
                return Err(Error::BufferTooShort {
                    need: size,
                    have: buf.len(),
                    what: "BER length output",
                });
            }
            if size == 1 {
                if len > 0x7F {
                    return Err(Error::FixedBerLengthTooSmall {
                        len,
                        width: n.get(),
                    });
                }
                buf[0] = len as u8;
            } else {
                let following = size - 1;
                if following < 8 && len >= (1u64 << (following * 8)) {
                    return Err(Error::FixedBerLengthTooSmall {
                        len,
                        width: n.get(),
                    });
                }
                buf[0] = 0x80 | (following as u8);
                let be = len.to_be_bytes();
                buf[1..size].copy_from_slice(&be[8 - following..]);
            }
            Ok(size)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn short_form_round_trip() {
        for len in [0u64, 1, 42, 127] {
            let mut buf = [0u8; 9];
            let n = encode_ber_length(len, &mut buf).unwrap();
            assert_eq!(n, 1);
            let (decoded, consumed) = decode_ber_length(&buf[..n]).unwrap();
            assert_eq!(decoded, len);
            assert_eq!(consumed, 1);
        }
    }

    #[test]
    fn long_form_round_trip() {
        for len in [128u64, 255, 256, 65535, 65536, u32::MAX as u64, u64::MAX] {
            let mut buf = [0u8; 9];
            let n = encode_ber_length(len, &mut buf).unwrap();
            assert!(n >= 2);
            let (decoded, consumed) = decode_ber_length(&buf[..n]).unwrap();
            assert_eq!(decoded, len);
            assert_eq!(consumed, n);
        }
    }

    #[test]
    fn minimal_long_form_encoding() {
        // 128 needs exactly one following byte.
        let mut buf = [0u8; 9];
        let n = encode_ber_length(128, &mut buf).unwrap();
        assert_eq!(n, 2);
        assert_eq!(buf[0], 0x81);
        assert_eq!(buf[1], 128);
    }

    #[test]
    fn indefinite_length_rejected() {
        assert_eq!(
            decode_ber_length(&[0x80]).unwrap_err(),
            Error::BerIndefiniteLength
        );
    }

    #[test]
    fn overlong_form_rejected() {
        // 0x89 claims 9 following bytes, exceeding the 8-byte cap.
        assert_eq!(
            decode_ber_length(&[0x89, 0, 0, 0, 0, 0, 0, 0, 0, 0]).unwrap_err(),
            Error::BerLengthTooLong { bytes: 9 }
        );
    }

    #[test]
    fn truncated_long_form_rejected() {
        assert!(matches!(
            decode_ber_length(&[0x84, 0, 0]).unwrap_err(),
            Error::BufferTooShort { .. }
        ));
    }

    #[test]
    fn empty_buffer_rejected() {
        assert!(matches!(
            decode_ber_length(&[]).unwrap_err(),
            Error::BufferTooShort { .. }
        ));
    }
}
