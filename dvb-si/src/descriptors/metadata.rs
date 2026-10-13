//! Metadata Descriptor — ISO/IEC 13818-1 §2.6.60, Table 2-89 (tag 0x26).
//!
//! Carries metadata for a program or elementary stream. The DSM-CC block is
//! conditional on `DSM-CC_flag`; the decoder-config block is one of several
//! variants selected by `decoder_config_flags` (001/011/100/101|110).

use super::descriptor_body;
use crate::descriptors::decoder_config_flags::DecoderConfigFlags;
use crate::descriptors::metadata_format::MetadataFormat;
use crate::error::{Error, Result};
use broadcast_common::{Parse, Serialize};

/// Descriptor tag for metadata_descriptor.
pub const TAG: u8 = 0x26;
const HEADER_LEN: usize = 2;
/// `DSM-CC_flag` bit of the `decoder_config_flags` byte (Table 2-89).
const DSMCC_FLAG_BIT: u8 = 0x10;
/// Default of the 4-bit `reserved` tail of the flags byte (H.222.0 Table
/// 2-89): clause 2.1 "reserved" = `1` (all editions 2006-08/2023, audit §5; no
/// fixture carries the descriptor).
pub const RESERVED_DEFAULT: u8 = 0b1111;
const RESERVED_MASK: u8 = 0x0F;

/// Decoder configuration carried in this descriptor (decoder_config_flags == 0b001).
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct DecoderConfigInDescriptor<'a> {
    /// Opaque decoder config bytes.
    #[cfg_attr(feature = "serde", serde(borrow))]
    pub decoder_config: &'a [u8],
}

/// Decoder configuration carried in a DSM-CC carousel (decoder_config_flags == 0b011).
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct DecoderConfigDsmcc<'a> {
    /// Opaque dec_config_identification_record bytes.
    #[cfg_attr(feature = "serde", serde(borrow))]
    pub dec_config_identification: &'a [u8],
}

/// Decoder configuration carried in another metadata service (decoder_config_flags == 0b100).
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct DecoderConfigOtherService {
    /// Decoder config metadata service ID.
    pub decoder_config_metadata_service_id: u8,
}

/// Reserved data block (decoder_config_flags == 0b101 or 0b110).
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct ReservedData<'a> {
    /// Opaque reserved data bytes.
    #[cfg_attr(feature = "serde", serde(borrow))]
    pub data: &'a [u8],
}

/// All decoder-config variants, selected by decoder_config_flags.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub enum DecoderConfig<'a> {
    /// No decoder config needed (0b000).
    None,
    /// Carried in this descriptor (0b001).
    InDescriptor(DecoderConfigInDescriptor<'a>),
    /// Carried in same metadata service (0b010, no data).
    SameService,
    /// Carried in DSM-CC carousel (0b011).
    DsmccCarousel(DecoderConfigDsmcc<'a>),
    /// Carried in another metadata service (0b100).
    OtherService(DecoderConfigOtherService),
    /// Reserved data (0b101|0b110).
    ReservedData(ReservedData<'a>),
    /// Privately defined (0b111, no data).
    Private,
}

/// Metadata Descriptor.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[cfg_attr(feature = "yoke", derive(yoke::Yokeable))]
#[non_exhaustive]
pub struct MetadataDescriptor<'a> {
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
    /// Decoder config flags (Table 2-90, 3-bit).
    pub decoder_config_flags: DecoderConfigFlags,
    /// DSM-CC service identification flag, as parsed from the wire.
    ///
    /// Informational only: [`Serialize`] derives `DSM-CC_flag` from
    /// `service_identification.is_some()`, so this field can never make
    /// the serializer emit a flag without its record (or a record without
    /// its flag) — r03-W5. Parse sets it equal to that derivation.
    pub dsmcc_flag: bool,
    /// DSM-CC service identification — present when `dsmcc_flag` is true.
    #[cfg_attr(feature = "serde", serde(borrow))]
    pub service_identification: Option<&'a [u8]>,
    /// Decoder configuration block.
    pub decoder_config: DecoderConfig<'a>,
    /// Trailing private data bytes.
    #[cfg_attr(feature = "serde", serde(borrow))]
    pub private_data: &'a [u8],
    /// `reserved(4)` tail of the flags byte (4 bits, right-aligned). Captured
    /// on parse, re-emitted verbatim; [`MetadataDescriptor::new`] / [`Default`]
    /// use [`RESERVED_DEFAULT`].
    pub reserved: u8,
}

impl<'a> MetadataDescriptor<'a> {
    /// Build a minimal descriptor (no identifiers, no DSM-CC record, no decoder
    /// config, no private data) with the spec-default reserved bits. Set the
    /// remaining fields directly.
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
            decoder_config_flags: DecoderConfigFlags::None,
            dsmcc_flag: false,
            service_identification: None,
            decoder_config: DecoderConfig::None,
            private_data: &[],
            reserved: RESERVED_DEFAULT,
        }
    }
}

impl<'a> Parse<'a> for MetadataDescriptor<'a> {
    type Error = crate::error::Error;

    fn parse(bytes: &'a [u8]) -> Result<Self> {
        let body = descriptor_body(
            bytes,
            TAG,
            "MetadataDescriptor",
            "unexpected tag for metadata_descriptor",
        )?;

        // Minimum: metadata_application_format(2) + metadata_format(1) + metadata_service_id(1) + flags(1) = 5
        if body.len() < 5 {
            return Err(Error::InvalidDescriptor {
                tag: TAG,
                reason: "metadata_descriptor too short (< 5 body bytes)",
            });
        }

        let metadata_application_format = u16::from_be_bytes([body[0], body[1]]);
        let mut pos = 2;

        let metadata_application_format_identifier = if metadata_application_format == 0xFFFF {
            if body.len() < pos + 4 {
                return Err(Error::InvalidDescriptor {
                    tag: TAG,
                    reason: "metadata_descriptor too short for metadata_application_format_identifier",
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
                reason: "metadata_descriptor too short for metadata_format",
            });
        }
        let metadata_format = MetadataFormat::from_u8(body[pos]);
        pos += 1;

        let metadata_format_identifier = if body[pos - 1] == 0xFF {
            if body.len() < pos + 4 {
                return Err(Error::InvalidDescriptor {
                    tag: TAG,
                    reason: "metadata_descriptor too short for metadata_format_identifier",
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
                reason: "metadata_descriptor too short for service_id + flags",
            });
        }
        let metadata_service_id = body[pos];
        pos += 1;

        let flags = body[pos];
        let decoder_config_raw = (flags & 0xE0) >> 5;
        let decoder_config_flags = DecoderConfigFlags::from_u8(decoder_config_raw);
        let dsmcc_flag = (flags & 0x10) != 0;
        let reserved = flags & RESERVED_MASK;
        pos += 1;

        let (service_identification, mut pos) = if dsmcc_flag {
            if body.len() < pos + 1 {
                return Err(Error::InvalidDescriptor {
                    tag: TAG,
                    reason: "metadata_descriptor too short for service_identification_length",
                });
            }
            let si_len = body[pos] as usize;
            pos += 1;
            if body.len() < pos + si_len {
                return Err(Error::InvalidDescriptor {
                    tag: TAG,
                    reason: "metadata_descriptor too short for service_identification_record",
                });
            }
            let si = &body[pos..pos + si_len];
            pos += si_len;
            (Some(si), pos)
        } else {
            (None, pos)
        };

        let decoder_config = match decoder_config_raw {
            0 => DecoderConfig::None,
            1 => {
                if body.len() < pos + 1 {
                    return Err(Error::InvalidDescriptor {
                        tag: TAG,
                        reason: "metadata_descriptor too short for decoder_config_length",
                    });
                }
                let dc_len = body[pos] as usize;
                pos += 1;
                if body.len() < pos + dc_len {
                    return Err(Error::InvalidDescriptor {
                        tag: TAG,
                        reason: "metadata_descriptor too short for decoder_config bytes",
                    });
                }
                let dc = &body[pos..pos + dc_len];
                pos += dc_len;
                DecoderConfig::InDescriptor(DecoderConfigInDescriptor { decoder_config: dc })
            }
            2 => DecoderConfig::SameService,
            3 => {
                if body.len() < pos + 1 {
                    return Err(Error::InvalidDescriptor {
                        tag: TAG,
                        reason: "metadata_descriptor too short for dec_config_identification_record_length",
                    });
                }
                let dci_len = body[pos] as usize;
                pos += 1;
                if body.len() < pos + dci_len {
                    return Err(Error::InvalidDescriptor {
                        tag: TAG,
                        reason: "metadata_descriptor too short for dec_config_identification_record",
                    });
                }
                let dci = &body[pos..pos + dci_len];
                pos += dci_len;
                DecoderConfig::DsmccCarousel(DecoderConfigDsmcc {
                    dec_config_identification: dci,
                })
            }
            4 => {
                if body.len() < pos + 1 {
                    return Err(Error::InvalidDescriptor {
                        tag: TAG,
                        reason: "metadata_descriptor too short for decoder_config_metadata_service_id",
                    });
                }
                let dcm = body[pos];
                pos += 1;
                DecoderConfig::OtherService(DecoderConfigOtherService {
                    decoder_config_metadata_service_id: dcm,
                })
            }
            5 | 6 => {
                if body.len() < pos + 1 {
                    return Err(Error::InvalidDescriptor {
                        tag: TAG,
                        reason: "metadata_descriptor too short for reserved_data_length",
                    });
                }
                let rd_len = body[pos] as usize;
                pos += 1;
                if body.len() < pos + rd_len {
                    return Err(Error::InvalidDescriptor {
                        tag: TAG,
                        reason: "metadata_descriptor too short for reserved_data",
                    });
                }
                let rd = &body[pos..pos + rd_len];
                pos += rd_len;
                DecoderConfig::ReservedData(ReservedData { data: rd })
            }
            _ => DecoderConfig::Private,
        };

        let private_data = &body[pos..];

        Ok(Self {
            metadata_application_format,
            metadata_application_format_identifier,
            metadata_format,
            metadata_format_identifier,
            metadata_service_id,
            decoder_config_flags,
            dsmcc_flag,
            service_identification,
            decoder_config,
            private_data,
            reserved,
        })
    }
}

impl Serialize for MetadataDescriptor<'_> {
    type Error = crate::error::Error;

    fn serialized_len(&self) -> usize {
        let mut len: usize = HEADER_LEN + 5; // metadata_application_format(2) + metadata_format(1) + service_id(1) + flags(1)
        if self.metadata_application_format_identifier.is_some() {
            len += 4;
        }
        if self.metadata_format_identifier.is_some() {
            len += 4;
        }
        if let Some(si) = self.service_identification {
            len += 1 + si.len();
        }
        match &self.decoder_config {
            DecoderConfig::InDescriptor(d) => len += 1 + d.decoder_config.len(),
            DecoderConfig::DsmccCarousel(d) => len += 1 + d.dec_config_identification.len(),
            DecoderConfig::OtherService(_) => len += 1,
            DecoderConfig::ReservedData(d) => len += 1 + d.data.len(),
            DecoderConfig::None | DecoderConfig::SameService | DecoderConfig::Private => {}
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
        crate::descriptors::write_descriptor_header(buf, TAG, len - HEADER_LEN)?;

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

        // `DSM-CC_flag` is derived from the Option (never the stored
        // `dsmcc_flag`) so flag and record can never disagree (r03-W5).
        let mut flags =
            ((self.decoder_config_flags.to_u8() & 0x07) << 5) | (self.reserved & RESERVED_MASK);
        if self.service_identification.is_some() {
            flags |= DSMCC_FLAG_BIT;
        }
        buf[pos] = flags;
        pos += 1;

        if let Some(si) = self.service_identification {
            buf[pos] = broadcast_common::len::fit_u8(si.len(), "service_identification_length")?;
            pos += 1;
            buf[pos..pos + si.len()].copy_from_slice(si);
            pos += si.len();
        }

        match &self.decoder_config {
            DecoderConfig::InDescriptor(d) => {
                buf[pos] =
                    broadcast_common::len::fit_u8(d.decoder_config.len(), "decoder_config_length")?;
                pos += 1;
                buf[pos..pos + d.decoder_config.len()].copy_from_slice(d.decoder_config);
                pos += d.decoder_config.len();
            }
            DecoderConfig::DsmccCarousel(d) => {
                buf[pos] = broadcast_common::len::fit_u8(
                    d.dec_config_identification.len(),
                    "dec_config_identification_length",
                )?;
                pos += 1;
                buf[pos..pos + d.dec_config_identification.len()]
                    .copy_from_slice(d.dec_config_identification);
                pos += d.dec_config_identification.len();
            }
            DecoderConfig::OtherService(d) => {
                buf[pos] = d.decoder_config_metadata_service_id;
                pos += 1;
            }
            DecoderConfig::ReservedData(d) => {
                buf[pos] = broadcast_common::len::fit_u8(d.data.len(), "private_data_byte_length")?;
                pos += 1;
                buf[pos..pos + d.data.len()].copy_from_slice(d.data);
                pos += d.data.len();
            }
            DecoderConfig::None | DecoderConfig::SameService | DecoderConfig::Private => {}
        }

        buf[pos..pos + self.private_data.len()].copy_from_slice(self.private_data);
        Ok(len)
    }
}

impl<'a> crate::traits::DescriptorDef<'a> for MetadataDescriptor<'a> {
    const TAG: u8 = TAG;
    const NAME: &'static str = "METADATA";
}

#[cfg(test)]
mod tests {
    use super::*;

    fn serialize_round_trip(d: &MetadataDescriptor<'_>) {
        let mut buf = vec![0u8; d.serialized_len()];
        let written = d.serialize_into(&mut buf).unwrap();
        assert_eq!(written, d.serialized_len());
        let reparsed = MetadataDescriptor::parse(&buf).unwrap();
        assert_eq!(*d, reparsed, "round-trip mismatch");
    }

    #[test]
    fn round_trip_decoder_config_none() {
        let d = MetadataDescriptor {
            metadata_application_format: 0x0010,
            metadata_application_format_identifier: None,
            metadata_format: MetadataFormat::TeM,
            metadata_format_identifier: None,
            metadata_service_id: 5,
            decoder_config_flags: DecoderConfigFlags::None,
            dsmcc_flag: false,
            service_identification: None,
            decoder_config: DecoderConfig::None,
            reserved: RESERVED_DEFAULT,
            private_data: &[],
        };
        serialize_round_trip(&d);
    }

    #[test]
    fn round_trip_decoder_config_in_descriptor() {
        let d = MetadataDescriptor {
            metadata_application_format: 0x0011,
            metadata_application_format_identifier: None,
            metadata_format: MetadataFormat::BiM,
            metadata_format_identifier: None,
            metadata_service_id: 3,
            decoder_config_flags: DecoderConfigFlags::InDescriptor,
            dsmcc_flag: false,
            service_identification: None,
            decoder_config: DecoderConfig::InDescriptor(DecoderConfigInDescriptor {
                decoder_config: &[0x01, 0x02, 0x03],
            }),
            reserved: RESERVED_DEFAULT,
            private_data: &[0xFF],
        };
        serialize_round_trip(&d);
    }

    #[test]
    fn round_trip_decoder_config_same_service() {
        let d = MetadataDescriptor {
            metadata_application_format: 0x0100,
            metadata_application_format_identifier: None,
            metadata_format: MetadataFormat::AppFormat,
            metadata_format_identifier: None,
            metadata_service_id: 1,
            decoder_config_flags: DecoderConfigFlags::SameService,
            dsmcc_flag: true,
            service_identification: Some(&[0xAA, 0xBB]),
            decoder_config: DecoderConfig::SameService,
            reserved: RESERVED_DEFAULT,
            private_data: &[],
        };
        serialize_round_trip(&d);
    }

    #[test]
    fn round_trip_decoder_config_dsmcc() {
        let d = MetadataDescriptor {
            metadata_application_format: 0x0010,
            metadata_application_format_identifier: None,
            metadata_format: MetadataFormat::Reserved1(0x30),
            metadata_format_identifier: None,
            metadata_service_id: 7,
            decoder_config_flags: DecoderConfigFlags::DsmccCarousel,
            dsmcc_flag: false,
            service_identification: None,
            decoder_config: DecoderConfig::DsmccCarousel(DecoderConfigDsmcc {
                dec_config_identification: &[0x11, 0x22, 0x33, 0x44],
            }),
            reserved: RESERVED_DEFAULT,
            private_data: &[],
        };
        serialize_round_trip(&d);
    }

    #[test]
    fn round_trip_decoder_config_other_service() {
        let d = MetadataDescriptor {
            metadata_application_format: 0x0010,
            metadata_application_format_identifier: None,
            metadata_format: MetadataFormat::Private(0x50),
            metadata_format_identifier: None,
            metadata_service_id: 2,
            decoder_config_flags: DecoderConfigFlags::OtherService,
            dsmcc_flag: false,
            service_identification: None,
            decoder_config: DecoderConfig::OtherService(DecoderConfigOtherService {
                decoder_config_metadata_service_id: 99,
            }),
            reserved: RESERVED_DEFAULT,
            private_data: &[],
        };
        serialize_round_trip(&d);
    }

    #[test]
    fn round_trip_decoder_config_reserved_data() {
        let d = MetadataDescriptor {
            metadata_application_format: 0x0010,
            metadata_application_format_identifier: None,
            metadata_format: MetadataFormat::Identifier,
            metadata_format_identifier: Some(0xDEADBEEF),
            metadata_service_id: 4,
            decoder_config_flags: DecoderConfigFlags::Reserved(5),
            dsmcc_flag: false,
            service_identification: None,
            decoder_config: DecoderConfig::ReservedData(ReservedData {
                data: &[0x99, 0x88, 0x77],
            }),
            reserved: RESERVED_DEFAULT,
            private_data: &[],
        };
        serialize_round_trip(&d);
    }

    #[test]
    fn round_trip_decoder_config_private() {
        let d = MetadataDescriptor {
            metadata_application_format: 0x0010,
            metadata_application_format_identifier: None,
            metadata_format: MetadataFormat::TeM,
            metadata_format_identifier: None,
            metadata_service_id: 6,
            decoder_config_flags: DecoderConfigFlags::Private,
            dsmcc_flag: false,
            service_identification: None,
            decoder_config: DecoderConfig::Private,
            reserved: RESERVED_DEFAULT,
            private_data: &[0x01],
        };
        serialize_round_trip(&d);
    }

    #[test]
    fn round_trip_with_ffff_and_ff() {
        let d = MetadataDescriptor {
            metadata_application_format: 0xFFFF,
            metadata_application_format_identifier: Some(0x11223344),
            metadata_format: MetadataFormat::Identifier,
            metadata_format_identifier: Some(0x55667788),
            metadata_service_id: 8,
            decoder_config_flags: DecoderConfigFlags::None,
            dsmcc_flag: true,
            service_identification: Some(&[0xCA, 0xFE]),
            decoder_config: DecoderConfig::None,
            reserved: RESERVED_DEFAULT,
            private_data: &[],
        };
        serialize_round_trip(&d);
    }

    fn base<'a>() -> MetadataDescriptor<'a> {
        MetadataDescriptor {
            metadata_application_format: 0x0100,
            metadata_application_format_identifier: None,
            metadata_format: MetadataFormat::AppFormat,
            metadata_format_identifier: None,
            metadata_service_id: 1,
            decoder_config_flags: DecoderConfigFlags::None,
            dsmcc_flag: false,
            service_identification: None,
            decoder_config: DecoderConfig::None,
            reserved: RESERVED_DEFAULT,
            private_data: &[],
        }
    }

    /// r03-W5: a stored `dsmcc_flag` that disagrees with
    /// `service_identification` must never misframe the descriptor — the
    /// wire flag is derived from the Option.
    #[test]
    fn serialize_derives_dsmcc_flag_from_option() {
        // flag=true, no record: previously wrote the flag with no length byte.
        let mut d = base();
        d.dsmcc_flag = true;
        let mut buf = vec![0u8; d.serialized_len()];
        d.serialize_into(&mut buf).unwrap();
        let re = MetadataDescriptor::parse(&buf).unwrap();
        assert!(!re.dsmcc_flag);
        assert!(re.service_identification.is_none());
        assert_eq!(buf[buf.len() - 1] & DSMCC_FLAG_BIT, 0);
        // flag=false, record present: previously wrote bytes with no flag.
        let mut d = base();
        d.service_identification = Some(&[0xAA, 0xBB]);
        let mut buf = vec![0u8; d.serialized_len()];
        d.serialize_into(&mut buf).unwrap();
        let re = MetadataDescriptor::parse(&buf).unwrap();
        assert!(re.dsmcc_flag);
        assert_eq!(re.service_identification, Some(&[0xAA, 0xBB][..]));
    }

    #[test]
    fn parse_rejects_wrong_tag() {
        let err = MetadataDescriptor::parse(&[0x02, 5, 0, 0, 0, 0, 0]).unwrap_err();
        assert!(matches!(err, Error::InvalidDescriptor { tag: 0x02, .. }));
    }

    #[test]
    fn parse_rejects_too_short() {
        let err = MetadataDescriptor::parse(&[TAG, 0]).unwrap_err();
        assert!(matches!(err, Error::InvalidDescriptor { tag: TAG, .. }));
    }

    /// H.222.0 Table 2-89. `reserved(4)` is preserved verbatim (zero-reserved
    /// senders are no longer normalized to `1111`).
    #[test]
    fn metadata_reserved_preserved() {
        for r in 0u8..=15 {
            let bytes = [
                TAG,
                9,
                0x01,
                0x00,
                0x3F,
                0x01,
                0x10 | r,
                0x02,
                0xAA,
                0xBB,
                0xEE,
            ];
            let d = MetadataDescriptor::parse(&bytes).unwrap();
            assert_eq!(d.reserved, r);
            let mut out = vec![0u8; d.serialized_len()];
            d.serialize_into(&mut out).unwrap();
            assert_eq!(out, bytes);
        }
    }

    #[test]
    fn metadata_default_is_spec_value() {
        let mut d = MetadataDescriptor::new(0x0100, MetadataFormat::Reserved1(0x3F), 1);
        d.service_identification = Some(&[0xAA, 0xBB]);
        d.private_data = &[0xEE];
        let mut out = vec![0u8; d.serialized_len()];
        d.serialize_into(&mut out).unwrap();
        assert_eq!(
            out,
            [TAG, 9, 0x01, 0x00, 0x3F, 0x01, 0x1F, 0x02, 0xAA, 0xBB, 0xEE]
        );
    }
}
