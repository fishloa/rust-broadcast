//! message_descriptor — ETSI TS 102 006 §9.5.2.12 Table 35 (UNT tag 0x04).
//!
//! Carries a message, in one language per descriptor, that may inform the
//! user about the purpose of the SSU to obtain consent. A message may span
//! several descriptors (`descriptor_number` / `last_descriptor_number`); an
//! occurrence in the operational loop overrides the common loop's. UNT common
//! and operational loops (Table 19).

use crate::descriptors::{descriptor_body, write_descriptor_header};
use crate::error::{Error, Result};
use crate::text::{DvbText, LangCode};
use broadcast_common::{Parse, Serialize};

/// Descriptor tag for message_descriptor (UNT namespace).
pub const TAG: u8 = 0x04;
const HEADER_LEN: usize = 2;
const FIXED_LEN: usize = 4;
const NIBBLE_MASK: u8 = 0x0F;
const NUMBER_SHIFT: u8 = 4;

/// message_descriptor.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[cfg_attr(feature = "yoke", derive(yoke::Yokeable))]
pub struct MessageDescriptor<'a> {
    /// `descriptor_number` (4 bits) — 0 for the first of an associated set.
    pub descriptor_number: u8,
    /// `last_descriptor_number` (4 bits) — number of the last descriptor of
    /// the set.
    pub last_descriptor_number: u8,
    /// `ISO_639_language_code` of the text.
    pub language_code: LangCode,
    /// `text_char`s — the message (EN 300 468 Annex A).
    pub text: DvbText<'a>,
}

impl<'a> Parse<'a> for MessageDescriptor<'a> {
    type Error = Error;
    fn parse(bytes: &'a [u8]) -> Result<Self> {
        let body = descriptor_body(
            bytes,
            TAG,
            "MessageDescriptor",
            "unexpected tag for message_descriptor",
        )?;
        let (f, text) = body
            .split_first_chunk::<FIXED_LEN>()
            .ok_or(Error::InvalidDescriptor {
                tag: TAG,
                reason: "message_descriptor body shorter than 4 bytes",
            })?;
        Ok(Self {
            descriptor_number: f[0] >> NUMBER_SHIFT,
            last_descriptor_number: f[0] & NIBBLE_MASK,
            language_code: LangCode([f[1], f[2], f[3]]),
            text: DvbText::new(text),
        })
    }
}

impl Serialize for MessageDescriptor<'_> {
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
        let number = broadcast_common::len::fit_mask(
            self.descriptor_number,
            NIBBLE_MASK,
            "descriptor_number",
        )?;
        let last = broadcast_common::len::fit_mask(
            self.last_descriptor_number,
            NIBBLE_MASK,
            "last_descriptor_number",
        )?;
        write_descriptor_header(buf, TAG, len - HEADER_LEN)?;
        buf[2] = (number << NUMBER_SHIFT) | last;
        buf[3..6].copy_from_slice(&self.language_code.0);
        buf[6..len].copy_from_slice(self.text.raw());
        Ok(len)
    }
}

impl<'a> crate::traits::DescriptorDef<'a> for MessageDescriptor<'a> {
    const TAG: u8 = TAG;
    const NAME: &'static str = "MESSAGE";
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_and_round_trip() {
        let mut bytes = vec![TAG, 10, 0x01, b'f', b'r', b'e'];
        bytes.extend_from_slice(b"Mise a jour");
        bytes[1] = (bytes.len() - 2) as u8;
        let d = MessageDescriptor::parse(&bytes).unwrap();
        assert_eq!(d.descriptor_number, 0);
        assert_eq!(d.last_descriptor_number, 1);
        assert_eq!(d.language_code, LangCode(*b"fre"));
        assert_eq!(d.text.raw(), b"Mise a jour");
        let mut out = vec![0u8; d.serialized_len()];
        d.serialize_into(&mut out).unwrap();
        assert_eq!(out, bytes);
    }

    #[test]
    fn rejects_short_wrong_tag_and_over_range_numbers() {
        assert!(MessageDescriptor::parse(&[TAG, 3, 0, b'f', b'r']).is_err());
        assert!(MessageDescriptor::parse(&[0x0C, 4, 0, b'f', b'r', b'e']).is_err());
        let d = MessageDescriptor {
            descriptor_number: 16,
            last_descriptor_number: 0,
            language_code: LangCode(*b"eng"),
            text: DvbText::new(b""),
        };
        assert!(d.serialize_into(&mut [0u8; 8]).is_err());
    }
}
