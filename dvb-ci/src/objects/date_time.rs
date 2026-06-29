//! Date-Time objects — ETSI EN 50221 §8.5.2, Tables 31-32 (PDF p. 35).
//!
//! - `date_time_enq` (`9F 84 40`, Table 31) — one-byte `response_interval`.
//! - `date_time` (`9F 84 41`, Table 32) — 5-byte MJD+BCD UTC time, plus an
//!   optional 16-bit signed `local_offset` (length_field = 5 or 7).
//!
//! (The PDF mislabels both tables "Date-Time Enquiry"; Table 32 is the
//! `date_time()` object — see `docs/en_50221/datetime.md`.) The `UTC_time` is
//! carried as the verbatim 5 wire bytes (MJD+BCD, EN 300 468 Annex C) so it
//! round-trips without needing the `chrono` feature.

use crate::error::{Error, Result};
use crate::tag::{self, ApduTag};
use crate::traits::ApduDef;
use broadcast_common::{Parse, Serialize};

/// Length of the `UTC_time` field (40 bits = 5 bytes).
pub const UTC_TIME_LEN: usize = 5;

/// `date_time_enq()` — Date-Time Enquiry (Table 31).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct DateTimeEnq {
    /// `response_interval` — if 0, the host replies once; otherwise it replies
    /// immediately then every `response_interval` seconds.
    pub response_interval: u8,
}

/// `date_time()` — current UTC date/time with optional local offset (Table 32).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct DateTime {
    /// `UTC_time` — 5 bytes, MJD (2) + BCD HH:MM:SS (3), as in TDT/TOT.
    pub utc_time: [u8; UTC_TIME_LEN],
    /// `local_offset` — optional signed minutes (`Local = UTC + offset`); present
    /// only when the host has reliable knowledge (length_field = 7 vs 5).
    pub local_offset: Option<i16>,
}

impl<'a> Parse<'a> for DateTimeEnq {
    type Error = Error;
    fn parse(bytes: &'a [u8]) -> Result<Self> {
        let body = super::parse_apdu_header(bytes, tag::DATE_TIME_ENQ, "date_time_enq")?;
        let &[response_interval] = body else {
            return Err(Error::InvalidObject {
                what: "date_time_enq",
                reason: "body must be exactly 1 byte (response_interval)",
            });
        };
        Ok(Self { response_interval })
    }
}

impl Serialize for DateTimeEnq {
    type Error = Error;
    fn serialized_len(&self) -> usize {
        super::apdu_len(1)
    }
    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        let pos = super::write_apdu_header(tag::DATE_TIME_ENQ, 1, buf)?;
        buf[pos] = self.response_interval;
        Ok(pos + 1)
    }
}

impl<'a> ApduDef<'a> for DateTimeEnq {
    const TAG: ApduTag = tag::DATE_TIME_ENQ;
    const NAME: &'static str = "DATE_TIME_ENQ";
}

impl<'a> Parse<'a> for DateTime {
    type Error = Error;
    fn parse(bytes: &'a [u8]) -> Result<Self> {
        let body = super::parse_apdu_header(bytes, tag::DATE_TIME, "date_time")?;
        let utc_time: [u8; UTC_TIME_LEN] = match body.len() {
            UTC_TIME_LEN | 7 => body[..UTC_TIME_LEN].try_into().unwrap(),
            _ => {
                return Err(Error::InvalidObject {
                    what: "date_time",
                    reason: "body must be 5 or 7 bytes",
                })
            }
        };
        let local_offset = if body.len() == 7 {
            Some(i16::from_be_bytes([body[5], body[6]]))
        } else {
            None
        };
        Ok(Self {
            utc_time,
            local_offset,
        })
    }
}

impl Serialize for DateTime {
    type Error = Error;
    fn serialized_len(&self) -> usize {
        let body = if self.local_offset.is_some() { 7 } else { 5 };
        super::apdu_len(body)
    }
    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        let body_len = if self.local_offset.is_some() { 7 } else { 5 };
        let mut pos = super::write_apdu_header(tag::DATE_TIME, body_len, buf)?;
        buf[pos..pos + UTC_TIME_LEN].copy_from_slice(&self.utc_time);
        pos += UTC_TIME_LEN;
        if let Some(off) = self.local_offset {
            buf[pos..pos + 2].copy_from_slice(&off.to_be_bytes());
            pos += 2;
        }
        Ok(pos)
    }
}

impl<'a> ApduDef<'a> for DateTime {
    const TAG: ApduTag = tag::DATE_TIME;
    const NAME: &'static str = "DATE_TIME";
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn enq_round_trip() {
        let e = DateTimeEnq {
            response_interval: 10,
        };
        let bytes = e.to_bytes();
        assert_eq!(bytes, [0x9F, 0x84, 0x40, 0x01, 0x0A]);
        assert_eq!(DateTimeEnq::parse(&bytes).unwrap(), e);
    }

    #[test]
    fn date_time_no_offset_round_trip() {
        let dt = DateTime {
            utc_time: [0xC0, 0x79, 0x12, 0x34, 0x56],
            local_offset: None,
        };
        let bytes = dt.to_bytes();
        assert_eq!(&bytes[..4], &[0x9F, 0x84, 0x41, 0x05]); // len 5
        assert_eq!(DateTime::parse(&bytes).unwrap(), dt);
    }

    #[test]
    fn date_time_with_offset_round_trip() {
        let dt = DateTime {
            utc_time: [0xC0, 0x79, 0x12, 0x34, 0x56],
            local_offset: Some(-60),
        };
        let bytes = dt.to_bytes();
        assert_eq!(&bytes[..4], &[0x9F, 0x84, 0x41, 0x07]); // len 7
        let parsed = DateTime::parse(&bytes).unwrap();
        assert_eq!(parsed, dt);
        assert_eq!(parsed.local_offset, Some(-60));
    }

    #[test]
    fn mutating_offset_changes_bytes_and_length() {
        let dt = DateTime {
            utc_time: [0; 5],
            local_offset: None,
        };
        let a = dt.to_bytes();
        let mut other = dt;
        other.local_offset = Some(120);
        let b = other.to_bytes();
        assert_ne!(a, b);
        assert_eq!(a.len(), 9);
        assert_eq!(b.len(), 11);
    }
}
