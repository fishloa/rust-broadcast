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

use crate::ber::{ber_length_size, decode_ber_length, encode_ber_length};
use crate::error::{Error, Result};
use crate::types::{UlBytes, ul_bytes_from_prefix};

/// A single KLV triplet: a 16-byte Key, a BER-encoded Length, and the Value
/// bytes it describes (`docs/st377-1.md` §6.3).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct KlvItem<'a> {
    /// The 16-byte Key.
    pub key: UlBytes,
    /// The Value bytes (borrowed from the input).
    pub value: &'a [u8],
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
        let (len, len_size) = decode_ber_length(&bytes[16..])?;
        let value_start = 16 + len_size;
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
        16 + ber_length_size(self.value.len() as u64) + self.value.len()
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
        let len_size = encode_ber_length(self.value.len() as u64, &mut buf[16..])?;
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
        let item = KlvItem {
            key: [0xAAu8; 16],
            value: &[1, 2, 3, 4],
        };
        let mut buf = alloc::vec![0u8; item.serialized_len()];
        item.serialize_into(&mut buf).unwrap();
        assert_eq!(buf.len(), 16 + 1 + 4);
        assert_eq!(KlvItem::parse(&buf).unwrap(), item);
    }

    #[test]
    fn klv_item_round_trip_long_form() {
        let value = alloc::vec![0x42u8; 200];
        let item = KlvItem {
            key: [0xBBu8; 16],
            value: &value,
        };
        let mut buf = alloc::vec![0u8; item.serialized_len()];
        item.serialize_into(&mut buf).unwrap();
        assert_eq!(buf.len(), 16 + 2 + 200);
        assert_eq!(KlvItem::parse(&buf).unwrap(), item);
    }

    #[test]
    fn walk_multiple_items() {
        let a = KlvItem {
            key: [1u8; 16],
            value: &[10, 20],
        };
        let b = KlvItem {
            key: [2u8; 16],
            value: &[30, 40, 50],
        };
        let mut buf = Vec::new();
        buf.extend(a.to_bytes());
        buf.extend(b.to_bytes());

        let items = collect_klv_items(&buf).unwrap();
        assert_eq!(items.len(), 2);
        assert_eq!(items[0], (0, a));
        assert_eq!(items[1].1, b);
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
}
