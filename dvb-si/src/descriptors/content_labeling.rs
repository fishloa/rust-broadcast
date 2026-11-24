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
use broadcast_common::len::need_out;
use broadcast_common::reserved::ReservedOnes;
use broadcast_common::{Parse, Serialize};

/// Descriptor tag for content_labeling_descriptor.
pub const TAG: u8 = 0x24;
const HEADER_LEN: usize = 2;
/// Default of the 3-bit `reserved` after `content_time_base_indicator`, of
/// each 7-bit `reserved` ahead of a 33-bit time-base value and of the 1-bit
/// `reserved` ahead of `contentId` (H.222.0 Table 2-83): clause 2.1
/// "reserved" = `1` (all editions 2006-08/2023, audit §5). The
/// `time_base_association_data` bytes are `reserved(8)` per entry and are
/// carried verbatim in [`ContentTimeBaseAssociation::data`].
pub const RESERVED_3_DEFAULT: ReservedOnes<RESERVED_AFTER_TIME_BASE_INDICATOR_BITS> =
    ReservedOnes::<RESERVED_AFTER_TIME_BASE_INDICATOR_BITS>::FILL;
/// Width of the 3-bit `reserved` after `content_time_base_indicator`.
pub const RESERVED_AFTER_TIME_BASE_INDICATOR_BITS: u32 = 3;
/// Width of the 7-bit `reserved` ahead of each 33-bit time-base value.
pub const RESERVED_BEFORE_TIME_BASE_BITS: u32 = 7;
/// Width of the 1-bit `reserved` ahead of `contentId`.
pub const RESERVED_BEFORE_CONTENT_ID_BITS: u32 = 1;
/// Width of `metadata_application_format`, bytes.
const FORMAT_LEN: usize = core::mem::size_of::<u16>();
/// Width of the flags byte, bytes.
const FLAGS_LEN: usize = 1;
/// Width of `metadata_application_format` + the flags byte, bytes.
const HEADER_PREFIX_LEN: usize = FORMAT_LEN + FLAGS_LEN;
/// Width of `metadata_application_format_identifier`, bytes.
const FORMAT_IDENTIFIER_LEN: usize = core::mem::size_of::<u32>();
/// Width of one `reserved(7) | value(33)` group, bytes.
const TIME_BASE_FIELD_LEN: usize = 5;
/// Width of the two-group time base block, bytes.
const TIME_BASE_BLOCK_LEN: usize = TIME_BASE_FIELD_LEN + TIME_BASE_FIELD_LEN;
/// Width of a 33-bit time base value, bits.
const TIME_BASE_VALUE_BITS: u32 = 33;
/// Shift that places the 33-bit value's high bit.
const TIME_BASE_HI_SHIFT: u32 = 32;
/// Shift between the reserved bits and the 33-bit value's high bit.
const RESERVED_SHIFT: u8 = 1;
/// Mask of the 33-bit value's high bit.
const LOW_BIT_MASK: u8 = 0x01;
/// `metadata_application_format` value that adds the format identifier.
const FORMAT_IDENTIFIER_PRESENT: u16 = 0xFFFF;
/// Shift of `metadata_application_format`'s high byte.
const FORMAT_HI_SHIFT: u8 = 8;
/// Mask of `content_reference_id_record_flag`.
const CONTENT_REFERENCE_ID_RECORD_FLAG_MASK: u8 = 0x80;
/// Mask of `content_time_base_indicator`.
const CONTENT_TIME_BASE_INDICATOR_MASK: u8 = 0x0F;
/// Shift of `content_time_base_indicator` in the flags byte.
const CONTENT_TIME_BASE_INDICATOR_SHIFT: u8 = 3;
/// `content_time_base_indicator` value that adds a `contentId`.
const CONTENT_TIME_BASE_INDICATOR_WITH_CONTENT_ID: u8 = 2;
/// First `content_time_base_indicator` value that adds association data.
const CONTENT_TIME_BASE_INDICATOR_ASSOCIATION_FIRST: u8 = 3;
/// Last `content_time_base_indicator` value that adds association data.
const CONTENT_TIME_BASE_INDICATOR_ASSOCIATION_LAST: u8 = 7;
/// Mask of `contentId`.
const CONTENT_ID_MASK: u8 = 0x7F;
/// Shift of the `reserved` bit above `contentId`.
const CONTENT_ID_RESERVED_SHIFT: u8 = 7;
/// Default of each 7-bit `reserved` ahead of a 33-bit time-base value (see
/// [`RESERVED_3_DEFAULT`]).
pub const RESERVED_7_DEFAULT: ReservedOnes<RESERVED_BEFORE_TIME_BASE_BITS> =
    ReservedOnes::<RESERVED_BEFORE_TIME_BASE_BITS>::FILL;
/// Default of the 1-bit `reserved` ahead of `contentId` (see
/// [`RESERVED_3_DEFAULT`]).
pub const RESERVED_1_DEFAULT: ReservedOnes<RESERVED_BEFORE_CONTENT_ID_BITS> =
    ReservedOnes::<RESERVED_BEFORE_CONTENT_ID_BITS>::FILL;

/// Time-base block — present when content_time_base_indicator is 1 or 2.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub struct ContentTimeBase {
    /// 33-bit content time base value (stored as u64, masked to 33 bits).
    pub content_time_base_value: u64,
    /// 33-bit metadata time base value (stored as u64, masked to 33 bits).
    pub metadata_time_base_value: u64,
    /// `reserved(7)` ahead of `content_time_base_value` (7 bits,
    /// right-aligned). Captured on parse, re-emitted verbatim; defaults to
    /// [`RESERVED_7_DEFAULT`].
    pub reserved_content: ReservedOnes<RESERVED_BEFORE_TIME_BASE_BITS>,
    /// `reserved(7)` ahead of `metadata_time_base_value` (7 bits,
    /// right-aligned). Captured on parse, re-emitted verbatim; defaults to
    /// [`RESERVED_7_DEFAULT`].
    pub reserved_metadata: ReservedOnes<RESERVED_BEFORE_TIME_BASE_BITS>,
}

impl ContentTimeBase {
    /// Build with the spec-default reserved bits.
    #[must_use]
    pub const fn new(content_time_base_value: u64, metadata_time_base_value: u64) -> Self {
        Self {
            content_time_base_value,
            metadata_time_base_value,
            reserved_content: RESERVED_7_DEFAULT,
            reserved_metadata: RESERVED_7_DEFAULT,
        }
    }
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
#[non_exhaustive]
pub struct ContentLabelingDescriptor<'a> {
    /// Metadata application format (u16; see Table 2-84).
    pub metadata_application_format: u16,
    /// Metadata application format identifier — present when `metadata_application_format == 0xFFFF`.
    pub metadata_application_format_identifier: Option<u32>,
    /// Content reference ID record present flag, as parsed from the wire.
    ///
    /// Informational only: [`Serialize`] derives the wire flag from
    /// `content_reference_id_record.is_some()`, so this field can never make
    /// the serializer emit a flag with no record (r03-W5).
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
    /// `reserved(3)` after `content_time_base_indicator` (3 bits,
    /// right-aligned). Captured on parse, re-emitted verbatim; defaults to
    /// [`RESERVED_3_DEFAULT`].
    pub reserved: ReservedOnes<RESERVED_AFTER_TIME_BASE_INDICATOR_BITS>,
    /// `reserved(1)` ahead of `contentId` (1 bit, right-aligned); on the wire
    /// only when `content_id` is present. Captured on parse, re-emitted
    /// verbatim; defaults to [`RESERVED_1_DEFAULT`].
    pub content_id_reserved: ReservedOnes<RESERVED_BEFORE_CONTENT_ID_BITS>,
}

impl<'a> ContentLabelingDescriptor<'a> {
    /// Build a minimal descriptor (indicator `0`, no optional blocks, no
    /// private data) with the spec-default reserved bits. Set the remaining
    /// fields (indicator + matching blocks, record, private data) directly.
    #[must_use]
    pub const fn new(metadata_application_format: u16) -> Self {
        Self {
            metadata_application_format,
            metadata_application_format_identifier: None,
            content_reference_id_record_flag: false,
            content_time_base_indicator: ContentTimeBaseIndicator::None,
            content_reference_id_record: None,
            time_base: None,
            content_id: None,
            time_base_association: None,
            private_data: &[],
            reserved: RESERVED_3_DEFAULT,
            content_id_reserved: RESERVED_1_DEFAULT,
        }
    }
}

impl Default for ContentLabelingDescriptor<'_> {
    fn default() -> Self {
        Self::new(0)
    }
}

// Table 2-83: `reserved(7) | value(33)` per group — the reserved bits are the
// TOP 7 bits of the first byte, and the 33-bit value's MSB is bit 0 of that
// same byte, followed by 4 full bytes (32 more bits). Returns
// `(reserved(7), value(33))`.
fn read_33bit(bytes: &[u8], pos: usize) -> (ReservedOnes<RESERVED_BEFORE_TIME_BASE_BITS>, u64) {
    (
        ReservedOnes::from_bits(u64::from(bytes[pos] >> RESERVED_SHIFT)),
        read_33bit_value(bytes, pos),
    )
}

fn read_33bit_value(bytes: &[u8], pos: usize) -> u64 {
    let mut value_bytes = [0u8; core::mem::size_of::<u32>()];
    value_bytes.copy_from_slice(&bytes[pos + 1..pos + TIME_BASE_FIELD_LEN]);
    ((u64::from(bytes[pos]) & u64::from(LOW_BIT_MASK)) << TIME_BASE_HI_SHIFT)
        | u64::from(u32::from_be_bytes(value_bytes))
}

fn write_33bit(
    buf: &mut [u8],
    pos: usize,
    reserved: ReservedOnes<RESERVED_BEFORE_TIME_BASE_BITS>,
    value: u64,
) -> Result<()> {
    // A value wider than 33 bits used to be masked down silently (audit
    // r03-W3 class).
    let v = broadcast_common::len::fit_bits(value, TIME_BASE_VALUE_BITS, "time_base_value")?;
    buf[pos] = ((reserved.get() as u8) << RESERVED_SHIFT)
        | ((v >> TIME_BASE_HI_SHIFT) as u8 & LOW_BIT_MASK);
    buf[pos + 1..pos + TIME_BASE_FIELD_LEN].copy_from_slice(&(v as u32).to_be_bytes());
    Ok(())
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
        if body.len() < HEADER_PREFIX_LEN {
            return Err(Error::InvalidDescriptor {
                tag: TAG,
                reason: "content_labeling_descriptor too short (< 3 body bytes)",
            });
        }

        let metadata_application_format = u16::from_be_bytes([body[0], body[1]]);
        let mut pos = FORMAT_LEN;

        let metadata_application_format_identifier = if metadata_application_format
            == FORMAT_IDENTIFIER_PRESENT
        {
            if body.len() < pos + FORMAT_IDENTIFIER_LEN {
                return Err(Error::InvalidDescriptor {
                    tag: TAG,
                    reason: "content_labeling_descriptor too short for metadata_application_format_identifier",
                });
            }
            let mut id_bytes = [0u8; FORMAT_IDENTIFIER_LEN];
            id_bytes.copy_from_slice(&body[pos..pos + FORMAT_IDENTIFIER_LEN]);
            let id = u32::from_be_bytes(id_bytes);
            pos += FORMAT_IDENTIFIER_LEN;
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
        let content_reference_id_record_flag = (flags & CONTENT_REFERENCE_ID_RECORD_FLAG_MASK) != 0;
        let indicator_raw =
            (flags >> CONTENT_TIME_BASE_INDICATOR_SHIFT) & CONTENT_TIME_BASE_INDICATOR_MASK;
        let content_time_base_indicator = ContentTimeBaseIndicator::from_u8(indicator_raw);
        let reserved = ReservedOnes::from_bits(u64::from(flags));
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

        let time_base =
            if indicator_raw == 1 || indicator_raw == CONTENT_TIME_BASE_INDICATOR_WITH_CONTENT_ID {
                // 7 reserved bits + 33-bit content_time_base_value + 7 reserved bits + 33-bit metadata_time_base_value
                // = 80 bits = 10 bytes
                if body.len() < pos + TIME_BASE_BLOCK_LEN {
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
                let (reserved_content, ctv) = read_33bit(&body[pos..], 0);
                let (reserved_metadata, mtv) = read_33bit(&body[pos + TIME_BASE_FIELD_LEN..], 0);
                pos += TIME_BASE_BLOCK_LEN;
                Some(ContentTimeBase {
                    content_time_base_value: ctv,
                    metadata_time_base_value: mtv,
                    reserved_content,
                    reserved_metadata,
                })
            } else {
                None
            };

        let mut content_id_reserved = RESERVED_1_DEFAULT;
        let content_id = if indicator_raw == CONTENT_TIME_BASE_INDICATOR_WITH_CONTENT_ID {
            if body.len() < pos + 1 {
                return Err(Error::InvalidDescriptor {
                    tag: TAG,
                    reason: "content_labeling_descriptor too short for contentId byte",
                });
            }
            let id = body[pos] & CONTENT_ID_MASK;
            content_id_reserved =
                ReservedOnes::from_bits(u64::from(body[pos] >> CONTENT_ID_RESERVED_SHIFT));
            pos += 1;
            Some(id)
        } else {
            None
        };

        let time_base_association = if (CONTENT_TIME_BASE_INDICATOR_ASSOCIATION_FIRST
            ..=CONTENT_TIME_BASE_INDICATOR_ASSOCIATION_LAST)
            .contains(&indicator_raw)
        {
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
            reserved,
            content_id_reserved,
        })
    }
}

impl Serialize for ContentLabelingDescriptor<'_> {
    type Error = crate::error::Error;

    fn serialized_len(&self) -> usize {
        let mut len: usize = HEADER_LEN + HEADER_PREFIX_LEN;
        if self.metadata_application_format_identifier.is_some() {
            len += FORMAT_IDENTIFIER_LEN;
        }
        if let Some(rec) = self.content_reference_id_record {
            len += 1 + rec.len();
        }
        if self.time_base.is_some() {
            len += TIME_BASE_BLOCK_LEN;
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
        need_out(buf, len)?;
        // The indicator selects which optional blocks are on the wire; a
        // disagreement with the Options would misframe on re-parse (r03-W5).
        let ind = broadcast_common::len::fit_mask(
            self.content_time_base_indicator.to_u8(),
            CONTENT_TIME_BASE_INDICATOR_MASK,
            "content_time_base_indicator",
        )?;
        let want_time_base = ind == 1 || ind == CONTENT_TIME_BASE_INDICATOR_WITH_CONTENT_ID;
        let want_content_id = ind == CONTENT_TIME_BASE_INDICATOR_WITH_CONTENT_ID;
        let want_assoc = (CONTENT_TIME_BASE_INDICATOR_ASSOCIATION_FIRST
            ..=CONTENT_TIME_BASE_INDICATOR_ASSOCIATION_LAST)
            .contains(&ind);
        for (field, want, have) in [
            ("time_base", want_time_base, self.time_base.is_some()),
            ("content_id", want_content_id, self.content_id.is_some()),
            (
                "time_base_association",
                want_assoc,
                self.time_base_association.is_some(),
            ),
        ] {
            if want != have {
                return Err(Error::ValueOutOfRange {
                    field,
                    reason: "presence must match content_time_base_indicator",
                });
            }
        }
        crate::descriptors::write_descriptor_header(buf, TAG, len - HEADER_LEN)?;

        buf[HEADER_LEN] = (self.metadata_application_format >> FORMAT_HI_SHIFT) as u8;
        buf[HEADER_LEN + 1] = self.metadata_application_format as u8;
        let mut pos = HEADER_LEN + FORMAT_LEN;

        if let Some(id) = self.metadata_application_format_identifier {
            buf[pos..pos + FORMAT_IDENTIFIER_LEN].copy_from_slice(&id.to_be_bytes());
            pos += FORMAT_IDENTIFIER_LEN;
        }

        let mut flags = (broadcast_common::len::fit_mask(
            self.content_time_base_indicator.to_u8(),
            CONTENT_TIME_BASE_INDICATOR_MASK,
            "content_time_base_indicator",
        )? << CONTENT_TIME_BASE_INDICATOR_SHIFT)
            | self.reserved.get() as u8;
        // Derived from the Option, never the stored flag (r03-W5).
        if self.content_reference_id_record.is_some() {
            flags |= CONTENT_REFERENCE_ID_RECORD_FLAG_MASK;
        }
        buf[pos] = flags;
        pos += 1;

        if let Some(rec) = self.content_reference_id_record {
            buf[pos] =
                broadcast_common::len::fit_u8(rec.len(), "content_reference_id_record_length")?;
            pos += 1;
            buf[pos..pos + rec.len()].copy_from_slice(rec);
            pos += rec.len();
        }

        if let Some(ref tb) = self.time_base {
            // reserved(7) + 33-bit content_time_base_value
            write_33bit(buf, pos, tb.reserved_content, tb.content_time_base_value)?;
            pos += TIME_BASE_FIELD_LEN;
            // reserved(7) + 33-bit metadata_time_base_value
            write_33bit(buf, pos, tb.reserved_metadata, tb.metadata_time_base_value)?;
            pos += TIME_BASE_FIELD_LEN;
        }

        if let Some(cid) = self.content_id {
            buf[pos] = ((self.content_id_reserved.get() as u8) << CONTENT_ID_RESERVED_SHIFT)
                | broadcast_common::len::fit_mask(cid, CONTENT_ID_MASK, "cid")?;
            pos += 1;
        }

        if let Some(ref assoc) = self.time_base_association {
            buf[pos] = broadcast_common::len::fit_u8(
                assoc.data.len(),
                "content_time_base_association_data_length",
            )?;
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
    use alloc::vec;

    /// r03-W3 class: a 33-bit time-base value wider than its field used to be
    /// masked down silently into a different timestamp.
    #[test]
    fn serialize_rejects_time_base_value_wider_than_33_bits() {
        let mut d = ContentLabelingDescriptor::new(0x0001);
        d.content_time_base_indicator = ContentTimeBaseIndicator::Stc;
        d.time_base = Some(ContentTimeBase::new(1 << 33, 0));
        let mut buf = vec![0u8; d.serialized_len()];
        assert!(matches!(
            d.serialize_into(&mut buf),
            Err(Error::FieldOverflow(_))
        ));
        d.time_base = Some(ContentTimeBase::new((1 << 33) - 1, (1 << 33) - 1));
        let mut buf = vec![0u8; d.serialized_len()];
        d.serialize_into(&mut buf).unwrap();
        let re = ContentLabelingDescriptor::parse(&buf).unwrap();
        assert_eq!(re.time_base, d.time_base);
    }

    // Helper to build a descriptor with all needed bytes
    fn serialize_round_trip(d: &ContentLabelingDescriptor<'_>) {
        let mut buf = vec![0u8; d.serialized_len()];
        let written = d.serialize_into(&mut buf).unwrap();
        assert_eq!(written, d.serialized_len());
        let reparsed = ContentLabelingDescriptor::parse(&buf).unwrap();
        assert_eq!(*d, reparsed, "round-trip mismatch");
    }

    #[test]
    fn round_trips_every_case() {
        for d in [
            ContentLabelingDescriptor {
                metadata_application_format: 0x0010,
                metadata_application_format_identifier: None,
                content_reference_id_record_flag: false,
                content_time_base_indicator: ContentTimeBaseIndicator::None,
                content_reference_id_record: None,
                time_base: None,
                content_id: None,
                time_base_association: None,
                reserved: RESERVED_3_DEFAULT,
                content_id_reserved: RESERVED_1_DEFAULT,
                private_data: &[],
            },
            ContentLabelingDescriptor {
                metadata_application_format: 0x0011,
                metadata_application_format_identifier: None,
                content_reference_id_record_flag: true,
                content_time_base_indicator: ContentTimeBaseIndicator::None,
                content_reference_id_record: Some(&[0xAA, 0xBB, 0xCC]),
                time_base: None,
                content_id: None,
                time_base_association: None,
                reserved: RESERVED_3_DEFAULT,
                content_id_reserved: RESERVED_1_DEFAULT,
                private_data: &[0xDD, 0xEE],
            },
            ContentLabelingDescriptor {
                metadata_application_format: 0x0100,
                metadata_application_format_identifier: None,
                content_reference_id_record_flag: false,
                content_time_base_indicator: ContentTimeBaseIndicator::Stc,
                content_reference_id_record: None,
                time_base: Some(ContentTimeBase::new(0x123456789, 0x1ABCDEF01)),
                content_id: None,
                time_base_association: None,
                reserved: RESERVED_3_DEFAULT,
                content_id_reserved: RESERVED_1_DEFAULT,
                private_data: &[],
            },
            ContentLabelingDescriptor {
                metadata_application_format: 0x0100,
                metadata_application_format_identifier: None,
                content_reference_id_record_flag: false,
                content_time_base_indicator: ContentTimeBaseIndicator::Npt,
                content_reference_id_record: None,
                time_base: Some(ContentTimeBase::new(0x1AABBCCDD, 0x0)),
                content_id: Some(42),
                time_base_association: None,
                reserved: RESERVED_3_DEFAULT,
                content_id_reserved: RESERVED_1_DEFAULT,
                private_data: &[],
            },
            ContentLabelingDescriptor {
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
                reserved: RESERVED_3_DEFAULT,
                content_id_reserved: RESERVED_1_DEFAULT,
                private_data: &[],
            },
            ContentLabelingDescriptor {
                metadata_application_format: 0xFFFF,
                metadata_application_format_identifier: Some(0xDEADBEEF),
                content_reference_id_record_flag: true,
                content_time_base_indicator: ContentTimeBaseIndicator::None,
                content_reference_id_record: Some(&[0x01, 0x02]),
                time_base: None,
                content_id: None,
                time_base_association: None,
                reserved: RESERVED_3_DEFAULT,
                content_id_reserved: RESERVED_1_DEFAULT,
                private_data: &[0xFF],
            },
        ] {
            serialize_round_trip(&d);
        }
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
            reserved: RESERVED_3_DEFAULT,
            content_id_reserved: RESERVED_1_DEFAULT,
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

    fn minimal<'a>() -> ContentLabelingDescriptor<'a> {
        ContentLabelingDescriptor {
            metadata_application_format: 0x0010,
            metadata_application_format_identifier: None,
            content_reference_id_record_flag: false,
            content_time_base_indicator: ContentTimeBaseIndicator::None,
            content_reference_id_record: None,
            time_base: None,
            content_id: None,
            time_base_association: None,
            reserved: RESERVED_3_DEFAULT,
            content_id_reserved: RESERVED_1_DEFAULT,
            private_data: &[],
        }
    }

    /// r03-W5: the wire flag follows `content_reference_id_record`, not the
    /// stored bool, in both mismatch directions.
    #[test]
    fn serialize_derives_record_flag_from_option() {
        let mut d = minimal();
        d.content_reference_id_record_flag = true; // lies: no record
        let mut buf = vec![0u8; d.serialized_len()];
        d.serialize_into(&mut buf).unwrap();
        let re = ContentLabelingDescriptor::parse(&buf).unwrap();
        assert!(!re.content_reference_id_record_flag);
        assert!(re.content_reference_id_record.is_none());

        let mut d = minimal();
        d.content_reference_id_record = Some(&[0xAA, 0xBB]); // flag stays false
        let mut buf = vec![0u8; d.serialized_len()];
        d.serialize_into(&mut buf).unwrap();
        let re = ContentLabelingDescriptor::parse(&buf).unwrap();
        assert!(re.content_reference_id_record_flag);
        assert_eq!(re.content_reference_id_record, Some(&[0xAA, 0xBB][..]));
    }

    /// An indicator whose required block is absent (or present when not
    /// allowed) is rejected rather than misframed.
    #[test]
    fn serialize_rejects_indicator_option_mismatch() {
        let mut d = minimal();
        d.content_time_base_indicator = ContentTimeBaseIndicator::Stc; // needs time_base
        let mut buf = vec![0u8; 64];
        assert!(matches!(
            d.serialize_into(&mut buf),
            Err(Error::ValueOutOfRange {
                field: "time_base",
                ..
            })
        ));
        let mut d = minimal();
        d.content_id = Some(1); // indicator None forbids content_id
        assert!(matches!(
            d.serialize_into(&mut buf),
            Err(Error::ValueOutOfRange {
                field: "content_id",
                ..
            })
        ));
    }

    /// H.222.0 Table 2-83. Every reserved region (flags-byte `reserved(3)`,
    /// both `reserved(7)` ahead of the 33-bit values, `reserved(1)` ahead of
    /// `contentId`) is preserved verbatim, zero and non-default values.
    #[test]
    fn content_labeling_reserved_preserved() {
        for r3 in 0u8..=7 {
            for r7 in [0u8, 1, 0x2A, 0x55, 0x7F] {
                for r1 in 0u8..=1 {
                    let bytes = [
                        TAG,
                        3 + 10 + 1,
                        0x01,
                        0x00,
                        (2 << 3) | r3, // indicator 2 (NPT), no record
                        (r7 << 1) | 1,
                        0x23,
                        0x45,
                        0x67,
                        0x89,
                        ((r7 ^ 0x7F) << 1),
                        0x00,
                        0x00,
                        0x00,
                        0x01,
                        (r1 << 7) | 42,
                    ];
                    let d = ContentLabelingDescriptor::parse(&bytes).unwrap();
                    assert_eq!(d.reserved.get(), u64::from(r3));
                    assert_eq!(d.content_id_reserved.get(), u64::from(r1));
                    let tb = d.time_base.as_ref().unwrap();
                    assert_eq!(tb.reserved_content.get(), u64::from(r7));
                    assert_eq!(tb.reserved_metadata.get(), u64::from(r7 ^ 0x7F));
                    let mut out = vec![0u8; d.serialized_len()];
                    d.serialize_into(&mut out).unwrap();
                    assert_eq!(out, bytes);
                }
            }
        }
    }

    /// Issue #1005 spec vector, independent of the serializer: Table 2-83 is
    /// `reserved(7) | content_time_base_value(33)`, so the conformant bytes
    /// `FE 00 00 00 01` decode to 1 (the pre-fix reader took the FIRST 33
    /// bits and returned 0x1_FC00_0000), and `FF FF FF FF FF` decodes to
    /// the 33-bit maximum.
    #[test]
    fn time_base_values_read_the_low_33_bits() {
        let bytes = [
            TAG, 14, 0x01, 0x00, 0x17, 0xFE, 0x00, 0x00, 0x00, 0x01, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF,
            0xAA,
        ];
        let d = ContentLabelingDescriptor::parse(&bytes).unwrap();
        let tb = d.time_base.as_ref().unwrap();
        assert_eq!(tb.content_time_base_value, 1);
        assert_eq!(tb.metadata_time_base_value, 0x1_FFFF_FFFF);
        let mut out = vec![0u8; d.serialized_len()];
        d.serialize_into(&mut out).unwrap();
        assert_eq!(out, bytes);
    }

    #[test]
    fn content_labeling_default_is_spec_value() {
        let mut d = ContentLabelingDescriptor::new(0x0100);
        d.content_time_base_indicator = ContentTimeBaseIndicator::Npt;
        d.time_base = Some(ContentTimeBase::new(0x1_2345_6789, 0));
        d.content_id = Some(42);
        let mut out = vec![0u8; d.serialized_len()];
        d.serialize_into(&mut out).unwrap();
        assert_eq!(
            out,
            [
                TAG, 14, 0x01, 0x00, 0x17, 0xFF, 0x23, 0x45, 0x67, 0x89, 0xFE, 0x00, 0x00, 0x00,
                0x00, 0xAA
            ]
        );
    }

    /// The minimal 0xFFFF form (2 + 4 identifier + 1 flags = 7 body bytes) and
    /// the identifier's fourth byte coming from its own position, not the flags
    /// byte.
    #[test]
    fn minimal_application_format_identifier_form_round_trips() {
        let bytes = [TAG, 7, 0xFF, 0xFF, 0x12, 0x34, 0x56, 0x78, 0x07];
        let d = ContentLabelingDescriptor::parse(&bytes).unwrap();
        assert_eq!(d.metadata_application_format, 0xFFFF);
        assert_eq!(d.metadata_application_format_identifier, Some(0x1234_5678));
        assert_eq!(
            d.content_time_base_indicator,
            ContentTimeBaseIndicator::None
        );
        assert!(d.content_reference_id_record.is_none());
        assert!(d.time_base.is_none());
        assert!(d.private_data.is_empty());
        let mut out = vec![0u8; d.serialized_len()];
        d.serialize_into(&mut out).unwrap();
        assert_eq!(out, bytes);
    }

    /// Bodies truncated before the identifier or before the flags byte are
    /// errors, never an out-of-bounds read.
    #[test]
    fn truncated_application_format_forms_are_errors() {
        assert!(matches!(
            ContentLabelingDescriptor::parse(&[TAG, 5, 0xFF, 0xFF, 0x12, 0x34, 0x56]).unwrap_err(),
            Error::InvalidDescriptor { .. }
        ));
        // The identifier is complete here; the missing byte is the flags byte.
        assert_eq!(
            ContentLabelingDescriptor::parse(&[TAG, 6, 0xFF, 0xFF, 0x12, 0x34, 0x56, 0x78])
                .unwrap_err(),
            Error::InvalidDescriptor {
                tag: TAG,
                reason: "content_labeling_descriptor too short for flags byte"
            }
        );
        // Flags byte is the last body byte and nothing follows.
        let bytes = [TAG, 3, 0x00, 0x01, 0x07];
        let d = ContentLabelingDescriptor::parse(&bytes).unwrap();
        assert_eq!(
            d.content_time_base_indicator,
            ContentTimeBaseIndicator::None
        );
        assert!(d.private_data.is_empty());
    }

    /// Flag-set descriptors whose record-length or association-length byte is
    /// the last one are valid with an empty block.
    #[test]
    fn zero_length_trailing_blocks_are_valid() {
        // Content reference record present, length 0.
        let bytes = [TAG, 4, 0x00, 0x01, 0x87, 0x00];
        let d = ContentLabelingDescriptor::parse(&bytes).unwrap();
        assert_eq!(d.content_reference_id_record, Some(&[][..]));
        assert!(d.private_data.is_empty());
        let mut out = vec![0u8; d.serialized_len()];
        d.serialize_into(&mut out).unwrap();
        assert_eq!(out, bytes);

        // Time base association (indicator 3), length 0.
        let bytes = [TAG, 4, 0x00, 0x01, 0x1F, 0x00];
        let d = ContentLabelingDescriptor::parse(&bytes).unwrap();
        assert_eq!(
            d.content_time_base_indicator,
            ContentTimeBaseIndicator::Reserved(3)
        );
        assert_eq!(
            d.time_base_association.as_ref().map(|a| a.data),
            Some(&[][..])
        );
        assert!(d.private_data.is_empty());
        let mut out = vec![0u8; d.serialized_len()];
        d.serialize_into(&mut out).unwrap();
        assert_eq!(out, bytes);
    }
}
