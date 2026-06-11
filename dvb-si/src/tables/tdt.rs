//! Time and Date Table — ETSI EN 300 468 §5.2.5.
//!
//! Short-form section on PID 0x0014 with table_id 0x70. Body is exactly
//! 5 bytes of UTC time (16-bit MJD + 24-bit BCD HHMMSS). No CRC.

use crate::error::{Error, Result};
use dvb_common::{Parse, Serialize};

/// table_id for Time and Date Table.
pub const TABLE_ID: u8 = 0x70;
/// Well-known PID on which TDT is carried.
pub const PID: u16 = 0x0014;

const HEADER_LEN: usize = 3;
const UTC_TIME_LEN: usize = 5;

/// Time and Date Table.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct TdtSection {
    /// Raw 5-byte UTC time (16-bit MJD + 24-bit BCD HHMMSS) per
    /// EN 300 468 Annex C. Private — use [`utc_time_decoded`](Self::utc_time_decoded)
    /// for decoded access.
    pub(crate) utc_time_raw: [u8; 5],
}

impl TdtSection {
    /// Decode the UTC time to a plain date-time struct (no `chrono` feature
    /// required).
    ///
    /// Returns `None` if the date/time fields are out of range. MJD→calendar
    /// conversion per ETSI EN 300 468 Annex C.
    #[must_use]
    pub fn utc_time_decoded(&self) -> Option<dvb_common::time::MjdBcdDateTime> {
        dvb_common::time::decode_mjd_bcd(self.utc_time_raw)
    }

    /// Set the UTC time, encoding it from a [`dvb_common::time::MjdBcdDateTime`].
    ///
    /// # Errors
    /// [`ValueOutOfRange`](crate::Error::ValueOutOfRange) if the date is
    /// outside the representable 16-bit MJD range.
    pub fn set_utc_time_decoded(&mut self, dt: dvb_common::time::MjdBcdDateTime) -> Result<()> {
        self.utc_time_raw = dvb_common::time::encode_mjd_bcd(dt).ok_or(Error::ValueOutOfRange {
            field: "TdtSection::utc_time",
            reason: "date not representable in 16-bit MJD",
        })?;
        Ok(())
    }

    /// Raw 5-byte UTC time field (for round-trip / serialization).
    #[must_use]
    pub fn utc_time_raw(&self) -> [u8; 5] {
        self.utc_time_raw
    }

    /// Construct a `TdtSection` from raw wire fields.
    #[must_use]
    pub fn new(utc_time_raw: [u8; 5]) -> Self {
        Self { utc_time_raw }
    }
}

#[cfg(feature = "chrono")]
impl TdtSection {
    /// Decode the UTC time to a chrono DateTime when the `chrono` feature is on.
    ///
    /// MJD→calendar conversion per ETSI EN 300 468 Annex C.
    #[must_use]
    pub fn utc_time(&self) -> Option<chrono::DateTime<chrono::Utc>> {
        dvb_common::time::decode_mjd_bcd_utc(self.utc_time_raw)
    }

    /// Set the UTC time, encoding it into the 40-bit `utc_time` field.
    ///
    /// # Errors
    /// [`ValueOutOfRange`](crate::Error::ValueOutOfRange) if the date is
    /// outside the representable 16-bit MJD range.
    pub fn set_utc_time(&mut self, utc_time: chrono::DateTime<chrono::Utc>) -> Result<()> {
        self.utc_time_raw =
            dvb_common::time::encode_mjd_bcd_utc(utc_time).ok_or(Error::ValueOutOfRange {
                field: "TdtSection::utc_time",
                reason: "date not representable in 16-bit MJD",
            })?;
        Ok(())
    }
}

impl<'a> Parse<'a> for TdtSection {
    type Error = crate::error::Error;
    fn parse(bytes: &'a [u8]) -> Result<Self> {
        let min_len = HEADER_LEN + UTC_TIME_LEN;
        if bytes.len() < min_len {
            return Err(Error::BufferTooShort {
                need: min_len,
                have: bytes.len(),
                what: "TdtSection",
            });
        }
        if bytes[0] != TABLE_ID {
            return Err(Error::UnexpectedTableId {
                table_id: bytes[0],
                what: "TdtSection",
                expected: &[TABLE_ID],
            });
        }
        let section_length = ((bytes[1] & 0x0F) as u16) << 8 | bytes[2] as u16;
        if section_length as usize != UTC_TIME_LEN {
            return Err(Error::SectionLengthOverflow {
                declared: section_length as usize,
                available: UTC_TIME_LEN,
            });
        }
        let utc_time_raw = [bytes[3], bytes[4], bytes[5], bytes[6], bytes[7]];
        Ok(TdtSection { utc_time_raw })
    }
}

impl Serialize for TdtSection {
    type Error = crate::error::Error;
    fn serialized_len(&self) -> usize {
        HEADER_LEN + UTC_TIME_LEN
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
        buf[1] = super::SECTION_B1_FLAGS_SHORT | ((UTC_TIME_LEN as u16 >> 8) as u8 & 0x0F);
        buf[2] = UTC_TIME_LEN as u8;
        buf[3..8].copy_from_slice(&self.utc_time_raw);
        Ok(len)
    }
}
impl<'a> crate::traits::TableDef<'a> for TdtSection {
    const TABLE_ID_RANGES: &'static [(u8, u8)] = &[(TABLE_ID, TABLE_ID)];
    const NAME: &'static str = "TIME_AND_DATE";
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_extracts_utc_time_raw() {
        let bytes = [TABLE_ID, 0x70, 0x05, 0xE4, 0x09, 0x12, 0x34, 0x56];
        let tdt = TdtSection::parse(&bytes).unwrap();
        assert_eq!(tdt.utc_time_raw(), [0xE4, 0x09, 0x12, 0x34, 0x56]);
    }

    #[test]
    fn parse_rejects_wrong_tag() {
        let bytes = [0x71, 0x70, 0x05, 0, 0, 0, 0, 0];
        assert!(matches!(
            TdtSection::parse(&bytes).unwrap_err(),
            Error::UnexpectedTableId { table_id: 0x71, .. }
        ));
    }

    #[test]
    fn parse_rejects_wrong_section_length() {
        let bytes = [TABLE_ID, 0x70, 0x04, 0, 0, 0, 0, 0];
        assert!(matches!(
            TdtSection::parse(&bytes).unwrap_err(),
            Error::SectionLengthOverflow { .. }
        ));
    }

    #[test]
    fn serialize_round_trip() {
        let tdt = TdtSection {
            utc_time_raw: [0xE4, 0x09, 0x12, 0x34, 0x56],
        };
        let mut buf = vec![0u8; tdt.serialized_len()];
        tdt.serialize_into(&mut buf).unwrap();
        let re = TdtSection::parse(&buf).unwrap();
        assert_eq!(tdt, re);
    }

    #[cfg(feature = "chrono")]
    #[test]
    fn utc_time_decodes_to_chrono() {
        let tdt = TdtSection {
            utc_time_raw: [0xEA, 0x19, 0x12, 0x34, 0x56],
        };
        let dt = tdt.utc_time();
        assert!(dt.is_some());
    }

    #[test]
    fn utc_time_decodes_without_chrono() {
        let tdt = TdtSection {
            utc_time_raw: [0xE4, 0x09, 0x12, 0x34, 0x56],
        };
        let dt = tdt.utc_time_decoded().unwrap();
        assert_eq!(dt.year, 2018);
        assert_eq!(dt.month, 9);
        assert_eq!(dt.day, 16);
        assert_eq!(dt.hour, 12);
        assert_eq!(dt.minute, 34);
        assert_eq!(dt.second, 56);
    }
}
