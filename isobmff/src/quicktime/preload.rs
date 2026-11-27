//! QuickTime track loading settings (`load`) — read and written.
//!
//! Source: `private/specs-verified/apple/isobmff/quicktime-file-format-2001.md`,
//! "Track loading settings atom 'load'".

use alloc::vec::Vec;
use broadcast_core::len::{need_in, need_out};
use broadcast_core::wire::{be_u32, u32_at};
use broadcast_core::{Parse, Serialize};

use crate::error::{Error, Result};

/// Four-CC of the track loading settings atom (`load`).
pub const LOAD: [u8; 4] = *b"load";
/// Byte length of the fixed body: start time, duration and flags.
const BODY_LEN: usize = core::mem::size_of::<u32>() * 3;
const START_OFFSET: usize = 0;
const DURATION_OFFSET: usize = core::mem::size_of::<u32>();
const FLAGS_OFFSET: usize = core::mem::size_of::<u32>() * 2;

/// Track loading settings atom (`load`) — QuickTime File Format.
///
/// `preload_duration` of `0xFFFF_FFFF` means the segment runs to the end of
/// the track.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub struct PreloadBox {
    preload_start_time: u32,
    preload_duration: u32,
    preload_flags: u32,
}

impl PreloadBox {
    /// Build one from its three fields.
    #[must_use]
    pub const fn new(preload_start_time: u32, preload_duration: u32, preload_flags: u32) -> Self {
        Self {
            preload_start_time,
            preload_duration,
            preload_flags,
        }
    }

    /// `preload_start_time`, in the movie's time coordinate system.
    #[must_use]
    pub const fn preload_start_time(self) -> u32 {
        self.preload_start_time
    }

    /// `preload_duration`; `u32::MAX` extends to the end of the track.
    #[must_use]
    pub const fn preload_duration(self) -> u32 {
        self.preload_duration
    }

    /// `preload_flags` (1: preload regardless of enabled state, 2: only when
    /// enabled).
    #[must_use]
    pub const fn preload_flags(self) -> u32 {
        self.preload_flags
    }

    /// Parse the whole atom, box header included.
    pub fn parse_box(bytes: &[u8]) -> Result<Self> {
        let item = crate::box_types::parse_box(bytes)?.0;
        if !item.header.box_type.is(&LOAD) {
            return Err(Error::UnexpectedBox { expected: "load" });
        }
        Self::parse_body(item.body)
    }

    /// Parse the payload after the 8-byte box header.
    pub fn parse_body(body: &[u8]) -> Result<Self> {
        need_in(body, BODY_LEN, "load")?;
        Ok(Self {
            preload_start_time: u32_at(body, START_OFFSET),
            preload_duration: be_u32(body, DURATION_OFFSET, "load preload_duration")?,
            preload_flags: be_u32(body, FLAGS_OFFSET, "load preload_flags")?,
        })
    }
}

impl<'a> Parse<'a> for PreloadBox {
    type Error = Error;

    fn parse(bytes: &'a [u8]) -> Result<Self> {
        Self::parse_box(bytes)
    }
}

impl Serialize for PreloadBox {
    type Error = Error;

    fn serialized_len(&self) -> usize {
        crate::box_types::box_len(crate::box_types::BOX_HEADER_MIN_SIZE + BODY_LEN)
    }

    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        let need = self.serialized_len();
        need_out(buf, need)?;
        let mut c = crate::box_types::write_box_header(buf, &LOAD, need)?;
        buf[c..c + core::mem::size_of::<u32>()]
            .copy_from_slice(&self.preload_start_time.to_be_bytes());
        c += core::mem::size_of::<u32>();
        buf[c..c + core::mem::size_of::<u32>()]
            .copy_from_slice(&self.preload_duration.to_be_bytes());
        c += core::mem::size_of::<u32>();
        buf[c..c + core::mem::size_of::<u32>()].copy_from_slice(&self.preload_flags.to_be_bytes());
        c += core::mem::size_of::<u32>();
        Ok(c)
    }
}

impl PreloadBox {
    /// The atom's bytes, for a writer.
    pub fn to_bytes(self) -> Result<Vec<u8>> {
        let mut out = alloc::vec![0u8; self.serialized_len()];
        self.serialize_into(&mut out)?;
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use super::PreloadBox;

    #[test]
    fn round_trips() {
        let value = PreloadBox::new(100, u32::MAX, 1);
        let bytes = value.to_bytes().expect("serializes");
        assert_eq!(PreloadBox::parse_box(&bytes).expect("parses"), value);
    }

    #[test]
    fn wrong_type_is_rejected() {
        let mut bytes = PreloadBox::new(0, 0, 0).to_bytes().expect("serializes");
        bytes[4..8].copy_from_slice(b"free");
        assert!(PreloadBox::parse_box(&bytes).is_err());
    }
}
