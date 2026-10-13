//! Session Protocol Data Unit (SPDU) framing — ETSI EN 50221 §7.2.4-7.2.7,
//! Tables 4-14 (PDF pp. 19-23).
//!
//! An SPDU is a one-byte `spdu_tag`, a [`length_field`](crate::length) coding
//! the length of the *session object value* (NOT any following APDUs), and that
//! value. Only `session_number` (tag `90`) is followed by an SPDU body of
//! APDUs; this module models the session-management objects and the
//! `session_number` header (the trailing APDU body is the caller's, parsed with
//! [`AnyApdu`](crate::AnyApdu)).
//!
//! Each object's `Parse`/`Serialize` covers the whole SPDU header
//! (`spdu_tag` + `length_field` + value); lengths are computed from content.

use crate::error::{Error, Result};
use crate::length::{self, LengthForm};
use crate::resource::ResourceId;
use broadcast_common::{Parse, Serialize};

/// `spdu_tag` values — Table 14 (p. 23). One byte each.
pub mod tags {
    /// `Tsession_number` = `90`.
    pub const SESSION_NUMBER: u8 = 0x90;
    /// `Topen_session_request` = `91`.
    pub const OPEN_SESSION_REQUEST: u8 = 0x91;
    /// `Topen_session_response` = `92`.
    pub const OPEN_SESSION_RESPONSE: u8 = 0x92;
    /// `Tcreate_session` = `93`.
    pub const CREATE_SESSION: u8 = 0x93;
    /// `Tcreate_session_response` = `94`.
    pub const CREATE_SESSION_RESPONSE: u8 = 0x94;
    /// `Tclose_session_request` = `95`.
    pub const CLOSE_SESSION_REQUEST: u8 = 0x95;
    /// `Tclose_session_response` = `96`.
    pub const CLOSE_SESSION_RESPONSE: u8 = 0x96;
}

/// `session_status` values — Tables 7 (open/create) and 12 (close), pp. 20-22.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub enum SessionStatus {
    /// `00` — session opened / closed as required.
    Ok,
    /// `F0` — resource non-existent (open/create), or session_nb not allocated
    /// (close).
    ResourceNonExistent,
    /// `F1` — resource exists but unavailable.
    ResourceUnavailable,
    /// `F2` — resource exists but version lower than requested.
    ResourceVersionTooLow,
    /// `F3` — resource busy.
    ResourceBusy,
    /// Any other value (reserved).
    Reserved(u8),
}

impl SessionStatus {
    /// Decode a `session_status` byte.
    #[must_use]
    pub fn from_u8(v: u8) -> Self {
        match v {
            0x00 => Self::Ok,
            0xF0 => Self::ResourceNonExistent,
            0xF1 => Self::ResourceUnavailable,
            0xF2 => Self::ResourceVersionTooLow,
            0xF3 => Self::ResourceBusy,
            other => Self::Reserved(other),
        }
    }
    /// Wire byte.
    #[must_use]
    pub const fn to_u8(self) -> u8 {
        match self {
            Self::Ok => 0x00,
            Self::ResourceNonExistent => 0xF0,
            Self::ResourceUnavailable => 0xF1,
            Self::ResourceVersionTooLow => 0xF2,
            Self::ResourceBusy => 0xF3,
            Self::Reserved(v) => v,
        }
    }
    /// Spec token, or `"reserved"`.
    #[must_use]
    pub fn name(&self) -> &'static str {
        match self {
            Self::Ok => "ok",
            Self::ResourceNonExistent => "resource_non_existent",
            Self::ResourceUnavailable => "resource_unavailable",
            Self::ResourceVersionTooLow => "resource_version_too_low",
            Self::ResourceBusy => "resource_busy",
            Self::Reserved(_) => "reserved",
        }
    }
}
broadcast_common::impl_spec_display!(SessionStatus, Reserved);

// --- shared SPDU header helpers ---

fn parse_spdu_header<'a>(
    bytes: &'a [u8],
    expected: u8,
    what: &'static str,
) -> Result<(LengthForm, &'a [u8])> {
    let first = *bytes.first().ok_or(Error::BufferTooShort {
        need: 1,
        have: 0,
        what,
    })?;
    if first != expected {
        return Err(Error::UnexpectedSpduTag {
            got: first,
            expected,
            what,
        });
    }
    let (len, hdr, form) = length::decode_form(&bytes[1..])?;
    let start = 1 + hdr;
    let end = start + len;
    if bytes.len() < end {
        return Err(Error::LengthMismatch {
            what,
            declared: len,
            actual: bytes.len().saturating_sub(start),
        });
    }
    Ok((form, &bytes[start..end]))
}

fn spdu_len(value_len: usize, form: LengthForm) -> usize {
    1 + length::encoded_len_with(value_len, form) + value_len
}

fn write_spdu_header(tag: u8, value_len: usize, form: LengthForm, buf: &mut [u8]) -> Result<usize> {
    let total = spdu_len(value_len, form);
    if buf.len() < total {
        return Err(Error::OutputBufferTooSmall {
            need: total,
            have: buf.len(),
        });
    }
    buf[0] = tag;
    let n = length::encode_with(value_len, form, &mut buf[1..])?;
    Ok(1 + n)
}

/// `open_session_request()` (Table 5) — module → host, `length=4`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct OpenSessionRequest {
    /// How the `length_field` was coded on the wire: a non-minimal long form
    /// (e.g. `82 00 09`) is preserved so the object re-serializes
    /// byte-for-byte; [`LengthForm::Minimal`] for a hand-built value.
    #[cfg_attr(feature = "serde", serde(skip))]
    pub length_form: crate::length::LengthForm,

    /// The requested `resource_identifier()`.
    pub resource: ResourceId,
}

impl<'a> Parse<'a> for OpenSessionRequest {
    type Error = Error;
    fn parse(bytes: &'a [u8]) -> Result<Self> {
        let (length_form, v) =
            parse_spdu_header(bytes, tags::OPEN_SESSION_REQUEST, "open_session_request")?;
        Ok(Self {
            length_form,
            resource: ResourceId::parse(v)?,
        })
    }
}
impl Serialize for OpenSessionRequest {
    type Error = Error;
    fn serialized_len(&self) -> usize {
        spdu_len(ResourceId::LEN, self.length_form)
    }
    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        let pos = write_spdu_header(
            tags::OPEN_SESSION_REQUEST,
            ResourceId::LEN,
            self.length_form,
            buf,
        )?;
        let n = self.resource.serialize_into(&mut buf[pos..])?;
        Ok(pos + n)
    }
}

/// `open_session_response()` (Table 6) — host → module, `length=7`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct OpenSessionResponse {
    /// How the `length_field` was coded on the wire: a non-minimal long form
    /// (e.g. `82 00 09`) is preserved so the object re-serializes
    /// byte-for-byte; [`LengthForm::Minimal`] for a hand-built value.
    #[cfg_attr(feature = "serde", serde(skip))]
    pub length_form: crate::length::LengthForm,

    /// `session_status`.
    pub status: SessionStatus,
    /// The `resource_identifier()`.
    pub resource: ResourceId,
    /// `session_nb` (0 reserved; meaningless when status != ok).
    pub session_nb: u16,
}

impl<'a> Parse<'a> for OpenSessionResponse {
    type Error = Error;
    fn parse(bytes: &'a [u8]) -> Result<Self> {
        let (length_form, v) =
            parse_spdu_header(bytes, tags::OPEN_SESSION_RESPONSE, "open_session_response")?;
        if v.len() != 7 {
            return Err(Error::InvalidObject {
                what: "open_session_response",
                reason: "value must be 7 bytes",
            });
        }
        Ok(Self {
            length_form,
            status: SessionStatus::from_u8(v[0]),
            resource: ResourceId::parse(&v[1..5])?,
            session_nb: u16::from_be_bytes([v[5], v[6]]),
        })
    }
}
impl Serialize for OpenSessionResponse {
    type Error = Error;
    fn serialized_len(&self) -> usize {
        spdu_len(7, self.length_form)
    }
    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        let mut pos = write_spdu_header(tags::OPEN_SESSION_RESPONSE, 7, self.length_form, buf)?;
        buf[pos] = self.status.to_u8();
        pos += 1;
        pos += self.resource.serialize_into(&mut buf[pos..])?;
        buf[pos..pos + 2].copy_from_slice(&self.session_nb.to_be_bytes());
        Ok(pos + 2)
    }
}

/// `create_session()` (Table 8) — host → module, `length=6`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct CreateSession {
    /// How the `length_field` was coded on the wire: a non-minimal long form
    /// (e.g. `82 00 09`) is preserved so the object re-serializes
    /// byte-for-byte; [`LengthForm::Minimal`] for a hand-built value.
    #[cfg_attr(feature = "serde", serde(skip))]
    pub length_form: crate::length::LengthForm,

    /// The `resource_identifier()`.
    pub resource: ResourceId,
    /// `session_nb` allocated for the session.
    pub session_nb: u16,
}

impl<'a> Parse<'a> for CreateSession {
    type Error = Error;
    fn parse(bytes: &'a [u8]) -> Result<Self> {
        let (length_form, v) = parse_spdu_header(bytes, tags::CREATE_SESSION, "create_session")?;
        if v.len() != 6 {
            return Err(Error::InvalidObject {
                what: "create_session",
                reason: "value must be 6 bytes",
            });
        }
        Ok(Self {
            length_form,
            resource: ResourceId::parse(&v[0..4])?,
            session_nb: u16::from_be_bytes([v[4], v[5]]),
        })
    }
}
impl Serialize for CreateSession {
    type Error = Error;
    fn serialized_len(&self) -> usize {
        spdu_len(6, self.length_form)
    }
    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        let mut pos = write_spdu_header(tags::CREATE_SESSION, 6, self.length_form, buf)?;
        pos += self.resource.serialize_into(&mut buf[pos..])?;
        buf[pos..pos + 2].copy_from_slice(&self.session_nb.to_be_bytes());
        Ok(pos + 2)
    }
}

/// `create_session_response()` (Table 9) — module → host, `length=7`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct CreateSessionResponse {
    /// How the `length_field` was coded on the wire: a non-minimal long form
    /// (e.g. `82 00 09`) is preserved so the object re-serializes
    /// byte-for-byte; [`LengthForm::Minimal`] for a hand-built value.
    #[cfg_attr(feature = "serde", serde(skip))]
    pub length_form: crate::length::LengthForm,

    /// `session_status`.
    pub status: SessionStatus,
    /// The `resource_identifier()`.
    pub resource: ResourceId,
    /// `session_nb` (equals the create_session it replies to).
    pub session_nb: u16,
}

impl<'a> Parse<'a> for CreateSessionResponse {
    type Error = Error;
    fn parse(bytes: &'a [u8]) -> Result<Self> {
        let (length_form, v) = parse_spdu_header(
            bytes,
            tags::CREATE_SESSION_RESPONSE,
            "create_session_response",
        )?;
        if v.len() != 7 {
            return Err(Error::InvalidObject {
                what: "create_session_response",
                reason: "value must be 7 bytes",
            });
        }
        Ok(Self {
            length_form,
            status: SessionStatus::from_u8(v[0]),
            resource: ResourceId::parse(&v[1..5])?,
            session_nb: u16::from_be_bytes([v[5], v[6]]),
        })
    }
}
impl Serialize for CreateSessionResponse {
    type Error = Error;
    fn serialized_len(&self) -> usize {
        spdu_len(7, self.length_form)
    }
    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        let mut pos = write_spdu_header(tags::CREATE_SESSION_RESPONSE, 7, self.length_form, buf)?;
        buf[pos] = self.status.to_u8();
        pos += 1;
        pos += self.resource.serialize_into(&mut buf[pos..])?;
        buf[pos..pos + 2].copy_from_slice(&self.session_nb.to_be_bytes());
        Ok(pos + 2)
    }
}

/// `close_session_request()` (Table 10) — `length=2`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct CloseSessionRequest {
    /// How the `length_field` was coded on the wire: a non-minimal long form
    /// (e.g. `82 00 09`) is preserved so the object re-serializes
    /// byte-for-byte; [`LengthForm::Minimal`] for a hand-built value.
    #[cfg_attr(feature = "serde", serde(skip))]
    pub length_form: crate::length::LengthForm,

    /// `session_nb` to close.
    pub session_nb: u16,
}

impl<'a> Parse<'a> for CloseSessionRequest {
    type Error = Error;
    fn parse(bytes: &'a [u8]) -> Result<Self> {
        let (length_form, v) =
            parse_spdu_header(bytes, tags::CLOSE_SESSION_REQUEST, "close_session_request")?;
        if v.len() != 2 {
            return Err(Error::InvalidObject {
                what: "close_session_request",
                reason: "value must be 2 bytes",
            });
        }
        Ok(Self {
            length_form,
            session_nb: u16::from_be_bytes([v[0], v[1]]),
        })
    }
}
impl Serialize for CloseSessionRequest {
    type Error = Error;
    fn serialized_len(&self) -> usize {
        spdu_len(2, self.length_form)
    }
    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        let pos = write_spdu_header(tags::CLOSE_SESSION_REQUEST, 2, self.length_form, buf)?;
        buf[pos..pos + 2].copy_from_slice(&self.session_nb.to_be_bytes());
        Ok(pos + 2)
    }
}

/// `close_session_response()` (Table 11) — `length=3`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct CloseSessionResponse {
    /// How the `length_field` was coded on the wire: a non-minimal long form
    /// (e.g. `82 00 09`) is preserved so the object re-serializes
    /// byte-for-byte; [`LengthForm::Minimal`] for a hand-built value.
    #[cfg_attr(feature = "serde", serde(skip))]
    pub length_form: crate::length::LengthForm,

    /// `session_status` (Table 12 values).
    pub status: SessionStatus,
    /// `session_nb` that was closed.
    pub session_nb: u16,
}

impl<'a> Parse<'a> for CloseSessionResponse {
    type Error = Error;
    fn parse(bytes: &'a [u8]) -> Result<Self> {
        let (length_form, v) = parse_spdu_header(
            bytes,
            tags::CLOSE_SESSION_RESPONSE,
            "close_session_response",
        )?;
        if v.len() != 3 {
            return Err(Error::InvalidObject {
                what: "close_session_response",
                reason: "value must be 3 bytes",
            });
        }
        Ok(Self {
            length_form,
            status: SessionStatus::from_u8(v[0]),
            session_nb: u16::from_be_bytes([v[1], v[2]]),
        })
    }
}
impl Serialize for CloseSessionResponse {
    type Error = Error;
    fn serialized_len(&self) -> usize {
        spdu_len(3, self.length_form)
    }
    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        let mut pos = write_spdu_header(tags::CLOSE_SESSION_RESPONSE, 3, self.length_form, buf)?;
        buf[pos] = self.status.to_u8();
        pos += 1;
        buf[pos..pos + 2].copy_from_slice(&self.session_nb.to_be_bytes());
        Ok(pos + 2)
    }
}

/// `session_number()` (Table 13) — `length=2`. Precedes an SPDU body of APDUs;
/// this models only the header (the trailing APDU body is parsed separately by
/// the caller with [`AnyApdu`](crate::AnyApdu)).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct SessionNumber {
    /// How the `length_field` was coded on the wire: a non-minimal long form
    /// (e.g. `82 00 09`) is preserved so the object re-serializes
    /// byte-for-byte; [`LengthForm::Minimal`] for a hand-built value.
    #[cfg_attr(feature = "serde", serde(skip))]
    pub length_form: crate::length::LengthForm,

    /// `session_nb` the following APDUs belong to.
    pub session_nb: u16,
}

impl SessionNumber {
    /// Header length in bytes of the *minimally coded* header (`spdu_tag` +
    /// 1-byte `length_field=2` + `session_nb`). A header whose `length_field`
    /// was coded in a longer form is longer: use [`header_len`](Self::header_len)
    /// to find where the trailing APDU body begins.
    pub const HEADER_LEN: usize = 4;

    /// Length in bytes of *this* header as parsed/serialized (honouring its
    /// [`length_form`](Self::length_form)) = the offset at which the trailing
    /// APDU body begins.
    #[must_use]
    pub fn header_len(&self) -> usize {
        1 + length::encoded_len_with(2, self.length_form) + 2
    }
}

impl<'a> Parse<'a> for SessionNumber {
    type Error = Error;
    fn parse(bytes: &'a [u8]) -> Result<Self> {
        // session_number is followed by an APDU body; only the value (length=2)
        // belongs to the SPDU header, so we read exactly that and ignore any
        // trailing APDU bytes.
        let (length_form, v) = parse_spdu_header(bytes, tags::SESSION_NUMBER, "session_number")?;
        if v.len() != 2 {
            return Err(Error::InvalidObject {
                what: "session_number",
                reason: "value must be 2 bytes",
            });
        }
        Ok(Self {
            length_form,
            session_nb: u16::from_be_bytes([v[0], v[1]]),
        })
    }
}
impl Serialize for SessionNumber {
    type Error = Error;
    fn serialized_len(&self) -> usize {
        spdu_len(2, self.length_form)
    }
    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        let pos = write_spdu_header(tags::SESSION_NUMBER, 2, self.length_form, buf)?;
        buf[pos..pos + 2].copy_from_slice(&self.session_nb.to_be_bytes());
        Ok(pos + 2)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::resource::CONDITIONAL_ACCESS_SUPPORT;

    #[test]
    fn open_session_request_round_trip() {
        let o = OpenSessionRequest {
            length_form: crate::length::LengthForm::Minimal,
            resource: CONDITIONAL_ACCESS_SUPPORT,
        };
        let bytes = o.to_bytes();
        assert_eq!(bytes, [0x91, 0x04, 0x00, 0x03, 0x00, 0x41]);
        assert_eq!(OpenSessionRequest::parse(&bytes).unwrap(), o);
    }

    #[test]
    fn open_session_response_round_trip() {
        let o = OpenSessionResponse {
            length_form: crate::length::LengthForm::Minimal,
            status: SessionStatus::Ok,
            resource: CONDITIONAL_ACCESS_SUPPORT,
            session_nb: 0x0102,
        };
        let bytes = o.to_bytes();
        assert_eq!(bytes[0], tags::OPEN_SESSION_RESPONSE);
        assert_eq!(bytes[1], 0x07);
        let parsed = OpenSessionResponse::parse(&bytes).unwrap();
        assert_eq!(parsed, o);
        assert_eq!(parsed.status.name(), "ok");
    }

    #[test]
    fn create_session_round_trip() {
        let c = CreateSession {
            length_form: crate::length::LengthForm::Minimal,
            resource: CONDITIONAL_ACCESS_SUPPORT,
            session_nb: 5,
        };
        let bytes = c.to_bytes();
        assert_eq!(bytes[1], 0x06);
        assert_eq!(CreateSession::parse(&bytes).unwrap(), c);
    }

    #[test]
    fn create_session_response_round_trip() {
        let c = CreateSessionResponse {
            length_form: crate::length::LengthForm::Minimal,
            status: SessionStatus::ResourceBusy,
            resource: CONDITIONAL_ACCESS_SUPPORT,
            session_nb: 5,
        };
        let bytes = c.to_bytes();
        let parsed = CreateSessionResponse::parse(&bytes).unwrap();
        assert_eq!(parsed, c);
        assert_eq!(parsed.status.name(), "resource_busy");
    }

    #[test]
    fn close_session_round_trips() {
        let req = CloseSessionRequest {
            length_form: crate::length::LengthForm::Minimal,
            session_nb: 0x00FF,
        };
        assert_eq!(req.to_bytes(), [0x95, 0x02, 0x00, 0xFF]);
        assert_eq!(CloseSessionRequest::parse(&req.to_bytes()).unwrap(), req);

        let resp = CloseSessionResponse {
            length_form: crate::length::LengthForm::Minimal,
            status: SessionStatus::Ok,
            session_nb: 0x00FF,
        };
        assert_eq!(resp.to_bytes(), [0x96, 0x03, 0x00, 0x00, 0xFF]);
        assert_eq!(CloseSessionResponse::parse(&resp.to_bytes()).unwrap(), resp);
    }

    #[test]
    fn session_number_round_trip_and_header_len() {
        let sn = SessionNumber {
            length_form: crate::length::LengthForm::Minimal,
            session_nb: 0x1234,
        };
        let bytes = sn.to_bytes();
        assert_eq!(bytes, [0x90, 0x02, 0x12, 0x34]);
        assert_eq!(bytes.len(), SessionNumber::HEADER_LEN);
        // Parses fine even with a trailing APDU body.
        let mut with_body = bytes.to_vec();
        with_body.extend_from_slice(&[0x9F, 0x80, 0x30, 0x00]);
        assert_eq!(SessionNumber::parse(&with_body).unwrap(), sn);
    }

    #[test]
    fn mutating_session_nb_changes_bytes() {
        let req = CloseSessionRequest {
            length_form: crate::length::LengthForm::Minimal,
            session_nb: 1,
        };
        let a = req.to_bytes();
        let b = CloseSessionRequest {
            length_form: crate::length::LengthForm::Minimal,
            session_nb: 2,
        }
        .to_bytes();
        assert_ne!(a, b);
    }

    #[test]
    fn rejects_wrong_tag() {
        assert!(matches!(
            OpenSessionRequest::parse(&[0x92, 0x04, 0, 0, 0, 0]),
            Err(Error::UnexpectedSpduTag { .. })
        ));
    }

    /// Rewrite the `length_field` of an SPDU wire (1-byte tag + minimal
    /// length) into the long form with `octets` length octets.
    fn widen(wire: &[u8], octets: usize) -> alloc::vec::Vec<u8> {
        let (value, hdr) = length::decode(&wire[1..]).unwrap();
        let mut out = alloc::vec![wire[0], 0x80 | octets as u8];
        for i in (0..octets).rev() {
            out.push((value >> (8 * i)) as u8);
        }
        out.extend_from_slice(&wire[1 + hdr..]);
        out
    }

    /// r10-W-4: every SPDU parses a non-minimal `length_field` (`81 NN`,
    /// `82 00 NN`, `83 00 00 NN`, `84 00 00 00 NN`) and re-serializes the
    /// exact bytes; a hand-built value is minimal.
    #[test]
    fn every_spdu_round_trips_each_length_form_byte_exactly() {
        macro_rules! spdu_forms {
            ($ty:ty, $wire:expr) => {{
                let wire: &[u8] = &$wire;
                assert_eq!(<$ty>::parse(wire).unwrap().to_bytes(), wire);
                for octets in 1..=4 {
                    let wide = widen(wire, octets);
                    let parsed = <$ty>::parse(&wide).unwrap();
                    assert_eq!(
                        parsed.to_bytes(),
                        wide,
                        "{} {octets}-octet form",
                        stringify!($ty)
                    );
                    assert_ne!(parsed, <$ty>::parse(wire).unwrap());
                }
            }};
        }
        spdu_forms!(OpenSessionRequest, [0x91, 0x04, 0x00, 0x03, 0x00, 0x41]);
        spdu_forms!(
            OpenSessionResponse,
            [0x92, 0x07, 0x00, 0x00, 0x03, 0x00, 0x41, 0x01, 0x02]
        );
        spdu_forms!(
            CreateSession,
            [0x93, 0x06, 0x00, 0x03, 0x00, 0x41, 0x01, 0x02]
        );
        spdu_forms!(
            CreateSessionResponse,
            [0x94, 0x07, 0x00, 0x00, 0x03, 0x00, 0x41, 0x01, 0x02]
        );
        spdu_forms!(CloseSessionRequest, [0x95, 0x02, 0x01, 0x02]);
        spdu_forms!(CloseSessionResponse, [0x96, 0x03, 0x00, 0x01, 0x02]);
        spdu_forms!(SessionNumber, [0x90, 0x02, 0x01, 0x02]);
    }

    #[test]
    fn session_number_header_len_follows_the_length_form() {
        let minimal = SessionNumber::parse(&[0x90, 0x02, 0x00, 0x03]).unwrap();
        assert_eq!(minimal.header_len(), SessionNumber::HEADER_LEN);
        let long = SessionNumber::parse(&[0x90, 0x82, 0x00, 0x02, 0x00, 0x03]).unwrap();
        assert_eq!(long.header_len(), 6);
        assert_eq!(long.session_nb, 3);
    }
}
