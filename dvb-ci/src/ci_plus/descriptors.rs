//! CI Plus Sample-Mode descriptors (IV / key-identifier) — ETSI TS 103 205
//! V1.4.1 §7.5.5.4, Tables 45-47 (PDF pp. 79-80). See
//! `docs/ts_103_205/ci-plus-descriptors.md`.
//!
//! Both follow the standard TLV form `descriptor_tag` (8) + `descriptor_length`
//! (8) + `descriptor_length` opaque body bytes (§7.5.5.4.1). They may be carried
//! in CI Plus Sample-Mode messages (Sample Start TS Packet) and the
//! media-interface fragment-header descriptor loop.
//!
//! - `ciplus_initialization_vector_descriptor` (`0xD0`, Table 46) — the IV bytes.
//! - `ciplus_key_identifier_descriptor` (`0xD1`, Table 47) — the key-id bytes.
//!
//! The body is an **opaque** crypto blob in both cases; only the variable length
//! and the byte sequence are structural.

use crate::error::{Error, Result};
use dvb_common::{Parse, Serialize};

/// `descriptor_tag` of the `ciplus_initialization_vector_descriptor` (Table 46).
pub const IV_DESCRIPTOR_TAG: u8 = 0xD0;
/// `descriptor_tag` of the `ciplus_key_identifier_descriptor` (Table 47).
pub const KEY_IDENTIFIER_DESCRIPTOR_TAG: u8 = 0xD1;

// descriptor_tag(1) + descriptor_length(1).
const DESC_HEADER: usize = 2;

/// `ciplus_initialization_vector_descriptor()` (Table 46): the IV associated with
/// the following Sample. The IV bytes are an opaque crypto blob.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct CiplusInitializationVectorDescriptor<'a> {
    /// `IV_data_byte` sequence (length = `descriptor_length`).
    #[cfg_attr(feature = "serde", serde(borrow, with = "crate::objects::bytes_serde"))]
    pub iv_data: &'a [u8],
}

impl<'a> Parse<'a> for CiplusInitializationVectorDescriptor<'a> {
    type Error = Error;
    fn parse(bytes: &'a [u8]) -> Result<Self> {
        let body = parse_tlv(
            bytes,
            IV_DESCRIPTOR_TAG,
            "ciplus_initialization_vector_descriptor",
        )?;
        Ok(Self { iv_data: body })
    }
}
impl Serialize for CiplusInitializationVectorDescriptor<'_> {
    type Error = Error;
    fn serialized_len(&self) -> usize {
        DESC_HEADER + self.iv_data.len()
    }
    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        write_tlv(IV_DESCRIPTOR_TAG, self.iv_data, buf)
    }
}

/// `ciplus_key_identifier_descriptor()` (Table 47): the content key identifier
/// associated with the following Sample. The key-id bytes are an opaque blob.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct CiplusKeyIdentifierDescriptor<'a> {
    /// `key_id_data_byte` sequence (length = `descriptor_length`).
    #[cfg_attr(feature = "serde", serde(borrow, with = "crate::objects::bytes_serde"))]
    pub key_id_data: &'a [u8],
}

impl<'a> Parse<'a> for CiplusKeyIdentifierDescriptor<'a> {
    type Error = Error;
    fn parse(bytes: &'a [u8]) -> Result<Self> {
        let body = parse_tlv(
            bytes,
            KEY_IDENTIFIER_DESCRIPTOR_TAG,
            "ciplus_key_identifier_descriptor",
        )?;
        Ok(Self { key_id_data: body })
    }
}
impl Serialize for CiplusKeyIdentifierDescriptor<'_> {
    type Error = Error;
    fn serialized_len(&self) -> usize {
        DESC_HEADER + self.key_id_data.len()
    }
    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        write_tlv(KEY_IDENTIFIER_DESCRIPTOR_TAG, self.key_id_data, buf)
    }
}

/// Parse a `descriptor_tag(8) + descriptor_length(8) + body` TLV, verifying the
/// tag and returning the exactly-`descriptor_length` body slice.
fn parse_tlv<'a>(bytes: &'a [u8], expected_tag: u8, what: &'static str) -> Result<&'a [u8]> {
    if bytes.len() < DESC_HEADER {
        return Err(Error::BufferTooShort {
            need: DESC_HEADER,
            have: bytes.len(),
            what,
        });
    }
    if bytes[0] != expected_tag {
        return Err(Error::InvalidObject {
            what,
            reason: "descriptor_tag mismatch",
        });
    }
    let len = bytes[1] as usize;
    let end = DESC_HEADER + len;
    if bytes.len() < end {
        return Err(Error::LengthMismatch {
            what,
            declared: len,
            actual: bytes.len().saturating_sub(DESC_HEADER),
        });
    }
    Ok(&bytes[DESC_HEADER..end])
}

/// Serialize a TLV descriptor (`descriptor_length` is computed from `body`).
fn write_tlv(tag: u8, body: &[u8], buf: &mut [u8]) -> Result<usize> {
    if body.len() > u8::MAX as usize {
        return Err(Error::LengthTooLarge(body.len()));
    }
    let total = DESC_HEADER + body.len();
    if buf.len() < total {
        return Err(Error::OutputBufferTooSmall {
            need: total,
            have: buf.len(),
        });
    }
    buf[0] = tag;
    buf[1] = body.len() as u8;
    buf[DESC_HEADER..total].copy_from_slice(body);
    Ok(total)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn iv_descriptor_round_trips_and_bites() {
        let d = CiplusInitializationVectorDescriptor {
            iv_data: &[0x00, 0x11, 0x22, 0x33],
        };
        let bytes = d.to_bytes();
        assert_eq!(bytes, [0xD0, 0x04, 0x00, 0x11, 0x22, 0x33]);
        assert_eq!(
            CiplusInitializationVectorDescriptor::parse(&bytes).unwrap(),
            d
        );
        let other = CiplusInitializationVectorDescriptor {
            iv_data: &[0x00, 0x11, 0x22, 0x34],
        };
        assert_ne!(bytes, other.to_bytes());
    }

    #[test]
    fn iv_descriptor_zero_byte_body() {
        let d = CiplusInitializationVectorDescriptor { iv_data: &[] };
        let bytes = d.to_bytes();
        assert_eq!(bytes, [0xD0, 0x00]);
        let parsed = CiplusInitializationVectorDescriptor::parse(&bytes).unwrap();
        assert_eq!(parsed, d);
        assert!(parsed.iv_data.is_empty());
    }

    #[test]
    fn key_id_descriptor_round_trips() {
        let d = CiplusKeyIdentifierDescriptor {
            key_id_data: &[0xDE, 0xAD],
        };
        let bytes = d.to_bytes();
        assert_eq!(bytes, [0xD1, 0x02, 0xDE, 0xAD]);
        assert_eq!(CiplusKeyIdentifierDescriptor::parse(&bytes).unwrap(), d);
    }

    #[test]
    fn key_id_descriptor_zero_byte_body() {
        let d = CiplusKeyIdentifierDescriptor { key_id_data: &[] };
        let bytes = d.to_bytes();
        assert_eq!(bytes, [0xD1, 0x00]);
        assert_eq!(CiplusKeyIdentifierDescriptor::parse(&bytes).unwrap(), d);
    }

    #[test]
    fn wrong_tag_rejected() {
        let bytes = [0xD1, 0x00];
        assert!(matches!(
            CiplusInitializationVectorDescriptor::parse(&bytes),
            Err(Error::InvalidObject { .. })
        ));
    }
}
