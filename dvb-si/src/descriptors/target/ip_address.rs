//! target_IP_address_descriptor — ETSI EN 301 192 V1.7.1 §8.4.5.8 Table 26
//! (INT) / ETSI TS 102 006 §9.5.2.3 Table 22 (UNT) (tag 0x09).
//!
//! Targets a single or a group of receivers by IPv4 address. A bit set in
//! `IPv4_addr_mask` (UNT: `IP_addr_mask`) is compared against the same bit of
//! each `IPv4_addr` (UNT: `IP_addr_match`); repeated addresses are a logical
//! OR.

use crate::descriptors::entry_list::{HEADER_LEN, parse_masked, write_masked};
use crate::error::Result;
use alloc::vec::Vec;
use broadcast_common::{Parse, Serialize};
use core::net::Ipv4Addr;

/// Descriptor tag for target_IP_address_descriptor.
pub const TAG: u8 = 0x09;
const ADDR_LEN: usize = 4;

/// target_IP_address_descriptor.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[cfg_attr(feature = "yoke", derive(yoke::Yokeable))]
pub struct TargetIpAddressDescriptor<'a> {
    /// `IPv4_addr_mask` — which address bits are compared.
    pub addr_mask: Ipv4Addr,
    /// `IPv4_addr` entries (a list of addressed receivers, OR-ed).
    pub addrs: Vec<Ipv4Addr>,
    /// Bytes after the last whole address inside `descriptor_length`; the
    /// spec defines none, so this is parse tolerance only (empty for a
    /// conformant descriptor, always shorter than one address); serialize
    /// rejects a longer `trailing` (it would re-parse as addresses).
    pub trailing: &'a [u8],
}

impl<'a> Parse<'a> for TargetIpAddressDescriptor<'a> {
    type Error = crate::error::Error;
    fn parse(bytes: &'a [u8]) -> Result<Self> {
        let (addr_mask, addrs, trailing) =
            parse_masked::<ADDR_LEN, Ipv4Addr>(bytes, TAG, "TargetIpAddressDescriptor")?;
        Ok(Self {
            addr_mask,
            addrs,
            trailing,
        })
    }
}

impl Serialize for TargetIpAddressDescriptor<'_> {
    type Error = crate::error::Error;
    fn serialized_len(&self) -> usize {
        HEADER_LEN + ADDR_LEN + self.addrs.len() * ADDR_LEN + self.trailing.len()
    }
    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        write_masked::<ADDR_LEN, Ipv4Addr>(buf, TAG, self.addr_mask, &self.addrs, self.trailing)
    }
}

impl<'a> crate::traits::DescriptorDef<'a> for TargetIpAddressDescriptor<'a> {
    const TAG: u8 = TAG;
    const NAME: &'static str = "TARGET_IP_ADDRESS";
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_and_round_trip() {
        // TSDuck test-022: mask 12.34.56.78, addrs 1.2.3.4 and 54.124.12.78.
        let bytes = [TAG, 12, 12, 34, 56, 78, 1, 2, 3, 4, 54, 124, 12, 78];
        let d = TargetIpAddressDescriptor::parse(&bytes).unwrap();
        assert_eq!(d.addr_mask, Ipv4Addr::new(12, 34, 56, 78));
        assert_eq!(
            d.addrs,
            [Ipv4Addr::new(1, 2, 3, 4), Ipv4Addr::new(54, 124, 12, 78)]
        );
        assert!(d.trailing.is_empty());
        let mut out = vec![0u8; d.serialized_len()];
        d.serialize_into(&mut out).unwrap();
        assert_eq!(out, bytes);
    }

    #[test]
    fn mask_only_and_partial_tail_round_trip() {
        for bytes in [
            &[TAG, 4, 12, 34, 56, 78][..],
            &[TAG, 7, 1, 2, 3, 4, 9, 9, 9][..],
        ] {
            let d = TargetIpAddressDescriptor::parse(bytes).unwrap();
            let mut out = vec![0u8; d.serialized_len()];
            d.serialize_into(&mut out).unwrap();
            assert_eq!(out, bytes);
        }
    }

    #[test]
    fn rejects_missing_mask_and_wrong_tag() {
        assert!(TargetIpAddressDescriptor::parse(&[TAG, 3, 1, 2, 3]).is_err());
        assert!(TargetIpAddressDescriptor::parse(&[0x0A, 4, 1, 2, 3, 4]).is_err());
    }
}
