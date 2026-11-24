//! Big-endian field readers shared by every parser in the crate (audit r04-O5):
//! one bounds-checked `take`, so the per-width readers cannot drift apart. Every
//! addition is checked (no wrap on a hostile cursor).
//!
//! Two families, one home:
//!
//! - the checked [`take`]/[`read_u8`]/[`read_u16`]/[`read_u32`]/[`read_u48`]/
//!   [`be_u16`]/[`be_u32`]/[`be_u64`] readers, which reject an offset the
//!   caller has not proved in range — use these on untrusted offsets;
//! - the unchecked `*_at` readers ([`u16_at`] … [`fourcc_at`]), which decode
//!   one field at an offset the caller's own guard (a `need_in`, a `get`, or a
//!   box-body slice) has already covered.

use crate::error::{Error, Result};
use alloc::vec::Vec;
use broadcast_common::error::BufferTooShort;

/// Byte width of the 24-bit fields (ISO/IEC 14496-12:2026 `size` prefixes,
/// `esds` lengths).
pub(crate) const U24_LEN: usize = 3;
/// Byte width of the 48-bit `hvcC` `general_constraint_indicator_flags`.
const U48_LEN: usize = 6;

/// Take the next `n` bytes at `*cursor`, advancing it. `BufferTooShort` (with the
/// byte count needed) if fewer remain.
pub(crate) fn take<'a>(
    bytes: &'a [u8],
    cursor: &mut usize,
    n: usize,
    what: &'static str,
) -> Result<&'a [u8]> {
    let end = cursor.saturating_add(n);
    let slice = bytes
        .get(*cursor..end)
        .ok_or(Error::BufferTooShort(BufferTooShort {
            need: end,
            have: bytes.len(),
            what,
        }))?;
    *cursor = end;
    Ok(slice)
}

pub(crate) fn read_u8(bytes: &[u8], cursor: &mut usize, what: &'static str) -> Result<u8> {
    Ok(take(bytes, cursor, 1, what)?[0])
}

pub(crate) fn read_u16(bytes: &[u8], cursor: &mut usize, what: &'static str) -> Result<u16> {
    let b = take(bytes, cursor, core::mem::size_of::<u16>(), what)?;
    let mut arr = [0u8; core::mem::size_of::<u16>()];
    arr.copy_from_slice(b);
    Ok(u16::from_be_bytes(arr))
}

pub(crate) fn read_u32(bytes: &[u8], cursor: &mut usize, what: &'static str) -> Result<u32> {
    let b = take(bytes, cursor, core::mem::size_of::<u32>(), what)?;
    let mut arr = [0u8; core::mem::size_of::<u32>()];
    arr.copy_from_slice(b);
    Ok(u32::from_be_bytes(arr))
}

/// A 48-bit big-endian value (the `hvcC` `general_constraint_indicator_flags`).
pub(crate) fn read_u48(bytes: &[u8], cursor: &mut usize, what: &'static str) -> Result<u64> {
    let b = take(bytes, cursor, U48_LEN, what)?;
    Ok(b.iter()
        .fold(0u64, |acc, &x| (acc << u8::BITS) | u64::from(x)))
}

/// A big-endian `u16` at absolute offset `off`.
pub(crate) fn be_u16(bytes: &[u8], off: usize, what: &'static str) -> Result<u16> {
    let mut cursor = off;
    read_u16(bytes, &mut cursor, what)
}

/// A big-endian `u32` at absolute offset `off`.
pub(crate) fn be_u32(bytes: &[u8], off: usize, what: &'static str) -> Result<u32> {
    let mut cursor = off;
    read_u32(bytes, &mut cursor, what)
}

/// A 16-bit-length-prefixed byte string (an `avcC` SPS/PPS entry).
pub(crate) fn read_nalu_16(
    bytes: &[u8],
    cursor: &mut usize,
    what: &'static str,
) -> Result<Vec<u8>> {
    let len = usize::from(read_u16(bytes, cursor, what)?);
    Ok(take(bytes, cursor, len, what)?.to_vec())
}

/// A big-endian `u64` at absolute offset `off` (the ISOBMFF version-1 64-bit
/// `creation_time`/`modification_time`/`duration` fields). `BufferTooShort`
/// instead of a panic when fewer than 8 bytes remain at `off`.
pub(crate) fn be_u64(bytes: &[u8], off: usize, what: &'static str) -> Result<u64> {
    let mut cursor = off;
    let b = take(bytes, &mut cursor, core::mem::size_of::<u64>(), what)?;
    Ok(b.iter()
        .fold(0u64, |acc, &x| (acc << u8::BITS) | u64::from(x)))
}

// ---------------------------------------------------------------------------
// Unchecked field readers (the caller's guard covers the offset)
// ---------------------------------------------------------------------------

/// A big-endian `u16` at `off`, the caller having already proved 2 bytes are
/// there.
pub(crate) fn u16_at(bytes: &[u8], off: usize) -> u16 {
    let mut field = [0u8; core::mem::size_of::<u16>()];
    field.copy_from_slice(&bytes[off..off + core::mem::size_of::<u16>()]);
    u16::from_be_bytes(field)
}

/// A big-endian `i16` at `off`, the caller having already proved 2 bytes are
/// there.
pub(crate) fn i16_at(bytes: &[u8], off: usize) -> i16 {
    let mut field = [0u8; core::mem::size_of::<i16>()];
    field.copy_from_slice(&bytes[off..off + core::mem::size_of::<i16>()]);
    i16::from_be_bytes(field)
}

/// A big-endian 24-bit value at `off`, widened to `u32`, the caller having
/// already proved 3 bytes are there.
pub(crate) fn u24_at(bytes: &[u8], off: usize) -> u32 {
    let mut field = [0u8; core::mem::size_of::<u32>()];
    field[core::mem::size_of::<u32>() - U24_LEN..].copy_from_slice(&bytes[off..off + U24_LEN]);
    u32::from_be_bytes(field)
}

/// A big-endian `u32` at `off`, the caller having already proved 4 bytes are
/// there.
pub(crate) fn u32_at(bytes: &[u8], off: usize) -> u32 {
    let mut field = [0u8; core::mem::size_of::<u32>()];
    field.copy_from_slice(&bytes[off..off + core::mem::size_of::<u32>()]);
    u32::from_be_bytes(field)
}

/// A big-endian `i32` at `off` (a transformation-matrix coefficient), the
/// caller having already proved 4 bytes are there.
pub(crate) fn i32_at(bytes: &[u8], off: usize) -> i32 {
    let mut field = [0u8; core::mem::size_of::<i32>()];
    field.copy_from_slice(&bytes[off..off + core::mem::size_of::<i32>()]);
    i32::from_be_bytes(field)
}

/// A big-endian `u64` at `off`, the caller having already proved 8 bytes are
/// there.
pub(crate) fn u64_at(bytes: &[u8], off: usize) -> u64 {
    let mut field = [0u8; core::mem::size_of::<u64>()];
    field.copy_from_slice(&bytes[off..off + core::mem::size_of::<u64>()]);
    u64::from_be_bytes(field)
}

/// A four-character code at `off` (the raw 4 bytes, not a number), the caller
/// having already proved 4 bytes are there.
pub(crate) fn fourcc_at(bytes: &[u8], off: usize) -> [u8; core::mem::size_of::<u32>()] {
    let mut field = [0u8; core::mem::size_of::<u32>()];
    field.copy_from_slice(&bytes[off..off + core::mem::size_of::<u32>()]);
    field
}

#[cfg(test)]
mod be_u64_tests {
    use broadcast_common::error::BufferTooShort;

    use super::*;

    #[test]
    fn exact_ok_short_err() {
        let b = [0u8, 0, 1, 2, 3, 4, 5, 6, 7, 8];
        assert_eq!(be_u64(&b, 2, "t").unwrap(), 0x0102_0304_0506_0708);
        assert!(matches!(
            be_u64(&b, 3, "t"),
            Err(Error::BufferTooShort(BufferTooShort {
                need: 11,
                have: 10,
                what: "t"
            }))
        ));
        assert!(matches!(
            be_u64(&b, usize::MAX, "t"),
            Err(Error::BufferTooShort(BufferTooShort {
                need: usize::MAX,
                ..
            }))
        ));
    }
}
