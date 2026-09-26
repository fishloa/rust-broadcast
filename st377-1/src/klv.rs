//! Generic KLV (Key-Length-Value) triplet — SMPTE ST 377-1:2019 §6.3
//! (`docs/st377-1.md`), the base framing primitive every other KLV item in
//! an MXF file (Partition Packs, the Primer Pack, Header Metadata Sets,
//! Index Table Segments, the Random Index Pack, and every Essence Container
//! element) rides on.
//!
//! Zero-copy: [`KlvItem`] borrows its `value` from the input buffer, so
//! walking a (potentially huge, essence-carrying) MXF file never copies
//! sample bytes.

extern crate alloc;

use alloc::vec::Vec;

use broadcast_common::{Parse, Serialize};

use crate::ber::{BerLength, ber_length_size_for, decode_ber_length, encode_ber_length_as};
use crate::error::{Error, Result};
use crate::types::{UlBytes, ul_bytes_from_prefix};

/// A single KLV triplet: a 16-byte Key, a BER-encoded Length, and the Value
/// bytes it describes (`docs/st377-1.md` §6.3).
///
/// `PartialEq`/`Eq` compare only `key`/`value` — `len_size` is a
/// serialization-*form* preference (which on-wire BER width to reproduce),
/// not part of the item's logical value, so two items differing only in
/// `len_size` (e.g. a freshly-built `Minimal` one vs. the same item
/// reparsed, which always carries the concrete `Fixed` width it found)
/// still compare equal — the project-wide "parse -> serialize -> parse
/// gives an equal value" round-trip check stays meaningful. Byte-identity
/// (which DOES depend on `len_size`) is asserted separately, directly on
/// the serialized bytes (issue #1047 / audit MX-C1).
#[derive(Debug, Clone, Copy, Default)]
pub struct KlvItem<'a> {
    /// The 16-byte Key.
    pub key: UlBytes,
    /// The Value bytes (borrowed from the input).
    pub value: &'a [u8],
    /// The on-wire BER length-field width (issue #1047 / audit MX-C1) —
    /// [`BerLength::Minimal`] (the default) for a freshly built value, or
    /// the exact width [`Self::parse_prefix`] found on the wire, so
    /// `serialize_into` reproduces the original length token exactly
    /// rather than always re-canonicalizing it.
    pub len_size: BerLength,
}

impl PartialEq for KlvItem<'_> {
    fn eq(&self, other: &Self) -> bool {
        self.key == other.key && self.value == other.value
    }
}

impl Eq for KlvItem<'_> {}

impl<'a> KlvItem<'a> {
    /// Build a freshly-constructed item that serializes with the
    /// canonical minimal BER length form (`len_size: BerLength::Minimal`).
    #[must_use]
    pub fn new(key: UlBytes, value: &'a [u8]) -> Self {
        Self {
            key,
            value,
            len_size: BerLength::Minimal,
        }
    }
}

/// The KLV Fill item key (§6.3.3), matching byte 8 (the version number) as
/// a wildcard per the spec's own note ("MXF decoders shall ignore the
/// version number byte ... when determining if a KLV key is the Fill item
/// key" — some early encoders wrote `0x01` there instead of the RP 210
/// value `0x02`).
pub const FILL_ITEM_KEY_PREFIX: [u8; 7] = [0x06, 0x0E, 0x2B, 0x34, 0x01, 0x01, 0x01];
/// Byte 8 (version) is a wildcard; bytes 9-16 of the Fill item key.
pub const FILL_ITEM_KEY_SUFFIX: [u8; 8] = [0x03, 0x01, 0x02, 0x10, 0x01, 0x00, 0x00, 0x00];

/// True if `key` is the KLV Fill item key (§6.3.3), ignoring byte 8 (the
/// version number) per the spec's own decoder rule.
#[must_use]
pub fn is_fill_item_key(key: &UlBytes) -> bool {
    key[..7] == FILL_ITEM_KEY_PREFIX && key[8..] == FILL_ITEM_KEY_SUFFIX
}

impl<'a> KlvItem<'a> {
    /// Parse one KLV triplet from the start of `bytes`, returning it along
    /// with the total number of bytes consumed (key + length + value) — use
    /// this to walk a sequence of KLV items in a stream.
    pub fn parse_prefix(bytes: &'a [u8]) -> Result<(Self, usize)> {
        if bytes.len() < 16 {
            return Err(Error::BufferTooShort {
                need: 16,
                have: bytes.len(),
                what: "KLV key",
            });
        }
        let key: UlBytes = ul_bytes_from_prefix(bytes);
        let (len, len_token_size) = decode_ber_length(&bytes[16..])?;
        let value_start = 16 + len_token_size;
        let len = usize::try_from(len).map_err(|_| Error::BufferTooShort {
            need: usize::MAX,
            have: bytes.len(),
            what: "KLV value (length exceeds platform usize)",
        })?;
        let value_end = value_start.checked_add(len).ok_or(Error::BufferTooShort {
            need: usize::MAX,
            have: bytes.len(),
            what: "KLV value (length overflow)",
        })?;
        if bytes.len() < value_end {
            return Err(Error::BufferTooShort {
                need: value_end,
                have: bytes.len(),
                what: "KLV value",
            });
        }
        Ok((
            KlvItem {
                key,
                value: &bytes[value_start..value_end],
                len_size: BerLength::fixed_from_consumed(len_token_size),
            },
            value_end,
        ))
    }
}

impl<'a> Parse<'a> for KlvItem<'a> {
    type Error = Error;

    fn parse(bytes: &'a [u8]) -> Result<Self> {
        let (item, consumed) = Self::parse_prefix(bytes)?;
        if consumed != bytes.len() {
            return Err(Error::BufferTooShort {
                need: consumed,
                have: bytes.len(),
                what: "KLV item (trailing bytes after exact-fit parse)",
            });
        }
        Ok(item)
    }
}

impl Serialize for KlvItem<'_> {
    type Error = Error;

    fn serialized_len(&self) -> usize {
        16 + ber_length_size_for(self.value.len() as u64, self.len_size) + self.value.len()
    }

    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        let total = self.serialized_len();
        if buf.len() < total {
            return Err(Error::BufferTooShort {
                need: total,
                have: buf.len(),
                what: "KLV item",
            });
        }
        buf[..16].copy_from_slice(&self.key);
        let len_size =
            encode_ber_length_as(self.value.len() as u64, self.len_size, &mut buf[16..])?;
        let value_start = 16 + len_size;
        buf[value_start..value_start + self.value.len()].copy_from_slice(self.value);
        Ok(total)
    }
}

/// Walk every KLV item in `bytes` (e.g. one Partition's full body), calling
/// `f` with each item and its byte offset from the start of `bytes`. Stops
/// at the first parse error (returned to the caller) or when the buffer is
/// exhausted.
pub fn walk_klv_items<'a>(
    mut bytes: &'a [u8],
    mut f: impl FnMut(usize, KlvItem<'a>) -> Result<()>,
) -> Result<()> {
    let mut offset = 0usize;
    while !bytes.is_empty() {
        let (item, consumed) = KlvItem::parse_prefix(bytes)?;
        f(offset, item)?;
        offset += consumed;
        bytes = &bytes[consumed..];
    }
    Ok(())
}

/// Collect every KLV item in `bytes` into a `Vec` (small helper for tests
/// and examples; large real files should prefer [`walk_klv_items`] to avoid
/// buffering every item at once).
pub fn collect_klv_items(bytes: &[u8]) -> Result<Vec<(usize, KlvItem<'_>)>> {
    let mut out = Vec::new();
    walk_klv_items(bytes, |offset, item| {
        out.push((offset, item));
        Ok(())
    })?;
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn klv_item_round_trip_short_form() {
        // Built with `len_size: BerLength::Minimal` (via `new`); a
        // reparsed item always carries `BerLength::Fixed(..)` (the width
        // it actually found on the wire) instead, even when that width is
        // numerically the same as what Minimal would have chosen — so the
        // round-trip check here compares key/value, not the whole struct
        // (see `ber_module::preserves_non_minimal_length_round_trip` for
        // the width-preservation check itself).
        let item = KlvItem::new([0xAAu8; 16], &[1, 2, 3, 4]);
        let mut buf = alloc::vec![0u8; item.serialized_len()];
        item.serialize_into(&mut buf).unwrap();
        assert_eq!(buf.len(), 16 + 1 + 4);
        let reparsed = KlvItem::parse(&buf).unwrap();
        assert_eq!(reparsed.key, item.key);
        assert_eq!(reparsed.value, item.value);
    }

    #[test]
    fn klv_item_round_trip_long_form() {
        let value = alloc::vec![0x42u8; 200];
        let item = KlvItem::new([0xBBu8; 16], &value);
        let mut buf = alloc::vec![0u8; item.serialized_len()];
        item.serialize_into(&mut buf).unwrap();
        assert_eq!(buf.len(), 16 + 2 + 200);
        let reparsed = KlvItem::parse(&buf).unwrap();
        assert_eq!(reparsed.key, item.key);
        assert_eq!(reparsed.value, item.value);
    }

    #[test]
    fn walk_multiple_items() {
        let a = KlvItem::new([1u8; 16], &[10, 20]);
        let b = KlvItem::new([2u8; 16], &[30, 40, 50]);
        let mut buf = Vec::new();
        buf.extend(a.to_bytes());
        buf.extend(b.to_bytes());

        let items = collect_klv_items(&buf).unwrap();
        assert_eq!(items.len(), 2);
        assert_eq!(items[0].0, 0);
        assert_eq!(items[0].1.key, a.key);
        assert_eq!(items[0].1.value, a.value);
        assert_eq!(items[1].1.key, b.key);
        assert_eq!(items[1].1.value, b.value);
    }

    #[test]
    fn fill_item_key_matches_ignoring_version_byte() {
        let rp210 = [
            0x06, 0x0E, 0x2B, 0x34, 0x01, 0x01, 0x01, 0x02, 0x03, 0x01, 0x02, 0x10, 0x01, 0x00,
            0x00, 0x00,
        ];
        let legacy = [
            0x06, 0x0E, 0x2B, 0x34, 0x01, 0x01, 0x01, 0x01, 0x03, 0x01, 0x02, 0x10, 0x01, 0x00,
            0x00, 0x00,
        ];
        assert!(is_fill_item_key(&rp210));
        assert!(is_fill_item_key(&legacy));
        assert!(!is_fill_item_key(&[0u8; 16]));
    }

    #[test]
    fn truncated_key_is_error() {
        assert!(matches!(
            KlvItem::parse(&[0u8; 10]),
            Err(Error::BufferTooShort { .. })
        ));
    }

    /// Issue #1047 (audit MX-C1): a non-minimal (fixed-width long-form)
    /// on-wire length token must round-trip byte-identically, not get
    /// silently re-canonicalized to the shortest form that fits.
    #[test]
    fn non_minimal_length_token_round_trips_byte_identical() {
        // Hand-built KLV: key + `0x82 00 04` (long form, 2 following
        // bytes, value 4) — canonical minimal form for value 4 is the
        // 1-byte short form `0x04`, so this is deliberately non-minimal.
        let mut original = alloc::vec![0xAAu8; 16];
        original.extend_from_slice(&[0x82, 0x00, 0x04]);
        original.extend_from_slice(&[1, 2, 3, 4]);

        let item = KlvItem::parse(&original).unwrap();
        assert_eq!(
            item.len_size,
            BerLength::Fixed(core::num::NonZeroU8::new(3).unwrap()),
            "parse must record the true 3-byte on-wire length-token width"
        );

        let mut out = alloc::vec![0u8; item.serialized_len()];
        item.serialize_into(&mut out).unwrap();
        assert_eq!(
            out, original,
            "re-serializing a parsed item must reproduce its exact original bytes, \
             including a non-minimal length token"
        );
    }

    /// A freshly-built item (`len_size: BerLength::Minimal`, the `new()`
    /// default) still serializes with the canonical minimal form.
    #[test]
    fn freshly_built_item_uses_minimal_form() {
        let item = KlvItem::new([0xAAu8; 16], &[1, 2, 3, 4]);
        assert_eq!(item.len_size, BerLength::Minimal);
        let bytes = item.to_bytes();
        assert_eq!(&bytes[16..17], &[0x04], "value 4 must use short form");
    }
}
