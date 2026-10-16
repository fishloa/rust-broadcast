//! RNT scan descriptor — ETSI TS 102 323 V1.4.1 §5.3.7 Table 14 (RNT tag
//! 0x42).
//!
//! References transport streams that carry RNTs: a list of
//! (`transport_stream_id`, `original_network_id`, `scan_weighting`) entries.
//! Permitted in the common descriptor loop of an RNT section (Table 4); tag
//! 0x42 here overrides the SI meaning of that tag (`stuffing_descriptor`)
//! inside an RNT.

use crate::descriptors::entry_list::{Entry, list_len, parse_list, write_list};
use crate::error::Result;
use alloc::vec::Vec;
use broadcast_common::{Parse, Serialize};

/// Descriptor tag for RNT_scan_descriptor (RNT namespace).
pub const TAG: u8 = 0x42;
const ENTRY_LEN: usize = 5;

/// One scan entry.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct RntScanEntry {
    /// `transport_stream_id` of the referenced transport stream.
    pub transport_stream_id: u16,
    /// `original_network_id` of the referenced transport stream.
    pub original_network_id: u16,
    /// `scan_weighting` — intended tuning order; larger weights are tried
    /// before smaller ones.
    pub scan_weighting: u8,
}

impl Entry<ENTRY_LEN> for RntScanEntry {
    fn read(b: &[u8; ENTRY_LEN]) -> Self {
        Self {
            transport_stream_id: u16::from_be_bytes([b[0], b[1]]),
            original_network_id: u16::from_be_bytes([b[2], b[3]]),
            scan_weighting: b[4],
        }
    }
    fn write(&self, b: &mut [u8; ENTRY_LEN]) {
        b[..2].copy_from_slice(&self.transport_stream_id.to_be_bytes());
        b[2..4].copy_from_slice(&self.original_network_id.to_be_bytes());
        b[4] = self.scan_weighting;
    }
}

/// RNT scan descriptor.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[cfg_attr(feature = "yoke", derive(yoke::Yokeable))]
pub struct RntScanDescriptor<'a> {
    /// The referenced transport streams.
    pub entries: Vec<RntScanEntry>,
    /// Bytes after the last whole entry inside `descriptor_length`; the spec
    /// defines none (kept for byte-identical re-serialization).
    pub trailing: &'a [u8],
}

impl<'a> Parse<'a> for RntScanDescriptor<'a> {
    type Error = crate::error::Error;
    fn parse(bytes: &'a [u8]) -> Result<Self> {
        let (entries, trailing) =
            parse_list::<ENTRY_LEN, RntScanEntry>(bytes, TAG, "RntScanDescriptor")?;
        Ok(Self { entries, trailing })
    }
}

impl Serialize for RntScanDescriptor<'_> {
    type Error = crate::error::Error;
    fn serialized_len(&self) -> usize {
        list_len(self.entries.len(), ENTRY_LEN, self.trailing.len())
    }
    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        write_list::<ENTRY_LEN, _>(buf, TAG, &self.entries, self.trailing)
    }
}

impl<'a> crate::traits::DescriptorDef<'a> for RntScanDescriptor<'a> {
    const TAG: u8 = TAG;
    const NAME: &'static str = "RNT_SCAN";
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_and_round_trip() {
        let bytes = [
            TAG, 10, 0x12, 0x34, 0x56, 0x78, 9, 0xAB, 0xCD, 0xEF, 0x01, 3,
        ];
        let d = RntScanDescriptor::parse(&bytes).unwrap();
        assert_eq!(
            d.entries,
            [
                RntScanEntry {
                    transport_stream_id: 0x1234,
                    original_network_id: 0x5678,
                    scan_weighting: 9
                },
                RntScanEntry {
                    transport_stream_id: 0xABCD,
                    original_network_id: 0xEF01,
                    scan_weighting: 3
                },
            ]
        );
        let mut out = vec![0u8; d.serialized_len()];
        d.serialize_into(&mut out).unwrap();
        assert_eq!(out, bytes);
    }

    #[test]
    fn empty_and_partial_tail_round_trip() {
        for bytes in [&[TAG, 0][..], &[TAG, 3, 1, 2, 3][..]] {
            let d = RntScanDescriptor::parse(bytes).unwrap();
            let mut out = vec![0u8; d.serialized_len()];
            d.serialize_into(&mut out).unwrap();
            assert_eq!(out, bytes);
        }
        assert!(RntScanDescriptor::parse(&[0x41, 0]).is_err());
    }
}
