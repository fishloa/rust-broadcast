//! IP/MAC Notification Table — ETSI EN 301 192 v1.7.1 §8.4.
//!
//! INT is referenced by a `data_broadcast_id_descriptor` (data_broadcast_id 0x000B)
//! in the PMT ES_info loop; there is no fixed PID.  table_id is 0x4C.
//!
//! The target/operational descriptor-loop pairs in the body loop are unfolded
//! into [`IntLoopEntry`] instances (Tables 13/17/18, §8.4.4.1).

use crate::descriptors::DescriptorLoop;
use crate::error::{Error, Result};
use alloc::vec::Vec;
use broadcast_common::{Parse, Serialize};

/// `table_id` for IP/MAC Notification Table.
pub const TABLE_ID: u8 = 0x4C;

// No fixed PID: this table's PID is signalled (PMT / descriptors). A `PID`
// constant of `0x0000` used to sit here, which is the real PAT PID, so a
// caller filtering on it would pick up PAT traffic (audit r02-W15/O12).

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
    pub const fn to_u8(self) -> u8 {
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
broadcast_common::impl_spec_display!(IntActionType, DvbReserved);

const OUTER_HEADER_LEN: usize = 3;
const INT_FIXED_LEN: usize = 9;
const LOOP_LEN_FIELD: usize = 2;
use super::CRC_LEN;
const MIN_SECTION_LEN: usize = OUTER_HEADER_LEN + INT_FIXED_LEN + LOOP_LEN_FIELD + CRC_LEN;

const OFF_ACTION_TYPE: usize = 3;
const OFF_PLATFORM_ID_HASH: usize = 4;
const OFF_VERSION_BYTE: usize = 5;
const OFF_SECTION_NUMBER: usize = 6;
const OFF_LAST_SECTION_NUMBER: usize = 7;
const OFF_PLATFORM_ID: usize = 8;
const OFF_PROCESSING_ORDER: usize = 11;
const OFF_PLATFORM_DESC_LEN: usize = 12;

/// A target/operational descriptor-loop pair in the INT body loop
/// (Tables 17/18, §8.4.4.1).
///
/// Shared with the UNT platform loop (r02-W22); kept as a local alias so
/// each spec's name remains discoverable from its own table module.
pub type IntLoopEntry<'a> = crate::tables::TargetOperationalLoop<'a>;

/// IP/MAC Notification Table (INT), ETSI EN 301 192 v1.7.1 §8.4, Table 13.
///
/// The `loops` field is unfolded into typed [`IntLoopEntry`] instances
/// (target + operational descriptor-loop pairs).
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[cfg_attr(feature = "yoke", derive(yoke::Yokeable))]
#[non_exhaustive]
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
    /// Header bit fields (`reserved_future_use`, `reserved(2)` x2; default
    /// [`SectionBits::dvb`](super::SectionBits::dvb)).
    #[cfg_attr(feature = "serde", serde(skip))]
    pub bits: super::SectionBits,
    /// `reserved(4)` before `platform_descriptor_loop_length` (`1111`).
    #[cfg_attr(feature = "serde", serde(skip))]
    pub reserved_platform_descriptor_loop_length: u8,
}

impl<'a> IntSection<'a> {
    /// Construct an INT section with the spec `reserved` fill.
    #[must_use]
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        action_type: IntActionType,
        platform_id_hash: u8,
        version_number: u8,
        current_next_indicator: bool,
        section_number: u8,
        last_section_number: u8,
        platform_id: u32,
        processing_order: u8,
        platform_descriptors: DescriptorLoop<'a>,
        loops: Vec<IntLoopEntry<'a>>,
    ) -> Self {
        Self {
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
            bits: super::SectionBits::dvb(),
            reserved_platform_descriptor_loop_length: super::RESERVED_4,
        }
    }
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

        let section_length = super::section_length_of(bytes);
        let total = super::check_section_length(
            bytes.len(),
            OUTER_HEADER_LEN,
            section_length,
            MIN_SECTION_LEN,
        )?;

        let action_type = IntActionType::from_u8(bytes[OFF_ACTION_TYPE]);
        let platform_id_hash = bytes[OFF_PLATFORM_ID_HASH];
        let version_byte = bytes[OFF_VERSION_BYTE];
        let version_number = super::version_number_of(version_byte);
        let current_next_indicator = super::current_next_of(version_byte);
        let section_number = bytes[OFF_SECTION_NUMBER];
        let last_section_number = bytes[OFF_LAST_SECTION_NUMBER];
        let platform_id = ((bytes[OFF_PLATFORM_ID] as u32) << 16)
            | ((bytes[OFF_PLATFORM_ID + 1] as u32) << 8)
            | bytes[OFF_PLATFORM_ID + 2] as u32;
        let processing_order = bytes[OFF_PROCESSING_ORDER];

        let plat_desc_len = super::desc_loop_len_of(
            bytes[OFF_PLATFORM_DESC_LEN],
            bytes[OFF_PLATFORM_DESC_LEN + 1],
        );
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
            let target_len = super::desc_loop_len_of(bytes[pos], bytes[pos + 1]);
            let target_start = pos + LOOP_LEN_FIELD;
            let target_end = target_start + target_len;
            if target_end > payload_end {
                return Err(Error::SectionLengthOverflow {
                    declared: target_len,
                    available: payload_end.saturating_sub(target_start),
                });
            }
            let reserved_target_length = super::len_reserved_of(bytes[pos]);
            let target_descriptors = DescriptorLoop::new(&bytes[target_start..target_end]);
            pos = target_end;

            if pos + LOOP_LEN_FIELD > payload_end {
                return Err(Error::BufferTooShort {
                    need: pos + LOOP_LEN_FIELD,
                    have: payload_end,
                    what: "IntSection operational_descriptor_loop length",
                });
            }
            let op_len = super::desc_loop_len_of(bytes[pos], bytes[pos + 1]);
            let op_start = pos + LOOP_LEN_FIELD;
            let op_end = op_start + op_len;
            if op_end > payload_end {
                return Err(Error::SectionLengthOverflow {
                    declared: op_len,
                    available: payload_end.saturating_sub(op_start),
                });
            }
            let reserved_operational_length = super::len_reserved_of(bytes[pos]);
            let operational_descriptors = DescriptorLoop::new(&bytes[op_start..op_end]);
            pos = op_end;

            loops.push(IntLoopEntry {
                target_descriptors,
                operational_descriptors,
                reserved_target_length,
                reserved_operational_length,
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
            bits: super::SectionBits::parse(bytes[1], Some(bytes[OFF_VERSION_BYTE])),
            reserved_platform_descriptor_loop_length: super::len_reserved_of(
                bytes[OFF_PLATFORM_DESC_LEN],
            ),
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
                .map(crate::tables::TargetOperationalLoop::serialized_len)
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

        // 24-bit field: the top byte of a wider value used to be dropped
        // silently (audit r03-W3 class).
        broadcast_common::len::fit_bits(u64::from(self.platform_id), 24, "platform_id")?;
        buf[0] = TABLE_ID;
        buf[1] = self.bits.b1(true);
        super::write_section_length(buf, len - OUTER_HEADER_LEN)?;

        buf[OFF_ACTION_TYPE] = self.action_type.to_u8();
        buf[OFF_PLATFORM_ID_HASH] = self.platform_id_hash;
        buf[OFF_VERSION_BYTE] = self
            .bits
            .b5(self.version_number, self.current_next_indicator);
        buf[OFF_SECTION_NUMBER] = self.section_number;
        buf[OFF_LAST_SECTION_NUMBER] = self.last_section_number;
        buf[OFF_PLATFORM_ID] = ((self.platform_id >> 16) & 0xFF) as u8;
        buf[OFF_PLATFORM_ID + 1] = ((self.platform_id >> 8) & 0xFF) as u8;
        buf[OFF_PLATFORM_ID + 2] = (self.platform_id & 0xFF) as u8;
        buf[OFF_PROCESSING_ORDER] = self.processing_order;

        let pdl = broadcast_common::len::fit_bits(
            self.platform_descriptors.len() as u64,
            12,
            "platform_descriptors_loop_length",
        )?;
        super::write_desc_loop_len_r(
            &mut buf[OFF_PLATFORM_DESC_LEN..OFF_PLATFORM_DESC_LEN + 2],
            pdl as usize,
            self.reserved_platform_descriptor_loop_length,
        )?;

        let plat_start = OFF_PLATFORM_DESC_LEN + LOOP_LEN_FIELD;
        let plat_end = plat_start + self.platform_descriptors.len();
        buf[plat_start..plat_end].copy_from_slice(self.platform_descriptors.raw());

        let mut pos = plat_end;
        for entry in &self.loops {
            let tl = broadcast_common::len::fit_bits(
                entry.target_descriptors.len() as u64,
                12,
                "target_descriptor_loop_length",
            )?;
            super::write_desc_loop_len_r(
                &mut buf[pos..pos + 2],
                tl as usize,
                entry.reserved_target_length,
            )?;
            pos += LOOP_LEN_FIELD;
            buf[pos..pos + entry.target_descriptors.len()]
                .copy_from_slice(entry.target_descriptors.raw());
            pos += entry.target_descriptors.len();

            let ol = broadcast_common::len::fit_bits(
                entry.operational_descriptors.len() as u64,
                12,
                "operational_descriptor_loop_length",
            )?;
            super::write_desc_loop_len_r(
                &mut buf[pos..pos + 2],
                ol as usize,
                entry.reserved_operational_length,
            )?;
            pos += LOOP_LEN_FIELD;
            buf[pos..pos + entry.operational_descriptors.len()]
                .copy_from_slice(entry.operational_descriptors.raw());
            pos += entry.operational_descriptors.len();
        }

        let crc_pos = len - CRC_LEN;
        super::write_crc(buf, crc_pos);
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

    /// r03-W3 class: a 24-bit `platform_id` wider than its field lost its top
    /// byte silently.
    #[test]
    fn serialize_rejects_platform_id_wider_than_24_bits() {
        let mk = |platform_id| {
            IntSection::new(
                IntActionType::IpMacStreamLocation,
                0,
                1,
                true,
                0,
                0,
                platform_id,
                0,
                DescriptorLoop::new(&[]),
                vec![],
            )
        };
        let wide = mk(0x0100_0000);
        let mut buf = vec![0u8; wide.serialized_len()];
        assert!(matches!(
            wide.serialize_into(&mut buf),
            Err(Error::FieldOverflow(_))
        ));
        let ok = mk(0x00FF_FFFF);
        let mut buf = vec![0u8; ok.serialized_len()];
        ok.serialize_into(&mut buf).unwrap();
        assert_eq!(IntSection::parse(&buf).unwrap().platform_id, 0x00FF_FFFF);
    }

    #[test]
    fn parse_happy_path_no_loops() {
        let plat_desc: &[u8] = &[0x81, 0x02, 0xAB, 0xCD];
        let int = IntSection::new(
            IntActionType::IpMacStreamLocation,
            0x12 ^ 0x34,
            3,
            true,
            0,
            0,
            0x00_12_34,
            0x00,
            DescriptorLoop::new(plat_desc),
            Vec::new(),
        );
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
        let int = IntSection::new(
            IntActionType::IpMacStreamLocation,
            0x56,
            5,
            false,
            1,
            1,
            0x00_56_78,
            0x01,
            DescriptorLoop::new(&[]),
            vec![
                IntLoopEntry::new(
                    DescriptorLoop::new(target_desc),
                    DescriptorLoop::new(op_desc),
                ),
                IntLoopEntry::new(DescriptorLoop::new(&[]), DescriptorLoop::new(&[])),
            ],
        );
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
        let int = IntSection::new(
            IntActionType::IpMacStreamLocation,
            0xAB,
            15,
            true,
            2,
            3,
            0x00_AB_CD,
            0x00,
            DescriptorLoop::new(plat_desc),
            vec![IntLoopEntry::new(
                DescriptorLoop::new(&[]),
                DescriptorLoop::new(&[]),
            )],
        );
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
        let int = IntSection::new(
            IntActionType::IpMacStreamLocation,
            0x00,
            0,
            true,
            0,
            0,
            0,
            0,
            DescriptorLoop::new(&[]),
            Vec::new(),
        );
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
        let int = IntSection::new(
            IntActionType::IpMacStreamLocation,
            0x00,
            0,
            true,
            0,
            0,
            0,
            0,
            DescriptorLoop::new(&[]),
            Vec::new(),
        );
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

    /// section_length is 12 bits (max 4095 = INT_FIXED_LEN(9) +
    /// LOOP_LEN_FIELD(2) + platform_descriptors + CRC(4), so 4080 is the
    /// largest platform_descriptors loop that fits with no target/
    /// operational loop entries).
    #[test]
    fn serialize_rejects_platform_descriptors_exceeding_section_length() {
        let desc = vec![0xAAu8; 4081];
        let int = IntSection::new(
            IntActionType::IpMacStreamLocation,
            0,
            0,
            true,
            0,
            0,
            0,
            0,
            DescriptorLoop::new(&desc),
            vec![],
        );
        let mut buf = vec![0u8; int.serialized_len()];
        assert!(matches!(
            int.serialize_into(&mut buf).unwrap_err(),
            Error::FieldOverflow(_)
        ));
    }

    #[test]
    fn serialize_accepts_platform_descriptors_at_section_length_boundary() {
        let desc = vec![0xAAu8; 4080];
        let int = IntSection::new(
            IntActionType::IpMacStreamLocation,
            0,
            0,
            true,
            0,
            0,
            0,
            0,
            DescriptorLoop::new(&desc),
            vec![],
        );
        let mut buf = vec![0u8; int.serialized_len()];
        int.serialize_into(&mut buf).expect("boundary fits");
        let reparsed = IntSection::parse(&buf).expect("reparse");
        assert_eq!(int, reparsed);
    }

    #[test]
    fn platform_id_24bit_boundary() {
        let int = IntSection::new(
            IntActionType::IpMacStreamLocation,
            0xFF,
            0,
            true,
            0,
            0,
            0x00FF_FFFF,
            0x00,
            DescriptorLoop::new(&[]),
            Vec::new(),
        );
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
        let crc = broadcast_common::crc32_mpeg2::compute(&bytes);
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
