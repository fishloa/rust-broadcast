//! Logical Channel Descriptor — EACEM/NorDig/D-Book private (tag 0x83).
//!
//! Carried inside NIT transport_stream_loop second descriptor loop.
//! Assigns an LCN (Logical Channel Number) to each service, plus a
//! `visible_service` flag for hiding services from the channel list.

use super::descriptor_body;
use crate::error::{Error, Result};
use alloc::vec::Vec;
use broadcast_common::{Parse, Serialize};

/// Descriptor tag for logical_channel_descriptor.
pub const TAG: u8 = 0x83;
const HEADER_LEN: usize = 2;
const ENTRY_LEN: usize = 4;
const VISIBLE_MASK: u8 = 0x80;
const RESERVED_BITS_MASK: u8 = 0x7C;
const RESERVED_SHIFT: u8 = 2;
const LCN_HI_MASK: u8 = 0x03;

/// Default of the 5-bit per-entry `reserved` between `visible_service_flag`
/// and the LCN: all ones (`0x1F`). EACEM/D-Book private layout (PDS 0x28; the
/// NorDig v1 layout on the same tag has `reserved(1)` + a 14-bit LCN, see
/// `nordig.rs`). Real-capture evidence: the run is `11111` in all 2456 entries
/// of the PDS-0x28 captures in the local archive (France TNT mux, TSDuck
/// test-059/092/124/184; `tests/local_archive_reserved_defaults.rs`). The bits
/// are preserved on parse.
pub const RESERVED_DEFAULT: u8 = 0x1F;
/// `logical_channel_number` is 10 bits (0..=1023).
const LCN_BITS: u32 = 10;

/// One LCN assignment.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub struct LogicalChannelEntry {
    /// Service being numbered.
    pub service_id: u16,
    /// Visible in the viewer's channel list.
    pub visible_service: bool,
    /// `reserved(5)` between the visible flag and the LCN (5 bits,
    /// right-aligned). Captured on parse and re-emitted verbatim;
    /// [`LogicalChannelEntry::new`] defaults to [`RESERVED_DEFAULT`] (`0x1F`).
    pub reserved: u8,
    /// 10-bit logical channel number (0..=1023).
    pub logical_channel_number: u16,
}

impl LogicalChannelEntry {
    /// Build an entry with the default reserved bits.
    #[must_use]
    pub const fn new(service_id: u16, visible_service: bool, logical_channel_number: u16) -> Self {
        Self {
            service_id,
            visible_service,
            reserved: RESERVED_DEFAULT,
            logical_channel_number,
        }
    }
}

/// Logical Channel Descriptor.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct LogicalChannelDescriptor {
    /// Entries in wire order.
    pub entries: Vec<LogicalChannelEntry>,
}

impl<'a> Parse<'a> for LogicalChannelDescriptor {
    type Error = crate::error::Error;
    fn parse(bytes: &'a [u8]) -> Result<Self> {
        let body = descriptor_body(
            bytes,
            TAG,
            "LogicalChannelDescriptor",
            "unexpected tag for logical_channel_descriptor",
        )?;
        if body.len() % ENTRY_LEN != 0 {
            return Err(Error::InvalidDescriptor {
                tag: TAG,
                reason: "descriptor_length must be a multiple of 4",
            });
        }
        let mut entries = Vec::with_capacity(body.len() / ENTRY_LEN);
        for chunk in body.chunks_exact(ENTRY_LEN) {
            let service_id = crate::wire::be_u16(chunk, 0, "logical_channel")?;
            let flags = chunk[2];
            let visible_service = flags & VISIBLE_MASK != 0;
            let reserved = (flags & RESERVED_BITS_MASK) >> RESERVED_SHIFT;
            let lcn = (u16::from(flags & LCN_HI_MASK) << 8) | u16::from(chunk[3]);
            entries.push(LogicalChannelEntry {
                service_id,
                visible_service,
                reserved,
                logical_channel_number: lcn,
            });
        }
        Ok(Self { entries })
    }
}

impl Serialize for LogicalChannelDescriptor {
    type Error = crate::error::Error;
    fn serialized_len(&self) -> usize {
        HEADER_LEN + ENTRY_LEN * self.entries.len()
    }

    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        let len = self.serialized_len();
        if buf.len() < len {
            return Err(Error::OutputBufferTooSmall {
                need: len,
                have: buf.len(),
            });
        }
        crate::descriptors::write_descriptor_header(buf, TAG, len - HEADER_LEN)?;
        let mut offset = HEADER_LEN;
        for entry in &self.entries {
            buf[offset..offset + 2].copy_from_slice(&entry.service_id.to_be_bytes());
            let visible_byte = if entry.visible_service {
                VISIBLE_MASK
            } else {
                0
            };
            let lcn = broadcast_common::len::fit_bits(
                u64::from(entry.logical_channel_number),
                LCN_BITS,
                "logical_channel_number",
            )? as u16;
            let flags = visible_byte
                | (broadcast_common::len::fit_mask(
                    entry.reserved,
                    RESERVED_BITS_MASK >> RESERVED_SHIFT,
                    "reserved",
                )? << RESERVED_SHIFT)
                | ((lcn >> 8) as u8 & LCN_HI_MASK);
            buf[offset + 2] = flags;
            buf[offset + 3] = (lcn & 0xFF) as u8;
            offset += ENTRY_LEN;
        }
        Ok(len)
    }
}
impl crate::traits::DescriptorDef<'_> for LogicalChannelDescriptor {
    const TAG: u8 = TAG;
    const NAME: &'static str = "LOGICAL_CHANNEL";
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_single_entry() {
        let bytes = [TAG, 4, 0x00, 0x01, 0xFC, 0x05];
        let d = LogicalChannelDescriptor::parse(&bytes).unwrap();
        assert_eq!(d.entries.len(), 1);
        assert_eq!(d.entries[0].service_id, 1);
        assert!(d.entries[0].visible_service);
        assert_eq!(d.entries[0].logical_channel_number, 5);
    }

    #[test]
    fn parse_extracts_visible_service_false() {
        let bytes = [TAG, 4, 0x00, 0x02, 0x7C, 0x0A];
        let d = LogicalChannelDescriptor::parse(&bytes).unwrap();
        assert_eq!(d.entries.len(), 1);
        assert!(!d.entries[0].visible_service);
        assert_eq!(d.entries[0].service_id, 2);
        assert_eq!(d.entries[0].logical_channel_number, 10);
    }

    #[test]
    fn parse_extracts_logical_channel_number_full_10_bit_range() {
        let bytes = [TAG, 4, 0x00, 0x03, 0xFF, 0xFF];
        let d = LogicalChannelDescriptor::parse(&bytes).unwrap();
        assert_eq!(d.entries.len(), 1);
        assert_eq!(d.entries[0].logical_channel_number, 1023);
        assert!(d.entries[0].visible_service);
    }

    #[test]
    fn parse_multiple_entries_preserves_order() {
        let bytes = [
            TAG, 12, 0x00, 0x01, 0xFC, 0x01, 0x00, 0x02, 0xFC, 0x02, 0x00, 0x03, 0xFC, 0x03,
        ];
        let d = LogicalChannelDescriptor::parse(&bytes).unwrap();
        assert_eq!(d.entries.len(), 3);
        assert_eq!(d.entries[0].service_id, 1);
        assert_eq!(d.entries[0].logical_channel_number, 1);
        assert_eq!(d.entries[1].service_id, 2);
        assert_eq!(d.entries[1].logical_channel_number, 2);
        assert_eq!(d.entries[2].service_id, 3);
        assert_eq!(d.entries[2].logical_channel_number, 3);
    }

    #[test]
    fn parse_rejects_wrong_tag() {
        let err = LogicalChannelDescriptor::parse(&[0x84, 4, 0x00, 0x01, 0xFC, 0x05]).unwrap_err();
        assert!(matches!(err, Error::InvalidDescriptor { tag: 0x84, .. }));
    }

    #[test]
    fn parse_rejects_length_not_multiple_of_4() {
        let bytes = [TAG, 5, 0x00, 0x01, 0xFC, 0x05, 0xFF];
        let err = LogicalChannelDescriptor::parse(&bytes).unwrap_err();
        assert!(matches!(err, Error::InvalidDescriptor { tag: TAG, .. }));
    }

    #[test]
    fn parse_tolerates_cleared_reserved_bits() {
        // Real-world broadcasts (TNTSat / ORF Digital) emit
        // logical_channel_descriptors where the reserved bits aren't all
        // set. ETSI EN 300 468 §5.1 says "decoders shall ignore reserved
        // bits" — so we MUST accept the descriptor and just decode the
        // documented fields.
        //
        // The flags byte 0x00 here has visible_service=0, reserved=0,
        // LCN-hi=0. Combined with byte 0x05 (LCN-lo) the LCN is 5.
        let bytes = [TAG, 4, 0x00, 0x01, 0x00, 0x05];
        let d = LogicalChannelDescriptor::parse(&bytes).expect("tolerant parse");
        assert_eq!(d.entries.len(), 1);
        assert_eq!(d.entries[0].service_id, 1);
        assert_eq!(d.entries[0].logical_channel_number, 5);
        assert!(!d.entries[0].visible_service);
    }

    #[test]
    fn empty_descriptor_valid() {
        let bytes = [TAG, 0];
        let d = LogicalChannelDescriptor::parse(&bytes).unwrap();
        assert!(d.entries.is_empty());
    }

    #[test]
    fn serialize_round_trip() {
        let d = LogicalChannelDescriptor {
            entries: vec![
                LogicalChannelEntry::new(1, true, 5),
                LogicalChannelEntry::new(0x0102, false, 1023),
            ],
        };
        let mut buf = vec![0u8; d.serialized_len()];
        d.serialize_into(&mut buf).unwrap();
        let re = LogicalChannelDescriptor::parse(&buf).unwrap();
        assert_eq!(d, re);
    }

    #[test]
    fn serialize_rejects_over_10bit_lcn() {
        // Pre-fix `>> 8 as u8 & 0x03` silently clipped bits 10..16 (r03-W3).
        let d = LogicalChannelDescriptor {
            entries: alloc::vec![LogicalChannelEntry::new(1, true, 1024)],
        };
        let mut buf = vec![0u8; d.serialized_len()];
        assert!(matches!(
            d.serialize_into(&mut buf).unwrap_err(),
            Error::FieldOverflow(_)
        ));
    }

    /// Every `reserved(5)` value re-serializes byte-identically (real senders
    /// such as TNTSat / ORF write `0`; default `0x1F` is kept for new values).
    #[test]
    fn reserved_preserved() {
        for r in 0u8..=0x1F {
            let flags = 0x80 | (r << 2) | 0x02;
            let bytes = [TAG, 4, 0x00, 0x01, flags, 0x05];
            let d = LogicalChannelDescriptor::parse(&bytes).unwrap();
            assert_eq!(d.entries[0].reserved, r);
            let mut buf = vec![0u8; d.serialized_len()];
            d.serialize_into(&mut buf).unwrap();
            assert_eq!(buf, bytes, "r {r:#x}");
        }
    }

    /// `new()` carries the documented default (`0x1F`).
    #[test]
    fn default_reserved_is_documented_value() {
        let d = LogicalChannelDescriptor {
            entries: vec![LogicalChannelEntry::new(1, true, 5)],
        };
        let mut buf = vec![0u8; d.serialized_len()];
        d.serialize_into(&mut buf).unwrap();
        assert_eq!(buf, [TAG, 4, 0x00, 0x01, 0xFC, 0x05]);
    }
}
