//! Resolution provider Notification Table — ETSI TS 102 323 v1.4.1 §5.2.2.
//!
//! Carries the locations of CRI (Content Referencing Information) and metadata
//! for CRID authorities. Carried on PID 0x0016 with table_id 0x79.
//!
//! The resolution-provider loop is unfolded into [`ResolutionProvider`] and
//! [`CridAuthority`] entries (Table 1, §5.2.2).

use crate::descriptors::DescriptorLoop;
use crate::error::{Error, Result};
use crate::text::DvbText;
use dvb_common::{Parse, Serialize};

/// `table_id` for the Resolution provider Notification Table.
pub const TABLE_ID: u8 = 0x79;
/// Well-known PID on which RNT sections are carried.
pub const PID: u16 = 0x0016;

const HEADER_LEN: usize = 3;
const EXTENSION_HEADER_LEN: usize = 6;
const COMMON_DESC_LEN_FIELD: usize = 2;
const CRC_LEN: usize = 4;
const MIN_LEN: usize = HEADER_LEN + EXTENSION_HEADER_LEN + COMMON_DESC_LEN_FIELD + CRC_LEN;

const RP_INFO_LEN_FIELD: usize = 2;
const RP_NAME_LEN_FIELD: usize = 1;
const RP_DESC_LEN_FIELD: usize = 2;
const CA_NAME_LEN_FIELD: usize = 1;
const CA_HEADER_LEN: usize = 2;

const RESERVED_NIBBLE: u8 = 0xF0;

/// CRID authority policy — ETSI TS 102 323 §5.2.2 Table 3.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub enum CridAuthorityPolicy {
    /// '00' — Permanent (CRIDs are never re-used).
    Permanent,
    /// '01' — Transient (CRIDs may be re-used over time).
    Transient,
    /// '10' — Either (each CRID may be transient or permanent).
    Either,
    /// '11' — Reserved.
    Reserved,
}

impl CridAuthorityPolicy {
    #[must_use]
    /// Decode from the wire value.  Every value maps (lossless).
    pub fn from_u8(v: u8) -> Self {
        match v & 0x03 {
            0 => Self::Permanent,
            1 => Self::Transient,
            2 => Self::Either,
            _ => Self::Reserved,
        }
    }

    #[must_use]
    /// Encode to the wire value.  Inverse of `from_u8` / `from_u16`.
    pub fn to_u8(self) -> u8 {
        match self {
            Self::Permanent => 0,
            Self::Transient => 1,
            Self::Either => 2,
            Self::Reserved => 3,
        }
    }

    #[must_use]
    /// Human-readable spec display name.
    pub fn name(self) -> &'static str {
        match self {
            Self::Permanent => "Permanent",
            Self::Transient => "Transient",
            Self::Either => "Either",
            Self::Reserved => "Reserved",
        }
    }
}

/// Context ID type — ETSI TS 102 323 §5.2.2 Table 2.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub enum ContextIdType {
    /// 0x00 — context_id is a value of bouquet_id.
    BouquetId,
    /// 0x01 — context_id is a value of original_network_id.
    OriginalNetworkId,
    /// 0x02 — context_id is a value of network_id.
    NetworkId,
    /// 0x03..=0x7F — DVB reserved.
    DvbReserved(u8),
    /// 0x80..=0xFF — User defined.
    UserDefined(u8),
}

impl ContextIdType {
    #[must_use]
    /// Decode from the wire value.  Every value maps (lossless).
    pub fn from_u8(v: u8) -> Self {
        match v {
            0x00 => Self::BouquetId,
            0x01 => Self::OriginalNetworkId,
            0x02 => Self::NetworkId,
            v if v < 0x80 => Self::DvbReserved(v),
            _ => Self::UserDefined(v),
        }
    }

    #[must_use]
    /// Encode to the wire value.  Inverse of `from_u8` / `from_u16`.
    pub fn to_u8(self) -> u8 {
        match self {
            Self::BouquetId => 0x00,
            Self::OriginalNetworkId => 0x01,
            Self::NetworkId => 0x02,
            Self::DvbReserved(v) | Self::UserDefined(v) => v,
        }
    }

    #[must_use]
    /// Human-readable spec display name.
    pub fn name(self) -> &'static str {
        match self {
            Self::BouquetId => "Bouquet ID",
            Self::OriginalNetworkId => "Original Network ID",
            Self::NetworkId => "Network ID",
            Self::DvbReserved(_) => "DVB Reserved",
            Self::UserDefined(_) => "User Defined",
        }
    }
}

/// A CRID authority entry within a resolution provider (Table 1, §5.2.2).
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct CridAuthority<'a> {
    /// CRID authority name (EN 300 468 Annex A text).
    pub name: DvbText<'a>,
    /// `CRID_authority_policy` — 2-bit value (Table 3):
    /// 0 = permanent, 1 = transient, 2 = either, 3 = reserved.
    pub crid_authority_policy: CridAuthorityPolicy,
    /// CRID authority descriptor loop.
    pub descriptors: DescriptorLoop<'a>,
}

/// A resolution-provider entry (Table 1, §5.2.2).
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct ResolutionProvider<'a> {
    /// Resolution provider name (EN 300 468 Annex A text).
    pub name: DvbText<'a>,
    /// Per-provider descriptor loop.
    pub descriptors: DescriptorLoop<'a>,
    /// CRID authority sub-entries.
    pub crid_authorities: Vec<CridAuthority<'a>>,
}

fn crid_authority_serialized_len(ca: &CridAuthority) -> usize {
    CA_NAME_LEN_FIELD + ca.name.len() + CA_HEADER_LEN + ca.descriptors.len()
}

fn resolution_provider_serialized_len(rp: &ResolutionProvider) -> usize {
    RP_NAME_LEN_FIELD
        + rp.name.len()
        + RP_DESC_LEN_FIELD
        + rp.descriptors.len()
        + rp.crid_authorities
            .iter()
            .map(crid_authority_serialized_len)
            .sum::<usize>()
}

/// Resolution provider Notification Table (ETSI TS 102 323 v1.4.1 §5.2.2,
/// Table 1).
///
/// The resolution-provider loop is unfolded into typed
/// [`ResolutionProvider`] entries.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[cfg_attr(feature = "yoke", derive(yoke::Yokeable))]
pub struct RntSection<'a> {
    /// 16-bit context identifier (table_id_extension).
    pub context_id: u16,
    /// 5-bit version_number.
    pub version_number: u8,
    /// `current_next_indicator` bit.
    pub current_next_indicator: bool,
    /// section_number in the sub-table sequence.
    pub section_number: u8,
    /// last_section_number in the sub-table sequence.
    pub last_section_number: u8,
    /// `context_id_type` byte (Table 2).
    pub context_id_type: ContextIdType,
    /// Common descriptor loop. Serializes as the typed descriptor sequence;
    /// `.raw()` yields the wire bytes.
    pub common_descriptors: DescriptorLoop<'a>,
    /// Resolution-provider entries — unfolded per Table 1.
    pub resolution_providers: Vec<ResolutionProvider<'a>>,
}

impl<'a> Parse<'a> for RntSection<'a> {
    type Error = crate::error::Error;

    fn parse(bytes: &'a [u8]) -> Result<Self> {
        if bytes.len() < MIN_LEN {
            return Err(Error::BufferTooShort {
                need: MIN_LEN,
                have: bytes.len(),
                what: "RntSection",
            });
        }
        if bytes[0] != TABLE_ID {
            return Err(Error::UnexpectedTableId {
                table_id: bytes[0],
                what: "RntSection",
                expected: &[TABLE_ID],
            });
        }

        let section_length = ((bytes[1] & 0x0F) as u16) << 8 | bytes[2] as u16;
        let total =
            super::check_section_length(bytes.len(), HEADER_LEN, section_length as usize, MIN_LEN)?;

        let context_id = u16::from_be_bytes([bytes[3], bytes[4]]);
        let version_number = (bytes[5] >> 1) & 0x1F;
        let current_next_indicator = (bytes[5] & 0x01) != 0;
        let section_number = bytes[6];
        let last_section_number = bytes[7];
        let context_id_type = ContextIdType::from_u8(bytes[8]);

        let common_desc_len_pos = HEADER_LEN + EXTENSION_HEADER_LEN;
        let common_descriptors_length = (((bytes[common_desc_len_pos] & 0x0F) as usize) << 8)
            | bytes[common_desc_len_pos + 1] as usize;
        let common_desc_start = common_desc_len_pos + COMMON_DESC_LEN_FIELD;
        let common_desc_end = common_desc_start + common_descriptors_length;
        if common_desc_end > total - CRC_LEN {
            return Err(Error::SectionLengthOverflow {
                declared: common_descriptors_length,
                available: (total - CRC_LEN).saturating_sub(common_desc_start),
            });
        }
        let common_descriptors = DescriptorLoop::new(&bytes[common_desc_start..common_desc_end]);

        let payload_end = total - CRC_LEN;
        let mut pos = common_desc_end;
        let mut resolution_providers = Vec::new();

        while pos < payload_end {
            if pos + RP_INFO_LEN_FIELD > payload_end {
                return Err(Error::BufferTooShort {
                    need: pos + RP_INFO_LEN_FIELD,
                    have: payload_end,
                    what: "RntSection resolution_provider_info_length",
                });
            }
            let rp_info_length = (((bytes[pos] & 0x0F) as usize) << 8) | bytes[pos + 1] as usize;
            pos += RP_INFO_LEN_FIELD;
            let rp_end = pos + rp_info_length;
            if rp_end > payload_end {
                return Err(Error::SectionLengthOverflow {
                    declared: rp_info_length,
                    available: payload_end.saturating_sub(pos),
                });
            }

            if pos + RP_NAME_LEN_FIELD > rp_end {
                return Err(Error::BufferTooShort {
                    need: pos + RP_NAME_LEN_FIELD,
                    have: rp_end,
                    what: "RntSection resolution_provider_name_length",
                });
            }
            let name_len = bytes[pos] as usize;
            pos += RP_NAME_LEN_FIELD;
            if pos + name_len > rp_end {
                return Err(Error::BufferTooShort {
                    need: pos + name_len,
                    have: rp_end,
                    what: "RntSection resolution_provider_name",
                });
            }
            let name = DvbText::new(&bytes[pos..pos + name_len]);
            pos += name_len;

            if pos + RP_DESC_LEN_FIELD > rp_end {
                return Err(Error::BufferTooShort {
                    need: pos + RP_DESC_LEN_FIELD,
                    have: rp_end,
                    what: "RntSection resolution_provider_descriptors_length",
                });
            }
            let rp_desc_len = (((bytes[pos] & 0x0F) as usize) << 8) | bytes[pos + 1] as usize;
            pos += RP_DESC_LEN_FIELD;
            let rp_desc_start = pos;
            let rp_desc_end = rp_desc_start + rp_desc_len;
            if rp_desc_end > rp_end {
                return Err(Error::SectionLengthOverflow {
                    declared: rp_desc_len,
                    available: rp_end.saturating_sub(rp_desc_start),
                });
            }
            let descriptors = DescriptorLoop::new(&bytes[rp_desc_start..rp_desc_end]);
            pos = rp_desc_end;

            let mut crid_authorities = Vec::new();
            while pos < rp_end {
                if pos + CA_NAME_LEN_FIELD > rp_end {
                    return Err(Error::BufferTooShort {
                        need: pos + CA_NAME_LEN_FIELD,
                        have: rp_end,
                        what: "RntSection CRID_authority_name_length",
                    });
                }
                let ca_name_len = bytes[pos] as usize;
                pos += CA_NAME_LEN_FIELD;
                if pos + ca_name_len > rp_end {
                    return Err(Error::BufferTooShort {
                        need: pos + ca_name_len,
                        have: rp_end,
                        what: "RntSection CRID_authority_name",
                    });
                }
                let ca_name = DvbText::new(&bytes[pos..pos + ca_name_len]);
                pos += ca_name_len;

                if pos + CA_HEADER_LEN > rp_end {
                    return Err(Error::BufferTooShort {
                        need: pos + CA_HEADER_LEN,
                        have: rp_end,
                        what: "RntSection CRID_authority header",
                    });
                }
                let ca_packed = bytes[pos];
                let crid_authority_policy = CridAuthorityPolicy::from_u8((ca_packed >> 4) & 0x03);
                let ca_desc_len = (((ca_packed & 0x0F) as usize) << 8) | bytes[pos + 1] as usize;
                pos += CA_HEADER_LEN;
                let ca_desc_start = pos;
                let ca_desc_end = ca_desc_start + ca_desc_len;
                if ca_desc_end > rp_end {
                    return Err(Error::SectionLengthOverflow {
                        declared: ca_desc_len,
                        available: rp_end.saturating_sub(ca_desc_start),
                    });
                }
                let ca_descriptors = DescriptorLoop::new(&bytes[ca_desc_start..ca_desc_end]);
                pos = ca_desc_end;

                crid_authorities.push(CridAuthority {
                    name: ca_name,
                    crid_authority_policy,
                    descriptors: ca_descriptors,
                });
            }

            resolution_providers.push(ResolutionProvider {
                name,
                descriptors,
                crid_authorities,
            });
            pos = rp_end;
        }

        Ok(RntSection {
            context_id,
            version_number,
            current_next_indicator,
            section_number,
            last_section_number,
            context_id_type,
            common_descriptors,
            resolution_providers,
        })
    }
}

impl Serialize for RntSection<'_> {
    type Error = crate::error::Error;

    fn serialized_len(&self) -> usize {
        HEADER_LEN
            + EXTENSION_HEADER_LEN
            + COMMON_DESC_LEN_FIELD
            + self.common_descriptors.len()
            + self
                .resolution_providers
                .iter()
                .map(|rp| RP_INFO_LEN_FIELD + resolution_provider_serialized_len(rp))
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
        buf[1] = super::SECTION_B1_FLAGS_DVB | ((section_length >> 8) as u8 & 0x0F);
        buf[2] = (section_length & 0xFF) as u8;

        buf[3..5].copy_from_slice(&self.context_id.to_be_bytes());
        buf[5] = 0xC0 | ((self.version_number & 0x1F) << 1) | u8::from(self.current_next_indicator);
        buf[6] = self.section_number;
        buf[7] = self.last_section_number;
        buf[8] = self.context_id_type.to_u8();

        let cdl = self.common_descriptors.len() as u16;
        let cdl_pos = HEADER_LEN + EXTENSION_HEADER_LEN;
        buf[cdl_pos] = RESERVED_NIBBLE | ((cdl >> 8) as u8 & 0x0F);
        buf[cdl_pos + 1] = (cdl & 0xFF) as u8;

        let cd_start = cdl_pos + COMMON_DESC_LEN_FIELD;
        let cd_end = cd_start + self.common_descriptors.len();
        buf[cd_start..cd_end].copy_from_slice(self.common_descriptors.raw());

        let mut pos = cd_end;
        for rp in &self.resolution_providers {
            let rp_body_len = resolution_provider_serialized_len(rp);
            let rp_info_length = rp_body_len as u16;
            buf[pos] = RESERVED_NIBBLE | ((rp_info_length >> 8) as u8 & 0x0F);
            buf[pos + 1] = (rp_info_length & 0xFF) as u8;
            pos += RP_INFO_LEN_FIELD;

            if rp.name.len() > u8::MAX as usize {
                return Err(Error::ValueOutOfRange {
                    field: "resolution_provider_name_length",
                    reason: "exceeds 255 bytes",
                });
            }
            buf[pos] = rp.name.len() as u8;
            pos += RP_NAME_LEN_FIELD;
            buf[pos..pos + rp.name.len()].copy_from_slice(rp.name.raw());
            pos += rp.name.len();

            let rdl = rp.descriptors.len() as u16;
            buf[pos] = RESERVED_NIBBLE | ((rdl >> 8) as u8 & 0x0F);
            buf[pos + 1] = (rdl & 0xFF) as u8;
            pos += RP_DESC_LEN_FIELD;
            buf[pos..pos + rp.descriptors.len()].copy_from_slice(rp.descriptors.raw());
            pos += rp.descriptors.len();

            for ca in &rp.crid_authorities {
                if ca.name.len() > u8::MAX as usize {
                    return Err(Error::ValueOutOfRange {
                        field: "crid_authority_name_length",
                        reason: "exceeds 255 bytes",
                    });
                }
                buf[pos] = ca.name.len() as u8;
                pos += CA_NAME_LEN_FIELD;
                buf[pos..pos + ca.name.len()].copy_from_slice(ca.name.raw());
                pos += ca.name.len();

                let adl = ca.descriptors.len() as u16;
                buf[pos] = 0xC0
                    | ((ca.crid_authority_policy.to_u8() & 0x03) << 4)
                    | ((adl >> 8) as u8 & 0x0F);
                buf[pos + 1] = (adl & 0xFF) as u8;
                pos += CA_HEADER_LEN;
                buf[pos..pos + ca.descriptors.len()].copy_from_slice(ca.descriptors.raw());
                pos += ca.descriptors.len();
            }
        }

        let crc_pos = len - CRC_LEN;
        let crc = dvb_common::crc32_mpeg2::compute(&buf[..crc_pos]);
        buf[crc_pos..len].copy_from_slice(&crc.to_be_bytes());
        Ok(len)
    }
}
impl<'a> crate::traits::TableDef<'a> for RntSection<'a> {
    const TABLE_ID_RANGES: &'static [(u8, u8)] = &[(TABLE_ID, TABLE_ID)];
    const NAME: &'static str = "RESOLUTION_PROVIDER_NOTIFICATION";
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_happy_path() {
        let common_desc = [0x83u8, 0x02, 0xAB, 0xCD];
        let rp = ResolutionProvider {
            name: DvbText::new(b"bb"),
            descriptors: DescriptorLoop::new(&[]),
            crid_authorities: vec![CridAuthority {
                name: DvbText::new(b"au"),
                crid_authority_policy: CridAuthorityPolicy::Transient,
                descriptors: DescriptorLoop::new(&[]),
            }],
        };
        let rnt = RntSection {
            context_id: 0x0042,
            version_number: 3,
            current_next_indicator: true,
            section_number: 0,
            last_section_number: 0,
            context_id_type: ContextIdType::BouquetId,
            common_descriptors: DescriptorLoop::new(&common_desc),
            resolution_providers: vec![rp],
        };
        let mut buf = vec![0u8; rnt.serialized_len()];
        rnt.serialize_into(&mut buf).unwrap();
        let parsed = RntSection::parse(&buf).unwrap();
        assert_eq!(parsed.context_id, 0x0042);
        assert_eq!(parsed.version_number, 3);
        assert!(parsed.current_next_indicator);
        assert_eq!(parsed.context_id_type, ContextIdType::BouquetId);
        assert_eq!(parsed.resolution_providers.len(), 1);
        assert_eq!(parsed.resolution_providers[0].name.raw(), b"bb");
        assert_eq!(parsed.resolution_providers[0].crid_authorities.len(), 1);
        assert_eq!(
            parsed.resolution_providers[0].crid_authorities[0].crid_authority_policy,
            CridAuthorityPolicy::Transient
        );
        assert_eq!(
            parsed.resolution_providers[0].crid_authorities[0]
                .name
                .raw(),
            b"au"
        );
    }

    #[test]
    fn parse_no_descriptors_no_providers() {
        let rnt = RntSection {
            context_id: 0x0000,
            version_number: 0,
            current_next_indicator: false,
            section_number: 0,
            last_section_number: 0,
            context_id_type: ContextIdType::BouquetId,
            common_descriptors: DescriptorLoop::new(&[]),
            resolution_providers: Vec::new(),
        };
        let mut buf = vec![0u8; rnt.serialized_len()];
        rnt.serialize_into(&mut buf).unwrap();
        let parsed = RntSection::parse(&buf).unwrap();
        assert_eq!(parsed.common_descriptors.len(), 0);
        assert!(parsed.resolution_providers.is_empty());
    }

    #[test]
    fn byte_exact_round_trip() {
        let rp = ResolutionProvider {
            name: DvbText::new(b"provider"),
            descriptors: DescriptorLoop::new(&[0x40, 0x03, b'R', b'N', b'T']),
            crid_authorities: vec![
                CridAuthority {
                    name: DvbText::new(b"auth1"),
                    crid_authority_policy: CridAuthorityPolicy::Permanent,
                    descriptors: DescriptorLoop::new(&[]),
                },
                CridAuthority {
                    name: DvbText::new(b"auth2"),
                    crid_authority_policy: CridAuthorityPolicy::Either,
                    descriptors: DescriptorLoop::new(&[0x42, 0x00]),
                },
            ],
        };
        let rnt = RntSection {
            context_id: 0xABCD,
            version_number: 15,
            current_next_indicator: true,
            section_number: 1,
            last_section_number: 2,
            context_id_type: ContextIdType::NetworkId,
            common_descriptors: DescriptorLoop::new(&[0x40, 0x03, b'R', b'N', b'T']),
            resolution_providers: vec![rp],
        };
        let mut buf = vec![0u8; rnt.serialized_len()];
        rnt.serialize_into(&mut buf).unwrap();
        let re = RntSection::parse(&buf).unwrap();
        let mut buf2 = vec![0u8; re.serialized_len()];
        re.serialize_into(&mut buf2).unwrap();
        assert_eq!(buf, buf2, "byte-exact re-serialize");
        let re = RntSection::parse(&buf).unwrap();
        assert_eq!(re.resolution_providers.len(), 1);
        assert_eq!(re.resolution_providers[0].name.raw(), b"provider");
        assert_eq!(re.resolution_providers[0].crid_authorities.len(), 2);
        assert_eq!(
            re.resolution_providers[0].crid_authorities[1].crid_authority_policy,
            CridAuthorityPolicy::Either
        );
    }

    #[test]
    fn parse_rejects_wrong_table_id() {
        let rnt = RntSection {
            context_id: 0x0001,
            version_number: 0,
            current_next_indicator: true,
            section_number: 0,
            last_section_number: 0,
            context_id_type: ContextIdType::BouquetId,
            common_descriptors: DescriptorLoop::new(&[]),
            resolution_providers: Vec::new(),
        };
        let mut buf = vec![0u8; rnt.serialized_len()];
        rnt.serialize_into(&mut buf).unwrap();
        buf[0] = 0x70;
        assert!(matches!(
            RntSection::parse(&buf).unwrap_err(),
            Error::UnexpectedTableId { table_id: 0x70, .. }
        ));
    }

    #[test]
    fn parse_rejects_short_buffer() {
        assert!(matches!(
            RntSection::parse(&[0x79, 0x00]).unwrap_err(),
            Error::BufferTooShort { .. }
        ));
    }

    #[test]
    fn serialize_rejects_too_small_buffer() {
        let rnt = RntSection {
            context_id: 0x0001,
            version_number: 0,
            current_next_indicator: true,
            section_number: 0,
            last_section_number: 0,
            context_id_type: ContextIdType::BouquetId,
            common_descriptors: DescriptorLoop::new(&[]),
            resolution_providers: Vec::new(),
        };
        let mut buf = vec![0u8; 2];
        assert!(matches!(
            rnt.serialize_into(&mut buf).unwrap_err(),
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
            RntSection::parse(&buf).unwrap_err(),
            Error::SectionLengthOverflow { .. }
        ));
    }

    #[test]
    fn parse_handwritten_rnt_no_providers() {
        let mut bytes: Vec<u8> = vec![
            0x79, 0xF0, 0x0C, 0x00, 0x42, 0xC7, 0x00, 0x00, 0x01, 0xF0, 0x00,
        ];
        let crc = dvb_common::crc32_mpeg2::compute(&bytes);
        bytes.extend_from_slice(&crc.to_be_bytes());
        let rnt = RntSection::parse(&bytes).unwrap();
        assert_eq!(rnt.context_id, 0x0042);
        assert_eq!(rnt.version_number, 3);
        assert!(rnt.current_next_indicator);
        assert!(rnt.resolution_providers.is_empty());
    }

    #[test]
    fn crid_authority_policy_round_trip() {
        for v in 0u8..=3 {
            let p = CridAuthorityPolicy::from_u8(v);
            assert_eq!(p.to_u8(), v, "CridAuthorityPolicy round-trip for {v}");
        }
    }

    #[test]
    fn context_id_type_full_range_round_trip() {
        for byte in 0u8..=0xFF {
            let ct = ContextIdType::from_u8(byte);
            assert_eq!(
                ct.to_u8(),
                byte,
                "ContextIdType round-trip failed for {byte:#04x}"
            );
        }
    }
}
