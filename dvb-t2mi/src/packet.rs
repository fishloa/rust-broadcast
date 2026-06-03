//! T2-MI packet header and type parsing.

use num_enum::TryFromPrimitive;

/// Packet types per ETSI TS 102 773 Table 1.
///
/// Reserved for future use: `0x22..=0x2F`, `0x34..=0xFF`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, TryFromPrimitive)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[repr(u8)]
pub enum PacketType {
    /// Baseband Frame (BBFRAME) — §5.2.1
    BasebandFrame = 0x00,
    /// Auxiliary stream I/Q data — §5.2.2
    AuxiliaryIqData = 0x01,
    /// Arbitrary cell insertion — §5.2.3
    ArbitraryCellInsertion = 0x02,
    /// L1-current signalling — §5.2.4
    L1Current = 0x10,
    /// L1-future signalling — §5.2.5
    L1Future = 0x11,
    /// P2 bias balancing cells — §5.2.6
    P2BiasBalancing = 0x12,
    /// DVB-T2 timestamp — §5.2.7
    Timestamp = 0x20,
    /// Individual addressing — §5.2.8
    IndividualAddressing = 0x21,
    /// FEF part: Null — §5.2.9
    FefPartNull = 0x30,
    /// FEF part: I/Q data — §5.2.10
    FefPartIqData = 0x31,
    /// FEF part: composite — §5.2.11
    FefPartComposite = 0x32,
    /// FEF sub-part — §5.2.12
    FefSubPart = 0x33,
}

impl From<PacketType> for u8 {
    fn from(pt: PacketType) -> Self {
        pt as u8
    }
}

impl From<num_enum::TryFromPrimitiveError<PacketType>> for crate::error::Error {
    fn from(e: num_enum::TryFromPrimitiveError<PacketType>) -> Self {
        crate::error::Error::InvalidPacketType { found: e.number }
    }
}

/// T2-MI packet header (6 bytes / 48 bits) per §5.1.
///
/// Layout:
/// - byte 0: packet_type (8 bits) — Table 1
/// - byte 1: packet_count (8 bits) — wraps 0xFF→0x00, arbitrary start
/// - byte 2 [7:4]: superframe_idx (4 bits)
/// - byte 2 [3]: rfu (1 bit) — must be 0
/// - byte 2 [2:0]: t2mi_stream_id (3 bits)
/// - byte 3: rfu (8 bits) — must be 0
/// - byte 4-5: payload_len (16 bits, unit = bits)
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct Header {
    /// Table 1 packet type.
    pub packet_type: PacketType,
    /// Wraps 0xFF→0x00; arbitrary start value.
    pub packet_count: u8,
    /// Super-frame index [0..=15].
    pub superframe_idx: u8,
    /// T2-MI stream ID [0..=7].
    pub t2mi_stream_id: u8,
    /// Payload length in bits (not bytes).
    pub payload_len_bits: u16,
}

impl<'a> dvb_common::Parse<'a> for Header {
    type Error = crate::error::Error;

    fn parse(bytes: &'a [u8]) -> Result<Self, crate::error::Error> {
        use super::error::Error;

        let len = bytes.len();
        if len < 6 {
            return Err(Error::BufferTooShort {
                need: 6,
                have: len,
                what: "T2MI Header",
            });
        }

        let packet_type = PacketType::try_from(bytes[0])?;
        let packet_count = bytes[1];

        // byte 2 [7:4] = superframe_idx
        let superframe_idx = (bytes[2] >> 4) & 0x0F;

        // byte 2 [3] = rfu — spec says must be 0
        if bytes[2] & 0x08 != 0 {
            return Err(Error::ReservedBitsViolation {
                field: "byte 2 bit 3",
                reason: "RFU must be zero (ETSI TS 102 773 §5.1)",
            });
        }

        // byte 2 [2:0] = t2mi_stream_id
        let t2mi_stream_id = bytes[2] & 0x07;

        // byte 3 = rfu — spec says must be 0
        if bytes[3] != 0 {
            return Err(Error::ReservedBitsViolation {
                field: "byte 3",
                reason: "All 8 RFU bits must be zero (ETSI TS 102 773 §5.1)",
            });
        }

        let payload_len_bits = u16::from_be_bytes([bytes[4], bytes[5]]);

        Ok(Header {
            packet_type,
            packet_count,
            superframe_idx,
            t2mi_stream_id,
            payload_len_bits,
        })
    }
}

impl Header {
    /// Compute payload length in bytes (ceil(bits / 8)).
    #[must_use]
    pub fn payload_len_bytes(&self) -> usize {
        (self.payload_len_bits as usize).div_ceil(8)
    }

    /// Total packet size including header + payload + CRC-32 trailer, in bytes.
    ///
    /// = 6 (header) + ceil(payload_len_bits / 8) (payload + padding) + 4 (CRC).
    #[must_use]
    pub fn total_bytes(&self) -> usize {
        6 + self.payload_len_bytes() + super::crc::CRC_LEN
    }
}

impl dvb_common::Serialize for Header {
    type Error = crate::error::Error;

    fn serialized_len(&self) -> usize {
        6
    }

    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize, crate::error::Error> {
        use super::error::Error;

        if buf.len() < 6 {
            return Err(Error::OutputBufferTooSmall {
                need: 6,
                have: buf.len(),
            });
        }

        if self.t2mi_stream_id > 7 {
            return Err(Error::ReservedBitsViolation {
                field: "t2mi_stream_id",
                reason: "Must be in range 0..=7 (3-bit field)",
            });
        }

        buf[0] = self.packet_type.into();
        buf[1] = self.packet_count;
        buf[2] = (self.superframe_idx & 0x0F) << 4 | (self.t2mi_stream_id & 0x07);
        buf[3] = 0; // RFU = 0
        let len_be = self.payload_len_bits.to_be_bytes();
        buf[4] = len_be[0];
        buf[5] = len_be[1];

        Ok(6)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use dvb_common::{Parse, Serialize};

    #[test]
    fn packet_type_try_from_all_valid() {
        // Table 1 — every defined type must round-trip
        let valid_types = [
            0x00, 0x01, 0x02, 0x10, 0x11, 0x12, 0x20, 0x21, 0x30, 0x31, 0x32, 0x33,
        ];
        for v in valid_types {
            let result = PacketType::try_from(v);
            assert!(result.is_ok(), "PacketType::try_from({:#04x}) failed", v);
        }
    }

    #[test]
    fn packet_type_rejects_reserved() {
        // 0x22..=0x2F and 0x34..=0xFF are RFU
        for v in 0x22..=0x2F {
            assert!(
                PacketType::try_from(v).is_err(),
                "0x{v:02x} should be rejected"
            );
        }
        for v in 0x34..=0xFF {
            assert!(
                PacketType::try_from(v).is_err(),
                "0x{v:02x} should be rejected"
            );
        }
    }

    #[test]
    fn exhaustive_byte_sweep() {
        let mut matched = 0u16;
        for byte in 0u8..=0xFF {
            if let Ok(v) = PacketType::try_from(byte) {
                assert_eq!(v as u8, byte, "round-trip failed for {byte:#04x}");
                matched += 1;
            }
        }
        assert_eq!(matched, 12, "expected 12 matched variants");
    }

    #[test]
    fn parse_rejects_buffer_shorter_than_6() {
        let buf = [0x00u8; 5];
        let result = Header::parse(&buf);
        assert!(result.is_err());
        let err = result.unwrap_err();
        if let crate::Error::BufferTooShort { need, have, what } = err {
            assert_eq!(need, 6);
            assert_eq!(have, 5);
            assert_eq!(what, "T2MI Header");
        } else {
            panic!("Expected BufferTooShort, got {:?}", err);
        }
    }

    #[test]
    fn parse_extracts_packet_type_and_count() {
        let buf = [0x10u8, 0xAB, 0x00, 0x00, 0x00, 0x08];
        let hdr = Header::parse(&buf).unwrap();
        assert_eq!(hdr.packet_type, PacketType::L1Current);
        assert_eq!(hdr.packet_count, 0xAB);
    }

    #[test]
    fn parse_extracts_superframe_idx() {
        let buf = [0x00u8, 0x00, 0x50, 0x00, 0x00, 0x08];
        let hdr = Header::parse(&buf).unwrap();
        assert_eq!(hdr.superframe_idx, 5);
    }

    #[test]
    fn parse_accepts_all_defined_packet_types() {
        let types = [
            0x00u8, 0x01, 0x02, 0x10, 0x11, 0x12, 0x20, 0x21, 0x30, 0x31, 0x32, 0x33,
        ];
        for &t in &types {
            let buf = [t, 0x00, 0x00, 0x00, 0x00, 0x08];
            let result = Header::parse(&buf);
            assert!(
                result.is_ok(),
                "parse failed for packet_type {:#04x}: {:?}",
                t,
                result
            );
            assert_eq!(
                result.unwrap().packet_type,
                PacketType::try_from(t).unwrap()
            );
        }
    }

    #[test]
    fn parse_rejects_reserved_packet_type_0x22() {
        let buf = [0x22u8, 0x00, 0x00, 0x00, 0x00, 0x08];
        let result = Header::parse(&buf);
        assert!(result.is_err());
        assert!(matches!(
            result.unwrap_err(),
            crate::Error::InvalidPacketType { found: 0x22 }
        ));
    }

    #[test]
    fn parse_extracts_t2mi_stream_id_0_through_7() {
        for id in 0..=7 {
            let buf = [0x00u8, 0x00, id, 0x00, 0x00, 0x08];
            let hdr = Header::parse(&buf).unwrap();
            assert_eq!(hdr.t2mi_stream_id, id, "stream_id mismatch for id={}", id);
        }
    }

    #[test]
    fn parse_rejects_nonzero_rfu_bits_in_byte2() {
        // byte 2 bit 3 (0x08) is rfu — must be 0
        let buf = [0x00u8, 0x00, 0x08, 0x00, 0x00, 0x08];
        let result = Header::parse(&buf);
        assert!(result.is_err());
        assert!(matches!(
            result.unwrap_err(),
            crate::Error::ReservedBitsViolation { .. }
        ));
    }

    #[test]
    fn parse_rejects_nonzero_byte3() {
        let buf = [0x00u8, 0x00, 0x00, 0x01, 0x00, 0x08];
        let result = Header::parse(&buf);
        assert!(result.is_err());
    }

    #[test]
    fn parse_extracts_payload_len_bits() {
        let buf = [0x00u8, 0x00, 0x00, 0x00, 0x01, 0x00];
        let hdr = Header::parse(&buf).unwrap();
        assert_eq!(hdr.payload_len_bits, 0x0100);
    }

    #[test]
    fn payload_len_bytes_rounds_up() {
        let hdr = Header {
            packet_type: PacketType::BasebandFrame,
            packet_count: 0,
            superframe_idx: 0,
            t2mi_stream_id: 0,
            payload_len_bits: 13,
        };
        assert_eq!(hdr.payload_len_bytes(), 2); // ceil(13/8) = 2

        let hdr2 = Header {
            payload_len_bits: 16,
            ..hdr
        };
        assert_eq!(hdr2.payload_len_bytes(), 2);

        let hdr3 = Header {
            payload_len_bits: 0,
            ..hdr
        };
        assert_eq!(hdr3.payload_len_bytes(), 0);
    }

    // ── Serialize tests ──

    #[test]
    fn serialize_writes_6_bytes() {
        let hdr = Header {
            packet_type: PacketType::BasebandFrame,
            packet_count: 42,
            superframe_idx: 7,
            t2mi_stream_id: 3,
            payload_len_bits: 128,
        };
        let mut buf = [0u8; 256];
        let written = hdr.serialize_into(&mut buf).unwrap();
        assert_eq!(written, 6);
        assert_eq!(buf[0], 0x00);
        assert_eq!(buf[1], 42);
        assert_eq!(buf[2], (7 << 4) | 3);
        assert_eq!(buf[3], 0);
        assert_eq!(buf[4], 0);
        assert_eq!(buf[5], 128);
    }

    #[test]
    fn serialize_round_trip_identity_for_every_packet_type() {
        let types = [
            0x00u8, 0x01, 0x02, 0x10, 0x11, 0x12, 0x20, 0x21, 0x30, 0x31, 0x32, 0x33,
        ];
        for &t in &types {
            let original = Header {
                packet_type: PacketType::try_from(t).unwrap(),
                packet_count: 13,
                superframe_idx: 7,
                t2mi_stream_id: 2,
                payload_len_bits: 512,
            };
            let mut buf = [0u8; 6];
            original.serialize_into(&mut buf).unwrap();
            let parsed = Header::parse(&buf).unwrap();
            assert_eq!(original, parsed, "Round-trip failed for type {:#04x}", t);
        }
    }

    #[test]
    fn serialize_rejects_too_small_buffer() {
        let hdr = Header {
            packet_type: PacketType::BasebandFrame,
            packet_count: 0,
            superframe_idx: 0,
            t2mi_stream_id: 0,
            payload_len_bits: 0,
        };
        let mut buf = [0u8; 5];
        let result = hdr.serialize_into(&mut buf);
        assert!(result.is_err());
    }

    #[test]
    fn serialize_rejects_t2mi_stream_id_above_7() {
        let hdr = Header {
            packet_type: PacketType::BasebandFrame,
            packet_count: 0,
            superframe_idx: 0,
            t2mi_stream_id: 8,
            payload_len_bits: 0,
        };
        let mut buf = [0u8; 6];
        let result = hdr.serialize_into(&mut buf);
        assert!(result.is_err());
    }
}
