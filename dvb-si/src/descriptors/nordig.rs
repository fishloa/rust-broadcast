//! NorDig Logical Channel Descriptor v1 & v2 — NorDig Unified Requirements
//! v3.1.1 §12.2.9.2–12.2.9.3 (tags 0x83, 0x87), scoped by
//! PDS_NORDIG = 0x00000029.
//!
//! v1 (tag 0x83) assigns a 14-bit LCN per service (4 bytes/entry).
//! v2 (tag 0x87) groups services into named channel-lists with a 10-bit LCN,
//! plus a per-list country_code and Annex-A channel_list_name.

use super::descriptor_body;
use crate::error::{Error, Result};
use crate::text::LangCode;
use alloc::vec::Vec;
use broadcast_common::{Parse, Serialize};

// ---------------------------------------------------------------------------
// NordigLogicalChannelV1 — tag 0x83, 14-bit LCN
// ---------------------------------------------------------------------------

/// Descriptor tag for NorDig Logical Channel Descriptor v1.
pub const TAG_V1: u8 = 0x83;
const V1_HEADER_LEN: usize = 2;
const V1_ENTRY_LEN: usize = 4;
const V1_VISIBLE_MASK: u8 = 0x80;
const V1_RESERVED_MASK: u8 = 0x40;
const V1_LCN_HI_MASK: u8 = 0x3F;

/// One NorDig LCD v1 LCN assignment.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct NordigLogicalChannelV1Entry {
    /// Service being numbered.
    pub service_id: u16,
    /// Visible in the viewer's channel list.
    pub visible_service_flag: bool,
    /// 14-bit logical channel number (0..=16383).
    pub logical_channel_number: u16,
}

/// NorDig Logical Channel Descriptor version 1.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct NordigLogicalChannelV1 {
    /// Entries in wire order.
    pub entries: Vec<NordigLogicalChannelV1Entry>,
}

impl<'a> Parse<'a> for NordigLogicalChannelV1 {
    type Error = crate::error::Error;
    fn parse(bytes: &'a [u8]) -> Result<Self> {
        let body = descriptor_body(
            bytes,
            TAG_V1,
            "NordigLogicalChannelV1",
            "unexpected tag for nordig_logical_channel_descriptor_v1",
        )?;
        if body.len() % V1_ENTRY_LEN != 0 {
            return Err(Error::InvalidDescriptor {
                tag: TAG_V1,
                reason: "descriptor_length must be a multiple of 4",
            });
        }
        let mut entries = Vec::with_capacity(body.len() / V1_ENTRY_LEN);
        for chunk in body.chunks_exact(V1_ENTRY_LEN) {
            let (sid_bytes, rest) = chunk.split_first_chunk::<2>().unwrap();
            let service_id = u16::from_be_bytes(*sid_bytes);
            let flags = rest[0];
            let visible_service_flag = flags & V1_VISIBLE_MASK != 0;
            let lcn = (u16::from(flags & V1_LCN_HI_MASK) << 8) | u16::from(rest[1]);
            entries.push(NordigLogicalChannelV1Entry {
                service_id,
                visible_service_flag,
                logical_channel_number: lcn,
            });
        }
        Ok(Self { entries })
    }
}

impl Serialize for NordigLogicalChannelV1 {
    type Error = crate::error::Error;
    fn serialized_len(&self) -> usize {
        V1_HEADER_LEN + V1_ENTRY_LEN * self.entries.len()
    }

    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        let len = self.serialized_len();
        if buf.len() < len {
            return Err(Error::OutputBufferTooSmall {
                need: len,
                have: buf.len(),
            });
        }
        buf[0] = TAG_V1;
        buf[1] = ((len - V1_HEADER_LEN) / V1_ENTRY_LEN * V1_ENTRY_LEN) as u8;
        let mut offset = V1_HEADER_LEN;
        for entry in &self.entries {
            buf[offset..offset + 2].copy_from_slice(&entry.service_id.to_be_bytes());
            let visible_byte = if entry.visible_service_flag {
                V1_VISIBLE_MASK
            } else {
                0
            };
            let flags = visible_byte
                | V1_RESERVED_MASK
                | ((entry.logical_channel_number >> 8) as u8 & V1_LCN_HI_MASK);
            buf[offset + 2] = flags;
            buf[offset + 3] = (entry.logical_channel_number & 0xFF) as u8;
            offset += V1_ENTRY_LEN;
        }
        Ok(len)
    }
}

impl crate::traits::DescriptorDef<'_> for NordigLogicalChannelV1 {
    const TAG: u8 = TAG_V1;
    const NAME: &'static str = "NORDIG_LOGICAL_CHANNEL_V1";
}

// ---------------------------------------------------------------------------
// NordigLogicalChannelV2 — tag 0x87, channel-list grouped
// ---------------------------------------------------------------------------

/// Descriptor tag for NorDig Logical Channel Descriptor v2.
pub const TAG_V2: u8 = 0x87;
const V2_HEADER_LEN: usize = 2;
const V2_SERVICE_ENTRY_LEN: usize = 4;
const V2_VISIBLE_MASK: u8 = 0x80;
const V2_RESERVED_MASK: u8 = 0x7C;
const V2_LCN_HI_MASK: u8 = 0x03;

/// One service entry inside a NorDig LCD v2 channel list.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct NordigLogicalChannelV2Service {
    /// Service being numbered.
    pub service_id: u16,
    /// Visible in the viewer's channel list.
    pub visible_service_flag: bool,
    /// 10-bit logical channel number (0..=1023).
    pub logical_channel_number: u16,
}

/// One channel list inside a NorDig LCD v2 descriptor.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct NordigLogicalChannelV2ChannelList {
    /// Channel list identifier.
    pub channel_list_id: u8,
    /// Raw Annex-A bytes (EN 300 468) for the channel list name.
    /// Use `crate::text::DvbText::new(&self.channel_list_name)` to decode.
    pub channel_list_name: Vec<u8>,
    /// ISO 3166 country code.
    pub country_code: LangCode,
    /// Services in this list, wire order.
    pub services: Vec<NordigLogicalChannelV2Service>,
}

/// NorDig Logical Channel Descriptor version 2.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct NordigLogicalChannelV2 {
    /// Channel lists in wire order.
    pub channel_lists: Vec<NordigLogicalChannelV2ChannelList>,
}

impl<'a> Parse<'a> for NordigLogicalChannelV2 {
    type Error = crate::error::Error;
    fn parse(bytes: &'a [u8]) -> Result<Self> {
        let mut body = descriptor_body(
            bytes,
            TAG_V2,
            "NordigLogicalChannelV2",
            "unexpected tag for nordig_logical_channel_descriptor_v2",
        )?;
        let mut channel_lists = Vec::new();
        while !body.is_empty() {
            if body.len() < 2 {
                return Err(Error::BufferTooShort {
                    need: 2,
                    have: body.len(),
                    what: "NordigLogicalChannelV2 channel_list header",
                });
            }
            let channel_list_id = body[0];
            let name_len = body[1] as usize;
            let name_start = 2;
            let name_end = name_start + name_len;
            if body.len() < name_end + 3 + 1 {
                return Err(Error::BufferTooShort {
                    need: name_end + 3 + 1,
                    have: body.len(),
                    what: "NordigLogicalChannelV2 channel_list country_code + descriptor_length",
                });
            }
            let channel_list_name = body[name_start..name_end].to_vec();
            let cc_start = name_end;
            let country_code = LangCode([body[cc_start], body[cc_start + 1], body[cc_start + 2]]);
            let desc_len = body[cc_start + 3] as usize;
            let svc_start = cc_start + 4;
            let svc_end = svc_start + desc_len;
            if body.len() < svc_end {
                return Err(Error::BufferTooShort {
                    need: svc_end,
                    have: body.len(),
                    what: "NordigLogicalChannelV2 service loop",
                });
            }
            if desc_len % V2_SERVICE_ENTRY_LEN != 0 {
                return Err(Error::InvalidDescriptor {
                    tag: TAG_V2,
                    reason: "descriptor_length in channel list must be a multiple of 4",
                });
            }
            let svc_body = &body[svc_start..svc_end];
            let mut services = Vec::with_capacity(svc_body.len() / V2_SERVICE_ENTRY_LEN);
            for chunk in svc_body.chunks_exact(V2_SERVICE_ENTRY_LEN) {
                let (sid_bytes, rest) = chunk.split_first_chunk::<2>().unwrap();
                let service_id = u16::from_be_bytes(*sid_bytes);
                let flags = rest[0];
                let visible_service_flag = flags & V2_VISIBLE_MASK != 0;
                let lcn = (u16::from(flags & V2_LCN_HI_MASK) << 8) | u16::from(rest[1]);
                services.push(NordigLogicalChannelV2Service {
                    service_id,
                    visible_service_flag,
                    logical_channel_number: lcn,
                });
            }
            channel_lists.push(NordigLogicalChannelV2ChannelList {
                channel_list_id,
                channel_list_name,
                country_code,
                services,
            });
            body = &body[svc_end..];
        }
        Ok(Self { channel_lists })
    }
}

impl Serialize for NordigLogicalChannelV2 {
    type Error = crate::error::Error;
    fn serialized_len(&self) -> usize {
        let mut total = V2_HEADER_LEN;
        for cl in &self.channel_lists {
            total += 2 // id + name_len
                + cl.channel_list_name.len()
                + 3 // country_code
                + 1 // descriptor_length
                + V2_SERVICE_ENTRY_LEN * cl.services.len();
        }
        total
    }

    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        let len = self.serialized_len();
        if buf.len() < len {
            return Err(Error::OutputBufferTooSmall {
                need: len,
                have: buf.len(),
            });
        }
        buf[0] = TAG_V2;
        buf[1] = (len - V2_HEADER_LEN) as u8;
        let mut offset = V2_HEADER_LEN;
        for cl in &self.channel_lists {
            buf[offset] = cl.channel_list_id;
            buf[offset + 1] = cl.channel_list_name.len() as u8;
            offset += 2;
            buf[offset..offset + cl.channel_list_name.len()].copy_from_slice(&cl.channel_list_name);
            offset += cl.channel_list_name.len();
            buf[offset..offset + 3].copy_from_slice(&cl.country_code.0);
            offset += 3;
            let desc_len = V2_SERVICE_ENTRY_LEN * cl.services.len();
            buf[offset] = desc_len as u8;
            offset += 1;
            for svc in &cl.services {
                buf[offset..offset + 2].copy_from_slice(&svc.service_id.to_be_bytes());
                let visible_byte = if svc.visible_service_flag {
                    V2_VISIBLE_MASK
                } else {
                    0
                };
                let flags = visible_byte
                    | V2_RESERVED_MASK
                    | ((svc.logical_channel_number >> 8) as u8 & V2_LCN_HI_MASK);
                buf[offset + 2] = flags;
                buf[offset + 3] = (svc.logical_channel_number & 0xFF) as u8;
                offset += V2_SERVICE_ENTRY_LEN;
            }
        }
        Ok(len)
    }
}

impl crate::traits::DescriptorDef<'_> for NordigLogicalChannelV2 {
    const TAG: u8 = TAG_V2;
    const NAME: &'static str = "NORDIG_LOGICAL_CHANNEL_V2";
}

#[cfg(test)]
mod tests {
    use super::*;

    // -----------------------------------------------------------------------
    // V1 tests
    // -----------------------------------------------------------------------

    #[test]
    fn v1_parse_single_entry() {
        // LCN=5: LCN_hi(6 bits)=0, LCN_lo=5
        // Flags: visible=1<<7=0x80 | reserved=1<<6=0x40 | LCN_hi=0 = 0xC0
        let bytes = [TAG_V1, 4, 0x00, 0x01, 0xC0, 0x05];
        let d = NordigLogicalChannelV1::parse(&bytes).unwrap();
        assert_eq!(d.entries.len(), 1);
        assert_eq!(d.entries[0].service_id, 1);
        assert!(d.entries[0].visible_service_flag);
        assert_eq!(d.entries[0].logical_channel_number, 5);
    }

    #[test]
    fn v1_parse_visible_service_false() {
        // LCN=10: LCN_hi=0, LCN_lo=10
        // Flags: visible=0 | reserved=1<<6=0x40 | LCN_hi=0 = 0x40
        let bytes = [TAG_V1, 4, 0x00, 0x02, 0x40, 0x0A];
        let d = NordigLogicalChannelV1::parse(&bytes).unwrap();
        assert_eq!(d.entries.len(), 1);
        assert!(!d.entries[0].visible_service_flag);
        assert_eq!(d.entries[0].service_id, 2);
        assert_eq!(d.entries[0].logical_channel_number, 10);
    }

    #[test]
    fn v1_parse_full_14_bit_lcn_range() {
        // LCN=16383 = 0x3FFF: LCN_hi(6)=0x3F, LCN_lo=0xFF
        // Flags: visible=1 | reserved=1 | LCN_hi=0x3F
        let bytes = [TAG_V1, 4, 0x00, 0x03, 0xFF, 0xFF];
        let d = NordigLogicalChannelV1::parse(&bytes).unwrap();
        assert_eq!(d.entries.len(), 1);
        assert_eq!(d.entries[0].logical_channel_number, 16383);
    }

    #[test]
    fn v1_parse_lcn_exceeds_10_bit_max() {
        // 1024 = 0b_00 0100_0000_0000 — top 6 bits of 14-bit LCN in flags
        let flags_lcn_hi = ((1024u16 >> 8) & 0x3F) as u8;
        let lcn_lo = (1024u16 & 0xFF) as u8;
        let bytes = [
            TAG_V1,
            4,
            0x00,
            0x01,
            V1_VISIBLE_MASK | V1_RESERVED_MASK | flags_lcn_hi,
            lcn_lo,
        ];
        let d = NordigLogicalChannelV1::parse(&bytes).unwrap();
        assert_eq!(d.entries.len(), 1);
        assert_eq!(d.entries[0].service_id, 1);
        assert!(d.entries[0].visible_service_flag);
        assert_eq!(d.entries[0].logical_channel_number, 1024);
    }

    #[test]
    fn v1_parse_multiple_entries() {
        // LCN=1: flags=0xC0(visible+reserved), lo=0x01
        let bytes = [
            TAG_V1, 12, 0x00, 0x01, 0xC0, 0x01, 0x00, 0x02, 0xC0, 0x02, 0x00, 0x03, 0xC0, 0x03,
        ];
        let d = NordigLogicalChannelV1::parse(&bytes).unwrap();
        assert_eq!(d.entries.len(), 3);
        assert_eq!(d.entries[0].service_id, 1);
        assert_eq!(d.entries[0].logical_channel_number, 1);
        assert_eq!(d.entries[1].service_id, 2);
        assert_eq!(d.entries[1].logical_channel_number, 2);
        assert_eq!(d.entries[2].service_id, 3);
        assert_eq!(d.entries[2].logical_channel_number, 3);
    }

    #[test]
    fn v1_parse_rejects_wrong_tag() {
        let err = NordigLogicalChannelV1::parse(&[0x84, 4, 0x00, 0x01, 0xC0, 0x05]).unwrap_err();
        assert!(matches!(err, Error::InvalidDescriptor { tag: 0x84, .. }));
    }

    #[test]
    fn v1_parse_rejects_length_not_multiple_of_4() {
        let bytes = [TAG_V1, 5, 0x00, 0x01, 0xC0, 0x05, 0xFF];
        let err = NordigLogicalChannelV1::parse(&bytes).unwrap_err();
        assert!(matches!(err, Error::InvalidDescriptor { tag: TAG_V1, .. }));
    }

    #[test]
    fn v1_parse_tolerates_cleared_reserved_bit() {
        // visible=1, reserved=0, LCN_hi=0, LCN_lo=5
        let bytes = [TAG_V1, 4, 0x00, 0x01, 0x80, 0x05];
        let d = NordigLogicalChannelV1::parse(&bytes).unwrap();
        assert_eq!(d.entries.len(), 1);
        assert_eq!(d.entries[0].service_id, 1);
        assert_eq!(d.entries[0].logical_channel_number, 5);
    }

    #[test]
    fn v1_empty_descriptor_valid() {
        let bytes = [TAG_V1, 0];
        let d = NordigLogicalChannelV1::parse(&bytes).unwrap();
        assert!(d.entries.is_empty());
    }

    #[test]
    fn v1_serialize_round_trip() {
        let d = NordigLogicalChannelV1 {
            entries: vec![
                NordigLogicalChannelV1Entry {
                    service_id: 1,
                    visible_service_flag: true,
                    logical_channel_number: 5,
                },
                NordigLogicalChannelV1Entry {
                    service_id: 0x0102,
                    visible_service_flag: false,
                    logical_channel_number: 16383,
                },
            ],
        };
        let mut buf = vec![0u8; d.serialized_len()];
        d.serialize_into(&mut buf).unwrap();
        let re = NordigLogicalChannelV1::parse(&buf).unwrap();
        assert_eq!(d, re);
    }

    #[test]
    fn v1_serialize_round_trip_byte_identity() {
        let bytes = [TAG_V1, 8, 0x00, 0x01, 0xC0, 0x05, 0x00, 0x02, 0x40, 0x0A];
        let d = NordigLogicalChannelV1::parse(&bytes).unwrap();
        let mut buf = vec![0u8; d.serialized_len()];
        d.serialize_into(&mut buf).unwrap();
        assert_eq!(buf, bytes);
    }

    // -----------------------------------------------------------------------
    // V2 tests
    // -----------------------------------------------------------------------

    #[test]
    fn v2_parse_single_channel_list_single_service() {
        let name = b"Terrestrial";
        let mut bytes = vec![TAG_V2, 0];
        bytes.push(0x01); // channel_list_id
        bytes.push(name.len() as u8);
        bytes.extend_from_slice(name);
        bytes.extend_from_slice(b"GBR"); // country_code
        bytes.push(4); // descriptor_length
        bytes.extend_from_slice(&[0x00, 0x01, 0xFC, 0x05]);
        // Fix descriptor_length
        bytes[1] = (bytes.len() - 2) as u8;

        let d = NordigLogicalChannelV2::parse(&bytes).unwrap();
        assert_eq!(d.channel_lists.len(), 1);
        let cl = &d.channel_lists[0];
        assert_eq!(cl.channel_list_id, 0x01);
        assert_eq!(cl.channel_list_name, name);
        assert_eq!(cl.country_code, LangCode(*b"GBR"));
        assert_eq!(cl.services.len(), 1);
        assert_eq!(cl.services[0].service_id, 1);
        assert!(cl.services[0].visible_service_flag);
        assert_eq!(cl.services[0].logical_channel_number, 5);
    }

    #[test]
    fn v2_parse_multiple_channel_lists_multiple_services() {
        let name1 = b"Terrestrial";
        let name2 = b"Cable";
        let mut bytes = vec![TAG_V2, 0];
        // List 1
        bytes.push(0x01);
        bytes.push(name1.len() as u8);
        bytes.extend_from_slice(name1);
        bytes.extend_from_slice(b"GBR");
        bytes.push(8);
        bytes.extend_from_slice(&[0x00, 0x01, 0xFC, 0x01, 0x00, 0x02, 0xFC, 0x02]);
        // List 2
        bytes.push(0x02);
        bytes.push(name2.len() as u8);
        bytes.extend_from_slice(name2);
        bytes.extend_from_slice(b"DEU");
        bytes.push(4);
        bytes.extend_from_slice(&[0x00, 0x03, 0xFC, 0x03]);
        bytes[1] = (bytes.len() - 2) as u8;

        let d = NordigLogicalChannelV2::parse(&bytes).unwrap();
        assert_eq!(d.channel_lists.len(), 2);

        let cl1 = &d.channel_lists[0];
        assert_eq!(cl1.channel_list_id, 1);
        assert_eq!(cl1.channel_list_name, name1);
        assert_eq!(cl1.country_code, LangCode([b'G', b'B', b'R']));
        assert_eq!(cl1.services.len(), 2);
        assert_eq!(cl1.services[0].service_id, 1);
        assert_eq!(cl1.services[0].logical_channel_number, 1);
        assert_eq!(cl1.services[1].service_id, 2);
        assert_eq!(cl1.services[1].logical_channel_number, 2);

        let cl2 = &d.channel_lists[1];
        assert_eq!(cl2.channel_list_id, 2);
        assert_eq!(cl2.channel_list_name, name2);
        assert_eq!(cl2.country_code, LangCode([b'D', b'E', b'U']));
        assert_eq!(cl2.services.len(), 1);
        assert_eq!(cl2.services[0].service_id, 3);
        assert_eq!(cl2.services[0].logical_channel_number, 3);
    }

    #[test]
    fn v2_parse_visible_service_false() {
        let mut bytes = vec![TAG_V2, 0];
        bytes.push(0x01);
        bytes.push(0);
        bytes.extend_from_slice(b"NOR");
        bytes.push(4);
        bytes.extend_from_slice(&[0x00, 0x01, 0x7C, 0x0A]);
        bytes[1] = (bytes.len() - 2) as u8;

        let d = NordigLogicalChannelV2::parse(&bytes).unwrap();
        assert!(!d.channel_lists[0].services[0].visible_service_flag);
        assert_eq!(d.channel_lists[0].services[0].logical_channel_number, 10);
    }

    #[test]
    fn v2_parse_lcn_full_10_bit() {
        let mut bytes = vec![TAG_V2, 0];
        bytes.push(0x01);
        bytes.push(0);
        bytes.extend_from_slice(b"SWE");
        bytes.push(4);
        bytes.extend_from_slice(&[0x00, 0x01, 0xFF, 0xFF]);
        bytes[1] = (bytes.len() - 2) as u8;

        let d = NordigLogicalChannelV2::parse(&bytes).unwrap();
        assert_eq!(d.channel_lists[0].services[0].logical_channel_number, 1023);
    }

    #[test]
    fn v2_parse_empty_channel_list_name() {
        let mut bytes = vec![TAG_V2, 0];
        bytes.push(0x01);
        bytes.push(0); // name_len = 0
        bytes.extend_from_slice(b"FRA");
        bytes.push(0); // descriptor_length = 0
        bytes[1] = (bytes.len() - 2) as u8;

        let d = NordigLogicalChannelV2::parse(&bytes).unwrap();
        assert_eq!(d.channel_lists.len(), 1);
        assert!(d.channel_lists[0].channel_list_name.is_empty());
        assert!(d.channel_lists[0].services.is_empty());
    }

    #[test]
    fn v2_parse_rejects_wrong_tag() {
        let err = NordigLogicalChannelV2::parse(&[0x88, 0]).unwrap_err();
        assert!(matches!(err, Error::InvalidDescriptor { tag: 0x88, .. }));
    }

    #[test]
    fn v2_serialize_round_trip() {
        let d = NordigLogicalChannelV2 {
            channel_lists: vec![
                NordigLogicalChannelV2ChannelList {
                    channel_list_id: 1,
                    channel_list_name: b"Terrestrial".to_vec(),
                    country_code: LangCode([b'G', b'B', b'R']),
                    services: vec![
                        NordigLogicalChannelV2Service {
                            service_id: 1,
                            visible_service_flag: true,
                            logical_channel_number: 1,
                        },
                        NordigLogicalChannelV2Service {
                            service_id: 2,
                            visible_service_flag: false,
                            logical_channel_number: 1023,
                        },
                    ],
                },
                NordigLogicalChannelV2ChannelList {
                    channel_list_id: 2,
                    channel_list_name: vec![],
                    country_code: LangCode([b'D', b'E', b'U']),
                    services: vec![NordigLogicalChannelV2Service {
                        service_id: 3,
                        visible_service_flag: true,
                        logical_channel_number: 3,
                    }],
                },
            ],
        };
        let mut buf = vec![0u8; d.serialized_len()];
        d.serialize_into(&mut buf).unwrap();
        let re = NordigLogicalChannelV2::parse(&buf).unwrap();
        assert_eq!(d, re);
    }

    #[test]
    fn v2_serialize_round_trip_byte_identity() {
        let mut bytes = vec![TAG_V2, 0];
        bytes.push(0x01);
        bytes.push(0);
        bytes.extend_from_slice(b"GBR");
        bytes.push(4);
        bytes.extend_from_slice(&[0x00, 0x01, 0xFC, 0x05]);
        bytes[1] = (bytes.len() - 2) as u8;

        let d = NordigLogicalChannelV2::parse(&bytes).unwrap();
        let mut buf = vec![0u8; d.serialized_len()];
        d.serialize_into(&mut buf).unwrap();
        assert_eq!(buf, bytes);
    }
}
