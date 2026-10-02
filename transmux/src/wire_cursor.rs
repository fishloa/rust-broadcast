//! Big-endian byte cursor shared by the `avcC`/`hvcC` decoder-configuration
//! parsers (audit r04-O5): one bounds-checked `take`, so the per-width readers
//! cannot drift apart. Every addition is checked (no wrap on a hostile cursor).

use crate::error::{Error, Result};
use alloc::vec::Vec;

/// Take the next `n` bytes at `*cursor`, advancing it. `BufferTooShort` (with the
/// byte count needed) if fewer remain.
pub(crate) fn take<'a>(
    bytes: &'a [u8],
    cursor: &mut usize,
    n: usize,
    what: &'static str,
) -> Result<&'a [u8]> {
    let end = cursor.saturating_add(n);
    let slice = bytes.get(*cursor..end).ok_or(Error::BufferTooShort {
        need: end,
        have: bytes.len(),
        what,
    })?;
    *cursor = end;
    Ok(slice)
}

pub(crate) fn read_u8(bytes: &[u8], cursor: &mut usize, what: &'static str) -> Result<u8> {
    Ok(take(bytes, cursor, 1, what)?[0])
}

pub(crate) fn read_u16(bytes: &[u8], cursor: &mut usize, what: &'static str) -> Result<u16> {
    let b = take(bytes, cursor, 2, what)?;
    Ok(u16::from_be_bytes([b[0], b[1]]))
}

pub(crate) fn read_u32(bytes: &[u8], cursor: &mut usize, what: &'static str) -> Result<u32> {
    let b = take(bytes, cursor, 4, what)?;
    Ok(u32::from_be_bytes([b[0], b[1], b[2], b[3]]))
}

/// A 48-bit big-endian value (the `hvcC` `general_constraint_indicator_flags`).
pub(crate) fn read_u48(bytes: &[u8], cursor: &mut usize, what: &'static str) -> Result<u64> {
    let b = take(bytes, cursor, 6, what)?;
    Ok(b.iter().fold(0u64, |acc, &x| (acc << 8) | u64::from(x)))
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
