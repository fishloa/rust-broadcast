//! Update Notification Table — ETSI TS 102 006 v1.4.1 §9.4.
//!
//! The UNT delivers software-update instructions for DVB receivers. It is
//! carried on a PID that is **signalled** — there is no fixed PID. The PMT
//! ES_info loop for the update data carousel contains a
//! `data_broadcast_id_descriptor` (tag 0x66) with `data_broadcast_id = 0x000A`;
//! the associated elementary PID is the one carrying UNT sections.
//!
//! The platform loop is unfolded into [`UntPlatform`] entries (Tables 11/15/17/18,
//! §9.4.2.2–9.4.2.4). The `compatibilityDescriptor()` block is typed as
//! [`CompatibilityDescriptor`] (ISO/IEC 13818-6 groupInfo form — NOT a standard
//! SI tag/length descriptor).

use crate::compatibility::CompatibilityDescriptor;
use crate::descriptors::DescriptorLoop;
use crate::error::{Error, Result};
use dvb_common::{Parse, Serialize};

/// `table_id` for the Update Notification Table.
pub const TABLE_ID: u8 = 0x4B;

/// Well-known PID for UNT: **none** — the UNT has no fixed PID.
pub const PID: u16 = 0x0000;

/// Action type coding — ETSI TS 102 006 §9.4.2 Table 12.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub enum UntActionType {
    /// 0x00 — reserved.
    Reserved,
    /// 0x01 — System Software Update.
    SystemSoftwareUpdate,
    /// 0x02..=0x7F — DVB reserved for future use.
    DvbReserved(u8),
    /// 0x80..=0xFF — user defined.
    UserDefined(u8),
}

impl UntActionType {
    #[must_use]
    /// Decode from the wire value.  Every value maps (lossless).
    pub fn from_u8(v: u8) -> Self {
        match v {
            0x00 => Self::Reserved,
            0x01 => Self::SystemSoftwareUpdate,
            v if v < 0x80 => Self::DvbReserved(v),
            _ => Self::UserDefined(v),
        }
    }

    #[must_use]
    /// Encode to the wire value.  Inverse of `from_u8` / `from_u16`.
    pub fn to_u8(self) -> u8 {
        match self {
            Self::Reserved => 0x00,
            Self::SystemSoftwareUpdate => 0x01,
            Self::DvbReserved(v) | Self::UserDefined(v) => v,
        }
    }

    #[must_use]
    /// Human-readable spec display name.
    pub fn name(self) -> &'static str {
        match self {
            Self::Reserved => "Reserved",
            Self::SystemSoftwareUpdate => "System Software Update",
            Self::DvbReserved(_) => "DVB Reserved",
            Self::UserDefined(_) => "User Defined",
        }
    }
}

const HEADER_LEN: usize = 3;
const FIXED_BODY_LEN: usize = 9;
const COMMON_DESC_LEN_FIELD: usize = 2;
const CRC_LEN: usize = 4;
const MIN_SECTION_LEN: usize = HEADER_LEN + FIXED_BODY_LEN + COMMON_DESC_LEN_FIELD + CRC_LEN;

const OFFSET_ACTION_TYPE: usize = HEADER_LEN;
const OFFSET_OUI_HASH: usize = HEADER_LEN + 1;
const OFFSET_FLAGS: usize = HEADER_LEN + 2;
const OFFSET_SECTION_NUMBER: usize = HEADER_LEN + 3;
const OFFSET_LAST_SECTION_NUMBER: usize = HEADER_LEN + 4;
const OFFSET_OUI: usize = HEADER_LEN + 5;
const OFFSET_PROCESSING_ORDER: usize = HEADER_LEN + 8;
const OFFSET_COMMON_DESC_LEN: usize = HEADER_LEN + FIXED_BODY_LEN;

const VERSION_NUMBER_MASK: u8 = 0x3E;
const VERSION_NUMBER_SHIFT: u8 = 1;
const CURRENT_NEXT_MASK: u8 = 0x01;
const LENGTH_HIGH_NIBBLE_MASK: u8 = 0x0F;
const FLAGS_RESERVED_BITS: u8 = 0xC0;
const RESERVED_NIBBLE: u8 = 0xF0;

const PLATFORM_LOOP_LEN_FIELD: usize = 2;
const DESC_LOOP_LEN_FIELD: usize = 2;

/// A single platform entry in the UNT platform loop
/// (Tables 11/15/17/18, §9.4.2.2–9.4.2.4).
///
/// Each entry consists of a `compatibilityDescriptor()` block (typed as
/// [`CompatibilityDescriptor`] — ISO/IEC 13818-6 groupInfo structure, not a
/// standard SI descriptor), followed by a `platform_loop_length` field and
/// target/operational descriptor-loop pairs.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct UntPlatform<'a> {
    /// `compatibilityDescriptor()` — TS 102 006 Table 15 / ISO/IEC 13818-6.
    pub compatibility_descriptor: CompatibilityDescriptor<'a>,
    /// N pairs of (target_descriptor_loop, operational_descriptor_loop) per
    /// TS 102 006 Table 11.
    pub target_operational_pairs: Vec<(DescriptorLoop<'a>, DescriptorLoop<'a>)>,
}

fn unt_platform_serialized_len(p: &UntPlatform) -> usize {
    p.compatibility_descriptor.serialized_len()
        + PLATFORM_LOOP_LEN_FIELD
        + p.target_operational_pairs
            .iter()
            .map(|(t, o)| DESC_LOOP_LEN_FIELD + t.len() + DESC_LOOP_LEN_FIELD + o.len())
            .sum::<usize>()
}

/// Update Notification Table (UNT), ETSI TS 102 006 v1.4.1 §9.4, Table 11.
///
/// The platform loop is unfolded into typed [`UntPlatform`] entries.
/// The `compatibilityDescriptor()` within each entry is typed as
/// [`CompatibilityDescriptor`] (ISO/IEC 13818-6 groupInfo form — not a
/// standard SI tag/length descriptor).
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[cfg_attr(feature = "yoke", derive(yoke::Yokeable))]
pub struct UntSection<'a> {
    /// Action type (Table 12): 0x01 = System Software Update, 0x80–0xFF user defined.
    pub action_type: UntActionType,
    /// OUI hash: XOR of the three OUI bytes.
    pub oui_hash: u8,
    /// 5-bit version_number of this sub-table.
    pub version_number: u8,
    /// `current_next_indicator`: `true` means currently applicable.
    pub current_next_indicator: bool,
    /// Index of this section within the sub-table.
    pub section_number: u8,
    /// Index of the last section in the sub-table.
    pub last_section_number: u8,
    /// 24-bit IEEE OUI (low 24 bits of u32).
    pub oui: u32,
    /// Processing order (Table 13).
    pub processing_order: u8,
    /// Body of `common_descriptor_loop()` — the bytes AFTER the 12-bit length
    /// field.
    pub common_descriptors: DescriptorLoop<'a>,
    /// Platform entries — unfolded per §9.4.2.2–9.4.2.4.
    pub platforms: Vec<UntPlatform<'a>>,
}

impl<'a> Parse<'a> for UntSection<'a> {
    type Error = crate::error::Error;

    fn parse(bytes: &'a [u8]) -> Result<Self> {
        if bytes.len() < MIN_SECTION_LEN {
            return Err(Error::BufferTooShort {
                need: MIN_SECTION_LEN,
                have: bytes.len(),
                what: "UntSection",
            });
        }
        if bytes[0] != TABLE_ID {
            return Err(Error::UnexpectedTableId {
                table_id: bytes[0],
                what: "UntSection",
                expected: &[TABLE_ID],
            });
        }

        let section_length =
            (((bytes[1] & LENGTH_HIGH_NIBBLE_MASK) as usize) << 8) | bytes[2] as usize;
        let total =
            super::check_section_length(bytes.len(), HEADER_LEN, section_length, MIN_SECTION_LEN)?;

        let action_type = UntActionType::from_u8(bytes[OFFSET_ACTION_TYPE]);
        let oui_hash = bytes[OFFSET_OUI_HASH];
        let flags_byte = bytes[OFFSET_FLAGS];
        let version_number = (flags_byte & VERSION_NUMBER_MASK) >> VERSION_NUMBER_SHIFT;
        let current_next_indicator = (flags_byte & CURRENT_NEXT_MASK) != 0;
        let section_number = bytes[OFFSET_SECTION_NUMBER];
        let last_section_number = bytes[OFFSET_LAST_SECTION_NUMBER];
        let oui = ((bytes[OFFSET_OUI] as u32) << 16)
            | ((bytes[OFFSET_OUI + 1] as u32) << 8)
            | (bytes[OFFSET_OUI + 2] as u32);
        let processing_order = bytes[OFFSET_PROCESSING_ORDER];

        let cdl = (((bytes[OFFSET_COMMON_DESC_LEN] & LENGTH_HIGH_NIBBLE_MASK) as usize) << 8)
            | bytes[OFFSET_COMMON_DESC_LEN + 1] as usize;
        let common_desc_start = OFFSET_COMMON_DESC_LEN + COMMON_DESC_LEN_FIELD;
        let common_desc_end = common_desc_start + cdl;
        if common_desc_end > total - CRC_LEN {
            return Err(Error::SectionLengthOverflow {
                declared: cdl,
                available: (total - CRC_LEN).saturating_sub(common_desc_start),
            });
        }
        let common_descriptors = DescriptorLoop::new(&bytes[common_desc_start..common_desc_end]);

        let payload_end = total - CRC_LEN;
        let mut pos = common_desc_end;
        let mut platforms = Vec::new();
        while pos < payload_end {
            if pos + crate::compatibility::COMPAT_DESC_LEN_FIELD > payload_end {
                return Err(Error::BufferTooShort {
                    need: pos + crate::compatibility::COMPAT_DESC_LEN_FIELD,
                    have: payload_end,
                    what: "UntSection compatibilityDescriptorLength",
                });
            }
            let compat_desc_len = u16::from_be_bytes([bytes[pos], bytes[pos + 1]]) as usize;
            let compat_total = crate::compatibility::COMPAT_DESC_LEN_FIELD + compat_desc_len;
            if pos + compat_total > payload_end {
                return Err(Error::SectionLengthOverflow {
                    declared: compat_desc_len,
                    available: payload_end
                        .saturating_sub(pos + crate::compatibility::COMPAT_DESC_LEN_FIELD),
                });
            }
            let compatibility_descriptor =
                CompatibilityDescriptor::parse(&bytes[pos..pos + compat_total])?;
            pos += compat_total;

            if pos + PLATFORM_LOOP_LEN_FIELD > payload_end {
                return Err(Error::BufferTooShort {
                    need: pos + PLATFORM_LOOP_LEN_FIELD,
                    have: payload_end,
                    what: "UntSection platform_loop_length",
                });
            }
            let platform_loop_length = u16::from_be_bytes([bytes[pos], bytes[pos + 1]]) as usize;
            pos += PLATFORM_LOOP_LEN_FIELD;
            let platform_end = pos + platform_loop_length;
            if platform_end > payload_end {
                return Err(Error::SectionLengthOverflow {
                    declared: platform_loop_length,
                    available: payload_end.saturating_sub(pos),
                });
            }

            let mut target_operational_pairs = Vec::new();
            while pos < platform_end {
                if pos + DESC_LOOP_LEN_FIELD > platform_end {
                    return Err(Error::BufferTooShort {
                        need: pos + DESC_LOOP_LEN_FIELD,
                        have: platform_end,
                        what: "UntSection target_descriptor_loop length",
                    });
                }
                let target_len = (((bytes[pos] & 0x0F) as usize) << 8) | bytes[pos + 1] as usize;
                let target_start = pos + DESC_LOOP_LEN_FIELD;
                let target_end = target_start + target_len;
                if target_end > platform_end {
                    return Err(Error::SectionLengthOverflow {
                        declared: target_len,
                        available: platform_end.saturating_sub(target_start),
                    });
                }
                let target_descriptors = DescriptorLoop::new(&bytes[target_start..target_end]);
                pos = target_end;

                if pos + DESC_LOOP_LEN_FIELD > platform_end {
                    return Err(Error::BufferTooShort {
                        need: pos + DESC_LOOP_LEN_FIELD,
                        have: platform_end,
                        what: "UntSection operational_descriptor_loop length",
                    });
                }
                let op_len = (((bytes[pos] & 0x0F) as usize) << 8) | bytes[pos + 1] as usize;
                let op_start = pos + DESC_LOOP_LEN_FIELD;
                let op_end = op_start + op_len;
                if op_end > platform_end {
                    return Err(Error::SectionLengthOverflow {
                        declared: op_len,
                        available: platform_end.saturating_sub(op_start),
                    });
                }
                let operational_descriptors = DescriptorLoop::new(&bytes[op_start..op_end]);
                pos = op_end;

                target_operational_pairs.push((target_descriptors, operational_descriptors));
            }
            if pos != platform_end {
                return Err(Error::SectionLengthOverflow {
                    declared: platform_loop_length,
                    available: pos.saturating_sub(platform_end - platform_loop_length),
                });
            }

            platforms.push(UntPlatform {
                compatibility_descriptor,
                target_operational_pairs,
            });
        }

        Ok(UntSection {
            action_type,
            oui_hash,
            version_number,
            current_next_indicator,
            section_number,
            last_section_number,
            oui,
            processing_order,
            common_descriptors,
            platforms,
        })
    }
}

impl Serialize for UntSection<'_> {
    type Error = crate::error::Error;

    fn serialized_len(&self) -> usize {
        HEADER_LEN
            + FIXED_BODY_LEN
            + COMMON_DESC_LEN_FIELD
            + self.common_descriptors.len()
            + self
                .platforms
                .iter()
                .map(unt_platform_serialized_len)
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

        let section_length = (len - HEADER_LEN) as u16;
        if section_length > 0x0FFF {
            return Err(Error::SectionLengthOverflow {
                declared: section_length as usize,
                available: 0x0FFF,
            });
        }
        buf[0] = TABLE_ID;
        buf[1] =
            super::SECTION_B1_FLAGS_DVB | ((section_length >> 8) as u8 & LENGTH_HIGH_NIBBLE_MASK);
        buf[2] = (section_length & 0xFF) as u8;

        buf[OFFSET_ACTION_TYPE] = self.action_type.to_u8();
        buf[OFFSET_OUI_HASH] = self.oui_hash;
        buf[OFFSET_FLAGS] = FLAGS_RESERVED_BITS
            | ((self.version_number & 0x1F) << VERSION_NUMBER_SHIFT)
            | u8::from(self.current_next_indicator);
        buf[OFFSET_SECTION_NUMBER] = self.section_number;
        buf[OFFSET_LAST_SECTION_NUMBER] = self.last_section_number;
        buf[OFFSET_OUI] = ((self.oui >> 16) & 0xFF) as u8;
        buf[OFFSET_OUI + 1] = ((self.oui >> 8) & 0xFF) as u8;
        buf[OFFSET_OUI + 2] = (self.oui & 0xFF) as u8;
        buf[OFFSET_PROCESSING_ORDER] = self.processing_order;

        let cdl = self.common_descriptors.len() as u16;
        buf[OFFSET_COMMON_DESC_LEN] =
            RESERVED_NIBBLE | ((cdl >> 8) as u8 & LENGTH_HIGH_NIBBLE_MASK);
        buf[OFFSET_COMMON_DESC_LEN + 1] = (cdl & 0xFF) as u8;

        let common_start = OFFSET_COMMON_DESC_LEN + COMMON_DESC_LEN_FIELD;
        let common_end = common_start + self.common_descriptors.len();
        buf[common_start..common_end].copy_from_slice(self.common_descriptors.raw());

        let mut pos = common_end;
        for platform in &self.platforms {
            let written = platform
                .compatibility_descriptor
                .serialize_into(&mut buf[pos..])?;
            pos += written;

            let inner_len: usize = platform
                .target_operational_pairs
                .iter()
                .map(|(t, o)| DESC_LOOP_LEN_FIELD + t.len() + DESC_LOOP_LEN_FIELD + o.len())
                .sum();
            buf[pos..pos + PLATFORM_LOOP_LEN_FIELD]
                .copy_from_slice(&(inner_len as u16).to_be_bytes());
            pos += PLATFORM_LOOP_LEN_FIELD;

            for (target_descriptors, operational_descriptors) in &platform.target_operational_pairs
            {
                let tl = target_descriptors.len() as u16;
                buf[pos] = RESERVED_NIBBLE | ((tl >> 8) as u8 & 0x0F);
                buf[pos + 1] = (tl & 0xFF) as u8;
                pos += DESC_LOOP_LEN_FIELD;
                buf[pos..pos + target_descriptors.len()].copy_from_slice(target_descriptors.raw());
                pos += target_descriptors.len();

                let ol = operational_descriptors.len() as u16;
                buf[pos] = RESERVED_NIBBLE | ((ol >> 8) as u8 & 0x0F);
                buf[pos + 1] = (ol & 0xFF) as u8;
                pos += DESC_LOOP_LEN_FIELD;
                buf[pos..pos + operational_descriptors.len()]
                    .copy_from_slice(operational_descriptors.raw());
                pos += operational_descriptors.len();
            }
        }

        let crc_pos = len - CRC_LEN;
        let crc = dvb_common::crc32_mpeg2::compute(&buf[..crc_pos]);
        buf[crc_pos..len].copy_from_slice(&crc.to_be_bytes());
        Ok(len)
    }
}
impl<'a> crate::traits::TableDef<'a> for UntSection<'a> {
    const TABLE_ID_RANGES: &'static [(u8, u8)] = &[(TABLE_ID, TABLE_ID)];
    const NAME: &'static str = "UPDATE_NOTIFICATION";
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_happy_path() {
        let oui: u32 = 0x00_01_5A;
        let oui_hash: u8 = 0x01 ^ 0x5A;
        let common_descs: &[u8] = &[0x66, 0x04, 0x00, 0x0A, 0x00, 0x00];
        let unt = UntSection {
            action_type: UntActionType::SystemSoftwareUpdate,
            oui_hash,
            version_number: 7,
            current_next_indicator: true,
            section_number: 0,
            last_section_number: 0,
            oui,
            processing_order: 0x00,
            common_descriptors: DescriptorLoop::new(common_descs),
            platforms: vec![UntPlatform {
                compatibility_descriptor: CompatibilityDescriptor {
                    descriptors: vec![],
                },
                target_operational_pairs: vec![(
                    DescriptorLoop::new(&[]),
                    DescriptorLoop::new(&[]),
                )],
            }],
        };
        let sl = unt.serialized_len();
        let mut buf = vec![0u8; sl];
        unt.serialize_into(&mut buf).unwrap();
        let parsed = UntSection::parse(&buf).unwrap();
        assert_eq!(parsed.action_type, UntActionType::SystemSoftwareUpdate);
        assert_eq!(parsed.oui_hash, oui_hash);
        assert_eq!(parsed.version_number, 7);
        assert!(parsed.current_next_indicator);
        assert_eq!(parsed.oui, oui);
        assert_eq!(parsed.common_descriptors.raw(), common_descs);
        assert_eq!(parsed.platforms.len(), 1);
        assert!(parsed.platforms[0]
            .compatibility_descriptor
            .descriptors
            .is_empty());
    }

    #[test]
    fn parse_empty_platforms() {
        let unt = UntSection {
            action_type: UntActionType::SystemSoftwareUpdate,
            oui_hash: 0x5B,
            version_number: 1,
            current_next_indicator: false,
            section_number: 1,
            last_section_number: 2,
            oui: 0x00015A,
            processing_order: 0x01,
            common_descriptors: DescriptorLoop::new(&[]),
            platforms: Vec::new(),
        };
        let mut buf = vec![0u8; unt.serialized_len()];
        unt.serialize_into(&mut buf).unwrap();
        let parsed = UntSection::parse(&buf).unwrap();
        assert!(!parsed.current_next_indicator);
        assert!(parsed.platforms.is_empty());
    }

    #[test]
    fn byte_exact_round_trip() {
        let target_desc: &[u8] = &[0x09, 0x01, 0xAA];
        let op_desc: &[u8] = &[0x0A, 0x01, 0xBB];
        let unt = UntSection {
            action_type: UntActionType::SystemSoftwareUpdate,
            oui_hash: 0x5B,
            version_number: 15,
            current_next_indicator: true,
            section_number: 2,
            last_section_number: 5,
            oui: 0x00015A,
            processing_order: 0x02,
            common_descriptors: DescriptorLoop::new(&[0x66, 0x04, 0x00, 0x0A, 0x00, 0x00]),
            platforms: vec![UntPlatform {
                compatibility_descriptor: CompatibilityDescriptor {
                    descriptors: vec![],
                },
                target_operational_pairs: vec![(
                    DescriptorLoop::new(target_desc),
                    DescriptorLoop::new(op_desc),
                )],
            }],
        };
        let mut buf = vec![0u8; unt.serialized_len()];
        unt.serialize_into(&mut buf).unwrap();
        let re = UntSection::parse(&buf).unwrap();
        let mut buf2 = vec![0u8; re.serialized_len()];
        re.serialize_into(&mut buf2).unwrap();
        assert_eq!(buf, buf2, "byte-exact re-serialize");
        let re = UntSection::parse(&buf).unwrap();
        assert_eq!(re.platforms.len(), 1);
        assert!(re.platforms[0]
            .compatibility_descriptor
            .descriptors
            .is_empty());
        assert_eq!(re.platforms[0].target_operational_pairs.len(), 1);
        assert_eq!(
            re.platforms[0].target_operational_pairs[0].0.raw(),
            target_desc
        );
        assert_eq!(re.platforms[0].target_operational_pairs[0].1.raw(), op_desc);
    }

    #[test]
    fn round_trip_platform_with_multiple_pairs() {
        let t0: &[u8] = &[0x09, 0x01, 0xAA];
        let o0: &[u8] = &[0x0A, 0x01, 0xBB];
        let t1: &[u8] = &[0x01, 0x02, 0xCC, 0xDD];
        let o1: &[u8] = &[];
        let unt = UntSection {
            action_type: UntActionType::SystemSoftwareUpdate,
            oui_hash: 0x5B,
            version_number: 15,
            current_next_indicator: true,
            section_number: 0,
            last_section_number: 0,
            oui: 0x00015A,
            processing_order: 0x02,
            common_descriptors: DescriptorLoop::new(&[]),
            platforms: vec![UntPlatform {
                compatibility_descriptor: CompatibilityDescriptor {
                    descriptors: vec![],
                },
                target_operational_pairs: vec![
                    (DescriptorLoop::new(t0), DescriptorLoop::new(o0)),
                    (DescriptorLoop::new(t1), DescriptorLoop::new(o1)),
                ],
            }],
        };
        let mut buf = vec![0u8; unt.serialized_len()];
        unt.serialize_into(&mut buf).unwrap();
        let re = UntSection::parse(&buf).unwrap();
        assert_eq!(re.platforms.len(), 1);
        let pairs = &re.platforms[0].target_operational_pairs;
        assert_eq!(pairs.len(), 2, "both pairs must survive the round-trip");
        assert_eq!(pairs[0].0.raw(), t0);
        assert_eq!(pairs[0].1.raw(), o0);
        assert_eq!(pairs[1].0.raw(), t1);
        assert_eq!(pairs[1].1.raw(), o1);
        // serialize is deterministic.
        let mut buf2 = vec![0u8; unt.serialized_len()];
        unt.serialize_into(&mut buf2).unwrap();
        assert_eq!(buf, buf2, "byte-exact re-serialize");
    }

    #[test]
    fn round_trip_platform_with_nonempty_compat() {
        // A platform carrying a non-empty compatibilityDescriptor() (one entry
        // with a sub-descriptor) — the other UNT tests only exercise the empty
        // form, so this pins the full compat block through UntSection framing.
        use crate::compatibility::{
            CompatibilityDescriptorEntry, DescriptorType, SpecifierType, SubDescriptor,
            SubDescriptorType,
        };
        let unt = UntSection {
            action_type: UntActionType::SystemSoftwareUpdate,
            oui_hash: 0x5B,
            version_number: 3,
            current_next_indicator: true,
            section_number: 0,
            last_section_number: 0,
            oui: 0x00015A,
            processing_order: 0x00,
            common_descriptors: DescriptorLoop::new(&[]),
            platforms: vec![UntPlatform {
                compatibility_descriptor: CompatibilityDescriptor {
                    descriptors: vec![CompatibilityDescriptorEntry {
                        descriptor_type: DescriptorType::SystemHardware,
                        specifier_type: SpecifierType::IeeeOui,
                        specifier_data: [0x00, 0x15, 0x0A],
                        model: 0x1234,
                        version: 0x0001,
                        sub_descriptors: vec![SubDescriptor {
                            sub_descriptor_type: SubDescriptorType::Unallocated(0x05),
                            data: &[0xAA, 0xBB],
                        }],
                    }],
                },
                target_operational_pairs: vec![(
                    DescriptorLoop::new(&[]),
                    DescriptorLoop::new(&[]),
                )],
            }],
        };
        let mut buf = vec![0u8; unt.serialized_len()];
        unt.serialize_into(&mut buf).unwrap();
        let re = UntSection::parse(&buf).unwrap();
        assert_eq!(re, unt);
        let entry = &re.platforms[0].compatibility_descriptor.descriptors[0];
        assert_eq!(entry.descriptor_type, DescriptorType::SystemHardware);
        assert_eq!(entry.model, 0x1234);
        assert_eq!(entry.sub_descriptors[0].data, &[0xAA, 0xBB]);
        let mut buf2 = vec![0u8; unt.serialized_len()];
        unt.serialize_into(&mut buf2).unwrap();
        assert_eq!(buf, buf2, "byte-exact re-serialize");
    }

    #[test]
    fn parse_rejects_wrong_table_id() {
        let unt = UntSection {
            action_type: UntActionType::SystemSoftwareUpdate,
            oui_hash: 0x5B,
            version_number: 0,
            current_next_indicator: true,
            section_number: 0,
            last_section_number: 0,
            oui: 0x00015A,
            processing_order: 0x00,
            common_descriptors: DescriptorLoop::new(&[]),
            platforms: Vec::new(),
        };
        let mut buf = vec![0u8; unt.serialized_len()];
        unt.serialize_into(&mut buf).unwrap();
        buf[0] = 0x4A;
        assert!(matches!(
            UntSection::parse(&buf).unwrap_err(),
            Error::UnexpectedTableId { table_id: 0x4A, .. }
        ));
    }

    #[test]
    fn parse_rejects_short_buffer() {
        assert!(matches!(
            UntSection::parse(&[TABLE_ID, 0x00]).unwrap_err(),
            Error::BufferTooShort { .. }
        ));
    }

    #[test]
    fn serialize_rejects_small_output_buffer() {
        let unt = UntSection {
            action_type: UntActionType::SystemSoftwareUpdate,
            oui_hash: 0x5B,
            version_number: 0,
            current_next_indicator: true,
            section_number: 0,
            last_section_number: 0,
            oui: 0x00015A,
            processing_order: 0x00,
            common_descriptors: DescriptorLoop::new(&[]),
            platforms: Vec::new(),
        };
        let mut buf = vec![0u8; unt.serialized_len() - 1];
        assert!(matches!(
            unt.serialize_into(&mut buf).unwrap_err(),
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
            UntSection::parse(&buf).unwrap_err(),
            Error::SectionLengthOverflow { .. }
        ));
    }

    #[test]
    fn parse_handwritten_unt_no_platforms() {
        let mut bytes: Vec<u8> = vec![
            0x4B, 0xF0, 0x0F, 0x01, 0x5B, 0xC1, 0x00, 0x00, 0x00, 0x01, 0x5A, 0x00, 0xF0, 0x00,
        ];
        let crc = dvb_common::crc32_mpeg2::compute(&bytes);
        bytes.extend_from_slice(&crc.to_be_bytes());
        let unt = UntSection::parse(&bytes).unwrap();
        assert_eq!(unt.action_type, UntActionType::SystemSoftwareUpdate);
        assert_eq!(unt.oui, 0x00015A);
        assert!(unt.current_next_indicator);
        assert!(unt.platforms.is_empty());
    }

    #[test]
    fn action_type_full_range_round_trip() {
        for byte in 0u8..=0xFF {
            let at = UntActionType::from_u8(byte);
            assert_eq!(
                at.to_u8(),
                byte,
                "UntActionType round-trip failed for {byte:#04x}"
            );
        }
    }
}
