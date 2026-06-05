//! Service List Descriptor — ETSI EN 300 468 §6.2.36 (tag 0x41).
//!
//! Carried inside NIT/BAT transport_stream_loop second descriptor loop.
//! Enumerates the service_id/service_type pairs available on the TS.

use crate::error::{Error, Result};
use crate::traits::Descriptor;
use dvb_common::{Parse, Serialize};

/// Descriptor tag for service_list_descriptor.
pub const TAG: u8 = 0x41;
const HEADER_LEN: usize = 2;
const ENTRY_LEN: usize = 3;

/// One (service_id, service_type) pair.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct ServiceListEntry {
    /// DVB service_id (matches program_number in PAT).
    pub service_id: u16,
    /// service_type byte (ETSI Table 87).
    pub service_type: u8,
}

/// Service List Descriptor.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct ServiceListDescriptor {
    /// Entries in wire order.
    pub entries: Vec<ServiceListEntry>,
}

impl<'a> Parse<'a> for ServiceListDescriptor {
    type Error = crate::error::Error;
    fn parse(bytes: &'a [u8]) -> Result<Self> {
        if bytes.len() < HEADER_LEN {
            return Err(Error::BufferTooShort {
                need: HEADER_LEN,
                have: bytes.len(),
                what: "ServiceListDescriptor header",
            });
        }
        if bytes[0] != TAG {
            return Err(Error::InvalidDescriptor {
                tag: bytes[0],
                reason: "unexpected tag for service_list_descriptor",
            });
        }
        let length = bytes[1] as usize;
        if length % 3 != 0 {
            return Err(Error::InvalidDescriptor {
                tag: TAG,
                reason: "descriptor_length must be a multiple of 3",
            });
        }
        let body_start = HEADER_LEN;
        let body_end = body_start + length;
        if bytes.len() < body_end {
            return Err(Error::BufferTooShort {
                need: body_end,
                have: bytes.len(),
                what: "ServiceListDescriptor body",
            });
        }
        let mut entries = Vec::with_capacity(length / ENTRY_LEN);
        let mut offset = body_start;
        while offset < body_end {
            let service_id = u16::from_be_bytes([bytes[offset], bytes[offset + 1]]);
            let service_type = bytes[offset + 2];
            entries.push(ServiceListEntry {
                service_id,
                service_type,
            });
            offset += ENTRY_LEN;
        }
        Ok(Self { entries })
    }
}

impl Serialize for ServiceListDescriptor {
    type Error = crate::error::Error;
    fn serialized_len(&self) -> usize {
        HEADER_LEN + ENTRY_LEN * self.entries.len()
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
        let mut offset = HEADER_LEN;
        for entry in &self.entries {
            buf[offset..offset + 2].copy_from_slice(&entry.service_id.to_be_bytes());
            buf[offset + 2] = entry.service_type;
            offset += ENTRY_LEN;
        }
        Ok(len)
    }
}

impl<'a> Descriptor<'a> for ServiceListDescriptor {
    const TAG: u8 = TAG;
    fn descriptor_length(&self) -> u8 {
        (self.serialized_len() - HEADER_LEN) as u8
    }
}

impl<'a> crate::traits::DescriptorDef<'a> for ServiceListDescriptor {
    const TAG: u8 = TAG;
    const NAME: &'static str = "SERVICE_LIST";
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_single_entry() {
        let bytes = [TAG, 3, 0x00, 0x01, 0x01];
        let d = ServiceListDescriptor::parse(&bytes).unwrap();
        assert_eq!(d.entries.len(), 1);
        assert_eq!(d.entries[0].service_id, 1);
        assert_eq!(d.entries[0].service_type, 0x01);
    }

    #[test]
    fn parse_multiple_entries_preserves_order() {
        let bytes = [TAG, 9, 0x00, 0x01, 0x01, 0x00, 0x02, 0x02, 0x00, 0x03, 0x03];
        let d = ServiceListDescriptor::parse(&bytes).unwrap();
        assert_eq!(d.entries.len(), 3);
        assert_eq!(d.entries[0].service_id, 1);
        assert_eq!(d.entries[0].service_type, 0x01);
        assert_eq!(d.entries[1].service_id, 2);
        assert_eq!(d.entries[1].service_type, 0x02);
        assert_eq!(d.entries[2].service_id, 3);
        assert_eq!(d.entries[2].service_type, 0x03);
    }

    #[test]
    fn parse_rejects_wrong_tag() {
        let err = ServiceListDescriptor::parse(&[0x42, 3, 0x00, 0x01, 0x01]).unwrap_err();
        assert!(matches!(err, Error::InvalidDescriptor { tag: 0x42, .. }));
    }

    #[test]
    fn parse_rejects_length_not_multiple_of_3() {
        let bytes = [TAG, 4, 0x00, 0x01, 0x01, 0xFF];
        let err = ServiceListDescriptor::parse(&bytes).unwrap_err();
        assert!(matches!(err, Error::InvalidDescriptor { tag: TAG, .. }));
    }

    #[test]
    fn parse_rejects_truncated_body() {
        let bytes = [TAG, 6, 0x00, 0x01];
        let err = ServiceListDescriptor::parse(&bytes).unwrap_err();
        assert!(matches!(err, Error::BufferTooShort { .. }));
    }

    #[test]
    fn empty_descriptor_valid() {
        let bytes = [TAG, 0];
        let d = ServiceListDescriptor::parse(&bytes).unwrap();
        assert!(d.entries.is_empty());
    }

    #[test]
    fn serialize_round_trip() {
        let d = ServiceListDescriptor {
            entries: vec![
                ServiceListEntry {
                    service_id: 1,
                    service_type: 0x01,
                },
                ServiceListEntry {
                    service_id: 0x0102,
                    service_type: 0x04,
                },
            ],
        };
        let mut buf = vec![0u8; d.serialized_len()];
        d.serialize_into(&mut buf).unwrap();
        let re = ServiceListDescriptor::parse(&buf).unwrap();
        assert_eq!(d, re);
    }
}
