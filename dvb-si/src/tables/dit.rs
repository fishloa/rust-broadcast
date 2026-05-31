//! Discontinuity Information Table — ETSI EN 300 468 §5.2.9.
//!
//! Carried on PID `0x001E` with `table_id = 0x7E`. Used in partial transport
//! streams to signal that a discontinuity has occurred (e.g. stream cut,
//! encoder restart). The table body is exactly 2 bytes: `discontinuity_counter`.

use crate::error::{Error, Result};
use crate::traits::Table;
use dvb_common::{Parse, Serialize};

/// table_id for Discontinuity Information Table.
pub const TABLE_ID: u8 = 0x7E;
/// Well-known PID on which DIT is carried.
pub const PID: u16 = 0x001E;

const HEADER_LEN: usize = 3;
const DISC_COUNTER_LEN: usize = 2;

/// Discontinuity Information Table.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct Dit {
    /// Discontinuity counter — increments on discontinuity.
    pub discontinuity_counter: u16,
}

impl<'a> Parse<'a> for Dit {
    type Error = crate::error::Error;

    fn parse(bytes: &'a [u8]) -> Result<Self> {
        let min_len = HEADER_LEN + DISC_COUNTER_LEN;
        if bytes.len() < min_len {
            return Err(Error::BufferTooShort {
                need: min_len,
                have: bytes.len(),
                what: "Dit",
            });
        }
        if bytes[0] != TABLE_ID {
            return Err(Error::UnexpectedTableId {
                table_id: bytes[0],
                what: "Dit",
                expected: &[TABLE_ID],
            });
        }
        let section_length = ((bytes[1] & 0x0F) as u16) << 8 | bytes[2] as u16;
        if section_length as usize != DISC_COUNTER_LEN {
            return Err(Error::InvalidDescriptor {
                tag: TABLE_ID,
                reason: "DIT section_length must equal 2",
            });
        }
        let discontinuity_counter = ((bytes[3] as u16) << 8) | bytes[4] as u16;
        Ok(Dit { discontinuity_counter })
    }
}

impl Serialize for Dit {
    type Error = crate::error::Error;

    fn serialized_len(&self) -> usize {
        HEADER_LEN + DISC_COUNTER_LEN
    }

    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        let len = self.serialized_len();
        if buf.len() < len {
            return Err(Error::OutputBufferTooSmall {
                need: len,
                have: buf.len(),
            });
        }
        let section_length = DISC_COUNTER_LEN as u16;
        buf[0] = TABLE_ID;
        // section_syntax_indicator=1 (bit 7), private_indicator=0 (bit 6)
        buf[1] = 0xB0 | ((section_length >> 8) as u8 & 0x0F);
        buf[2] = (section_length & 0xFF) as u8;
        buf[3] = (self.discontinuity_counter >> 8) as u8;
        buf[4] = self.discontinuity_counter as u8;
        Ok(len)
    }
}

impl<'a> Table<'a> for Dit {
    const TABLE_ID: u8 = TABLE_ID;
    const PID: u16 = PID;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_extracts_discontinuity_counter() {
        let bytes = [TABLE_ID, 0xB0, 0x02, 0xAB, 0xCD];
        let dit = Dit::parse(&bytes).unwrap();
        assert_eq!(dit.discontinuity_counter, 0xABCD);
    }

    #[test]
    fn parse_rejects_wrong_tag() {
        let bytes = [0x7F, 0xB0, 0x02, 0x00, 0x01];
        assert!(matches!(
            Dit::parse(&bytes).unwrap_err(),
            Error::UnexpectedTableId { table_id: 0x7F, .. }
        ));
    }

    #[test]
    fn parse_rejects_wrong_section_length() {
        let bytes = [TABLE_ID, 0xB0, 0x03, 0x00, 0x01];
        assert!(matches!(
            Dit::parse(&bytes).unwrap_err(),
            Error::InvalidDescriptor { .. }
        ));
    }

    #[test]
    fn parse_rejects_short_buffer() {
        let bytes = [TABLE_ID, 0xB0];
        assert!(matches!(
            Dit::parse(&bytes).unwrap_err(),
            Error::BufferTooShort { .. }
        ));
    }

    #[test]
    fn serialize_round_trip() {
        let dit = Dit { discontinuity_counter: 0x1234 };
        let mut buf = vec![0u8; dit.serialized_len()];
        dit.serialize_into(&mut buf).unwrap();
        let re = Dit::parse(&buf).unwrap();
        assert_eq!(dit, re);
    }

    #[test]
    fn serialize_zero_counter() {
        let dit = Dit { discontinuity_counter: 0 };
        let mut buf = vec![0u8; dit.serialized_len()];
        dit.serialize_into(&mut buf).unwrap();
        assert_eq!(buf, [TABLE_ID, 0xB0, 0x02, 0x00, 0x00]);
    }

    #[test]
    fn serialize_max_counter() {
        let dit = Dit { discontinuity_counter: 0xFFFF };
        let mut buf = vec![0u8; dit.serialized_len()];
        dit.serialize_into(&mut buf).unwrap();
        assert_eq!(buf, [TABLE_ID, 0xB0, 0x02, 0xFF, 0xFF]);
    }

    #[test]
    fn serialize_into_too_small_buffer() {
        let dit = Dit { discontinuity_counter: 0 };
        let mut buf = [0u8; 3];
        assert!(matches!(
            dit.serialize_into(&mut buf).unwrap_err(),
            Error::OutputBufferTooSmall { .. }
        ));
    }

    #[test]
    fn serialized_len_is_five() {
        let dit = Dit { discontinuity_counter: 0 };
        assert_eq!(dit.serialized_len(), 5);
    }

    #[test]
    fn serde_json_round_trip() {
        let dit = Dit { discontinuity_counter: 0xDEAD };
        let json = serde_json::to_string(&dit).unwrap();
        let restored: Dit = serde_json::from_str(&json).unwrap();
        assert_eq!(dit, restored);
    }

    #[test]
    fn serde_json_round_trip_zero() {
        let dit = Dit { discontinuity_counter: 0 };
        let json = serde_json::to_string(&dit).unwrap();
        let restored: Dit = serde_json::from_str(&json).unwrap();
        assert_eq!(dit, restored);
    }

    #[test]
    fn table_trait_constants() {
        assert_eq!(Dit::TABLE_ID, 0x7E);
        assert_eq!(Dit::PID, 0x001E);
    }
}
