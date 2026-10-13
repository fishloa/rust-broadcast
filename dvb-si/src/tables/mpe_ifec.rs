//! MPE-IFEC section — ETSI TS 102 772 v1.1.1 §5.2 (table_id 0x7A).
//!
//! Carries Inter-burst FEC (IFEC) data for DVB-SH / IP datacast. Syntax per
//! "Table 2 — MPE-IFEC section" (`dvb-si/docs/ts_102_772_mpe_ifec.md`, §5.2,
//! PDF p. 17):
//!
//! ```text
//! MPE-IFEC_section() {
//!   table_id                  8   (0x7A)
//!   section_syntax_indicator  1
//!   private_indicator         1
//!   reserved                  2
//!   section_length           12
//!   burst_number              8
//!   IFEC_burst_size           8
//!   reserved                  2
//!   version                   5
//!   current_next_indicator    1
//!   section_number            8
//!   last_section_number       8
//!   real_time_parameters()   32
//!   for (i=0; i<Nmax; i++) { IFEC_data_byte 8 }
//!   CRC_32                   32
//! }
//! ```
//!
//! Unlike MPE-FEC, this section DOES carry a 5-bit `version` (byte 5), and the
//! byte-3/4 slot is `burst_number(8)` + `IFEC_burst_size(8)` rather than a
//! 16-bit identifier. The `real_time_parameters()` block (32 bits) has the SAME
//! bit-packing as MPE-FEC's but DIFFERENT field semantics per "Table 3"
//! (§5.3, PDF p. 18): `delta_t(12)` | `mpe_boundary(1)` | `frame_boundary(1)`
//! | `prev_burst_size(18)`. The doc deliberately keeps a local copy of the
//! struct (no shared module) — a coordinator may dedup later if warranted.
//!
//! MPE-IFEC is a private section (byte-1 bit 6 = `private_indicator`), handled
//! with the [`crate::tables::cit`] idiom. `ifec_data` is a borrowed raw slice.
//! No well-known PID — carriage is descriptor-signalled, so `PID = 0x0000`
//! follows the [`crate::tables::dsmcc`] precedent.

use crate::error::{Error, Result};
use crate::tables::real_time_parameters::{RTP_LEN, RealTimeParametersBits};
use broadcast_common::{Parse, Serialize};

/// table_id for the MPE-IFEC section.
pub const TABLE_ID: u8 = 0x7A;

/// MPE-IFEC has no well-known PID — its carriage is signalled via descriptors.
/// `0x0000` follows the DSM-CC precedent.
pub const PID: mpeg_ts::pid::Pid = mpeg_ts::pid::Pid::new(0x0000);

/// Bytes 0-2: table_id (1) + flags + section_length (2).
const HEADER_LEN: usize = 3;

/// Bytes 3-7: burst_number(1) + IFEC_burst_size(1) + reserved/version/cni(1)
/// + section_number(1) + last_section_number(1).
const EXTENSION_HEADER_LEN: usize = 5;

use super::CRC_LEN;

/// Minimum total encoded length: header + extension + RTP + CRC.
const MIN_LEN: usize = HEADER_LEN + EXTENSION_HEADER_LEN + RTP_LEN + CRC_LEN;

/// Time-slicing / MPE-IFEC real-time parameters (TS 102 772 §5.3, Table 3).
///
/// 32 bits: `delta_t(12)` | `mpe_boundary(1)` | `frame_boundary(1)`
/// | `prev_burst_size(18)`. Same bit layout as the MPE-FEC variant but the
/// final two fields are `mpe_boundary` / `prev_burst_size`, not
/// `table_boundary` / `address`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct RealTimeParameters {
    /// 12-bit `delta_t` — time until the start of the next burst.
    pub delta_t: u16,
    /// `mpe_boundary` flag.
    pub mpe_boundary: bool,
    /// `frame_boundary` flag.
    pub frame_boundary: bool,
    /// 18-bit `prev_burst_size`.
    pub prev_burst_size: u32,
}

impl RealTimeParameters {
    /// Decode the 4-byte real_time_parameters block via the shared codec
    /// (r02-W22).
    fn from_bytes(b: [u8; RTP_LEN]) -> Self {
        let bits = RealTimeParametersBits::from_bytes(b);
        RealTimeParameters {
            delta_t: bits.delta_t,
            mpe_boundary: bits.boundary,
            frame_boundary: bits.frame_boundary,
            prev_burst_size: bits.tail,
        }
    }

    /// Encode into the 4-byte real_time_parameters block via the shared codec.
    fn to_bytes(self) -> Result<[u8; RTP_LEN]> {
        RealTimeParametersBits {
            delta_t: self.delta_t,
            boundary: self.mpe_boundary,
            frame_boundary: self.frame_boundary,
            tail: self.prev_burst_size,
        }
        .to_bytes()
        .map_err(Error::from)
    }
}

/// MPE-IFEC section (ETSI TS 102 772 v1.1.1 §5.2).
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[cfg_attr(feature = "yoke", derive(yoke::Yokeable))]
#[non_exhaustive]
pub struct MpeIfecSection<'a> {
    /// `private_indicator` bit from byte 1 (MPE-IFEC is a private section).
    pub private_indicator: bool,
    /// `burst_number` (byte 3) — the IFEC burst this section belongs to.
    pub burst_number: u8,
    /// `IFEC_burst_size` (byte 4) — number of application data tables in the
    /// burst.
    pub ifec_burst_size: u8,
    /// 5-bit `version` (byte 5).
    pub version: u8,
    /// `current_next_indicator` bit.
    pub current_next_indicator: bool,
    /// section_number in the sub-table sequence.
    pub section_number: u8,
    /// last_section_number in the sub-table sequence.
    pub last_section_number: u8,
    /// The decoded real-time parameters block.
    pub real_time_parameters: RealTimeParameters,
    /// Raw IFEC data bytes (everything between the real_time_parameters block
    /// and the CRC-32 trailer).
    pub ifec_data: &'a [u8],
    /// `reserved(2)` in byte 1 bits `[5:4]` (`11`); preserved from the wire.
    #[cfg_attr(feature = "serde", serde(skip))]
    pub reserved_b1: u8,
    /// `reserved(2)` in front of `version` (`11`); preserved.
    #[cfg_attr(feature = "serde", serde(skip))]
    pub reserved_b5: u8,
}

impl<'a> MpeIfecSection<'a> {
    /// Construct an MPE-IFEC section with the spec `reserved` fill.
    #[must_use]
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        private_indicator: bool,
        burst_number: u8,
        ifec_burst_size: u8,
        version: u8,
        current_next_indicator: bool,
        section_number: u8,
        last_section_number: u8,
        real_time_parameters: RealTimeParameters,
        ifec_data: &'a [u8],
    ) -> Self {
        Self {
            private_indicator,
            burst_number,
            ifec_burst_size,
            version,
            current_next_indicator,
            section_number,
            last_section_number,
            real_time_parameters,
            ifec_data,
            reserved_b1: super::RESERVED_2,
            reserved_b5: super::RESERVED_2,
        }
    }
}

impl<'a> Parse<'a> for MpeIfecSection<'a> {
    type Error = crate::error::Error;

    fn parse(bytes: &'a [u8]) -> Result<Self> {
        if bytes.len() < MIN_LEN {
            return Err(Error::BufferTooShort {
                need: MIN_LEN,
                have: bytes.len(),
                what: "MpeIfecSection",
            });
        }

        if bytes[0] != TABLE_ID {
            return Err(Error::UnexpectedTableId {
                table_id: bytes[0],
                what: "MpeIfecSection",
                expected: &[TABLE_ID],
            });
        }

        let section_length = super::section_length_of(bytes);
        let total = super::check_section_length(bytes.len(), HEADER_LEN, section_length, MIN_LEN)?;

        let private_indicator = (bytes[1] & 0x40) != 0;
        let burst_number = bytes[3];
        let ifec_burst_size = bytes[4];
        // byte 5: reserved(2) | version(5) | current_next_indicator(1)
        let version = super::version_number_of(bytes[5]);
        let current_next_indicator = super::current_next_of(bytes[5]);
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
        let ifec_data = &bytes[data_start..data_end];

        Ok(MpeIfecSection {
            private_indicator,
            burst_number,
            ifec_burst_size,
            version,
            current_next_indicator,
            section_number,
            last_section_number,
            real_time_parameters,
            ifec_data,
            reserved_b1: super::b1_reserved_of(bytes[1]),
            reserved_b5: super::b5_reserved_of(bytes[5]),
        })
    }
}

impl Serialize for MpeIfecSection<'_> {
    type Error = crate::error::Error;

    fn serialized_len(&self) -> usize {
        HEADER_LEN + EXTENSION_HEADER_LEN + RTP_LEN + self.ifec_data.len() + CRC_LEN
    }

    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        let len = self.serialized_len();
        if buf.len() < len {
            return Err(Error::OutputBufferTooSmall {
                need: len,
                have: buf.len(),
            });
        }

        // Byte 0: table_id.
        buf[0] = TABLE_ID;
        // Byte 1: section_syntax_indicator(1)=1 | private_indicator(1)
        //         | reserved(2)=11 | section_length[11:8](4).
        buf[1] = super::b1_of(true, self.private_indicator, self.reserved_b1);
        super::write_section_length(buf, len - HEADER_LEN)?;

        // Extension header.
        buf[3] = self.burst_number;
        buf[4] = self.ifec_burst_size;
        // reserved(2)=11 | version(5) | current_next_indicator(1)
        buf[5] = super::b5_of(self.reserved_b5, self.version, self.current_next_indicator);
        buf[6] = self.section_number;
        buf[7] = self.last_section_number;

        // real_time_parameters.
        let rtp_start = HEADER_LEN + EXTENSION_HEADER_LEN;
        buf[rtp_start..rtp_start + RTP_LEN].copy_from_slice(&self.real_time_parameters.to_bytes()?);

        // ifec_data.
        let data_start = rtp_start + RTP_LEN;
        let data_end = data_start + self.ifec_data.len();
        buf[data_start..data_end].copy_from_slice(self.ifec_data);

        // CRC-32 over everything up to (but not including) the CRC slot.
        super::write_crc(buf, data_end);

        Ok(len)
    }
}
impl<'a> crate::traits::TableDef<'a> for MpeIfecSection<'a> {
    const TABLE_ID_RANGES: &'static [(u8, u8)] = &[(TABLE_ID, TABLE_ID)];
    const NAME: &'static str = "MPE_IFEC";
}

#[cfg(test)]
mod tests {
    use super::*;

    #[allow(clippy::too_many_arguments)]
    fn build_mpe_ifec(
        burst_number: u8,
        ifec_burst_size: u8,
        version: u8,
        current_next: bool,
        section_number: u8,
        last_section_number: u8,
        rtp: RealTimeParameters,
        ifec_data: &[u8],
    ) -> Vec<u8> {
        let s = MpeIfecSection::new(
            true,
            burst_number,
            ifec_burst_size,
            version,
            current_next,
            section_number,
            last_section_number,
            rtp,
            ifec_data,
        );
        let mut buf = vec![0u8; s.serialized_len()];
        s.serialize_into(&mut buf).unwrap();
        buf
    }

    fn sample_rtp() -> RealTimeParameters {
        RealTimeParameters {
            delta_t: 0x0ABC,
            mpe_boundary: true,
            frame_boundary: false,
            prev_burst_size: 0x0001_2345,
        }
    }

    #[test]
    fn parse_happy_path() {
        let ifec = [0x11u8, 0x22, 0x33, 0x44];
        let bytes = build_mpe_ifec(5, 16, 9, true, 1, 3, sample_rtp(), &ifec);
        let s = MpeIfecSection::parse(&bytes).unwrap();
        assert!(s.private_indicator);
        assert_eq!(s.burst_number, 5);
        assert_eq!(s.ifec_burst_size, 16);
        assert_eq!(s.version, 9);
        assert!(s.current_next_indicator);
        assert_eq!(s.section_number, 1);
        assert_eq!(s.last_section_number, 3);
        assert_eq!(s.real_time_parameters, sample_rtp());
        assert_eq!(s.ifec_data, &ifec[..]);
    }

    #[test]
    fn parse_empty_ifec_data() {
        let bytes = build_mpe_ifec(0, 0, 0, false, 0, 0, sample_rtp(), &[]);
        let s = MpeIfecSection::parse(&bytes).unwrap();
        assert_eq!(s.burst_number, 0);
        assert_eq!(s.version, 0);
        assert!(!s.current_next_indicator);
        assert!(s.ifec_data.is_empty());
        assert_eq!(s.real_time_parameters, sample_rtp());
    }

    #[test]
    fn rtp_bit_packing_round_trips_extremes() {
        let rtp = RealTimeParameters {
            delta_t: 0x0FFF,
            mpe_boundary: false,
            frame_boundary: true,
            prev_burst_size: 0x0003_FFFF,
        };
        assert_eq!(RealTimeParameters::from_bytes(rtp.to_bytes().unwrap()), rtp);
    }

    #[test]
    fn parse_rejects_wrong_tag() {
        let mut bytes = build_mpe_ifec(0, 0, 0, true, 0, 0, sample_rtp(), &[]);
        bytes[0] = 0x70; // not 0x7A
        assert!(matches!(
            MpeIfecSection::parse(&bytes).unwrap_err(),
            Error::UnexpectedTableId { table_id: 0x70, .. }
        ));
    }

    #[test]
    fn parse_rejects_short_buffer() {
        assert!(matches!(
            MpeIfecSection::parse(&[0x7A, 0x80]).unwrap_err(),
            Error::BufferTooShort { .. }
        ));
    }

    #[test]
    fn parse_rejects_section_length_overflow() {
        let mut bytes = build_mpe_ifec(0, 0, 0, true, 0, 0, sample_rtp(), &[]);
        let fake_sl: u16 = (bytes.len() as u16) + 100 - HEADER_LEN as u16;
        bytes[1] = (bytes[1] & 0xF0) | ((fake_sl >> 8) as u8 & 0x0F);
        bytes[2] = (fake_sl & 0xFF) as u8;
        assert!(matches!(
            MpeIfecSection::parse(&bytes).unwrap_err(),
            Error::SectionLengthOverflow { .. }
        ));
    }

    #[test]
    fn serialize_round_trip() {
        let ifec = [0xDEu8, 0xAD, 0xBE, 0xEF, 0x00];
        let original = MpeIfecSection::new(false, 200, 32, 31, false, 2, 4, sample_rtp(), &ifec);
        let mut buf = vec![0u8; original.serialized_len()];
        original.serialize_into(&mut buf).unwrap();
        assert_eq!(MpeIfecSection::parse(&buf).unwrap(), original);
    }

    #[test]
    fn serialize_rejects_output_buffer_too_small() {
        let s = MpeIfecSection::new(false, 0, 0, 0, true, 0, 0, sample_rtp(), &[]);
        let mut buf = vec![0u8; 2];
        assert!(matches!(
            s.serialize_into(&mut buf).unwrap_err(),
            Error::OutputBufferTooSmall { .. }
        ));
    }

    #[test]
    fn serialize_rejects_over_wide_real_time_parameters() {
        let mut rtp = sample_rtp();
        rtp.delta_t = 0x1000; // 13 bits
        let mut s = MpeIfecSection::new(false, 0, 0, 0, true, 0, 0, rtp, &[]);
        let mut buf = vec![0u8; s.serialized_len()];
        assert!(matches!(
            s.serialize_into(&mut buf).unwrap_err(),
            Error::FieldOverflow(_)
        ));
        s.real_time_parameters = sample_rtp();
        s.real_time_parameters.prev_burst_size = 0x4_0000; // 19 bits
        assert!(matches!(
            s.serialize_into(&mut buf).unwrap_err(),
            Error::FieldOverflow(_)
        ));
    }

    #[test]
    fn table_trait_constants() {
        assert_eq!(TABLE_ID, 0x7A);
        assert_eq!(PID.value(), 0x0000);
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
            MpeIfecSection::parse(&buf).unwrap_err(),
            Error::SectionLengthOverflow { .. }
        ));
    }

    /// section_length is 12 bits (max 4095 = EXTENSION_HEADER_LEN(5) +
    /// RTP_LEN(4) + ifec_data + CRC_LEN(4), so 4082 is the largest
    /// ifec_data that fits).
    #[test]
    fn serialize_rejects_ifec_data_exceeding_section_length() {
        let ifec_data = vec![0xAAu8; 4083];
        let mpe_ifec = MpeIfecSection::new(
            false,
            0,
            0,
            0,
            true,
            0,
            0,
            RealTimeParameters {
                delta_t: 0,
                mpe_boundary: false,
                frame_boundary: false,
                prev_burst_size: 0,
            },
            &ifec_data,
        );
        let mut buf = vec![0u8; mpe_ifec.serialized_len()];
        assert!(matches!(
            mpe_ifec.serialize_into(&mut buf).unwrap_err(),
            Error::FieldOverflow(_)
        ));
    }

    #[test]
    fn serialize_accepts_ifec_data_at_section_length_boundary() {
        let ifec_data = vec![0xAAu8; 4082];
        let mpe_ifec = MpeIfecSection::new(
            false,
            0,
            0,
            0,
            true,
            0,
            0,
            RealTimeParameters {
                delta_t: 0,
                mpe_boundary: false,
                frame_boundary: false,
                prev_burst_size: 0,
            },
            &ifec_data,
        );
        let mut buf = vec![0u8; mpe_ifec.serialized_len()];
        mpe_ifec.serialize_into(&mut buf).expect("boundary fits");
        let reparsed = MpeIfecSection::parse(&buf).expect("reparse");
        assert_eq!(mpe_ifec, reparsed);
    }

    #[cfg(feature = "serde")]
    #[test]
    fn serde_json_serializes_fields() {
        // Serialize-only: assert serialization yields valid, field-bearing JSON.
        let ifec = [0x01u8, 0x02];
        let bytes = build_mpe_ifec(7, 8, 3, true, 0, 0, sample_rtp(), &ifec);
        let s = MpeIfecSection::parse(&bytes).unwrap();
        let v: serde_json::Value = serde_json::to_value(&s).unwrap();
        assert_eq!(v["burst_number"], 7);
        assert_eq!(v["ifec_burst_size"], 8);
        assert_eq!(v["version"], 3);
        assert_eq!(v["ifec_data"], serde_json::json!([0x01, 0x02]));
        assert_eq!(v["real_time_parameters"]["prev_burst_size"], 0x0001_2345);
        assert_eq!(v["real_time_parameters"]["mpe_boundary"], true);
    }
}
