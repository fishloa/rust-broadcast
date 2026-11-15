//! `anc_data_descriptor` (tag `0xC4`) + the `"VANC"` registration_descriptor
//! `format_identifier` — SMPTE ST 2038:2021 §4.1 (Table 1, p. 4).
//!
//! The ANC data stream is signalled in the PMT with `stream_type == 0x06`
//! (PES private data, §4.1.1), preceded by an ISO/IEC 13818-1
//! `registration_descriptor` carrying `format_identifier == 0x56414E43`
//! (`"VANC"`, §4.1.3), and accompanied by the `anc_data_descriptor` (tag
//! `0xC4`, §4.1.2) whose body is a descriptor loop the spec leaves undefined
//! ("Compliant receive devices shall ignore unrecognized descriptors",
//! §4.1.2.1). Each entry is walked with the ISO/IEC 13818-1 §2.6
//! `descriptor()` syntax into a typed [`Descriptor`].

use alloc::vec::Vec;

use broadcast_common::Serialize;
use broadcast_common::len::{need_in, need_out};

use crate::error::{Error, Result};

/// `stream_type` for the ANC data ES in the PMT — `0x06` (PES private data),
/// ST 2038 §4.1.1.
pub const ANC_STREAM_TYPE: u8 = 0x06;

/// `anc_data_descriptor` tag — `0xC4` (user-defined in ATSC/DVB/SCTE),
/// ST 2038 §4.1.2.
pub const ANC_DATA_DESCRIPTOR_TAG: u8 = 0xC4;

/// `registration_descriptor` `format_identifier` for ST 2038 — `0x56414E43`
/// (the ASCII `"VANC"`), ST 2038 §4.1.3.
pub const VANC_FORMAT_IDENTIFIER: u32 = 0x5641_4E43;

/// The four ASCII bytes of [`VANC_FORMAT_IDENTIFIER`] (`b"VANC"`).
pub const VANC_FORMAT_IDENTIFIER_BYTES: [u8; 4] = *b"VANC";

/// Minimum wire size: `descriptor_tag` + `descriptor_length`.
const HEADER_LEN: usize = 2;

/// One entry of an [`AncDataDescriptor`]'s inner descriptor loop
/// (ISO/IEC 13818-1 §2.6 `descriptor()`: an 8-bit `descriptor_tag`, an 8-bit
/// `descriptor_length`, then that many body octets).
///
/// ST 2038 §4.1.2.1 defines no descriptors for this loop, so the clause-owning
/// reader is `dvb-si`'s tag registry ([`Descriptor`]); an entry this crate
/// cannot interpret is still kept, as [`Descriptor::Other`], and reserializes
/// byte-identically.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub enum Descriptor<'a> {
    /// Any descriptor body this crate does not interpret, kept verbatim for
    /// a lossless round-trip ("compliant receive devices shall ignore
    /// unrecognized descriptors", ST 2038 §4.1.2.1).
    Other {
        /// `descriptor_tag`.
        tag: u8,
        /// `descriptor_length` body octets.
        #[cfg_attr(feature = "serde", serde(borrow, with = "serde_bytes_compat"))]
        body: &'a [u8],
    },
}

impl<'a> Descriptor<'a> {
    /// The spec token for this variant (issue #204 label convention). ST 2038
    /// names no descriptor for this loop, so every entry is "other".
    #[must_use]
    pub fn name(&self) -> &'static str {
        match self {
            Self::Other { .. } => "other",
        }
    }

    /// The `descriptor_tag`.
    #[must_use]
    pub fn tag(&self) -> u8 {
        match self {
            Self::Other { tag, .. } => *tag,
        }
    }

    /// The `descriptor_length` body octets.
    #[must_use]
    pub fn body(&self) -> &'a [u8] {
        match self {
            Self::Other { body, .. } => body,
        }
    }

    /// Parse one descriptor from the start of `b`, returning it with the
    /// octets it occupies (tag + length + body).
    ///
    /// # Errors
    /// [`Error::BufferTooShort`] if the header or the declared body is
    /// truncated.
    fn parse(b: &'a [u8]) -> Result<(Self, usize)> {
        need_in(b, HEADER_LEN, "descriptor header")?;
        let tag = b[0];
        let len = usize::from(b[1]);
        let end = HEADER_LEN + len;
        need_in(b, end, "descriptor body")?;
        Ok((
            Self::Other {
                tag,
                body: &b[HEADER_LEN..end],
            },
            end,
        ))
    }

    /// Wire size: `descriptor_tag` + `descriptor_length` + body.
    #[must_use]
    pub fn serialized_len(&self) -> usize {
        HEADER_LEN + self.body().len()
    }

    /// Whether serialize would write a different length than the stored one
    /// (an 8-bit `descriptor_length` caps the body at 255 octets).
    fn checked_length(&self) -> Result<u8> {
        let len = self.body().len();
        broadcast_common::len::fit_bits(len as u64, 8, "descriptor_length")?;
        Ok(len as u8)
    }
}

impl core::fmt::Display for Descriptor<'_> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "{}(0x{:02X})", self.name(), self.tag())
    }
}

impl Serialize for Descriptor<'_> {
    type Error = Error;

    fn serialized_len(&self) -> usize {
        Descriptor::serialized_len(self)
    }

    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        let len = self.serialized_len();
        need_out(buf, len)?;
        let tag = self.tag();
        let body_len = self.checked_length()?;
        buf[0] = tag;
        buf[1] = body_len;
        buf[HEADER_LEN..len].copy_from_slice(self.body());
        Ok(len)
    }
}

/// The `anc_data_descriptor` (Table 1): a tag/length wrapper around its inner
/// descriptor loop. The `descriptor_length` body is walked into typed
/// [`Descriptor`] entries (ST 2038 §4.1.2.1 leaves their meaning to the
/// receiver); the loop is stored typed rather than as the raw body octets, so
/// a caller builds and inspects descriptors instead of hand-assembling the
/// tag/length framing.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct AncDataDescriptor<'a> {
    /// The `descriptor()` loop entries (`descriptor_length` octets,
    /// concatenated), undefined by ST 2038 §4.1.2.1 and ignored by compliant
    /// receivers.
    pub descriptors: Vec<Descriptor<'a>>,
}

impl<'a> AncDataDescriptor<'a> {
    /// Parse an `anc_data_descriptor` from the bytes starting at its
    /// `descriptor_tag`.
    ///
    /// # Errors
    /// [`Error::BufferTooShort`] if truncated; [`Error::BadDescriptorTag`] if
    /// the tag byte is not [`ANC_DATA_DESCRIPTOR_TAG`].
    pub fn parse(b: &'a [u8]) -> Result<Self> {
        need_in(b, HEADER_LEN, "anc_data_descriptor header")?;
        if b[0] != ANC_DATA_DESCRIPTOR_TAG {
            return Err(Error::BadDescriptorTag(b[0]));
        }
        let len = usize::from(b[1]);
        let end = HEADER_LEN + len;
        need_in(b, end, "anc_data_descriptor body")?;
        let mut rest = &b[HEADER_LEN..end];
        let mut descriptors = Vec::new();
        while !rest.is_empty() {
            let (descriptor, used) = Descriptor::parse(rest)?;
            descriptors.push(descriptor);
            rest = &rest[used..];
        }
        Ok(Self { descriptors })
    }

    /// The descriptor loop's wire size: the sum of its entries' tag/length
    /// framing and bodies.
    fn body_len(&self) -> usize {
        self.descriptors
            .iter()
            .map(Descriptor::serialized_len)
            .sum()
    }

    /// Serialized length in bytes (tag + length + loop body).
    #[must_use]
    pub fn serialized_len(&self) -> usize {
        HEADER_LEN + self.body_len()
    }

    /// Serialize back to bytes.
    ///
    /// # Errors
    /// [`Error::BufferTooShort`] if `buf` is too small; [`Error::FieldOverflow`]
    /// if the loop body exceeds the 8-bit `descriptor_length` or an entry
    /// body exceeds its own 8-bit `descriptor_length`.
    pub fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        let len = self.serialized_len();
        need_out(buf, len)?;
        let body_len = broadcast_common::len::fit_bits(
            self.body_len() as u64,
            8,
            "anc_data_descriptor descriptor_length",
        )? as u8;
        buf[0] = ANC_DATA_DESCRIPTOR_TAG;
        buf[1] = body_len;
        let mut pos = HEADER_LEN;
        for descriptor in &self.descriptors {
            let wrote = descriptor.serialize_into(&mut buf[pos..])?;
            pos += wrote;
        }
        Ok(pos)
    }
}

#[cfg(feature = "serde")]
mod serde_bytes_compat {
    pub fn serialize<S: serde::Serializer>(b: &[u8], s: S) -> Result<S::Ok, S::Error> {
        s.serialize_bytes(b)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    extern crate alloc;
    use alloc::vec;

    #[test]
    fn vanc_identifier_is_ascii() {
        assert_eq!(VANC_FORMAT_IDENTIFIER.to_be_bytes(), *b"VANC");
        assert_eq!(VANC_FORMAT_IDENTIFIER_BYTES, *b"VANC");
    }

    #[test]
    fn serialize_into_short_output_is_output_buffer_too_small() {
        let b = [ANC_DATA_DESCRIPTOR_TAG, 0x03, 0x05, 0x01, 0xAA];
        let d = AncDataDescriptor::parse(&b).unwrap();
        let mut out = vec![0u8; b.len() - 1];
        assert_eq!(
            d.serialize_into(&mut out),
            Err(Error::OutputBufferTooSmall(
                broadcast_common::error::OutputBufferTooSmall {
                    need: b.len(),
                    have: b.len() - 1
                }
            ))
        );
    }

    #[test]
    fn round_trip_empty_body() {
        let b = [ANC_DATA_DESCRIPTOR_TAG, 0x00];
        let d = AncDataDescriptor::parse(&b).unwrap();
        assert!(d.descriptors.is_empty());
        let mut out = vec![0u8; d.serialized_len()];
        d.serialize_into(&mut out).unwrap();
        assert_eq!(out, b);
    }

    #[test]
    fn round_trip_with_body() {
        let b = [ANC_DATA_DESCRIPTOR_TAG, 0x03, 0x05, 0x01, 0xAA];
        let d = AncDataDescriptor::parse(&b).unwrap();
        assert_eq!(
            d.descriptors,
            vec![Descriptor::Other {
                tag: 0x05,
                body: &[0xAA]
            }]
        );
        let mut out = vec![0u8; d.serialized_len()];
        d.serialize_into(&mut out).unwrap();
        assert_eq!(out, b);
    }

    /// The loop is typed: each entry's tag and body are reachable without the
    /// caller re-walking `descriptor_tag`/`descriptor_length` by hand, and a
    /// descriptor built in code serializes to exactly the wire form a parse
    /// of those same octets produces.
    #[test]
    fn inner_loop_is_typed_and_builds_the_wire_form() {
        let b = [
            ANC_DATA_DESCRIPTOR_TAG,
            0x06,
            0x05,
            0x02,
            0xAA,
            0xBB,
            0x7F,
            0x00,
        ];
        let parsed = AncDataDescriptor::parse(&b).unwrap();
        assert_eq!(parsed.descriptors.len(), 2);
        assert_eq!(
            (parsed.descriptors[0].tag(), parsed.descriptors[0].body()),
            (0x05, &[0xAA, 0xBB][..])
        );
        assert_eq!(
            (parsed.descriptors[1].tag(), parsed.descriptors[1].body()),
            (0x7F, &[][..])
        );

        let built = AncDataDescriptor {
            descriptors: vec![
                Descriptor::Other {
                    tag: 0x05,
                    body: &[0xAA, 0xBB],
                },
                Descriptor::Other {
                    tag: 0x7F,
                    body: &[],
                },
            ],
        };
        assert_eq!(built, parsed);
        let mut out = vec![0u8; built.serialized_len()];
        built.serialize_into(&mut out).unwrap();
        assert_eq!(out, b);
    }

    #[test]
    fn rejects_truncated_inner_descriptor_header() {
        let b = [ANC_DATA_DESCRIPTOR_TAG, 0x01, 0x05];
        assert!(matches!(
            AncDataDescriptor::parse(&b),
            Err(Error::BufferTooShort(_))
        ));
    }

    #[test]
    fn rejects_truncated_inner_descriptor_body() {
        let b = [ANC_DATA_DESCRIPTOR_TAG, 0x03, 0x05, 0x04, 0xAA];
        assert!(matches!(
            AncDataDescriptor::parse(&b),
            Err(Error::BufferTooShort(_))
        ));
    }

    #[test]
    fn rejects_bad_tag() {
        let b = [0xC5, 0x00];
        assert!(matches!(
            AncDataDescriptor::parse(&b),
            Err(Error::BadDescriptorTag(0xC5))
        ));
    }

    #[test]
    fn rejects_truncated_body() {
        let b = [ANC_DATA_DESCRIPTOR_TAG, 0x05, 0x01];
        assert!(matches!(
            AncDataDescriptor::parse(&b),
            Err(Error::BufferTooShort(..))
        ));
    }
}
