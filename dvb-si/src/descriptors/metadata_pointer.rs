//! Metadata Pointer Descriptor — ISO/IEC 13818-1 §2.6.58, Table 2-86 (tag 0x25).
//!
//! Points to a metadata service that carries metadata for the associated
//! program or elementary stream. The program_number block is conditional on
//! `MPEG_carriage_flags <= 2`; the transport_stream_location + transport_stream_id
//! block only when `MPEG_carriage_flags == 1`.

use super::descriptor_body;
use crate::descriptors::metadata_format::MetadataFormat;
use crate::descriptors::mpeg_carriage_flags::MpegCarriageFlags;
use crate::error::{Error, Result};
use broadcast_core::len::need_out;
use broadcast_core::reserved::ReservedOnes;
use broadcast_core::{Parse, Serialize};

/// Descriptor tag for metadata_pointer_descriptor.
pub const TAG: u8 = 0x25;
const HEADER_LEN: usize = 2;
/// Default of the 5-bit `reserved` tail of the flags byte (H.222.0 Table
/// 2-86): clause 2.1 "reserved" = `1` (all editions 2006-08/2023, audit §5; the
/// MPEG_carriage_flags semantics widen in 2017 but the layout is unchanged; no
/// fixture carries the descriptor).
pub const RESERVED_DEFAULT: ReservedOnes<RESERVED_BITS> = ReservedOnes::<RESERVED_BITS>::FILL;
/// Width of the `reserved` field in the flags byte (5 bits).
pub const RESERVED_BITS: u32 = 5;
/// Minimum body: application_format(2) + format(1) + service_id(1) + flags(1).
const MIN_BODY_LEN: usize = 5;
/// `metadata_application_format` value that introduces an identifier.
const METADATA_APPLICATION_FORMAT_IDENTIFIER_PRESENT: u16 = 0xFFFF;
/// `metadata_format` value that introduces an identifier.
const METADATA_FORMAT_IDENTIFIER_PRESENT: u8 = 0xFF;
/// Wire width of a format identifier (32 bits).
const IDENTIFIER_LEN: usize = size_of::<u32>();
/// Wire width of `metadata_application_format`.
const METADATA_APPLICATION_FORMAT_LEN: usize = size_of::<u16>();
/// Body bytes occupied by `metadata_service_id` plus the flags byte.
const SERVICE_ID_AND_FLAGS_LEN: usize = 2;
/// `metadata_locator_record_flag` bit of the flags byte.
const LOCATOR_RECORD_FLAG: u8 = 0x80;
/// Mask of the 2-bit `MPEG_carriage_flags` field (Table 2-88).
const MPEG_CARRIAGE_FLAGS_MASK: u8 = 0x03;
/// Shift of `MPEG_carriage_flags` in the flags byte.
const MPEG_CARRIAGE_FLAGS_SHIFT: u8 = 5;
/// Highest `MPEG_carriage_flags` value that carries `program_number`.
const PROGRAM_NUMBER_PRESENT_MAX_CARRIAGE: u8 = 2;
/// `MPEG_carriage_flags` value that carries the TS location fields.
const TRANSPORT_STREAM_LOCATION_CARRIAGE: u8 = 1;
/// Wire width of `transport_stream_location` plus `transport_stream_id`.
const TRANSPORT_STREAM_LOCATION_LEN: usize = 2 * size_of::<u16>();
/// Shift of the high byte of `metadata_application_format`.
const HIGH_BYTE_SHIFT: u32 = 8;

/// Metadata Pointer Descriptor.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[cfg_attr(feature = "yoke", derive(yoke::Yokeable))]
#[non_exhaustive]
pub struct MetadataPointerDescriptor<'a> {
    /// Metadata application format (u16; Table 2-84).
    pub metadata_application_format: u16,
    /// Metadata application format identifier — present when `metadata_application_format == 0xFFFF`.
    pub metadata_application_format_identifier: Option<u32>,
    /// Metadata format (Table 2-87).
    pub metadata_format: MetadataFormat,
    /// Metadata format identifier — present when `metadata_format == 0xFF`.
    pub metadata_format_identifier: Option<u32>,
    /// Metadata service ID.
    pub metadata_service_id: u8,
    /// Metadata locator record present flag.
    pub metadata_locator_record_flag: bool,
    /// MPEG carriage flags (Table 2-88).
    pub mpeg_carriage_flags: MpegCarriageFlags,
    /// Metadata locator record — present when `metadata_locator_record_flag` is true.
    #[cfg_attr(feature = "serde", serde(borrow))]
    pub metadata_locator_record: Option<&'a [u8]>,
    /// Program number — present when `MPEG_carriage_flags <= 2`.
    pub program_number: Option<u16>,
    /// Transport stream location — present when `MPEG_carriage_flags == 1`.
    pub transport_stream_location: Option<u16>,
    /// Transport stream ID — present when `MPEG_carriage_flags == 1`.
    pub transport_stream_id: Option<u16>,
    /// Trailing private data bytes.
    #[cfg_attr(feature = "serde", serde(borrow))]
    pub private_data: &'a [u8],
    /// `reserved(5)` tail of the flags byte (5 bits). Captured on parse,
    /// re-emitted verbatim; [`MetadataPointerDescriptor::new`] / [`Default`]
    /// use [`RESERVED_DEFAULT`].
    #[cfg_attr(feature = "serde", serde(skip))]
    pub reserved: ReservedOnes<RESERVED_BITS>,
}

impl<'a> MetadataPointerDescriptor<'a> {
    /// Build a minimal descriptor (no identifiers, no locator record, no
    /// program/TS fields, no private data, `MPEG_carriage_flags` = 3) with the
    /// spec-default reserved bits. Set the remaining fields directly.
    #[must_use]
    pub const fn new(
        metadata_application_format: u16,
        metadata_format: MetadataFormat,
        metadata_service_id: u8,
    ) -> Self {
        Self {
            metadata_application_format,
            metadata_application_format_identifier: None,
            metadata_format,
            metadata_format_identifier: None,
            metadata_service_id,
            metadata_locator_record_flag: false,
            mpeg_carriage_flags: MpegCarriageFlags::None,
            metadata_locator_record: None,
            program_number: None,
            transport_stream_location: None,
            transport_stream_id: None,
            private_data: &[],
            reserved: RESERVED_DEFAULT,
        }
    }
}

impl<'a> Parse<'a> for MetadataPointerDescriptor<'a> {
    type Error = crate::error::Error;

    fn parse(bytes: &'a [u8]) -> Result<Self> {
        let body = descriptor_body(
            bytes,
            TAG,
            "MetadataPointerDescriptor",
            "unexpected tag for metadata_pointer_descriptor",
        )?;

        // Minimum: metadata_application_format(2) + metadata_format(1) + metadata_service_id(1) + flags(1) = 5
        if body.len() < MIN_BODY_LEN {
            return Err(Error::InvalidDescriptor {
                tag: TAG,
                reason: "metadata_pointer_descriptor too short (< 5 body bytes)",
            });
        }

        let metadata_application_format = u16::from_be_bytes([body[0], body[1]]);
        let mut pos = METADATA_APPLICATION_FORMAT_LEN;

        let metadata_application_format_identifier = if metadata_application_format
            == METADATA_APPLICATION_FORMAT_IDENTIFIER_PRESENT
        {
            if body.len() < pos + IDENTIFIER_LEN {
                return Err(Error::InvalidDescriptor {
                    tag: TAG,
                    reason: "metadata_pointer_descriptor too short for metadata_application_format_identifier",
                });
            }
            let id = u32::from_be_bytes(
                body[pos..pos + IDENTIFIER_LEN]
                    .try_into()
                    .unwrap_or_default(),
            );
            pos += IDENTIFIER_LEN;
            Some(id)
        } else {
            None
        };

        if body.len() < pos + 1 {
            return Err(Error::InvalidDescriptor {
                tag: TAG,
                reason: "metadata_pointer_descriptor too short for metadata_format",
            });
        }
        let metadata_format = MetadataFormat::from_u8(body[pos]);
        pos += 1;

        let metadata_format_identifier = if body[pos - 1] == METADATA_FORMAT_IDENTIFIER_PRESENT {
            if body.len() < pos + IDENTIFIER_LEN {
                return Err(Error::InvalidDescriptor {
                    tag: TAG,
                    reason: "metadata_pointer_descriptor too short for metadata_format_identifier",
                });
            }
            let id = u32::from_be_bytes(
                body[pos..pos + IDENTIFIER_LEN]
                    .try_into()
                    .unwrap_or_default(),
            );
            pos += IDENTIFIER_LEN;
            Some(id)
        } else {
            None
        };

        if body.len() < pos + SERVICE_ID_AND_FLAGS_LEN {
            return Err(Error::InvalidDescriptor {
                tag: TAG,
                reason: "metadata_pointer_descriptor too short for service_id + flags",
            });
        }
        let metadata_service_id = body[pos];
        pos += 1;

        let flags = body[pos];
        let metadata_locator_record_flag = (flags & LOCATOR_RECORD_FLAG) != 0;
        let mpeg_carriage_flags = MpegCarriageFlags::from_u8(
            (flags >> MPEG_CARRIAGE_FLAGS_SHIFT) & MPEG_CARRIAGE_FLAGS_MASK,
        );
        let reserved = ReservedOnes::from_bits(u64::from(flags));
        pos += 1;

        let (metadata_locator_record, mut pos) = if metadata_locator_record_flag {
            if body.len() < pos + 1 {
                return Err(Error::InvalidDescriptor {
                    tag: TAG,
                    reason: "metadata_pointer_descriptor too short for metadata_locator_record_length",
                });
            }
            let rec_len = body[pos] as usize;
            pos += 1;
            if body.len() < pos + rec_len {
                return Err(Error::InvalidDescriptor {
                    tag: TAG,
                    reason: "metadata_pointer_descriptor too short for metadata_locator_record",
                });
            }
            let rec = &body[pos..pos + rec_len];
            pos += rec_len;
            (Some(rec), pos)
        } else {
            (None, pos)
        };

        let carriage_val = mpeg_carriage_flags.to_u8();

        let program_number = if carriage_val <= PROGRAM_NUMBER_PRESENT_MAX_CARRIAGE {
            if body.len() < pos + size_of::<u16>() {
                return Err(Error::InvalidDescriptor {
                    tag: TAG,
                    reason: "metadata_pointer_descriptor too short for program_number",
                });
            }
            let pn = crate::wire::be_u16(body, pos, "program_number")?;
            pos += size_of::<u16>();
            Some(pn)
        } else {
            None
        };

        let (transport_stream_location, transport_stream_id, pos) = if carriage_val
            == TRANSPORT_STREAM_LOCATION_CARRIAGE
        {
            if body.len() < pos + TRANSPORT_STREAM_LOCATION_LEN {
                return Err(Error::InvalidDescriptor {
                    tag: TAG,
                    reason: "metadata_pointer_descriptor too short for transport_stream_location+id",
                });
            }
            let tsl = crate::wire::be_u16(body, pos, "transport_stream_location")?;
            let tsi = crate::wire::be_u16(body, pos + size_of::<u16>(), "transport_stream_id")?;
            pos += TRANSPORT_STREAM_LOCATION_LEN;
            (Some(tsl), Some(tsi), pos)
        } else {
            (None, None, pos)
        };

        let private_data = &body[pos..];

        Ok(Self {
            metadata_application_format,
            metadata_application_format_identifier,
            metadata_format,
            metadata_format_identifier,
            metadata_service_id,
            metadata_locator_record_flag,
            mpeg_carriage_flags,
            metadata_locator_record,
            program_number,
            transport_stream_location,
            transport_stream_id,
            private_data,
            reserved,
        })
    }
}

impl Serialize for MetadataPointerDescriptor<'_> {
    type Error = crate::error::Error;

    fn serialized_len(&self) -> usize {
        let mut len: usize = HEADER_LEN + MIN_BODY_LEN;
        if self.metadata_application_format_identifier.is_some() {
            len += IDENTIFIER_LEN;
        }
        if self.metadata_format_identifier.is_some() {
            len += IDENTIFIER_LEN;
        }
        if let Some(rec) = self.metadata_locator_record {
            len += 1 + rec.len();
        }
        if self.program_number.is_some() {
            len += size_of::<u16>();
        }
        if self.transport_stream_location.is_some() {
            len += TRANSPORT_STREAM_LOCATION_LEN;
        }
        len += self.private_data.len();
        len
    }

    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        let len = self.serialized_len();
        need_out(buf, len)?;
        crate::descriptors::write_descriptor_header(buf, TAG, len - HEADER_LEN)?;

        buf[HEADER_LEN] = (self.metadata_application_format >> HIGH_BYTE_SHIFT) as u8;
        buf[HEADER_LEN + 1] = self.metadata_application_format as u8;
        let mut pos = HEADER_LEN + METADATA_APPLICATION_FORMAT_LEN;

        if let Some(id) = self.metadata_application_format_identifier {
            buf[pos..pos + IDENTIFIER_LEN].copy_from_slice(&id.to_be_bytes());
            pos += IDENTIFIER_LEN;
        }

        buf[pos] = self.metadata_format.to_u8();
        pos += 1;

        if let Some(id) = self.metadata_format_identifier {
            buf[pos..pos + IDENTIFIER_LEN].copy_from_slice(&id.to_be_bytes());
            pos += IDENTIFIER_LEN;
        }

        buf[pos] = self.metadata_service_id;
        pos += 1;

        // The record flag is derived from the Option, so it can never claim
        // a record that is not written (r03-W5).
        // `reserved(5)` (Table 2-86) is re-emitted as parsed.
        let mut flags = (broadcast_core::len::fit_mask(
            self.mpeg_carriage_flags.to_u8(),
            MPEG_CARRIAGE_FLAGS_MASK,
            "mpeg_carriage_flags",
        )? << MPEG_CARRIAGE_FLAGS_SHIFT)
            | self.reserved.get() as u8;
        if self.metadata_locator_record.is_some() {
            flags |= LOCATOR_RECORD_FLAG;
        }
        buf[pos] = flags;
        pos += 1;

        if let Some(rec) = self.metadata_locator_record {
            buf[pos] = broadcast_core::len::fit_u8(rec.len(), "metadata_locator_record_length")?;
            pos += 1;
            buf[pos..pos + rec.len()].copy_from_slice(rec);
            pos += rec.len();
        }

        if let Some(pn) = self.program_number {
            buf[pos..pos + size_of::<u16>()].copy_from_slice(&pn.to_be_bytes());
            pos += size_of::<u16>();
        }
        if let Some(tsl) = self.transport_stream_location {
            // `transport_stream_id` is co-present on the wire; fabricating 0
            // would misframe, so it must be present too (r03-W5).
            let tsi = self.transport_stream_id.ok_or(Error::ValueOutOfRange {
                field: "transport_stream_id",
                reason: "required when transport_stream_location is present",
            })?;
            buf[pos..pos + size_of::<u16>()].copy_from_slice(&tsl.to_be_bytes());
            buf[pos + size_of::<u16>()..pos + TRANSPORT_STREAM_LOCATION_LEN]
                .copy_from_slice(&tsi.to_be_bytes());
            pos += TRANSPORT_STREAM_LOCATION_LEN;
        }

        buf[pos..pos + self.private_data.len()].copy_from_slice(self.private_data);
        Ok(len)
    }
}

impl<'a> crate::traits::DescriptorDef<'a> for MetadataPointerDescriptor<'a> {
    const TAG: u8 = TAG;
    const NAME: &'static str = "METADATA_POINTER";
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::vec;

    fn serialize_round_trip(d: &MetadataPointerDescriptor<'_>) {
        let mut buf = vec![0u8; d.serialized_len()];
        let written = d.serialize_into(&mut buf).unwrap();
        assert_eq!(written, d.serialized_len());
        let reparsed = MetadataPointerDescriptor::parse(&buf).unwrap();
        assert_eq!(*d, reparsed, "round-trip mismatch");
    }

    #[test]
    fn round_trips_every_case() {
        for d in [
            MetadataPointerDescriptor {
                metadata_application_format: 0x0010,
                metadata_application_format_identifier: None,
                metadata_format: MetadataFormat::TeM,
                metadata_format_identifier: None,
                metadata_service_id: 5,
                metadata_locator_record_flag: false,
                mpeg_carriage_flags: MpegCarriageFlags::SameTs,
                metadata_locator_record: None,
                program_number: Some(0x0101),
                transport_stream_location: None,
                transport_stream_id: None,
                reserved: RESERVED_DEFAULT,
                private_data: &[],
            },
            MetadataPointerDescriptor {
                metadata_application_format: 0x0011,
                metadata_application_format_identifier: None,
                metadata_format: MetadataFormat::BiM,
                metadata_format_identifier: None,
                metadata_service_id: 7,
                metadata_locator_record_flag: false,
                mpeg_carriage_flags: MpegCarriageFlags::DifferentTs,
                metadata_locator_record: None,
                program_number: Some(0x0202),
                transport_stream_location: Some(0x0303),
                transport_stream_id: Some(0x0404),
                reserved: RESERVED_DEFAULT,
                private_data: &[],
            },
            MetadataPointerDescriptor {
                metadata_application_format: 0x0100,
                metadata_application_format_identifier: None,
                metadata_format: MetadataFormat::AppFormat,
                metadata_format_identifier: None,
                metadata_service_id: 3,
                metadata_locator_record_flag: false,
                mpeg_carriage_flags: MpegCarriageFlags::ProgramStream,
                metadata_locator_record: None,
                program_number: Some(0x0505),
                transport_stream_location: None,
                transport_stream_id: None,
                reserved: RESERVED_DEFAULT,
                private_data: &[],
            },
            MetadataPointerDescriptor {
                metadata_application_format: 0x0200,
                metadata_application_format_identifier: None,
                metadata_format: MetadataFormat::Private(0x80),
                metadata_format_identifier: None,
                metadata_service_id: 1,
                metadata_locator_record_flag: true,
                mpeg_carriage_flags: MpegCarriageFlags::None,
                metadata_locator_record: Some(&[0xAA, 0xBB]),
                program_number: None,
                transport_stream_location: None,
                transport_stream_id: None,
                reserved: RESERVED_DEFAULT,
                private_data: &[0xCC],
            },
            MetadataPointerDescriptor {
                metadata_application_format: 0xFFFF,
                metadata_application_format_identifier: Some(0x12345678),
                metadata_format: MetadataFormat::Identifier,
                metadata_format_identifier: Some(0x9ABCDEF0),
                metadata_service_id: 9,
                metadata_locator_record_flag: false,
                mpeg_carriage_flags: MpegCarriageFlags::SameTs,
                metadata_locator_record: None,
                program_number: Some(42),
                transport_stream_location: None,
                transport_stream_id: None,
                reserved: RESERVED_DEFAULT,
                private_data: &[0xFF],
            },
            MetadataPointerDescriptor {
                metadata_application_format: 0x0010,
                metadata_application_format_identifier: None,
                metadata_format: MetadataFormat::Reserved0(0x05),
                metadata_format_identifier: None,
                metadata_service_id: 0,
                metadata_locator_record_flag: false,
                mpeg_carriage_flags: MpegCarriageFlags::None,
                metadata_locator_record: None,
                program_number: None,
                transport_stream_location: None,
                transport_stream_id: None,
                reserved: RESERVED_DEFAULT,
                private_data: &[],
            },
        ] {
            serialize_round_trip(&d);
        }
    }

    #[test]
    fn parse_rejects_wrong_tag() {
        let err = MetadataPointerDescriptor::parse(&[0x02, 5, 0, 0, 0, 0, 0]).unwrap_err();
        assert!(matches!(err, Error::InvalidDescriptor { tag: 0x02, .. }));
    }

    #[test]
    fn parse_rejects_too_short() {
        let err = MetadataPointerDescriptor::parse(&[TAG, 0]).unwrap_err();
        assert!(matches!(err, Error::InvalidDescriptor { tag: TAG, .. }));
    }
    #[test]
    fn serialize_derives_locator_flag_and_requires_co_present_tsid() {
        // r03-W5: `metadata_locator_record_flag` used to be a stored bool, so
        // it could claim a record that was never written, and a lone
        // `transport_stream_location` fabricated a 0 transport_stream_id.
        let base = MetadataPointerDescriptor {
            metadata_application_format: 1,
            metadata_application_format_identifier: None,
            metadata_format: MetadataFormat::from_u8(1),
            metadata_format_identifier: None,
            metadata_service_id: 2,
            metadata_locator_record_flag: false,
            metadata_locator_record: None,
            program_number: None,
            transport_stream_location: None,
            transport_stream_id: None,
            mpeg_carriage_flags: MpegCarriageFlags::from_u8(0),
            reserved: RESERVED_DEFAULT,
            private_data: &[],
        };
        let mut d = base.clone();
        d.mpeg_carriage_flags = MpegCarriageFlags::from_u8(1);
        d.transport_stream_location = Some(0x0101);
        let mut buf = vec![0u8; d.serialized_len()];
        assert!(matches!(
            d.serialize_into(&mut buf).unwrap_err(),
            Error::ValueOutOfRange {
                field: "transport_stream_id",
                ..
            }
        ));
        // carriage_flags == 3 needs no program_number, so the second case can
        // exercise the lying flag: the *stored* flag says true but no record
        // is written; serialize must emit record_flag=0 (derived from the
        // Option) and an exact round-trip of the flag byte.
        let mut d = base;
        d.metadata_locator_record_flag = true;
        d.mpeg_carriage_flags = MpegCarriageFlags::None;
        let mut buf = vec![0u8; d.serialized_len()];
        d.serialize_into(&mut buf).unwrap();
        let flags_byte = buf[buf.len() - 1 - d.private_data.len() - 1];
        assert_eq!(flags_byte & 0x80, 0, "flag must be derived, not stored");
        let re = MetadataPointerDescriptor::parse(&buf).unwrap();
        assert!(!re.metadata_locator_record_flag);
    }

    /// H.222.0 Table 2-86. `reserved(5)` is preserved verbatim (zero-reserved
    /// senders are no longer normalized to `11111`).
    #[test]
    fn metadata_pointer_reserved_preserved() {
        for r in 0u8..=31 {
            let bytes = [TAG, 6, 0x00, 0x01, 0x3F, 0x02, 0x60 | r, 0xEE];
            let d = MetadataPointerDescriptor::parse(&bytes).unwrap();
            assert_eq!(d.reserved, ReservedOnes::from_bits(u64::from(r)));
            let mut out = vec![0u8; d.serialized_len()];
            d.serialize_into(&mut out).unwrap();
            assert_eq!(out, bytes);
        }
    }

    #[test]
    fn metadata_pointer_default_is_spec_value() {
        let mut d = MetadataPointerDescriptor::new(0x0001, MetadataFormat::Reserved1(0x3F), 2);
        d.private_data = &[0xEE];
        let mut out = vec![0u8; d.serialized_len()];
        d.serialize_into(&mut out).unwrap();
        assert_eq!(out, [TAG, 6, 0x00, 0x01, 0x3F, 0x02, 0x7F, 0xEE]);
    }

    /// The field is 5 bits wide, so a wider value is unrepresentable and can
    /// never spill into `MPEG_carriage_flags` above it.
    #[test]
    fn reserved_cannot_hold_more_than_five_bits() {
        let mut d = MetadataPointerDescriptor::new(0x0001, MetadataFormat::Reserved1(0x3F), 2);
        d.reserved = ReservedOnes::from_bits(0xFF);
        assert_eq!(d.reserved.get(), 0x1F);
        let mut out = vec![0u8; d.serialized_len()];
        d.serialize_into(&mut out).unwrap();
        assert_eq!(out[6], 0x7F);
    }

    /// Both explicit-identifier forms, with identifier bytes distinct from the
    /// surrounding format and flags bytes.
    #[test]
    fn explicit_identifiers_parse_and_round_trip() {
        let bytes = [
            TAG, 11, 0xFF, 0xFF, 0x01, 0x02, 0x03, 0x04, 0x3F, 0x05, 0x00, 0x12, 0x34,
        ];
        let d = MetadataPointerDescriptor::parse(&bytes).unwrap();
        assert_eq!(d.metadata_application_format_identifier, Some(0x0102_0304));
        assert_eq!(d.program_number, Some(0x1234));
        let mut out = vec![0u8; d.serialized_len()];
        d.serialize_into(&mut out).unwrap();
        assert_eq!(out, bytes);

        let bytes = [
            TAG, 11, 0x00, 0x01, 0xFF, 0x0A, 0x0B, 0x0C, 0x0D, 0x05, 0x00, 0x12, 0x34,
        ];
        let d = MetadataPointerDescriptor::parse(&bytes).unwrap();
        assert_eq!(d.metadata_format, MetadataFormat::Identifier);
        assert_eq!(d.metadata_format_identifier, Some(0x0A0B_0C0D));
        assert_eq!(d.program_number, Some(0x1234));
        let mut out = vec![0u8; d.serialized_len()];
        d.serialize_into(&mut out).unwrap();
        assert_eq!(out, bytes);
    }

    /// Bodies truncated before the identifier, the format byte or the flags
    /// byte are errors, never an out-of-bounds read.
    #[test]
    fn truncated_forms_are_errors() {
        for bytes in [
            &[TAG, 5, 0xFF, 0xFF, 0x01, 0x02, 0x03][..],
            &[TAG, 2, 0x00, 0x01][..],
            &[TAG, 5, 0x00, 0x01, 0xFF, 0x01, 0x02][..],
            &[TAG, 4, 0x00, 0x01, 0x3F, 0x07][..],
        ] {
            assert!(matches!(
                MetadataPointerDescriptor::parse(bytes).unwrap_err(),
                Error::InvalidDescriptor { .. }
            ));
        }
        // The identifier is complete in each of these; the failing field is
        // the one named in the reason, not an earlier one.
        assert_eq!(
            MetadataPointerDescriptor::parse(&[TAG, 6, 0xFF, 0xFF, 0x01, 0x02, 0x03, 0x04])
                .unwrap_err(),
            Error::InvalidDescriptor {
                tag: TAG,
                reason: "metadata_pointer_descriptor too short for metadata_format"
            }
        );
        assert_eq!(
            MetadataPointerDescriptor::parse(&[TAG, 7, 0xFF, 0xFF, 0x01, 0x02, 0x03, 0x04, 0x05])
                .unwrap_err(),
            Error::InvalidDescriptor {
                tag: TAG,
                reason: "metadata_pointer_descriptor too short for service_id + flags"
            }
        );
        assert_eq!(
            MetadataPointerDescriptor::parse(&[TAG, 7, 0x00, 0x01, 0xFF, 0x01, 0x02, 0x03, 0x04])
                .unwrap_err(),
            Error::InvalidDescriptor {
                tag: TAG,
                reason: "metadata_pointer_descriptor too short for service_id + flags"
            }
        );
    }

    /// A locator record that is absent (length 0) or runs exactly to the end
    /// of the body is valid; carriage flags 3 leave no program number.
    #[test]
    fn locator_record_ending_at_the_body_boundary_is_valid() {
        let bytes = [TAG, 6, 0x00, 0x01, 0x3F, 0x05, 0xE0, 0x00];
        let d = MetadataPointerDescriptor::parse(&bytes).unwrap();
        assert_eq!(d.metadata_locator_record, Some(&[][..]));
        assert_eq!(d.mpeg_carriage_flags, MpegCarriageFlags::None);
        assert!(d.private_data.is_empty());
        let mut out = vec![0u8; d.serialized_len()];
        d.serialize_into(&mut out).unwrap();
        assert_eq!(out, bytes);

        let bytes = [TAG, 8, 0x00, 0x01, 0x3F, 0x05, 0xE0, 0x02, 0xAA, 0xBB];
        let d = MetadataPointerDescriptor::parse(&bytes).unwrap();
        assert_eq!(d.metadata_locator_record, Some(&[0xAA, 0xBB][..]));
        assert!(d.private_data.is_empty());
        let mut out = vec![0u8; d.serialized_len()];
        d.serialize_into(&mut out).unwrap();
        assert_eq!(out, bytes);
    }
}
