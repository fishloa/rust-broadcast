//! Exp-Golomb bit reader over an RBSP byte stream.
//!
//! RBSP (Raw Byte Sequence Payload) requires **emulation prevention byte removal**
//! before parsing: every `00 00 03` triplet in the NAL unit byte stream is
//! replaced by `00 00` (the `03` is discarded).  This is done once when
//! constructing the reader via [`BitReader::with_unescape`].
//!
//! Supports `ue(v)` (unsigned) and `se(v)` (signed) Exp-Golomb coding per
//! ITU-T H.264 §9.1 and H.265 §9.2.2. The bit extraction itself is
//! [`broadcast_core::bits::BitReader`]; this type adds the owning RBSP buffer
//! and the typed errors.

use alloc::vec::Vec;
use broadcast_core::bits::{BitError, BitWriter};
use broadcast_core::error::{BufferTooShort, InvalidValue};

use crate::error::{Error, Result};

/// Widest field [`read_bits_at`] and [`BitReader::read_bits`] accept.
const MAX_READ_BITS: usize = u64::BITS as usize;

/// Read `n` (<= 64) bits MSB-first from `data` at `*bit_pos`, advancing it.
///
/// `None` when `n > 64` or fewer than `n` bits remain; `*bit_pos` is then left
/// unchanged.
pub fn read_bits_at(data: &[u8], bit_pos: &mut usize, n: usize) -> Option<u64> {
    let n_u32 = u32::try_from(n).ok().filter(|&n| n <= u64::BITS)?;
    let mut br = broadcast_core::bits::BitReader::starting_at(data, *bit_pos).ok()?;
    let val = br.read_bits(n_u32).ok()?;
    *bit_pos += n;
    Some(val)
}

/// [`read_bits_at`] with this crate's typed errors: `n > 64` is
/// [`Error::InvalidValue`], running out of data is [`Error::BufferTooShort`].
pub fn read_bits_checked(
    data: &[u8],
    bit_pos: &mut usize,
    n: usize,
    what: &'static str,
) -> Result<u64> {
    if n > MAX_READ_BITS {
        return Err(Error::InvalidValue(InvalidValue {
            field: what,
            value: n as u64,
            reason: "bit count > 64",
        }));
    }
    read_bits_at(data, bit_pos, n).ok_or(Error::BufferTooShort(BufferTooShort {
        need: bytes_needed(*bit_pos, n),
        have: data.len(),
        what,
    }))
}

/// Bytes needed to hold `n` more bits starting at `bit_pos` (saturating, for
/// the `need` field of a `BufferTooShort`).
pub fn bytes_needed(bit_pos: usize, n: usize) -> usize {
    bit_pos.saturating_add(n).div_ceil(u8::BITS as usize)
}

/// Maps a shared [`BitError`] onto this crate's typed errors: a short buffer
/// goes through the workspace's one `BitError` -> `BufferTooShort` conversion
/// (whole bytes), anything else is an `InvalidValue`.
pub fn bit_error(what: &'static str, e: BitError) -> Error {
    match e {
        BitError::OutOfBounds { .. } => {
            let mut short: BufferTooShort = e.into();
            short.what = what;
            Error::BufferTooShort(short)
        }
        BitError::TooManyBits { requested } => Error::InvalidValue(InvalidValue {
            field: what,
            value: u64::from(requested),
            reason: "field wider than 64 bits",
        }),
        BitError::ValueTooWide { value, .. } => Error::InvalidValue(InvalidValue {
            field: what,
            value,
            reason: "value does not fit the field width",
        }),
        _ => Error::InvalidValue(InvalidValue {
            field: what,
            value: 0,
            reason: "bit reader error",
        }),
    }
}

/// Writes the low `n` bits of `value` after a `fit_bits` range check, with
/// this crate's typed errors.
pub fn write_bits_checked(
    w: &mut BitWriter<'_>,
    value: u64,
    n: u32,
    what: &'static str,
) -> Result<()> {
    let value = broadcast_core::len::fit_bits(value, n, what)?;
    w.write_bits(value, n).map_err(|e| bit_error(what, e))
}

/// Bit-level reader over an RBSP buffer (after emulation-prevention byte removal).
///
/// Reads bits from left to right within each byte (MSB-first, big-endian
/// bit numbering — ITU-T H.264 §7.2).
///
/// The struct owns the RBSP `Vec<u8>` and tracks the current bit position.
#[non_exhaustive]
pub struct BitReader {
    data: Vec<u8>,
    bit_pos: usize,
}

impl BitReader {
    /// Create a new reader from already-unescaped RBSP bytes (no lifetime
    /// entanglement — the data is copied into the reader).
    pub fn from_rbsp(data: &[u8], what: &'static str) -> Result<Self> {
        if data.is_empty() {
            return Err(Error::BufferTooShort(BufferTooShort {
                need: 1,
                have: 0,
                what,
            }));
        }
        Ok(Self {
            data: data.to_vec(),
            bit_pos: 0,
        })
    }

    /// Create a new reader from a NAL unit body (after the NAL header),
    /// with emulation-prevention byte removal.
    pub fn with_unescape(nal_body: &[u8], what: &'static str) -> Result<Self> {
        let rbsp = crate::rbsp::unescape(nal_body);
        if rbsp.is_empty() {
            return Err(Error::BufferTooShort(BufferTooShort {
                need: 1,
                have: 0,
                what,
            }));
        }
        Ok(Self {
            data: rbsp,
            bit_pos: 0,
        })
    }

    fn has_bits(&self, n: usize) -> bool {
        self.bit_pos + n <= self.data.len() * u8::BITS as usize
    }

    /// Read `n` bits as an unsigned integer (`u(n)` / `f(n)`).
    ///
    /// An over-wide field (`n > 64`) is [`Error::InvalidValue`], the same
    /// class the crate's `BitError` conversion gives `BitError::TooManyBits`;
    /// only a genuinely short buffer is [`Error::BufferTooShort`].
    pub fn read_bits(&mut self, n: usize, what: &'static str) -> Result<u64> {
        if n > MAX_READ_BITS {
            return Err(Error::InvalidValue(InvalidValue {
                field: what,
                value: n as u64,
                reason: "field wider than 64 bits",
            }));
        }
        if !self.has_bits(n) {
            return Err(self.short(n, what));
        }
        if n == 0 {
            return Ok(0);
        }
        let mut reader = broadcast_core::bits::BitReader::starting_at(&self.data, self.bit_pos)
            .map_err(|_| self.short(n, what))?;
        let val = reader
            .read_bits(u32::try_from(n).map_err(|_| self.short(n, what))?)
            .map_err(|_| self.short(n, what))?;
        self.bit_pos = reader.bits_read();
        Ok(val)
    }

    /// Running out of data with `n` bits wanted from the current position:
    /// `need` is the size of the buffer the read would fit in, `have` the
    /// buffer's own size, both in whole bytes (a partial byte rounds up),
    /// named for `what`.
    fn short(&self, n: usize, what: &'static str) -> Error {
        Error::BufferTooShort(BufferTooShort {
            need: self.bit_pos.saturating_add(n).div_ceil(u8::BITS as usize),
            have: self.data.len(),
            what,
        })
    }

    /// Read one bit as a `bool`.
    pub fn read_flag(&mut self, what: &'static str) -> Result<bool> {
        Ok(self.read_bits(1, what)? != 0)
    }

    /// Consume padding bits up to the next byte boundary (e.g.
    /// `gci_alignment_zero_bit` / `ptl_reserved_zero_bit`, H.266 §7.3.3).
    pub fn align_to_byte(&mut self, what: &'static str) -> Result<()> {
        while !self.bit_pos.is_multiple_of(u8::BITS as usize) {
            let _ = self.read_bits(1, what)?;
        }
        Ok(())
    }

    /// Parse `ue(v)` — unsigned integer Exp-Golomb-coded syntax element.
    ///
    /// H.264 §9.1: leadingZeroBits (count of zero bits before the first 1-bit),
    /// then read that many bits as the unsigned value `codeNum`.
    pub fn read_ue(&mut self, what: &'static str) -> Result<u64> {
        // H.264 §9.1 / H.265 §9.2.2: a `ue(v)` is a leadingZeroBits run
        // terminated by a 1-bit — with no bits left there is no codeNum to
        // decode, so EOF must be an error. Returning `Ok(0)` here does not
        // consume anything, so every caller that loops on the decoded value
        // never terminates.
        if !self.has_bits(1) {
            return Err(self.short(1, what));
        }
        let mut leading_zero_bits: u32 = 0;
        while self.read_bits(1, what)? == 0 {
            leading_zero_bits += 1;
            // H.264 §9.1 / H.265 §9.2.2 bound codeNum (leadingZeroBits ≤ 32);
            // a longer run is not a valid `ue(v)`.
            if leading_zero_bits > u32::BITS {
                return Err(Error::InvalidValue(InvalidValue {
                    field: what,
                    value: leading_zero_bits as u64,
                    reason: "ue(v) with more than 32 leading zero bits",
                }));
            }
            if !self.has_bits(1) {
                return Err(self.short(1, what));
            }
        }
        // The loop exits just past the terminating 1-bit; the
        // `leading_zero_bits` info bits must still be present.
        if !self.has_bits(leading_zero_bits as usize) {
            return Err(self.short(leading_zero_bits as usize, what));
        }
        let info = self.read_bits(leading_zero_bits as usize, what)?;
        Ok((1u64 << leading_zero_bits) - 1 + info)
    }

    /// Parse `se(v)` — signed integer Exp-Golomb-coded syntax element.
    ///
    /// H.264 §9.1.1: mapping from `codeNum` to signed value.
    pub fn read_se(&mut self, what: &'static str) -> Result<i64> {
        let code_num = self.read_ue(what)?;
        if code_num & 1 == 0 {
            Ok(-((code_num >> 1) as i64))
        } else {
            Ok(((code_num + 1) >> 1) as i64)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::vec;

    /// Drift pin (#1141): the shared checked reader rejects `n > 64` as
    /// `InvalidValue` (DTS used to report it as a short buffer) and reports a
    /// short buffer as `BufferTooShort`.
    #[test]
    fn read_bits_checked_distinguishes_oversize_from_short_buffer() {
        let data = [0xA5u8, 0x5A];
        let mut pos = 0usize;
        assert!(matches!(
            read_bits_checked(&data, &mut pos, 65, "x"),
            Err(Error::InvalidValue(..))
        ));
        assert_eq!(read_bits_checked(&data, &mut pos, 4, "x").unwrap(), 0xA);
        assert!(matches!(
            read_bits_checked(&data, &mut pos, 16, "x"),
            Err(Error::BufferTooShort(..))
        ));
        assert_eq!(pos, 4, "a failed read leaves the cursor unchanged");
    }

    /// A full 64-bit field is legal and reads the first eight bytes big-endian;
    /// over 64 bits is refused.
    #[test]
    fn a_64_bit_field_reads_and_a_65_bit_request_is_refused() {
        let data = [1u8, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16];
        let expected = u64::from_be_bytes([1, 2, 3, 4, 5, 6, 7, 8]);
        let mut pos = 0usize;
        assert_eq!(
            read_bits_checked(&data, &mut pos, 64, "x").unwrap(),
            expected
        );
        assert_eq!(pos, 64);

        let mut reader = BitReader::from_rbsp(&data, "x").unwrap();
        assert_eq!(reader.read_bits(64, "x").unwrap(), expected);

        let mut reader = BitReader::from_rbsp(&data, "x").unwrap();
        let refused = reader.read_bits(65, "x").unwrap_err();
        let Error::InvalidValue(invalid) = refused else {
            panic!("an over-wide field is InvalidValue, not a short buffer: {refused:?}");
        };
        assert_eq!(invalid.field, "x");
        assert_eq!(invalid.value, 65);
        assert_eq!(invalid.reason, "field wider than 64 bits");
        assert_eq!(reader.bit_pos, 0, "a refused read leaves the cursor alone");
    }

    #[test]
    fn bit_error_carries_the_offending_value_and_reason() {
        for (err, value, reason) in [
            (
                BitError::TooManyBits { requested: 65 },
                65,
                "field wider than 64 bits",
            ),
            (
                BitError::ValueTooWide {
                    value: 300,
                    bits: 8,
                },
                300,
                "value does not fit the field width",
            ),
        ] {
            let Error::InvalidValue(invalid) = bit_error("vvcC", err) else {
                panic!("expected InvalidValue");
            };
            assert_eq!(invalid.field, "vvcC");
            assert_eq!(invalid.value, value);
            assert_eq!(invalid.reason, reason);
        }
    }

    /// Every short-buffer path here reports `need`/`have` in whole bytes,
    /// through the workspace's one `BitError` -> `BufferTooShort` conversion.
    #[test]
    fn the_rbsp_reader_reports_bytes() {
        let mut reader = BitReader::from_rbsp(&[0xFF], "RbspBitReader").expect("one byte");
        assert_eq!(reader.read_bits(8, "byte").expect("whole byte"), 0xFF);
        let Error::BufferTooShort(short) = reader.read_bits(12, "twelve_bits").expect_err("eof")
        else {
            panic!("expected a short buffer");
        };
        assert_eq!(
            short.need, 3,
            "the read starts at the end of the one-byte buffer and spans two more"
        );
        assert_eq!(short.have, 1, "one byte is present");
    }

    #[test]
    fn exp_golomb_across_byte_boundaries_and_the_32_zero_bound() {
        // ue: 1 | 010 | 011 | 00100 | 00101 | 00110 | 00111, se: -1 as 011
        let mut r =
            BitReader::from_rbsp(&[0b1010_0110, 0b0100_0010, 0b1001_1000, 0b1110_0000], "t")
                .unwrap();
        for expected in [0, 1, 2, 3, 4, 5, 6] {
            assert_eq!(r.read_ue("t").unwrap(), expected);
        }
        let mut r = BitReader::from_rbsp(&[0b0110_0100, 0b0000_0000], "t").unwrap();
        assert_eq!(r.read_se("t").unwrap(), -1);
        assert_eq!(r.read_se("t").unwrap(), 2);
        let mut r = BitReader::from_rbsp(&[0, 0, 0, 0, 0x80], "t").unwrap();
        assert_eq!(
            r.read_ue("t").unwrap_err(),
            Error::BufferTooShort(BufferTooShort {
                need: 9,
                have: 5,
                what: "t",
            })
        );
        let mut r = BitReader::from_rbsp(&[0x00, 0x00, 0x00, 0x00, 0x80, 0, 0, 0, 0], "t").unwrap();
        assert_eq!(r.read_ue("t").unwrap(), u64::from(u32::MAX));
    }

    #[test]
    fn a_read_straddling_the_end_fails_and_keeps_the_cursor() {
        let mut r = BitReader::from_rbsp(&[0xF0, 0x0F], "t").unwrap();
        assert_eq!(r.read_bits(4, "t").unwrap(), 0xF);
        assert!(r.read_bits(13, "t").is_err());
        assert_eq!(r.read_bits(12, "t").unwrap(), 0x00F);
        assert!(r.read_flag("t").is_err());
        r.align_to_byte("t").unwrap();
    }

    #[test]
    fn unescape_removes_emulation_prevention_bytes() {
        let nal = [0x00, 0x00, 0x03, 0x04, 0x00, 0x00];
        let rbsp = crate::rbsp::unescape(&nal);
        assert_eq!(rbsp, &[0x00, 0x00, 0x04, 0x00, 0x00]);
    }

    #[test]
    fn unescape_no_epb_passthrough() {
        let nal = [0x01, 0x02, 0x03, 0x04];
        let rbsp = crate::rbsp::unescape(&nal);
        assert_eq!(rbsp, nal);
    }

    #[test]
    fn ue_simple() {
        let mut r = BitReader::from_rbsp(&[0x80], "test").unwrap();
        assert_eq!(r.read_ue("test").unwrap(), 0);
    }

    #[test]
    fn ue_value_3() {
        let mut r = BitReader::from_rbsp(&[0x20], "test").unwrap();
        assert_eq!(r.read_ue("test").unwrap(), 3);
    }

    #[test]
    fn se_signed() {
        let mut r = BitReader::from_rbsp(&[0x80], "test").unwrap();
        assert_eq!(r.read_se("test").unwrap(), 0);

        let mut r = BitReader::from_rbsp(&[0x40], "test").unwrap();
        assert_eq!(r.read_se("test").unwrap(), 1);

        let mut r = BitReader::from_rbsp(&[0x60], "test").unwrap();
        assert_eq!(r.read_se("test").unwrap(), -1);
    }

    /// H.264 §9.1: `ue(v)` is a leadingZeroBits run terminated by a 1-bit —
    /// with no bits left there is no codeNum to decode, so EOF must be `Err`.
    /// Pre-fix it returned `Ok(0)` **without consuming anything**, and every
    /// caller that loops on a decoded count spun forever.
    #[test]
    fn read_ue_at_eof_is_error() {
        // Eight 1-bits: eight `ue(v) == 0`, then the reader is empty.
        let mut r = BitReader::from_rbsp(&[0xFF], "test").unwrap();
        for _ in 0..8 {
            assert_eq!(r.read_ue("test").unwrap(), 0);
        }
        assert!(matches!(
            r.read_ue("ue at EOF"),
            Err(crate::error::Error::BufferTooShort(..))
        ));
    }

    /// H.264 §9.1 / H.265 §9.2.2 bound `leadingZeroBits` (codeNum < 2^33):
    /// more than 32 leading zeros cannot be a valid `ue(v)`.
    #[test]
    fn read_ue_beyond_32_leading_zeros_is_error() {
        // 40 leading zero bits, then a 1-bit and 40 info bits — enough tail
        // (88 bits) that the pre-fix reader returned `Ok(~2^41)` rather than
        // hitting EOF.
        let mut rbsp = vec![0x00u8; 5];
        rbsp.extend_from_slice(&[0xFFu8; 6]);
        let mut r = BitReader::from_rbsp(&rbsp, "test").unwrap();
        assert!(matches!(
            r.read_ue("huge ue"),
            Err(crate::error::Error::InvalidValue(..))
        ));
    }
}
