//! CLUT Definition Segment — ETSI EN 300 743 §7.2.4, Table 16 (segment_type 0x12).

use crate::error::{Error, Result};
use dvb_common::{Parse, Serialize};

/// The CLUT_definition_segment segment_type.
pub const SEGMENT_TYPE: u8 = 0x12;
/// Header: 6 bytes.
pub const HEADER_LEN: usize = 6;
/// Fixed body: CLUT_id(1) + CLUT_version_number(4b)+reserved(4b) = 2 bytes.
pub const FIXED_LEN: usize = 2;
/// Entry header: CLUT_entry_id(1) + flags(1) = 2 bytes.
pub const ENTRY_HEADER_LEN: usize = 2;
/// Full-range colour: 4 bytes.
pub const ENTRY_FULL_LEN: usize = 4;
/// Reduced-range colour: 2 bytes.
pub const ENTRY_REDUCED_LEN: usize = 2;

/// A single CLUT entry.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct ClutEntry {
    /// CLUT entry number.
    pub clut_entry_id: u8,
    /// Whether this loads into the 2-bit/entry CLUT.
    pub flag_2bit: bool,
    /// Whether this loads into the 4-bit/entry CLUT.
    pub flag_4bit: bool,
    /// Whether this loads into the 8-bit/entry CLUT.
    pub flag_8bit: bool,
    /// Reserved bits in the flags byte (bits `[4:1]`, must be preserved for round-trip).
    pub reserved_flags: u8,
    /// Whether full 8-bit resolution colour values follow.
    pub full_range_flag: bool,
    /// Y output value.
    pub y_value: u8,
    /// Cr output value.
    pub cr_value: u8,
    /// Cb output value.
    pub cb_value: u8,
    /// T (transparency) output value.
    pub t_value: u8,
}

impl ClutEntry {
    fn serialized_len(&self) -> usize {
        ENTRY_HEADER_LEN
            + if self.full_range_flag {
                ENTRY_FULL_LEN
            } else {
                ENTRY_REDUCED_LEN
            }
    }

    fn serialize_into(&self, buf: &mut [u8]) {
        buf[0] = self.clut_entry_id;
        let mut flags: u8 = self.reserved_flags;
        if self.flag_2bit {
            flags |= 0x80;
        }
        if self.flag_4bit {
            flags |= 0x40;
        }
        if self.flag_8bit {
            flags |= 0x20;
        }
        if self.full_range_flag {
            flags |= 0x01;
        }
        buf[1] = flags;
        if self.full_range_flag {
            buf[2] = self.y_value;
            buf[3] = self.cr_value;
            buf[4] = self.cb_value;
            buf[5] = self.t_value;
        } else {
            buf[2] = (self.y_value << 2) | (self.cr_value >> 2);
            buf[3] = (self.cr_value << 6) | (self.cb_value << 2) | self.t_value;
        }
    }

    fn parse(bytes: &[u8]) -> Result<(Self, usize)> {
        if bytes.len() < ENTRY_HEADER_LEN {
            return Err(Error::BufferTooShort {
                need: ENTRY_HEADER_LEN,
                have: bytes.len(),
                what: "CLUT_entry header",
            });
        }
        let clut_entry_id = bytes[0];
        let flag_2bit = (bytes[1] & 0x80) != 0;
        let flag_4bit = (bytes[1] & 0x40) != 0;
        let flag_8bit = (bytes[1] & 0x20) != 0;
        let reserved_flags = bytes[1] & 0x1E;
        let full_range_flag = (bytes[1] & 0x01) != 0;

        let total = ENTRY_HEADER_LEN
            + if full_range_flag {
                ENTRY_FULL_LEN
            } else {
                ENTRY_REDUCED_LEN
            };
        if bytes.len() < total {
            return Err(Error::BufferTooShort {
                need: total,
                have: bytes.len(),
                what: "CLUT_entry data",
            });
        }
        let (y, cr, cb, t) = if full_range_flag {
            (bytes[2], bytes[3], bytes[4], bytes[5])
        } else {
            let y = bytes[2] >> 2;
            let cr = ((bytes[2] & 0x03) << 2) | (bytes[3] >> 6);
            let cb = (bytes[3] >> 2) & 0x0F;
            let t = bytes[3] & 0x03;
            (y, cr, cb, t)
        };
        Ok((
            ClutEntry {
                clut_entry_id,
                flag_2bit,
                flag_4bit,
                flag_8bit,
                reserved_flags,
                full_range_flag,
                y_value: y,
                cr_value: cr,
                cb_value: cb,
                t_value: t,
            },
            total,
        ))
    }
}

/// CLUT Definition Segment.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct ClutDefinitionSegment {
    /// The page_id from the segment header.
    pub page_id: u16,
    /// CLUT family identifier.
    pub clut_id: u8,
    /// CLUT version number (modulo 16).
    pub clut_version_number: u8,
    /// Reserved bits in the body byte 1 (bits `[3:0]`).
    pub reserved: u8,
    /// CLUT entries.
    pub entries: alloc::vec::Vec<ClutEntry>,
    /// Trailing bytes after the last successfully-parsed entry (preserved for round-trip).
    #[cfg_attr(feature = "serde", serde(skip))]
    pub(crate) suffix: alloc::vec::Vec<u8>,
}

impl<'a> Parse<'a> for ClutDefinitionSegment {
    type Error = Error;

    fn parse(bytes: &'a [u8]) -> Result<Self> {
        if bytes.len() < HEADER_LEN + FIXED_LEN {
            return Err(Error::BufferTooShort {
                need: HEADER_LEN + FIXED_LEN,
                have: bytes.len(),
                what: "CLUT_definition_segment",
            });
        }
        if bytes[1] != SEGMENT_TYPE {
            return Err(Error::UnknownSegmentType(bytes[1]));
        }
        let page_id = u16::from_be_bytes([bytes[2], bytes[3]]);
        let segment_length = u16::from_be_bytes([bytes[4], bytes[5]]) as usize;
        let total = HEADER_LEN + segment_length;
        if bytes.len() < total {
            return Err(Error::BufferTooShort {
                need: total,
                have: bytes.len(),
                what: "CLUT_definition_segment data",
            });
        }
        let body = &bytes[HEADER_LEN..HEADER_LEN + segment_length];
        if body.len() < FIXED_LEN {
            return Err(Error::BufferTooShort {
                need: FIXED_LEN,
                have: body.len(),
                what: "CLUT_definition_segment body",
            });
        }
        let clut_id = body[0];
        let clut_version_number = body[1] >> 4;
        let reserved = body[1] & 0x0F;

        let rest = &body[FIXED_LEN..];
        let mut entries = alloc::vec::Vec::new();
        let mut pos: usize = 0;
        let mut suffix = alloc::vec::Vec::new();
        while pos < rest.len() {
            match ClutEntry::parse(&rest[pos..]) {
                Ok((entry, entry_len)) => {
                    entries.push(entry);
                    pos += entry_len;
                }
                Err(_) => {
                    // Truncated or malformed entry — preserve remainder as raw
                    suffix.extend_from_slice(&rest[pos..]);
                    break;
                }
            }
        }

        Ok(ClutDefinitionSegment {
            page_id,
            clut_id,
            clut_version_number,
            reserved,
            entries,
            suffix,
        })
    }
}

impl Serialize for ClutDefinitionSegment {
    type Error = Error;

    fn serialized_len(&self) -> usize {
        HEADER_LEN
            + FIXED_LEN
            + self
                .entries
                .iter()
                .map(|e| e.serialized_len())
                .sum::<usize>()
            + self.suffix.len()
    }

    fn serialize_into(&self, buf: &mut [u8]) -> core::result::Result<usize, Self::Error> {
        let len = self.serialized_len();
        if buf.len() < len {
            return Err(Error::BufferTooShort {
                need: len,
                have: buf.len(),
                what: "CLUT_definition_segment serialize",
            });
        }
        buf[0] = 0x0F;
        buf[1] = SEGMENT_TYPE;
        buf[2..4].copy_from_slice(&self.page_id.to_be_bytes());
        let seg_len = (len - HEADER_LEN) as u16;
        buf[4..6].copy_from_slice(&seg_len.to_be_bytes());

        buf[6] = self.clut_id;
        buf[7] = (self.clut_version_number << 4) | (self.reserved & 0x0F);

        let mut off = HEADER_LEN + FIXED_LEN;
        for entry in &self.entries {
            entry.serialize_into(&mut buf[off..]);
            off += entry.serialized_len();
        }
        buf[off..off + self.suffix.len()].copy_from_slice(&self.suffix);
        Ok(len)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use dvb_common::{Parse, Serialize};

    #[test]
    fn round_trip_full_range() {
        let bytes = [
            0x0F, 0x12, 0x00, 0x01, 0x00, 0x0E, 0x03, 0x10, 0x00, 0xA1, 0x80, 0x80, 0x80, 0x80,
            0x01, 0x61, 0xFF, 0xFF, 0xFF, 0x00,
        ];
        let seg = ClutDefinitionSegment::parse(&bytes).unwrap();
        assert_eq!(seg.clut_id, 3);
        assert_eq!(seg.entries.len(), 2);
        assert_eq!(seg.entries[0].clut_entry_id, 0);
        assert!(seg.entries[0].flag_8bit);
        assert!(seg.entries[0].full_range_flag);
        assert_eq!(seg.entries[0].y_value, 128);
        assert_eq!(seg.entries[1].clut_entry_id, 1);
        assert!(seg.entries[1].flag_4bit);
        let out = seg.to_bytes();
        assert_eq!(out, bytes);

        // Biting test
        let mut seg2 = seg.clone();
        seg2.clut_id = 5;
        let out2 = seg2.to_bytes();
        assert_ne!(out2, bytes);
        let reparse = ClutDefinitionSegment::parse(&out2).unwrap();
        assert_eq!(reparse.clut_id, 5);
    }

    #[test]
    fn round_trip_reduced_range() {
        let bytes = [
            0x0F, 0x12, 0x00, 0x01, 0x00, 0x0A, 0x03, 0x10, 0x00, 0x80, 0x00, 0x00, 0x01, 0x40,
            0xFD, 0xFC,
        ];
        let seg = ClutDefinitionSegment::parse(&bytes).unwrap();
        assert_eq!(seg.entries.len(), 2);
        assert!(seg.entries[0].flag_2bit);
        assert!(!seg.entries[0].full_range_flag);
        assert_eq!(seg.entries[0].y_value, 0);
        let out = seg.to_bytes();
        assert_eq!(out, bytes);

        // Biting test
        let mut seg2 = seg.clone();
        seg2.entries[0].flag_2bit = false;
        seg2.entries[0].flag_8bit = true;
        let out2 = seg2.to_bytes();
        assert_ne!(out2, bytes);
        let reparse = ClutDefinitionSegment::parse(&out2).unwrap();
        assert!(reparse.entries[0].flag_8bit);
    }

    #[test]
    fn tolerates_nonzero_reserved_bits() {
        let bytes = [
            0x0F, 0x12, 0x00, 0x01, 0x00,
            0x06, // seg_len=6 = 2 fixed + 4 entry (2 header + 2 reduced)
            0x03, 0x1A, // CLUT_id=3, version=1, reserved=0xA
            0x00, 0x8A, 0x00,
            0x00, // entry: flag_2bit=1, reserved_flags=0x0A, full=0, Y=Cr=Cb=T=0
        ];
        let seg = ClutDefinitionSegment::parse(&bytes).unwrap();
        assert_eq!(seg.reserved, 0x0A);
        assert_eq!(seg.entries.len(), 1);
        assert_eq!(seg.entries[0].reserved_flags, 0x0A);
        assert!(seg.entries[0].flag_2bit);
        let out = seg.to_bytes();
        assert_eq!(out, bytes);
        let seg2 = ClutDefinitionSegment::parse(&out).unwrap();
        assert_eq!(seg2.reserved, 0x0A);
        assert_eq!(seg2.entries[0].reserved_flags, 0x0A);
    }
}
