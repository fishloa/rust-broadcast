//! target_MAC_address_range_descriptor — ETSI EN 301 192 V1.7.1 §8.4.5.7
//! Table 25 (tag 0x0E).
//!
//! Targets receivers whose MAC address falls in one or more ranges: every
//! receiver from `MAC_addr_low` up to and including `MAC_addr_high` is
//! selected. INT target loop only (Table 19).

use crate::descriptors::entry_list::{Entry, MacAddress, list_len, parse_list, write_list};
use crate::error::Result;
use alloc::vec::Vec;
use broadcast_common::{Parse, Serialize};

/// Descriptor tag for target_MAC_address_range_descriptor.
pub const TAG: u8 = 0x0E;
const ENTRY_LEN: usize = 12;

/// One `MAC_addr_low` / `MAC_addr_high` pair (inclusive range).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct MacAddressRange {
    /// `MAC_addr_low` — the lowest MAC address in the range.
    pub low: MacAddress,
    /// `MAC_addr_high` — the highest MAC address in the range.
    pub high: MacAddress,
}

impl Entry<ENTRY_LEN> for MacAddressRange {
    fn read(b: &[u8; ENTRY_LEN]) -> Self {
        let (low, high) = b.split_at(6);
        Self {
            low: low.try_into().unwrap_or_default(),
            high: high.try_into().unwrap_or_default(),
        }
    }
    fn write(&self, b: &mut [u8; ENTRY_LEN]) {
        b[..6].copy_from_slice(&self.low);
        b[6..].copy_from_slice(&self.high);
    }
}

/// target_MAC_address_range_descriptor.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[cfg_attr(feature = "yoke", derive(yoke::Yokeable))]
pub struct TargetMacAddressRangeDescriptor<'a> {
    /// The address ranges.
    pub ranges: Vec<MacAddressRange>,
    /// Bytes after the last whole range inside `descriptor_length`; the spec
    /// defines none (kept for byte-identical re-serialization).
    pub trailing: &'a [u8],
}

impl<'a> Parse<'a> for TargetMacAddressRangeDescriptor<'a> {
    type Error = crate::error::Error;
    fn parse(bytes: &'a [u8]) -> Result<Self> {
        let (ranges, trailing) = parse_list::<ENTRY_LEN, MacAddressRange>(
            bytes,
            TAG,
            "TargetMacAddressRangeDescriptor",
        )?;
        Ok(Self { ranges, trailing })
    }
}

impl Serialize for TargetMacAddressRangeDescriptor<'_> {
    type Error = crate::error::Error;
    fn serialized_len(&self) -> usize {
        list_len(self.ranges.len(), ENTRY_LEN, self.trailing.len())
    }
    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        write_list::<ENTRY_LEN, _>(buf, TAG, &self.ranges, self.trailing)
    }
}

impl<'a> crate::traits::DescriptorDef<'a> for TargetMacAddressRangeDescriptor<'a> {
    const TAG: u8 = TAG;
    const NAME: &'static str = "TARGET_MAC_ADDRESS_RANGE";
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_and_round_trip() {
        // TSDuck test-022: 12:78:65:32:59:32 .. 78:44:54:54:98:87.
        let bytes = [
            TAG, 12, 0x12, 0x78, 0x65, 0x32, 0x59, 0x32, 0x78, 0x44, 0x54, 0x54, 0x98, 0x87,
        ];
        let d = TargetMacAddressRangeDescriptor::parse(&bytes).unwrap();
        assert_eq!(d.ranges.len(), 1);
        assert_eq!(d.ranges[0].low, [0x12, 0x78, 0x65, 0x32, 0x59, 0x32]);
        assert_eq!(d.ranges[0].high, [0x78, 0x44, 0x54, 0x54, 0x98, 0x87]);
        let mut out = vec![0u8; d.serialized_len()];
        d.serialize_into(&mut out).unwrap();
        assert_eq!(out, bytes);
    }

    #[test]
    fn empty_and_partial_tail_round_trip() {
        for bytes in [&[TAG, 0][..], &[TAG, 3, 1, 2, 3][..]] {
            let d = TargetMacAddressRangeDescriptor::parse(bytes).unwrap();
            let mut out = vec![0u8; d.serialized_len()];
            d.serialize_into(&mut out).unwrap();
            assert_eq!(out, bytes);
        }
        assert!(TargetMacAddressRangeDescriptor::parse(&[0x0F, 0]).is_err());
    }
}
