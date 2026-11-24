//! Checked big-endian field readers.
//!
//! Replaces the `*bytes[off..].first_chunk::<N>().unwrap()` idiom: a short
//! buffer is reported as [`Error::BufferTooShort`] instead of panicking.

use crate::error::{Error, Result};

/// Size in bytes of a big-endian `u16` field.
pub(crate) const U16_LEN: usize = core::mem::size_of::<u16>();
/// Size in bytes of a big-endian `u32` field.
pub(crate) const U32_LEN: usize = core::mem::size_of::<u32>();

/// Read a big-endian `u16` at `off`, or `BufferTooShort` naming `what`.
pub(crate) fn be_u16(bytes: &[u8], off: usize, what: &'static str) -> Result<u16> {
    match bytes.get(off..).and_then(|s| s.first_chunk::<U16_LEN>()) {
        Some(b) => Ok(u16::from_be_bytes(*b)),
        None => Err(Error::BufferTooShort(
            broadcast_common::error::BufferTooShort {
                need: off.saturating_add(U16_LEN),
                have: bytes.len(),
                what,
            },
        )),
    }
}

/// Read a big-endian `u32` at `off`, or `BufferTooShort` naming `what`.
pub(crate) fn be_u32(bytes: &[u8], off: usize, what: &'static str) -> Result<u32> {
    match bytes.get(off..).and_then(|s| s.first_chunk::<U32_LEN>()) {
        Some(b) => Ok(u32::from_be_bytes(*b)),
        None => Err(Error::BufferTooShort(
            broadcast_common::error::BufferTooShort {
                need: off.saturating_add(U32_LEN),
                have: bytes.len(),
                what,
            },
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exact_ok() {
        assert_eq!(be_u16(&[1, 2], 0, "t").unwrap(), 0x0102);
        assert_eq!(be_u16(&[9, 1, 2], 1, "t").unwrap(), 0x0102);
        assert_eq!(be_u32(&[1, 2, 3, 4], 0, "t").unwrap(), 0x0102_0304);
    }

    #[test]
    fn short_is_err_not_panic() {
        assert!(matches!(
            be_u16(&[1], 0, "t"),
            Err(Error::BufferTooShort(
                broadcast_common::error::BufferTooShort {
                    need: 2,
                    have: 1,
                    what: "t"
                }
            ))
        ));
        assert!(matches!(
            &(be_u16(&[1, 2], 1, "t")),
            Err(crate::error::Error::BufferTooShort(..))
        ));
        assert!(matches!(
            &(be_u16(&[1, 2], 9, "t")),
            Err(crate::error::Error::BufferTooShort(..))
        ));
        assert!(matches!(
            &(be_u16(&[], usize::MAX, "t")),
            Err(crate::error::Error::BufferTooShort(..))
        ));
        assert!(matches!(
            &(be_u32(&[1, 2, 3], 0, "t")),
            Err(crate::error::Error::BufferTooShort(..))
        ));
    }
}
