//! target_IPv6_address_descriptor — ETSI EN 301 192 V1.7.1 §8.4.5.11
//! Table 29 (INT) / ETSI TS 102 006 §9.5.2.4 Table 23 (UNT) (tag 0x0A).
//!
//! Targets a single or a group of receivers by IPv6 address. A bit set in
//! `IPv6_addr_mask` is compared against the same bit of each `IPv6_addr`
//! (UNT: `IPv6_addr_match`); repeated addresses are a logical OR.

use crate::descriptors::entry_list::{HEADER_LEN, parse_masked, write_masked};
use crate::error::Result;
use alloc::vec::Vec;
use broadcast_common::{Parse, Serialize};
use core::net::Ipv6Addr;

/// Descriptor tag for target_IPv6_address_descriptor.
pub const TAG: u8 = 0x0A;
const ADDR_LEN: usize = 16;

/// target_IPv6_address_descriptor.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[cfg_attr(feature = "yoke", derive(yoke::Yokeable))]
pub struct TargetIpv6AddressDescriptor<'a> {
    /// `IPv6_addr_mask` — which address bits are compared.
    pub addr_mask: Ipv6Addr,
    /// `IPv6_addr` entries (a list of addressed receivers, OR-ed).
    pub addrs: Vec<Ipv6Addr>,
    /// Bytes after the last whole address inside `descriptor_length`; the
    /// spec defines none (kept for byte-identical re-serialization).
    pub trailing: &'a [u8],
}

impl<'a> Parse<'a> for TargetIpv6AddressDescriptor<'a> {
    type Error = crate::error::Error;
    fn parse(bytes: &'a [u8]) -> Result<Self> {
        let (addr_mask, addrs, trailing) =
            parse_masked::<ADDR_LEN, Ipv6Addr>(bytes, TAG, "TargetIpv6AddressDescriptor")?;
        Ok(Self {
            addr_mask,
            addrs,
            trailing,
        })
    }
}

impl Serialize for TargetIpv6AddressDescriptor<'_> {
    type Error = crate::error::Error;
    fn serialized_len(&self) -> usize {
        HEADER_LEN + ADDR_LEN + self.addrs.len() * ADDR_LEN + self.trailing.len()
    }
    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        write_masked::<ADDR_LEN, Ipv6Addr>(buf, TAG, self.addr_mask, &self.addrs, self.trailing)
    }
}

impl<'a> crate::traits::DescriptorDef<'a> for TargetIpv6AddressDescriptor<'a> {
    const TAG: u8 = TAG;
    const NAME: &'static str = "TARGET_IPV6_ADDRESS";
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_and_round_trip() {
        let mask = Ipv6Addr::new(0xffff, 0xf0, 0, 0, 0, 0, 0, 0);
        let a1 = Ipv6Addr::new(0xfe80, 0, 0, 0, 0x93a3, 0xdea0, 0x2108, 0xb81e);
        let mut bytes = vec![TAG, 48];
        for a in [mask, a1, Ipv6Addr::LOCALHOST] {
            bytes.extend_from_slice(&a.octets());
        }
        let d = TargetIpv6AddressDescriptor::parse(&bytes).unwrap();
        assert_eq!(d.addr_mask, mask);
        assert_eq!(d.addrs, [a1, Ipv6Addr::LOCALHOST]);
        let mut out = vec![0u8; d.serialized_len()];
        d.serialize_into(&mut out).unwrap();
        assert_eq!(out, bytes);
    }

    #[test]
    fn partial_tail_round_trip_and_rejects_short_mask() {
        let mut bytes = vec![TAG, 18];
        bytes.extend_from_slice(&[0xAA; 18]);
        let d = TargetIpv6AddressDescriptor::parse(&bytes).unwrap();
        assert_eq!(d.trailing.len(), 2);
        let mut out = vec![0u8; d.serialized_len()];
        d.serialize_into(&mut out).unwrap();
        assert_eq!(out, bytes);
        assert!(TargetIpv6AddressDescriptor::parse(&[TAG, 3, 1, 2, 3]).is_err());
    }
}
