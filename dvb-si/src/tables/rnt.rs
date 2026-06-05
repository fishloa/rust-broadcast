//! Resolution provider Notification Table — ETSI TS 102 323 v1.4.1 §5.2.2.
//!
//! Carries the locations of CRI (Content Referencing Information) and metadata
//! for CRID authorities. Carried on PID 0x0016 with table_id 0x79.
//!
//! The resolution-provider loop internals (name bytes, per-provider descriptors,
//! CRID authority sub-loops) are kept as raw bytes — callers that need them
//! can walk `resolution_providers` directly.

use crate::descriptors::DescriptorLoop;
use crate::error::{Error, Result};
use crate::traits::Table;
use dvb_common::{Parse, Serialize};

/// table_id for the Resolution provider Notification Table.
pub const TABLE_ID: u8 = 0x79;
/// Well-known PID on which RNT sections are carried.
pub const PID: u16 = 0x0016;

/// Byte offset of the 3-byte outer header (table_id + section_length word).
const HEADER_LEN: usize = 3;
/// Bytes in the extension header: context_id(2) + version/cni byte(1)
/// + section_number(1) + last_section_number(1) + context_id_type(1) = 6.
const EXTENSION_HEADER_LEN: usize = 6;
/// Bytes for the common_descriptors_length field (2, carries a reserved nibble + 12-bit length).
const COMMON_DESC_LEN_FIELD: usize = 2;
/// Bytes consumed by the CRC-32 trailer.
const CRC_LEN: usize = 4;
/// Minimum total bytes required to attempt parsing.
const MIN_LEN: usize = HEADER_LEN + EXTENSION_HEADER_LEN + COMMON_DESC_LEN_FIELD + CRC_LEN;

/// Resolution provider Notification Table (ETSI TS 102 323 v1.4.1 §5.2.2).
///
/// Variable-length fields are kept as borrowed byte slices so no allocation is
/// required. The `resolution_providers` slice contains everything between the
/// end of the common-descriptor loop and the CRC-32 trailer; the internal
/// sub-structure (provider names, per-provider descriptors, CRID authority
/// loops) is not parsed further.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct Rnt<'a> {
    /// 16-bit context identifier (table_id_extension at bytes 3–4).
    pub context_id: u16,
    /// 5-bit version_number.
    pub version_number: u8,
    /// current_next_indicator bit.
    pub current_next_indicator: bool,
    /// section_number in the sub-table sequence.
    pub section_number: u8,
    /// last_section_number in the sub-table sequence.
    pub last_section_number: u8,
    /// context_id_type byte (0x00 = bouquet_id, 0x01 = original_network_id,
    /// 0x02 = network_id, 0x03–0x7F DVB reserved, 0x80–0xFF user defined).
    pub context_id_type: u8,
    /// Common descriptor loop (`common_descriptors_length` bytes). Serializes
    /// as the typed descriptor sequence; `.raw()` yields the wire bytes.
    pub common_descriptors: DescriptorLoop<'a>,
    /// Raw bytes of the resolution-provider loop (everything after the common
    /// descriptors up to, but not including, the CRC-32 trailer).
    pub resolution_providers: &'a [u8],
}

impl<'a> Parse<'a> for Rnt<'a> {
    type Error = crate::error::Error;

    fn parse(bytes: &'a [u8]) -> Result<Self> {
        if bytes.len() < MIN_LEN {
            return Err(Error::BufferTooShort {
                need: MIN_LEN,
                have: bytes.len(),
                what: "Rnt",
            });
        }

        if bytes[0] != TABLE_ID {
            return Err(Error::UnexpectedTableId {
                table_id: bytes[0],
                what: "Rnt",
                expected: &[TABLE_ID],
            });
        }

        // bytes[1] = section_syntax_indicator(1) | reserved(1) | reserved(2) | section_length[11:8](4)
        // bytes[2] = section_length[7:0]
        let section_length = ((bytes[1] & 0x0F) as u16) << 8 | bytes[2] as u16;
        let total = HEADER_LEN + section_length as usize;
        if bytes.len() < total {
            return Err(Error::SectionLengthOverflow {
                declared: section_length as usize,
                available: bytes.len() - HEADER_LEN,
            });
        }

        // Extension header (bytes 3–8):
        //   bytes[3..5]  = context_id (table_id_extension)
        //   bytes[5]     = reserved(2) | version_number(5) | current_next_indicator(1)
        //   bytes[6]     = section_number
        //   bytes[7]     = last_section_number
        //   bytes[8]     = context_id_type
        let context_id = u16::from_be_bytes([bytes[3], bytes[4]]);
        let version_number = (bytes[5] >> 1) & 0x1F;
        let current_next_indicator = (bytes[5] & 0x01) != 0;
        let section_number = bytes[6];
        let last_section_number = bytes[7];
        let context_id_type = bytes[8];

        // bytes[9..11] = reserved(4) | common_descriptors_length(12)
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

        // Everything from the end of the common descriptor loop up to (but not
        // including) the 4-byte CRC trailer is the resolution-provider loop.
        let rp_start = common_desc_end;
        let rp_end = total - CRC_LEN;
        let resolution_providers = &bytes[rp_start..rp_end];

        Ok(Rnt {
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

impl Serialize for Rnt<'_> {
    type Error = crate::error::Error;

    fn serialized_len(&self) -> usize {
        HEADER_LEN
            + EXTENSION_HEADER_LEN
            + COMMON_DESC_LEN_FIELD
            + self.common_descriptors.len()
            + self.resolution_providers.len()
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

        // Outer header.
        let section_length = (len - HEADER_LEN) as u16;
        buf[0] = TABLE_ID;
        // section_syntax_indicator=1, reserved=1, reserved=2, section_length top nibble.
        buf[1] = 0xB0 | ((section_length >> 8) as u8 & 0x0F);
        buf[2] = (section_length & 0xFF) as u8;

        // Extension header.
        buf[3..5].copy_from_slice(&self.context_id.to_be_bytes());
        buf[5] = 0xC0 | ((self.version_number & 0x1F) << 1) | u8::from(self.current_next_indicator);
        buf[6] = self.section_number;
        buf[7] = self.last_section_number;
        buf[8] = self.context_id_type;

        // common_descriptors_length field (reserved nibble = 0xF).
        let cdl = self.common_descriptors.len() as u16;
        let cdl_pos = HEADER_LEN + EXTENSION_HEADER_LEN;
        buf[cdl_pos] = 0xF0 | ((cdl >> 8) as u8 & 0x0F);
        buf[cdl_pos + 1] = (cdl & 0xFF) as u8;

        // Common descriptors.
        let cd_start = cdl_pos + COMMON_DESC_LEN_FIELD;
        let cd_end = cd_start + self.common_descriptors.len();
        buf[cd_start..cd_end].copy_from_slice(self.common_descriptors.raw());

        // Resolution-provider loop (opaque bytes).
        let rp_end = cd_end + self.resolution_providers.len();
        buf[cd_end..rp_end].copy_from_slice(self.resolution_providers);

        // CRC-32: compute over everything up to (but not including) the CRC slot.
        let crc_pos = len - CRC_LEN;
        let crc = dvb_common::crc32_mpeg2::compute(&buf[..crc_pos]);
        buf[crc_pos..len].copy_from_slice(&crc.to_be_bytes());

        Ok(len)
    }
}

impl<'a> Table<'a> for Rnt<'a> {
    const TABLE_ID: u8 = TABLE_ID;
    const PID: u16 = PID;
}

impl<'a> crate::traits::TableDef<'a> for Rnt<'a> {
    const TABLE_ID_RANGES: &'static [(u8, u8)] = &[(TABLE_ID, TABLE_ID)];
    const NAME: &'static str = "RELATED_AND_NEIGHBOURING";
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Build a complete RNT section byte vector with the given field values.
    /// `common_desc` and `resolution_providers` are pasted in verbatim; the
    /// CRC slot is zeroed (matching the serialize contract).
    #[allow(clippy::too_many_arguments)]
    fn build_rnt(
        context_id: u16,
        version: u8,
        current_next: bool,
        section_number: u8,
        last_section_number: u8,
        context_id_type: u8,
        common_desc: &[u8],
        resolution_providers: &[u8],
    ) -> Vec<u8> {
        let rnt = Rnt {
            context_id,
            version_number: version,
            current_next_indicator: current_next,
            section_number,
            last_section_number,
            context_id_type,
            common_descriptors: DescriptorLoop::new(common_desc),
            resolution_providers,
        };
        let mut buf = vec![0u8; rnt.serialized_len()];
        rnt.serialize_into(&mut buf).unwrap();
        buf
    }

    #[test]
    fn parse_happy_path() {
        // context_id 0x0042, version 3, CNI=true, section 0/0,
        // context_id_type 0x01 (original_network_id),
        // one dummy common descriptor (tag=0x83, length=2, data=[0xAB, 0xCD]),
        // one minimal resolution-provider stub (6 opaque bytes).
        let common_desc = [0x83u8, 0x02, 0xAB, 0xCD];
        let rp_bytes = [0xF0u8, 0x00, 0x02, b'b', b'b', 0xF0, 0x00];
        let bytes = build_rnt(0x0042, 3, true, 0, 0, 0x01, &common_desc, &rp_bytes);

        let rnt = Rnt::parse(&bytes).unwrap();
        assert_eq!(rnt.context_id, 0x0042);
        assert_eq!(rnt.version_number, 3);
        assert!(rnt.current_next_indicator);
        assert_eq!(rnt.section_number, 0);
        assert_eq!(rnt.last_section_number, 0);
        assert_eq!(rnt.context_id_type, 0x01);
        assert_eq!(rnt.common_descriptors.raw(), &common_desc[..]);
        assert_eq!(rnt.resolution_providers, &rp_bytes[..]);
    }

    #[test]
    fn parse_no_descriptors_no_providers() {
        let bytes = build_rnt(0x0000, 0, false, 0, 0, 0x00, &[], &[]);
        let rnt = Rnt::parse(&bytes).unwrap();
        assert_eq!(rnt.common_descriptors.len(), 0);
        assert_eq!(rnt.resolution_providers.len(), 0);
    }

    #[test]
    fn parse_rejects_wrong_table_id() {
        let mut bytes = build_rnt(0x0001, 0, true, 0, 0, 0x00, &[], &[]);
        bytes[0] = 0x70; // not 0x79
        let err = Rnt::parse(&bytes).unwrap_err();
        assert!(matches!(
            err,
            Error::UnexpectedTableId { table_id: 0x70, .. }
        ));
    }

    #[test]
    fn parse_rejects_short_buffer() {
        let err = Rnt::parse(&[0x79, 0x00]).unwrap_err();
        assert!(matches!(err, Error::BufferTooShort { .. }));
    }

    #[test]
    fn serialize_round_trip() {
        let common_desc = [0x40u8, 0x03, b'R', b'N', b'T'];
        // Minimal resolution-provider entry: 2-byte length header (0 bytes length),
        // 1-byte name length (0), 2-byte provider descriptors length (0).
        let rp_bytes = [0xF0u8, 0x03, 0x00, 0xF0, 0x00];
        let rnt = Rnt {
            context_id: 0xABCD,
            version_number: 15,
            current_next_indicator: true,
            section_number: 1,
            last_section_number: 2,
            context_id_type: 0x02,
            common_descriptors: DescriptorLoop::new(&common_desc),
            resolution_providers: &rp_bytes,
        };

        let mut buf = vec![0u8; rnt.serialized_len()];
        rnt.serialize_into(&mut buf).unwrap();
        let parsed = Rnt::parse(&buf).unwrap();
        assert_eq!(rnt, parsed);
    }

    #[test]
    fn serialize_rejects_too_small_buffer() {
        let rnt = Rnt {
            context_id: 0x0001,
            version_number: 0,
            current_next_indicator: true,
            section_number: 0,
            last_section_number: 0,
            context_id_type: 0x00,
            common_descriptors: DescriptorLoop::new(&[]),
            resolution_providers: &[],
        };
        let mut buf = vec![0u8; 2];
        let err = rnt.serialize_into(&mut buf).unwrap_err();
        assert!(matches!(err, Error::OutputBufferTooSmall { .. }));
    }
}
