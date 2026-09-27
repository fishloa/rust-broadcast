//! Primer Pack — SMPTE ST 377-1:2019 §9.2, Tables 13-15 (`docs/st377-1.md`):
//! the per-Partition lookup table mapping every 2-byte local tag used in
//! this Partition's Header Metadata to its full UL/UUID.

extern crate alloc;

use alloc::vec::Vec;

use broadcast_common::{Parse, Serialize};

use crate::ber::{BerLength, ber_length_size_for, decode_ber_length, encode_ber_length_as};
use crate::error::{Error, Result};
use crate::types::{UlBytes, ul_bytes_from_prefix};

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
///
/// `PartialEq`/`Eq` compare only `entries` — `len_size` is a
/// serialization-*form* preference, not part of the Pack's logical value
/// (see [`crate::KlvItem`]'s doc; issue #1047 / audit MX-C1).
#[derive(Debug, Clone, Default)]
pub struct PrimerPack {
    /// Every local-tag -> UL/UUID mapping in this Partition's Header
    /// Metadata.
    pub entries: Vec<(u16, UlBytes)>,
    /// The on-wire BER length-field width — [`BerLength::Minimal`] for a
    /// freshly built value, or the exact width `parse` found on the wire.
    pub len_size: BerLength,
}

impl PartialEq for PrimerPack {
    fn eq(&self, other: &Self) -> bool {
        self.entries == other.entries
    }
}

impl Eq for PrimerPack {}

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
        let key: UlBytes = ul_bytes_from_prefix(bytes);
        Self::check_key(&key)?;

        let (len, len_token_size) = decode_ber_length(&bytes[16..])?;
        let value_start = 16 + len_token_size;
        // #1108/MX-W1: `len` is a 64-bit BER length; a bare `as usize`
        // truncates it on a 32-bit target instead of rejecting an
        // over-range value.
        let len = usize::try_from(len).map_err(|_| Error::BufferTooShort {
            need: usize::MAX,
            have: bytes.len(),
            what: "Primer Pack value (length exceeds platform usize)",
        })?;
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
        // #1108/MX-W1: `count as usize * 18` wraps on a 32-bit target for a
        // large enough `count`, which could make an attacker-chosen count
        // spuriously equal `body.len()` and then abort the process at
        // `Vec::with_capacity(count as usize)`. `checked_mul` rejects the
        // overflow instead, and the allocation below is sized from the
        // already-bounded `body.len()`, never from the untrusted `count`.
        let entry_len = LOCAL_TAG_ENTRY_LEN as usize;
        let expected_len = (count as usize).checked_mul(entry_len);
        if item_len != LOCAL_TAG_ENTRY_LEN || expected_len != Some(body.len()) {
            return Err(Error::InvalidBatchHeader {
                count,
                item_len,
                buffer_len: body.len(),
            });
        }
        let mut entries = Vec::with_capacity(body.len() / entry_len);
        for chunk in body.chunks_exact(18) {
            let tag = u16::from_be_bytes([chunk[0], chunk[1]]);
            let uid: UlBytes = ul_bytes_from_prefix(&chunk[2..]);
            entries.push((tag, uid));
        }
        // #1108/MX-W4: each local tag (and, by construction, the UL it
        // resolves to) must be unique within the Primer — `resolve_ul`/
        // `resolve_tag`'s linear scan silently returns the first match on a
        // duplicate otherwise. O(n^2) is fine: a Primer's entry count is
        // bounded by the local tags actually used in this Partition's
        // Header Metadata (KB-scale, per the crate's own doc comments).
        for i in 0..entries.len() {
            for j in (i + 1)..entries.len() {
                if entries[i].0 == entries[j].0 {
                    return Err(Error::DuplicatePrimerTag(entries[i].0));
                }
                if entries[i].1 == entries[j].1 {
                    return Err(Error::DuplicatePrimerUl(entries[i].0, entries[j].0));
                }
            }
        }
        Ok((
            PrimerPack {
                entries,
                len_size: BerLength::fixed_from_consumed(len_token_size),
            },
            value_end,
        ))
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
        16 + ber_length_size_for(value_len as u64, self.len_size) + value_len
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
        let len_size = encode_ber_length_as(value_len as u64, self.len_size, &mut buf[16..])?;
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
            ..Default::default()
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

    /// Issue #1047 (audit MX-C1): a non-minimal (fixed-width long-form) BER
    /// length token must round-trip byte-identically.
    #[test]
    fn non_minimal_length_token_round_trips_byte_identical() {
        let entries = alloc::vec![(0x3B02u16, [0xAAu8; 16])];
        let value_len = 8 + entries.len() * 18; // 26, fits short form
        assert!(value_len <= 0x7F);

        let mut original = alloc::vec::Vec::new();
        original.extend_from_slice(&PrimerPack::key());
        original.push(0x81); // long form, 1 following byte
        original.push(value_len as u8);
        original.extend_from_slice(&(entries.len() as u32).to_be_bytes());
        original.extend_from_slice(&LOCAL_TAG_ENTRY_LEN.to_be_bytes());
        for (tag, uid) in &entries {
            original.extend_from_slice(&tag.to_be_bytes());
            original.extend_from_slice(uid);
        }

        let parsed = PrimerPack::parse(&original).unwrap();
        assert_eq!(
            parsed.len_size,
            BerLength::Fixed(core::num::NonZeroU8::new(2).unwrap())
        );

        let mut out = alloc::vec![0u8; parsed.serialized_len()];
        parsed.serialize_into(&mut out).unwrap();
        assert_eq!(out, original);
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

    /// MX-W4 (#1108): each local tag must be unique within a Primer;
    /// `resolve_tag`'s linear scan silently returned the first match on a
    /// duplicate, and parse never rejected one.
    #[test]
    fn rejects_duplicate_primer_tag() {
        let pack = PrimerPack {
            entries: alloc::vec![(0x3B02, [0xAAu8; 16]), (0x3B02, [0xBBu8; 16])],
            ..Default::default()
        };
        let mut buf = alloc::vec![0u8; pack.serialized_len()];
        pack.serialize_into(&mut buf).unwrap();
        assert!(matches!(
            PrimerPack::parse(&buf),
            Err(Error::DuplicatePrimerTag(0x3B02))
        ));
    }

    /// MX-W4 (#1108): two different local tags must not resolve to the same
    /// UL/UUID.
    #[test]
    fn rejects_duplicate_primer_ul() {
        let pack = PrimerPack {
            entries: alloc::vec![(0x3B02, [0xAAu8; 16]), (0x3B05, [0xAAu8; 16])],
            ..Default::default()
        };
        let mut buf = alloc::vec![0u8; pack.serialized_len()];
        pack.serialize_into(&mut buf).unwrap();
        assert!(matches!(
            PrimerPack::parse(&buf),
            Err(Error::DuplicatePrimerUl(0x3B02, 0x3B05))
        ));
    }
}
