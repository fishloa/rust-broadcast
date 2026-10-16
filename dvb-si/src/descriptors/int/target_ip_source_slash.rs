//! target_IP_source_slash_descriptor — ETSI EN 301 192 V1.7.1 §8.4.5.10
//! Table 28 (tag 0x10).
//!
//! Targets receivers by both IPv4 source and destination address, each with
//! a subnet mask in short "slash" form. INT target loop only (Table 19).

use crate::descriptors::entry_list::{list_len, parse_list, write_list, Entry};
use crate::error::Result;
use alloc::vec::Vec;
use broadcast_common::{Parse, Serialize};
use core::net::Ipv4Addr;

/// Descriptor tag for target_IP_source_slash_descriptor.
pub const TAG: u8 = 0x10;
const ENTRY_LEN: usize = 10;

/// One source/destination address pair with slash masks.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct Ipv4SourceDestSlash {
    /// `IPv4_source_addr`.
    pub source_addr: Ipv4Addr,
    /// `IPv4_source_slash_mask`.
    pub source_slash_mask: u8,
    /// `IPv4_dest_addr`.
    pub dest_addr: Ipv4Addr,
    /// `IPv4_dest_slash_mask`.
    pub dest_slash_mask: u8,
}

impl Entry<ENTRY_LEN> for Ipv4SourceDestSlash {
    fn read(b: &[u8; ENTRY_LEN]) -> Self {
        Self {
            source_addr: Ipv4Addr::new(b[0], b[1], b[2], b[3]),
            source_slash_mask: b[4],
            dest_addr: Ipv4Addr::new(b[5], b[6], b[7], b[8]),
            dest_slash_mask: b[9],
        }
    }
    fn write(&self, b: &mut [u8; ENTRY_LEN]) {
        b[..4].copy_from_slice(&self.source_addr.octets());
        b[4] = self.source_slash_mask;
        b[5..9].copy_from_slice(&self.dest_addr.octets());
        b[9] = self.dest_slash_mask;
    }
}

/// target_IP_source_slash_descriptor.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[cfg_attr(feature = "yoke", derive(yoke::Yokeable))]
pub struct TargetIpSourceSlashDescriptor<'a> {
    /// The source/destination entries.
    pub entries: Vec<Ipv4SourceDestSlash>,
    /// Bytes after the last whole entry inside `descriptor_length`; the spec
    /// defines none (kept for byte-identical re-serialization).
    pub trailing: &'a [u8],
}

impl<'a> Parse<'a> for TargetIpSourceSlashDescriptor<'a> {
    type Error = crate::error::Error;
    fn parse(bytes: &'a [u8]) -> Result<Self> {
        let (entries, trailing) = parse_list::<ENTRY_LEN, Ipv4SourceDestSlash>(
            bytes,
            TAG,
            "TargetIpSourceSlashDescriptor",
        )?;
        Ok(Self { entries, trailing })
    }
}

impl Serialize for TargetIpSourceSlashDescriptor<'_> {
    type Error = crate::error::Error;
    fn serialized_len(&self) -> usize {
        list_len(self.entries.len(), ENTRY_LEN, self.trailing.len())
    }
    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        write_list::<ENTRY_LEN, _>(buf, TAG, &self.entries, self.trailing)
    }
}

impl<'a> crate::traits::DescriptorDef<'a> for TargetIpSourceSlashDescriptor<'a> {
    const TAG: u8 = TAG;
    const NAME: &'static str = "TARGET_IP_SOURCE_SLASH";
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_and_round_trip() {
        // TSDuck test-022: 1.2.3.4/8 -> 5.6.7.8/16.
        let bytes = [TAG, 10, 1, 2, 3, 4, 8, 5, 6, 7, 8, 16];
        let d = TargetIpSourceSlashDescriptor::parse(&bytes).unwrap();
        assert_eq!(d.entries.len(), 1);
        assert_eq!(d.entries[0].source_addr, Ipv4Addr::new(1, 2, 3, 4));
        assert_eq!(d.entries[0].source_slash_mask, 8);
        assert_eq!(d.entries[0].dest_addr, Ipv4Addr::new(5, 6, 7, 8));
        assert_eq!(d.entries[0].dest_slash_mask, 16);
        let mut out = vec![0u8; d.serialized_len()];
        d.serialize_into(&mut out).unwrap();
        assert_eq!(out, bytes);
    }

    #[test]
    fn empty_and_partial_tail_round_trip() {
        for bytes in [&[TAG, 0][..], &[TAG, 4, 1, 2, 3, 4][..]] {
            let d = TargetIpSourceSlashDescriptor::parse(bytes).unwrap();
            let mut out = vec![0u8; d.serialized_len()];
            d.serialize_into(&mut out).unwrap();
            assert_eq!(out, bytes);
        }
        assert!(TargetIpSourceSlashDescriptor::parse(&[0x0F, 0]).is_err());
    }
}
