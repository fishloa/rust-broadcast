//! time_descriptor() — ANSI/SCTE 35 2023r1 §10.3.4, Table 27 (tag 0x03).
//!
//! Carries a programmer's wall-clock time in Precision Time Protocol (PTP/TAI)
//! format plus a `UTC_offset` for conversion to UTC/NTP.

use super::header::{self, CUEI, HEADER_LEN};
use crate::error::{Error, Result};
use crate::traits::SpliceDescriptorDef;
use crate::{U16_LEN, U32_LEN, be};
use broadcast_common::len::need_out;
use broadcast_common::{Parse, Serialize};

/// `splice_descriptor_tag` for time_descriptor (§10.1, Table 16).
pub const TAG: u8 = 0x03;

/// Body length: TAI_seconds (6) + TAI_ns (4) + UTC_offset (2).
const BODY_LEN: usize = 12;

const TAI_SECONDS_OFFSET: usize = 0;
const TAI_SECONDS_LEN: usize = 6;
const TAI_NS_OFFSET: usize = TAI_SECONDS_OFFSET + TAI_SECONDS_LEN;
const UTC_OFFSET_OFFSET: usize = TAI_NS_OFFSET + core::mem::size_of::<u32>();
const TAI_SECONDS_MAX: u64 = (1u64 << (TAI_SECONDS_LEN as u32 * u8::BITS)) - 1;
const TAI_SECONDS_TOP_SHIFT: u32 = 40;
const TAI_SECONDS_HIGH_SHIFT: u32 = 32;
const TAI_SECONDS_MID_SHIFT: u32 = 24;
const TAI_SECONDS_LOW_SHIFT: u32 = 16;
const TAI_SECONDS_BOTTOM_SHIFT: u32 = 8;
const TAI_SECONDS_BYTE_TOP: usize = TAI_SECONDS_OFFSET;
const TAI_SECONDS_BYTE_HIGH: usize = TAI_SECONDS_OFFSET + 1;
const TAI_SECONDS_BYTE_MID: usize = TAI_SECONDS_OFFSET + 2;
const TAI_SECONDS_BYTE_LOW: usize = TAI_SECONDS_OFFSET + 3;
const TAI_SECONDS_BYTE_BOTTOM: usize = TAI_SECONDS_OFFSET + 4;
const TAI_SECONDS_BYTE_LAST: usize = TAI_SECONDS_OFFSET + 5;

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
            return Err(Error::BufferTooShort(
                broadcast_common::error::BufferTooShort {
                    need: HEADER_LEN + BODY_LEN,
                    have: bytes.len(),
                    what: "time_descriptor body",
                },
            ));
        }
        let tai_seconds = (u64::from(body[TAI_SECONDS_BYTE_TOP]) << TAI_SECONDS_TOP_SHIFT)
            | (u64::from(body[TAI_SECONDS_BYTE_HIGH]) << TAI_SECONDS_HIGH_SHIFT)
            | (u64::from(body[TAI_SECONDS_BYTE_MID]) << TAI_SECONDS_MID_SHIFT)
            | (u64::from(body[TAI_SECONDS_BYTE_LOW]) << TAI_SECONDS_LOW_SHIFT)
            | (u64::from(body[TAI_SECONDS_BYTE_BOTTOM]) << TAI_SECONDS_BOTTOM_SHIFT)
            | u64::from(body[TAI_SECONDS_BYTE_LAST]);
        let tai_ns = u32::from_be_bytes(be::<U32_LEN>(body, TAI_NS_OFFSET)?);
        let utc_offset = u16::from_be_bytes(be::<U16_LEN>(body, UTC_OFFSET_OFFSET)?);
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
        need_out(buf, need)?;
        if self.tai_seconds > TAI_SECONDS_MAX {
            return Err(Error::FieldOverflow(broadcast_common::len::FieldOverflow {
                field: "time_descriptor.tai_seconds",
                value: self.tai_seconds,
                max: TAI_SECONDS_MAX,
            }));
        }
        header::write_header(buf, TAG, self.identifier, BODY_LEN)?;
        let s = self.tai_seconds;
        buf[HEADER_LEN + TAI_SECONDS_BYTE_TOP] = (s >> TAI_SECONDS_TOP_SHIFT) as u8;
        buf[HEADER_LEN + TAI_SECONDS_BYTE_HIGH] = (s >> TAI_SECONDS_HIGH_SHIFT) as u8;
        buf[HEADER_LEN + TAI_SECONDS_BYTE_MID] = (s >> TAI_SECONDS_MID_SHIFT) as u8;
        buf[HEADER_LEN + TAI_SECONDS_BYTE_LOW] = (s >> TAI_SECONDS_LOW_SHIFT) as u8;
        buf[HEADER_LEN + TAI_SECONDS_BYTE_BOTTOM] = (s >> TAI_SECONDS_BOTTOM_SHIFT) as u8;
        buf[HEADER_LEN + TAI_SECONDS_BYTE_LAST] = s as u8;
        buf[HEADER_LEN + TAI_NS_OFFSET..HEADER_LEN + TAI_NS_OFFSET + U32_LEN]
            .copy_from_slice(&self.tai_ns.to_be_bytes());
        buf[HEADER_LEN + UTC_OFFSET_OFFSET..need].copy_from_slice(&self.utc_offset.to_be_bytes());
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
        let bytes = d.try_to_bytes().unwrap();
        assert_eq!(bytes[0], TAG);
        assert_eq!(bytes[1], 0x10);
        let back = TimeDescriptor::parse(&bytes).unwrap();
        assert_eq!(d, back);
        assert_eq!(back.try_to_bytes().unwrap(), bytes);
    }
}
