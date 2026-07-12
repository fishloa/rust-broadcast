//! Primer Pack — SMPTE ST 377-1:2019 §9.2, Tables 13-15 (`docs/st377-1.md`):
//! the per-Partition lookup table mapping every 2-byte local tag used in
//! this Partition's Header Metadata to its full UL/UUID.

extern crate alloc;

use alloc::vec::Vec;

use broadcast_common::{Parse, Serialize};

use crate::ber::{ber_length_size, decode_ber_length, encode_ber_length};
use crate::error::{Error, Result};
use crate::types::UlBytes;

/// Fixed bytes 1-13 of the Primer Pack Key (Table 13), i.e. everything
/// except byte 8 (registry version, wildcard on parse).
const PRIMER_KEY_PREFIX: [u8; 7] = [0x06, 0x0E, 0x2B, 0x34, 0x02, 0x05, 0x01];
const PRIMER_KEY_MID: [u8; 4] = [0x0D, 0x01, 0x02, 0x01];
/// Byte 14 (Set/Pack Kind = Primer Pack) and byte 15 (Primer version).
const PRIMER_KEY_TAIL: [u8; 2] = [0x05, 0x01];

/// The size in bytes of one `LocalTagEntry` (Table 15): a 2-byte tag plus a
/// 16-byte AUID.
const LOCAL_TAG_ENTRY_LEN: u32 = 18;

/// The Primer Pack — SMPTE ST 377-1:2019 §9.2, Tables 13-15: a Batch of
/// `{local_tag: u16, uid: AUID}` entries, scoped to the single Partition
/// that contains it (§9.2 — never accumulated across Partitions).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct PrimerPack {
    /// Every local-tag -> UL/UUID mapping in this Partition's Header
    /// Metadata.
    pub entries: Vec<(u16, UlBytes)>,
}

impl PrimerPack {
    /// Build the 16-byte Primer Pack Key (Table 13).
    #[must_use]
    pub fn key() -> UlBytes {
        let mut key = [0u8; 16];
        key[0..7].copy_from_slice(&PRIMER_KEY_PREFIX);
        key[7] = 0x01; // registry version
        key[8..12].copy_from_slice(&PRIMER_KEY_MID);
        key[12] = 0x01; // Structure Kind
        key[13..15].copy_from_slice(&PRIMER_KEY_TAIL);
        key[15] = 0x00; // reserved
        key
    }

    /// True if `key` is the Primer Pack Key (Table 13), ignoring byte 8
    /// (registry version, wildcard).
    #[must_use]
    pub fn is_primer_key(key: &UlBytes) -> bool {
        key[0..7] == PRIMER_KEY_PREFIX
            && key[8..12] == PRIMER_KEY_MID
            && key[12] == 0x01
            && key[13..15] == PRIMER_KEY_TAIL
    }

    fn check_key(key: &UlBytes) -> Result<()> {
        if Self::is_primer_key(key) {
            Ok(())
        } else {
            Err(Error::KeyPrefixMismatch {
                what: "Primer Pack (Table 13)",
            })
        }
    }

    /// Resolve a UL/UUID to its local tag in this Primer Pack, if present.
    /// Used to decode "dyn" (dynamically-allocated-tag) properties whose
    /// static tag the spec deliberately does not fix (`docs/st377-1.md`'s
    /// Annex A tables) — e.g. `Preface`'s `IsRIPPresent`.
    #[must_use]
    pub fn resolve_ul(&self, ul: &UlBytes) -> Option<u16> {
        self.entries.iter().find(|(_, u)| u == ul).map(|(t, _)| *t)
    }

    /// Look up the UL/UUID for a local tag, if present.
    #[must_use]
    pub fn resolve_tag(&self, tag: u16) -> Option<UlBytes> {
        self.entries
            .iter()
            .find(|(t, _)| *t == tag)
            .map(|(_, u)| *u)
    }

    /// Parse a Primer Pack (Key + Length + Value) from `bytes`, validating
    /// the Key and consuming exactly one KLV item's worth. Also usable in a
    /// stream context via the returned consumed-byte count.
    pub fn parse_prefix(bytes: &[u8]) -> Result<(Self, usize)> {
        if bytes.len() < 16 {
            return Err(Error::BufferTooShort {
                need: 16,
                have: bytes.len(),
                what: "Primer Pack key",
            });
        }
        let key: UlBytes = bytes[0..16].try_into().expect("16-byte slice");
        Self::check_key(&key)?;

        let (len, len_size) = decode_ber_length(&bytes[16..])?;
        let value_start = 16 + len_size;
        let len = len as usize;
        let value_end = value_start.checked_add(len).ok_or(Error::BufferTooShort {
            need: usize::MAX,
            have: bytes.len(),
            what: "Primer Pack value (length overflow)",
        })?;
        if bytes.len() < value_end {
            return Err(Error::BufferTooShort {
                need: value_end,
                have: bytes.len(),
                what: "Primer Pack value",
            });
        }
        let v = &bytes[value_start..value_end];
        if v.len() < 8 {
            return Err(Error::InvalidBatchHeader {
                count: 0,
                item_len: 0,
                buffer_len: v.len(),
            });
        }
        let count = u32::from_be_bytes([v[0], v[1], v[2], v[3]]);
        let item_len = u32::from_be_bytes([v[4], v[5], v[6], v[7]]);
        let body = &v[8..];
        if item_len != LOCAL_TAG_ENTRY_LEN || body.len() != count as usize * 18 {
            return Err(Error::InvalidBatchHeader {
                count,
                item_len,
                buffer_len: body.len(),
            });
        }
        let mut entries = Vec::with_capacity(count as usize);
        for chunk in body.chunks_exact(18) {
            let tag = u16::from_be_bytes([chunk[0], chunk[1]]);
            let uid: UlBytes = chunk[2..18].try_into().expect("16 bytes");
            entries.push((tag, uid));
        }
        Ok((PrimerPack { entries }, value_end))
    }
}

impl<'a> Parse<'a> for PrimerPack {
    type Error = Error;

    fn parse(bytes: &'a [u8]) -> Result<Self> {
        let (pack, consumed) = Self::parse_prefix(bytes)?;
        if consumed != bytes.len() {
            return Err(Error::BufferTooShort {
                need: consumed,
                have: bytes.len(),
                what: "Primer Pack (trailing bytes after exact-fit parse)",
            });
        }
        Ok(pack)
    }
}

impl Serialize for PrimerPack {
    type Error = Error;

    fn serialized_len(&self) -> usize {
        let value_len = 8 + self.entries.len() * 18;
        16 + ber_length_size(value_len as u64) + value_len
    }

    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        let total = self.serialized_len();
        if buf.len() < total {
            return Err(Error::BufferTooShort {
                need: total,
                have: buf.len(),
                what: "Primer Pack",
            });
        }
        buf[0..16].copy_from_slice(&Self::key());
        let value_len = 8 + self.entries.len() * 18;
        let len_size = encode_ber_length(value_len as u64, &mut buf[16..])?;
        let mut pos = 16 + len_size;
        buf[pos..pos + 4].copy_from_slice(&(self.entries.len() as u32).to_be_bytes());
        pos += 4;
        buf[pos..pos + 4].copy_from_slice(&LOCAL_TAG_ENTRY_LEN.to_be_bytes());
        pos += 4;
        for (tag, uid) in &self.entries {
            buf[pos..pos + 2].copy_from_slice(&tag.to_be_bytes());
            pos += 2;
            buf[pos..pos + 16].copy_from_slice(uid);
            pos += 16;
        }
        Ok(pos)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn primer_pack_round_trip() {
        let pack = PrimerPack {
            entries: alloc::vec![(0x3B02, [0xAAu8; 16]), (0x3B05, [0xBBu8; 16])],
        };
        let mut buf = alloc::vec![0u8; pack.serialized_len()];
        pack.serialize_into(&mut buf).unwrap();
        let parsed = PrimerPack::parse(&buf).unwrap();
        assert_eq!(parsed, pack);
        assert_eq!(parsed.resolve_tag(0x3B02), Some([0xAAu8; 16]));
        assert_eq!(parsed.resolve_ul(&[0xBBu8; 16]), Some(0x3B05));
        assert_eq!(parsed.resolve_tag(0x9999), None);
    }

    #[test]
    fn empty_primer_pack_round_trip() {
        let pack = PrimerPack::default();
        let mut buf = alloc::vec![0u8; pack.serialized_len()];
        pack.serialize_into(&mut buf).unwrap();
        assert_eq!(PrimerPack::parse(&buf).unwrap(), pack);
    }

    #[test]
    fn wrong_key_rejected() {
        let mut bytes = alloc::vec![0u8; 17];
        bytes[16] = 0; // zero-length value
        assert!(matches!(
            PrimerPack::parse(&bytes),
            Err(Error::KeyPrefixMismatch { .. })
        ));
    }
}
