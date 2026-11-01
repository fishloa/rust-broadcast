//! ATSC `caption_service_descriptor` — ATSC A/65:2013 §6.9.2 Table 6.26
//! (`private/specs-verified/atsc/a65/a65-caption-service-descriptor.md`), tag `0x86`.
//!
//! Announces the CEA-608 data streams and CEA-708 caption services a video
//! stream carries; it belongs in the PMT descriptor loop of the video
//! elementary stream and in EIT events. Tag `0x86` is context dependent (the
//! Chinese GY/T 270 descriptor reuses it with a different layout), so it is not
//! auto-dispatched: enable it with
//! [`crate::descriptors::DescriptorRegistry::with_atsc_caption_service`].
//!
//! ```text
//! descriptor_tag(8)=0x86 descriptor_length(8)
//! reserved(3)='111' number_of_services(5)
//! for each service (6 bytes):
//!   language(24) digital_cc(1) reserved(1)='1'
//!   digital_cc==0: reserved(5)='11111' line21_field(1)
//!   digital_cc==1: caption_service_number(6)
//!   easy_reader(1) wide_aspect_ratio(1) reserved(14)='11111111111111'
//! ```
//!
//! Every reserved run is captured on parse and re-emitted verbatim; hand-built
//! values default to the spec's all-ones.

use super::{descriptor_body, write_descriptor_header};
use crate::error::{Error, Result};
use crate::text::LangCode;
use alloc::vec::Vec;
use broadcast_common::len::{fit_bits, fit_u8};
use broadcast_common::{Parse, Serialize};

/// Descriptor tag for the ATSC caption_service_descriptor.
pub const TAG: u8 = 0x86;
const HEADER_LEN: usize = 2;
const COUNT_BYTE_LEN: usize = 1;
const SERVICE_LEN: usize = 6;
const LANGUAGE_LEN: usize = 3;
const MAX_SERVICES: usize = 16;
const COUNT_MASK: u8 = 0x1F;
const RESERVED_HEAD_SHIFT: u32 = 5;
const DIGITAL_CC_BIT: u8 = 0x80;
const RESERVED_A_BIT: u8 = 0x40;
const RESERVED_A_SHIFT: u32 = 6;
const SERVICE_NUMBER_MASK: u8 = 0x3F;
const LINE21_FIELD_BIT: u8 = 0x01;
const LINE21_RESERVED_MASK: u8 = 0x3E;
const LINE21_RESERVED_SHIFT: u32 = 1;
const EASY_READER_BIT: u8 = 0x80;
const WIDE_ASPECT_RATIO_BIT: u8 = 0x40;
const RESERVED_B_HI_MASK: u8 = 0x3F;
const RESERVED_HEAD_ONES: u8 = 0b111;
const RESERVED_A_ONES: u8 = 1;
const RESERVED_LINE21_ONES: u8 = 0b11111;
const RESERVED_B_ONES: u16 = 0x3FFF;
const RESERVED_HEAD_BITS: u32 = 3;
const RESERVED_LINE21_BITS: u32 = 5;
const RESERVED_B_BITS: u32 = 14;
const SERVICE_NUMBER_BITS: u32 = 6;

/// The `digital_cc` branch of one service entry (Table 6.26).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub enum AtscCaptionKind {
    /// `digital_cc` = 0: a CEA-608 data stream.
    Cea608 {
        /// `reserved` (5 bits) before `line21_field`.
        reserved: u8,
        /// `line21_field` (1 bit); A/65 does not expect receivers to use it.
        line21_field: bool,
    },
    /// `digital_cc` = 1: a CEA-708 service.
    Cea708 {
        /// `caption_service_number` (6 bits); zero is prohibited by A/65.
        caption_service_number: u8,
    },
}

impl AtscCaptionKind {
    /// Spec label (`TYPE OF SERVICE`, A/65 §6.9.2).
    #[must_use]
    pub fn name(&self) -> &'static str {
        match self {
            Self::Cea608 { .. } => "608",
            Self::Cea708 { .. } => "708",
        }
    }
}
broadcast_common::impl_spec_display!(AtscCaptionKind);

/// One caption service entry of the descriptor loop (Table 6.26).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub struct AtscCaptionService {
    /// `language` (24 bits): ISO 639-2/B code, ISO 8859-1 characters.
    pub language: LangCode,
    /// `reserved` (1 bit) after `digital_cc`.
    pub reserved_a: u8,
    /// `digital_cc` branch: 608 stream or 708 service.
    pub kind: AtscCaptionKind,
    /// `easy_reader`.
    pub easy_reader: bool,
    /// `wide_aspect_ratio`: `false` = 4:3, `true` = 16:9.
    pub wide_aspect_ratio: bool,
    /// `reserved` (14 bits) closing the entry.
    pub reserved_b: u16,
}

impl AtscCaptionService {
    /// A CEA-608 entry with the spec's all-ones reserved defaults.
    #[must_use]
    pub const fn cea608(
        language: LangCode,
        line21_field: bool,
        easy_reader: bool,
        wide_aspect_ratio: bool,
    ) -> Self {
        Self {
            language,
            reserved_a: RESERVED_A_ONES,
            kind: AtscCaptionKind::Cea608 {
                reserved: RESERVED_LINE21_ONES,
                line21_field,
            },
            easy_reader,
            wide_aspect_ratio,
            reserved_b: RESERVED_B_ONES,
        }
    }

    /// A CEA-708 entry with the spec's all-ones reserved defaults.
    #[must_use]
    pub const fn cea708(
        language: LangCode,
        caption_service_number: u8,
        easy_reader: bool,
        wide_aspect_ratio: bool,
    ) -> Self {
        Self {
            language,
            reserved_a: RESERVED_A_ONES,
            kind: AtscCaptionKind::Cea708 {
                caption_service_number,
            },
            easy_reader,
            wide_aspect_ratio,
            reserved_b: RESERVED_B_ONES,
        }
    }
}

/// ATSC `caption_service_descriptor` (A/65 Table 6.26).
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub struct AtscCaptionServiceDescriptor {
    /// `reserved` (3 bits) before `number_of_services`.
    pub reserved_head: u8,
    /// The caption services; `number_of_services` is derived on serialize.
    pub services: Vec<AtscCaptionService>,
}

impl AtscCaptionServiceDescriptor {
    /// Build a descriptor with all-ones reserved defaults.
    #[must_use]
    pub fn new(services: Vec<AtscCaptionService>) -> Self {
        Self {
            reserved_head: RESERVED_HEAD_ONES,
            services,
        }
    }
}

impl<'a> Parse<'a> for AtscCaptionServiceDescriptor {
    type Error = crate::error::Error;
    fn parse(bytes: &'a [u8]) -> Result<Self> {
        let body = descriptor_body(
            bytes,
            TAG,
            "AtscCaptionServiceDescriptor",
            "unexpected tag for caption_service_descriptor",
        )?;
        let Some(&head) = body.first() else {
            return Err(Error::BufferTooShort {
                need: COUNT_BYTE_LEN,
                have: 0,
                what: "AtscCaptionServiceDescriptor",
            });
        };
        let n = usize::from(head & COUNT_MASK);
        if body.len() != COUNT_BYTE_LEN + n * SERVICE_LEN {
            return Err(Error::InvalidDescriptor {
                tag: TAG,
                reason: "descriptor_length does not match number_of_services",
            });
        }
        let services = body[COUNT_BYTE_LEN..]
            .chunks_exact(SERVICE_LEN)
            .map(|chunk| {
                let flags = chunk[LANGUAGE_LEN];
                let kind = if flags & DIGITAL_CC_BIT == 0 {
                    AtscCaptionKind::Cea608 {
                        reserved: (flags & LINE21_RESERVED_MASK) >> LINE21_RESERVED_SHIFT,
                        line21_field: flags & LINE21_FIELD_BIT != 0,
                    }
                } else {
                    AtscCaptionKind::Cea708 {
                        caption_service_number: flags & SERVICE_NUMBER_MASK,
                    }
                };
                let tail = chunk[LANGUAGE_LEN + 1];
                AtscCaptionService {
                    language: LangCode([chunk[0], chunk[1], chunk[2]]),
                    reserved_a: (flags & RESERVED_A_BIT) >> RESERVED_A_SHIFT,
                    kind,
                    easy_reader: tail & EASY_READER_BIT != 0,
                    wide_aspect_ratio: tail & WIDE_ASPECT_RATIO_BIT != 0,
                    reserved_b: (u16::from(tail & RESERVED_B_HI_MASK) << u8::BITS)
                        | u16::from(chunk[LANGUAGE_LEN + 2]),
                }
            })
            .collect();
        Ok(Self {
            reserved_head: head >> RESERVED_HEAD_SHIFT,
            services,
        })
    }
}

impl Serialize for AtscCaptionServiceDescriptor {
    type Error = crate::error::Error;
    fn serialized_len(&self) -> usize {
        HEADER_LEN + COUNT_BYTE_LEN + self.services.len() * SERVICE_LEN
    }

    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        let len = self.serialized_len();
        if buf.len() < len {
            return Err(Error::OutputBufferTooSmall {
                need: len,
                have: buf.len(),
            });
        }
        if self.services.len() > MAX_SERVICES {
            return Err(Error::ValueOutOfRange {
                field: "number_of_services",
                reason: "exceeds the 16 services A/65 allows",
            });
        }
        write_descriptor_header(buf, TAG, len - HEADER_LEN)?;
        let count = fit_u8(self.services.len(), "number_of_services")?;
        let head = fit_bits(
            u64::from(self.reserved_head),
            RESERVED_HEAD_BITS,
            "reserved",
        )? as u8;
        buf[HEADER_LEN] = (head << RESERVED_HEAD_SHIFT) | count;
        let mut i = HEADER_LEN + COUNT_BYTE_LEN;
        for s in &self.services {
            let reserved_a = fit_bits(u64::from(s.reserved_a), 1, "reserved")? as u8;
            let (kind_bit, low) = match s.kind {
                AtscCaptionKind::Cea608 {
                    reserved,
                    line21_field,
                } => {
                    let reserved =
                        fit_bits(u64::from(reserved), RESERVED_LINE21_BITS, "reserved")? as u8;
                    (
                        0,
                        (reserved << LINE21_RESERVED_SHIFT) | u8::from(line21_field),
                    )
                }
                AtscCaptionKind::Cea708 {
                    caption_service_number,
                } => (
                    DIGITAL_CC_BIT,
                    fit_bits(
                        u64::from(caption_service_number),
                        SERVICE_NUMBER_BITS,
                        "caption_service_number",
                    )? as u8,
                ),
            };
            let reserved_b = fit_bits(u64::from(s.reserved_b), RESERVED_B_BITS, "reserved")? as u16;
            buf[i..i + LANGUAGE_LEN].copy_from_slice(&s.language.0);
            buf[i + LANGUAGE_LEN] = kind_bit | (reserved_a << RESERVED_A_SHIFT) | low;
            buf[i + LANGUAGE_LEN + 1] = (u8::from(s.easy_reader) * EASY_READER_BIT)
                | (u8::from(s.wide_aspect_ratio) * WIDE_ASPECT_RATIO_BIT)
                | (reserved_b >> u8::BITS) as u8;
            buf[i + LANGUAGE_LEN + 2] = (reserved_b & u16::from(u8::MAX)) as u8;
            i += SERVICE_LEN;
        }
        Ok(i)
    }
}

impl crate::traits::DescriptorDef<'_> for AtscCaptionServiceDescriptor {
    const TAG: u8 = TAG;
    const NAME: &'static str = "ATSC_CAPTION_SERVICE";
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::string::ToString;
    use alloc::vec;

    fn round_trip(bytes: &[u8]) -> AtscCaptionServiceDescriptor {
        let d = AtscCaptionServiceDescriptor::parse(bytes).unwrap();
        let mut out = vec![0u8; d.serialized_len()];
        let n = d.serialize_into(&mut out).unwrap();
        assert_eq!(&out[..n], bytes, "byte-exact round trip");
        d
    }

    /// A/65 Table 6.26: one 708 service 'eng' number 1, 16:9, not easy reader.
    #[test]
    fn table_6_26_single_708_service() {
        let bytes = [TAG, 7, 0xE1, b'e', b'n', b'g', 0xC1, 0x7F, 0xFF];
        let d = round_trip(&bytes);
        let s = d.services[0];
        assert_eq!(s.language, LangCode(*b"eng"));
        assert_eq!(
            s.kind,
            AtscCaptionKind::Cea708 {
                caption_service_number: 1
            }
        );
        assert!(!s.easy_reader);
        assert!(s.wide_aspect_ratio);
        assert_eq!(
            (d.reserved_head, s.reserved_a, s.reserved_b),
            (7, 1, 0x3FFF)
        );
    }

    /// One 608 entry (line21_field set, easy reader, 4:3) then one 708 entry.
    #[test]
    fn table_6_26_mixed_608_and_708() {
        let bytes = [
            TAG, 13, 0xE2, b's', b'p', b'a', 0x7F, 0xBF, 0xFF, b'e', b'n', b'g', 0xC2, 0x7F, 0xFF,
        ];
        let d = round_trip(&bytes);
        assert_eq!(
            d.services[0].kind,
            AtscCaptionKind::Cea608 {
                reserved: 0b11111,
                line21_field: true
            }
        );
        assert!(d.services[0].easy_reader);
        assert!(!d.services[0].wide_aspect_ratio);
        assert_eq!(d.services[1].kind.name(), "708");
    }

    #[test]
    fn non_default_reserved_bits_survive() {
        let bytes = [TAG, 7, 0xA1, b'f', b'r', b'e', 0x80, 0x00, 0x00];
        let d = round_trip(&bytes);
        assert_eq!(d.reserved_head, 5);
        assert_eq!(d.services[0].reserved_a, 0);
        assert_eq!(d.services[0].reserved_b, 0);
    }

    #[test]
    fn rejects_bad_tag_length_and_truncation() {
        assert!(AtscCaptionServiceDescriptor::parse(&[0x87, 1, 0xE0]).is_err());
        assert!(AtscCaptionServiceDescriptor::parse(&[TAG, 1, 0xE1]).is_err());
        assert!(AtscCaptionServiceDescriptor::parse(&[TAG, 0]).is_err());
        assert!(AtscCaptionServiceDescriptor::parse(&[TAG, 7, 0xE1]).is_err());
    }

    #[test]
    fn serialize_rejects_overwide_fields() {
        let d = AtscCaptionServiceDescriptor::new(vec![AtscCaptionService::cea708(
            LangCode(*b"eng"),
            64,
            false,
            false,
        )]);
        assert!(d.serialize_into(&mut [0u8; 16]).is_err());
        let many = AtscCaptionServiceDescriptor::new(vec![
            AtscCaptionService::cea608(
                LangCode(*b"eng"),
                false,
                false,
                false
            );
            17
        ]);
        assert!(many.serialize_into(&mut [0u8; 256]).is_err());
    }

    #[test]
    fn built_defaults_are_all_ones() {
        let d = AtscCaptionServiceDescriptor::new(vec![
            AtscCaptionService::cea608(LangCode(*b"eng"), false, false, false),
            AtscCaptionService::cea708(LangCode(*b"spa"), 2, true, true),
        ]);
        let mut out = vec![0u8; d.serialized_len()];
        d.serialize_into(&mut out).unwrap();
        assert_eq!(
            out,
            [
                TAG, 13, 0xE2, b'e', b'n', b'g', 0x7E, 0x3F, 0xFF, b's', b'p', b'a', 0xC2, 0xFF,
                0xFF
            ]
        );
    }

    #[test]
    fn kind_label() {
        assert_eq!(
            AtscCaptionKind::Cea608 {
                reserved: 31,
                line21_field: false
            }
            .to_string(),
            "608"
        );
    }
}
