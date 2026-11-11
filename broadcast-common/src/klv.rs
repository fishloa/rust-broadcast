//! Generic KLV (Key-Length-Value) triplet and its BER length — SMPTE ST 336
//! "Data Encoding Protocol Using Key-Length-Value", as constrained for MXF by
//! SMPTE ST 377-1:2019 §6.3 (`private/specs-verified/smpte/st377-1/st377-1.md`)
//! and for MISB metadata by MISB ST 0601.
//!
//! The base framing primitive every KLV item in an MXF file (Partition Packs,
//! the Primer Pack, Header Metadata Sets, Index Table Segments, the Random
//! Index Pack, and every Essence Container element) and every MISB Local Set
//! packet rides on: a 16-byte [`UniversalLabel`] key, a [`BerLength`]-encoded
//! length (ISO/IEC 8825-1 §8.1.3), and the value bytes it describes.
//!
//! Zero-copy: [`KlvItem`] borrows its `value` from the input buffer, so walking
//! a (potentially huge, essence-carrying) file never copies sample bytes.

use core::fmt;

use alloc::vec::Vec;

pub use crate::ber::{
    ber_length_size, ber_length_size_for, decode_ber_length, encode_ber_length,
    encode_ber_length_as,
};
use crate::error::{BufferTooShort, InvalidValue, OutputBufferTooSmall};
use crate::len::{need_in, need_out};
use crate::{Parse, Serialize};

/// Length of a SMPTE Universal Label key, in bytes (SMPTE ST 336).
pub const UNIVERSAL_LABEL_LEN: usize = 16;

/// A 16-byte SMPTE Universal Label (KLV key).
pub type UniversalLabel = [u8; UNIVERSAL_LABEL_LEN];

/// Copy the first [`UNIVERSAL_LABEL_LEN`] bytes of `bytes` into a key.
fn universal_label_from_prefix(bytes: &[u8]) -> UniversalLabel {
    let mut out = [0u8; UNIVERSAL_LABEL_LEN];
    let mut i = 0;
    while i < UNIVERSAL_LABEL_LEN {
        out[i] = bytes[i];
        i += 1;
    }
    out
}

/// An error from the KLV triplet / BER length codec (SMPTE ST 336,
/// ISO/IEC 8825-1 §8.1.3).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum KlvError {
    /// The input buffer was shorter than the triplet being parsed.
    BufferTooShort(BufferTooShort),
    /// The caller's output buffer was smaller than the serialized triplet.
    OutputBufferTooSmall(OutputBufferTooSmall),
    /// A BER length token used the reserved "unspecified length" form (a
    /// long-form first byte claiming zero following bytes).
    BerIndefiniteLength,
    /// A BER long-form length used more following bytes than an unsigned
    /// 64-bit length could need, or than the 9-byte token cap allows.
    BerLengthTooLong {
        /// Following-byte count found (or requested on serialize).
        bytes: usize,
    },
    /// [`BerLength::Fixed`] requested a specific on-wire token width too small
    /// to represent the length being serialized.
    FixedBerLengthTooSmall {
        /// The length that needed encoding.
        len: u64,
        /// The requested fixed total token width, in bytes.
        width: u8,
    },
    /// A BER length field was structurally invalid (a value the spec does not
    /// allow, carrying the reason).
    InvalidLength(InvalidValue),
}

impl fmt::Display for KlvError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::BufferTooShort(e) => write!(f, "{e}"),
            Self::OutputBufferTooSmall(e) => write!(f, "{e}"),
            Self::BerIndefiniteLength => {
                f.write_str("BER length used the reserved indefinite-length token (0x80)")
            }
            Self::BerLengthTooLong { bytes } => write!(
                f,
                "BER long-form length has {bytes} following bytes, exceeding the 8-byte cap"
            ),
            Self::FixedBerLengthTooSmall { len, width } => write!(
                f,
                "length {len} does not fit in the fixed BER width of {width} byte(s)"
            ),
            Self::InvalidLength(e) => write!(f, "{e}"),
        }
    }
}

impl core::error::Error for KlvError {}

impl From<BufferTooShort> for KlvError {
    fn from(e: BufferTooShort) -> Self {
        Self::BufferTooShort(e)
    }
}

impl From<OutputBufferTooSmall> for KlvError {
    fn from(e: OutputBufferTooSmall) -> Self {
        Self::OutputBufferTooSmall(e)
    }
}

/// Result alias for the KLV codec.
pub type Result<T> = core::result::Result<T, KlvError>;

/// The BER length-field width to serialize a value's length with (issue
/// #1047 / audit MX-C1). `private/specs-verified/smpte/st377-1/st377-1.md` §6.3.4 permits any valid BER
/// form for a length token; real MXF encoders routinely write a longer,
/// fixed-width long form (rather than the shortest one that fits) so a
/// pack can be rewritten in place later without shifting every later
/// absolute offset. Every type that owns a top-level Key-Length-Value
/// envelope (`KlvItem`, `PartitionPack`, `PrimerPack`, `RandomIndexPack`,
/// `LocalSet`) stores one of these, set to `Fixed` from the on-wire width
/// on parse so `serialize_into` reproduces it exactly, and defaulting to
/// `Minimal` for a freshly built value.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[non_exhaustive]
pub enum BerLength {
    /// Always (re)compute the canonical minimal-width encoding for
    /// whatever length is being serialized.
    #[default]
    Minimal,
    /// Reproduce this specific on-wire total token width in bytes: `1` is
    /// short form; `2..=9` is long form with `width - 1` following length
    /// bytes.
    Fixed(core::num::NonZeroU8),
}

impl BerLength {
    /// Build a [`Self::Fixed`] from a length token's total consumed width
    /// (as [`decode_ber_length`] returns it) — always in range since a
    /// successful decode consumes at least 1 and at most 9 bytes.
    #[must_use]
    pub fn fixed_from_consumed(consumed: usize) -> Self {
        Self::Fixed(
            u8::try_from(consumed)
                .ok()
                .and_then(core::num::NonZeroU8::new)
                .unwrap_or(core::num::NonZeroU8::MIN),
        )
    }
}

/// A single KLV triplet: a 16-byte Key, a BER-encoded Length, and the Value
/// bytes it describes (`private/specs-verified/smpte/st377-1/st377-1.md` §6.3).
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
    pub key: UniversalLabel,
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
    pub fn new(key: UniversalLabel, value: &'a [u8]) -> Self {
        Self {
            key,
            value,
            len_size: BerLength::Minimal,
        }
    }
}

impl<'a> KlvItem<'a> {
    /// Parse one KLV triplet from the start of `bytes`, returning it along
    /// with the total number of bytes consumed (key + length + value) — use
    /// this to walk a sequence of KLV items in a stream.
    pub fn parse_prefix(bytes: &'a [u8]) -> Result<(Self, usize)> {
        need_in(bytes, UNIVERSAL_LABEL_LEN, "KLV key")?;
        let key: UniversalLabel = universal_label_from_prefix(bytes);
        let (len, len_token_size) = decode_ber_length(&bytes[UNIVERSAL_LABEL_LEN..])?;
        let value_start = UNIVERSAL_LABEL_LEN + len_token_size;
        let len = usize::try_from(len).map_err(|_| {
            KlvError::BufferTooShort(BufferTooShort {
                need: usize::MAX,
                have: bytes.len(),
                what: "KLV value (length exceeds platform usize)",
            })
        })?;
        let value_end = value_start
            .checked_add(len)
            .ok_or(KlvError::BufferTooShort(BufferTooShort {
                need: usize::MAX,
                have: bytes.len(),
                what: "KLV value (length overflow)",
            }))?;
        need_in(bytes, value_end, "KLV value")?;
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
    type Error = KlvError;

    fn parse(bytes: &'a [u8]) -> Result<Self> {
        let (item, consumed) = Self::parse_prefix(bytes)?;
        if consumed != bytes.len() {
            return Err(KlvError::BufferTooShort(BufferTooShort {
                need: consumed,
                have: bytes.len(),
                what: "KLV item (trailing bytes after exact-fit parse)",
            }));
        }
        Ok(item)
    }
}

impl Serialize for KlvItem<'_> {
    type Error = KlvError;

    fn serialized_len(&self) -> usize {
        UNIVERSAL_LABEL_LEN
            + ber_length_size_for(
                u64::try_from(self.value.len()).unwrap_or(u64::MAX),
                self.len_size,
            )
            + self.value.len()
    }

    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        let total = self.serialized_len();
        need_out(buf, total)?;
        buf[..UNIVERSAL_LABEL_LEN].copy_from_slice(&self.key);
        let len_size = encode_ber_length_as(
            u64::try_from(self.value.len()).unwrap_or(u64::MAX),
            self.len_size,
            &mut buf[UNIVERSAL_LABEL_LEN..],
        )?;
        let value_start = UNIVERSAL_LABEL_LEN + len_size;
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
    fn serialize_into_short_output_is_output_buffer_too_small() {
        let item = KlvItem::new([0xAAu8; 16], &[1, 2, 3, 4]);
        let need = item.serialized_len();
        let mut buf = alloc::vec![0u8; need - 1];
        assert_eq!(
            item.serialize_into(&mut buf),
            Err(KlvError::OutputBufferTooSmall(OutputBufferTooSmall {
                need,
                have: need - 1
            }))
        );
    }

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
        buf.extend(a.try_to_bytes().unwrap());
        buf.extend(b.try_to_bytes().unwrap());

        let items = collect_klv_items(&buf).unwrap();
        assert_eq!(items.len(), 2);
        assert_eq!(items[0].0, 0);
        assert_eq!(items[0].1.key, a.key);
        assert_eq!(items[0].1.value, a.value);
        assert_eq!(items[1].1.key, b.key);
        assert_eq!(items[1].1.value, b.value);
    }

    #[test]
    fn truncated_key_is_error() {
        assert!(matches!(
            KlvItem::parse(&[0u8; 10]),
            Err(KlvError::BufferTooShort(..))
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
        let bytes = item.try_to_bytes().unwrap();
        assert_eq!(&bytes[16..17], &[0x04], "value 4 must use short form");
    }
}
