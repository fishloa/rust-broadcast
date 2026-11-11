//! BER-encoded KLV Length (ISO/IEC 8825-1 Basic Encoding Rules, as
//! constrained for MXF by `private/specs-verified/smpte/st377-1/st377-1.md` §6.3.4).
//!
//! - **Short form**: one byte, high bit clear (`0x00`-`0x7F`) — the length
//!   itself.
//! - **Long form**: first byte's high bit set; low 7 bits = the number of
//!   following big-endian length bytes. MXF forbids the "unspecified
//!   length" token (`0x80` alone, i.e. long-form claiming zero following
//!   bytes) and caps the encoding at 9 bytes total (1 header + up to 8
//!   length bytes) — `private/specs-verified/smpte/st377-1/st377-1.md` §6.3.4.

use crate::error::BufferTooShort;
use crate::klv::{BerLength, KlvError};
use crate::len::{need_in, need_out};

/// Result alias for the BER length codec.
pub type Result<T> = core::result::Result<T, KlvError>;

/// High bit of the first BER length byte: set means long form (X.690 §8.1.3).
pub(crate) const BER_LONG_FORM_FLAG: u8 = 0x80;
/// Low 7 bits of a long-form first byte: the count of following length bytes.
pub(crate) const BER_LONG_FORM_COUNT_MASK: u8 = 0x7F;
/// Largest length representable in short form (one byte, high bit clear).
pub(crate) const BER_SHORT_FORM_MAX: u64 = 0x7F;
/// Longest BER length token: 1 header byte + 8 length bytes (ST 377-1 §6.3.4).
pub(crate) const BER_MAX_TOKEN_LEN: usize = 9;
/// Longest long form: the following length bytes of a full-width token.
pub(crate) const BER_MAX_LONG_FORM_BYTES: usize = BER_MAX_TOKEN_LEN - 1;

/// Decode a BER length token at the start of `bytes`.
///
/// Returns `(length, bytes_consumed_by_the_length_token_itself)`. Does not
/// touch the value bytes that follow.
pub fn decode_ber_length(bytes: &[u8]) -> Result<(u64, usize)> {
    let first = *bytes
        .first()
        .ok_or(KlvError::BufferTooShort(BufferTooShort {
            need: 1,
            have: 0,
            what: "BER length first byte",
        }))?;

    if first & BER_LONG_FORM_FLAG == 0 {
        // Short form: value is the byte itself.
        return Ok((u64::from(first), 1));
    }

    let following = usize::from(first & BER_LONG_FORM_COUNT_MASK);
    if following == 0 {
        // 0x80 alone: reserved "unspecified length" — forbidden in MXF.
        return Err(KlvError::BerIndefiniteLength);
    }
    if following > BER_MAX_LONG_FORM_BYTES {
        return Err(KlvError::BerLengthTooLong { bytes: following });
    }
    need_in(bytes, 1 + following, "BER long-form length bytes")?;

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
    if len <= BER_SHORT_FORM_MAX {
        1
    } else {
        let bytes_needed =
            usize::try_from((64 - len.leading_zeros()).div_ceil(8)).unwrap_or(usize::MAX);
        1 + bytes_needed
    }
}

/// Encode `len` as a canonical minimal-form BER length into `buf`. Returns
/// the number of bytes written (always [`ber_length_size`]`(len)`).
pub fn encode_ber_length(len: u64, buf: &mut [u8]) -> Result<usize> {
    let size = ber_length_size(len);
    need_out(buf, size)?;
    if len <= BER_SHORT_FORM_MAX {
        buf[0] = u8::try_from(len).unwrap_or(u8::MAX);
    } else {
        let following = size - 1;
        buf[0] = BER_LONG_FORM_FLAG | u8::try_from(following).unwrap_or(u8::MAX);
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
/// Errors (never produced by parsing a well-formed file; only reachable by
/// building/mutating a value by hand into an inconsistent state):
/// - `Fixed(n)` with `n > 9` (more than the 8 following length bytes §6.3.4
///   allows): [`KlvError::BerLengthTooLong`], checked before anything else.
/// - `BufferTooShort`, when `buf` is shorter than the fixed width.
/// - `Fixed(1)` (short form) when `len > 0x7F`, or `Fixed(n)` (long form,
///   `n - 1` length bytes) when `len` doesn't fit in `n - 1` bytes:
///   [`KlvError::FixedBerLengthTooSmall`], since the requested fixed width
///   cannot represent this length.
pub fn encode_ber_length_as(len: u64, width: BerLength, buf: &mut [u8]) -> Result<usize> {
    match width {
        BerLength::Minimal => encode_ber_length(len, buf),
        BerLength::Fixed(n) => {
            let size = usize::from(n.get());
            if size > BER_MAX_TOKEN_LEN {
                return Err(KlvError::BerLengthTooLong { bytes: size - 1 });
            }
            need_out(buf, size)?;
            if size == 1 {
                if len > BER_SHORT_FORM_MAX {
                    return Err(KlvError::FixedBerLengthTooSmall {
                        len,
                        width: n.get(),
                    });
                }
                buf[0] = u8::try_from(len).unwrap_or(u8::MAX);
            } else {
                let following = size - 1;
                if following < BER_MAX_LONG_FORM_BYTES && len >= (1u64 << (following * 8)) {
                    return Err(KlvError::FixedBerLengthTooSmall {
                        len,
                        width: n.get(),
                    });
                }
                buf[0] = BER_LONG_FORM_FLAG | u8::try_from(following).unwrap_or(u8::MAX);
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
    fn fixed_width_over_nine_is_rejected_not_panicking() {
        use core::num::NonZeroU8;
        let mut buf = [0u8; 300];
        for width in [10u8, 11, 64, 255] {
            let w = BerLength::Fixed(NonZeroU8::new(width).unwrap());
            assert_eq!(
                encode_ber_length_as(5, w, &mut buf),
                Err(KlvError::BerLengthTooLong {
                    bytes: usize::from(width) - 1
                }),
                "width {width}"
            );
        }
        // The widest legal token (9 bytes) still encodes.
        let w9 = BerLength::Fixed(NonZeroU8::new(9).unwrap());
        assert_eq!(encode_ber_length_as(5, w9, &mut buf), Ok(9));
        assert_eq!(&buf[..9], &[0x88, 0, 0, 0, 0, 0, 0, 0, 5]);
    }

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
            KlvError::BerIndefiniteLength
        );
    }

    #[test]
    fn overlong_form_rejected() {
        // 0x89 claims 9 following bytes, exceeding the 8-byte cap.
        assert_eq!(
            decode_ber_length(&[0x89, 0, 0, 0, 0, 0, 0, 0, 0, 0]).unwrap_err(),
            KlvError::BerLengthTooLong { bytes: 9 }
        );
    }

    #[test]
    fn truncated_long_form_rejected() {
        assert!(matches!(
            decode_ber_length(&[0x84, 0, 0]).unwrap_err(),
            KlvError::BufferTooShort(..)
        ));
    }

    #[test]
    fn empty_buffer_rejected() {
        assert!(matches!(
            decode_ber_length(&[]).unwrap_err(),
            KlvError::BufferTooShort(..)
        ));
    }

    #[test]
    fn a_fixed_width_too_narrow_for_the_length_is_rejected() {
        use core::num::NonZeroU8;
        let mut buf = [0u8; 16];
        for (len, width) in [(0x80u64, 1u8), (0x100, 2)] {
            let w = BerLength::Fixed(NonZeroU8::new(width).unwrap());
            assert_eq!(
                encode_ber_length_as(len, w, &mut buf),
                Err(KlvError::FixedBerLengthTooSmall { len, width })
            );
        }
    }
}
