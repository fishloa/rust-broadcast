//! Discontinuity Information Table — ETSI EN 300 468 §7.1.2.
//!
//! Carried on PID `0x001E` with `table_id = 0x7E`, only in partial transport
//! streams (e.g. a recording). A short-form section whose body is a single
//! byte: `transition_flag(1) | reserved_future_use(7)`. No CRC.

use crate::error::{Error, Result};
use crate::traits::Table;
use dvb_common::{Parse, Serialize};

/// table_id for the Discontinuity Information Table.
pub const TABLE_ID: u8 = 0x7E;
/// Well-known PID on which the DIT is carried.
pub const PID: u16 = 0x001E;

const HEADER_LEN: usize = 3;
/// Body length: one byte holding `transition_flag` + reserved bits (§7.1.2).
const BODY_LEN: usize = 1;

/// Discontinuity Information Table (§7.1.2, Table 163).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct Dit {
    /// When set, a discontinuity in the transport stream occurs at this point.
    pub transition_flag: bool,
}

impl<'a> Parse<'a> for Dit {
    type Error = crate::error::Error;

    fn parse(bytes: &'a [u8]) -> Result<Self> {
        let min_len = HEADER_LEN + BODY_LEN;
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
        let section_length = ((bytes[1] & 0x0F) as usize) << 8 | bytes[2] as usize;
        if section_length != BODY_LEN {
            return Err(Error::SectionLengthOverflow {
                declared: section_length,
                available: BODY_LEN,
            });
        }
        // transition_flag is the top bit of the body byte; rest is reserved.
        let transition_flag = bytes[3] & 0x80 != 0;
        Ok(Dit { transition_flag })
    }
}

impl Serialize for Dit {
    type Error = crate::error::Error;

    fn serialized_len(&self) -> usize {
        HEADER_LEN + BODY_LEN
    }

    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        let len = self.serialized_len();
        if buf.len() < len {
            return Err(Error::OutputBufferTooSmall {
                need: len,
                have: buf.len(),
            });
        }
        buf[0] = TABLE_ID;
        // section_syntax_indicator=0 (short form), reserved_future_use=1,
        // reserved=11, section_length high nibble.
        buf[1] = 0x70 | ((BODY_LEN >> 8) as u8 & 0x0F);
        buf[2] = (BODY_LEN & 0xFF) as u8;
        // transition_flag in bit 7; remaining 7 bits reserved (set to 1).
        buf[3] = (u8::from(self.transition_flag) << 7) | 0x7F;
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
    fn parse_transition_flag_set() {
        // section_length=1, body byte 0x80 → transition_flag=1
        let bytes = [TABLE_ID, 0x70, 0x01, 0x80];
        let dit = Dit::parse(&bytes).unwrap();
        assert!(dit.transition_flag);
    }

    #[test]
    fn parse_transition_flag_clear() {
        let bytes = [TABLE_ID, 0x70, 0x01, 0x7F];
        let dit = Dit::parse(&bytes).unwrap();
        assert!(!dit.transition_flag);
    }

    #[test]
    fn parse_rejects_wrong_tag() {
        let bytes = [0x7F, 0x70, 0x01, 0x80];
        assert!(matches!(
            Dit::parse(&bytes).unwrap_err(),
            Error::UnexpectedTableId { table_id: 0x7F, .. }
        ));
    }

    #[test]
    fn parse_rejects_wrong_section_length() {
        let bytes = [TABLE_ID, 0x70, 0x02, 0x80, 0x00];
        assert!(matches!(
            Dit::parse(&bytes).unwrap_err(),
            Error::SectionLengthOverflow { .. }
        ));
    }

    #[test]
    fn parse_rejects_short_buffer() {
        let bytes = [TABLE_ID, 0x70];
        assert!(matches!(
            Dit::parse(&bytes).unwrap_err(),
            Error::BufferTooShort { .. }
        ));
    }

    #[test]
    fn serialize_round_trip_set() {
        let dit = Dit {
            transition_flag: true,
        };
        let mut buf = vec![0u8; dit.serialized_len()];
        dit.serialize_into(&mut buf).unwrap();
        assert_eq!(buf, [TABLE_ID, 0x70, 0x01, 0xFF]);
        assert_eq!(Dit::parse(&buf).unwrap(), dit);
    }

    #[test]
    fn serialize_round_trip_clear() {
        let dit = Dit {
            transition_flag: false,
        };
        let mut buf = vec![0u8; dit.serialized_len()];
        dit.serialize_into(&mut buf).unwrap();
        assert_eq!(buf, [TABLE_ID, 0x70, 0x01, 0x7F]);
        assert_eq!(Dit::parse(&buf).unwrap(), dit);
    }

    #[test]
    fn serialize_into_too_small_buffer() {
        let dit = Dit {
            transition_flag: false,
        };
        let mut buf = [0u8; 3];
        assert!(matches!(
            dit.serialize_into(&mut buf).unwrap_err(),
            Error::OutputBufferTooSmall { .. }
        ));
    }

    #[test]
    fn serialized_len_is_four() {
        assert_eq!(
            Dit {
                transition_flag: false
            }
            .serialized_len(),
            4
        );
    }

    #[test]
    fn serde_json_round_trip() {
        let dit = Dit {
            transition_flag: true,
        };
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
