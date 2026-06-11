//! IP/MAC Notification Table — ETSI EN 301 192 v1.7.1 §8.4.
//!
//! INT is referenced by a `data_broadcast_id_descriptor` (data_broadcast_id 0x000B)
//! in the PMT ES_info loop; there is no fixed PID.  table_id is 0x4C.
//!
//! The target/operational descriptor-loop pairs in the body loop are unfolded
//! into [`IntLoopEntry`] instances (Tables 13/17/18, §8.4.4.1).

use crate::descriptors::DescriptorLoop;
use crate::error::{Error, Result};
use dvb_common::{Parse, Serialize};

/// `table_id` for IP/MAC Notification Table.
pub const TABLE_ID: u8 = 0x4C;

/// PID on which INT is carried.
///
/// INT does not have a fixed PID.  It is discovered through a
/// `data_broadcast_id_descriptor` (data_broadcast_id 0x000B) inside the PMT
/// ES_info loop.  This constant is therefore 0x0000 (unknown/variable).
pub const PID: u16 = 0x0000;

/// Action type coding — ETSI EN 301 192 §8.4.4.1 Table 14.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub enum IntActionType {
    /// 0x00 — reserved.
    Reserved,
    /// 0x01 — location of IP/MAC streams in DVB networks.
    IpMacStreamLocation,
    /// 0x02..=0xFF — reserved for future use.
    DvbReserved(u8),
}

impl IntActionType {
    #[must_use]
    /// Decode from the wire value.  Every value maps (lossless).
    pub fn from_u8(v: u8) -> Self {
        match v {
            0x00 => Self::Reserved,
            0x01 => Self::IpMacStreamLocation,
            _ => Self::DvbReserved(v),
        }
    }

    #[must_use]
    /// Encode to the wire value.  Inverse of `from_u8` / `from_u16`.
    pub fn to_u8(self) -> u8 {
        match self {
            Self::Reserved => 0x00,
            Self::IpMacStreamLocation => 0x01,
            Self::DvbReserved(v) => v,
        }
    }

    #[must_use]
    /// Human-readable spec display name.
    pub fn name(self) -> &'static str {
        match self {
            Self::Reserved => "Reserved",
            Self::IpMacStreamLocation => "IP/MAC Stream Location",
            Self::DvbReserved(_) => "DVB Reserved",
        }
    }
}

const OUTER_HEADER_LEN: usize = 3;
const INT_FIXED_LEN: usize = 9;
const LOOP_LEN_FIELD: usize = 2;
const CRC_LEN: usize = 4;
const MIN_SECTION_LEN: usize = OUTER_HEADER_LEN + INT_FIXED_LEN + LOOP_LEN_FIELD + CRC_LEN;

const OFF_ACTION_TYPE: usize = 3;
const OFF_PLATFORM_ID_HASH: usize = 4;
const OFF_VERSION_BYTE: usize = 5;
const OFF_SECTION_NUMBER: usize = 6;
const OFF_LAST_SECTION_NUMBER: usize = 7;
const OFF_PLATFORM_ID: usize = 8;
const OFF_PROCESSING_ORDER: usize = 11;
const OFF_PLATFORM_DESC_LEN: usize = 12;

const RESERVED_NIBBLE: u8 = 0xF0;

/// A target/operational descriptor-loop pair in the INT body loop
/// (Tables 17/18, §8.4.4.1).
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct IntLoopEntry<'a> {
    /// Target descriptor loop — raw descriptor bytes (after the 12-bit length
    /// field).  Serializes as the typed descriptor sequence; `.raw()` yields the
    /// wire bytes.
    pub target_descriptors: DescriptorLoop<'a>,
    /// Operational descriptor loop — raw descriptor bytes (after the 12-bit
    /// length field).  Serializes as the typed descriptor sequence; `.raw()`
    /// yields the wire bytes.
    pub operational_descriptors: DescriptorLoop<'a>,
}

fn int_loop_entry_serialized_len(e: &IntLoopEntry) -> usize {
    LOOP_LEN_FIELD + e.target_descriptors.len() + LOOP_LEN_FIELD + e.operational_descriptors.len()
}

/// IP/MAC Notification Table (INT), ETSI EN 301 192 v1.7.1 §8.4, Table 13.
///
/// The `loops` field is unfolded into typed [`IntLoopEntry`] instances
/// (target + operational descriptor-loop pairs).
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[cfg_attr(feature = "yoke", derive(yoke::Yokeable))]
pub struct IntSection<'a> {
    /// Semantics of this INT announcement — 0x01 = stream announcement/location.
    pub action_type: IntActionType,
    /// 8-bit XOR hash over the 24-bit platform_id.
    pub platform_id_hash: u8,
    /// 5-bit version_number.
    pub version_number: u8,
    /// `current_next_indicator` bit.
    pub current_next_indicator: bool,
    /// section_number within this sub-table.
    pub section_number: u8,
    /// last_section_number in this sub-table.
    pub last_section_number: u8,
    /// 24-bit platform identifier (TS 101 162) in the low 24 bits of a u32.
    pub platform_id: u32,
    /// Processing order relative to other INT sections.
    pub processing_order: u8,
    /// The `platform_descriptor_loop` (descriptors only, not the 2-byte length
    /// field). Serializes as the typed descriptor sequence; `.raw()` yields the
    /// wire bytes.
    pub platform_descriptors: DescriptorLoop<'a>,
    /// Target/operational descriptor-loop pairs — unfolded per Tables 17/18.
    pub loops: Vec<IntLoopEntry<'a>>,
}

impl<'a> Parse<'a> for IntSection<'a> {
    type Error = crate::error::Error;

    fn parse(bytes: &'a [u8]) -> Result<Self> {
        if bytes.len() < MIN_SECTION_LEN {
            return Err(Error::BufferTooShort {
                need: MIN_SECTION_LEN,
                have: bytes.len(),
                what: "IntSection",
            });
        }
        if bytes[0] != TABLE_ID {
            return Err(Error::UnexpectedTableId {
                table_id: bytes[0],
                what: "IntSection",
                expected: &[TABLE_ID],
            });
        }

        let section_length = (((bytes[1] & 0x0F) as usize) << 8) | bytes[2] as usize;
        let total = super::check_section_length(
            bytes.len(),
            OUTER_HEADER_LEN,
            section_length,
            MIN_SECTION_LEN,
        )?;

        let action_type = IntActionType::from_u8(bytes[OFF_ACTION_TYPE]);
        let platform_id_hash = bytes[OFF_PLATFORM_ID_HASH];
        let version_byte = bytes[OFF_VERSION_BYTE];
        let version_number = (version_byte >> 1) & 0x1F;
        let current_next_indicator = (version_byte & 0x01) != 0;
        let section_number = bytes[OFF_SECTION_NUMBER];
        let last_section_number = bytes[OFF_LAST_SECTION_NUMBER];
        let platform_id = ((bytes[OFF_PLATFORM_ID] as u32) << 16)
            | ((bytes[OFF_PLATFORM_ID + 1] as u32) << 8)
            | bytes[OFF_PLATFORM_ID + 2] as u32;
        let processing_order = bytes[OFF_PROCESSING_ORDER];

        let plat_desc_len = (((bytes[OFF_PLATFORM_DESC_LEN] & 0x0F) as usize) << 8)
            | bytes[OFF_PLATFORM_DESC_LEN + 1] as usize;
        let plat_desc_start = OFF_PLATFORM_DESC_LEN + LOOP_LEN_FIELD;
        let plat_desc_end = plat_desc_start + plat_desc_len;
        if plat_desc_end > total - CRC_LEN {
            return Err(Error::SectionLengthOverflow {
                declared: plat_desc_len,
                available: (total - CRC_LEN).saturating_sub(plat_desc_start),
            });
        }
        let platform_descriptors = DescriptorLoop::new(&bytes[plat_desc_start..plat_desc_end]);

        let payload_end = total - CRC_LEN;
        let mut pos = plat_desc_end;
        let mut loops = Vec::new();
        while pos < payload_end {
            if pos + LOOP_LEN_FIELD > payload_end {
                return Err(Error::BufferTooShort {
                    need: pos + LOOP_LEN_FIELD,
                    have: payload_end,
                    what: "IntSection target_descriptor_loop length",
                });
            }
            let target_len = (((bytes[pos] & 0x0F) as usize) << 8) | bytes[pos + 1] as usize;
            let target_start = pos + LOOP_LEN_FIELD;
            let target_end = target_start + target_len;
            if target_end > payload_end {
                return Err(Error::SectionLengthOverflow {
                    declared: target_len,
                    available: payload_end.saturating_sub(target_start),
                });
            }
            let target_descriptors = DescriptorLoop::new(&bytes[target_start..target_end]);
            pos = target_end;

            if pos + LOOP_LEN_FIELD > payload_end {
                return Err(Error::BufferTooShort {
                    need: pos + LOOP_LEN_FIELD,
                    have: payload_end,
                    what: "IntSection operational_descriptor_loop length",
                });
            }
            let op_len = (((bytes[pos] & 0x0F) as usize) << 8) | bytes[pos + 1] as usize;
            let op_start = pos + LOOP_LEN_FIELD;
            let op_end = op_start + op_len;
            if op_end > payload_end {
                return Err(Error::SectionLengthOverflow {
                    declared: op_len,
                    available: payload_end.saturating_sub(op_start),
                });
            }
            let operational_descriptors = DescriptorLoop::new(&bytes[op_start..op_end]);
            pos = op_end;

            loops.push(IntLoopEntry {
                target_descriptors,
                operational_descriptors,
            });
        }

        Ok(IntSection {
            action_type,
            platform_id_hash,
            version_number,
            current_next_indicator,
            section_number,
            last_section_number,
            platform_id,
            processing_order,
            platform_descriptors,
            loops,
        })
    }
}

impl Serialize for IntSection<'_> {
    type Error = crate::error::Error;

    fn serialized_len(&self) -> usize {
        OUTER_HEADER_LEN
            + INT_FIXED_LEN
            + LOOP_LEN_FIELD
            + self.platform_descriptors.len()
            + self
                .loops
                .iter()
                .map(int_loop_entry_serialized_len)
                .sum::<usize>()
            + CRC_LEN
    }

    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        let len = self.serialized_len();
        if buf.len() < len {
            return Err(Error::OutputBufferTooSmall {
                need: len,
                have: buf.len(),
            });
        }

        let section_length = (len - OUTER_HEADER_LEN) as u16;
        buf[0] = TABLE_ID;
        buf[1] = super::SECTION_B1_FLAGS_DVB | ((section_length >> 8) as u8 & 0x0F);
        buf[2] = (section_length & 0xFF) as u8;

        buf[OFF_ACTION_TYPE] = self.action_type.to_u8();
        buf[OFF_PLATFORM_ID_HASH] = self.platform_id_hash;
        buf[OFF_VERSION_BYTE] =
            0xC0 | ((self.version_number & 0x1F) << 1) | u8::from(self.current_next_indicator);
        buf[OFF_SECTION_NUMBER] = self.section_number;
        buf[OFF_LAST_SECTION_NUMBER] = self.last_section_number;
        buf[OFF_PLATFORM_ID] = ((self.platform_id >> 16) & 0xFF) as u8;
        buf[OFF_PLATFORM_ID + 1] = ((self.platform_id >> 8) & 0xFF) as u8;
        buf[OFF_PLATFORM_ID + 2] = (self.platform_id & 0xFF) as u8;
        buf[OFF_PROCESSING_ORDER] = self.processing_order;

        let pdl = self.platform_descriptors.len() as u16;
        buf[OFF_PLATFORM_DESC_LEN] = RESERVED_NIBBLE | ((pdl >> 8) as u8 & 0x0F);
        buf[OFF_PLATFORM_DESC_LEN + 1] = (pdl & 0xFF) as u8;

        let plat_start = OFF_PLATFORM_DESC_LEN + LOOP_LEN_FIELD;
        let plat_end = plat_start + self.platform_descriptors.len();
        buf[plat_start..plat_end].copy_from_slice(self.platform_descriptors.raw());

        let mut pos = plat_end;
        for entry in &self.loops {
            let tl = entry.target_descriptors.len() as u16;
            buf[pos] = RESERVED_NIBBLE | ((tl >> 8) as u8 & 0x0F);
            buf[pos + 1] = (tl & 0xFF) as u8;
            pos += LOOP_LEN_FIELD;
            buf[pos..pos + entry.target_descriptors.len()]
                .copy_from_slice(entry.target_descriptors.raw());
            pos += entry.target_descriptors.len();

            let ol = entry.operational_descriptors.len() as u16;
            buf[pos] = RESERVED_NIBBLE | ((ol >> 8) as u8 & 0x0F);
            buf[pos + 1] = (ol & 0xFF) as u8;
            pos += LOOP_LEN_FIELD;
            buf[pos..pos + entry.operational_descriptors.len()]
                .copy_from_slice(entry.operational_descriptors.raw());
            pos += entry.operational_descriptors.len();
        }

        let crc_pos = len - CRC_LEN;
        let crc = dvb_common::crc32_mpeg2::compute(&buf[..crc_pos]);
        buf[crc_pos..len].copy_from_slice(&crc.to_be_bytes());
        Ok(len)
    }
}
impl<'a> crate::traits::TableDef<'a> for IntSection<'a> {
    const TABLE_ID_RANGES: &'static [(u8, u8)] = &[(TABLE_ID, TABLE_ID)];
    const NAME: &'static str = "IP_MAC_NOTIFICATION";
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_happy_path_no_loops() {
        let plat_desc: &[u8] = &[0x81, 0x02, 0xAB, 0xCD];
        let int = IntSection {
            action_type: IntActionType::IpMacStreamLocation,
            platform_id_hash: 0x12 ^ 0x34,
            version_number: 3,
            current_next_indicator: true,
            section_number: 0,
            last_section_number: 0,
            platform_id: 0x00_12_34,
            processing_order: 0x00,
            platform_descriptors: DescriptorLoop::new(plat_desc),
            loops: Vec::new(),
        };
        let mut buf = vec![0u8; int.serialized_len()];
        int.serialize_into(&mut buf).unwrap();
        let parsed = IntSection::parse(&buf).unwrap();
        assert_eq!(parsed.action_type, IntActionType::IpMacStreamLocation);
        assert_eq!(parsed.platform_id, 0x00_12_34);
        assert_eq!(parsed.platform_descriptors.raw(), plat_desc);
        assert!(parsed.loops.is_empty());
    }

    #[test]
    fn parse_happy_path_with_loops() {
        let target_desc: &[u8] = &[0x09, 0x01, 0xAA];
        let op_desc: &[u8] = &[0x0A, 0x01, 0xBB];
        let int = IntSection {
            action_type: IntActionType::IpMacStreamLocation,
            platform_id_hash: 0x56,
            version_number: 5,
            current_next_indicator: false,
            section_number: 1,
            last_section_number: 1,
            platform_id: 0x00_56_78,
            processing_order: 0x01,
            platform_descriptors: DescriptorLoop::new(&[]),
            loops: vec![
                IntLoopEntry {
                    target_descriptors: DescriptorLoop::new(target_desc),
                    operational_descriptors: DescriptorLoop::new(op_desc),
                },
                IntLoopEntry {
                    target_descriptors: DescriptorLoop::new(&[]),
                    operational_descriptors: DescriptorLoop::new(&[]),
                },
            ],
        };
        let mut buf = vec![0u8; int.serialized_len()];
        int.serialize_into(&mut buf).unwrap();
        let parsed = IntSection::parse(&buf).unwrap();
        assert_eq!(parsed.loops.len(), 2);
        assert_eq!(parsed.loops[0].target_descriptors.raw(), target_desc);
        assert_eq!(parsed.loops[0].operational_descriptors.raw(), op_desc);
        assert_eq!(parsed.loops[1].target_descriptors.len(), 0);
        assert_eq!(parsed.loops[1].operational_descriptors.len(), 0);
    }

    #[test]
    fn byte_exact_round_trip() {
        let plat_desc: &[u8] = &[0x7C, 0x04, 0x01, 0x02, 0x03, 0x04];
        let int = IntSection {
            action_type: IntActionType::IpMacStreamLocation,
            platform_id_hash: 0xAB,
            version_number: 15,
            current_next_indicator: true,
            section_number: 2,
            last_section_number: 3,
            platform_id: 0x00_AB_CD,
            processing_order: 0x00,
            platform_descriptors: DescriptorLoop::new(plat_desc),
            loops: vec![IntLoopEntry {
                target_descriptors: DescriptorLoop::new(&[]),
                operational_descriptors: DescriptorLoop::new(&[]),
            }],
        };
        let mut buf = vec![0u8; int.serialized_len()];
        int.serialize_into(&mut buf).unwrap();
        let re = IntSection::parse(&buf).unwrap();
        let mut buf2 = vec![0u8; re.serialized_len()];
        re.serialize_into(&mut buf2).unwrap();
        assert_eq!(buf, buf2, "byte-exact re-serialize");
        assert_eq!(re, int);
    }

    #[test]
    fn parse_rejects_wrong_table_id() {
        let int = IntSection {
            action_type: IntActionType::IpMacStreamLocation,
            platform_id_hash: 0x00,
            version_number: 0,
            current_next_indicator: true,
            section_number: 0,
            last_section_number: 0,
            platform_id: 0,
            processing_order: 0,
            platform_descriptors: DescriptorLoop::new(&[]),
            loops: Vec::new(),
        };
        let mut buf = vec![0u8; int.serialized_len()];
        int.serialize_into(&mut buf).unwrap();
        buf[0] = 0x4B;
        assert!(matches!(
            IntSection::parse(&buf).unwrap_err(),
            Error::UnexpectedTableId { table_id: 0x4B, .. }
        ));
    }

    #[test]
    fn parse_rejects_buffer_too_short() {
        assert!(matches!(
            IntSection::parse(&[TABLE_ID, 0xF0]).unwrap_err(),
            Error::BufferTooShort {
                what: "IntSection",
                ..
            }
        ));
    }

    #[test]
    fn serialize_rejects_too_small_output_buffer() {
        let int = IntSection {
            action_type: IntActionType::IpMacStreamLocation,
            platform_id_hash: 0x00,
            version_number: 0,
            current_next_indicator: true,
            section_number: 0,
            last_section_number: 0,
            platform_id: 0,
            processing_order: 0,
            platform_descriptors: DescriptorLoop::new(&[]),
            loops: Vec::new(),
        };
        let mut buf = vec![0u8; 2];
        assert!(matches!(
            int.serialize_into(&mut buf).unwrap_err(),
            Error::OutputBufferTooSmall { .. }
        ));
    }

    #[test]
    fn parse_rejects_zero_section_length() {
        let mut buf = vec![0u8; 64];
        buf[0] = TABLE_ID;
        buf[1] = 0xF0;
        buf[2] = 0x00;
        for b in &mut buf[3..] {
            *b = 0xFF;
        }
        assert!(matches!(
            IntSection::parse(&buf).unwrap_err(),
            Error::SectionLengthOverflow { .. }
        ));
    }

    #[test]
    fn platform_id_24bit_boundary() {
        let int = IntSection {
            action_type: IntActionType::IpMacStreamLocation,
            platform_id_hash: 0xFF,
            version_number: 0,
            current_next_indicator: true,
            section_number: 0,
            last_section_number: 0,
            platform_id: 0x00FF_FFFF,
            processing_order: 0x00,
            platform_descriptors: DescriptorLoop::new(&[]),
            loops: Vec::new(),
        };
        let mut buf = vec![0u8; int.serialized_len()];
        int.serialize_into(&mut buf).unwrap();
        let parsed = IntSection::parse(&buf).unwrap();
        assert_eq!(parsed.platform_id, 0x00FF_FFFF);
    }

    #[test]
    fn parse_handwritten_int_no_loops() {
        let mut bytes: Vec<u8> = vec![
            0x4C, 0xF0, 0x0F, 0x01, 0x00, 0xC7, 0x00, 0x00, 0x00, 0x00, 0x01, 0x00, 0xF0, 0x00,
        ];
        let crc = dvb_common::crc32_mpeg2::compute(&bytes);
        bytes.extend_from_slice(&crc.to_be_bytes());
        let int = IntSection::parse(&bytes).unwrap();
        assert_eq!(int.action_type, IntActionType::IpMacStreamLocation);
        assert_eq!(int.platform_id, 0x000001);
        assert_eq!(int.version_number, 3);
        assert!(int.current_next_indicator);
        assert!(int.loops.is_empty());
    }

    #[test]
    fn action_type_full_range_round_trip() {
        for byte in 0u8..=0xFF {
            let at = IntActionType::from_u8(byte);
            assert_eq!(
                at.to_u8(),
                byte,
                "IntActionType round-trip failed for {byte:#04x}"
            );
        }
    }
}
