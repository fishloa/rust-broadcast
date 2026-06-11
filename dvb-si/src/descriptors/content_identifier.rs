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
    /// crid_type byte — identifies what the entry references.
    /// Per TS 102 323 Table 8: 0x01 = episode, 0x02 = series,
    /// 0x03 = recommendation, 0x31..=0x3F = user-defined.
    pub crid_type: u8,
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
            let crid_type = (header_byte & CRID_TYPE_MASK) >> 2;
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
                    // big-endian u16 per spec
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
            let header = (entry.crid_type << 2) & CRID_TYPE_MASK;
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
        // fill any remaining buffer that might be passed in
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
        assert_eq!(d.entries[0].crid_type, 0x01);
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
        assert_eq!(d.entries[0].crid_type, 0x02);
        match d.entries[0].location {
            CridLocation::Reference(val) => assert_eq!(val, 0x0042),
            _ => panic!("expected Reference"),
        }
    }

    #[test]
    fn parse_multiple_entries() {
        let inline_data = b"EPG/EPG123";
        let ref_val: u16 = 0x0100;
        // First entry: inline CRID (type=0x01, location=0x00)
        let mut buf = vec![
            TAG,
            0x00, // placeholder for length
            0x01 << 2,
            inline_data.len() as u8,
        ];
        buf.extend_from_slice(inline_data);
        // Second entry: reference CRID (type=0x03, location=0x01)
        buf.push((0x03 << 2) | 0x01);
        buf.extend_from_slice(&ref_val.to_be_bytes());
        // Patch descriptor_length
        let body_len = buf.len() - HEADER_LEN;
        buf[1] = body_len as u8;

        let d = ContentIdentifierDescriptor::parse(&buf).unwrap();
        assert_eq!(d.entries.len(), 2);
        // Verify first entry
        assert_eq!(d.entries[0].crid_type, 0x01);
        match &d.entries[0].location {
            CridLocation::Inline(bytes) => assert_eq!(*bytes, inline_data.as_slice()),
            _ => panic!("expected Inline for first entry"),
        }
        // Verify second entry
        assert_eq!(d.entries[1].crid_type, 0x03);
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
        // Full descriptor: length=4 body = [header, crid_len=10, 0xAA, 0xBB].
        // Inline crid_length=10 exceeds the 2 remaining body bytes.
        let buf = [TAG, 4, 0x01 << 2, 10, 0xAA, 0xBB];
        assert!(matches!(
            ContentIdentifierDescriptor::parse(&buf).unwrap_err(),
            Error::InvalidDescriptor { tag: TAG, .. }
        ));
    }

    #[test]
    fn parse_rejects_reference_truncated() {
        // Full descriptor: length=2 body = [header, 0xAA]. Reference location
        // needs 2 bytes but only 1 remains.
        let buf = [TAG, 2, (0x02 << 2) | 0x01, 0xAA];
        assert!(matches!(
            ContentIdentifierDescriptor::parse(&buf).unwrap_err(),
            Error::InvalidDescriptor { tag: TAG, .. }
        ));
    }

    #[test]
    fn parse_rejects_reserved_location() {
        // location=0x02 is reserved
        let buf = [TAG, 0x01, (0x01 << 2) | 0x02];
        assert!(matches!(
            ContentIdentifierDescriptor::parse(&buf).unwrap_err(),
            Error::InvalidDescriptor { tag: TAG, .. }
        ));
        // location=0x03 is also reserved
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
                    crid_type: 0x01,
                    location: CridLocation::Inline(inline_data.as_slice()),
                },
                CridEntry {
                    crid_type: 0x03,
                    location: CridLocation::Reference(ref_val),
                },
            ],
        };
        let mut buf = vec![0u8; desc.serialized_len()];
        desc.serialize_into(&mut buf).unwrap();
        let parsed = ContentIdentifierDescriptor::parse(&buf).unwrap();
        assert_eq!(parsed.entries.len(), desc.entries.len());
        // Check inline entry
        match &parsed.entries[0].location {
            CridLocation::Inline(bytes) => assert_eq!(*bytes, inline_data.as_slice()),
            _ => panic!("expected Inline"),
        }
        assert_eq!(parsed.entries[0].crid_type, 0x01);
        // Check reference entry
        match parsed.entries[1].location {
            CridLocation::Reference(val) => assert_eq!(val, ref_val),
            _ => panic!("expected Reference"),
        }
        assert_eq!(parsed.entries[1].crid_type, 0x03);
    }
}
