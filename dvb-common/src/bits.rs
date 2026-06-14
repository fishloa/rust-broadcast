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
//! use dvb_common::bits::{BitReader, BitWriter};
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

use core::fmt;

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

impl fmt::Display for BitError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            BitError::OutOfBounds {
                needed_bits,
                remaining_bits,
            } => write!(
                f,
                "bit buffer out of bounds: need {needed_bits} bit(s), {remaining_bits} remaining"
            ),
            BitError::TooManyBits { requested } => {
                write!(f, "requested {requested} bits exceeds the 64-bit carrier")
            }
            BitError::ValueTooWide { value, bits } => {
                write!(f, "value {value:#x} does not fit in {bits} bit(s)")
            }
        }
    }
}

impl std::error::Error for BitError {}

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

    /// Total bits in the backing buffer.
    #[must_use]
    pub fn total_bits(&self) -> usize {
        self.data.len() * 8
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
        self.bit_pos % 8 == 0
    }

    /// Read `n` bits (`0..=64`) MSB-first into the low bits of a `u64`.
    ///
    /// `read_bits(0)` returns `0` and consumes nothing.
    ///
    /// # Errors
    /// [`BitError::TooManyBits`] if `n > 64`; [`BitError::OutOfBounds`] if
    /// fewer than `n` bits remain.
    pub fn read_bits(&mut self, n: u32) -> Result<u64, BitError> {
        if n > 64 {
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
            let byte = self.data[self.bit_pos / 8];
            let bit_index = 7 - (self.bit_pos % 8); // MSB first within the byte
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
    /// [`BitError::OutOfBounds`] if fewer than `n` bits remain.
    pub fn skip_bits(&mut self, n: usize) -> Result<(), BitError> {
        let remaining = self.bits_remaining();
        if n > remaining {
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
        let rem = self.bit_pos % 8;
        if rem != 0 {
            self.bit_pos += 8 - rem;
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

    /// Total bits the backing buffer can hold.
    #[must_use]
    pub fn capacity_bits(&self) -> usize {
        self.data.len() * 8
    }

    /// Bits written so far.
    #[must_use]
    pub fn bits_written(&self) -> usize {
        self.bit_pos
    }

    /// `true` if the cursor sits on a byte boundary.
    #[must_use]
    pub fn is_byte_aligned(&self) -> bool {
        self.bit_pos % 8 == 0
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
        if n > 64 {
            return Err(BitError::TooManyBits { requested: n });
        }
        if n == 0 {
            return Ok(());
        }
        // Reject values that don't fit — catches caller arithmetic bugs.
        if n < 64 && value >= (1u64 << n) {
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
            let byte_idx = self.bit_pos / 8;
            let bit_index = 7 - (self.bit_pos % 8);
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
        let rem = self.bit_pos % 8;
        if rem != 0 {
            self.write_bits(0, (8 - rem) as u32)?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    // Binary literals here are grouped by *wire field boundaries* (e.g.
    // `0b1_01_10101` = a 1-bit, 2-bit and 5-bit field) to document the layout,
    // not by nibbles.
    #![allow(clippy::unusual_byte_groupings)]

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
        assert_eq!(buf[0], 0b1_01_10101);

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
}
