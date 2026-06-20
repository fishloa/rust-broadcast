//! Application-layer APDU objects (resource APDUs) — ETSI EN 50221 §8.4-§8.6.
//!
//! Each object implements [`dvb_common::Parse`] / [`dvb_common::Serialize`] over
//! the **whole** APDU including its `apdu_tag` (3 bytes) + `length_field`
//! header, so dispatch routes on the header and round-trips are byte-symmetric.
//! The shared header helpers here keep every object's length field computed from
//! its content.

use crate::error::{Error, Result};
use crate::length;
use crate::tag::ApduTag;

pub mod application_info;
pub mod ca_info;
pub mod ca_pmt;
pub mod ca_pmt_reply;
pub mod date_time;
pub mod host_control;
pub mod low_speed_comms;
pub mod mmi_close;
pub mod mmi_display;
pub mod mmi_high;
pub mod resource_manager;

/// Parse an APDU header: verify the 3-byte `apdu_tag` matches `expected`, decode
/// the `length_field`, and return the body slice (exactly `length_value` bytes).
pub(crate) fn parse_apdu_header<'a>(
    bytes: &'a [u8],
    expected: ApduTag,
    what: &'static str,
) -> Result<&'a [u8]> {
    if bytes.len() < 3 {
        return Err(Error::BufferTooShort {
            need: 3,
            have: bytes.len(),
            what,
        });
    }
    let got = ApduTag::from_bytes(bytes[0], bytes[1], bytes[2]);
    if got != expected {
        return Err(Error::UnexpectedApduTag {
            got: got.as_u24(),
            expected: expected.as_u24(),
            what,
        });
    }
    let (len_value, len_hdr) = length::decode(&bytes[3..])?;
    let body_start = 3 + len_hdr;
    let body_end = body_start + len_value;
    if bytes.len() < body_end {
        return Err(Error::LengthMismatch {
            what,
            declared: len_value,
            actual: bytes.len().saturating_sub(body_start),
        });
    }
    Ok(&bytes[body_start..body_end])
}

/// Parse a header-only (empty-body) APDU, erroring if the body is non-empty.
pub(crate) fn parse_empty_apdu(bytes: &[u8], expected: ApduTag, what: &'static str) -> Result<()> {
    let body = parse_apdu_header(bytes, expected, what)?;
    if !body.is_empty() {
        return Err(Error::InvalidObject {
            what,
            reason: "expected empty body",
        });
    }
    Ok(())
}

/// Serialized length of an APDU with a `body_len`-byte body.
pub(crate) fn apdu_len(body_len: usize) -> usize {
    3 + length::encoded_len(body_len) + body_len
}

/// Serialized length of a header-only (empty-body) APDU.
pub(crate) fn empty_apdu_len() -> usize {
    apdu_len(0)
}

/// Write an APDU header (tag + `length_field`) into `buf`, returning the number
/// of header bytes written (the body starts at that offset). Checks the buffer
/// can hold the whole APDU (`apdu_len(body_len)`) up front.
pub(crate) fn write_apdu_header(tag: ApduTag, body_len: usize, buf: &mut [u8]) -> Result<usize> {
    let total = apdu_len(body_len);
    if buf.len() < total {
        return Err(Error::OutputBufferTooSmall {
            need: total,
            have: buf.len(),
        });
    }
    buf[..3].copy_from_slice(&tag.to_bytes());
    let n = length::encode_into(body_len, &mut buf[3..])?;
    Ok(3 + n)
}

/// Serialize a header-only (empty-body) APDU into `buf`.
pub(crate) fn serialize_empty_apdu(tag: ApduTag, buf: &mut [u8]) -> Result<usize> {
    write_apdu_header(tag, 0, buf)
}

/// serde helper: serialize a borrowed `&[u8]` field as a byte sequence.
#[cfg(feature = "serde")]
pub(crate) mod bytes_serde {
    pub fn serialize<S: serde::Serializer>(bytes: &[u8], s: S) -> Result<S::Ok, S::Error> {
        s.serialize_bytes(bytes)
    }
}
