//! T2-MI payload type 0x00: Baseband Frame (BBFRAME) — §5.2.1.

use dvb_common::{Parse, Serialize};

/// BBFrame payload (type 0x00) per ETSI TS 102 773 §5.2.1.
///
/// Layout:
/// - byte 0: frame_idx (8 bits) — FRAME_IDX of first T2 frame the IF is mapped to
/// - byte 1: plp_id (8 bits) — PLP_ID per EN 302 755
/// - byte 2 bit 7: intl_frame_start (1 bit) — 1 = first BBFRAME of an IF; 0 = subsequent
/// - byte 2 bits 6..0: rfu (7 bits) — must be 0
/// - bytes 3..: bbframe (Kbch bits) — encoded BBFRAME body
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[cfg_attr(feature = "yoke", derive(yoke::Yokeable))]
pub struct BbframePayload<'a> {
    /// FRAME_IDX of first T2 frame the IF is mapped to.
    pub frame_idx: u8,
    /// PLP_ID per EN 302 755.
    pub plp_id: u8,
    /// `true` = first BBFRAME of an interleaving frame.
    pub intl_frame_start: bool,
    /// The raw BBFrame data (Kbch bits = bytes when aligned).
    pub bbframe: &'a [u8],
}

impl<'a> Parse<'a> for BbframePayload<'a> {
    type Error = crate::error::Error;

    fn parse(bytes: &'a [u8]) -> Result<Self, crate::error::Error> {
        if bytes.len() < 3 {
            return Err(crate::Error::BufferTooShort {
                need: 3,
                have: bytes.len(),
                what: "BbframePayload header (frame_idx + plp_id + intl_frame_start + rfu)",
            });
        }

        let frame_idx = bytes[0];
        let plp_id = bytes[1];
        let intl_frame_start = bytes[2] & 0x80 != 0;

        // RFU: byte 2 bits 6..0 — must be zero
        if bytes[2] & 0x7F != 0 {
            return Err(crate::Error::ReservedBitsViolation {
                field: "byte 2 bits 6..0",
                reason: "RFU must be zero (ETSI TS 102 773 §5.2.1)",
            });
        }

        let bbframe = &bytes[3..];

        Ok(BbframePayload {
            frame_idx,
            plp_id,
            intl_frame_start,
            bbframe,
        })
    }
}

impl<'a> crate::traits::PayloadDef<'a> for BbframePayload<'a> {
    const PACKET_TYPE: u8 = 0x00;
    const NAME: &'static str = "BBFRAME";
}

impl Serialize for BbframePayload<'_> {
    type Error = crate::error::Error;

    fn serialized_len(&self) -> usize {
        3 + self.bbframe.len()
    }

    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize, crate::error::Error> {
        if buf.len() < self.serialized_len() {
            return Err(crate::Error::OutputBufferTooSmall {
                need: self.serialized_len(),
                have: buf.len(),
            });
        }

        buf[0] = self.frame_idx;
        buf[1] = self.plp_id;
        buf[2] = if self.intl_frame_start { 0x80 } else { 0x00 };
        if !self.bbframe.is_empty() {
            buf[3..3 + self.bbframe.len()].copy_from_slice(self.bbframe);
        }

        Ok(self.serialized_len())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_extracts_frame_idx_and_plp_id() {
        let buf = [0x42u8, 0x05, 0x00, 0xDE, 0xAD, 0xBE, 0xEF];
        let result = BbframePayload::parse(&buf).unwrap();
        assert_eq!(result.frame_idx, 0x42);
        assert_eq!(result.plp_id, 0x05);
        assert_eq!(result.bbframe, &[0xDE, 0xAD, 0xBE, 0xEF]);
    }

    #[test]
    fn parse_extracts_intl_frame_start_flag() {
        let buf = [0x00u8, 0x00, 0x80];
        let result = BbframePayload::parse(&buf).unwrap();
        assert!(result.intl_frame_start);

        let buf2 = [0x00u8, 0x00, 0x00];
        let result2 = BbframePayload::parse(&buf2).unwrap();
        assert!(!result2.intl_frame_start);
    }

    #[test]
    fn parse_rejects_nonzero_rfu_bits() {
        let buf = [0x00u8, 0x00, 0x40];
        let result = BbframePayload::parse(&buf);
        assert!(result.is_err());
        assert!(matches!(
            result.unwrap_err(),
            crate::Error::ReservedBitsViolation { .. }
        ));
    }

    #[test]
    fn parse_bbframe_body_slices_from_offset_3() {
        let body: Vec<u8> = (0..100).collect();
        let mut buf = vec![0x0Fu8, 0x01, 0x00];
        buf.extend_from_slice(&body);
        let result = BbframePayload::parse(&buf).unwrap();
        assert_eq!(result.bbframe, body.as_slice());
    }

    #[test]
    fn parse_rejects_body_shorter_than_3_bytes() {
        assert!(BbframePayload::parse(&[0x00, 0x00]).is_err());
        assert!(BbframePayload::parse(&[]).is_err());
    }

    #[test]
    fn serialize_round_trip_preserves_all_fields() {
        let orig = BbframePayload {
            frame_idx: 0xAB,
            plp_id: 0x07,
            intl_frame_start: true,
            bbframe: &[0xCA, 0xFE, 0xBA, 0xBE],
        };
        let mut buf = vec![0u8; orig.serialized_len()];
        orig.serialize_into(&mut buf).unwrap();
        let parsed = BbframePayload::parse(&buf).unwrap();
        assert_eq!(orig, parsed);
    }

    #[test]
    fn serialize_zeros_rfu_bits() {
        let payload = BbframePayload {
            frame_idx: 0x00,
            plp_id: 0x00,
            intl_frame_start: false,
            bbframe: &[],
        };
        let mut buf = [0xFFu8; 3];
        payload.serialize_into(&mut buf).unwrap();
        assert_eq!(buf[0], 0x00);
        assert_eq!(buf[1], 0x00);
        assert_eq!(buf[2] & 0x7F, 0x00);
    }
}
