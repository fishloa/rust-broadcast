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
use dvb_common::{Parse, Serialize};

/// Descriptor tag for metadata_pointer_descriptor.
pub const TAG: u8 = 0x25;
const HEADER_LEN: usize = 2;

/// Metadata Pointer Descriptor.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[cfg_attr(feature = "yoke", derive(yoke::Yokeable))]
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
        if body.len() < 5 {
            return Err(Error::InvalidDescriptor {
                tag: TAG,
                reason: "metadata_pointer_descriptor too short (< 5 body bytes)",
            });
        }

        let metadata_application_format = u16::from_be_bytes([body[0], body[1]]);
        let mut pos = 2;

        let metadata_application_format_identifier = if metadata_application_format == 0xFFFF {
            if body.len() < pos + 4 {
                return Err(Error::InvalidDescriptor {
                        tag: TAG,
                        reason: "metadata_pointer_descriptor too short for metadata_application_format_identifier",
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
                reason: "metadata_pointer_descriptor too short for metadata_format",
            });
        }
        let metadata_format = MetadataFormat::from_u8(body[pos]);
        pos += 1;

        let metadata_format_identifier = if body[pos - 1] == 0xFF {
            if body.len() < pos + 4 {
                return Err(Error::InvalidDescriptor {
                    tag: TAG,
                    reason: "metadata_pointer_descriptor too short for metadata_format_identifier",
                });
            }
            let id = u32::from_be_bytes([body[pos], body[pos + 1], body[pos + 2], body[pos + 3]]);
            pos += 4;
            Some(id)
        } else {
            None
        };

        if body.len() < pos + 2 {
            return Err(Error::InvalidDescriptor {
                tag: TAG,
                reason: "metadata_pointer_descriptor too short for service_id + flags",
            });
        }
        let metadata_service_id = body[pos];
        pos += 1;

        let flags = body[pos];
        let metadata_locator_record_flag = (flags & 0x80) != 0;
        let mpeg_carriage_flags = MpegCarriageFlags::from_u8((flags >> 5) & 0x03);
        pos += 1;

        let (metadata_locator_record, mut pos) = if metadata_locator_record_flag {
            if body.len() < pos + 1 {
                return Err(Error::InvalidDescriptor {
                    tag: TAG,
                    reason:
                        "metadata_pointer_descriptor too short for metadata_locator_record_length",
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

        let program_number = if carriage_val <= 2 {
            if body.len() < pos + 2 {
                return Err(Error::InvalidDescriptor {
                    tag: TAG,
                    reason: "metadata_pointer_descriptor too short for program_number",
                });
            }
            let pn = u16::from_be_bytes([body[pos], body[pos + 1]]);
            pos += 2;
            Some(pn)
        } else {
            None
        };

        let (transport_stream_location, transport_stream_id, pos) = if carriage_val == 1 {
            if body.len() < pos + 4 {
                return Err(Error::InvalidDescriptor {
                    tag: TAG,
                    reason:
                        "metadata_pointer_descriptor too short for transport_stream_location+id",
                });
            }
            let tsl = u16::from_be_bytes([body[pos], body[pos + 1]]);
            let tsi = u16::from_be_bytes([body[pos + 2], body[pos + 3]]);
            pos += 4;
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
        })
    }
}

impl Serialize for MetadataPointerDescriptor<'_> {
    type Error = crate::error::Error;

    fn serialized_len(&self) -> usize {
        let mut len: usize = HEADER_LEN + 5; // metadata_application_format(2) + metadata_format(1) + service_id(1) + flags(1)
        if self.metadata_application_format_identifier.is_some() {
            len += 4;
        }
        if self.metadata_format_identifier.is_some() {
            len += 4;
        }
        if let Some(rec) = self.metadata_locator_record {
            len += 1 + rec.len();
        }
        if self.program_number.is_some() {
            len += 2;
        }
        if self.transport_stream_location.is_some() {
            len += 4;
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

        buf[pos] = self.metadata_format.to_u8();
        pos += 1;

        if let Some(id) = self.metadata_format_identifier {
            buf[pos..pos + 4].copy_from_slice(&id.to_be_bytes());
            pos += 4;
        }

        buf[pos] = self.metadata_service_id;
        pos += 1;

        let mut flags = (self.mpeg_carriage_flags.to_u8() & 0x03) << 5;
        if self.metadata_locator_record_flag {
            flags |= 0x80;
        }
        buf[pos] = flags;
        pos += 1;

        if let Some(rec) = self.metadata_locator_record {
            buf[pos] = rec.len() as u8;
            pos += 1;
            buf[pos..pos + rec.len()].copy_from_slice(rec);
            pos += rec.len();
        }

        if let Some(pn) = self.program_number {
            buf[pos..pos + 2].copy_from_slice(&pn.to_be_bytes());
            pos += 2;
        }
        if let Some(tsl) = self.transport_stream_location {
            buf[pos..pos + 2].copy_from_slice(&tsl.to_be_bytes());
            buf[pos + 2..pos + 4]
                .copy_from_slice(&self.transport_stream_id.unwrap_or(0).to_be_bytes());
            pos += 4;
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

    fn serialize_round_trip(d: &MetadataPointerDescriptor<'_>) {
        let mut buf = vec![0u8; d.serialized_len()];
        let written = d.serialize_into(&mut buf).unwrap();
        assert_eq!(written, d.serialized_len());
        let reparsed = MetadataPointerDescriptor::parse(&buf).unwrap();
        assert_eq!(*d, reparsed, "round-trip mismatch");
    }

    #[test]
    fn round_trip_carriage_0_same_ts() {
        let d = MetadataPointerDescriptor {
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
            private_data: &[],
        };
        serialize_round_trip(&d);
    }

    #[test]
    fn round_trip_carriage_1_different_ts() {
        let d = MetadataPointerDescriptor {
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
            private_data: &[],
        };
        serialize_round_trip(&d);
    }

    #[test]
    fn round_trip_carriage_2_program_stream() {
        let d = MetadataPointerDescriptor {
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
            private_data: &[],
        };
        serialize_round_trip(&d);
    }

    #[test]
    fn round_trip_carriage_3_none() {
        let d = MetadataPointerDescriptor {
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
            private_data: &[0xCC],
        };
        serialize_round_trip(&d);
    }

    #[test]
    fn round_trip_with_ffff_and_ff_identifiers() {
        let d = MetadataPointerDescriptor {
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
            private_data: &[0xFF],
        };
        serialize_round_trip(&d);
    }

    #[test]
    fn round_trip_all_empty_private() {
        let d = MetadataPointerDescriptor {
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
            private_data: &[],
        };
        serialize_round_trip(&d);
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
}
