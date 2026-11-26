//! Page Composition Segment — ETSI EN 300 743 §7.2.2, Table 9 (segment_type 0x10).

use crate::error::{Error, Result};
use crate::segments::{
    PAGE_ID_LEN, PAGE_ID_OFFSET, SEGMENT_LENGTH_LEN, SEGMENT_LENGTH_OFFSET, SEGMENT_TYPE_OFFSET,
    be_u16,
};
use broadcast_core::len::{need_in, need_out};
use broadcast_core::{Parse, Serialize};

/// The page_composition_segment segment_type.
pub const SEGMENT_TYPE: u8 = 0x10;
/// Header: sync_byte(1) + segment_type(1) + page_id(2) + segment_length(2) = 6 bytes.
pub const HEADER_LEN: usize = 6;
/// Fixed after header: page_time_out(1) + page_version(4b)+page_state(2b)+reserved(2b) = 2 bytes.
pub const FIXED_LEN: usize = 2;
/// Each region entry: region_id(1) + reserved(1) + region_horizontal_address(2) + region_vertical_address(2) = 6 bytes.
pub const REGION_ENTRY_LEN: usize = 6;

/// Offset of `page_time_out` within the segment body.
const PAGE_TIME_OUT_OFFSET: usize = 0;
/// Offset of the version/state/reserved byte within the segment body.
const PAGE_STATE_OFFSET: usize = 1;
/// Shift placing `page_version_number` at the top of its byte.
const PAGE_VERSION_SHIFT: u32 = 4;
/// Mask of `page_version_number`.
const PAGE_VERSION_MASK: u8 = 0x0F;
/// Shift placing `page_state` below the version nibble.
const PAGE_STATE_SHIFT: u32 = 2;
/// Mask of `page_state`.
const PAGE_STATE_MASK: u8 = 0x03;
/// Mask of the reserved bits beside `page_state`.
const RESERVED_MASK: u8 = 0x03;

/// Offsets within a page region entry.
const REGION_ID_OFFSET: usize = 0;
const REGION_RESERVED_OFFSET: usize = 1;
const REGION_HORIZONTAL_OFFSET: usize = 2;
const REGION_VERTICAL_OFFSET: usize = 4;

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
            Self::Reserved(v) => v,
        }
    }
}

broadcast_core::impl_spec_display!(PageState, Reserved);

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
    fn serialize_into(&self, buf: &mut [u8]) {
        buf[REGION_ID_OFFSET] = self.region_id;
        buf[REGION_RESERVED_OFFSET] = self.reserved;
        buf[REGION_HORIZONTAL_OFFSET..REGION_VERTICAL_OFFSET]
            .copy_from_slice(&self.region_horizontal_address.to_be_bytes());
        buf[REGION_VERTICAL_OFFSET..REGION_ENTRY_LEN]
            .copy_from_slice(&self.region_vertical_address.to_be_bytes());
    }

    fn parse(bytes: &[u8]) -> Result<Self> {
        need_in(bytes, REGION_ENTRY_LEN, "page_region_entry")?;
        let reserved = bytes[REGION_RESERVED_OFFSET];
        Ok(PageRegionEntry {
            region_id: bytes[REGION_ID_OFFSET],
            reserved,
            region_horizontal_address: be_u16(bytes, REGION_HORIZONTAL_OFFSET),
            region_vertical_address: be_u16(bytes, REGION_VERTICAL_OFFSET),
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
}

impl<'a> Parse<'a> for PageCompositionSegment {
    type Error = Error;

    fn parse(bytes: &'a [u8]) -> Result<Self> {
        need_in(bytes, HEADER_LEN + FIXED_LEN, "page_composition_segment")?;
        if bytes[0] != crate::pes_data_field::SYNC_BYTE {
            return Err(Error::InvalidSync(broadcast_core::error::InvalidSync {
                what: "sync_byte",
                expected: u32::from(crate::pes_data_field::SYNC_BYTE),
                found: u32::from(bytes[0]),
            }));
        }
        if bytes[SEGMENT_TYPE_OFFSET] != SEGMENT_TYPE {
            return Err(Error::UnknownSegmentType(bytes[SEGMENT_TYPE_OFFSET]));
        }
        let page_id = be_u16(bytes, PAGE_ID_OFFSET);
        let segment_length = be_u16(bytes, SEGMENT_LENGTH_OFFSET) as usize;
        let total = HEADER_LEN + segment_length;
        need_in(bytes, total, "page_composition_segment data")?;
        let body = &bytes[HEADER_LEN..HEADER_LEN + segment_length];
        need_in(body, FIXED_LEN, "page_composition_segment body")?;
        let page_time_out = body[PAGE_TIME_OUT_OFFSET];
        let page_version_number = body[PAGE_STATE_OFFSET] >> PAGE_VERSION_SHIFT;
        let page_state_bits = (body[PAGE_STATE_OFFSET] >> PAGE_STATE_SHIFT) & PAGE_STATE_MASK;
        let reserved = body[PAGE_STATE_OFFSET] & RESERVED_MASK;
        let page_state = match page_state_bits {
            0x00 => PageState::Normal,
            0x01 => PageState::Acquisition,
            0x02 => PageState::ModeChange,
            v => PageState::Reserved(v),
        };

        let region_data = &body[FIXED_LEN..];
        let region_count = region_data.len() / REGION_ENTRY_LEN;
        // Table 9's region loop is `while (processed_length < segment_length)`
        // over whole 6-byte entries — there is no stuffing/suffix provision,
        // so a remainder means the segment is malformed, not that trailing
        // bytes should be captured (#1108/DS-W6: this used to be checked
        // here and then ALSO stored in a `suffix` field that, because of
        // this very check, could never end up holding anything).
        if !region_data.len().is_multiple_of(REGION_ENTRY_LEN) {
            return Err(Error::BufferTooShort(
                broadcast_core::error::BufferTooShort {
                    need: (region_count + 1) * REGION_ENTRY_LEN,
                    have: region_data.len(),
                    what: "page_composition_segment regions",
                },
            ));
        }
        let mut regions = alloc::vec::Vec::with_capacity(region_count);
        for i in 0..region_count {
            let entry_bytes = &region_data[i * REGION_ENTRY_LEN..][..REGION_ENTRY_LEN];
            regions.push(PageRegionEntry::parse(entry_bytes)?);
        }

        Ok(PageCompositionSegment {
            page_id,
            page_time_out,
            page_version_number,
            page_state,
            reserved,
            regions,
        })
    }
}

impl Serialize for PageCompositionSegment {
    type Error = Error;

    fn serialized_len(&self) -> usize {
        HEADER_LEN + FIXED_LEN + self.regions.len() * REGION_ENTRY_LEN
    }

    fn serialize_into(&self, buf: &mut [u8]) -> core::result::Result<usize, Self::Error> {
        let len = self.serialized_len();
        need_out(buf, len)?;
        buf[0] = crate::pes_data_field::SYNC_BYTE;
        buf[SEGMENT_TYPE_OFFSET] = SEGMENT_TYPE;
        buf[PAGE_ID_OFFSET..PAGE_ID_OFFSET + PAGE_ID_LEN]
            .copy_from_slice(&self.page_id.to_be_bytes());
        let seg_len = crate::segments::check_segment_length(len - HEADER_LEN)?;
        buf[SEGMENT_LENGTH_OFFSET..SEGMENT_LENGTH_OFFSET + SEGMENT_LENGTH_LEN]
            .copy_from_slice(&seg_len.to_be_bytes());

        buf[HEADER_LEN + PAGE_TIME_OUT_OFFSET] = self.page_time_out;
        buf[HEADER_LEN + PAGE_STATE_OFFSET] = (broadcast_core::len::fit_mask(
            self.page_version_number,
            PAGE_VERSION_MASK,
            "page_version_number",
        )? << PAGE_VERSION_SHIFT)
            | (broadcast_core::len::fit_mask(
                self.page_state.to_bits(),
                PAGE_STATE_MASK,
                "page_state",
            )? << PAGE_STATE_SHIFT)
            | broadcast_core::len::fit_mask(self.reserved, RESERVED_MASK, "reserved")?;

        for (i, region) in self.regions.iter().enumerate() {
            let off = HEADER_LEN + FIXED_LEN + i * REGION_ENTRY_LEN;
            region.serialize_into(&mut buf[off..off + REGION_ENTRY_LEN]);
        }
        Ok(len)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use broadcast_core::{Parse, Serialize};

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
        let out = seg.try_to_bytes().unwrap();
        assert_eq!(out, bytes);

        // Biting test: mutate page_time_out changes output
        let mut seg2 = seg.clone();
        seg2.page_time_out = 20;
        let out2 = seg2.try_to_bytes().unwrap();
        assert_ne!(out2, bytes);
        let reparse = PageCompositionSegment::parse(&out2).unwrap();
        assert_eq!(reparse.page_time_out, 20);
    }

    /// DS-W2 (#1108): sync_byte wasn't checked by any typed segment parser.
    #[test]
    fn rejects_bad_sync_byte() {
        let bytes = [
            0x00, 0x10, 0x00, 0x01, 0x00, 0x0E, 0x0A, 0x04, 0x01, 0x00, 0x00, 0x64, 0x00, 0x32,
            0x02, 0x00, 0x00, 0xC8, 0x00, 0x96,
        ];
        assert!(matches!(
            PageCompositionSegment::parse(&bytes),
            Err(Error::InvalidSync(broadcast_core::error::InvalidSync {
                found: 0x00,
                ..
            }))
        ));
    }
}
