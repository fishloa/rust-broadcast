//! Private Data Specifier Descriptor — ETSI EN 300 468 §6.2.31 (tag 0x5F).
//!
//! Table 85 (PDF p. 98). A single 32-bit `private_data_specifier` value
//! (registered in EN 300 468 Annex/TR 101 162) that scopes the interpretation
//! of any subsequent private (user-defined) descriptors in the same loop.

use super::descriptor_body;
use crate::error::{Error, Result};
use dvb_common::{Parse, Serialize};

/// Well-known `private_data_specifier` for EACEM (European Association of
/// Consumer Electronics Manufacturers) — ETSI TS 101 162 register.
pub const PDS_EACEM: u32 = 0x0000_0028;
/// Well-known `private_data_specifier` for NorDig (Nordic Digital TV) —
/// ETSI TS 101 162 register.
pub const PDS_NORDIG: u32 = 0x0000_0031;

/// Descriptor tag for private_data_specifier_descriptor.
pub const TAG: u8 = 0x5F;
const HEADER_LEN: usize = 2;
/// Fixed payload length: a single 32-bit specifier (EN 300 468 Table 85).
const BODY_LEN: u8 = 4;

/// Best-effort, non-exhaustive mapping from a 32-bit `private_data_specifier`
/// to a human-readable organization name.  Generated at build time from
/// vendored TSDuck `.names` data (`registries/tsPDS.names`); attribution in
/// `registries/README.md`.
#[must_use]
pub fn private_data_specifier_name(v: u32) -> Option<&'static str> {
    crate::registry_names::private_data_specifier_name_generated(v)
}

/// Private Data Specifier Descriptor (tag 0x5F).
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct PrivateDataSpecifierDescriptor {
    /// 32-bit registered private_data_specifier (ETSI Table 85, PDF p. 98).
    pub private_data_specifier: u32,
}

impl<'a> Parse<'a> for PrivateDataSpecifierDescriptor {
    type Error = crate::error::Error;
    fn parse(bytes: &'a [u8]) -> Result<Self> {
        let body = descriptor_body(
            bytes,
            TAG,
            "PrivateDataSpecifierDescriptor",
            "unexpected tag for private_data_specifier_descriptor",
        )?;
        if body.len() != BODY_LEN as usize {
            return Err(Error::InvalidDescriptor {
                tag: TAG,
                reason: "private_data_specifier_descriptor length must equal 4",
            });
        }
        let (hdr, _) = body
            .split_first_chunk::<4>()
            .ok_or(Error::InvalidDescriptor {
                tag: TAG,
                reason: "private_data_specifier_descriptor length must equal 4",
            })?;
        let private_data_specifier = u32::from_be_bytes(*hdr);
        Ok(Self {
            private_data_specifier,
        })
    }
}

impl Serialize for PrivateDataSpecifierDescriptor {
    type Error = crate::error::Error;
    fn serialized_len(&self) -> usize {
        HEADER_LEN + BODY_LEN as usize
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
        buf[1] = BODY_LEN;
        buf[HEADER_LEN..HEADER_LEN + 4].copy_from_slice(&self.private_data_specifier.to_be_bytes());
        Ok(len)
    }
}
impl<'a> crate::traits::DescriptorDef<'a> for PrivateDataSpecifierDescriptor {
    const TAG: u8 = TAG;
    const NAME: &'static str = "PRIVATE_DATA_SPECIFIER";
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_extracts_specifier() {
        let bytes = [TAG, 4, 0x00, 0x00, 0x00, 0x28];
        let d = PrivateDataSpecifierDescriptor::parse(&bytes).unwrap();
        assert_eq!(d.private_data_specifier, 0x0000_0028);
    }

    #[test]
    fn parse_rejects_wrong_tag() {
        let err = PrivateDataSpecifierDescriptor::parse(&[0x60, 4, 0, 0, 0, 0]).unwrap_err();
        assert!(matches!(err, Error::InvalidDescriptor { tag: 0x60, .. }));
    }

    #[test]
    fn parse_rejects_short_buffer() {
        let err = PrivateDataSpecifierDescriptor::parse(&[TAG]).unwrap_err();
        assert!(matches!(err, Error::BufferTooShort { .. }));
    }

    #[test]
    fn parse_rejects_truncated_body() {
        // length=4 but only 2 payload bytes present.
        let err = PrivateDataSpecifierDescriptor::parse(&[TAG, 4, 0, 0]).unwrap_err();
        assert!(matches!(err, Error::BufferTooShort { .. }));
    }

    #[test]
    fn parse_rejects_wrong_length() {
        let err = PrivateDataSpecifierDescriptor::parse(&[TAG, 3, 0, 0, 0]).unwrap_err();
        assert!(matches!(err, Error::InvalidDescriptor { .. }));
    }

    #[test]
    fn serialize_round_trip() {
        let d = PrivateDataSpecifierDescriptor {
            private_data_specifier: 0xDEAD_BEEF,
        };
        let mut buf = vec![0u8; d.serialized_len()];
        d.serialize_into(&mut buf).unwrap();
        let re = PrivateDataSpecifierDescriptor::parse(&buf).unwrap();
        assert_eq!(d, re);
    }

    #[test]
    fn serialize_rejects_too_small_buffer() {
        let d = PrivateDataSpecifierDescriptor {
            private_data_specifier: 1,
        };
        let mut tiny = [0u8; 3];
        let err = d.serialize_into(&mut tiny).unwrap_err();
        assert!(matches!(err, Error::OutputBufferTooSmall { .. }));
    }

    #[test]
    fn descriptor_length_matches_payload() {
        let d = PrivateDataSpecifierDescriptor {
            private_data_specifier: 0,
        };
        assert_eq!(d.serialized_len() - 2, 4);
    }

    #[cfg(feature = "serde")]
    #[test]
    fn serde_round_trip() {
        let d = PrivateDataSpecifierDescriptor {
            private_data_specifier: 0x0000_233A,
        };
        let json = serde_json::to_string(&d).unwrap();
        // Serialize-only: assert the emitted JSON re-parses (serialize-stable).
        let _v: serde_json::Value = serde_json::from_str(&json).unwrap();
    }

    #[test]
    fn specifier_name_exact_entry() {
        // Exact entry from tsPDS.names [PrivateDataSpecifier].
        assert_eq!(private_data_specifier_name(0x0000_0001), Some("SES Astra"));
        assert_eq!(
            private_data_specifier_name(0x0000_0005),
            Some("ARD, ZDF, ORF")
        );
    }

    #[test]
    fn specifier_name_range_entry() {
        // Range entry 0x00000002-0x00000004 => "BskyB" from tsPDS.names.
        assert_eq!(private_data_specifier_name(0x0000_0002), Some("BskyB"));
        assert_eq!(private_data_specifier_name(0x0000_0004), Some("BskyB"));
    }

    #[test]
    fn specifier_name_unknown() {
        assert_eq!(private_data_specifier_name(0x0000_0000), None);
        assert_eq!(private_data_specifier_name(0x0000_003C), None);
        assert_eq!(private_data_specifier_name(0xDEAD_BEEF), None);
    }
}
