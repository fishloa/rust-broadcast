//! KLV (Key-Length-Value) timed metadata — SMPTE ST 336 framing + MISB ST 0601
//! UAS Datalink Local Set.
//!
//! KLV encodes each metadata item as a *Key*, a BER-encoded *Length*, and a
//! *Value*. Keys are either 16-byte SMPTE Universal Labels (the outer wrapper of
//! a Local Set / Global Set) or 1-byte BER-OID tags (the items inside a Local
//! Set). This module implements:
//!
//! - **BER length** encode/decode ([`decode_ber_length_usize`] /
//!   [`encode_ber_length_bytes`]):
//!   short form (`< 128`) and long form (top bit set, low 7 bits = count of
//!   big-endian length bytes) — ISO/IEC 8825-1 §8.1.3, via SMPTE ST 336. The
//!   codec itself is `broadcast_common::klv`'s.
//! - **Generic KLV item** ([`KlvItem`]): a 16-byte Universal Label key + owned
//!   value bytes, serialized through the borrowed shared
//!   `broadcast_common::klv::KlvItem` with a *computed* (never echoed) minimal
//!   BER length.
//! - **BER-OID tags** ([`ber_oid`] / [`encode_ber_oid`]): the 1-byte (multi-byte
//!   continuation on the top bit) integer keys used inside a Local Set.
//! - **UAS Datalink Local Set** ([`UasLocalSet`]): the MISB ST 0601 packet — the
//!   [`UAS_LS_KEY`] Universal Label wrapping a sequence of `tag + BER-length +
//!   value` items, with the [`TAG_PRECISION_TIMESTAMP`] (u64 BE µs since the
//!   POSIX epoch) and the [`TAG_CHECKSUM`] (a running 16-bit sum over the whole
//!   packet, [`checksum_bcc16`]).
//!
//! # Spec citations
//!
//! - **SMPTE ST 336** "Data Encoding Protocol Using Key-Length-Value" — KLV
//!   framing (16-byte UL key, BER length, BER-OID tags, Local Set). The standard
//!   itself is paywalled; the byte-level framing used here is fully covered by
//!   MISB ST 0601 + RFC 6597. See `private/specs-verified/mpeg/transmux/klv/klv-misb0601.md`.
//! - **MISB ST 0601** UAS Datalink Local Set — the [`UAS_LS_KEY`] Universal
//!   Label, tag ordering (tag 2 first, tag 1 last), tag 2 Precision Time Stamp
//!   and tag 1 Checksum (ST 0601 §5.5/§7.1 — "the lower 16-bits of summation",
//!   a running big-endian 16-bit sum, *not* a CRC; the standard's change
//!   history records removing the earlier "CRC-16" wording).
//! - **RFC 6597** — KLV-over-RTP payload format (see [`crate::rtp`]).
//!
//! `no_std` + `alloc`.

use alloc::vec::Vec;
use broadcast_common::error::{BufferTooShort, InvalidValue};
use broadcast_common::klv::{
    KlvItem as Item, UNIVERSAL_LABEL_LEN, UniversalLabel, ber_length_size,
    encode_ber_length as encode_ber_length_into,
};
use broadcast_common::len::{need_in, need_out};

use broadcast_common::{Parse, Serialize};

use crate::error::{Error, Result};

// ---------------------------------------------------------------------------
// Named constants — SMPTE ST 336 / MISB ST 0601 (no magic numbers)
// ---------------------------------------------------------------------------

/// The MISB ST 0601 UAS Datalink Local Set Universal Label
/// (`06 0E 2B 34 02 0B 01 01 0E 01 03 01 01 00 00 00`).
pub const UAS_LS_KEY: UniversalLabel = [
    0x06, 0x0E, 0x2B, 0x34, 0x02, 0x0B, 0x01, 0x01, 0x0E, 0x01, 0x03, 0x01, 0x01, 0x00, 0x00, 0x00,
];

/// Mask for the low 7 bits of a BER length/OID byte (the payload of the octet).
const BER_LOW7_MASK: u8 = 0x7F;
/// BER-OID continuation bit: bit 7 of a multi-byte OID tag byte (more bytes
/// follow). All current MISB ST 0601 tags fit in one byte (`<= 127`).
const BER_OID_CONTINUATION: u8 = 0x80;

/// MISB ST 0601 tag 1 — Checksum, MUST be the last LS item.
pub const TAG_CHECKSUM: u32 = 1;
/// The Checksum value length in bytes (a 16-bit value).
pub const CHECKSUM_LEN: usize = 2;
/// MISB ST 0601 tag 2 — Precision Time Stamp (u64 BE µs since the POSIX epoch),
/// MUST be the first LS item.
pub const TAG_PRECISION_TIMESTAMP: u32 = 2;
/// The Precision Time Stamp value length in bytes (a `u64`).
pub const PRECISION_TIMESTAMP_LEN: usize = 8;

// ---------------------------------------------------------------------------
// BER length (ISO/IEC 8825-1 §8.1.3, via SMPTE ST 336)
// ---------------------------------------------------------------------------

/// Encode `len` as the canonical minimal-form BER length (short form `< 128`,
/// else the shortest long form), as a freshly allocated byte vector.
pub fn encode_ber_length_vec(len: usize) -> Vec<u8> {
    let value = u64::try_from(len).unwrap_or(u64::MAX);
    let mut buf = alloc::vec![0u8; ber_length_size(value)];
    match encode_ber_length_into(value, &mut buf) {
        Ok(written) => buf.truncate(written),
        Err(_) => buf.clear(),
    }
    buf
}

/// Decode a BER length from the front of `bytes`. Returns `(length, consumed)`
/// where `consumed` is the number of length-encoding bytes read (never touches
/// the value that follows).
pub fn ber_length_usize(bytes: &[u8]) -> Result<(usize, usize)> {
    let (len, consumed) = broadcast_common::klv::decode_ber_length(bytes)?;
    let len = usize::try_from(len).map_err(|_| {
        Error::InvalidValue(InvalidValue {
            field: "ber_length",
            value: len,
            reason: "BER length exceeds usize width",
        })
    })?;
    Ok((len, consumed))
}

// ---------------------------------------------------------------------------
// BER-OID tag (Local Set item keys, ISO/IEC 8825-1 §8.19 / SMPTE ST 336)
// ---------------------------------------------------------------------------

/// Encode `tag` as a BER-OID integer (base-128, big-endian, continuation bit set
/// on all but the last byte).
pub fn encode_ber_oid(tag: u32) -> Vec<u8> {
    if tag < BER_OID_CONTINUATION as u32 {
        return alloc::vec![tag as u8];
    }
    let mut digits: Vec<u8> = Vec::new();
    let mut v = tag;
    while v > 0 {
        digits.push((v & BER_LOW7_MASK as u32) as u8);
        v >>= 7;
    }
    digits.reverse();
    let last = digits.len() - 1;
    for (i, d) in digits.iter_mut().enumerate() {
        if i != last {
            *d |= BER_OID_CONTINUATION;
        }
    }
    digits
}

/// Decode a BER-OID tag from the front of `bytes`. Returns `(tag, consumed)`.
pub fn ber_oid(bytes: &[u8]) -> Result<(u32, usize)> {
    let mut value: u32 = 0;
    let mut consumed = 0;
    loop {
        let b = *bytes
            .get(consumed)
            .ok_or(Error::BufferTooShort(BufferTooShort {
                need: consumed + 1,
                have: bytes.len(),
                what: "BER-OID tag byte",
            }))?;
        // Guard against overflow of the accumulator before shifting in 7 bits.
        if value > (u32::MAX >> 7) {
            return Err(Error::InvalidValue(InvalidValue {
                field: "ber_oid",
                value: value as u64,
                reason: "BER-OID tag exceeds u32 range",
            }));
        }
        value = (value << 7) | (b & BER_LOW7_MASK) as u32;
        consumed += 1;
        if b & BER_OID_CONTINUATION == 0 {
            break;
        }
    }
    Ok((value, consumed))
}

/// A single generic KLV triplet: a 16-byte Universal Label key and its owned
/// value bytes.
///
/// The wire form is the shared borrowed [`KlvItem`] from `broadcast_common`,
/// serialized with the canonical minimal BER length; only the owned value
/// bytes live here.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct KlvItem {
    /// The 16-byte SMPTE Universal Label key.
    pub key: UniversalLabel,
    /// The raw value bytes.
    pub value: Vec<u8>,
}

impl KlvItem {
    /// Build a KLV item from a Universal Label key and owned value bytes.
    pub fn new(key: UniversalLabel, value: Vec<u8>) -> Self {
        Self { key, value }
    }

    /// The borrowed triplet this item serializes as.
    #[must_use]
    pub fn as_item(&self) -> Item<'_> {
        Item::new(self.key, &self.value)
    }
}

impl<'a> Parse<'a> for KlvItem {
    type Error = Error;

    fn parse(bytes: &'a [u8]) -> Result<Self> {
        let item = Item::parse(bytes)?;
        Ok(Self {
            key: item.key,
            value: item.value.to_vec(),
        })
    }
}

impl Serialize for KlvItem {
    type Error = Error;

    fn serialized_len(&self) -> usize {
        self.as_item().serialized_len()
    }

    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        Ok(self.as_item().serialize_into(buf)?)
    }
}

// ---------------------------------------------------------------------------
// Local Set items (BER-OID tag + BER length + value)
// ---------------------------------------------------------------------------

/// One item inside a KLV Local Set: a BER-OID `tag`, and a value whose length is
/// computed on serialize.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct LocalSetItem {
    /// The BER-OID tag (item key within the Local Set).
    pub tag: u32,
    /// The raw value bytes.
    pub value: Vec<u8>,
}

impl LocalSetItem {
    /// Build a Local Set item from a tag and owned value bytes.
    pub fn new(tag: u32, value: Vec<u8>) -> Self {
        Self { tag, value }
    }

    /// Bytes this item occupies inside the Local Set value: OID tag + BER length
    /// + value.
    fn encoded_len(&self) -> usize {
        encode_ber_oid(self.tag).len()
            + encode_ber_length_vec(self.value.len()).len()
            + self.value.len()
    }

    /// Append this item's `tag | BER-length | value` encoding to `out`.
    fn encode_into(&self, out: &mut Vec<u8>) {
        out.extend_from_slice(&encode_ber_oid(self.tag));
        out.extend_from_slice(&encode_ber_length_vec(self.value.len()));
        out.extend_from_slice(&self.value);
    }
}

/// Parse the concatenated `tag + BER-length + value` items from a Local Set
/// value section. Consumes the whole slice.
fn parse_local_set_items(mut body: &[u8]) -> Result<Vec<LocalSetItem>> {
    let mut items = Vec::new();
    while !body.is_empty() {
        let (tag, tag_len) = ber_oid(body)?;
        let (val_len, len_len) = ber_length_usize(&body[tag_len..])?;
        let value_start = tag_len + len_len;
        // The BER length is wire-controlled up to `usize::MAX`; bound the end
        // before slicing (r04-C3).
        let value_end = value_start
            .checked_add(val_len)
            .filter(|&end| end <= body.len())
            .ok_or(Error::BufferTooShort(BufferTooShort {
                need: value_start.saturating_add(val_len),
                have: body.len(),
                what: "KLV Local Set item value",
            }))?;
        items.push(LocalSetItem {
            tag,
            value: body[value_start..value_end].to_vec(),
        });
        body = &body[value_end..];
    }
    Ok(items)
}

// ---------------------------------------------------------------------------
// UasLocalSet — MISB ST 0601 UAS Datalink Local Set
// ---------------------------------------------------------------------------

/// A MISB ST 0601 UAS Datalink Local Set packet: the [`UAS_LS_KEY`] Universal
/// Label wrapping a sequence of [`LocalSetItem`]s.
///
/// The outer BER length and the tag-1 Checksum are *computed* on
/// [`Serialize`]/[`serialize_with_checksum`](Self::serialize_with_checksum); the
/// stored `items` should carry the tag-2 Precision Time Stamp and any data tags,
/// but need not carry a checksum item (it is (re)computed on serialize).
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct UasLocalSet {
    /// The Local Set items, in wire order (tag 2 first, tag 1 checksum last).
    pub items: Vec<LocalSetItem>,
}

impl UasLocalSet {
    /// Build an empty UAS Local Set.
    pub fn new() -> Self {
        Self { items: Vec::new() }
    }

    /// Build a UAS Local Set from items (as parsed / assembled).
    pub fn from_items(items: Vec<LocalSetItem>) -> Self {
        Self { items }
    }

    /// The Precision Time Stamp (tag 2), if present and 8 bytes — µs since the
    /// POSIX epoch.
    pub fn precision_timestamp(&self) -> Option<u64> {
        let item = self
            .items
            .iter()
            .find(|i| i.tag == TAG_PRECISION_TIMESTAMP)?;
        if item.value.len() != PRECISION_TIMESTAMP_LEN {
            return None;
        }
        let mut b = [0u8; PRECISION_TIMESTAMP_LEN];
        b.copy_from_slice(&item.value);
        Some(u64::from_be_bytes(b))
    }

    /// The Checksum (tag 1) value carried in the set, if present and 2 bytes.
    pub fn stored_checksum(&self) -> Option<u16> {
        let item = self.items.iter().find(|i| i.tag == TAG_CHECKSUM)?;
        if item.value.len() != CHECKSUM_LEN {
            return None;
        }
        Some(u16::from_be_bytes([item.value[0], item.value[1]]))
    }

    /// Encode the Local Set value section (all items except any checksum, which
    /// is appended by the checksum step) — the items minus the last checksum.
    fn value_without_checksum(&self) -> Vec<u8> {
        let mut out = Vec::new();
        for item in &self.items {
            if item.tag == TAG_CHECKSUM {
                continue;
            }
            item.encode_into(&mut out);
        }
        out
    }

    /// Serialize the full UAS Local Set with a freshly computed tag-1 Checksum
    /// (`checksum_bcc16` over the entire packet, including the UL key and the
    /// checksum tag+length, per MISB ST 0601 tag 1).
    ///
    /// Any checksum item already in `items` is ignored and replaced.
    pub fn serialize_with_checksum(&self) -> Vec<u8> {
        // Value section: all non-checksum items, then the checksum item's
        // tag+length (the value bytes are filled in once they are summed).
        let mut value = self.value_without_checksum();
        value.extend_from_slice(&encode_ber_oid(TAG_CHECKSUM));
        value.extend_from_slice(&encode_ber_length_vec(CHECKSUM_LEN));

        // Packet prefix = UL key + BER length of (value + 2 value bytes) + value.
        let value_total = value.len() + CHECKSUM_LEN;
        let mut packet = Vec::with_capacity(UNIVERSAL_LABEL_LEN + 4 + value_total);
        packet.extend_from_slice(&UAS_LS_KEY);
        packet.extend_from_slice(&encode_ber_length_vec(value_total));
        packet.extend_from_slice(&value);

        // ST 0601 §5.5: the sum runs from the first byte of the 16-byte UL key
        // through the checksum item's own length field — i.e. everything
        // `packet` currently holds, which stops just short of the 2 value bytes.
        let checksum = checksum_bcc16(&packet);
        packet.extend_from_slice(&checksum.to_be_bytes());
        packet
    }

    /// Verify the tag-1 Checksum of a serialized UAS Local Set: recompute the
    /// ST 0601 running sum over everything up to the trailing 2 checksum bytes
    /// and compare.
    ///
    /// Note the coverage is the *whole packet through the checksum's length
    /// field*, so the 2 checksum value bytes are the only exclusion — the UL
    /// key, the BER packet length and every item are all summed.
    pub fn verify_checksum(packet: &[u8]) -> Result<bool> {
        need_in(
            packet,
            UNIVERSAL_LABEL_LEN + 1 + CHECKSUM_LEN,
            "UAS Local Set checksum",
        )?;
        let split = packet.len() - CHECKSUM_LEN;
        let expected = checksum_bcc16(&packet[..split]);
        let actual = u16::from_be_bytes([packet[split], packet[split + 1]]);
        Ok(expected == actual)
    }
}

impl Default for UasLocalSet {
    fn default() -> Self {
        Self::new()
    }
}

impl<'a> Parse<'a> for UasLocalSet {
    type Error = Error;

    fn parse(bytes: &'a [u8]) -> Result<Self> {
        let item = KlvItem::parse(bytes)?;
        if item.key != UAS_LS_KEY {
            return Err(Error::InvalidValue(InvalidValue {
                field: "uas_ls_key",
                value: item.key[0] as u64,
                reason: "not the MISB ST 0601 UAS Datalink Local Set Universal Label",
            }));
        }
        let items = parse_local_set_items(&item.value)?;
        Ok(Self { items })
    }
}

impl Serialize for UasLocalSet {
    type Error = Error;

    fn serialized_len(&self) -> usize {
        // Value = non-checksum items + checksum item (tag + len + 2 value bytes).
        let value_len: usize = self
            .items
            .iter()
            .filter(|i| i.tag != TAG_CHECKSUM)
            .map(|i| i.encoded_len())
            .sum::<usize>()
            + encode_ber_oid(TAG_CHECKSUM).len()
            + encode_ber_length_vec(CHECKSUM_LEN).len()
            + CHECKSUM_LEN;
        UNIVERSAL_LABEL_LEN + encode_ber_length_vec(value_len).len() + value_len
    }

    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        let bytes = self.serialize_with_checksum();
        need_out(buf, bytes.len())?;
        buf[..bytes.len()].copy_from_slice(&bytes);
        Ok(bytes.len())
    }
}

// ---------------------------------------------------------------------------
// Checksum (MISB ST 0601 tag 1)
// ---------------------------------------------------------------------------

/// MISB ST 0601 tag 1 Checksum: the **lower 16 bits of a running 16-bit sum**
/// over the packet (ST 0601 §7.1 "Lower 16-bits of summation"; §5.5 "The
/// checksum is a running 16-bit sum through the entire LDS packet").
///
/// Each byte is summed big-endian within its 16-bit word — byte `i` contributes
/// `b << (8 * ((i + 1) % 2))`, so an even index is the high byte and an odd
/// index the low byte (ST 0601 §5.5's `bcc_16` reference routine). This is the
/// standard's own definition, and it is *not* a CRC: ST 0601's change history
/// records the removal of "REQ-3.04 and references to 'CRC', or 'CRC-16' as tag
/// 1 represents a checksum and not a cyclic redundancy check".
pub fn checksum_bcc16(data: &[u8]) -> u16 {
    let mut bcc: u16 = 0;
    for (i, &byte) in data.iter().enumerate() {
        bcc = bcc.wrapping_add(u16::from(byte) << (8 * ((i + 1) % 2)));
    }
    bcc
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn checksum_bcc16_is_a_big_endian_word_sum() {
        // The ST 0601 §5.5 reference routine sums each byte as the high byte of
        // its 16-bit word at an even index and the low byte at an odd index, so
        // two bytes `AB CD` contribute `0xABCD`.
        assert_eq!(checksum_bcc16(&[0xAB, 0xCD]), 0xABCD);
        // The next byte opens the following word, again as its high byte
        // (`(i + 1) % 2` is 1 at index 2, so the shift is 8).
        assert_eq!(
            checksum_bcc16(&[0xAB, 0xCD, 0xEF]),
            0xABCD_u16.wrapping_add(0xEF00)
        );
        // The sum wraps modulo 2^16 rather than saturating:
        // 0xFFFF + 0xFFFF = 0x1FFFE -> 0xFFFE.
        assert_eq!(checksum_bcc16(&[0xFF, 0xFF, 0xFF, 0xFF]), 0xFFFE);
        assert_eq!(checksum_bcc16(&[]), 0);
    }

    #[test]
    fn ber_short_form() {
        assert_eq!(encode_ber_length_vec(5), alloc::vec![5]);
        assert_eq!(ber_length_usize(&[5, 0xAA]).unwrap(), (5, 1));
    }

    #[test]
    fn ber_long_form_300() {
        // 300 = 0x012C → 82 01 2C.
        assert_eq!(encode_ber_length_vec(300), alloc::vec![0x82, 0x01, 0x2C]);
        assert_eq!(ber_length_usize(&[0x82, 0x01, 0x2C]).unwrap(), (300, 3));
    }

    #[test]
    fn ber_oid_round_trip() {
        for tag in [1u32, 2, 127, 128, 300, 16_383] {
            let enc = encode_ber_oid(tag);
            assert_eq!(ber_oid(&enc).unwrap(), (tag, enc.len()));
        }
    }

    // r04-C3: a long-form BER length of 8×0xFF makes `value_start + len`
    // overflow; KlvItem::parse must return Err, not panic.
    #[test]
    fn rejects_hostile_ber_length() {
        let mut bytes = UAS_LS_KEY.to_vec();
        bytes.push(0x88);
        bytes.extend_from_slice(&[0xFFu8; 8]);
        let err = KlvItem::parse(&bytes).expect_err("hostile BER length must be rejected");
        assert!(
            matches!(
                err,
                Error::Klv(broadcast_common::klv::KlvError::BufferTooShort(..))
            ),
            "expected BufferTooShort, got {err:?}"
        );
    }

    // r04-C3: the same hostile length inside a Local Set item reaches
    // parse_local_set_items via UasLocalSet::parse; must be Err, not panic.
    #[test]
    fn rejects_local_set_hostile_ber_length() {
        // Inner value: tag 2 (BER-OID) + long-form BER length 8×0xFF.
        let mut value = vec![TAG_PRECISION_TIMESTAMP as u8, 0x88];
        value.extend_from_slice(&[0xFFu8; 8]);
        let mut packet = UAS_LS_KEY.to_vec();
        packet.extend_from_slice(&encode_ber_length_vec(value.len()));
        packet.extend_from_slice(&value);
        let err = UasLocalSet::parse(&packet).expect_err("hostile inner BER length must fail");
        assert!(
            matches!(err, Error::BufferTooShort(..)),
            "expected BufferTooShort, got {err:?}"
        );
    }
}
