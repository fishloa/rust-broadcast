//! Logical Channel Descriptor — EACEM/NorDig/D-Book private (tag 0x83).
//!
//! Carried inside NIT transport_stream_loop second descriptor loop.
//! Assigns an LCN (Logical Channel Number) to each service, plus a
//! `visible_service` flag for hiding services from the channel list.

use crate::error::{Error, Result};
use crate::traits::Descriptor;
use dvb_common::{Parse, Serialize};

/// Descriptor tag for logical_channel_descriptor.
pub const TAG: u8 = 0x83;
const HEADER_LEN: usize = 2;
const ENTRY_LEN: usize = 4;
const VISIBLE_MASK: u8 = 0x80;
const RESERVED_BITS_MASK: u8 = 0x7C;
const LCN_HI_MASK: u8 = 0x03;

/// One LCN assignment.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct LogicalChannelEntry {
    /// Service being numbered.
    pub service_id: u16,
    /// Visible in the viewer's channel list.
    pub visible_service: bool,
    /// 10-bit logical channel number (0..=1023).
    pub logical_channel_number: u16,
}

/// Logical Channel Descriptor.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct LogicalChannelDescriptor {
    /// Entries in wire order.
    pub entries: Vec<LogicalChannelEntry>,
}

impl<'a> Parse<'a> for LogicalChannelDescriptor {
    type Error = crate::error::Error;
    fn parse(bytes: &'a [u8]) -> Result<Self> {
        if bytes.len() < HEADER_LEN {
            return Err(Error::BufferTooShort {
                need: HEADER_LEN,
                have: bytes.len(),
                what: "LogicalChannelDescriptor header",
            });
        }
        if bytes[0] != TAG {
            return Err(Error::InvalidDescriptor {
                tag: bytes[0],
                reason: "unexpected tag for logical_channel_descriptor",
            });
        }
        let length = bytes[1] as usize;
        if length % ENTRY_LEN != 0 {
            return Err(Error::InvalidDescriptor {
                tag: TAG,
                reason: "descriptor_length must be a multiple of 4",
            });
        }
        let body_start = HEADER_LEN;
        let body_end = body_start + length;
        if bytes.len() < body_end {
            return Err(Error::BufferTooShort {
                need: body_end,
                have: bytes.len(),
                what: "LogicalChannelDescriptor body",
            });
        }
        let mut entries = Vec::with_capacity(length / ENTRY_LEN);
        let mut offset = body_start;
        while offset < body_end {
            let service_id = u16::from_be_bytes([bytes[offset], bytes[offset + 1]]);
            let flags = bytes[offset + 2];
            // ETSI EN 300 468 §5.1: "Decoders shall ignore reserved bits."
            // The earlier strict check (`flags & RESERVED_BITS_MASK ==
            // RESERVED_BITS_MASK`) rejected real-world broadcasts where the
            // reserved bits were cleared — observed on TNTSat / ORF Digital
            // on 5°W and 19.2°E — and silently swallowed every LCN. Tolerate
            // either reserved-bits value; only the documented
            // visible_service and lcn fields drive behaviour.
            let visible_service = flags & VISIBLE_MASK != 0;
            let lcn = (u16::from(flags & LCN_HI_MASK) << 8) | u16::from(bytes[offset + 3]);
            entries.push(LogicalChannelEntry {
                service_id,
                visible_service,
                logical_channel_number: lcn,
            });
            offset += ENTRY_LEN;
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
        buf[0] = TAG;
        buf[1] = (len - HEADER_LEN) as u8;
        let mut offset = HEADER_LEN;
        for entry in &self.entries {
            buf[offset..offset + 2].copy_from_slice(&entry.service_id.to_be_bytes());
            let visible_byte = if entry.visible_service {
                VISIBLE_MASK
            } else {
                0
            };
            let flags = visible_byte
                | RESERVED_BITS_MASK
                | ((entry.logical_channel_number >> 8) as u8 & LCN_HI_MASK);
            buf[offset + 2] = flags;
            buf[offset + 3] = (entry.logical_channel_number & 0xFF) as u8;
            offset += ENTRY_LEN;
        }
        Ok(len)
    }
}

impl<'a> Descriptor<'a> for LogicalChannelDescriptor {
    const TAG: u8 = TAG;
    fn descriptor_length(&self) -> u8 {
        (self.serialized_len() - HEADER_LEN) as u8
    }
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
                LogicalChannelEntry {
                    service_id: 1,
                    visible_service: true,
                    logical_channel_number: 5,
                },
                LogicalChannelEntry {
                    service_id: 0x0102,
                    visible_service: false,
                    logical_channel_number: 1023,
                },
            ],
        };
        let mut buf = vec![0u8; d.serialized_len()];
        d.serialize_into(&mut buf).unwrap();
        let re = LogicalChannelDescriptor::parse(&buf).unwrap();
        assert_eq!(d, re);
    }
}
