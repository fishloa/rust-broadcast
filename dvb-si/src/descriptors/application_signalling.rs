//! Application Signalling Descriptor — ETSI TS 102 809 §5.3.5.2.1, Table 17
//! (tag 0x6F).
//!
//! Carried in the PMT to point at the AIT and enumerate the application types
//! present alongside the AIT version expected. Per ts_102_809_apps.md
//! "Table 17 — Application signalling descriptor syntax" (PDF p. 37) each loop
//! entry is 3 bytes: reserved_future_use(1) + application_type(15), then
//! reserved_future_use(3) + AIT_version_number(5).

use super::descriptor_body;
use crate::error::{Error, Result};
use dvb_common::{Parse, Serialize};

/// Descriptor tag for application_signalling_descriptor.
pub const TAG: u8 = 0x6F;
const HEADER_LEN: usize = 2;
const ENTRY_LEN: usize = 3;

/// Largest representable 15-bit application_type.
const APPLICATION_TYPE_MAX: u16 = 0x7FFF;
/// Largest representable 5-bit AIT_version_number.
const AIT_VERSION_MAX: u8 = 0x1F;

/// One application-signalling loop entry.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct ApplicationSignallingEntry {
    /// 15-bit application_type (e.g. 0x0001 = DVB-J, 0x0010 = DVB-HTML).
    pub application_type: u16,
    /// 5-bit AIT_version_number this entry refers to.
    pub ait_version_number: u8,
}

/// Application Signalling Descriptor.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct ApplicationSignallingDescriptor {
    /// Entries in wire order.
    pub entries: Vec<ApplicationSignallingEntry>,
}

impl<'a> Parse<'a> for ApplicationSignallingDescriptor {
    type Error = crate::error::Error;
    fn parse(bytes: &'a [u8]) -> Result<Self> {
        let body = descriptor_body(
            bytes,
            TAG,
            "ApplicationSignallingDescriptor",
            "unexpected tag for application_signalling_descriptor",
        )?;
        if body.len() % ENTRY_LEN != 0 {
            return Err(Error::InvalidDescriptor {
                tag: TAG,
                reason: "application_signalling_descriptor length must be a multiple of 3",
            });
        }
        let mut entries = Vec::with_capacity(body.len() / ENTRY_LEN);
        for chunk in body.chunks_exact(ENTRY_LEN) {
            // reserved_future_use(1) ignored on parse.
            let application_type = u16::from_be_bytes([chunk[0], chunk[1]]) & APPLICATION_TYPE_MAX;
            // reserved_future_use(3) ignored on parse.
            let ait_version_number = chunk[2] & AIT_VERSION_MAX;
            entries.push(ApplicationSignallingEntry {
                application_type,
                ait_version_number,
            });
        }
        Ok(Self { entries })
    }
}

impl Serialize for ApplicationSignallingDescriptor {
    type Error = crate::error::Error;
    fn serialized_len(&self) -> usize {
        HEADER_LEN + self.entries.len() * ENTRY_LEN
    }

    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        for e in &self.entries {
            if e.application_type > APPLICATION_TYPE_MAX {
                return Err(Error::InvalidDescriptor {
                    tag: TAG,
                    reason: "application_type exceeds 15 bits",
                });
            }
            if e.ait_version_number > AIT_VERSION_MAX {
                return Err(Error::InvalidDescriptor {
                    tag: TAG,
                    reason: "ait_version_number exceeds 5 bits",
                });
            }
        }
        if self.entries.len() * ENTRY_LEN > u8::MAX as usize {
            return Err(Error::InvalidDescriptor {
                tag: TAG,
                reason: "application_signalling_descriptor body exceeds 255 bytes",
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
            // reserved_future_use(1) emitted as 1.
            let word = 0x8000 | (e.application_type & APPLICATION_TYPE_MAX);
            buf[pos..pos + 2].copy_from_slice(&word.to_be_bytes());
            // reserved_future_use(3) emitted as 1s.
            buf[pos + 2] = 0xE0 | (e.ait_version_number & AIT_VERSION_MAX);
            pos += ENTRY_LEN;
        }
        Ok(len)
    }
}
impl<'a> crate::traits::DescriptorDef<'a> for ApplicationSignallingDescriptor {
    const TAG: u8 = TAG;
    const NAME: &'static str = "APPLICATION_SIGNALLING";
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_single_entry() {
        // application_type=0x0010 (DVB-HTML), AIT_version=3, reserved bits set.
        let bytes = [TAG, 3, 0x80, 0x10, 0xE3];
        let d = ApplicationSignallingDescriptor::parse(&bytes).unwrap();
        assert_eq!(d.entries.len(), 1);
        assert_eq!(d.entries[0].application_type, 0x0010);
        assert_eq!(d.entries[0].ait_version_number, 3);
    }

    #[test]
    fn parse_multiple_entries() {
        let bytes = [TAG, 6, 0x00, 0x01, 0x05, 0x7F, 0xFF, 0x1F];
        let d = ApplicationSignallingDescriptor::parse(&bytes).unwrap();
        assert_eq!(d.entries.len(), 2);
        assert_eq!(d.entries[0].application_type, 0x0001);
        assert_eq!(d.entries[0].ait_version_number, 5);
        assert_eq!(d.entries[1].application_type, 0x7FFF);
        assert_eq!(d.entries[1].ait_version_number, 0x1F);
    }

    #[test]
    fn parse_ignores_reserved_bits() {
        // top bit of word + top 3 bits of version byte are reserved → masked out.
        let bytes = [TAG, 3, 0xFF, 0xFF, 0xFF];
        let d = ApplicationSignallingDescriptor::parse(&bytes).unwrap();
        assert_eq!(d.entries[0].application_type, 0x7FFF);
        assert_eq!(d.entries[0].ait_version_number, 0x1F);
    }

    #[test]
    fn parse_rejects_wrong_tag() {
        assert!(matches!(
            ApplicationSignallingDescriptor::parse(&[0x70, 0]).unwrap_err(),
            Error::InvalidDescriptor { tag: 0x70, .. }
        ));
    }

    #[test]
    fn parse_rejects_length_not_multiple_of_3() {
        let bytes = [TAG, 4, 0, 0, 0, 0];
        assert!(matches!(
            ApplicationSignallingDescriptor::parse(&bytes).unwrap_err(),
            Error::InvalidDescriptor { .. }
        ));
    }

    #[test]
    fn empty_descriptor_valid() {
        let bytes = [TAG, 0];
        let d = ApplicationSignallingDescriptor::parse(&bytes).unwrap();
        assert!(d.entries.is_empty());
    }

    #[test]
    fn serialize_round_trip() {
        let d = ApplicationSignallingDescriptor {
            entries: vec![
                ApplicationSignallingEntry {
                    application_type: 0x0001,
                    ait_version_number: 7,
                },
                ApplicationSignallingEntry {
                    application_type: 0x0010,
                    ait_version_number: 0,
                },
            ],
        };
        let mut buf = vec![0u8; d.serialized_len()];
        d.serialize_into(&mut buf).unwrap();
        assert_eq!(ApplicationSignallingDescriptor::parse(&buf).unwrap(), d);
    }

    #[test]
    fn serialize_rejects_application_type_over_range() {
        let d = ApplicationSignallingDescriptor {
            entries: vec![ApplicationSignallingEntry {
                application_type: 0x8000,
                ait_version_number: 0,
            }],
        };
        let mut buf = vec![0u8; d.serialized_len()];
        assert!(matches!(
            d.serialize_into(&mut buf).unwrap_err(),
            Error::InvalidDescriptor { .. }
        ));
    }

    #[test]
    fn serialize_rejects_ait_version_over_range() {
        let d = ApplicationSignallingDescriptor {
            entries: vec![ApplicationSignallingEntry {
                application_type: 0,
                ait_version_number: 0x20,
            }],
        };
        let mut buf = vec![0u8; d.serialized_len()];
        assert!(matches!(
            d.serialize_into(&mut buf).unwrap_err(),
            Error::InvalidDescriptor { .. }
        ));
    }

    #[cfg(feature = "serde")]
    #[test]
    fn serde_round_trip() {
        let d = ApplicationSignallingDescriptor {
            entries: vec![ApplicationSignallingEntry {
                application_type: 0x0010,
                ait_version_number: 4,
            }],
        };
        let j = serde_json::to_string(&d).unwrap();
        // Serialize-only: assert the emitted JSON re-parses (serialize-stable).
        let _v: serde_json::Value = serde_json::from_str(&j).unwrap();
    }
}
