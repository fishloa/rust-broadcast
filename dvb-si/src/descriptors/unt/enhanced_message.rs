//! enhanced_message_descriptor — ETSI TS 102 006 §9.5.2.14 Table 37 (UNT tag
//! 0x0C).
//!
//! Like the message_descriptor plus a manufacturer-defined `message_index`
//! (5 bits) letting the receiver pick the message matching its state; only
//! one message per (language, message_index) per loop, an operational-loop
//! occurrence overrides the common loop's. Per the spec `descriptor_length`
//! shall be greater than 4, i.e. at least one `text_char`. UNT common and
//! operational loops (Table 19).
//!
//! The 3-bit `reserved_for_future_use` has no value column in Table 37; the
//! reserved-bit audit (`dvb-si/docs/reserved-bits-edition-audit.md`, TS 102 006
//! row) records the convention `1` (`0b111`). It is preserved on parse and
//! re-emitted verbatim; hand-built descriptors default to it.

use crate::descriptors::{descriptor_body, write_descriptor_header};
use crate::error::{Error, Result};
use crate::text::{DvbText, LangCode};
use broadcast_common::{Parse, Serialize};

/// Descriptor tag for enhanced_message_descriptor (UNT namespace).
pub const TAG: u8 = 0x0C;
const HEADER_LEN: usize = 2;
const FIXED_LEN: usize = 5;
const NIBBLE_MASK: u8 = 0x0F;
const NUMBER_SHIFT: u8 = 4;
const RESERVED_SHIFT: u8 = 5;
const RESERVED_MASK: u8 = 0x07;
const MESSAGE_INDEX_MASK: u8 = 0x1F;
/// Default of the 3-bit `reserved_for_future_use`: `0b111` (all-ones
/// reserved convention; `dvb-si/docs/reserved-bits-edition-audit.md`).
pub const RESERVED_FOR_FUTURE_USE_DEFAULT: u8 = 0b111;

/// enhanced_message_descriptor.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[cfg_attr(feature = "yoke", derive(yoke::Yokeable))]
pub struct EnhancedMessageDescriptor<'a> {
    /// `descriptor_number` (4 bits) — 0 for the first of an associated set.
    pub descriptor_number: u8,
    /// `last_descriptor_number` (4 bits).
    pub last_descriptor_number: u8,
    /// `ISO_639_language_code` of the text.
    pub language_code: LangCode,
    /// `reserved_for_future_use` (3 bits, right-aligned). Captured on parse
    /// and re-emitted verbatim; [`Self::new`] defaults to
    /// [`RESERVED_FOR_FUTURE_USE_DEFAULT`].
    pub reserved_for_future_use: u8,
    /// `message_index` (5 bits) — manufacturer-defined meaning.
    pub message_index: u8,
    /// `text_char`s — the message (EN 300 468 Annex A).
    pub text: DvbText<'a>,
}

impl<'a> EnhancedMessageDescriptor<'a> {
    /// Build a descriptor with the spec-default reserved bits.
    #[must_use]
    pub fn new(
        descriptor_number: u8,
        last_descriptor_number: u8,
        language_code: LangCode,
        message_index: u8,
        text: DvbText<'a>,
    ) -> Self {
        Self {
            descriptor_number,
            last_descriptor_number,
            language_code,
            reserved_for_future_use: RESERVED_FOR_FUTURE_USE_DEFAULT,
            message_index,
            text,
        }
    }
}

impl<'a> Parse<'a> for EnhancedMessageDescriptor<'a> {
    type Error = Error;
    fn parse(bytes: &'a [u8]) -> Result<Self> {
        let body = descriptor_body(
            bytes,
            TAG,
            "EnhancedMessageDescriptor",
            "unexpected tag for enhanced_message_descriptor",
        )?;
        let (f, text) = body
            .split_first_chunk::<FIXED_LEN>()
            .ok_or(Error::InvalidDescriptor {
                tag: TAG,
                reason: "enhanced_message_descriptor body shorter than 5 bytes",
            })?;
        Ok(Self {
            descriptor_number: f[0] >> NUMBER_SHIFT,
            last_descriptor_number: f[0] & NIBBLE_MASK,
            language_code: LangCode([f[1], f[2], f[3]]),
            reserved_for_future_use: f[4] >> RESERVED_SHIFT,
            message_index: f[4] & MESSAGE_INDEX_MASK,
            text: DvbText::new(text),
        })
    }
}

impl Serialize for EnhancedMessageDescriptor<'_> {
    type Error = Error;
    fn serialized_len(&self) -> usize {
        HEADER_LEN + FIXED_LEN + self.text.len()
    }
    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        let len = self.serialized_len();
        if buf.len() < len {
            return Err(Error::OutputBufferTooSmall {
                need: len,
                have: buf.len(),
            });
        }
        let number =
            broadcast_common::len::fit_mask(self.descriptor_number, NIBBLE_MASK, "descriptor_number")?;
        let last = broadcast_common::len::fit_mask(
            self.last_descriptor_number,
            NIBBLE_MASK,
            "last_descriptor_number",
        )?;
        let reserved = broadcast_common::len::fit_mask(
            self.reserved_for_future_use,
            RESERVED_MASK,
            "reserved_for_future_use",
        )?;
        let index =
            broadcast_common::len::fit_mask(self.message_index, MESSAGE_INDEX_MASK, "message_index")?;
        write_descriptor_header(buf, TAG, len - HEADER_LEN)?;
        buf[2] = (number << NUMBER_SHIFT) | last;
        buf[3..6].copy_from_slice(&self.language_code.0);
        buf[6] = (reserved << RESERVED_SHIFT) | index;
        buf[7..len].copy_from_slice(self.text.raw());
        Ok(len)
    }
}

impl<'a> crate::traits::DescriptorDef<'a> for EnhancedMessageDescriptor<'a> {
    const TAG: u8 = TAG;
    const NAME: &'static str = "ENHANCED_MESSAGE";
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_and_round_trip() {
        // number 0 of 0, "eng", reserved 111, index 31, text "Hi".
        let bytes = [TAG, 7, 0x00, b'e', b'n', b'g', 0xFF, b'H', b'i'];
        let d = EnhancedMessageDescriptor::parse(&bytes).unwrap();
        assert_eq!(d.reserved_for_future_use, 0b111);
        assert_eq!(d.message_index, 31);
        assert_eq!(d.text.raw(), b"Hi");
        let mut out = vec![0u8; d.serialized_len()];
        d.serialize_into(&mut out).unwrap();
        assert_eq!(out, bytes);
    }

    #[test]
    fn off_spec_reserved_bits_are_preserved() {
        // A sender writing reserved = 000 must re-serialize byte-identically.
        let bytes = [TAG, 6, 0x10, b'f', b'r', b'e', 0x01, b'x'];
        let d = EnhancedMessageDescriptor::parse(&bytes).unwrap();
        assert_eq!(d.reserved_for_future_use, 0);
        assert_eq!((d.descriptor_number, d.last_descriptor_number), (1, 0));
        let mut out = vec![0u8; d.serialized_len()];
        d.serialize_into(&mut out).unwrap();
        assert_eq!(out, bytes);
        let built = EnhancedMessageDescriptor::new(1, 0, LangCode(*b"fre"), 1, DvbText::new(b"x"));
        assert_eq!(built.reserved_for_future_use, 0b111);
    }

    #[test]
    fn rejects_short_wrong_tag_and_over_range() {
        assert!(EnhancedMessageDescriptor::parse(&[TAG, 4, 0, b'e', b'n', b'g']).is_err());
        assert!(EnhancedMessageDescriptor::parse(&[0x04, 5, 0, b'e', b'n', b'g', 0]).is_err());
        let d = EnhancedMessageDescriptor::new(0, 0, LangCode(*b"eng"), 32, DvbText::new(b""));
        assert!(d.serialize_into(&mut [0u8; 8]).is_err());
    }
}
