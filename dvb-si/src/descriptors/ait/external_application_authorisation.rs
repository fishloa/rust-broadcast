//! External Application Authorisation Descriptor — ETSI TS 102 809 §5.3.5.7, Table 26
//! (AIT tag 0x05).
//!
//! Carried in the AIT common descriptor loop. Authorises external applications
//! by their application identifier + priority.

use crate::descriptors::descriptor_body;
use crate::error::{Error, Result};
use crate::tables::ait::ApplicationIdentifier;
use alloc::vec::Vec;
use dvb_common::{Parse, Serialize};

/// Descriptor tag for external_application_authorisation_descriptor (AIT namespace).
pub const TAG: u8 = 0x05;
const HEADER_LEN: usize = 2;
const ENTRY_LEN: usize = 7;

/// One external application authorisation entry.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct ExternalAppEntry {
    /// Application identifier (organisation_id + application_id).
    pub identifier: ApplicationIdentifier,
    /// Application priority.
    pub application_priority: u8,
}

/// External Application Authorisation Descriptor (AIT tag 0x05).
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct ExternalApplicationAuthorisationDescriptor {
    /// Authorised external applications in wire order.
    pub entries: Vec<ExternalAppEntry>,
}

impl<'a> Parse<'a> for ExternalApplicationAuthorisationDescriptor {
    type Error = crate::error::Error;
    fn parse(bytes: &'a [u8]) -> Result<Self> {
        let body = descriptor_body(
            bytes,
            TAG,
            "ExternalApplicationAuthorisationDescriptor",
            "unexpected tag for external_application_authorisation_descriptor",
        )?;
        if body.len() % ENTRY_LEN != 0 {
            return Err(Error::InvalidDescriptor {
                tag: TAG,
                reason:
                    "external_application_authorisation_descriptor length must be a multiple of 7",
            });
        }
        let mut entries = Vec::with_capacity(body.len() / ENTRY_LEN);
        for chunk in body.chunks_exact(ENTRY_LEN) {
            let (org_bytes, rest) = chunk.split_first_chunk::<4>().unwrap();
            let organisation_id = u32::from_be_bytes(*org_bytes);
            let (app_bytes, priority_slice) = rest.split_first_chunk::<2>().unwrap();
            let application_id = u16::from_be_bytes(*app_bytes);
            let application_priority = priority_slice[0];
            entries.push(ExternalAppEntry {
                identifier: ApplicationIdentifier {
                    organisation_id,
                    application_id,
                },
                application_priority,
            });
        }
        Ok(Self { entries })
    }
}

impl Serialize for ExternalApplicationAuthorisationDescriptor {
    type Error = crate::error::Error;
    fn serialized_len(&self) -> usize {
        HEADER_LEN + self.entries.len() * ENTRY_LEN
    }

    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        if self.entries.len() * ENTRY_LEN > u8::MAX as usize {
            return Err(Error::InvalidDescriptor {
                tag: TAG,
                reason: "external_application_authorisation_descriptor body exceeds 255 bytes",
            });
        }
        let len = self.serialized_len();
        if buf.len() < len {
            return Err(Error::OutputBufferTooSmall {
                need: len,
                have: buf.len(),
            });
        }
        buf[0] = TAG;
        buf[1] = (self.entries.len() * ENTRY_LEN) as u8;
        let mut pos = HEADER_LEN;
        for e in &self.entries {
            buf[pos..pos + 4].copy_from_slice(&e.identifier.organisation_id.to_be_bytes());
            buf[pos + 4..pos + 6].copy_from_slice(&e.identifier.application_id.to_be_bytes());
            buf[pos + 6] = e.application_priority;
            pos += ENTRY_LEN;
        }
        Ok(len)
    }
}

impl<'a> crate::traits::DescriptorDef<'a> for ExternalApplicationAuthorisationDescriptor {
    const TAG: u8 = TAG;
    const NAME: &'static str = "EXTERNAL_APPLICATION_AUTHORISATION";
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_single_entry() {
        let bytes = [
            TAG, 7, 0x00, 0x00, 0x00, 0x01, // organisation_id=1
            0x00, 0x0A, // application_id=10
            0x05, // priority=5
        ];
        let d = ExternalApplicationAuthorisationDescriptor::parse(&bytes).unwrap();
        assert_eq!(d.entries.len(), 1);
        assert_eq!(d.entries[0].identifier.organisation_id, 1);
        assert_eq!(d.entries[0].identifier.application_id, 10);
        assert_eq!(d.entries[0].application_priority, 5);
    }

    #[test]
    fn parse_multiple_entries() {
        let bytes = [
            TAG, 14, 0x00, 0x00, 0x00, 0x01, 0x00, 0x0A, 0x05, 0x00, 0x00, 0x00, 0x02, 0x00, 0x14,
            0x0A,
        ];
        let d = ExternalApplicationAuthorisationDescriptor::parse(&bytes).unwrap();
        assert_eq!(d.entries.len(), 2);
        assert_eq!(d.entries[1].identifier.organisation_id, 2);
    }

    #[test]
    fn parse_rejects_bad_length() {
        let bytes = [TAG, 8, 0, 0, 0, 1, 0, 5, 0, 0]; // body not a multiple of 7
        assert!(ExternalApplicationAuthorisationDescriptor::parse(&bytes).is_err());
    }

    #[test]
    fn serialize_round_trip() {
        let d = ExternalApplicationAuthorisationDescriptor {
            entries: alloc::vec![ExternalAppEntry {
                identifier: ApplicationIdentifier {
                    organisation_id: 0xDEAD,
                    application_id: 0xBEEF,
                },
                application_priority: 42,
            }],
        };
        let mut buf = vec![0u8; d.serialized_len()];
        d.serialize_into(&mut buf).unwrap();
        let re = ExternalApplicationAuthorisationDescriptor::parse(&buf).unwrap();
        assert_eq!(d, re);
    }

    #[test]
    fn serialize_byte_identical() {
        let bytes = [TAG, 7, 0x00, 0x00, 0x10, 0x00, 0x00, 0x01, 0xFF];
        let d = ExternalApplicationAuthorisationDescriptor::parse(&bytes).unwrap();
        let mut buf = vec![0u8; d.serialized_len()];
        d.serialize_into(&mut buf).unwrap();
        assert_eq!(buf.as_slice(), &bytes[..]);
    }
}
