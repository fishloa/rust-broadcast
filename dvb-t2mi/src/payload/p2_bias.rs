//! T2-MI payload type 0x12: P2 bias balancing cells — §5.2.6.

use dvb_common::{Parse, Serialize};

/// P2 bias balancing payload (type 0x12).
///
/// Layout: frame_idx(8) + rfu(2) + num_active_bias_cells_per_p2(15) = 25 bits.
/// Stored in 3 bytes with 7 bits of RFU padding added.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct P2BiasPayload {
    /// FRAME_IDX of T2 frame.
    pub frame_idx: u8,
    /// Number of active bias balancing cells per P2 symbol.
    pub num_active_bias_cells_per_p2: u16,
}

const P2_BIAS_HEADER_LEN: usize = 3;

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

        if bytes[1] & 0xC0 != 0 {
            return Err(crate::Error::ReservedBitsViolation {
                field: "2-bit RFU",
                reason: "Must be zero (ETSI TS 102 773 §5.2.6)",
            });
        }

        let num_active = ((bytes[1] as u16 & 0x3F) << 8) | (bytes[2] as u16);

        Ok(P2BiasPayload {
            frame_idx: bytes[0],
            num_active_bias_cells_per_p2: num_active,
        })
    }
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

        buf[0] = self.frame_idx;
        buf[1] = ((self.num_active_bias_cells_per_p2 >> 8) & 0x3F) as u8;
        buf[2] = (self.num_active_bias_cells_per_p2 & 0xFF) as u8;

        Ok(self.serialized_len())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_extracts_fields() {
        let buf = [0x42u8, 0x00, 0x0F];
        let result = P2BiasPayload::parse(&buf).unwrap();
        assert_eq!(result.frame_idx, 0x42);
        assert_eq!(result.num_active_bias_cells_per_p2, 0x0F);
    }

    #[test]
    fn parse_rejects_nonzero_rfu() {
        let buf = [0x00u8, 0xC0, 0x00];
        assert!(P2BiasPayload::parse(&buf).is_err());
    }

    #[test]
    fn parse_rejects_short_buffer() {
        assert!(P2BiasPayload::parse(&[0x00, 0x00]).is_err());
    }

    #[test]
    fn serialize_round_trip() {
        let orig = P2BiasPayload {
            frame_idx: 5,
            num_active_bias_cells_per_p2: 3000,
        };
        let mut buf = [0u8; 3];
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
        let mut buf = [0u8; 2];
        assert!(payload.serialize_into(&mut buf).is_err());
    }
}
