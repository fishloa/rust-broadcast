//! time_descriptor() — ANSI/SCTE 35 2023r1 §10.3.4, Table 27 (tag 0x03).
//!
//! Carries a programmer's wall-clock time in Precision Time Protocol (PTP/TAI)
//! format plus a `UTC_offset` for conversion to UTC/NTP.

use super::header::{self, CUEI, HEADER_LEN};
use crate::error::{Error, Result};
use crate::traits::SpliceDescriptorDef;
use broadcast_common::{Parse, Serialize};

/// `splice_descriptor_tag` for time_descriptor (§10.1, Table 16).
pub const TAG: u8 = 0x03;

/// Body length: TAI_seconds (6) + TAI_ns (4) + UTC_offset (2).
const BODY_LEN: usize = 12;

/// time_descriptor() — §10.3.4, Table 27.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct TimeDescriptor {
    /// 32-bit `identifier` (shall be "CUEI").
    pub identifier: u32,
    /// 48-bit TAI seconds.
    pub tai_seconds: u64,
    /// 32-bit TAI nanoseconds.
    pub tai_ns: u32,
    /// 16-bit `UTC_offset` (`UTC = TAI - UTC_offset`).
    pub utc_offset: u16,
}

impl Default for TimeDescriptor {
    fn default() -> Self {
        Self {
            identifier: CUEI,
            tai_seconds: 0,
            tai_ns: 0,
            utc_offset: 0,
        }
    }
}

impl<'a> Parse<'a> for TimeDescriptor {
    type Error = Error;
    fn parse(bytes: &'a [u8]) -> Result<Self> {
        let (identifier, body) = header::descriptor_body(bytes, TAG, "time_descriptor")?;
        if body.len() < BODY_LEN {
            return Err(Error::BufferTooShort {
                need: HEADER_LEN + BODY_LEN,
                have: bytes.len(),
                what: "time_descriptor body",
            });
        }
        let tai_seconds = (u64::from(body[0]) << 40)
            | (u64::from(body[1]) << 32)
            | (u64::from(body[2]) << 24)
            | (u64::from(body[3]) << 16)
            | (u64::from(body[4]) << 8)
            | u64::from(body[5]);
        let tai_ns =
            u32::from_be_bytes(*body[6..].first_chunk::<4>().ok_or(Error::BufferTooShort {
                need: HEADER_LEN + BODY_LEN,
                have: bytes.len(),
                what: "time_descriptor body",
            })?);
        let utc_offset =
            u16::from_be_bytes(*body[10..].first_chunk::<2>().ok_or(Error::BufferTooShort {
                need: HEADER_LEN + BODY_LEN,
                have: bytes.len(),
                what: "time_descriptor body",
            })?);
        Ok(Self {
            identifier,
            tai_seconds,
            tai_ns,
            utc_offset,
        })
    }
}

impl Serialize for TimeDescriptor {
    type Error = Error;
    fn serialized_len(&self) -> usize {
        HEADER_LEN + BODY_LEN
    }
    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        let need = self.serialized_len();
        if buf.len() < need {
            return Err(Error::OutputBufferTooSmall {
                need,
                have: buf.len(),
            });
        }
        if self.tai_seconds > (1u64 << 48) - 1 {
            return Err(Error::InvalidValue {
                field: "time_descriptor.tai_seconds",
                reason: "exceeds 48-bit range",
            });
        }
        header::write_header(buf, TAG, self.identifier, BODY_LEN);
        let s = self.tai_seconds;
        buf[HEADER_LEN] = (s >> 40) as u8;
        buf[HEADER_LEN + 1] = (s >> 32) as u8;
        buf[HEADER_LEN + 2] = (s >> 24) as u8;
        buf[HEADER_LEN + 3] = (s >> 16) as u8;
        buf[HEADER_LEN + 4] = (s >> 8) as u8;
        buf[HEADER_LEN + 5] = s as u8;
        buf[HEADER_LEN + 6..HEADER_LEN + 10].copy_from_slice(&self.tai_ns.to_be_bytes());
        buf[HEADER_LEN + 10..need].copy_from_slice(&self.utc_offset.to_be_bytes());
        Ok(need)
    }
}

impl SpliceDescriptorDef<'_> for TimeDescriptor {
    const TAG: u8 = TAG;
    const NAME: &'static str = "TIME";
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip() {
        let d = TimeDescriptor {
            identifier: CUEI,
            tai_seconds: 0x0000_1122_3344_5566 & ((1 << 48) - 1),
            tai_ns: 0x0102_0304,
            utc_offset: 37,
        };
        let bytes = d.to_bytes();
        assert_eq!(bytes[0], TAG);
        assert_eq!(bytes[1], 0x10);
        let back = TimeDescriptor::parse(&bytes).unwrap();
        assert_eq!(d, back);
        assert_eq!(back.to_bytes(), bytes);
    }
}
