//! Content Identifier Descriptor — ETSI TS 102 323 §12.1 (tag 0x76).
//!
//! Carries one or more Content Reference Identifier (CRID) entries that
//! uniquely identify TV/radio programmes, series, or recommendations.
//! Each entry specifies a type and a location indicator that determines
//! whether the CRID is carried inline or as a reference.

use super::descriptor_body;
use crate::error::{Error, Result};
use dvb_common::{Parse, Serialize};

/// Descriptor tag for content_identifier_descriptor.
pub const TAG: u8 = 0x76;
const HEADER_LEN: usize = 2;
const CRID_TYPE_MASK: u8 = 0xFC;
const CRID_LOCATION_MASK: u8 = 0x03;

/// CRID type — ETSI TS 102 323 Table 117.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub enum CridType {
    /// 0x00 — no type defined.
    NoTypeDefined,
    /// 0x01 — CRID references the item of content that this event is an
    /// instance of.
    ItemOfContent,
    /// 0x02 — CRID references a series that this event belongs to.
    Series,
    /// 0x03 — CRID references a recommendation.
    Recommendation,
    /// Reserved/unallocated wire value, preserved verbatim for round-trip.
    Reserved(u8),
}

impl CridType {
    #[must_use]
    /// Creates a value from a wire byte, preserving every possible
    /// byte value for lossless round-trip.
    pub fn from_u8(v: u8) -> Self {
        match v {
            0x00 => Self::NoTypeDefined,
            0x01 => Self::ItemOfContent,
            0x02 => Self::Series,
            0x03 => Self::Recommendation,
            v => Self::Reserved(v),
        }
    }

    #[must_use]
    /// Returns the wire byte for this value.
    pub fn to_u8(self) -> u8 {
        match self {
            Self::NoTypeDefined => 0x00,
            Self::ItemOfContent => 0x01,
            Self::Series => 0x02,
            Self::Recommendation => 0x03,
            Self::Reserved(v) => v,
        }
    }

    #[must_use]
    /// Returns a human-readable spec name for this value.
    pub fn name(self) -> &'static str {
        match self {
            Self::NoTypeDefined => "no type defined",
            Self::ItemOfContent => "item of content",
            Self::Series => "series",
            Self::Recommendation => "recommendation",
            Self::Reserved(_) => "reserved",
        }
    }
}

/// CRID location per TS 102 323 Table 10.
///
/// Only the two defined locations are representable. Locations `0b10`/`0b11`
/// are reserved with no defined payload length, so the parser rejects them
/// (it cannot know how many bytes the entry occupies) rather than producing
/// an un-round-trippable value.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub enum CridLocation<'a> {
    /// Location 0b00 — CRID carried inline as raw ASCII bytes.
    Inline(&'a [u8]),
    /// Location 0b01 — CRID reference (CIT index).
    Reference(u16),
}

/// One CRID entry within a Content Identifier Descriptor.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[cfg_attr(feature = "yoke", derive(yoke::Yokeable))]
pub struct CridEntry<'a> {
    /// crid_type — identifies what the entry references (TS 102 323 Table 117).
    pub crid_type: CridType,
    /// crid_location and its payload.
    pub location: CridLocation<'a>,
}

/// Content Identifier Descriptor.
///
/// Holds a sequence of CRID entries that identify programme content
/// for recording, scheduling, or recommendation purposes.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[cfg_attr(feature = "yoke", derive(yoke::Yokeable))]
pub struct ContentIdentifierDescriptor<'a> {
    /// Entries in wire order.
    pub entries: Vec<CridEntry<'a>>,
}

impl<'a> Parse<'a> for ContentIdentifierDescriptor<'a> {
    type Error = crate::error::Error;
    fn parse(bytes: &'a [u8]) -> Result<Self> {
        let body = descriptor_body(
            bytes,
            TAG,
            "ContentIdentifierDescriptor",
            "unexpected tag for ContentIdentifierDescriptor",
        )?;
        if body.is_empty() {
            return Ok(Self {
                entries: Vec::new(),
            });
        }
        let mut entries = Vec::new();
        let mut pos = 0;
        while pos < body.len() {
            let header_byte = body[pos];
            pos += 1;
            let crid_type = CridType::from_u8((header_byte & CRID_TYPE_MASK) >> 2);
            let crid_location = header_byte & CRID_LOCATION_MASK;
            let location = match crid_location {
                0x00 => {
                    if pos >= body.len() {
                        return Err(Error::InvalidDescriptor {
                            tag: TAG,
                            reason: "inline CRID length byte missing",
                        });
                    }
                    let crid_length = body[pos] as usize;
                    pos += 1;
                    if pos + crid_length > body.len() {
                        return Err(Error::InvalidDescriptor {
                            tag: TAG,
                            reason: "inline CRID length exceeds descriptor body",
                        });
                    }
                    let crid_bytes = &body[pos..pos + crid_length];
                    pos += crid_length;
                    CridLocation::Inline(crid_bytes)
                }
                0x01 => {
                    if pos + 2 > body.len() {
                        return Err(Error::InvalidDescriptor {
                            tag: TAG,
                            reason: "CRID reference truncated",
                        });
                    }
                    let crid_ref = u16::from_be_bytes([body[pos], body[pos + 1]]);
                    pos += 2;
                    CridLocation::Reference(crid_ref)
                }
                _ => {
                    return Err(Error::InvalidDescriptor {
                        tag: TAG,
                        reason: "reserved crid_location value",
                    });
                }
            };
            entries.push(CridEntry {
                crid_type,
                location,
            });
        }
        Ok(Self { entries })
    }
}

impl Serialize for ContentIdentifierDescriptor<'_> {
    type Error = crate::error::Error;
    fn serialized_len(&self) -> usize {
        let body_len: usize = self
            .entries
            .iter()
            .map(|e| match &e.location {
                CridLocation::Inline(data) => 2 + data.len(),
                CridLocation::Reference(_) => 3,
            })
            .sum();
        HEADER_LEN + body_len
    }

    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        let len = self.serialized_len();
        if buf.len() < len {
            return Err(Error::OutputBufferTooSmall {
                need: len,
                have: buf.len(),
            });
        }
        buf[0] = TAG;
        buf[1] = (len - HEADER_LEN) as u8;
        let mut pos = HEADER_LEN;
        for entry in &self.entries {
            let header = (entry.crid_type.to_u8() << 2) & CRID_TYPE_MASK;
            match &entry.location {
                CridLocation::Inline(data) => {
                    buf[pos] = header;
                    buf[pos + 1] = data.len() as u8;
                    buf[pos + 2..pos + 2 + data.len()].copy_from_slice(data);
                    pos += 2 + data.len();
                }
                CridLocation::Reference(val) => {
                    buf[pos] = header | 0x01;
                    let bytes = val.to_be_bytes();
                    buf[pos + 1] = bytes[0];
                    buf[pos + 2] = bytes[1];
                    pos += 3;
                }
            }
        }
        Ok(len)
    }
}
impl<'a> crate::traits::DescriptorDef<'a> for ContentIdentifierDescriptor<'a> {
    const TAG: u8 = TAG;
    const NAME: &'static str = "CONTENT_IDENTIFIER";
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_single_inline_crid() {
        let data = b"DVB/CRID/EPG123";
        let mut buf = vec![TAG, (data.len() + 2) as u8, 0x01 << 2, data.len() as u8];
        buf.extend_from_slice(data);
        let d = ContentIdentifierDescriptor::parse(&buf).unwrap();
        assert_eq!(d.entries.len(), 1);
        assert_eq!(d.entries[0].crid_type, CridType::ItemOfContent);
        match &d.entries[0].location {
            CridLocation::Inline(bytes) => assert_eq!(*bytes, data.as_slice()),
            _ => panic!("expected Inline"),
        }
    }

    #[test]
    fn parse_single_reference_crid() {
        let buf = [TAG, 0x03, (0x02 << 2) | 0x01, 0x00, 0x42];
        let d = ContentIdentifierDescriptor::parse(&buf).unwrap();
        assert_eq!(d.entries.len(), 1);
        assert_eq!(d.entries[0].crid_type, CridType::Series);
        match d.entries[0].location {
            CridLocation::Reference(val) => assert_eq!(val, 0x0042),
            _ => panic!("expected Reference"),
        }
    }

    #[test]
    fn parse_multiple_entries() {
        let inline_data = b"EPG/EPG123";
        let ref_val: u16 = 0x0100;
        let mut buf = vec![TAG, 0x00, 0x01 << 2, inline_data.len() as u8];
        buf.extend_from_slice(inline_data);
        buf.push((0x03 << 2) | 0x01);
        buf.extend_from_slice(&ref_val.to_be_bytes());
        let body_len = buf.len() - HEADER_LEN;
        buf[1] = body_len as u8;

        let d = ContentIdentifierDescriptor::parse(&buf).unwrap();
        assert_eq!(d.entries.len(), 2);
        assert_eq!(d.entries[0].crid_type, CridType::ItemOfContent);
        match &d.entries[0].location {
            CridLocation::Inline(bytes) => assert_eq!(*bytes, inline_data.as_slice()),
            _ => panic!("expected Inline for first entry"),
        }
        assert_eq!(d.entries[1].crid_type, CridType::Recommendation);
        match d.entries[1].location {
            CridLocation::Reference(val) => assert_eq!(val, ref_val),
            _ => panic!("expected Reference for second entry"),
        }
    }

    #[test]
    fn parse_rejects_wrong_tag() {
        let buf = [0x7A, 0x03, 0x04, 0x00, 0x42];
        assert!(matches!(
            ContentIdentifierDescriptor::parse(&buf).unwrap_err(),
            Error::InvalidDescriptor { tag: 0x7A, .. }
        ));
    }

    #[test]
    fn parse_rejects_inline_length_overrun() {
        let buf = [TAG, 4, 0x01 << 2, 10, 0xAA, 0xBB];
        assert!(matches!(
            ContentIdentifierDescriptor::parse(&buf).unwrap_err(),
            Error::InvalidDescriptor { tag: TAG, .. }
        ));
    }

    #[test]
    fn parse_rejects_reference_truncated() {
        let buf = [TAG, 2, (0x02 << 2) | 0x01, 0xAA];
        assert!(matches!(
            ContentIdentifierDescriptor::parse(&buf).unwrap_err(),
            Error::InvalidDescriptor { tag: TAG, .. }
        ));
    }

    #[test]
    fn parse_rejects_reserved_location() {
        let buf = [TAG, 0x01, (0x01 << 2) | 0x02];
        assert!(matches!(
            ContentIdentifierDescriptor::parse(&buf).unwrap_err(),
            Error::InvalidDescriptor { tag: TAG, .. }
        ));
        let buf = [TAG, 0x01, (0x01 << 2) | 0x03];
        assert!(matches!(
            ContentIdentifierDescriptor::parse(&buf).unwrap_err(),
            Error::InvalidDescriptor { tag: TAG, .. }
        ));
    }

    #[test]
    fn empty_descriptor_valid() {
        let buf = [TAG, 0x00];
        let d = ContentIdentifierDescriptor::parse(&buf).unwrap();
        assert_eq!(d.entries.len(), 0);
    }

    #[test]
    fn serialize_round_trip_inline_and_reference() {
        let inline_data = b"DVB/CRID/TEST456";
        let ref_val: u16 = 789;
        let desc = ContentIdentifierDescriptor {
            entries: vec![
                CridEntry {
                    crid_type: CridType::ItemOfContent,
                    location: CridLocation::Inline(inline_data.as_slice()),
                },
                CridEntry {
                    crid_type: CridType::Recommendation,
                    location: CridLocation::Reference(ref_val),
                },
            ],
        };
        let mut buf = vec![0u8; desc.serialized_len()];
        desc.serialize_into(&mut buf).unwrap();
        let parsed = ContentIdentifierDescriptor::parse(&buf).unwrap();
        assert_eq!(parsed.entries.len(), desc.entries.len());
        match &parsed.entries[0].location {
            CridLocation::Inline(bytes) => assert_eq!(*bytes, inline_data.as_slice()),
            _ => panic!("expected Inline"),
        }
        assert_eq!(parsed.entries[0].crid_type, CridType::ItemOfContent);
        match parsed.entries[1].location {
            CridLocation::Reference(val) => assert_eq!(val, ref_val),
            _ => panic!("expected Reference"),
        }
        assert_eq!(parsed.entries[1].crid_type, CridType::Recommendation);
    }

    #[test]
    fn crid_type_full_range_round_trip() {
        for b in 0..=0xFF_u8 {
            let ct = CridType::from_u8(b);
            assert_eq!(ct.to_u8(), b, "round-trip failed for byte 0x{b:02X}");
        }
    }

    #[test]
    fn crid_type_name_for_known() {
        assert_eq!(CridType::NoTypeDefined.name(), "no type defined");
        assert_eq!(CridType::ItemOfContent.name(), "item of content");
        assert_eq!(CridType::Series.name(), "series");
        assert_eq!(CridType::Recommendation.name(), "recommendation");
        assert_eq!(CridType::Reserved(0x55).name(), "reserved");
    }
}
