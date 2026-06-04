//! VBI Data Descriptor — ETSI EN 300 468 §6.2.47 (tag 0x45).
//!
//! Table 106 (PDF p. 110). Carried in PMT ES_info for an elementary stream
//! carrying VBI data (per ETSI EN 301 775). Body is a loop of entries, each a
//! `data_service_id` byte + an 8-bit `data_service_descriptor_length` + that
//! many service-descriptor bytes.
//!
//! The first loop level (data_service_id + length-delimited service block) is
//! typed. The inner per-line content (each byte either
//! `reserved(2)|field_parity(1)|line_offset(5)` for data_service_id ∈
//! {0x01,0x02,0x04,0x05,0x06,0x07}, or 8 reserved bits otherwise) is kept raw
//! as a borrowed `&[u8]` per house convention — its meaning is selected by
//! `data_service_id`, so per-byte typing would be fragile (Table 106/107).

use crate::error::{Error, Result};
use crate::traits::Descriptor;
use dvb_common::{Parse, Serialize};

/// Descriptor tag for VBI_data_descriptor.
pub const TAG: u8 = 0x45;
const HEADER_LEN: usize = 2;
const ENTRY_HEADER_LEN: usize = 2; // data_service_id + data_service_descriptor_length
/// Maximum body length expressible in the 8-bit `descriptor_length` field.
const MAX_BODY_LEN: usize = u8::MAX as usize;
/// Maximum per-entry service block length (8-bit length field).
const MAX_SERVICE_LEN: usize = u8::MAX as usize;

/// One VBI data service entry.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "serde", serde(bound(deserialize = "'de: 'a")))]
pub struct VbiDataEntry<'a> {
    /// data_service_id (EN 300 468 Table 107): 0x01 = EBU teletext,
    /// 0x02 = inverted teletext, 0x04 = VPS, 0x05 = WSS, 0x06 = closed
    /// captioning, 0x07 = monochrome 4:2:2 samples; others reserved/user.
    pub data_service_id: u8,
    /// Raw service-descriptor bytes (one byte per VBI line; layout selected by
    /// `data_service_id` per Table 106). Kept opaque per house convention.
    #[cfg_attr(feature = "serde", serde(borrow))]
    pub service_descriptor: &'a [u8],
}

/// VBI Data Descriptor.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "serde", serde(bound(deserialize = "'de: 'a")))]
pub struct VbiDataDescriptor<'a> {
    /// Service entries in wire order.
    #[cfg_attr(feature = "serde", serde(borrow))]
    pub entries: Vec<VbiDataEntry<'a>>,
}

impl<'a> Parse<'a> for VbiDataDescriptor<'a> {
    type Error = crate::error::Error;
    fn parse(bytes: &'a [u8]) -> Result<Self> {
        if bytes.len() < HEADER_LEN {
            return Err(Error::BufferTooShort {
                need: HEADER_LEN,
                have: bytes.len(),
                what: "VbiDataDescriptor header",
            });
        }
        if bytes[0] != TAG {
            return Err(Error::InvalidDescriptor {
                tag: bytes[0],
                reason: "unexpected tag for VBI_data_descriptor",
            });
        }
        let length = bytes[1] as usize;
        let end = HEADER_LEN + length;
        if bytes.len() < end {
            return Err(Error::BufferTooShort {
                need: end,
                have: bytes.len(),
                what: "VbiDataDescriptor body",
            });
        }
        let body = &bytes[HEADER_LEN..end];
        let mut entries = Vec::new();
        let mut pos = 0;
        while pos < body.len() {
            if pos + ENTRY_HEADER_LEN > body.len() {
                return Err(Error::InvalidDescriptor {
                    tag: TAG,
                    reason: "truncated VBI data entry header",
                });
            }
            let data_service_id = body[pos];
            let svc_len = body[pos + 1] as usize;
            pos += ENTRY_HEADER_LEN;
            if pos + svc_len > body.len() {
                return Err(Error::InvalidDescriptor {
                    tag: TAG,
                    reason: "data_service_descriptor_length exceeds descriptor body",
                });
            }
            let service_descriptor = &body[pos..pos + svc_len];
            pos += svc_len;
            entries.push(VbiDataEntry {
                data_service_id,
                service_descriptor,
            });
        }
        Ok(Self { entries })
    }
}

impl Serialize for VbiDataDescriptor<'_> {
    type Error = crate::error::Error;
    fn serialized_len(&self) -> usize {
        HEADER_LEN
            + self
                .entries
                .iter()
                .map(|e| ENTRY_HEADER_LEN + e.service_descriptor.len())
                .sum::<usize>()
    }

    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        let len = self.serialized_len();
        if buf.len() < len {
            return Err(Error::OutputBufferTooSmall {
                need: len,
                have: buf.len(),
            });
        }
        let body_len = len - HEADER_LEN;
        // 8-bit descriptor_length field: error rather than silently truncate.
        if body_len > MAX_BODY_LEN {
            return Err(Error::SectionLengthOverflow {
                declared: body_len,
                available: MAX_BODY_LEN,
            });
        }
        buf[0] = TAG;
        buf[1] = body_len as u8;
        let mut pos = HEADER_LEN;
        for e in &self.entries {
            // 8-bit data_service_descriptor_length field: error on over-range.
            if e.service_descriptor.len() > MAX_SERVICE_LEN {
                return Err(Error::InvalidDescriptor {
                    tag: TAG,
                    reason: "service_descriptor exceeds 255 bytes (8-bit length field)",
                });
            }
            buf[pos] = e.data_service_id;
            buf[pos + 1] = e.service_descriptor.len() as u8;
            pos += ENTRY_HEADER_LEN;
            buf[pos..pos + e.service_descriptor.len()].copy_from_slice(e.service_descriptor);
            pos += e.service_descriptor.len();
        }
        Ok(len)
    }
}

impl<'a> Descriptor<'a> for VbiDataDescriptor<'a> {
    const TAG: u8 = TAG;
    fn descriptor_length(&self) -> u8 {
        (self.serialized_len() - HEADER_LEN) as u8
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_single_entry() {
        // data_service_id=0x01 (EBU teletext), 2 line bytes
        let bytes = [TAG, 4, 0x01, 0x02, 0xC1, 0xC2];
        let d = VbiDataDescriptor::parse(&bytes).unwrap();
        assert_eq!(d.entries.len(), 1);
        assert_eq!(d.entries[0].data_service_id, 0x01);
        assert_eq!(d.entries[0].service_descriptor, &[0xC1, 0xC2]);
    }

    #[test]
    fn parse_multiple_entries() {
        let bytes = [TAG, 7, 0x04, 0x01, 0xAA, 0x05, 0x02, 0xBB, 0xCC];
        let d = VbiDataDescriptor::parse(&bytes).unwrap();
        assert_eq!(d.entries.len(), 2);
        assert_eq!(d.entries[0].data_service_id, 0x04);
        assert_eq!(d.entries[0].service_descriptor, &[0xAA]);
        assert_eq!(d.entries[1].data_service_id, 0x05);
        assert_eq!(d.entries[1].service_descriptor, &[0xBB, 0xCC]);
    }

    #[test]
    fn parse_entry_with_empty_service_block() {
        let bytes = [TAG, 2, 0x06, 0x00];
        let d = VbiDataDescriptor::parse(&bytes).unwrap();
        assert_eq!(d.entries.len(), 1);
        assert!(d.entries[0].service_descriptor.is_empty());
    }

    #[test]
    fn parse_rejects_wrong_tag() {
        assert!(matches!(
            VbiDataDescriptor::parse(&[0x46, 0]).unwrap_err(),
            Error::InvalidDescriptor { tag: 0x46, .. }
        ));
    }

    #[test]
    fn parse_rejects_short_buffer() {
        // declares 4 body bytes, only 2 present
        let bytes = [TAG, 4, 0x01, 0x02];
        assert!(matches!(
            VbiDataDescriptor::parse(&bytes).unwrap_err(),
            Error::BufferTooShort { .. }
        ));
    }

    #[test]
    fn parse_rejects_inner_length_overrun() {
        // entry declares 5 service bytes but only 1 remains in body
        let bytes = [TAG, 3, 0x01, 0x05, 0xAA];
        assert!(matches!(
            VbiDataDescriptor::parse(&bytes).unwrap_err(),
            Error::InvalidDescriptor { tag: TAG, .. }
        ));
    }

    #[test]
    fn empty_descriptor_valid() {
        let d = VbiDataDescriptor::parse(&[TAG, 0]).unwrap();
        assert!(d.entries.is_empty());
    }

    #[test]
    fn serialize_round_trip() {
        let d = VbiDataDescriptor {
            entries: vec![
                VbiDataEntry {
                    data_service_id: 0x01,
                    service_descriptor: &[0xC1, 0xC2, 0xC3],
                },
                VbiDataEntry {
                    data_service_id: 0x04,
                    service_descriptor: &[],
                },
            ],
        };
        let mut buf = vec![0u8; d.serialized_len()];
        d.serialize_into(&mut buf).unwrap();
        assert_eq!(VbiDataDescriptor::parse(&buf).unwrap(), d);
    }

    #[test]
    fn serialize_rejects_small_buffer() {
        let d = VbiDataDescriptor {
            entries: vec![VbiDataEntry {
                data_service_id: 0x01,
                service_descriptor: &[0xAA],
            }],
        };
        let mut tiny = [0u8; 3];
        assert!(matches!(
            d.serialize_into(&mut tiny).unwrap_err(),
            Error::OutputBufferTooSmall { .. }
        ));
    }

    #[cfg(feature = "serde")]
    #[test]
    fn serde_serialize_stable() {
        // Borrowed-byte fields cannot deserialize from a JSON array (serde_json
        // requires a borrowed-str for &[u8]); assert the Serialize half is
        // stable, matching the other borrowed descriptors (e.g.
        // content_identifier) in this crate.
        let make = || VbiDataDescriptor {
            entries: vec![VbiDataEntry {
                data_service_id: 0x01,
                service_descriptor: &[0xC1, 0xC2],
            }],
        };
        let json = serde_json::to_string(&make()).unwrap();
        assert!(json.contains("data_service_id"));
        assert_eq!(json, serde_json::to_string(&make()).unwrap());
    }
}
