//! target_IPv6_source_slash_descriptor — ETSI EN 301 192 V1.7.1 §8.4.5.13
//! Table 31 (tag 0x12).
//!
//! Targets receivers by both IPv6 source and destination address, each with
//! a subnet mask in short "slash" form. INT target loop only (Table 19).

use crate::descriptors::entry_list::{Entry, list_len, parse_list, write_list};
use crate::error::Result;
use alloc::vec::Vec;
use broadcast_common::{Parse, Serialize};
use core::net::Ipv6Addr;

/// Descriptor tag for target_IPv6_source_slash_descriptor.
pub const TAG: u8 = 0x12;
const ENTRY_LEN: usize = 34;
const ADDR_LEN: usize = 16;

/// One source/destination address pair with slash masks.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct Ipv6SourceDestSlash {
    /// `IPv6_source_addr`.
    pub source_addr: Ipv6Addr,
    /// `IPv6_source_slash_mask`.
    pub source_slash_mask: u8,
    /// `IPv6_dest_addr`.
    pub dest_addr: Ipv6Addr,
    /// `IPv6_dest_slash_mask`.
    pub dest_slash_mask: u8,
}

impl Entry<ENTRY_LEN> for Ipv6SourceDestSlash {
    fn read(b: &[u8; ENTRY_LEN]) -> Self {
        let mut s = [0u8; ADDR_LEN];
        let mut d = [0u8; ADDR_LEN];
        s.copy_from_slice(&b[..ADDR_LEN]);
        d.copy_from_slice(&b[ADDR_LEN + 1..2 * ADDR_LEN + 1]);
        Self {
            source_addr: Ipv6Addr::from(s),
            source_slash_mask: b[ADDR_LEN],
            dest_addr: Ipv6Addr::from(d),
            dest_slash_mask: b[2 * ADDR_LEN + 1],
        }
    }
    fn write(&self, b: &mut [u8; ENTRY_LEN]) {
        b[..ADDR_LEN].copy_from_slice(&self.source_addr.octets());
        b[ADDR_LEN] = self.source_slash_mask;
        b[ADDR_LEN + 1..2 * ADDR_LEN + 1].copy_from_slice(&self.dest_addr.octets());
        b[2 * ADDR_LEN + 1] = self.dest_slash_mask;
    }
}

/// target_IPv6_source_slash_descriptor.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[cfg_attr(feature = "yoke", derive(yoke::Yokeable))]
pub struct TargetIpv6SourceSlashDescriptor<'a> {
    /// The source/destination entries.
    pub entries: Vec<Ipv6SourceDestSlash>,
    /// Bytes after the last whole entry inside `descriptor_length`; the spec
    /// defines none (kept for byte-identical re-serialization).
    pub trailing: &'a [u8],
}

impl<'a> Parse<'a> for TargetIpv6SourceSlashDescriptor<'a> {
    type Error = crate::error::Error;
    fn parse(bytes: &'a [u8]) -> Result<Self> {
        let (entries, trailing) = parse_list::<ENTRY_LEN, Ipv6SourceDestSlash>(
            bytes,
            TAG,
            "TargetIpv6SourceSlashDescriptor",
        )?;
        Ok(Self { entries, trailing })
    }
}

impl Serialize for TargetIpv6SourceSlashDescriptor<'_> {
    type Error = crate::error::Error;
    fn serialized_len(&self) -> usize {
        list_len(self.entries.len(), ENTRY_LEN, self.trailing.len())
    }
    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        write_list::<ENTRY_LEN, _>(buf, TAG, &self.entries, self.trailing)
    }
}

impl<'a> crate::traits::DescriptorDef<'a> for TargetIpv6SourceSlashDescriptor<'a> {
    const TAG: u8 = TAG;
    const NAME: &'static str = "TARGET_IPV6_SOURCE_SLASH";
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_and_round_trip() {
        // TSDuck test-022: fe80::ba3:a0:8:be/128 -> ::1/64.
        let s = Ipv6Addr::new(0xfe80, 0, 0, 0, 0x0ba3, 0x00a0, 0x0008, 0x00be);
        let mut bytes = vec![TAG, 34];
        bytes.extend_from_slice(&s.octets());
        bytes.push(128);
        bytes.extend_from_slice(&Ipv6Addr::LOCALHOST.octets());
        bytes.push(64);
        let d = TargetIpv6SourceSlashDescriptor::parse(&bytes).unwrap();
        assert_eq!(d.entries.len(), 1);
        assert_eq!(d.entries[0].source_addr, s);
        assert_eq!(d.entries[0].source_slash_mask, 128);
        assert_eq!(d.entries[0].dest_addr, Ipv6Addr::LOCALHOST);
        assert_eq!(d.entries[0].dest_slash_mask, 64);
        let mut out = vec![0u8; d.serialized_len()];
        d.serialize_into(&mut out).unwrap();
        assert_eq!(out, bytes);
    }

    #[test]
    fn empty_and_partial_tail_round_trip() {
        for bytes in [&[TAG, 0][..], &[TAG, 3, 7, 7, 7][..]] {
            let d = TargetIpv6SourceSlashDescriptor::parse(bytes).unwrap();
            let mut out = vec![0u8; d.serialized_len()];
            d.serialize_into(&mut out).unwrap();
            assert_eq!(out, bytes);
        }
        assert!(TargetIpv6SourceSlashDescriptor::parse(&[0x11, 0]).is_err());
    }
}
