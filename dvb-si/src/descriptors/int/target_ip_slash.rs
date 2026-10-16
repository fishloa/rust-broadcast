//! target_IP_slash_descriptor — ETSI EN 301 192 V1.7.1 §8.4.5.9 Table 27
//! (tag 0x0F).
//!
//! Targets receivers by IPv4 address with a subnet mask in the short "slash"
//! form (`XXX.XXX.XXX.XXX/NN`): host or subnet announcements in unicast,
//! address ranges in multicast. INT target loop only (Table 19).

use crate::descriptors::entry_list::{list_len, parse_list, write_list, Entry};
use crate::error::Result;
use alloc::vec::Vec;
use broadcast_common::{Parse, Serialize};
use core::net::Ipv4Addr;

/// Descriptor tag for target_IP_slash_descriptor.
pub const TAG: u8 = 0x0F;
const ENTRY_LEN: usize = 5;

/// One `IPv4_addr` / `IPv4_slash_mask` entry.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct Ipv4Slash {
    /// `IPv4_addr` — unicast/multicast/broadcast address.
    pub addr: Ipv4Addr,
    /// `IPv4_slash_mask` — the mask in slash notation (e.g. 24 for `/24`).
    pub slash_mask: u8,
}

impl Entry<ENTRY_LEN> for Ipv4Slash {
    fn read(b: &[u8; ENTRY_LEN]) -> Self {
        Self {
            addr: Ipv4Addr::new(b[0], b[1], b[2], b[3]),
            slash_mask: b[4],
        }
    }
    fn write(&self, b: &mut [u8; ENTRY_LEN]) {
        b[..4].copy_from_slice(&self.addr.octets());
        b[4] = self.slash_mask;
    }
}

/// target_IP_slash_descriptor.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[cfg_attr(feature = "yoke", derive(yoke::Yokeable))]
pub struct TargetIpSlashDescriptor<'a> {
    /// The address/mask entries.
    pub entries: Vec<Ipv4Slash>,
    /// Bytes after the last whole entry inside `descriptor_length`; the spec
    /// defines none (kept for byte-identical re-serialization).
    pub trailing: &'a [u8],
}

impl<'a> Parse<'a> for TargetIpSlashDescriptor<'a> {
    type Error = crate::error::Error;
    fn parse(bytes: &'a [u8]) -> Result<Self> {
        let (entries, trailing) =
            parse_list::<ENTRY_LEN, Ipv4Slash>(bytes, TAG, "TargetIpSlashDescriptor")?;
        Ok(Self { entries, trailing })
    }
}

impl Serialize for TargetIpSlashDescriptor<'_> {
    type Error = crate::error::Error;
    fn serialized_len(&self) -> usize {
        list_len(self.entries.len(), ENTRY_LEN, self.trailing.len())
    }
    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        write_list::<ENTRY_LEN, _>(buf, TAG, &self.entries, self.trailing)
    }
}

impl<'a> crate::traits::DescriptorDef<'a> for TargetIpSlashDescriptor<'a> {
    const TAG: u8 = TAG;
    const NAME: &'static str = "TARGET_IP_SLASH";
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_and_round_trip() {
        // TSDuck test-022: 1.2.3.4/16 and 5.6.7.8/24.
        let bytes = [TAG, 10, 1, 2, 3, 4, 16, 5, 6, 7, 8, 24];
        let d = TargetIpSlashDescriptor::parse(&bytes).unwrap();
        assert_eq!(
            d.entries,
            [
                Ipv4Slash { addr: Ipv4Addr::new(1, 2, 3, 4), slash_mask: 16 },
                Ipv4Slash { addr: Ipv4Addr::new(5, 6, 7, 8), slash_mask: 24 },
            ]
        );
        let mut out = vec![0u8; d.serialized_len()];
        d.serialize_into(&mut out).unwrap();
        assert_eq!(out, bytes);
    }

    #[test]
    fn empty_and_partial_tail_round_trip() {
        for bytes in [&[TAG, 0][..], &[TAG, 7, 1, 2, 3, 4, 8, 9, 9][..]] {
            let d = TargetIpSlashDescriptor::parse(bytes).unwrap();
            let mut out = vec![0u8; d.serialized_len()];
            d.serialize_into(&mut out).unwrap();
            assert_eq!(out, bytes);
        }
        assert!(TargetIpSlashDescriptor::parse(&[0x10, 0]).is_err());
    }
}
