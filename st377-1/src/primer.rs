//! Primer Pack — SMPTE ST 377-1:2019 §9.2, Tables 13-15 (`private/specs-verified/smpte/st377-1/st377-1.md`):
//! the per-Partition lookup table mapping every 2-byte local tag used in
//! this Partition's Header Metadata to its full UL/UUID.

extern crate alloc;

use alloc::vec::Vec;
use broadcast_common::len::{fit_u32, need_in, need_out};

use broadcast_common::{Parse, Serialize};

use crate::ber::{BerLength, ber_length_size_for, decode_ber_length, encode_ber_length_as};
use crate::error::{Error, Result};
use crate::types::{
    REGISTRY_VERSION_BYTE, RESERVED_KEY_BYTE, STRUCTURE_KIND_BYTE, U16_LEN, U32_LEN, UL_LEN, be,
};
use crate::types::{UlBytes, ul_bytes_from_prefix};

/// Fixed bytes 1-13 of the Primer Pack Key (Table 13), i.e. everything
/// except byte 8 (registry version, wildcard on parse).
const PRIMER_KEY_PREFIX: [u8; 7] = [0x06, 0x0E, 0x2B, 0x34, 0x02, 0x05, 0x01];
const PRIMER_KEY_MID: [u8; 4] = [0x0D, 0x01, 0x02, 0x01];
/// Byte 14 (Set/Pack Kind = Primer Pack) and byte 15 (Primer version).
const PRIMER_KEY_TAIL: [u8; U16_LEN] = [0x05, 0x01];

/// The size in bytes of one `LocalTagEntry` (Table 15): a 2-byte tag plus a
/// 16-byte AUID.
const LOCAL_TAG_ENTRY_LEN: u32 = 18;
// Byte offsets within the 16-byte Primer Pack Key (0-based).
const PRIMER_KEY_REGISTRY_VERSION_OFFSET: usize = 7;
const PRIMER_KEY_STRUCTURE_KIND_OFFSET: usize = 12;
const PRIMER_KEY_RESERVED_OFFSET: usize = 15;
/// Wire size of the Batch header (`count: u32`, `item_len: u32`).
const PRIMER_HEADER_LEN: usize = U32_LEN + U32_LEN;
/// Window size for an adjacent-pair duplicate scan.
const ADJACENT_WINDOW: usize = 2;

/// One Primer Pack entry (Table 15): a 2-byte local tag and the 16-byte
/// UL/UUID it maps to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LocalTagEntry {
    /// The 2-byte local tag used in this Partition's Header Metadata.
    pub tag: u16,
    /// The full UL/UUID the tag maps to.
    pub uid: UlBytes,
}

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
    pub entries: Vec<LocalTagEntry>,
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
        let mut key = [0u8; UL_LEN];
        key[0..7].copy_from_slice(&PRIMER_KEY_PREFIX);
        key[PRIMER_KEY_REGISTRY_VERSION_OFFSET] = REGISTRY_VERSION_BYTE;
        key[8..12].copy_from_slice(&PRIMER_KEY_MID);
        key[PRIMER_KEY_STRUCTURE_KIND_OFFSET] = STRUCTURE_KIND_BYTE;
        key[13..15].copy_from_slice(&PRIMER_KEY_TAIL);
        key[PRIMER_KEY_RESERVED_OFFSET] = RESERVED_KEY_BYTE;
        UlBytes(key)
    }

    /// True if `key` is the Primer Pack Key (Table 13), ignoring byte 8
    /// (registry version, wildcard).
    #[must_use]
    pub fn is_primer_key(key: &UlBytes) -> bool {
        let bytes = key.as_bytes();
        bytes[0..7] == PRIMER_KEY_PREFIX
            && bytes[8..12] == PRIMER_KEY_MID
            && bytes[PRIMER_KEY_STRUCTURE_KIND_OFFSET] == STRUCTURE_KIND_BYTE
            && bytes[13..15] == PRIMER_KEY_TAIL
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
    /// static tag the spec deliberately does not fix (`private/specs-verified/smpte/st377-1/st377-1.md`'s
    /// Annex A tables) — e.g. `Preface`'s `IsRIPPresent`.
    #[must_use]
    pub fn resolve_ul(&self, ul: &UlBytes) -> Option<u16> {
        self.entries.iter().find(|e| e.uid == *ul).map(|e| e.tag)
    }

    /// Look up the UL/UUID for a local tag, if present.
    #[must_use]
    pub fn resolve_tag(&self, tag: u16) -> Option<UlBytes> {
        self.entries.iter().find(|e| e.tag == tag).map(|e| e.uid)
    }

    /// Parse a Primer Pack (Key + Length + Value) from `bytes`, validating
    /// the Key and consuming exactly one KLV item's worth. Also usable in a
    /// stream context via the returned consumed-byte count.
    pub fn parse_prefix(bytes: &[u8]) -> Result<(Self, usize)> {
        need_in(bytes, UL_LEN, "Primer Pack key")?;
        let key: UlBytes = ul_bytes_from_prefix(bytes);
        Self::check_key(&key)?;

        let (len, len_token_size) = decode_ber_length(&bytes[16..])?;
        let value_start = UL_LEN + len_token_size;
        // #1108/MX-W1: `len` is a 64-bit BER length; a bare `as usize`
        // truncates it on a 32-bit target instead of rejecting an
        // over-range value.
        let len = usize::try_from(len).map_err(|_| {
            Error::BufferTooShort(broadcast_common::error::BufferTooShort {
                need: usize::MAX,
                have: bytes.len(),
                what: "Primer Pack value (length exceeds platform usize)",
            })
        })?;
        let value_end = value_start.checked_add(len).ok_or(Error::BufferTooShort(
            broadcast_common::error::BufferTooShort {
                need: usize::MAX,
                have: bytes.len(),
                what: "Primer Pack value (length overflow)",
            },
        ))?;
        need_in(bytes, value_end, "Primer Pack value")?;
        let v = &bytes[value_start..value_end];
        if v.len() < PRIMER_HEADER_LEN {
            return Err(Error::InvalidBatchHeader {
                count: 0,
                item_len: 0,
                buffer_len: v.len(),
            });
        }
        let count = u32::from_be_bytes(be::<U32_LEN>(v, 0)?);
        let item_len = u32::from_be_bytes(be::<U32_LEN>(v, U32_LEN)?);
        let body = &v[PRIMER_HEADER_LEN..];
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
        for chunk in body.chunks_exact(entry_len) {
            let tag = u16::from_be_bytes(be::<U16_LEN>(chunk, 0)?);
            let uid: UlBytes = ul_bytes_from_prefix(&chunk[U16_LEN..]);
            entries.push(LocalTagEntry { tag, uid });
        }
        // #1108/MX-W4: each local tag (and, by construction, the UL it
        // resolves to) must be unique within the Primer — `resolve_ul`/
        // `resolve_tag`'s linear scan silently returns the first match on a
        // duplicate otherwise. A nested scan is O(n^2) over an
        // attacker-chosen entry count (a ~1.2 MB Primer of 65 535 distinct
        // tags is ~2e9 comparisons), so each dimension is sorted instead.
        let mut by_tag: Vec<u16> = entries.iter().map(|e| e.tag).collect();
        by_tag.sort_unstable();
        if let Some(tag) = by_tag
            .windows(ADJACENT_WINDOW)
            .find(|w| w[0] == w[1])
            .map(|w| w[0])
        {
            return Err(Error::DuplicatePrimerTag(tag));
        }
        let mut by_ul: Vec<UlBytes> = entries.iter().map(|e| e.uid).collect();
        by_ul.sort_unstable();
        if let Some(w) = by_ul.windows(ADJACENT_WINDOW).find(|w| w[0] == w[1]) {
            let dup_ul = w[0];
            let (first, second) = entries
                .iter()
                .filter(|e| e.uid == dup_ul)
                .map(|e| e.tag)
                .fold((None, None), |(first, second), tag| match first {
                    None => (Some(tag), None),
                    Some(_) => (first, second.or(Some(tag))),
                });
            return Err(Error::DuplicatePrimerUl(
                first.unwrap_or_default(),
                second.unwrap_or_default(),
            ));
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
            return Err(Error::BufferTooShort(
                broadcast_common::error::BufferTooShort {
                    need: consumed,
                    have: bytes.len(),
                    what: "Primer Pack (trailing bytes after exact-fit parse)",
                },
            ));
        }
        Ok(pack)
    }
}

impl Serialize for PrimerPack {
    type Error = Error;

    fn serialized_len(&self) -> usize {
        let value_len = PRIMER_HEADER_LEN + self.entries.len() * LOCAL_TAG_ENTRY_LEN as usize;
        UL_LEN + ber_length_size_for(value_len as u64, self.len_size) + value_len
    }

    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        let total = self.serialized_len();
        need_out(buf, total)?;
        buf[0..16].copy_from_slice(Self::key().as_ref());
        let value_len = PRIMER_HEADER_LEN + self.entries.len() * LOCAL_TAG_ENTRY_LEN as usize;
        let len_size = encode_ber_length_as(value_len as u64, self.len_size, &mut buf[16..])?;
        let mut pos = UL_LEN + len_size;
        buf[pos..pos + U32_LEN].copy_from_slice(
            &fit_u32(self.entries.len(), "Primer Pack entry count")?.to_be_bytes(),
        );
        pos += U32_LEN;
        buf[pos..pos + U32_LEN].copy_from_slice(&LOCAL_TAG_ENTRY_LEN.to_be_bytes());
        pos += U32_LEN;
        for entry in &self.entries {
            buf[pos..pos + U16_LEN].copy_from_slice(&entry.tag.to_be_bytes());
            pos += U16_LEN;
            buf[pos..pos + UL_LEN].copy_from_slice(entry.uid.as_ref());
            pos += UL_LEN;
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
            entries: alloc::vec![
                LocalTagEntry {
                    tag: 0x3B02,
                    uid: UlBytes([0xAAu8; 16])
                },
                LocalTagEntry {
                    tag: 0x3B05,
                    uid: UlBytes([0xBBu8; 16])
                },
            ],
            ..Default::default()
        };
        let mut buf = alloc::vec![0u8; pack.serialized_len()];
        pack.serialize_into(&mut buf).unwrap();
        let parsed = PrimerPack::parse(&buf).unwrap();
        assert_eq!(parsed, pack);
        assert_eq!(parsed.resolve_tag(0x3B02), Some(UlBytes([0xAAu8; 16])));
        assert_eq!(parsed.resolve_ul(&UlBytes([0xBBu8; 16])), Some(0x3B05));
        assert_eq!(parsed.resolve_tag(0x9999), None);
    }

    #[test]
    fn empty_primer_pack_round_trip() {
        let pack = PrimerPack::default();
        let mut buf = alloc::vec![0u8; pack.serialized_len()];
        pack.serialize_into(&mut buf).unwrap();
        assert_eq!(PrimerPack::parse(&buf).unwrap(), pack);
    }

    /// §9.2 Table 15's header states both the entry count and the 18-byte
    /// entry size; each is rejected on its own.
    #[test]
    fn primer_rejects_a_bad_item_len_and_a_bad_count_separately() {
        let mut value = alloc::vec![0u8, 0, 0, 1, 0, 0, 0, 17];
        value.extend_from_slice(&[0x3B, 0x02]);
        value.extend_from_slice(&[0xAA; 16]);
        let mut bytes = alloc::vec![0u8; 16 + 1 + value.len()];
        bytes[..16].copy_from_slice(PrimerPack::key().as_ref());
        bytes[16] = value.len() as u8;
        bytes[17..].copy_from_slice(&value);
        assert!(
            matches!(
                PrimerPack::parse(&bytes),
                Err(Error::InvalidBatchHeader { item_len: 17, .. })
            ),
            "item_len 17 must be rejected"
        );

        // Only the count is wrong this time (2 entries declared, 1 present).
        value[3] = 2;
        value[7] = 16;
        bytes[17..].copy_from_slice(&value);
        assert!(
            matches!(
                PrimerPack::parse(&bytes),
                Err(Error::InvalidBatchHeader { count: 2, .. })
            ),
            "a count/body mismatch must be rejected"
        );
    }

    /// Issue #1047 (audit MX-C1): a non-minimal (fixed-width long-form) BER
    /// length token must round-trip byte-identically.
    #[test]
    fn non_minimal_length_token_round_trips_byte_identical() {
        let entries = alloc::vec![LocalTagEntry {
            tag: 0x3B02,
            uid: UlBytes([0xAAu8; 16])
        }];
        let value_len = 8 + entries.len() * 18; // 26, fits short form
        assert!(value_len <= 0x7F);

        let mut original = alloc::vec::Vec::new();
        original.extend_from_slice(PrimerPack::key().as_ref());
        original.push(0x81); // long form, 1 following byte
        original.push(value_len as u8);
        original.extend_from_slice(&(entries.len() as u32).to_be_bytes());
        original.extend_from_slice(&LOCAL_TAG_ENTRY_LEN.to_be_bytes());
        for entry in &entries {
            original.extend_from_slice(&entry.tag.to_be_bytes());
            original.extend_from_slice(entry.uid.as_ref());
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
            entries: alloc::vec![
                LocalTagEntry {
                    tag: 0x3B02,
                    uid: UlBytes([0xAAu8; 16])
                },
                LocalTagEntry {
                    tag: 0x3B02,
                    uid: UlBytes([0xBBu8; 16])
                },
            ],
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
            entries: alloc::vec![
                LocalTagEntry {
                    tag: 0x3B02,
                    uid: UlBytes([0xAAu8; 16])
                },
                LocalTagEntry {
                    tag: 0x3B05,
                    uid: UlBytes([0xAAu8; 16])
                },
            ],
            ..Default::default()
        };
        let mut buf = alloc::vec![0u8; pack.serialized_len()];
        pack.serialize_into(&mut buf).unwrap();
        assert!(matches!(
            PrimerPack::parse(&buf),
            Err(Error::DuplicatePrimerUl(0x3B02, 0x3B05))
        ));
    }

    /// The duplicate scan is sorted, not a nested O(n^2) loop (#2758): the
    /// largest legal Primer (every one of the 65 536 local tags present, a
    /// duplicate last) is rejected in linear-ish time where the nested scan
    /// needed ~2e9 comparisons.
    #[test]
    fn a_full_primer_with_a_trailing_duplicate_is_rejected() {
        let distinct = usize::from(u16::MAX) + 1;
        let mut entries: Vec<LocalTagEntry> = (0..distinct)
            .map(|tag| {
                let tag = u16::try_from(tag).unwrap();
                let mut uid = [0u8; 16];
                uid[0..2].copy_from_slice(&tag.to_be_bytes());
                LocalTagEntry {
                    tag,
                    uid: UlBytes(uid),
                }
            })
            .collect();
        let pack = PrimerPack {
            entries: entries.clone(),
            ..Default::default()
        };
        let mut buf = alloc::vec![0u8; pack.serialized_len()];
        pack.serialize_into(&mut buf).unwrap();
        assert_eq!(PrimerPack::parse(&buf).unwrap().entries.len(), distinct);

        entries.push(entries[distinct - 1]);
        let pack = PrimerPack {
            entries,
            ..Default::default()
        };
        let mut buf = alloc::vec![0u8; pack.serialized_len()];
        pack.serialize_into(&mut buf).unwrap();
        assert_eq!(
            PrimerPack::parse(&buf),
            Err(Error::DuplicatePrimerTag(u16::MAX))
        );
    }
}
