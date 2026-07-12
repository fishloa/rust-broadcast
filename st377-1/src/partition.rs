//! Partition Pack — SMPTE ST 377-1:2019 §7.1-§7.4 (`docs/st377-1.md`), the
//! file's own backbone: every Header/Body/Footer Partition begins with one
//! of these.

extern crate alloc;

use alloc::vec::Vec;

use broadcast_common::{Parse, Serialize};

use crate::ber::{ber_length_size, decode_ber_length, encode_ber_length};
use crate::error::{Error, Result};
use crate::types::{UlBytes, parse_uid_batch, serialize_uid_batch, ul_bytes_from_prefix};

/// Fixed bytes 1-13 of every Partition Pack Key (Table 4), i.e. everything
/// except byte 8 (registry version, wildcard on parse), byte 14
/// ([`PartitionKind`]), byte 15 ([`PartitionStatus`]), and byte 16
/// (reserved, must be `0x00`).
const PARTITION_KEY_PREFIX: [u8; 7] = [0x06, 0x0E, 0x2B, 0x34, 0x02, 0x05, 0x01];
const PARTITION_KEY_MID: [u8; 4] = [0x0D, 0x01, 0x02, 0x01];

/// Which kind of Partition a Partition Pack Key identifies (Table 4 byte
/// 14; Tables 6-8).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[non_exhaustive]
pub enum PartitionKind {
    /// `0x02` — Header Partition (§7.2, Table 6).
    Header,
    /// `0x03` — Body Partition (§7.3, Table 7).
    Body,
    /// `0x04` — Footer Partition (§7.4, Table 8; never Open).
    Footer,
}

impl PartitionKind {
    /// The spec's own label.
    #[must_use]
    pub fn name(&self) -> &'static str {
        match self {
            Self::Header => "Header Partition",
            Self::Body => "Body Partition",
            Self::Footer => "Footer Partition",
        }
    }

    fn from_byte(b: u8) -> Result<Self> {
        match b {
            0x02 => Ok(Self::Header),
            0x03 => Ok(Self::Body),
            0x04 => Ok(Self::Footer),
            other => Err(Error::UnknownPartitionKind { byte: other }),
        }
    }

    fn to_byte(self) -> u8 {
        match self {
            Self::Header => 0x02,
            Self::Body => 0x03,
            Self::Footer => 0x04,
        }
    }
}

broadcast_common::impl_spec_display!(PartitionKind);

/// The Open/Closed × Complete/Incomplete status of a Partition (§6.2.3,
/// Table 4 byte 15).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[non_exhaustive]
pub enum PartitionStatus {
    /// `0x01` — Open and Incomplete.
    OpenIncomplete,
    /// `0x02` — Closed and Incomplete.
    ClosedIncomplete,
    /// `0x03` — Open and Complete.
    OpenComplete,
    /// `0x04` — Closed and Complete.
    ClosedComplete,
}

impl PartitionStatus {
    /// The spec's own label.
    #[must_use]
    pub fn name(&self) -> &'static str {
        match self {
            Self::OpenIncomplete => "open and incomplete",
            Self::ClosedIncomplete => "closed and incomplete",
            Self::OpenComplete => "open and complete",
            Self::ClosedComplete => "closed and complete",
        }
    }

    /// True for [`Self::OpenIncomplete`]/[`Self::OpenComplete`].
    #[must_use]
    pub fn is_open(&self) -> bool {
        matches!(self, Self::OpenIncomplete | Self::OpenComplete)
    }

    fn from_byte(b: u8) -> Result<Self> {
        match b {
            0x01 => Ok(Self::OpenIncomplete),
            0x02 => Ok(Self::ClosedIncomplete),
            0x03 => Ok(Self::OpenComplete),
            0x04 => Ok(Self::ClosedComplete),
            other => Err(Error::UnknownPartitionStatus { byte: other }),
        }
    }

    fn to_byte(self) -> u8 {
        match self {
            Self::OpenIncomplete => 0x01,
            Self::ClosedIncomplete => 0x02,
            Self::OpenComplete => 0x03,
            Self::ClosedComplete => 0x04,
        }
    }
}

broadcast_common::impl_spec_display!(PartitionStatus);

/// The Partition Pack — SMPTE ST 377-1:2019 §7.1, Table 4 (Key) + Table 5
/// (Value). Covers the Header/Body/Footer variants (§7.2-7.4); which one a
/// given instance is lives in `kind`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PartitionPack {
    /// Which kind of Partition this is (Table 4 byte 14).
    pub kind: PartitionKind,
    /// Open/Closed × Complete/Incomplete (Table 4 byte 15).
    pub status: PartitionStatus,
    /// Fixed `0x0001` for this revision.
    pub major_version: u16,
    /// Fixed `0x0003` for this revision (`0x0002` = ST 377M-2004).
    pub minor_version: u16,
    /// KLV Alignment Grid size in bytes: `0`=undefined, `1`=byte-aligned,
    /// `2..=1048576` a valid grid, `>1048576` forbidden (§6.4.1).
    pub kag_size: u32,
    /// Byte offset of this Partition from the start of the Header
    /// Partition (always 0 for the Header Partition itself).
    pub this_partition: u64,
    /// Byte offset of the previous Partition (0 for the first Partition
    /// after the Header).
    pub previous_partition: u64,
    /// Byte offset of the Footer Partition (0 if not yet known).
    pub footer_partition: u64,
    /// Bytes used for Header Metadata + Primer Pack in this Partition (0 if
    /// none).
    pub header_byte_count: u64,
    /// Bytes used for Index Table Segments in this Partition (0 if none).
    pub index_byte_count: u64,
    /// Index Table Segment stream ID in this Partition (0 = none).
    pub index_sid: u32,
    /// Byte offset of the Essence Container segment in this Partition,
    /// relative to the start of its BodySID stream.
    pub body_offset: u64,
    /// Essence Container stream ID in this Partition (0 = none).
    pub body_sid: u32,
    /// Operational Pattern UL (copy of the Preface's own value).
    pub operational_pattern: UlBytes,
    /// Batch of Essence Container ULs used in/referenced by this file.
    pub essence_containers: Vec<UlBytes>,
}

impl PartitionPack {
    /// Build the 16-byte Key for `kind`/`status` (Table 4/6/7/8).
    ///
    /// Byte numbering (1-indexed per the spec) -> 0-indexed array position:
    /// bytes 1-7 = the fixed prefix `06 0E 2B 34 02 05 01`; byte 8 =
    /// registry version (wildcard on parse, written as `0x01`); bytes 9-12
    /// = the fixed mid section `0D 01 02 01`; byte 13 = Structure Kind
    /// (`0x01`, part of the fixed prefix); byte 14 = Partition Kind
    /// ([`PartitionKind`]); byte 15 = Partition Status ([`PartitionStatus`]);
    /// byte 16 = reserved (`0x00`).
    #[must_use]
    pub fn key(kind: PartitionKind, status: PartitionStatus) -> UlBytes {
        let mut key = [0u8; 16];
        key[0..7].copy_from_slice(&PARTITION_KEY_PREFIX);
        key[7] = 0x01;
        key[8..12].copy_from_slice(&PARTITION_KEY_MID);
        key[12] = 0x01; // Structure Kind
        key[13] = kind.to_byte();
        key[14] = status.to_byte();
        key[15] = 0x00;
        key
    }

    /// True if `key` matches the Partition Pack Key (Table 4): the fixed
    /// prefix bytes AND byte 14 one of the three valid [`PartitionKind`]
    /// values. Useful to recognize a Partition Pack while walking a file's
    /// top-level KLV items, before committing to a full parse.
    ///
    /// The fixed prefix alone is **not** sufficient to distinguish a
    /// Partition Pack from a [`crate::PrimerPack`] or
    /// [`crate::RandomIndexPack`] Key: all three share the same "Defined-
    /// Length Pack, Set/Pack Registry" family (bytes 1-13, Tables 4/13/29)
    /// and differ only in byte 14 (`0x02`/`0x03`/`0x04` Partition vs. `0x05`
    /// Primer Pack vs. `0x11` Random Index Pack) — so byte 14 must be
    /// checked too, not just the shared prefix.
    #[must_use]
    pub fn is_partition_key(key: &UlBytes) -> bool {
        key[0..7] == PARTITION_KEY_PREFIX
            && key[8..12] == PARTITION_KEY_MID
            && key[12] == 0x01
            && PartitionKind::from_byte(key[13]).is_ok()
    }

    fn parse_key(key: &UlBytes) -> Result<(PartitionKind, PartitionStatus)> {
        if key[0..7] != PARTITION_KEY_PREFIX || key[8..12] != PARTITION_KEY_MID || key[12] != 0x01 {
            return Err(Error::KeyPrefixMismatch {
                what: "Partition Pack (Table 4)",
            });
        }
        let kind = PartitionKind::from_byte(key[13])?;
        let status = PartitionStatus::from_byte(key[14])?;
        if kind == PartitionKind::Footer && status.is_open() {
            return Err(Error::OpenFooterPartition { byte: key[14] });
        }
        Ok((kind, status))
    }
}

impl<'a> Parse<'a> for PartitionPack {
    type Error = Error;

    fn parse(bytes: &'a [u8]) -> Result<Self> {
        if bytes.len() < 16 {
            return Err(Error::BufferTooShort {
                need: 16,
                have: bytes.len(),
                what: "Partition Pack key",
            });
        }
        let key: UlBytes = ul_bytes_from_prefix(bytes);
        let (kind, status) = Self::parse_key(&key)?;

        let (len, len_size) = decode_ber_length(&bytes[16..])?;
        let value_start = 16 + len_size;
        let len = len as usize;
        let value_end = value_start.checked_add(len).ok_or(Error::BufferTooShort {
            need: usize::MAX,
            have: bytes.len(),
            what: "Partition Pack value (length overflow)",
        })?;
        if bytes.len() < value_end {
            return Err(Error::BufferTooShort {
                need: value_end,
                have: bytes.len(),
                what: "Partition Pack value",
            });
        }
        let v = &bytes[value_start..value_end];

        const FIXED_FIELDS_LEN: usize = 2 + 2 + 4 + 8 + 8 + 8 + 8 + 8 + 4 + 8 + 4 + 16;
        if v.len() < FIXED_FIELDS_LEN {
            return Err(Error::BufferTooShort {
                need: FIXED_FIELDS_LEN,
                have: v.len(),
                what: "Partition Pack fixed fields",
            });
        }
        let u16_at = |o: usize| u16::from_be_bytes([v[o], v[o + 1]]);
        let u32_at = |o: usize| u32::from_be_bytes([v[o], v[o + 1], v[o + 2], v[o + 3]]);
        let u64_at = |o: usize| {
            u64::from_be_bytes([
                v[o],
                v[o + 1],
                v[o + 2],
                v[o + 3],
                v[o + 4],
                v[o + 5],
                v[o + 6],
                v[o + 7],
            ])
        };

        let major_version = u16_at(0);
        let minor_version = u16_at(2);
        let kag_size = u32_at(4);
        let this_partition = u64_at(8);
        let previous_partition = u64_at(16);
        let footer_partition = u64_at(24);
        let header_byte_count = u64_at(32);
        let index_byte_count = u64_at(40);
        let index_sid = u32_at(48);
        let body_offset = u64_at(52);
        let body_sid = u32_at(60);
        let operational_pattern: UlBytes = ul_bytes_from_prefix(&v[64..]);
        let essence_containers = parse_uid_batch(&v[80..])?;

        Ok(PartitionPack {
            kind,
            status,
            major_version,
            minor_version,
            kag_size,
            this_partition,
            previous_partition,
            footer_partition,
            header_byte_count,
            index_byte_count,
            index_sid,
            body_offset,
            body_sid,
            operational_pattern,
            essence_containers,
        })
    }
}

impl Serialize for PartitionPack {
    type Error = Error;

    fn serialized_len(&self) -> usize {
        let value_len =
            2 + 2 + 4 + 8 + 8 + 8 + 8 + 8 + 4 + 8 + 4 + 16 + 8 + self.essence_containers.len() * 16;
        16 + ber_length_size(value_len as u64) + value_len
    }

    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        if self.kind == PartitionKind::Footer && self.status.is_open() {
            return Err(Error::OpenFooterPartition {
                byte: self.status.to_byte(),
            });
        }
        let total = self.serialized_len();
        if buf.len() < total {
            return Err(Error::BufferTooShort {
                need: total,
                have: buf.len(),
                what: "Partition Pack",
            });
        }
        buf[0..16].copy_from_slice(&Self::key(self.kind, self.status));
        let batch = serialize_uid_batch(&self.essence_containers);
        let value_len = 2 + 2 + 4 + 8 + 8 + 8 + 8 + 8 + 4 + 8 + 4 + 16 + batch.len();
        let len_size = encode_ber_length(value_len as u64, &mut buf[16..])?;
        let mut pos = 16 + len_size;

        buf[pos..pos + 2].copy_from_slice(&self.major_version.to_be_bytes());
        pos += 2;
        buf[pos..pos + 2].copy_from_slice(&self.minor_version.to_be_bytes());
        pos += 2;
        buf[pos..pos + 4].copy_from_slice(&self.kag_size.to_be_bytes());
        pos += 4;
        buf[pos..pos + 8].copy_from_slice(&self.this_partition.to_be_bytes());
        pos += 8;
        buf[pos..pos + 8].copy_from_slice(&self.previous_partition.to_be_bytes());
        pos += 8;
        buf[pos..pos + 8].copy_from_slice(&self.footer_partition.to_be_bytes());
        pos += 8;
        buf[pos..pos + 8].copy_from_slice(&self.header_byte_count.to_be_bytes());
        pos += 8;
        buf[pos..pos + 8].copy_from_slice(&self.index_byte_count.to_be_bytes());
        pos += 8;
        buf[pos..pos + 4].copy_from_slice(&self.index_sid.to_be_bytes());
        pos += 4;
        buf[pos..pos + 8].copy_from_slice(&self.body_offset.to_be_bytes());
        pos += 8;
        buf[pos..pos + 4].copy_from_slice(&self.body_sid.to_be_bytes());
        pos += 4;
        buf[pos..pos + 16].copy_from_slice(&self.operational_pattern);
        pos += 16;
        buf[pos..pos + batch.len()].copy_from_slice(&batch);
        pos += batch.len();
        Ok(pos)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample(kind: PartitionKind, status: PartitionStatus) -> PartitionPack {
        PartitionPack {
            kind,
            status,
            major_version: 1,
            minor_version: 3,
            kag_size: 512,
            this_partition: 0,
            previous_partition: 0,
            footer_partition: 12345,
            header_byte_count: 1000,
            index_byte_count: 0,
            index_sid: 0,
            body_offset: 0,
            body_sid: 1,
            operational_pattern: [0xAA; 16],
            essence_containers: alloc::vec![[0xBBu8; 16]],
        }
    }

    #[test]
    fn header_partition_round_trip() {
        let pp = sample(PartitionKind::Header, PartitionStatus::ClosedComplete);
        let mut buf = alloc::vec![0u8; pp.serialized_len()];
        pp.serialize_into(&mut buf).unwrap();
        let parsed = PartitionPack::parse(&buf).unwrap();
        assert_eq!(parsed, pp);
    }

    #[test]
    fn body_partition_round_trip_empty_essence_containers() {
        let mut pp = sample(PartitionKind::Body, PartitionStatus::OpenIncomplete);
        pp.essence_containers.clear();
        let mut buf = alloc::vec![0u8; pp.serialized_len()];
        pp.serialize_into(&mut buf).unwrap();
        let parsed = PartitionPack::parse(&buf).unwrap();
        assert_eq!(parsed, pp);
        assert!(parsed.essence_containers.is_empty());
    }

    #[test]
    fn footer_partition_cannot_be_open() {
        let pp = sample(PartitionKind::Footer, PartitionStatus::OpenComplete);
        let mut buf = alloc::vec![0u8; pp.serialized_len()];
        assert!(matches!(
            pp.serialize_into(&mut buf),
            Err(Error::OpenFooterPartition { .. })
        ));
    }

    #[test]
    fn parse_rejects_open_footer_key() {
        let key = PartitionPack::key(PartitionKind::Footer, PartitionStatus::OpenComplete);
        let mut bytes = alloc::vec::Vec::new();
        bytes.extend_from_slice(&key);
        bytes.push(0); // zero-length value (short-form BER)
        assert!(matches!(
            PartitionPack::parse(&bytes),
            Err(Error::OpenFooterPartition { .. })
        ));
    }

    #[test]
    fn unknown_kind_byte_rejected() {
        assert!(matches!(
            PartitionKind::from_byte(0xFF),
            Err(Error::UnknownPartitionKind { byte: 0xFF })
        ));
    }
}
