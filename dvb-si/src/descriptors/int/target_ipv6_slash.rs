//! target_IPv6_slash_descriptor — ETSI EN 301 192 V1.7.1 §8.4.5.12
//! Table 30 (tag 0x11).
//!
//! Targets receivers by IPv6 address with a subnet mask in short "slash"
//! form (an IP VLSM function). INT target loop only (Table 19).

use crate::descriptors::entry_list::{Entry, list_len, parse_list, write_list};
use crate::error::Result;
use alloc::vec::Vec;
use broadcast_common::{Parse, Serialize};
use core::net::Ipv6Addr;

/// Descriptor tag for target_IPv6_slash_descriptor.
pub const TAG: u8 = 0x11;
const ENTRY_LEN: usize = 17;

/// One `IPv6_addr` / `IPv6_slash_mask` entry.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct Ipv6Slash {
    /// `IPv6_addr` — unicast/multicast/anycast address.
    pub addr: Ipv6Addr,
    /// `IPv6_slash_mask` — the mask in slash notation (e.g. 64 for `/64`).
    pub slash_mask: u8,
}

impl Entry<ENTRY_LEN> for Ipv6Slash {
    fn read(b: &[u8; ENTRY_LEN]) -> Self {
        let mut a = [0u8; 16];
        a.copy_from_slice(&b[..16]);
        Self {
            addr: Ipv6Addr::from(a),
            slash_mask: b[16],
        }
    }
    fn write(&self, b: &mut [u8; ENTRY_LEN]) {
        b[..16].copy_from_slice(&self.addr.octets());
        b[16] = self.slash_mask;
    }
}

/// target_IPv6_slash_descriptor.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[cfg_attr(feature = "yoke", derive(yoke::Yokeable))]
pub struct TargetIpv6SlashDescriptor<'a> {
    /// The address/mask entries.
    pub entries: Vec<Ipv6Slash>,
    /// Bytes after the last whole entry inside `descriptor_length`; the spec
    /// defines none (kept for byte-identical re-serialization).
    pub trailing: &'a [u8],
}

impl<'a> Parse<'a> for TargetIpv6SlashDescriptor<'a> {
    type Error = crate::error::Error;
    fn parse(bytes: &'a [u8]) -> Result<Self> {
        let (entries, trailing) =
            parse_list::<ENTRY_LEN, Ipv6Slash>(bytes, TAG, "TargetIpv6SlashDescriptor")?;
        Ok(Self { entries, trailing })
    }
}

impl Serialize for TargetIpv6SlashDescriptor<'_> {
    type Error = crate::error::Error;
    fn serialized_len(&self) -> usize {
        list_len(self.entries.len(), ENTRY_LEN, self.trailing.len())
    }
    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        write_list::<ENTRY_LEN, _>(buf, TAG, &self.entries, self.trailing)
    }
}

impl<'a> crate::traits::DescriptorDef<'a> for TargetIpv6SlashDescriptor<'a> {
    const TAG: u8 = TAG;
    const NAME: &'static str = "TARGET_IPV6_SLASH";
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_and_round_trip() {
        // TSDuck test-022: 1:2:3:4:5:6:7:8/72.
        let a = Ipv6Addr::new(1, 2, 3, 4, 5, 6, 7, 8);
        let mut bytes = vec![TAG, 17];
        bytes.extend_from_slice(&a.octets());
        bytes.push(72);
        let d = TargetIpv6SlashDescriptor::parse(&bytes).unwrap();
        assert_eq!(
            d.entries,
            [Ipv6Slash {
                addr: a,
                slash_mask: 72
            }]
        );
        let mut out = vec![0u8; d.serialized_len()];
        d.serialize_into(&mut out).unwrap();
        assert_eq!(out, bytes);
    }

    #[test]
    fn empty_and_partial_tail_round_trip() {
        for bytes in [&[TAG, 0][..], &[TAG, 2, 7, 7][..]] {
            let d = TargetIpv6SlashDescriptor::parse(bytes).unwrap();
            let mut out = vec![0u8; d.serialized_len()];
            d.serialize_into(&mut out).unwrap();
            assert_eq!(out, bytes);
        }
        assert!(TargetIpv6SlashDescriptor::parse(&[0x12, 0]).is_err());
    }
}
