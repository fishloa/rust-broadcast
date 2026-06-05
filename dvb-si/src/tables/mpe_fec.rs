//! MPE-FEC section — ETSI EN 301 192 v1.7.1 §9.9 (table_id 0x78).
//!
//! Carries Reed-Solomon FEC data for a MPE-FEC frame in a DVB-H/IP datacast
//! time-slice burst. Syntax per "Table 45 — MPE-FEC section"
//! (`dvb-si/docs/en_301_192.md`, §9.9, PDF p. 57):
//!
//! ```text
//! MPE-FEC_section() {
//!   table_id                  8   (0x78)
//!   section_syntax_indicator  1
//!   private_indicator         1
//!   reserved                  2
//!   section_length           12
//!   padding_columns           8
//!   reserved_for_future_use   8
//!   reserved                  2
//!   reserved_for_future_use   5
//!   current_next_indicator    1
//!   section_number            8
//!   last_section_number       8
//!   real_time_parameters()   32
//!   for (i=0; i<N; i++) { rs_data_byte 8 }
//!   CRC_32                   32
//! }
//! ```
//!
//! Note the byte-3/4 "table_id_extension" slot is NOT a 16-bit identifier here:
//! it is `padding_columns(8)` followed by an 8-bit `reserved_for_future_use`.
//! There is no `version_number` field — byte 5 is reserved bits plus the
//! `current_next_indicator`. `real_time_parameters` is the shared time-slice /
//! MPE-FEC structure of "Table 46 — real time parameters" (§9.10): a 12-bit
//! `delta_t`, two boundary flags, and an 18-bit `address`, packed into 4 bytes.
//!
//! MPE-FEC is a private section (byte-1 bit 6 = `private_indicator`), handled
//! with the [`crate::tables::cit`] idiom. `rs_data` is a borrowed raw slice.
//! No well-known PID — carriage is descriptor-signalled, so `PID = 0x0000`
//! follows the [`crate::tables::dsmcc`] precedent.

use crate::error::{Error, Result};
use crate::traits::Table;
use dvb_common::{Parse, Serialize};

/// table_id for the MPE-FEC section.
pub const TABLE_ID: u8 = 0x78;

/// MPE-FEC has no well-known PID — its carriage is signalled via the
/// time_slice_fec_identifier_descriptor. `0x0000` follows the DSM-CC precedent.
pub const PID: u16 = 0x0000;

/// Bytes 0-2: table_id (1) + flags + section_length (2).
const HEADER_LEN: usize = 3;

/// Bytes 3-7: padding_columns(1) + reserved_for_future_use(1)
/// + reserved/rfu/cni byte(1) + section_number(1) + last_section_number(1).
const EXTENSION_HEADER_LEN: usize = 5;

/// Bytes 8-11: the 32-bit `real_time_parameters()`.
const RTP_LEN: usize = 4;

/// Bytes occupied by the trailing CRC-32 field.
const CRC_LEN: usize = 4;

/// Minimum total encoded length: header + extension + RTP + CRC.
const MIN_LEN: usize = HEADER_LEN + EXTENSION_HEADER_LEN + RTP_LEN + CRC_LEN;

/// Time-slicing / MPE-FEC real-time parameters (EN 301 192 §9.10, Table 46).
///
/// 32 bits: `delta_t(12)` | `table_boundary(1)` | `frame_boundary(1)`
/// | `address(18)`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct RealTimeParameters {
    /// 12-bit `delta_t` — time until the start of the next burst (in units of
    /// the MPE-FEC frame, see §9.10).
    pub delta_t: u16,
    /// `table_boundary` flag.
    pub table_boundary: bool,
    /// `frame_boundary` flag.
    pub frame_boundary: bool,
    /// 18-bit `address` — byte position of the first payload byte of this
    /// section within the MPE-FEC frame table.
    pub address: u32,
}

impl RealTimeParameters {
    /// Decode the 4-byte real_time_parameters block.
    fn from_bytes(b: [u8; RTP_LEN]) -> Self {
        // delta_t(12) = b[0] | top 4 bits of b[1]
        let delta_t = ((b[0] as u16) << 4) | ((b[1] >> 4) as u16);
        let table_boundary = (b[1] & 0x08) != 0;
        let frame_boundary = (b[1] & 0x04) != 0;
        // address(18) = bottom 2 bits of b[1] | b[2] | b[3]
        let address = (((b[1] & 0x03) as u32) << 16) | ((b[2] as u32) << 8) | (b[3] as u32);
        RealTimeParameters {
            delta_t,
            table_boundary,
            frame_boundary,
            address,
        }
    }

    /// Encode into the 4-byte real_time_parameters block.
    fn to_bytes(self) -> [u8; RTP_LEN] {
        let dt = self.delta_t & 0x0FFF;
        let addr = self.address & 0x0003_FFFF;
        [
            (dt >> 4) as u8,
            (((dt & 0x0F) as u8) << 4)
                | (u8::from(self.table_boundary) << 3)
                | (u8::from(self.frame_boundary) << 2)
                | ((addr >> 16) as u8 & 0x03),
            ((addr >> 8) & 0xFF) as u8,
            (addr & 0xFF) as u8,
        ]
    }
}

/// MPE-FEC section (ETSI EN 301 192 v1.7.1 §9.9).
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[cfg_attr(feature = "yoke", derive(yoke::Yokeable))]
pub struct MpeFec<'a> {
    /// `private_indicator` bit from byte 1 (MPE-FEC is a private section).
    pub private_indicator: bool,
    /// `padding_columns` — number of padding RS columns (byte 3).
    pub padding_columns: u8,
    /// `current_next_indicator` bit.
    pub current_next_indicator: bool,
    /// section_number in the sub-table sequence.
    pub section_number: u8,
    /// last_section_number in the sub-table sequence.
    pub last_section_number: u8,
    /// The decoded real-time parameters block.
    pub real_time_parameters: RealTimeParameters,
    /// Raw Reed-Solomon data bytes (everything between the real_time_parameters
    /// block and the CRC-32 trailer).
    pub rs_data: &'a [u8],
}

impl<'a> Parse<'a> for MpeFec<'a> {
    type Error = crate::error::Error;

    fn parse(bytes: &'a [u8]) -> Result<Self> {
        if bytes.len() < MIN_LEN {
            return Err(Error::BufferTooShort {
                need: MIN_LEN,
                have: bytes.len(),
                what: "MpeFec",
            });
        }

        if bytes[0] != TABLE_ID {
            return Err(Error::UnexpectedTableId {
                table_id: bytes[0],
                what: "MpeFec",
                expected: &[TABLE_ID],
            });
        }

        let section_length = (((bytes[1] & 0x0F) as usize) << 8) | bytes[2] as usize;
        let total = HEADER_LEN + section_length;
        if bytes.len() < total {
            return Err(Error::SectionLengthOverflow {
                declared: section_length,
                available: bytes.len() - HEADER_LEN,
            });
        }

        let private_indicator = (bytes[1] & 0x40) != 0;
        let padding_columns = bytes[3];
        // bytes[4] = reserved_for_future_use(8) — ignored on parse.
        // bytes[5] = reserved(2) | reserved_for_future_use(5) | current_next_indicator(1)
        let current_next_indicator = (bytes[5] & 0x01) != 0;
        let section_number = bytes[6];
        let last_section_number = bytes[7];

        let rtp_start = HEADER_LEN + EXTENSION_HEADER_LEN;
        let real_time_parameters = RealTimeParameters::from_bytes([
            bytes[rtp_start],
            bytes[rtp_start + 1],
            bytes[rtp_start + 2],
            bytes[rtp_start + 3],
        ]);

        let data_start = rtp_start + RTP_LEN;
        let data_end = total - CRC_LEN;
        let rs_data = &bytes[data_start..data_end];

        Ok(MpeFec {
            private_indicator,
            padding_columns,
            current_next_indicator,
            section_number,
            last_section_number,
            real_time_parameters,
            rs_data,
        })
    }
}

impl Serialize for MpeFec<'_> {
    type Error = crate::error::Error;

    fn serialized_len(&self) -> usize {
        HEADER_LEN + EXTENSION_HEADER_LEN + RTP_LEN + self.rs_data.len() + CRC_LEN
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

        // Byte 0: table_id.
        buf[0] = TABLE_ID;
        // Byte 1: section_syntax_indicator(1)=1 | private_indicator(1)
        //         | reserved(2)=11 | section_length[11:8](4).
        buf[1] = 0x80
            | (u8::from(self.private_indicator) << 6)
            | 0x30
            | ((section_length >> 8) as u8 & 0x0F);
        buf[2] = (section_length & 0xFF) as u8;

        // Extension header.
        buf[3] = self.padding_columns;
        buf[4] = 0xFF; // reserved_for_future_use(8) emitted as 1s.
                       // reserved(2)=11 | reserved_for_future_use(5)=11111 | current_next_indicator(1)
        buf[5] = 0xFE | u8::from(self.current_next_indicator);
        buf[6] = self.section_number;
        buf[7] = self.last_section_number;

        // real_time_parameters.
        let rtp_start = HEADER_LEN + EXTENSION_HEADER_LEN;
        buf[rtp_start..rtp_start + RTP_LEN].copy_from_slice(&self.real_time_parameters.to_bytes());

        // rs_data.
        let data_start = rtp_start + RTP_LEN;
        let data_end = data_start + self.rs_data.len();
        buf[data_start..data_end].copy_from_slice(self.rs_data);

        // CRC-32 over everything up to (but not including) the CRC slot.
        let crc = dvb_common::crc32_mpeg2::compute(&buf[..data_end]);
        buf[data_end..len].copy_from_slice(&crc.to_be_bytes());

        Ok(len)
    }
}

impl<'a> Table<'a> for MpeFec<'a> {
    const TABLE_ID: u8 = TABLE_ID;
    const PID: u16 = PID;
}

impl<'a> crate::traits::TableDef<'a> for MpeFec<'a> {
    const TABLE_ID_RANGES: &'static [(u8, u8)] = &[(TABLE_ID, TABLE_ID)];
    const NAME: &'static str = "MPE_FEC";
}

#[cfg(test)]
mod tests {
    use super::*;

    fn build_mpe_fec(
        padding_columns: u8,
        current_next: bool,
        section_number: u8,
        last_section_number: u8,
        rtp: RealTimeParameters,
        rs_data: &[u8],
    ) -> Vec<u8> {
        let s = MpeFec {
            private_indicator: true,
            padding_columns,
            current_next_indicator: current_next,
            section_number,
            last_section_number,
            real_time_parameters: rtp,
            rs_data,
        };
        let mut buf = vec![0u8; s.serialized_len()];
        s.serialize_into(&mut buf).unwrap();
        buf
    }

    fn sample_rtp() -> RealTimeParameters {
        RealTimeParameters {
            delta_t: 0x0ABC,
            table_boundary: true,
            frame_boundary: false,
            address: 0x0001_2345,
        }
    }

    #[test]
    fn parse_happy_path() {
        let rs = [0x11u8, 0x22, 0x33, 0x44];
        let bytes = build_mpe_fec(7, true, 1, 3, sample_rtp(), &rs);
        let s = MpeFec::parse(&bytes).unwrap();
        assert!(s.private_indicator);
        assert_eq!(s.padding_columns, 7);
        assert!(s.current_next_indicator);
        assert_eq!(s.section_number, 1);
        assert_eq!(s.last_section_number, 3);
        assert_eq!(s.real_time_parameters, sample_rtp());
        assert_eq!(s.rs_data, &rs[..]);
    }

    #[test]
    fn parse_empty_rs_data() {
        let bytes = build_mpe_fec(0, false, 0, 0, sample_rtp(), &[]);
        let s = MpeFec::parse(&bytes).unwrap();
        assert_eq!(s.padding_columns, 0);
        assert!(!s.current_next_indicator);
        assert!(s.rs_data.is_empty());
        assert_eq!(s.real_time_parameters, sample_rtp());
    }

    #[test]
    fn rtp_bit_packing_round_trips_extremes() {
        // Exercise full-width values to pin the 12/1/1/18 split.
        let rtp = RealTimeParameters {
            delta_t: 0x0FFF,
            table_boundary: false,
            frame_boundary: true,
            address: 0x0003_FFFF,
        };
        assert_eq!(RealTimeParameters::from_bytes(rtp.to_bytes()), rtp);
    }

    #[test]
    fn parse_rejects_wrong_tag() {
        let mut bytes = build_mpe_fec(0, true, 0, 0, sample_rtp(), &[]);
        bytes[0] = 0x70; // not 0x78
        assert!(matches!(
            MpeFec::parse(&bytes).unwrap_err(),
            Error::UnexpectedTableId { table_id: 0x70, .. }
        ));
    }

    #[test]
    fn parse_rejects_short_buffer() {
        assert!(matches!(
            MpeFec::parse(&[0x78, 0x80]).unwrap_err(),
            Error::BufferTooShort { .. }
        ));
    }

    #[test]
    fn parse_rejects_section_length_overflow() {
        let mut bytes = build_mpe_fec(0, true, 0, 0, sample_rtp(), &[]);
        let fake_sl: u16 = (bytes.len() as u16) + 100 - HEADER_LEN as u16;
        bytes[1] = (bytes[1] & 0xF0) | ((fake_sl >> 8) as u8 & 0x0F);
        bytes[2] = (fake_sl & 0xFF) as u8;
        assert!(matches!(
            MpeFec::parse(&bytes).unwrap_err(),
            Error::SectionLengthOverflow { .. }
        ));
    }

    #[test]
    fn serialize_round_trip() {
        let rs = [0xDEu8, 0xAD, 0xBE, 0xEF, 0x00];
        let original = MpeFec {
            private_indicator: false,
            padding_columns: 191,
            current_next_indicator: false,
            section_number: 2,
            last_section_number: 4,
            real_time_parameters: sample_rtp(),
            rs_data: &rs,
        };
        let mut buf = vec![0u8; original.serialized_len()];
        original.serialize_into(&mut buf).unwrap();
        assert_eq!(MpeFec::parse(&buf).unwrap(), original);
    }

    #[test]
    fn serialize_rejects_output_buffer_too_small() {
        let s = MpeFec {
            private_indicator: false,
            padding_columns: 0,
            current_next_indicator: true,
            section_number: 0,
            last_section_number: 0,
            real_time_parameters: sample_rtp(),
            rs_data: &[],
        };
        let mut buf = vec![0u8; 2];
        assert!(matches!(
            s.serialize_into(&mut buf).unwrap_err(),
            Error::OutputBufferTooSmall { .. }
        ));
    }

    #[test]
    fn table_trait_constants() {
        assert_eq!(<MpeFec as Table>::TABLE_ID, 0x78);
        assert_eq!(<MpeFec as Table>::PID, 0x0000);
    }

    #[cfg(feature = "serde")]
    #[test]
    fn serde_json_serializes_fields() {
        // Serialize-only: assert serialization yields valid, field-bearing JSON.
        let rs = [0x01u8, 0x02];
        let bytes = build_mpe_fec(12, true, 0, 0, sample_rtp(), &rs);
        let s = MpeFec::parse(&bytes).unwrap();
        let v: serde_json::Value = serde_json::to_value(&s).unwrap();
        assert_eq!(v["padding_columns"], 12);
        assert_eq!(v["current_next_indicator"], true);
        assert_eq!(v["rs_data"], serde_json::json!([0x01, 0x02]));
        assert_eq!(v["real_time_parameters"]["delta_t"], 0x0ABC);
        assert_eq!(v["real_time_parameters"]["table_boundary"], true);
    }
}
