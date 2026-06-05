//! Stuffing Table — ETSI EN 300 468 §5.2.8.
//!
//! Short-form section on PID 0x0014 with table_id 0x72. Payload is stuffing
//! bytes used to invalidate/replace sections; per §5.2.8 each `data_byte`
//! "may take any value and has no meaning" (0xFF fill is common on real
//! transponders). No CRC.

use crate::error::{Error, Result};
use crate::traits::Table;
use dvb_common::{Parse, Serialize};

/// table_id for Stuffing Table.
pub const TABLE_ID: u8 = 0x72;
/// Well-known PID on which ST is carried (shared with TDT/TOT).
pub const PID: u16 = 0x0014;

const HEADER_LEN: usize = 3;

/// Stuffing Table.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct St {
    /// Raw stuffing bytes — any value, no meaning (§5.2.8).
    pub payload: Vec<u8>,
}

impl St {
    /// Construct a new ST table with the given stuffing bytes.
    #[inline]
    #[must_use]
    pub fn new(payload: Vec<u8>) -> Self {
        Self { payload }
    }

    /// Number of stuffing bytes in the payload.
    #[inline]
    #[must_use]
    pub fn len(&self) -> usize {
        self.payload.len()
    }

    /// Returns `true` if the payload contains no stuffing bytes.
    #[inline]
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.payload.is_empty()
    }
}

impl<'a> Parse<'a> for St {
    type Error = crate::error::Error;

    fn parse(bytes: &'a [u8]) -> Result<Self> {
        if bytes.len() < HEADER_LEN {
            return Err(Error::BufferTooShort {
                need: HEADER_LEN,
                have: bytes.len(),
                what: "St",
            });
        }

        if bytes[0] != TABLE_ID {
            return Err(Error::UnexpectedTableId {
                table_id: bytes[0],
                what: "St",
                expected: &[TABLE_ID],
            });
        }

        let section_length = ((bytes[1] & 0x0F) as u16) << 8 | bytes[2] as u16;
        let payload_len = section_length as usize;

        if bytes.len() < HEADER_LEN + payload_len {
            return Err(Error::BufferTooShort {
                need: HEADER_LEN + payload_len,
                have: bytes.len(),
                what: "St payload",
            });
        }

        // §5.2.8: data_byte "may take any value and has no meaning" — no
        // value constraint; preserve verbatim.
        let payload = &bytes[HEADER_LEN..HEADER_LEN + payload_len];
        Ok(Self {
            payload: payload.to_vec(),
        })
    }
}

impl Serialize for St {
    type Error = crate::error::Error;

    fn serialized_len(&self) -> usize {
        HEADER_LEN + self.payload.len()
    }

    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        let len = self.serialized_len();
        if buf.len() < len {
            return Err(Error::OutputBufferTooSmall {
                need: len,
                have: buf.len(),
            });
        }

        // Byte 0: table_id = 0x72
        buf[0] = TABLE_ID;

        // Byte 1: SSI=0, reserved_future_use='1', reserved='11', upper nibble of
        // section_length. Top nibble 0b0111 = 0x70, matching DIT/RST/TDT/TOT.
        buf[1] = 0x70 | ((self.payload.len() >> 8) as u8 & 0x0F);

        // Byte 2: section_length low byte
        buf[2] = (self.payload.len() & 0xFF) as u8;

        // Payload: stuffing bytes
        buf[HEADER_LEN..len].copy_from_slice(&self.payload);

        Ok(len)
    }
}

impl<'a> Table<'a> for St {
    const TABLE_ID: u8 = TABLE_ID;
    const PID: u16 = PID;
}

impl<'a> crate::traits::TableDef<'a> for St {
    const TABLE_ID_RANGES: &'static [(u8, u8)] = &[(TABLE_ID, TABLE_ID)];
    const NAME: &'static str = "STUFFING";
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Build a minimal ST section byte vector.
    fn make_st_section(payload: &[u8]) -> Vec<u8> {
        let mut buf = vec![TABLE_ID, 0x70, payload.len() as u8];
        buf.extend_from_slice(payload);
        buf
    }

    #[test]
    fn parse_rejects_wrong_tag() {
        let bytes = [0x71, 0x70, 0x02, 0x00, 0x00];
        assert!(matches!(
            St::parse(&bytes).unwrap_err(),
            Error::UnexpectedTableId { table_id: 0x71, .. }
        ));
    }

    #[test]
    fn parse_empty_payload() {
        let bytes = make_st_section(&[]);
        let st = St::parse(&bytes).unwrap();
        assert!(st.is_empty());
    }

    /// §5.2.8: data_byte "may take any value and has no meaning" — 0xFF fill
    /// (common on real transponders) must parse and round-trip.
    #[test]
    fn parse_accepts_any_data_byte_value() {
        let bytes = make_st_section(&[0x00, 0xFF, 0xAA]);
        let st = St::parse(&bytes).unwrap();
        assert_eq!(st.payload, vec![0x00, 0xFF, 0xAA]);
        let mut buf = vec![0u8; st.serialized_len()];
        st.serialize_into(&mut buf).unwrap();
        assert_eq!(buf, bytes);
    }

    #[test]
    fn serialize_writes_correct_header() {
        let st = St::new(vec![0x00, 0x00]);
        let mut buf = vec![0u8; st.serialized_len()];
        st.serialize_into(&mut buf).unwrap();
        assert_eq!(buf[0], TABLE_ID);
        assert_eq!(buf[1], 0x70 | ((2 >> 8) as u8 & 0x0F));
        assert_eq!(buf[2], 2);
    }

    #[test]
    fn serialize_empty_payload() {
        let st = St::new(vec![]);
        let mut buf = vec![0u8; st.serialized_len()];
        st.serialize_into(&mut buf).unwrap();
        assert_eq!(buf, [TABLE_ID, 0x70, 0x00]);
    }

    #[test]
    fn serialize_rejects_too_small_buffer() {
        let st = St::new(vec![0x00]);
        let mut too_small = vec![0u8; st.serialized_len() - 1];
        let err = st.serialize_into(&mut too_small).unwrap_err();
        assert!(matches!(err, Error::OutputBufferTooSmall { .. }));
    }

    #[test]
    fn round_trip_preserves_all_fields() {
        let st = St::new(vec![0x00, 0x00, 0x00]);
        let mut buf = vec![0u8; st.serialized_len()];
        st.serialize_into(&mut buf).unwrap();
        let re = St::parse(&buf).unwrap();
        assert_eq!(st, re);
    }

    #[test]
    fn round_trip_empty() {
        let st = St::new(vec![]);
        let mut buf = vec![0u8; st.serialized_len()];
        st.serialize_into(&mut buf).unwrap();
        let re = St::parse(&buf).unwrap();
        assert_eq!(st, re);
    }

    #[test]
    fn round_trip_many_stuffs() {
        let st = St::new(vec![0x00; 185]);
        let mut buf = vec![0u8; st.serialized_len()];
        st.serialize_into(&mut buf).unwrap();
        let re = St::parse(&buf).unwrap();
        assert_eq!(st, re);
    }

    #[test]
    fn parse_rejects_buffer_too_short() {
        let bytes = [0x72, 0x70]; // only 2 bytes
        assert!(matches!(
            St::parse(&bytes).unwrap_err(),
            Error::BufferTooShort { need: 3, .. }
        ));
    }

    #[test]
    fn parse_rejects_section_length_exceeds_buffer() {
        // section_length = 10 but only 2 payload bytes available after header
        let bytes = [0x72, 0x70, 10, 0x00, 0x00];
        assert!(matches!(
            St::parse(&bytes).unwrap_err(),
            Error::BufferTooShort {
                what: "St payload",
                ..
            }
        ));
    }

    #[test]
    fn table_trait_constants() {
        assert_eq!(<St as Table>::TABLE_ID, 0x72);
        assert_eq!(<St as Table>::PID, 0x0014);
    }

    #[test]
    fn serialized_len_matches_wire_size() {
        let st = St::new(vec![0x00; 50]);
        assert_eq!(st.serialized_len(), HEADER_LEN + 50);
    }

    #[test]
    fn to_bytes_produces_valid_section() {
        let st = St::new(vec![0x00, 0x00]);
        let bytes = st.to_bytes();
        assert_eq!(bytes[0], TABLE_ID);
        assert_eq!(bytes.len(), st.serialized_len());
    }

    #[test]
    fn len_and_is_empty() {
        let empty = St::new(vec![]);
        assert!(empty.is_empty());
        assert_eq!(empty.len(), 0);

        let filled = St::new(vec![0x00; 10]);
        assert!(!filled.is_empty());
        assert_eq!(filled.len(), 10);
    }

    #[test]
    fn new_constructor() {
        let st = St::new(vec![0x00]);
        assert_eq!(st.len(), 1);
    }
}
