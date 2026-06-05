//! T2-MI payload type 0x33: FEF sub-part — §5.2.12.
//!
//! Carries individual sub-parts of a composite FEF part.
//! Sub-part variety determines the body format:
//! - 0x0000: Null (reserved_for_future_use 32 bits)
//! - 0x0001: IQ (reserved 32 bits + iq_data variable)
//! - 0x0002: PRBS (prbs_type 8 bits + reserved 96 bits)
//! - 0x0003: TX-SIG FEF (reserved 32 bits)
//! - 0x0004..0xFFFF: Reserved for future use

use std::fmt;

use num_enum::TryFromPrimitive;

use dvb_common::{Parse, Serialize};

/// Sub-part variety per §5.2.12 Table 13.
#[derive(Debug, Clone, Copy, PartialEq, Eq, TryFromPrimitive)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[repr(u16)]
pub enum SubpartVariety {
    /// Null — `reserved_for_future_use(32)` = 0.
    Null = 0x0000,
    /// IQ — `reserved_for_future_use(32)` + `iq_data`.
    Iq = 0x0001,
    /// PRBS — `prbs_type(8)` + `reserved_for_future_use(96)`.
    Prbs = 0x0002,
    /// TX-SIG FEF — `reserved_for_future_use(32)`.
    TxSigFef = 0x0003,
}

impl From<SubpartVariety> for u16 {
    fn from(sv: SubpartVariety) -> Self {
        sv as u16
    }
}

impl From<num_enum::TryFromPrimitiveError<SubpartVariety>> for crate::error::Error {
    fn from(_: num_enum::TryFromPrimitiveError<SubpartVariety>) -> Self {
        // subpart_variety is a 16-bit field — casting the offending value to u8
        // would truncate it, and InvalidPacketType is the wrong category anyway.
        crate::error::Error::ReservedBitsViolation {
            field: "subpart_variety",
            reason: "Must be 0x0000..=0x0003 per ETSI TS 102 773 §5.2.12 Table 13",
        }
    }
}

/// PRBS type for SubpartVariety::Prbs.
#[derive(Debug, Clone, Copy, PartialEq, Eq, TryFromPrimitive)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[repr(u8)]
pub enum PrbsType {
    /// User-defined test/measurement.
    UserDefined = 0x00,
}

impl From<PrbsType> for u8 {
    fn from(pt: PrbsType) -> Self {
        pt as u8
    }
}

/// FEF sub-part payload (type 0x33) per ETSI TS 102 773 §5.2.12.
///
/// Layout (Figure 16):
/// - byte 0: fef_idx (8 bits)
/// - bytes 1-2: tx_identifier (16 bits)
/// - bytes 3-6: rfu1 (32 bits) — must be 0
/// - bytes 7-8: subpart_idx (16 bits)
/// - bytes 9-10: subpart_variety (16 bits)
/// - bytes 11-14: rfu2 (10 bits) + subpart_length (22 bits)
/// - bytes 15..: subpart data (variable, format per variety)
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct FefSubPartPayload<'a> {
    /// FEF index within super-frame.
    pub fef_idx: u8,
    /// Transmitter identifier (0x0000 = broadcast).
    pub tx_identifier: u16,
    /// Sub-part index p.
    pub subpart_idx: u16,
    /// Sub-part variety (Null/IQ/PRBS/TX-SIG).
    pub subpart_variety: SubpartVariety,
    /// Length in elementary time periods.
    pub subpart_length: u32,
    /// Raw sub-part data (format depends on variety).
    #[cfg_attr(feature = "serde", serde(borrow))]
    pub subpart_data: &'a [u8],
}

/// Total header bytes before subpart_data: fef_idx(1) + tx_id(2) + rfu1(4) +
/// subpart_idx(2) + variety(2) + rfu2+subpart_len(4) = 15.
const FEF_SUBPART_HEADER_LEN: usize = 15;

/// Mask for 22-bit subpart_length field.
const SUBPART_LENGTH_MASK: u32 = 0x003F_FFFF;

impl<'a> Parse<'a> for FefSubPartPayload<'a> {
    type Error = crate::error::Error;

    fn parse(bytes: &'a [u8]) -> Result<Self, crate::error::Error> {
        if bytes.len() < FEF_SUBPART_HEADER_LEN {
            return Err(crate::Error::BufferTooShort {
                need: FEF_SUBPART_HEADER_LEN,
                have: bytes.len(),
                what: "FefSubPartPayload header",
            });
        }

        // rfu1: bytes 3-6 (32 bits) — must be 0
        if bytes[3] != 0 || bytes[4] != 0 || bytes[5] != 0 || bytes[6] != 0 {
            return Err(crate::Error::ReservedBitsViolation {
                field: "32-bit RFU (rfu1)",
                reason: "Must be zero (ETSI TS 102 773 §5.2.12)",
            });
        }

        let variety = SubpartVariety::try_from(u16::from_be_bytes([bytes[9], bytes[10]]))?;

        // rfu2: byte 11 (8 bits) + byte 12 [7:6] (2 bits) = 10 bits — must be 0
        if bytes[11] != 0 || (bytes[12] & 0xC0 != 0) {
            return Err(crate::Error::ReservedBitsViolation {
                field: "10-bit RFU (rfu2)",
                reason: "Must be zero (ETSI TS 102 773 §5.2.12)",
            });
        }

        // subpart_length: byte 12 [5:0] (6 bits) + byte 13 (8 bits) + byte 14 (8 bits) = 22 bits
        let subpart_length =
            ((bytes[12] & 0x3F) as u32) << 16 | (bytes[13] as u32) << 8 | (bytes[14] as u32);

        Ok(FefSubPartPayload {
            fef_idx: bytes[0],
            tx_identifier: u16::from_be_bytes([bytes[1], bytes[2]]),
            subpart_idx: u16::from_be_bytes([bytes[7], bytes[8]]),
            subpart_variety: variety,
            subpart_length,
            subpart_data: &bytes[FEF_SUBPART_HEADER_LEN..],
        })
    }
}

impl<'a> crate::traits::PayloadDef<'a> for FefSubPartPayload<'a> {
    const PACKET_TYPE: u8 = 0x33;
    const NAME: &'static str = "FEF_SUBPART";
}

impl Serialize for FefSubPartPayload<'_> {
    type Error = crate::error::Error;

    fn serialized_len(&self) -> usize {
        FEF_SUBPART_HEADER_LEN + self.subpart_data.len()
    }

    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize, crate::error::Error> {
        if buf.len() < self.serialized_len() {
            return Err(crate::Error::OutputBufferTooSmall {
                need: self.serialized_len(),
                have: buf.len(),
            });
        }

        if self.subpart_length > SUBPART_LENGTH_MASK {
            return Err(crate::Error::ReservedBitsViolation {
                field: "subpart_length",
                reason: "Must fit in 22 bits",
            });
        }

        buf[0] = self.fef_idx;
        let tx_id = self.tx_identifier.to_be_bytes();
        buf[1] = tx_id[0];
        buf[2] = tx_id[1];
        // rfu1 = 0
        buf[3] = 0;
        buf[4] = 0;
        buf[5] = 0;
        buf[6] = 0;
        let sub_idx = self.subpart_idx.to_be_bytes();
        buf[7] = sub_idx[0];
        buf[8] = sub_idx[1];
        let variety = u16::from(self.subpart_variety);
        buf[9] = (variety >> 8) as u8;
        buf[10] = (variety & 0xFF) as u8;
        // rfu2 = 0 (top 2 bits byte 12) + rfu2 = 0 (byte 11)
        buf[11] = 0;
        // subpart_length: 22 bits → byte 12 [5:0] = top 6, byte 13 = mid 8, byte 14 = bot 8
        buf[12] = ((self.subpart_length >> 16) & 0x3F) as u8;
        buf[13] = (self.subpart_length >> 8) as u8;
        buf[14] = (self.subpart_length & 0xFF) as u8;

        if !self.subpart_data.is_empty() {
            buf[FEF_SUBPART_HEADER_LEN..FEF_SUBPART_HEADER_LEN + self.subpart_data.len()]
                .copy_from_slice(self.subpart_data);
        }

        Ok(self.serialized_len())
    }
}

impl fmt::Display for FefSubPartPayload<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "FEF SubPart {{ fef_idx: {}, tx_id: 0x{:04X}, subpart_idx: {}, variety: {:?}, length: {} }}",
            self.fef_idx, self.tx_identifier, self.subpart_idx, self.subpart_variety, self.subpart_length
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_extracts_fields() {
        // subpart_length = 4096 = 0x1000.
        // 22-bit: byte 12 [5:0] = (0x1000 >> 16) & 0x3F = 0,
        //         byte 13 = (0x1000 >> 8) & 0xFF = 0x10,
        //         byte 14 = 0x1000 & 0xFF = 0x00.
        let mut buf = [0u8; 15];
        buf[0] = 0x02; // fef_idx
        buf[1] = 0x00;
        buf[2] = 0x01; // tx_id = 1
                       // bytes 3-6 = rfu1 = 0
        buf[7] = 0x00;
        buf[8] = 0x05; // subpart_idx = 5
        buf[9] = 0x00;
        buf[10] = 0x01; // variety = IQ
                        // byte 11 = rfu2 = 0
                        // byte 12 top 2 = rfu2 = 0, bottom 6 = subpart top = 0
                        // byte 13 = subpart mid = 0x10
        buf[13] = 0x10;
        // byte 14 = subpart bot = 0
        // subpart_data starts at offset 15

        let result = FefSubPartPayload::parse(&buf).unwrap();
        assert_eq!(result.fef_idx, 2);
        assert_eq!(result.tx_identifier, 0x0001);
        assert_eq!(result.subpart_idx, 5);
        assert_eq!(result.subpart_variety, SubpartVariety::Iq);
        assert_eq!(result.subpart_length, 0x1000);
    }

    #[test]
    fn parse_rejects_invalid_variety() {
        let mut buf = [0u8; 15];
        buf[9] = 0x00;
        buf[10] = 0x04; // variety = 0x0004 (reserved)
        assert!(FefSubPartPayload::parse(&buf).is_err());
    }

    #[test]
    fn parse_rejects_nonzero_rfu1() {
        let mut buf = [0u8; 15];
        buf[3] = 0x01;
        assert!(FefSubPartPayload::parse(&buf).is_err());
    }

    #[test]
    fn parse_rejects_nonzero_rfu2_byte11() {
        let mut buf = [0u8; 15];
        buf[11] = 0x01;
        assert!(FefSubPartPayload::parse(&buf).is_err());
    }

    #[test]
    fn parse_rejects_nonzero_rfu2_byte12_top() {
        let mut buf = [0u8; 15];
        buf[12] = 0xC0;
        assert!(FefSubPartPayload::parse(&buf).is_err());
    }

    #[test]
    fn parse_rejects_short_buffer() {
        let buf = [0u8; 14];
        assert!(FefSubPartPayload::parse(&buf).is_err());
    }

    #[test]
    fn serialize_round_trip() {
        let orig = FefSubPartPayload {
            fef_idx: 1,
            tx_identifier: 0x0000,
            subpart_idx: 3,
            subpart_variety: SubpartVariety::Null,
            subpart_length: 2048,
            subpart_data: &[],
        };
        let mut buf = vec![0u8; orig.serialized_len()];
        orig.serialize_into(&mut buf).unwrap();
        let parsed = FefSubPartPayload::parse(&buf).unwrap();
        assert_eq!(orig, parsed);
    }

    #[test]
    fn broadcast_tx_identifier() {
        let buf = [0u8; 15];
        let result = FefSubPartPayload::parse(&buf).unwrap();
        assert_eq!(result.tx_identifier, 0x0000);
    }

    #[test]
    fn subpart_variety_try_from_all() {
        assert_eq!(SubpartVariety::try_from(0x0000), Ok(SubpartVariety::Null));
        assert_eq!(SubpartVariety::try_from(0x0001), Ok(SubpartVariety::Iq));
        assert_eq!(SubpartVariety::try_from(0x0002), Ok(SubpartVariety::Prbs));
        assert_eq!(
            SubpartVariety::try_from(0x0003),
            Ok(SubpartVariety::TxSigFef)
        );
    }

    #[test]
    fn subpart_variety_try_from_reserved() {
        assert!(SubpartVariety::try_from(0x0004).is_err());
        assert!(SubpartVariety::try_from(0xFFFF).is_err());
    }

    #[test]
    fn exhaustive_subpart_variety_sweep() {
        let mut matched = 0u32;
        for value in 0u16..=0xFFFF {
            if let Ok(v) = SubpartVariety::try_from(value) {
                assert_eq!(v as u16, value, "round-trip failed for {value:#06x}");
                matched += 1;
            }
        }
        assert_eq!(matched, 4, "expected 4 matched variants");
    }

    #[test]
    fn exhaustive_prbs_type_sweep() {
        let mut matched = 0u16;
        for byte in 0u8..=0xFF {
            if let Ok(v) = PrbsType::try_from(byte) {
                assert_eq!(v as u8, byte, "round-trip failed for {byte:#04x}");
                matched += 1;
            }
        }
        assert_eq!(matched, 1, "expected 1 matched variant");
    }
}
