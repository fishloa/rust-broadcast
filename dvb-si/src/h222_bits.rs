//! Bit-level reader/writer shared by the Rec. ITU-T H.222.0 (08/2023)
//! | ISO/IEC 13818-1:2023 §2.6 descriptors whose syntax tables are written in
//! bits (not bytes). Internal helper, not part of the public API.
//!
//! The writer checks every value against its field width with
//! [`broadcast_common::len::fit_bits`] (an over-wide value is
//! `Error::FieldOverflow`, never masked), so the descriptors built on it
//! cannot silently truncate a field.

use crate::error::{Error, Result};
use alloc::vec::Vec;
use broadcast_common::len::fit_bits;

const BITS_PER_BYTE: usize = 8;
const MISALIGNED_BYTES: &str = "whole-byte access at a non-byte-aligned bit position";

/// MSB-first bit reader over a descriptor body.
pub(crate) struct BitReader<'a> {
    data: &'a [u8],
    /// Position in bits.
    pos: usize,
    what: &'static str,
}

impl<'a> BitReader<'a> {
    pub(crate) fn new(data: &'a [u8], what: &'static str) -> Self {
        Self { data, pos: 0, what }
    }

    fn short(&self, need_bits: usize) -> Error {
        Error::BufferTooShort {
            need: (self.pos + need_bits).div_ceil(BITS_PER_BYTE),
            have: self.data.len(),
            what: self.what,
        }
    }

    /// Bits not yet consumed.
    pub(crate) fn remaining_bits(&self) -> usize {
        self.data.len() * BITS_PER_BYTE - self.pos
    }

    /// Whole bytes not yet consumed (the reader must be byte aligned).
    pub(crate) fn remaining_bytes(&self) -> usize {
        self.remaining_bits() / BITS_PER_BYTE
    }

    /// Read `n` (1..=32) bits.
    pub(crate) fn bits(&mut self, n: usize) -> Result<u32> {
        if self.remaining_bits() < n {
            return Err(self.short(n));
        }
        let mut v: u32 = 0;
        for _ in 0..n {
            let byte = self.data[self.pos / BITS_PER_BYTE];
            let bit = (byte >> (BITS_PER_BYTE - 1 - self.pos % BITS_PER_BYTE)) & 1;
            v = (v << 1) | u32::from(bit);
            self.pos += 1;
        }
        Ok(v)
    }

    /// Read `n` (1..=64) bits.
    pub(crate) fn bits64(&mut self, n: usize) -> Result<u64> {
        if self.remaining_bits() < n {
            return Err(self.short(n));
        }
        let mut v: u64 = 0;
        for _ in 0..n {
            v = (v << 1) | u64::from(self.bits(1)?);
        }
        Ok(v)
    }

    /// Read `n` (1..=8) bits as a `u8`.
    pub(crate) fn u8(&mut self, n: usize) -> Result<u8> {
        Ok(u8::try_from(self.bits(n)?).unwrap_or(u8::MAX))
    }

    /// Read `n` (1..=16) bits as a `u16`.
    pub(crate) fn u16(&mut self, n: usize) -> Result<u16> {
        Ok(u16::try_from(self.bits(n)?).unwrap_or(u16::MAX))
    }

    pub(crate) fn flag(&mut self) -> Result<bool> {
        Ok(self.bits(1)? == 1)
    }

    /// Read `n` whole bytes. Errors (in release builds too) when the reader
    /// is not byte aligned, instead of silently reading shifted data.
    pub(crate) fn bytes(&mut self, n: usize) -> Result<&'a [u8]> {
        if !self.pos.is_multiple_of(BITS_PER_BYTE) {
            return Err(Error::ValueOutOfRange {
                field: self.what,
                reason: MISALIGNED_BYTES,
            });
        }
        if self.remaining_bytes() < n {
            return Err(self.short(n * BITS_PER_BYTE));
        }
        let start = self.pos / BITS_PER_BYTE;
        self.pos += n * BITS_PER_BYTE;
        Ok(&self.data[start..start + n])
    }

    /// Everything left (byte aligned).
    pub(crate) fn rest(&mut self) -> &'a [u8] {
        let n = self.remaining_bytes();
        self.bytes(n).unwrap_or(&[])
    }
}

/// MSB-first bit writer; every field is range-checked.
#[derive(Default)]
pub(crate) struct BitWriter {
    buf: Vec<u8>,
    /// Bits used in the last byte of `buf` (0 = byte aligned).
    used: usize,
}

impl BitWriter {
    pub(crate) fn new() -> Self {
        Self::default()
    }

    /// Append `value` as an `n`-bit field named `field`.
    pub(crate) fn put(&mut self, value: u64, n: usize, field: &'static str) -> Result<()> {
        let n32 = u32::try_from(n).unwrap_or(u32::MAX);
        let v = fit_bits(value, n32, field)?;
        for i in (0..n).rev() {
            let bit = (v >> i) & 1 == 1;
            if self.used == 0 {
                self.buf.push(0);
            }
            if bit && let Some(last) = self.buf.last_mut() {
                *last |= 1 << (BITS_PER_BYTE - 1 - self.used);
            }
            self.used = (self.used + 1) % BITS_PER_BYTE;
        }
        Ok(())
    }

    pub(crate) fn flag(&mut self, v: bool, field: &'static str) -> Result<()> {
        self.put(u64::from(v), 1, field)
    }

    /// Append whole bytes. Errors (in release builds too) when the writer is
    /// not byte aligned, instead of emitting shifted data.
    pub(crate) fn bytes(&mut self, b: &[u8]) -> Result<()> {
        if self.used != 0 {
            return Err(Error::ValueOutOfRange {
                field: "byte run",
                reason: MISALIGNED_BYTES,
            });
        }
        self.buf.extend_from_slice(b);
        Ok(())
    }

    pub(crate) fn into_vec(self) -> Vec<u8> {
        self.buf
    }
}

/// Implement `Parse` / `Serialize` / `DescriptorDef` for an owned descriptor
/// type that provides `parse_body(&[u8]) -> Result<Self>` and
/// `write_body(&self) -> Result<Vec<u8>>`.
macro_rules! impl_h222_descriptor {
    ($ty:ty, $tag:expr, $what:literal, $name:literal) => {
        impl<'a> broadcast_common::Parse<'a> for $ty {
            type Error = crate::error::Error;

            fn parse(bytes: &'a [u8]) -> crate::error::Result<Self> {
                let body = $crate::descriptors::descriptor_body(
                    bytes,
                    $tag,
                    $what,
                    concat!("unexpected tag for ", $what),
                )?;
                Self::parse_body(body)
            }
        }

        impl broadcast_common::Serialize for $ty {
            type Error = crate::error::Error;

            fn serialized_len(&self) -> usize {
                // An unencodable value is reported by `serialize_into`.
                self.write_body().map_or(0, |b| 2 + b.len())
            }

            fn serialize_into(&self, buf: &mut [u8]) -> crate::error::Result<usize> {
                let body = self.write_body()?;
                let len = 2 + body.len();
                if buf.len() < len {
                    return Err(crate::error::Error::OutputBufferTooSmall {
                        need: len,
                        have: buf.len(),
                    });
                }
                $crate::descriptors::write_descriptor_header(buf, $tag, body.len())?;
                buf[2..len].copy_from_slice(&body);
                Ok(len)
            }
        }

        impl crate::traits::DescriptorDef<'_> for $ty {
            const TAG: u8 = $tag;
            const NAME: &'static str = $name;
        }
    };
}
pub(crate) use impl_h222_descriptor;

/// Error for a value the syntax cannot express.
pub(crate) fn invalid(tag: u8, reason: &'static str) -> Error {
    Error::InvalidDescriptor { tag, reason }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reader_bytes_rejects_misaligned_access() {
        let mut r = BitReader::new(&[0xFF, 0xAA, 0xBB], "test");
        r.bits(3).unwrap();
        assert!(matches!(
            r.bytes(1).unwrap_err(),
            Error::ValueOutOfRange { field: "test", .. }
        ));
        let mut r = BitReader::new(&[0xFF, 0xAA, 0xBB], "test");
        r.bits(8).unwrap();
        assert_eq!(r.bytes(2).unwrap(), &[0xAA, 0xBB]);
    }

    #[test]
    fn writer_bytes_rejects_misaligned_access() {
        let mut w = BitWriter::new();
        w.put(1, 3, "x").unwrap();
        assert!(matches!(
            w.bytes(&[1]).unwrap_err(),
            Error::ValueOutOfRange { .. }
        ));
        let mut w = BitWriter::new();
        w.put(0xAB, 8, "x").unwrap();
        w.bytes(&[1]).unwrap();
        assert_eq!(w.into_vec(), [0xAB, 1]);
    }
}
