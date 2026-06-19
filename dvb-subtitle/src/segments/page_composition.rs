//! Page Composition Segment — ETSI EN 300 743 §7.2.2, Table 9 (segment_type 0x10).

use crate::error::{Error, Result};
use dvb_common::{Parse, Serialize};

/// The page_composition_segment segment_type.
pub const SEGMENT_TYPE: u8 = 0x10;
/// Header: sync_byte(1) + segment_type(1) + page_id(2) + segment_length(2) = 6 bytes.
pub const HEADER_LEN: usize = 6;
/// Fixed after header: page_time_out(1) + page_version(4b)+page_state(2b)+reserved(2b) = 2 bytes.
pub const FIXED_LEN: usize = 2;
/// Each region entry: region_id(1) + reserved(1) + region_horizontal_address(2) + region_vertical_address(2) = 6 bytes.
pub const REGION_ENTRY_LEN: usize = 6;

/// Page state as defined in Table 10.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[repr(u8)]
#[non_exhaustive]
pub enum PageState {
    /// Normal case — page update.
    Normal = 0x00,
    /// Acquisition point — page refresh.
    Acquisition = 0x01,
    /// Mode change — new page.
    ModeChange = 0x02,
    /// Reserved.
    Reserved(u8),
}

impl PageState {
    /// Human-readable name.
    #[must_use]
    pub fn name(&self) -> &'static str {
        match self {
            Self::Normal => "normal",
            Self::Acquisition => "acquisition",
            Self::ModeChange => "mode_change",
            Self::Reserved(_) => "reserved",
        }
    }

    fn to_bits(self) -> u8 {
        match self {
            Self::Normal => 0x00,
            Self::Acquisition => 0x01,
            Self::ModeChange => 0x02,
            Self::Reserved(v) => v & 0x03,
        }
    }
}

dvb_common::impl_spec_display!(PageState, Reserved);

/// A single region entry within a page composition segment.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct PageRegionEntry {
    /// Region identifier.
    pub region_id: u8,
    /// Reserved byte in the region entry (must be preserved for round-trip).
    pub reserved: u8,
    /// Horizontal address of top-left pixel.
    pub region_horizontal_address: u16,
    /// Vertical address of top line.
    pub region_vertical_address: u16,
}

impl PageRegionEntry {
    #[allow(dead_code)]
    fn serialized_len() -> usize {
        REGION_ENTRY_LEN
    }

    fn serialize_into(&self, buf: &mut [u8]) {
        buf[0] = self.region_id;
        buf[1] = self.reserved;
        buf[2..4].copy_from_slice(&self.region_horizontal_address.to_be_bytes());
        buf[4..6].copy_from_slice(&self.region_vertical_address.to_be_bytes());
    }

    fn parse(bytes: &[u8]) -> Result<Self> {
        if bytes.len() < REGION_ENTRY_LEN {
            return Err(Error::BufferTooShort {
                need: REGION_ENTRY_LEN,
                have: bytes.len(),
                what: "page_region_entry",
            });
        }
        let reserved = bytes[1];
        Ok(PageRegionEntry {
            region_id: bytes[0],
            reserved,
            region_horizontal_address: u16::from_be_bytes([bytes[2], bytes[3]]),
            region_vertical_address: u16::from_be_bytes([bytes[4], bytes[5]]),
        })
    }
}

/// Page Composition Segment.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct PageCompositionSegment {
    /// The page_id (from generic segment header).
    pub page_id: u16,
    /// Time-out period in seconds.
    pub page_time_out: u8,
    /// Version number (modulo 16).
    pub page_version_number: u8,
    /// Page state.
    pub page_state: PageState,
    /// Reserved bits in body byte 1 (bits `[1:0]`).
    pub reserved: u8,
    /// Region entries.
    pub regions: alloc::vec::Vec<PageRegionEntry>,
    /// Trailing bytes after the region loop (preserved for round-trip).
    #[cfg_attr(feature = "serde", serde(skip))]
    pub(crate) suffix: alloc::vec::Vec<u8>,
}

impl<'a> Parse<'a> for PageCompositionSegment {
    type Error = Error;

    fn parse(bytes: &'a [u8]) -> Result<Self> {
        if bytes.len() < HEADER_LEN + FIXED_LEN {
            return Err(Error::BufferTooShort {
                need: HEADER_LEN + FIXED_LEN,
                have: bytes.len(),
                what: "page_composition_segment",
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
                what: "page_composition_segment data",
            });
        }
        let body = &bytes[HEADER_LEN..HEADER_LEN + segment_length];
        if body.len() < FIXED_LEN {
            return Err(Error::BufferTooShort {
                need: FIXED_LEN,
                have: body.len(),
                what: "page_composition_segment body",
            });
        }
        let page_time_out = body[0];
        let page_version_number = body[1] >> 4;
        let page_state_bits = (body[1] >> 2) & 0x03;
        let reserved = body[1] & 0x03;
        let page_state = match page_state_bits {
            0x00 => PageState::Normal,
            0x01 => PageState::Acquisition,
            0x02 => PageState::ModeChange,
            v => PageState::Reserved(v),
        };

        let region_data = &body[FIXED_LEN..];
        let region_count = region_data.len() / REGION_ENTRY_LEN;
        if region_data.len() % REGION_ENTRY_LEN != 0 {
            return Err(Error::BufferTooShort {
                need: (region_count + 1) * REGION_ENTRY_LEN,
                have: region_data.len(),
                what: "page_composition_segment regions",
            });
        }
        let mut regions = alloc::vec::Vec::with_capacity(region_count);
        for i in 0..region_count {
            let entry_bytes = &region_data[i * REGION_ENTRY_LEN..][..REGION_ENTRY_LEN];
            regions.push(PageRegionEntry::parse(entry_bytes)?);
        }
        let suffix =
            alloc::vec::Vec::from(&region_data[region_count * REGION_ENTRY_LEN..region_data.len()]);

        Ok(PageCompositionSegment {
            page_id,
            page_time_out,
            page_version_number,
            page_state,
            reserved,
            regions,
            suffix,
        })
    }
}

impl Serialize for PageCompositionSegment {
    type Error = Error;

    fn serialized_len(&self) -> usize {
        HEADER_LEN + FIXED_LEN + self.regions.len() * REGION_ENTRY_LEN + self.suffix.len()
    }

    fn serialize_into(&self, buf: &mut [u8]) -> core::result::Result<usize, Self::Error> {
        let len = self.serialized_len();
        if buf.len() < len {
            return Err(Error::BufferTooShort {
                need: len,
                have: buf.len(),
                what: "page_composition_segment serialize",
            });
        }
        buf[0] = 0x0F;
        buf[1] = SEGMENT_TYPE;
        buf[2..4].copy_from_slice(&self.page_id.to_be_bytes());
        let seg_len = (len - HEADER_LEN) as u16;
        buf[4..6].copy_from_slice(&seg_len.to_be_bytes());

        buf[6] = self.page_time_out;
        buf[7] = (self.page_version_number << 4)
            | (self.page_state.to_bits() << 2)
            | (self.reserved & 0x03);

        for (i, region) in self.regions.iter().enumerate() {
            let off = HEADER_LEN + FIXED_LEN + i * REGION_ENTRY_LEN;
            region.serialize_into(&mut buf[off..off + REGION_ENTRY_LEN]);
        }
        let suffix_off = HEADER_LEN + FIXED_LEN + self.regions.len() * REGION_ENTRY_LEN;
        buf[suffix_off..suffix_off + self.suffix.len()].copy_from_slice(&self.suffix);
        Ok(len)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use dvb_common::{Parse, Serialize};

    #[test]
    fn round_trip() {
        let bytes = [
            0x0F, 0x10, 0x00, 0x01, 0x00, 0x0E, 0x0A, 0x04, 0x01, 0x00, 0x00, 0x64, 0x00, 0x32,
            0x02, 0x00, 0x00, 0xC8, 0x00, 0x96,
        ];
        let seg = PageCompositionSegment::parse(&bytes).unwrap();
        assert_eq!(seg.page_time_out, 10);
        assert_eq!(seg.page_version_number, 0);
        assert_eq!(seg.page_state, PageState::Acquisition);
        assert_eq!(seg.regions.len(), 2);
        assert_eq!(seg.regions[0].region_id, 1);
        assert_eq!(seg.regions[0].region_horizontal_address, 100);
        assert_eq!(seg.regions[0].region_vertical_address, 50);
        let out = seg.to_bytes();
        assert_eq!(out, bytes);

        // Biting test: mutate page_time_out changes output
        let mut seg2 = seg.clone();
        seg2.page_time_out = 20;
        let out2 = seg2.to_bytes();
        assert_ne!(out2, bytes);
        let reparse = PageCompositionSegment::parse(&out2).unwrap();
        assert_eq!(reparse.page_time_out, 20);
    }
}
