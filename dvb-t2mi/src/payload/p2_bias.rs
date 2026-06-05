//! T2-MI payload type 0x12: P2 bias balancing cells — §5.2.6.

use dvb_common::{Parse, Serialize};

/// P2 bias balancing payload (type 0x12), ETSI TS 102 773 §5.2.6 Figure 9.
///
/// Layout: `frame_idx(8) | rfu(17) | num_active_bias_cells_per_p2(15)` = 40 bits
/// = 5 bytes.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct P2BiasPayload {
    /// FRAME_IDX of the T2 frame carrying the bias balancing cells.
    pub frame_idx: u8,
    /// Number of active bias balancing cells per P2 symbol (15-bit).
    pub num_active_bias_cells_per_p2: u16,
}

const P2_BIAS_HEADER_LEN: usize = 5;

impl<'a> Parse<'a> for P2BiasPayload {
    type Error = crate::error::Error;

    fn parse(bytes: &'a [u8]) -> Result<Self, crate::error::Error> {
        if bytes.len() < P2_BIAS_HEADER_LEN {
            return Err(crate::Error::BufferTooShort {
                need: P2_BIAS_HEADER_LEN,
                have: bytes.len(),
                what: "P2BiasPayload header",
            });
        }

        // rfu is 17 bits: all of bytes[1], bytes[2], and the top bit of bytes[3].
        if bytes[1] != 0 || bytes[2] != 0 || (bytes[3] & 0x80) != 0 {
            return Err(crate::Error::ReservedBitsViolation {
                field: "17-bit RFU",
                reason: "Must be zero (ETSI TS 102 773 §5.2.6)",
            });
        }

        // num_active_bias_cells_per_p2 is 15 bits: bottom 7 of bytes[3] + bytes[4].
        let num_active = ((bytes[3] as u16 & 0x7F) << 8) | (bytes[4] as u16);

        Ok(P2BiasPayload {
            frame_idx: bytes[0],
            num_active_bias_cells_per_p2: num_active,
        })
    }
}

impl<'a> crate::traits::PayloadDef<'a> for P2BiasPayload {
    const PACKET_TYPE: u8 = 0x12;
    const NAME: &'static str = "P2_BIAS";
}

impl Serialize for P2BiasPayload {
    type Error = crate::error::Error;

    fn serialized_len(&self) -> usize {
        P2_BIAS_HEADER_LEN
    }

    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize, crate::error::Error> {
        if buf.len() < self.serialized_len() {
            return Err(crate::Error::OutputBufferTooSmall {
                need: self.serialized_len(),
                have: buf.len(),
            });
        }

        if self.num_active_bias_cells_per_p2 > 0x7FFF {
            return Err(crate::Error::ReservedBitsViolation {
                field: "num_active_bias_cells_per_p2",
                reason: "Must fit in 15 bits",
            });
        }

        buf[0] = self.frame_idx;
        buf[1] = 0; // rfu
        buf[2] = 0; // rfu
        buf[3] = ((self.num_active_bias_cells_per_p2 >> 8) as u8) & 0x7F;
        buf[4] = (self.num_active_bias_cells_per_p2 & 0xFF) as u8;

        Ok(self.serialized_len())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_extracts_fields() {
        // frame_idx=0x42, rfu=0, num_active=0x0F
        let buf = [0x42u8, 0x00, 0x00, 0x00, 0x0F];
        let result = P2BiasPayload::parse(&buf).unwrap();
        assert_eq!(result.frame_idx, 0x42);
        assert_eq!(result.num_active_bias_cells_per_p2, 0x0F);
    }

    #[test]
    fn parse_extracts_15bit_max() {
        // num_active = 0x7FFF (all 15 bits): bytes[3]=0x7F, bytes[4]=0xFF
        let buf = [0x00u8, 0x00, 0x00, 0x7F, 0xFF];
        let result = P2BiasPayload::parse(&buf).unwrap();
        assert_eq!(result.num_active_bias_cells_per_p2, 0x7FFF);
    }

    #[test]
    fn parse_rejects_nonzero_rfu() {
        // top bit of bytes[3] is part of the RFU
        let buf = [0x00u8, 0x00, 0x00, 0x80, 0x00];
        assert!(P2BiasPayload::parse(&buf).is_err());
        // a byte in the middle of the RFU span
        assert!(P2BiasPayload::parse(&[0x00u8, 0x01, 0x00, 0x00, 0x00]).is_err());
    }

    #[test]
    fn parse_rejects_short_buffer() {
        assert!(P2BiasPayload::parse(&[0x00, 0x00, 0x00, 0x00]).is_err());
    }

    #[test]
    fn serialize_round_trip() {
        let orig = P2BiasPayload {
            frame_idx: 5,
            num_active_bias_cells_per_p2: 3000,
        };
        let mut buf = [0u8; 5];
        orig.serialize_into(&mut buf).unwrap();
        let parsed = P2BiasPayload::parse(&buf).unwrap();
        assert_eq!(orig, parsed);
    }

    #[test]
    fn serialize_rejects_too_small_buffer() {
        let payload = P2BiasPayload {
            frame_idx: 0,
            num_active_bias_cells_per_p2: 0,
        };
        let mut buf = [0u8; 4];
        assert!(payload.serialize_into(&mut buf).is_err());
    }
}
