//! CA Identifier Descriptor — ETSI EN 300 468 §6.2.6 (tag 0x53).
//!
//! Table 22 (PDF p. 57). Carried in SDT/BAT/EIT descriptor loops; lists the
//! `CA_system_id`s whose conditional-access systems are available for the
//! associated service/bouquet/event. Body is simply `N` × 16-bit CAIDs.

pub use super::ca::ca_system_name;
use super::descriptor_body;
use crate::error::{Error, Result};
use dvb_common::{Parse, Serialize};

/// Descriptor tag for CA_identifier_descriptor.
pub const TAG: u8 = 0x53;
const HEADER_LEN: usize = 2;
const ENTRY_LEN: usize = 2;
/// Maximum body length expressible in the 8-bit `descriptor_length` field.
const MAX_BODY_LEN: usize = u8::MAX as usize;

/// CA Identifier Descriptor.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct CaIdentifierDescriptor {
    /// CA_system_id values in wire order.
    pub ca_system_ids: Vec<u16>,
}

impl<'a> Parse<'a> for CaIdentifierDescriptor {
    type Error = crate::error::Error;
    fn parse(bytes: &'a [u8]) -> Result<Self> {
        let body = descriptor_body(
            bytes,
            TAG,
            "CaIdentifierDescriptor",
            "unexpected tag for CA_identifier_descriptor",
        )?;
        if body.len() % ENTRY_LEN != 0 {
            return Err(Error::InvalidDescriptor {
                tag: TAG,
                reason: "descriptor_length must be a multiple of 2",
            });
        }
        let mut ca_system_ids = Vec::with_capacity(body.len() / ENTRY_LEN);
        for chunk in body.chunks_exact(ENTRY_LEN) {
            ca_system_ids.push(u16::from_be_bytes([chunk[0], chunk[1]]));
        }
        Ok(Self { ca_system_ids })
    }
}

impl Serialize for CaIdentifierDescriptor {
    type Error = crate::error::Error;
    fn serialized_len(&self) -> usize {
        HEADER_LEN + ENTRY_LEN * self.ca_system_ids.len()
    }

    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        let len = self.serialized_len();
        if buf.len() < len {
            return Err(Error::OutputBufferTooSmall {
                need: len,
                have: buf.len(),
            });
        }
        let body_len = ENTRY_LEN * self.ca_system_ids.len();
        // 8-bit descriptor_length field: error rather than silently truncate.
        if body_len > MAX_BODY_LEN {
            return Err(Error::InvalidDescriptor {
                tag: TAG,
                reason: "CA_identifier_descriptor body exceeds 255 bytes",
            });
        }
        buf[0] = TAG;
        buf[1] = body_len as u8;
        let mut pos = HEADER_LEN;
        for caid in &self.ca_system_ids {
            buf[pos..pos + ENTRY_LEN].copy_from_slice(&caid.to_be_bytes());
            pos += ENTRY_LEN;
        }
        Ok(len)
    }
}
impl<'a> crate::traits::DescriptorDef<'a> for CaIdentifierDescriptor {
    const TAG: u8 = TAG;
    const NAME: &'static str = "CA_IDENTIFIER";
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_single_caid() {
        let bytes = [TAG, 2, 0x06, 0x50];
        let d = CaIdentifierDescriptor::parse(&bytes).unwrap();
        assert_eq!(d.ca_system_ids, vec![0x0650]);
    }

    #[test]
    fn parse_multiple_caids_preserves_order() {
        let bytes = [TAG, 6, 0x05, 0x00, 0x06, 0x50, 0x0B, 0x00];
        let d = CaIdentifierDescriptor::parse(&bytes).unwrap();
        assert_eq!(d.ca_system_ids, vec![0x0500, 0x0650, 0x0B00]);
    }

    #[test]
    fn parse_rejects_wrong_tag() {
        assert!(matches!(
            CaIdentifierDescriptor::parse(&[0x54, 2, 0x06, 0x50]).unwrap_err(),
            Error::InvalidDescriptor { tag: 0x54, .. }
        ));
    }

    #[test]
    fn parse_rejects_short_buffer() {
        // declares 4 body bytes, only 2 present
        let bytes = [TAG, 4, 0x06, 0x50];
        assert!(matches!(
            CaIdentifierDescriptor::parse(&bytes).unwrap_err(),
            Error::BufferTooShort { .. }
        ));
    }

    #[test]
    fn parse_rejects_odd_length() {
        let bytes = [TAG, 3, 0x06, 0x50, 0x00];
        assert!(matches!(
            CaIdentifierDescriptor::parse(&bytes).unwrap_err(),
            Error::InvalidDescriptor { tag: TAG, .. }
        ));
    }

    #[test]
    fn empty_descriptor_valid() {
        let d = CaIdentifierDescriptor::parse(&[TAG, 0]).unwrap();
        assert!(d.ca_system_ids.is_empty());
    }

    #[test]
    fn serialize_round_trip() {
        let d = CaIdentifierDescriptor {
            ca_system_ids: vec![0x0100, 0x1800, 0x2600],
        };
        let mut buf = vec![0u8; d.serialized_len()];
        d.serialize_into(&mut buf).unwrap();
        assert_eq!(CaIdentifierDescriptor::parse(&buf).unwrap(), d);
    }

    #[test]
    fn serialize_rejects_over_range_body() {
        // 128 CAIDs = 256 body bytes, one past the u8 length field.
        let d = CaIdentifierDescriptor {
            ca_system_ids: vec![0x0500; 128],
        };
        let mut buf = vec![0u8; d.serialized_len()];
        assert!(matches!(
            d.serialize_into(&mut buf).unwrap_err(),
            Error::InvalidDescriptor { tag: TAG, .. }
        ));
    }

    #[cfg(feature = "serde")]
    #[test]
    fn serde_round_trip() {
        let d = CaIdentifierDescriptor {
            ca_system_ids: vec![0x0500, 0x0650],
        };
        let json = serde_json::to_string(&d).unwrap();
        // Serialize-only: assert the emitted JSON re-parses (serialize-stable).
        let _v: serde_json::Value = serde_json::from_str(&json).unwrap();
    }
}
