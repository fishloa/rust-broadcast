//! Time and Date Table — ETSI EN 300 468 §5.2.5.
//!
//! Short-form section on PID 0x0014 with table_id 0x70. Body is exactly
//! 5 bytes of UTC time (16-bit MJD + 24-bit BCD HHMMSS). No CRC.

use crate::error::{Error, Result};
use crate::traits::Table;
use dvb_common::{Parse, Serialize};

/// table_id for Time and Date Table.
pub const TABLE_ID: u8 = 0x70;
/// Well-known PID on which TDT is carried.
pub const PID: u16 = 0x0014;

const HEADER_LEN: usize = 3;
const UTC_TIME_LEN: usize = 5;

/// Time and Date Table.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct Tdt {
    /// Raw 5-byte UTC time (16-bit MJD + 24-bit BCD HHMMSS) per
    /// EN 300 468 Annex C.
    pub utc_time_raw: [u8; 5],
}

impl Tdt {
    /// Decode the UTC time to a chrono DateTime when the `chrono` feature is on.
    #[cfg(feature = "chrono")]
    #[must_use]
    pub fn utc_time(&self) -> Option<chrono::DateTime<chrono::Utc>> {
        use chrono::{NaiveDate, NaiveDateTime, TimeZone};
        let mjd = u16::from_be_bytes([self.utc_time_raw[0], self.utc_time_raw[1]]);
        let (y, m, d) = mjd_to_ymd(mjd);
        let h = bcd(self.utc_time_raw[2])?;
        let mi = bcd(self.utc_time_raw[3])?;
        let s = bcd(self.utc_time_raw[4])?;
        let date = NaiveDate::from_ymd_opt(y, m, d)?;
        let time = chrono::NaiveTime::from_hms_opt(h.into(), mi.into(), s.into())?;
        chrono::Utc
            .from_local_datetime(&NaiveDateTime::new(date, time))
            .single()
    }
}

#[cfg(feature = "chrono")]
fn bcd(b: u8) -> Option<u8> {
    let hi = b >> 4;
    let lo = b & 0x0F;
    if hi > 9 || lo > 9 {
        return None;
    }
    Some(hi * 10 + lo)
}

#[cfg(feature = "chrono")]
fn mjd_to_ymd(mjd: u16) -> (i32, u32, u32) {
    let mjd = i64::from(mjd);
    let y_prime = ((mjd as f64 - 15_078.2) / 365.25) as i64;
    let m_prime = ((mjd as f64 - 14_956.1 - (y_prime as f64 * 365.25).floor()) / 30.6001) as i64;
    let d = mjd
        - 14_956
        - (y_prime as f64 * 365.25).floor() as i64
        - (m_prime as f64 * 30.6001).floor() as i64;
    let k = if m_prime == 14 || m_prime == 15 { 1 } else { 0 };
    let y = y_prime + k + 1900;
    let m = m_prime - 1 - k * 12;
    (y as i32, m as u32, d as u32)
}

impl<'a> Parse<'a> for Tdt {
    type Error = crate::error::Error;
    fn parse(bytes: &'a [u8]) -> Result<Self> {
        let min_len = HEADER_LEN + UTC_TIME_LEN;
        if bytes.len() < min_len {
            return Err(Error::BufferTooShort {
                need: min_len,
                have: bytes.len(),
                what: "Tdt",
            });
        }
        if bytes[0] != TABLE_ID {
            return Err(Error::UnexpectedTableId {
                table_id: bytes[0],
                what: "Tdt",
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
        Ok(Tdt { utc_time_raw })
    }
}

impl Serialize for Tdt {
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
        buf[1] = 0x70 | ((UTC_TIME_LEN as u16 >> 8) as u8 & 0x0F);
        buf[2] = UTC_TIME_LEN as u8;
        buf[3..8].copy_from_slice(&self.utc_time_raw);
        Ok(len)
    }
}

impl<'a> Table<'a> for Tdt {
    const TABLE_ID: u8 = TABLE_ID;
    const PID: u16 = PID;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_extracts_utc_time_raw() {
        let bytes = [TABLE_ID, 0x70, 0x05, 0xE4, 0x09, 0x12, 0x34, 0x56];
        let tdt = Tdt::parse(&bytes).unwrap();
        assert_eq!(tdt.utc_time_raw, [0xE4, 0x09, 0x12, 0x34, 0x56]);
    }

    #[test]
    fn parse_rejects_wrong_tag() {
        let bytes = [0x71, 0x70, 0x05, 0, 0, 0, 0, 0];
        assert!(matches!(
            Tdt::parse(&bytes).unwrap_err(),
            Error::UnexpectedTableId { table_id: 0x71, .. }
        ));
    }

    #[test]
    fn parse_rejects_wrong_section_length() {
        let bytes = [TABLE_ID, 0x70, 0x04, 0, 0, 0, 0, 0];
        assert!(matches!(
            Tdt::parse(&bytes).unwrap_err(),
            Error::SectionLengthOverflow { .. }
        ));
    }

    #[test]
    fn serialize_round_trip() {
        let tdt = Tdt {
            utc_time_raw: [0xE4, 0x09, 0x12, 0x34, 0x56],
        };
        let mut buf = vec![0u8; tdt.serialized_len()];
        tdt.serialize_into(&mut buf).unwrap();
        let re = Tdt::parse(&buf).unwrap();
        assert_eq!(tdt, re);
    }

    #[test]
    fn utc_time_decodes_to_chrono() {
        let tdt = Tdt {
            utc_time_raw: [0xEA, 0x19, 0x12, 0x34, 0x56],
        };
        let dt = tdt.utc_time();
        assert!(dt.is_some());
    }
}
