//! insert_time_descriptor() — ANSI/SCTE 104 2023 §9.8.10, Table 9-32 (opID 0x0110).
//!
//! Supplemental usage. Adds a time_descriptor with PTP/TAI time to the
//! resulting SCTE 35 section.

use super::{U16_LEN, U32_LEN, be};
use crate::error::{Error, Result};
use crate::traits::OperationDef;
use broadcast_core::len::{need_in, need_out};
use broadcast_core::{Parse, Serialize};

/// `opID` for insert_time_descriptor (§8.3, Table 8-4).
pub const OP_ID: u16 = 0x0110;

/// Fixed wire length.
pub const LEN: usize = 12;

// Field byte offsets within the descriptor body (§8.3, Table 8-4).
const TAI_SECONDS_OFFSET: usize = 0;
const TAI_SECONDS_LEN: usize = 6;
const TAI_NS_OFFSET: usize = TAI_SECONDS_OFFSET + TAI_SECONDS_LEN;
const UTC_OFFSET_OFFSET: usize = TAI_NS_OFFSET + core::mem::size_of::<u32>();
/// Bits per byte, used to derive the 48-bit `TAI_seconds` bound.
const BYTE_SHIFT: u32 = u8::BITS;

/// insert_time_descriptor() — §9.8.10, Table 9-32.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct InsertTimeDescriptor {
    /// `TAI_seconds` — 6 bytes (48-bit big-endian).
    pub tai_seconds: u64,
    /// `TAI_ns` — 4 bytes (32-bit big-endian).
    pub tai_ns: u32,
    /// `UTC_offset` — 2 bytes (16-bit big-endian).
    pub utc_offset: u16,
}

impl<'a> Parse<'a> for InsertTimeDescriptor {
    type Error = Error;
    fn parse(bytes: &'a [u8]) -> Result<Self> {
        need_in(bytes, LEN, "insert_time_descriptor")?;
        // TAI_seconds: 6 bytes (48-bit)
        let tai_seconds = {
            let src = &bytes[TAI_SECONDS_OFFSET..TAI_SECONDS_OFFSET + TAI_SECONDS_LEN];
            let mut bytes_be = [0u8; core::mem::size_of::<u64>()];
            bytes_be[core::mem::size_of::<u16>()..].copy_from_slice(src);
            u64::from_be_bytes(bytes_be)
        };
        let tai_ns = u32::from_be_bytes(be::<U32_LEN>(bytes, TAI_NS_OFFSET)?);
        let utc_offset = u16::from_be_bytes(be::<U16_LEN>(bytes, UTC_OFFSET_OFFSET)?);
        Ok(Self {
            tai_seconds,
            tai_ns,
            utc_offset,
        })
    }
}

impl Serialize for InsertTimeDescriptor {
    type Error = Error;
    fn serialized_len(&self) -> usize {
        LEN
    }
    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        need_out(buf, LEN)?;
        if self.tai_seconds >> (TAI_SECONDS_LEN as u32 * BYTE_SHIFT) != 0 {
            return Err(Error::FieldOverflow(broadcast_core::len::FieldOverflow {
                field: "insert_time_descriptor.TAI_seconds",
                value: self.tai_seconds,
                max: (1 << (TAI_SECONDS_LEN as u32 * BYTE_SHIFT)) - 1,
            }));
        }
        // TAI_seconds: 6 bytes (48-bit)
        let tai_seconds_be = self.tai_seconds.to_be_bytes();
        buf[TAI_SECONDS_OFFSET..TAI_SECONDS_OFFSET + TAI_SECONDS_LEN]
            .copy_from_slice(&tai_seconds_be[core::mem::size_of::<u16>()..]);
        buf[6..10].copy_from_slice(&self.tai_ns.to_be_bytes());
        buf[10..12].copy_from_slice(&self.utc_offset.to_be_bytes());
        Ok(LEN)
    }
}

impl OperationDef<'_> for InsertTimeDescriptor {
    const OP_ID: u16 = OP_ID;
    const NAME: &'static str = "INSERT_TIME_DESCRIPTOR";
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip() {
        let op = InsertTimeDescriptor {
            tai_seconds: 0x0000_1234_5678_9ABC,
            tai_ns: 500_000_000,
            utc_offset: 37,
        };
        let bytes = op.try_to_bytes().unwrap();
        assert_eq!(bytes.len(), LEN);
        let back = InsertTimeDescriptor::parse(&bytes).unwrap();
        assert_eq!(op, back);
    }

    #[test]
    fn mutate_field_changes_output() {
        let op = InsertTimeDescriptor::default();
        let bytes = op.try_to_bytes().unwrap();
        let mut op2 = op;
        op2.utc_offset = 99;
        assert_ne!(op2.try_to_bytes().unwrap(), bytes);
    }

    /// A `TAI_seconds` above 48 bits is rejected, not silently masked.
    #[test]
    fn tai_seconds_over_48_bits_is_rejected() {
        let op = InsertTimeDescriptor {
            tai_seconds: 1 << 48,
            tai_ns: 0,
            utc_offset: 0,
        };
        assert_eq!(
            op.try_to_bytes().err(),
            Some(crate::error::Error::FieldOverflow(
                broadcast_core::len::FieldOverflow {
                    field: "insert_time_descriptor.TAI_seconds",
                    value: 1 << 48,
                    max: (1 << 48) - 1
                }
            ))
        );
    }
}
