//! target_MAC_address_descriptor — ETSI EN 301 192 V1.7.1 §8.4.5.6 Table 24
//! (INT) / ETSI TS 102 006 §9.5.2.2 Table 21 (UNT) (tag 0x07).
//!
//! Targets a single or a group of receivers by MAC address. A bit set in
//! `MAC_addr_mask` is compared against the same bit of each `MAC_addr` (UNT:
//! `MAC_addr_match`); repeated addresses are a logical OR.

use crate::descriptors::entry_list::{HEADER_LEN, MacAddress, parse_masked, write_masked};
use crate::error::Result;
use alloc::vec::Vec;
use broadcast_common::{Parse, Serialize};

/// Descriptor tag for target_MAC_address_descriptor.
pub const TAG: u8 = 0x07;
const ADDR_LEN: usize = 6;

/// target_MAC_address_descriptor.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[cfg_attr(feature = "yoke", derive(yoke::Yokeable))]
pub struct TargetMacAddressDescriptor<'a> {
    /// `MAC_addr_mask` (48 bits, MSB octet first) — which bits are compared.
    pub addr_mask: MacAddress,
    /// `MAC_addr` entries (a list of addressed receivers, OR-ed).
    pub addrs: Vec<MacAddress>,
    /// Bytes after the last whole address inside `descriptor_length`; the
    /// spec defines none, so this is parse tolerance only and is always shorter
    /// than one address; serialize rejects a longer `trailing` (it would
    /// re-parse as addresses).
    pub trailing: &'a [u8],
}

impl<'a> Parse<'a> for TargetMacAddressDescriptor<'a> {
    type Error = crate::error::Error;
    fn parse(bytes: &'a [u8]) -> Result<Self> {
        let (addr_mask, addrs, trailing) =
            parse_masked::<ADDR_LEN, MacAddress>(bytes, TAG, "TargetMacAddressDescriptor")?;
        Ok(Self {
            addr_mask,
            addrs,
            trailing,
        })
    }
}

impl Serialize for TargetMacAddressDescriptor<'_> {
    type Error = crate::error::Error;
    fn serialized_len(&self) -> usize {
        HEADER_LEN + ADDR_LEN + self.addrs.len() * ADDR_LEN + self.trailing.len()
    }
    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        write_masked::<ADDR_LEN, MacAddress>(buf, TAG, self.addr_mask, &self.addrs, self.trailing)
    }
}

impl<'a> crate::traits::DescriptorDef<'a> for TargetMacAddressDescriptor<'a> {
    const TAG: u8 = TAG;
    const NAME: &'static str = "TARGET_MAC_ADDRESS";
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_and_round_trip() {
        // TSDuck test-022: mask 01:A3:E4:BA:45:74, one address AB:CD:EF:01:23:45.
        let bytes = [
            TAG, 12, 0x01, 0xA3, 0xE4, 0xBA, 0x45, 0x74, 0xAB, 0xCD, 0xEF, 0x01, 0x23, 0x45,
        ];
        let d = TargetMacAddressDescriptor::parse(&bytes).unwrap();
        assert_eq!(d.addr_mask, [0x01, 0xA3, 0xE4, 0xBA, 0x45, 0x74]);
        assert_eq!(d.addrs, [[0xAB, 0xCD, 0xEF, 0x01, 0x23, 0x45]]);
        let mut out = vec![0u8; d.serialized_len()];
        d.serialize_into(&mut out).unwrap();
        assert_eq!(out, bytes);
    }

    #[test]
    fn partial_tail_round_trip_and_rejects_short_mask() {
        let bytes = [TAG, 8, 1, 2, 3, 4, 5, 6, 7, 8];
        let d = TargetMacAddressDescriptor::parse(&bytes).unwrap();
        assert_eq!(d.trailing, &[7, 8]);
        let mut out = vec![0u8; d.serialized_len()];
        d.serialize_into(&mut out).unwrap();
        assert_eq!(out, bytes);
        assert!(TargetMacAddressDescriptor::parse(&[TAG, 5, 1, 2, 3, 4, 5]).is_err());
    }
}
