//! Content Labeling Descriptor — ISO/IEC 13818-1 §2.6.56, Table 2-83 (tag 0x24).
//!
//! Associates metadata labelling with content, supporting time-base schemes
//! (STC, NPT, or privately defined) and optional content-reference records.
//! The time-base block is conditional on `content_time_base_indicator` 1 or 2;
//! the contentId block only when value 2; the time_base_association block
//! only when 3..=7.

use super::descriptor_body;
use crate::descriptors::content_time_base_indicator::ContentTimeBaseIndicator;
use crate::error::{Error, Result};
use dvb_common::{Parse, Serialize};

/// Descriptor tag for content_labeling_descriptor.
pub const TAG: u8 = 0x24;
const HEADER_LEN: usize = 2;

/// Time-base block — present when content_time_base_indicator is 1 or 2.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct ContentTimeBase {
    /// 33-bit content time base value (stored as u64, masked to 33 bits).
    pub content_time_base_value: u64,
    /// 33-bit metadata time base value (stored as u64, masked to 33 bits).
    pub metadata_time_base_value: u64,
}

/// Time-base association block — present when content_time_base_indicator is 3..=7.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct ContentTimeBaseAssociation<'a> {
    /// Opaque time_base_association_data bytes.
    #[cfg_attr(feature = "serde", serde(borrow))]
    pub data: &'a [u8],
}

/// Content Labeling Descriptor.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[cfg_attr(feature = "yoke", derive(yoke::Yokeable))]
pub struct ContentLabelingDescriptor<'a> {
    /// Metadata application format (u16; see Table 2-84).
    pub metadata_application_format: u16,
    /// Metadata application format identifier — present when `metadata_application_format == 0xFFFF`.
    pub metadata_application_format_identifier: Option<u32>,
    /// Content reference ID record present flag.
    pub content_reference_id_record_flag: bool,
    /// Time base indicator (4-bit; Table 2-85).
    pub content_time_base_indicator: ContentTimeBaseIndicator,
    /// Content reference ID record — present when `content_reference_id_record_flag` is true.
    #[cfg_attr(feature = "serde", serde(borrow))]
    pub content_reference_id_record: Option<&'a [u8]>,
    /// Time base block — present when indicator is 1 or 2.
    pub time_base: Option<ContentTimeBase>,
    /// contentId (7-bit) — present when indicator is 2.
    pub content_id: Option<u8>,
    /// Time base association block — present when indicator is 3..=7.
    pub time_base_association: Option<ContentTimeBaseAssociation<'a>>,
    /// Trailing private data bytes.
    #[cfg_attr(feature = "serde", serde(borrow))]
    pub private_data: &'a [u8],
}

fn read_33bit(bytes: &[u8], pos: usize) -> u64 {
    // 33 bits = 4 bytes + 1 bit, laid out across 5 bytes: byte[0..3] full,
    // byte[4] top bit is the MSB, bottom 7 bits reserved.
    ((bytes[pos] as u64) << 25)
        | ((bytes[pos + 1] as u64) << 17)
        | ((bytes[pos + 2] as u64) << 9)
        | ((bytes[pos + 3] as u64) << 1)
        | ((bytes[pos + 4] >> 7) as u64)
}

fn write_33bit(buf: &mut [u8], pos: usize, value: u64) {
    let v = value & 0x1_FFFF_FFFF;
    buf[pos] = ((v >> 25) & 0xFF) as u8;
    buf[pos + 1] = ((v >> 17) & 0xFF) as u8;
    buf[pos + 2] = ((v >> 9) & 0xFF) as u8;
    buf[pos + 3] = ((v >> 1) & 0xFF) as u8;
    buf[pos + 4] = ((v & 0x01) as u8) << 7;
}

impl<'a> Parse<'a> for ContentLabelingDescriptor<'a> {
    type Error = crate::error::Error;

    fn parse(bytes: &'a [u8]) -> Result<Self> {
        let body = descriptor_body(
            bytes,
            TAG,
            "ContentLabelingDescriptor",
            "unexpected tag for content_labeling_descriptor",
        )?;

        // Minimum: metadata_application_format(2) + flags(1) = 3 bytes
        if body.len() < 3 {
            return Err(Error::InvalidDescriptor {
                tag: TAG,
                reason: "content_labeling_descriptor too short (< 3 body bytes)",
            });
        }

        let metadata_application_format = u16::from_be_bytes([body[0], body[1]]);
        let mut pos = 2;

        let metadata_application_format_identifier = if metadata_application_format == 0xFFFF {
            if body.len() < pos + 4 {
                return Err(Error::InvalidDescriptor {
                        tag: TAG,
                        reason: "content_labeling_descriptor too short for metadata_application_format_identifier",
                    });
            }
            let id = u32::from_be_bytes([body[pos], body[pos + 1], body[pos + 2], body[pos + 3]]);
            pos += 4;
            Some(id)
        } else {
            None
        };

        if body.len() < pos + 1 {
            return Err(Error::InvalidDescriptor {
                tag: TAG,
                reason: "content_labeling_descriptor too short for flags byte",
            });
        }
        let flags = body[pos];
        let content_reference_id_record_flag = (flags & 0x80) != 0;
        let indicator_raw = (flags >> 3) & 0x0F;
        let content_time_base_indicator = ContentTimeBaseIndicator::from_u8(indicator_raw);
        pos += 1;

        let (content_reference_id_record, _) = if content_reference_id_record_flag {
            if body.len() < pos + 1 {
                return Err(Error::InvalidDescriptor {
                        tag: TAG,
                        reason: "content_labeling_descriptor too short for content_reference_id_record_length",
                    });
            }
            let rec_len = body[pos] as usize;
            pos += 1;
            if body.len() < pos + rec_len {
                return Err(Error::InvalidDescriptor {
                    tag: TAG,
                    reason: "content_labeling_descriptor too short for content_reference_id_record",
                });
            }
            let rec = &body[pos..pos + rec_len];
            pos += rec_len;
            (Some(rec), pos)
        } else {
            (None, pos)
        };

        let time_base = if indicator_raw == 1 || indicator_raw == 2 {
            // 7 reserved bits + 33-bit content_time_base_value + 7 reserved bits + 33-bit metadata_time_base_value
            // = 80 bits = 10 bytes
            if body.len() < pos + 10 {
                return Err(Error::InvalidDescriptor {
                    tag: TAG,
                    reason: "content_labeling_descriptor too short for time base block",
                });
            }
            // byte[pos]: reserved(7) | content_time_base_value top bit
            // That's: 1 reserved byte + 4.125 bytes for the 33-bit value
            // The 33-bit value spans bytes pos..pos+5, with byte[pos] having 7 reserved + 1 data bit
            // Wait — the syntax table shows:
            //   reserved(7) | content_time_base_value(33)
            // The reserved(7) occupies the first 7 bits of byte pos, then content_time_base_value starts at bit 0 of byte pos
            // Actually: 7 reserved + 33 = 40 bits = 5 bytes
            // Then: reserved(7) | metadata_time_base_value(33) = 40 bits = 5 bytes
            // Total = 10 bytes
            let ctv = read_33bit(&body[pos..], 0);
            let mtv = read_33bit(&body[pos + 5..], 0);
            pos += 10;
            Some(ContentTimeBase {
                content_time_base_value: ctv,
                metadata_time_base_value: mtv,
            })
        } else {
            None
        };

        let content_id = if indicator_raw == 2 {
            if body.len() < pos + 1 {
                return Err(Error::InvalidDescriptor {
                    tag: TAG,
                    reason: "content_labeling_descriptor too short for contentId byte",
                });
            }
            let id = body[pos] & 0x7F;
            pos += 1;
            Some(id)
        } else {
            None
        };

        let time_base_association = if (3..=7).contains(&indicator_raw) {
            if body.len() < pos + 1 {
                return Err(Error::InvalidDescriptor {
                    tag: TAG,
                    reason: "content_labeling_descriptor too short for time_base_association_data_length",
                });
            }
            let assoc_len = body[pos] as usize;
            pos += 1;
            if body.len() < pos + assoc_len {
                return Err(Error::InvalidDescriptor {
                    tag: TAG,
                    reason: "content_labeling_descriptor too short for time_base_association_data",
                });
            }
            let data = &body[pos..pos + assoc_len];
            pos += assoc_len;
            Some(ContentTimeBaseAssociation { data })
        } else {
            None
        };

        let private_data = &body[pos..];

        Ok(Self {
            metadata_application_format,
            metadata_application_format_identifier,
            content_reference_id_record_flag,
            content_time_base_indicator,
            content_reference_id_record,
            time_base,
            content_id,
            time_base_association,
            private_data,
        })
    }
}

impl Serialize for ContentLabelingDescriptor<'_> {
    type Error = crate::error::Error;

    fn serialized_len(&self) -> usize {
        let mut len: usize = HEADER_LEN + 3; // metadata_application_format(2) + flags(1)
        if self.metadata_application_format_identifier.is_some() {
            len += 4;
        }
        if let Some(rec) = self.content_reference_id_record {
            len += 1 + rec.len();
        }
        if self.time_base.is_some() {
            len += 10;
        }
        if self.content_id.is_some() {
            len += 1;
        }
        if let Some(ref assoc) = self.time_base_association {
            len += 1 + assoc.data.len();
        }
        len += self.private_data.len();
        len
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

        buf[HEADER_LEN] = (self.metadata_application_format >> 8) as u8;
        buf[HEADER_LEN + 1] = self.metadata_application_format as u8;
        let mut pos = HEADER_LEN + 2;

        if let Some(id) = self.metadata_application_format_identifier {
            buf[pos..pos + 4].copy_from_slice(&id.to_be_bytes());
            pos += 4;
        }

        let mut flags = (self.content_time_base_indicator.to_u8() & 0x0F) << 3;
        if self.content_reference_id_record_flag {
            flags |= 0x80;
        }
        buf[pos] = flags;
        pos += 1;

        if let Some(rec) = self.content_reference_id_record {
            buf[pos] = rec.len() as u8;
            pos += 1;
            buf[pos..pos + rec.len()].copy_from_slice(rec);
            pos += rec.len();
        }

        if let Some(ref tb) = self.time_base {
            // 7 reserved bits (0) + 33-bit content_time_base_value
            write_33bit(buf, pos, tb.content_time_base_value);
            pos += 5;
            // 7 reserved bits (0) + 33-bit metadata_time_base_value
            write_33bit(buf, pos, tb.metadata_time_base_value);
            pos += 5;
        }

        if let Some(cid) = self.content_id {
            buf[pos] = cid & 0x7F;
            pos += 1;
        }

        if let Some(ref assoc) = self.time_base_association {
            buf[pos] = assoc.data.len() as u8;
            pos += 1;
            buf[pos..pos + assoc.data.len()].copy_from_slice(assoc.data);
            pos += assoc.data.len();
        }

        buf[pos..pos + self.private_data.len()].copy_from_slice(self.private_data);
        Ok(len)
    }
}

impl<'a> crate::traits::DescriptorDef<'a> for ContentLabelingDescriptor<'a> {
    const TAG: u8 = TAG;
    const NAME: &'static str = "CONTENT_LABELING";
}

#[cfg(test)]
mod tests {
    use super::*;

    // Helper to build a descriptor with all needed bytes
    fn serialize_round_trip(d: &ContentLabelingDescriptor<'_>) {
        let mut buf = vec![0u8; d.serialized_len()];
        let written = d.serialize_into(&mut buf).unwrap();
        assert_eq!(written, d.serialized_len());
        let reparsed = ContentLabelingDescriptor::parse(&buf).unwrap();
        assert_eq!(*d, reparsed, "round-trip mismatch");
    }

    #[test]
    fn round_trip_indicator_0_minimal() {
        let d = ContentLabelingDescriptor {
            metadata_application_format: 0x0010,
            metadata_application_format_identifier: None,
            content_reference_id_record_flag: false,
            content_time_base_indicator: ContentTimeBaseIndicator::None,
            content_reference_id_record: None,
            time_base: None,
            content_id: None,
            time_base_association: None,
            private_data: &[],
        };
        serialize_round_trip(&d);
    }

    #[test]
    fn round_trip_indicator_0_with_ref_id_and_private() {
        let d = ContentLabelingDescriptor {
            metadata_application_format: 0x0011,
            metadata_application_format_identifier: None,
            content_reference_id_record_flag: true,
            content_time_base_indicator: ContentTimeBaseIndicator::None,
            content_reference_id_record: Some(&[0xAA, 0xBB, 0xCC]),
            time_base: None,
            content_id: None,
            time_base_association: None,
            private_data: &[0xDD, 0xEE],
        };
        serialize_round_trip(&d);
    }

    #[test]
    fn round_trip_indicator_1_stc_time_base() {
        let d = ContentLabelingDescriptor {
            metadata_application_format: 0x0100,
            metadata_application_format_identifier: None,
            content_reference_id_record_flag: false,
            content_time_base_indicator: ContentTimeBaseIndicator::Stc,
            content_reference_id_record: None,
            time_base: Some(ContentTimeBase {
                content_time_base_value: 0x123456789,
                metadata_time_base_value: 0x1ABCDEF01,
            }),
            content_id: None,
            time_base_association: None,
            private_data: &[],
        };
        serialize_round_trip(&d);
    }

    #[test]
    fn round_trip_indicator_2_npt_with_content_id() {
        let d = ContentLabelingDescriptor {
            metadata_application_format: 0x0100,
            metadata_application_format_identifier: None,
            content_reference_id_record_flag: false,
            content_time_base_indicator: ContentTimeBaseIndicator::Npt,
            content_reference_id_record: None,
            time_base: Some(ContentTimeBase {
                content_time_base_value: 0x1AABBCCDD,
                metadata_time_base_value: 0x0,
            }),
            content_id: Some(42),
            time_base_association: None,
            private_data: &[],
        };
        serialize_round_trip(&d);
    }

    #[test]
    fn round_trip_indicator_3_association() {
        let d = ContentLabelingDescriptor {
            metadata_application_format: 0x0100,
            metadata_application_format_identifier: None,
            content_reference_id_record_flag: false,
            content_time_base_indicator: ContentTimeBaseIndicator::Reserved(3),
            content_reference_id_record: None,
            time_base: None,
            content_id: None,
            time_base_association: Some(ContentTimeBaseAssociation {
                data: &[0x11, 0x22, 0x33, 0x44],
            }),
            private_data: &[],
        };
        serialize_round_trip(&d);
    }

    #[test]
    fn round_trip_with_ffff_identifier() {
        let d = ContentLabelingDescriptor {
            metadata_application_format: 0xFFFF,
            metadata_application_format_identifier: Some(0xDEADBEEF),
            content_reference_id_record_flag: true,
            content_time_base_indicator: ContentTimeBaseIndicator::None,
            content_reference_id_record: Some(&[0x01, 0x02]),
            time_base: None,
            content_id: None,
            time_base_association: None,
            private_data: &[0xFF],
        };
        serialize_round_trip(&d);
    }

    #[test]
    fn round_trip_indicator_private_8() {
        // 8..=15 — privately defined, no blocks
        let d = ContentLabelingDescriptor {
            metadata_application_format: 0x0101,
            metadata_application_format_identifier: None,
            content_reference_id_record_flag: false,
            content_time_base_indicator: ContentTimeBaseIndicator::Private(10),
            content_reference_id_record: None,
            time_base: None,
            content_id: None,
            time_base_association: None,
            private_data: &[0x99],
        };
        serialize_round_trip(&d);
    }

    #[test]
    fn parse_rejects_wrong_tag() {
        let err = ContentLabelingDescriptor::parse(&[0x02, 3, 0, 0, 0]).unwrap_err();
        assert!(matches!(err, Error::InvalidDescriptor { tag: 0x02, .. }));
    }

    #[test]
    fn parse_rejects_too_short() {
        let err = ContentLabelingDescriptor::parse(&[TAG, 0]).unwrap_err();
        assert!(matches!(err, Error::InvalidDescriptor { tag: TAG, .. }));
    }
}
