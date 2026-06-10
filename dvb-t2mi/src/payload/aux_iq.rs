//! T2-MI payload type 0x01: Auxiliary stream I/Q data — §5.2.2.
//!
//! Carries complex I/Q samples for auxiliary streams at `1/T` symbol rate.
//! Each sample: 12-bit two's complement I, then 12-bit two's complement Q.
//! `Re(x) = I / 2^9`, `Im(x) = Q / 2^9`. Can span multiple T2-MI packets.

use dvb_common::{Parse, Serialize};

/// Minimum valid aux_id value.
const AUX_ID_MIN: u8 = 1;
/// Maximum valid aux_id value (4-bit field).
const AUX_ID_MAX: u8 = 0x0F;

/// Auxiliary I/Q payload (type 0x01) per ETSI TS 102 773 §5.2.2.
///
/// Layout:
/// - byte 0: frame_idx (8 bits)
/// - byte 1 \[7:4\]: aux_id (4 bits), range 1..=15
/// - byte 1 \[3:0\] + byte 2 \[7:0\]: rfu (12 bits), must be 0
/// - bytes 3..: aux_stream_data (variable, 12-bit I + 12-bit Q samples)
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[cfg_attr(feature = "yoke", derive(yoke::Yokeable))]
pub struct AuxIqPayload<'a> {
    /// FRAME_IDX of the T2 frame.
    pub frame_idx: u8,
    /// Auxiliary stream identifier (1..=15).
    pub aux_id: u8,
    /// Raw auxiliary stream data bytes (I/Q pairs).
    pub aux_stream_data: &'a [u8],
}

impl<'a> Parse<'a> for AuxIqPayload<'a> {
    type Error = crate::error::Error;

    fn parse(bytes: &'a [u8]) -> Result<Self, crate::error::Error> {
        if bytes.len() < 3 {
            return Err(crate::Error::BufferTooShort {
                need: 3,
                have: bytes.len(),
                what: "AuxIqPayload header",
            });
        }

        let frame_idx = bytes[0];
        let aux_id = (bytes[1] >> 4) & 0x0F;

        if !(AUX_ID_MIN..=AUX_ID_MAX).contains(&aux_id) {
            return Err(crate::Error::ReservedBitsViolation {
                field: "aux_id",
                reason: "aux_id out of range 1..=15 (ETSI TS 102 773 §5.2.2)",
            });
        }

        // rfu: byte 1 [3:0] + byte 2 [7:0] = 12 bits
        let rfu = ((bytes[1] & 0x0F) as u16) << 8 | (bytes[2] as u16);
        if rfu != 0 {
            return Err(crate::Error::ReservedBitsViolation {
                field: "byte 1 [3:0] + byte 2",
                reason: "12-bit RFU must be zero (ETSI TS 102 773 §5.2.2)",
            });
        }

        Ok(AuxIqPayload {
            frame_idx,
            aux_id,
            aux_stream_data: &bytes[3..],
        })
    }
}

impl<'a> crate::traits::PayloadDef<'a> for AuxIqPayload<'a> {
    const PACKET_TYPE: u8 = 0x01;
    const NAME: &'static str = "AUX_IQ";
}

impl Serialize for AuxIqPayload<'_> {
    type Error = crate::error::Error;

    fn serialized_len(&self) -> usize {
        3 + self.aux_stream_data.len()
    }

    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize, crate::error::Error> {
        if buf.len() < self.serialized_len() {
            return Err(crate::Error::OutputBufferTooSmall {
                need: self.serialized_len(),
                have: buf.len(),
            });
        }

        if !(AUX_ID_MIN..=AUX_ID_MAX).contains(&self.aux_id) {
            return Err(crate::Error::ReservedBitsViolation {
                field: "aux_id",
                reason: "aux_id out of range 1..=15 (ETSI TS 102 773 §5.2.2)",
            });
        }

        buf[0] = self.frame_idx;
        buf[1] = (self.aux_id & 0x0F) << 4; // lower 4 bits of aux_id, upper 4 of header
        buf[2] = 0; // rfu = 0
        buf[3..3 + self.aux_stream_data.len()].copy_from_slice(self.aux_stream_data);

        Ok(self.serialized_len())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_extracts_frame_idx_and_aux_id() {
        let buf = [0x42u8, 0x70, 0x00, 0xCA, 0xFE];
        let result = AuxIqPayload::parse(&buf).unwrap();
        assert_eq!(result.frame_idx, 0x42);
        assert_eq!(result.aux_id, 0x07);
    }

    #[test]
    fn parse_rejects_aux_id_zero() {
        let buf = [0x00u8, 0x00, 0x00, 0xCA];
        let result = AuxIqPayload::parse(&buf);
        assert!(result.is_err());
    }

    #[test]
    fn parse_accepts_max_aux_id() {
        let buf = [0x00u8, 0xF0, 0x00, 0xCA];
        let result = AuxIqPayload::parse(&buf).unwrap();
        assert_eq!(result.aux_id, 0x0F);
    }

    #[test]
    fn parse_rejects_nonzero_rfu_bits() {
        let buf = [0x00u8, 0x1F, 0xFF, 0xCA];
        // rfu = 0x0F_F0 != 0 → reject
        let result = AuxIqPayload::parse(&buf);
        assert!(result.is_err());
    }

    #[test]
    fn parse_preserves_raw_aux_stream_data_bytes() {
        let data: Vec<u8> = (0..50).collect();
        let mut buf = vec![0x01u8, 0x50, 0x00];
        buf.extend_from_slice(&data);
        let result = AuxIqPayload::parse(&buf).unwrap();
        assert_eq!(result.aux_stream_data, data.as_slice());
    }

    #[test]
    fn parse_rejects_buffer_shorter_than_3() {
        assert!(AuxIqPayload::parse(&[0x00]).is_err());
        assert!(AuxIqPayload::parse(&[]).is_err());
    }

    #[test]
    fn serialize_round_trip() {
        let orig = AuxIqPayload {
            frame_idx: 0xAB,
            aux_id: 0x05,
            aux_stream_data: &[0x12, 0x34, 0x56, 0x78],
        };
        let mut buf = vec![0u8; orig.serialized_len()];
        orig.serialize_into(&mut buf).unwrap();
        let parsed = AuxIqPayload::parse(&buf).unwrap();
        assert_eq!(orig, parsed);
    }

    #[test]
    fn serialize_rejects_invalid_aux_id() {
        let payload = AuxIqPayload {
            frame_idx: 0x00,
            aux_id: 0x00, // must be 1..=15
            aux_stream_data: &[],
        };
        let mut buf = [0u8; 3];
        assert!(payload.serialize_into(&mut buf).is_err());
    }

    #[test]
    fn serialize_zeros_rfu_byte() {
        let payload = AuxIqPayload {
            frame_idx: 0x11,
            aux_id: 0x0A,
            aux_stream_data: &[],
        };
        let mut buf = [0xFFu8; 3];
        payload.serialize_into(&mut buf).unwrap();
        assert_eq!(buf[1] & 0x0F, 0x00);
        assert_eq!(buf[2], 0x00);
    }
}
