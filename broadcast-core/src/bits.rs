//! Big-endian, MSB-first bit reader/writer for dense sub-byte wire fields.
//!
//! DVB physical-layer signalling (e.g. the EN 302 755 §7.2 L1-pre / L1-post
//! tables) packs many fields that are not byte-aligned — 1-bit flags, 3-bit
//! codes, 18-bit sizes — back-to-back with no padding. [`BitReader`] and
//! [`BitWriter`] walk such a stream a field at a time, most-significant bit
//! first within each field and within each byte, which is the bit order used
//! throughout the DVB/MPEG specifications.
//!
//! Both sides are symmetric: bits written by [`BitWriter::write_bits`] read
//! back identically through [`BitReader::read_bits`]. The writer sets *and*
//! clears each target bit, so the destination buffer need not be pre-zeroed.
//!
//! ```
//! use broadcast_core::bits::{BitReader, BitWriter};
//!
//! let mut buf = [0u8; 2];
//! let mut w = BitWriter::new(&mut buf);
//! w.write_bits(0b101, 3).unwrap();   // 3-bit field
//! w.write_bits(0x1FF, 9).unwrap();   // 9-bit field — crosses the byte boundary
//! assert_eq!(w.bits_written(), 12);
//!
//! let mut r = BitReader::new(&buf);
//! assert_eq!(r.read_bits(3).unwrap(), 0b101);
//! assert_eq!(r.read_bits(9).unwrap(), 0x1FF);
//! ```

use alloc::vec::Vec;

use crate::error::BufferTooShort;

/// Bits in one byte: the unit every `BufferTooShort` count is expressed in.
const BITS_PER_BYTE: usize = 8;
/// Index of the most-significant bit within a byte (MSB-first order).
const BYTE_MSB_INDEX: usize = BITS_PER_BYTE - 1;
/// Largest field a single read/write call accepts (`u64` width).
const MAX_FIELD_BITS: u32 = u64::BITS;

/// Error from a [`BitReader`] / [`BitWriter`] operation.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum BitError {
    /// Asked to read/write past the end of the backing buffer.
    OutOfBounds {
        /// Bits the operation needed.
        needed_bits: usize,
        /// Bits actually left in the buffer.
        remaining_bits: usize,
    },
    /// Requested a field wider than the 64-bit carrier.
    TooManyBits {
        /// The over-wide width requested.
        requested: u32,
    },
    /// A value passed to [`BitWriter::write_bits`] does not fit in `bits` bits.
    ValueTooWide {
        /// The offending value.
        value: u64,
        /// The field width it was asked to fit in.
        bits: u32,
    },
}

impl core::fmt::Display for BitError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            BitError::OutOfBounds {
                needed_bits,
                remaining_bits,
            } => write!(
                f,
                "bit buffer out of bounds: need {} bit(s), {} remaining",
                needed_bits, remaining_bits
            ),
            BitError::TooManyBits { requested } => {
                write!(f, "requested {} bits exceeds the 64-bit carrier", requested)
            }
            BitError::ValueTooWide { value, bits } => {
                write!(f, "value {:#x} does not fit in {} bit(s)", value, bits)
            }
        }
    }
}
impl core::error::Error for BitError {}

impl From<BitError> for BufferTooShort {
    /// The one conversion from a bit-level failure to the workspace's
    /// [`BufferTooShort`], which counts **bytes** (its `Display` says
    /// "bytes"), never bits. [`BitError::OutOfBounds`] carries the bits a
    /// read needed and the bits that were left, and an adapter may report
    /// either as a width (`need 17 bits, 6 left`) or as a position (`needed
    /// 17, 8 remaining`), so both are read the same way: `need` is the larger
    /// rounded up to a whole byte and `have` the smaller rounded down. A
    /// partial byte therefore never counts as a whole one available, and a
    /// read that needs one more bit asks for one more byte.
    ///
    /// A [`BitError::TooManyBits`] or [`BitError::ValueTooWide`] is not a
    /// short buffer and becomes `need: 0, have: 0`: a caller with a typed
    /// "wider than 64 bits" / "value does not fit" error converts that
    /// itself, and one that does not still gets a `BufferTooShort` naming no
    /// real shortfall rather than a fabricated byte count.
    fn from(error: BitError) -> Self {
        match error {
            BitError::OutOfBounds {
                needed_bits,
                remaining_bits,
            } => BufferTooShort {
                need: needed_bits.max(remaining_bits).div_ceil(BITS_PER_BYTE),
                have: needed_bits.min(remaining_bits) / BITS_PER_BYTE,
                what: "",
            },
            BitError::TooManyBits { .. } | BitError::ValueTooWide { .. } => BufferTooShort {
                need: 0,
                have: 0,
                what: "",
            },
        }
    }
}

/// Reads fields MSB-first from a borrowed byte slice.
#[derive(Debug, Clone)]
pub struct BitReader<'a> {
    data: &'a [u8],
    /// Absolute bit cursor from the start of `data` (0 = MSB of byte 0).
    bit_pos: usize,
}

impl<'a> BitReader<'a> {
    /// Create a reader positioned at the first bit of `data`.
    #[must_use]
    pub fn new(data: &'a [u8]) -> Self {
        Self { data, bit_pos: 0 }
    }

    /// Create a reader positioned `bit_pos` bits into `data`.
    ///
    /// # Errors
    /// [`BitError::OutOfBounds`] if `bit_pos` is past the end of `data`.
    pub fn starting_at(data: &'a [u8], bit_pos: usize) -> Result<Self, BitError> {
        let mut reader = Self::new(data);
        reader.skip_bits(bit_pos)?;
        Ok(reader)
    }

    /// Total bits in the backing buffer.
    #[must_use]
    pub fn total_bits(&self) -> usize {
        self.data.len() * BITS_PER_BYTE
    }

    /// Bits already consumed.
    #[must_use]
    pub fn bits_read(&self) -> usize {
        self.bit_pos
    }

    /// Bits not yet consumed.
    #[must_use]
    pub fn bits_remaining(&self) -> usize {
        self.total_bits() - self.bit_pos
    }

    /// `true` if the cursor sits on a byte boundary.
    #[must_use]
    pub fn is_byte_aligned(&self) -> bool {
        self.bit_pos.is_multiple_of(BITS_PER_BYTE)
    }

    /// Read `n` bits (`0..=64`) MSB-first into the low bits of a `u64`.
    ///
    /// `read_bits(0)` returns `0` and consumes nothing.
    ///
    /// # Errors
    /// [`BitError::TooManyBits`] if `n > 64`; [`BitError::OutOfBounds`] if
    /// fewer than `n` bits remain.
    pub fn read_bits(&mut self, n: u32) -> Result<u64, BitError> {
        if n > MAX_FIELD_BITS {
            return Err(BitError::TooManyBits { requested: n });
        }
        if n == 0 {
            return Ok(0);
        }
        let need = n as usize;
        let remaining = self.bits_remaining();
        if need > remaining {
            return Err(BitError::OutOfBounds {
                needed_bits: need,
                remaining_bits: remaining,
            });
        }
        let mut value: u64 = 0;
        for _ in 0..n {
            let byte = self.data[self.bit_pos / BITS_PER_BYTE];
            let bit_index = BYTE_MSB_INDEX - (self.bit_pos % BITS_PER_BYTE); // MSB first within the byte
            let bit = (byte >> bit_index) & 1;
            value = (value << 1) | u64::from(bit);
            self.bit_pos += 1;
        }
        Ok(value)
    }

    /// Read a single bit as a `bool` (`1` → `true`).
    ///
    /// # Errors
    /// [`BitError::OutOfBounds`] if no bits remain.
    pub fn read_bool(&mut self) -> Result<bool, BitError> {
        Ok(self.read_bits(1)? != 0)
    }

    /// Skip `n` bits without interpreting them.
    ///
    /// # Errors
    /// [`BitError::OutOfBounds`] if fewer than `n` bits remain. The cursor is
    /// then left at the end of the buffer, never past it, so `bits_remaining`
    /// stays a real count after a failed skip.
    pub fn skip_bits(&mut self, n: usize) -> Result<(), BitError> {
        let remaining = self.bits_remaining();
        if n > remaining {
            self.bit_pos = self.total_bits();
            return Err(BitError::OutOfBounds {
                needed_bits: n,
                remaining_bits: remaining,
            });
        }
        self.bit_pos += n;
        Ok(())
    }

    /// Advance to the next byte boundary (no-op if already aligned).
    pub fn align_to_byte(&mut self) {
        let rem = self.bit_pos % BITS_PER_BYTE;
        if rem != 0 {
            self.bit_pos += BITS_PER_BYTE - rem;
        }
    }
}

/// Writes fields MSB-first into a borrowed mutable byte slice.
///
/// Each bit is explicitly set or cleared, so `buf` need not be zeroed first.
#[derive(Debug)]
pub struct BitWriter<'a> {
    data: &'a mut [u8],
    bit_pos: usize,
}

impl<'a> BitWriter<'a> {
    /// Create a writer positioned at the first bit of `data`.
    #[must_use]
    pub fn new(data: &'a mut [u8]) -> Self {
        Self { data, bit_pos: 0 }
    }

    /// Create a writer positioned `bit_pos` bits into `data`.
    ///
    /// # Errors
    /// [`BitError::OutOfBounds`] if `bit_pos` is past the end of `data`.
    pub fn starting_at(data: &'a mut [u8], bit_pos: usize) -> Result<Self, BitError> {
        let capacity = data.len() * BITS_PER_BYTE;
        if bit_pos > capacity {
            return Err(BitError::OutOfBounds {
                needed_bits: bit_pos,
                remaining_bits: capacity,
            });
        }
        Ok(Self { data, bit_pos })
    }

    /// Total bits the backing buffer can hold.
    #[must_use]
    pub fn capacity_bits(&self) -> usize {
        self.data.len() * BITS_PER_BYTE
    }

    /// Bits written so far.
    #[must_use]
    pub fn bits_written(&self) -> usize {
        self.bit_pos
    }

    /// `true` if the cursor sits on a byte boundary.
    #[must_use]
    pub fn is_byte_aligned(&self) -> bool {
        self.bit_pos.is_multiple_of(BITS_PER_BYTE)
    }

    /// Write the low `n` bits (`0..=64`) of `value`, MSB-first.
    ///
    /// `write_bits(_, 0)` writes nothing.
    ///
    /// # Errors
    /// [`BitError::TooManyBits`] if `n > 64`; [`BitError::ValueTooWide`] if
    /// `value` has bits set above bit `n-1`; [`BitError::OutOfBounds`] if the
    /// buffer cannot hold `n` more bits.
    pub fn write_bits(&mut self, value: u64, n: u32) -> Result<(), BitError> {
        if n > MAX_FIELD_BITS {
            return Err(BitError::TooManyBits { requested: n });
        }
        if n == 0 {
            return Ok(());
        }
        // Reject values that don't fit — catches caller arithmetic bugs.
        if n < MAX_FIELD_BITS && value >= (1u64 << n) {
            return Err(BitError::ValueTooWide { value, bits: n });
        }
        let need = n as usize;
        let remaining = self.capacity_bits() - self.bit_pos;
        if need > remaining {
            return Err(BitError::OutOfBounds {
                needed_bits: need,
                remaining_bits: remaining,
            });
        }
        for i in (0..n).rev() {
            let bit = ((value >> i) & 1) as u8;
            let byte_idx = self.bit_pos / BITS_PER_BYTE;
            let bit_index = BYTE_MSB_INDEX - (self.bit_pos % BITS_PER_BYTE);
            if bit == 1 {
                self.data[byte_idx] |= 1 << bit_index;
            } else {
                self.data[byte_idx] &= !(1u8 << bit_index);
            }
            self.bit_pos += 1;
        }
        Ok(())
    }

    /// Write a single bit from a `bool`.
    ///
    /// # Errors
    /// [`BitError::OutOfBounds`] if the buffer is full.
    pub fn write_bool(&mut self, value: bool) -> Result<(), BitError> {
        self.write_bits(u64::from(value), 1)
    }

    /// Pad with zero bits up to the next byte boundary (no-op if aligned).
    ///
    /// # Errors
    /// [`BitError::OutOfBounds`] if the buffer cannot hold the padding.
    pub fn align_to_byte(&mut self) -> Result<(), BitError> {
        let rem = self.bit_pos % BITS_PER_BYTE;
        if rem != 0 {
            self.write_bits(0, (BITS_PER_BYTE - rem) as u32)?;
        }
        Ok(())
    }
}

/// A growable MSB-first bit writer that owns its bytes.
///
/// The write side of [`BitWriter`] for callers that do not know the output
/// size up front: every field goes through [`BitWriter::write_bits`] over the
/// bytes written so far plus room for the new field, so range and width
/// checks, and bit order, are the slice writer's own.
#[derive(Debug, Clone, Default)]
pub struct BitBuf {
    data: Vec<u8>,
    bit_len: usize,
}

impl BitBuf {
    /// An empty buffer.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Bits written so far.
    #[must_use]
    pub fn bits_written(&self) -> usize {
        self.bit_len
    }

    /// `true` if the cursor sits on a byte boundary.
    #[must_use]
    pub fn is_byte_aligned(&self) -> bool {
        self.bit_len.is_multiple_of(BITS_PER_BYTE)
    }

    /// Append the low `n` bits (`0..=64`) of `value`, MSB-first.
    ///
    /// # Errors
    /// [`BitError::TooManyBits`] if `n > 64`; [`BitError::ValueTooWide`] if
    /// `value` has bits set above bit `n-1`. The buffer is unchanged on error.
    pub fn write_bits(&mut self, value: u64, n: u32) -> Result<(), BitError> {
        if n > MAX_FIELD_BITS {
            return Err(BitError::TooManyBits { requested: n });
        }
        let grown = (self.bit_len + n as usize).div_ceil(BITS_PER_BYTE);
        let previous = self.data.len();
        self.data.resize(grown, 0);
        let written = BitWriter::starting_at(&mut self.data, self.bit_len)
            .and_then(|mut writer| writer.write_bits(value, n));
        if written.is_err() {
            self.data.truncate(previous);
            return written;
        }
        self.bit_len += n as usize;
        Ok(())
    }

    /// Append a single bit from a `bool`.
    ///
    /// # Errors
    /// Never fails in practice; the signature matches [`BitWriter::write_bool`].
    pub fn write_bool(&mut self, value: bool) -> Result<(), BitError> {
        self.write_bits(u64::from(value), 1)
    }

    /// Pad with zero bits up to the next byte boundary (no-op if aligned).
    pub fn align_to_byte(&mut self) {
        self.bit_len = self.data.len() * BITS_PER_BYTE;
    }

    /// Append whole bytes at a byte boundary.
    ///
    /// # Errors
    /// [`BitError::OutOfBounds`] if the cursor is not byte aligned (`needed_bits`
    /// is the bits to the next boundary, `remaining_bits` is `0`); nothing is
    /// written.
    pub fn write_bytes(&mut self, bytes: &[u8]) -> Result<(), BitError> {
        if !self.is_byte_aligned() {
            return Err(BitError::OutOfBounds {
                needed_bits: BITS_PER_BYTE - self.bit_len % BITS_PER_BYTE,
                remaining_bits: 0,
            });
        }
        self.data.extend_from_slice(bytes);
        self.bit_len += bytes.len() * BITS_PER_BYTE;
        Ok(())
    }

    /// The bytes written so far, a final partial byte zero-padded.
    #[must_use]
    pub fn as_bytes(&self) -> &[u8] {
        &self.data
    }

    /// Consume the buffer into its bytes, a final partial byte zero-padded.
    #[must_use]
    pub fn into_vec(self) -> Vec<u8> {
        self.data
    }
}

#[cfg(test)]
mod tests {
    // Binary literals here are grouped by *wire field boundaries* (e.g.
    // `0b1_01_10101` = a 1-bit, 2-bit and 5-bit field) to document the layout,
    // not by nibbles.
    use super::*;

    #[test]
    fn single_byte_fields_round_trip() {
        let mut buf = [0u8; 1];
        let mut w = BitWriter::new(&mut buf);
        w.write_bits(0b1, 1).unwrap();
        w.write_bits(0b01, 2).unwrap();
        w.write_bits(0b10101, 5).unwrap();
        assert_eq!(w.bits_written(), 8);
        // 1 01 10101 = 0b1_01_10101 = 0xB5
        assert_eq!(buf[0], 0b10110101);

        let mut r = BitReader::new(&buf);
        assert_eq!(r.read_bits(1).unwrap(), 0b1);
        assert_eq!(r.read_bits(2).unwrap(), 0b01);
        assert_eq!(r.read_bits(5).unwrap(), 0b10101);
        assert_eq!(r.bits_remaining(), 0);
    }

    #[test]
    fn field_crossing_byte_boundary_round_trips() {
        // An 18-bit field like L1_POST_SIZE, offset so it straddles 3 bytes.
        let mut buf = [0u8; 4];
        let mut w = BitWriter::new(&mut buf);
        w.write_bits(0b101, 3).unwrap();
        w.write_bits(0b10_1010_1010_1010_1011, 18).unwrap();
        let val18 = 0b10_1010_1010_1010_1011u64;

        let mut r = BitReader::new(&buf);
        assert_eq!(r.read_bits(3).unwrap(), 0b101);
        assert_eq!(r.read_bits(18).unwrap(), val18);
    }

    #[test]
    fn read_zero_bits_is_noop() {
        let buf = [0xFFu8];
        let mut r = BitReader::new(&buf);
        assert_eq!(r.read_bits(0).unwrap(), 0);
        assert_eq!(r.bits_read(), 0);
    }

    #[test]
    fn full_64_bit_field() {
        let mut buf = [0u8; 8];
        let value = 0xDEAD_BEEF_CAFE_F00Du64;
        let mut w = BitWriter::new(&mut buf);
        w.write_bits(value, 64).unwrap();
        let mut r = BitReader::new(&buf);
        assert_eq!(r.read_bits(64).unwrap(), value);
    }

    #[test]
    fn read_past_end_errs() {
        let buf = [0xFFu8]; // 8 bits
        let mut r = BitReader::new(&buf);
        r.read_bits(7).unwrap();
        let err = r.read_bits(2).unwrap_err();
        assert_eq!(
            err,
            BitError::OutOfBounds {
                needed_bits: 2,
                remaining_bits: 1,
            }
        );
    }

    #[test]
    fn an_overrun_converts_to_a_byte_shortfall() {
        // Seven bits left, sixteen wanted: two whole bytes needed and none of
        // the seven bits left is a whole one.
        let buf = [0xFFu8];
        let mut r = BitReader::new(&buf);
        r.read_bits(1).unwrap();
        assert_eq!(
            BufferTooShort::from(r.read_bits(16).unwrap_err()),
            BufferTooShort {
                need: 2,
                have: 0,
                what: "",
            }
        );

        // One bit left, one more wanted: a single byte both spans and holds
        // the shortfall, so a partial byte rounds up and down to it.
        let mut r = BitReader::new(&buf);
        r.read_bits(7).unwrap();
        assert_eq!(
            BufferTooShort::from(r.read_bits(2).unwrap_err()),
            BufferTooShort {
                need: 1,
                have: 0,
                what: "",
            }
        );

        // A whole byte available and a two-byte read wanted: the shortfall is
        // one whole byte, so both counts agree on it.
        let buf = [0u8; 2];
        let mut r = BitReader::new(&buf);
        r.read_bits(8).unwrap();
        assert_eq!(
            BufferTooShort::from(r.read_bits(12).unwrap_err()),
            BufferTooShort {
                need: 2,
                have: 1,
                what: "",
            }
        );
    }

    #[test]
    fn a_position_and_a_width_convert_alike() {
        // An adapter reporting the position (12 bits needed) and one reporting
        // the width (12 bits wanted) both mean two bytes.
        for error in [
            BitError::OutOfBounds {
                needed_bits: 12,
                remaining_bits: 8,
            },
            BitError::OutOfBounds {
                needed_bits: 12,
                remaining_bits: 4,
            },
        ] {
            assert_eq!(BufferTooShort::from(error).need, 2);
        }
    }

    #[test]
    fn a_non_shortage_converts_to_no_shortfall() {
        for error in [
            BitError::TooManyBits { requested: 65 },
            BitError::ValueTooWide {
                value: 0b100,
                bits: 2,
            },
        ] {
            assert_eq!(
                BufferTooShort::from(error),
                BufferTooShort {
                    need: 0,
                    have: 0,
                    what: "",
                }
            );
        }
    }

    #[test]
    fn a_failed_skip_leaves_the_cursor_at_the_end() {
        let buf = [0xFFu8];
        let mut r = BitReader::new(&buf);
        r.skip_bits(3).unwrap();
        assert_eq!(
            r.skip_bits(9),
            Err(BitError::OutOfBounds {
                needed_bits: 9,
                remaining_bits: 5,
            })
        );
        assert_eq!(r.bits_remaining(), 0, "never past the end");
        assert_eq!(r.bits_read(), r.total_bits());
    }

    #[test]
    fn read_too_many_bits_errs() {
        let buf = [0u8; 16];
        let mut r = BitReader::new(&buf);
        assert_eq!(
            r.read_bits(65).unwrap_err(),
            BitError::TooManyBits { requested: 65 }
        );
    }

    #[test]
    fn write_value_too_wide_errs() {
        let mut buf = [0u8; 4];
        let mut w = BitWriter::new(&mut buf);
        // 0b100 needs 3 bits; asking for 2 must fail.
        assert_eq!(
            w.write_bits(0b100, 2).unwrap_err(),
            BitError::ValueTooWide {
                value: 0b100,
                bits: 2
            }
        );
    }

    #[test]
    fn write_past_end_errs() {
        let mut buf = [0u8; 1];
        let mut w = BitWriter::new(&mut buf);
        w.write_bits(0, 7).unwrap();
        assert_eq!(
            w.write_bits(0b11, 2).unwrap_err(),
            BitError::OutOfBounds {
                needed_bits: 2,
                remaining_bits: 1,
            }
        );
    }

    #[test]
    fn writer_does_not_require_zeroed_buffer() {
        // Pre-fill with 0xFF; writer must clear bits it writes as 0.
        let mut buf = [0xFFu8; 1];
        let mut w = BitWriter::new(&mut buf);
        w.write_bits(0b0000_0000, 8).unwrap();
        assert_eq!(buf[0], 0x00);
    }

    #[test]
    fn bool_round_trips() {
        let mut buf = [0u8; 1];
        let mut w = BitWriter::new(&mut buf);
        w.write_bool(true).unwrap();
        w.write_bool(false).unwrap();
        w.write_bool(true).unwrap();
        let mut r = BitReader::new(&buf);
        assert!(r.read_bool().unwrap());
        assert!(!r.read_bool().unwrap());
        assert!(r.read_bool().unwrap());
    }

    #[test]
    fn skip_and_align() {
        let buf = [0b1010_1100u8, 0b1111_0000];
        let mut r = BitReader::new(&buf);
        r.read_bits(2).unwrap(); // at bit 2
        r.skip_bits(3).unwrap(); // at bit 5
        assert!(!r.is_byte_aligned());
        r.align_to_byte(); // at bit 8
        assert!(r.is_byte_aligned());
        assert_eq!(r.read_bits(4).unwrap(), 0b1111);
    }

    #[test]
    fn writer_align_pads_with_zero() {
        let mut buf = [0xFFu8; 1];
        let mut w = BitWriter::new(&mut buf);
        w.write_bits(0b101, 3).unwrap();
        w.align_to_byte().unwrap();
        assert_eq!(w.bits_written(), 8);
        assert_eq!(buf[0], 0b1010_0000); // 3 data bits + 5 zero pad
    }

    #[test]
    fn exhaustive_small_width_round_trip() {
        // Every value of every width 1..=16 reads back identically.
        for bits in 1u32..=16 {
            let max = if bits == 64 {
                u64::MAX
            } else {
                (1u64 << bits) - 1
            };
            for value in [0u64, 1, max, max / 2] {
                let mut buf = [0u8; 8];
                let mut w = BitWriter::new(&mut buf);
                w.write_bits(value, bits).unwrap();
                let mut r = BitReader::new(&buf);
                assert_eq!(
                    r.read_bits(bits).unwrap(),
                    value,
                    "round-trip failed: value={value:#x} bits={bits}"
                );
            }
        }
    }

    #[test]
    fn starting_at_resumes_mid_byte() {
        let buf = [0b1010_1100u8, 0b1111_0000];
        let mut r = BitReader::starting_at(&buf, 6).unwrap();
        assert_eq!(r.read_bits(6).unwrap(), 0b00_1111);
        assert_eq!(
            BitReader::starting_at(&buf, 16).unwrap().bits_remaining(),
            0
        );
        assert_eq!(
            BitReader::starting_at(&buf, 17).unwrap_err(),
            BitError::OutOfBounds {
                needed_bits: 17,
                remaining_bits: 16,
            }
        );

        let mut out = [0xFFu8; 2];
        let mut w = BitWriter::starting_at(&mut out, 4).unwrap();
        w.write_bits(0, 8).unwrap();
        assert_eq!(out, [0xF0, 0x0F]);
        let mut out = [0u8; 1];
        assert_eq!(
            BitWriter::starting_at(&mut out, 9).unwrap_err(),
            BitError::OutOfBounds {
                needed_bits: 9,
                remaining_bits: 8,
            }
        );
    }

    #[test]
    fn bit_buf_matches_the_slice_writer() {
        let fields = [
            (0b101u64, 3u32),
            (0x1FF, 9),
            (0, 1),
            (0xDEAD_BEEF, 32),
            (1, 1),
        ];
        let mut buf = BitBuf::new();
        let mut reference = [0u8; 8];
        let mut w = BitWriter::new(&mut reference);
        for (value, n) in fields {
            buf.write_bits(value, n).unwrap();
            w.write_bits(value, n).unwrap();
        }
        assert_eq!(buf.bits_written(), 46);
        assert_eq!(buf.as_bytes(), &reference[..6]);
        assert_eq!(buf.as_bytes(), [0xbf, 0xf6, 0xf5, 0x6d, 0xf7, 0x7c]);
    }

    #[test]
    fn bit_buf_edges() {
        let mut buf = BitBuf::new();
        assert_eq!(
            buf.write_bits(0b100, 2),
            Err(BitError::ValueTooWide {
                value: 0b100,
                bits: 2
            })
        );
        assert_eq!(
            buf.write_bits(0, 65),
            Err(BitError::TooManyBits { requested: 65 })
        );
        assert_eq!(buf.bits_written(), 0);
        assert!(buf.as_bytes().is_empty());

        buf.write_bits(u64::MAX, 64).unwrap();
        assert_eq!(buf.as_bytes(), [0xFF; 8]);
        buf.write_bool(true).unwrap();
        assert!(!buf.is_byte_aligned());
        assert_eq!(
            buf.write_bytes(&[1]),
            Err(BitError::OutOfBounds {
                needed_bits: 7,
                remaining_bits: 0,
            })
        );
        buf.align_to_byte();
        assert!(buf.is_byte_aligned());
        buf.write_bytes(&[0xAA]).unwrap();
        assert_eq!(buf.bits_written(), 80);
        assert_eq!(
            buf.into_vec(),
            [0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0x80, 0xAA]
        );
    }
}
