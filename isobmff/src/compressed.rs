//! Compressed top-level boxes — ISO/IEC 14496-12:2026 §8.16.
//!
//! | Box | Type | Replacement |
//! |-----|------|-------------|
//! | [`CompressedBox`] | `!mov` | `moov` |
//! | [`CompressedBox`] | `!mof` | `moof` |
//! | [`CompressedBox`] | `!six` | `sidx` |
//! | [`CompressedBox`] | `!ssx` | `ssix` |
//!
//! `CompressedBox(to, replaced)` carries the compressed payload of the box it
//! replaces; the payload is opaque here (the compression algorithm is outside
//! this clause) and kept verbatim, with the header form (`size == 0` or
//! `largesize`) the file used.
//!
//! Transcription: `private/specs-verified/mpeg/isobmff/iso-iec-14496-12-2026.md`
//! §8.16.3–§8.16.9.

use alloc::vec::Vec;
use broadcast_core::len::{need_in, need_out};
use broadcast_core::{Parse, Serialize};

use crate::box_types::{BOX_HEADER_MIN_SIZE, BOX_TYPE_SIZE};
use crate::error::{Error, Result};

/// Compressed movie box (`!mov`) replaces `moov` (§8.16.6).
pub const COMPRESSED_MOVIE: [u8; BOX_TYPE_SIZE] = *b"!mov";
/// Compressed movie fragment box (`!mof`) replaces `moof` (§8.16.7).
pub const COMPRESSED_MOVIE_FRAGMENT: [u8; BOX_TYPE_SIZE] = *b"!mof";
/// Compressed segment index box (`!six`) replaces `sidx` (§8.16.8).
pub const COMPRESSED_SEGMENT_INDEX: [u8; BOX_TYPE_SIZE] = *b"!six";
/// Compressed subsegment index box (`!ssx`) replaces `ssix` (§8.16.9).
pub const COMPRESSED_SUBSEGMENT_INDEX: [u8; BOX_TYPE_SIZE] = *b"!ssx";

/// The four-CCs this crate parses as a [`CompressedBox`].
pub const COMPRESSED_TYPES: [[u8; BOX_TYPE_SIZE]; 4] = [
    COMPRESSED_MOVIE,
    COMPRESSED_MOVIE_FRAGMENT,
    COMPRESSED_SEGMENT_INDEX,
    COMPRESSED_SUBSEGMENT_INDEX,
];

/// `replacement_type` a compressed box's four-CC implies (§8.16.6–9).
#[must_use]
pub fn replacement_type(box_type: [u8; BOX_TYPE_SIZE]) -> Option<[u8; BOX_TYPE_SIZE]> {
    match &box_type {
        b"!mov" => Some(*b"moov"),
        b"!mof" => Some(*b"moof"),
        b"!six" => Some(*b"sidx"),
        b"!ssx" => Some(*b"ssix"),
        _ => None,
    }
}

/// A compressed top-level box (`!mov`, `!mof`, `!six`, `!ssx`) — §8.16.
///
/// One definition for the four codes: the syntax is the same `CompressedBox`
/// over a different replacement type.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub struct CompressedBox {
    box_type: [u8; BOX_TYPE_SIZE],
    compressed_data: Vec<u8>,
    to_end: bool,
    largesize: bool,
}

impl CompressedBox {
    /// Build one from a compressed-type four-CC and its payload.
    ///
    /// # Errors
    /// The four-CC is not one of `!mov`, `!mof`, `!six`, `!ssx`.
    pub fn new(box_type: [u8; BOX_TYPE_SIZE], compressed_data: Vec<u8>) -> Result<Self> {
        if replacement_type(box_type).is_none() {
            return Err(Error::UnexpectedBox {
                expected: "a compressed top-level box",
            });
        }
        Ok(Self {
            box_type,
            compressed_data,
            to_end: false,
            largesize: false,
        })
    }

    /// The box's four-CC.
    #[must_use]
    pub const fn box_type(&self) -> [u8; BOX_TYPE_SIZE] {
        self.box_type
    }

    /// The four-CC of the box this one replaces (`moov`, `moof`, `sidx`,
    /// `ssix`).
    #[must_use]
    pub fn replaced_type(&self) -> [u8; BOX_TYPE_SIZE] {
        replacement_type(self.box_type).unwrap_or(self.box_type)
    }

    /// The compressed payload, verbatim.
    #[must_use]
    pub fn compressed_data(&self) -> &[u8] {
        &self.compressed_data
    }

    /// Whether the wire used the `size == 0` form (§4.2).
    #[must_use]
    pub const fn extends_to_end(&self) -> bool {
        self.to_end
    }

    /// Whether the wire used the `largesize` form (§4.2).
    #[must_use]
    pub const fn has_largesize(&self) -> bool {
        self.largesize
    }

    /// Parse a whole compressed box, box header included.
    pub fn parse_box(bytes: &[u8]) -> Result<Self> {
        let item = crate::box_types::parse_box(bytes)?.0;
        let box_type = item.header.box_type.0;
        if replacement_type(box_type).is_none() {
            return Err(Error::UnexpectedBox {
                expected: "a compressed top-level box",
            });
        }
        need_in(item.body, 0, "compressed box")?;
        Ok(Self {
            box_type,
            compressed_data: item.body.to_vec(),
            to_end: item.to_end(),
            largesize: item.has_largesize(),
        })
    }
}

impl<'a> Parse<'a> for CompressedBox {
    type Error = Error;

    fn parse(bytes: &'a [u8]) -> Result<Self> {
        Self::parse_box(bytes)
    }
}

impl Serialize for CompressedBox {
    type Error = Error;

    fn serialized_len(&self) -> usize {
        let header = BOX_HEADER_MIN_SIZE
            + if self.largesize {
                core::mem::size_of::<u64>()
            } else {
                0
            };
        header + self.compressed_data.len()
    }

    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        let need = self.serialized_len();
        need_out(buf, need)?;
        let wide = self.largesize;
        let size = if self.to_end {
            0u32
        } else if wide {
            crate::box_types::SIZE_INDICATES_LARGESIZE
        } else {
            broadcast_core::len::fit_u32(need, "compressed box size")?
        };
        buf[..core::mem::size_of::<u32>()].copy_from_slice(&size.to_be_bytes());
        buf[core::mem::size_of::<u32>()..BOX_HEADER_MIN_SIZE].copy_from_slice(&self.box_type);
        let mut c = BOX_HEADER_MIN_SIZE;
        if wide {
            let large = u64::try_from(need).map_err(|_| {
                Error::InvalidInput("compressed box larger than 64 bits can address")
            })?;
            buf[c..c + core::mem::size_of::<u64>()].copy_from_slice(&large.to_be_bytes());
            c += core::mem::size_of::<u64>();
        }
        buf[c..c + self.compressed_data.len()].copy_from_slice(&self.compressed_data);
        Ok(need)
    }
}

#[cfg(test)]
mod tests {
    use super::{COMPRESSED_MOVIE, CompressedBox};
    use broadcast_core::Serialize;

    #[test]
    fn round_trips_with_the_replacement_type() {
        let value = CompressedBox::new(COMPRESSED_MOVIE, vec![1, 2, 3]).expect("builds");
        assert_eq!(value.replaced_type(), *b"moov");
        let mut out = alloc::vec![0u8; value.serialized_len()];
        value.serialize_into(&mut out).expect("serializes");
        let back = CompressedBox::parse_box(&out).expect("parses");
        assert_eq!(back, value);
    }

    #[test]
    fn a_non_compressed_type_is_refused() {
        assert!(CompressedBox::new(*b"moov", vec![]).is_err());
    }

    #[test]
    fn a_size_zero_tail_round_trips() {
        let mut bytes = alloc::vec![0u8, 0, 0, 0];
        bytes.extend_from_slice(b"!ssx");
        bytes.extend_from_slice(&[9, 8, 7]);
        let value = CompressedBox::parse_box(&bytes).expect("parses");
        assert!(value.extends_to_end());
        let mut out = alloc::vec![0u8; value.serialized_len()];
        value.serialize_into(&mut out).expect("serializes");
        assert_eq!(out, bytes);
    }
}
