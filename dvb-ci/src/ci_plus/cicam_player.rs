//! CICAM Player resource objects — ETSI TS 103 205 V1.4.1 §8.8, Tables 48-71
//! (PDF pp. 86-96). See `docs/ts_103_205/cicam-player.md`.
//!
//! Resource ID `0x00930041` (Class 147, Type 1, Version 1). The CICAM Player
//! resource lets the Host request the CICAM to initiate and play a service on its
//! behalf (IP-delivery CICAM player mode). Tags live in the CI Plus `0x9FA0xx`
//! namespace; the full 16-APDU set `9FA000`-`9FA00F` is implemented.
//!
//! - `CICAM_player_verify_req` (`9F A0 00`, Table 49) — Host → CICAM.
//! - `CICAM_player_verify_reply` (`9F A0 01`, Table 50) — CICAM → Host.
//! - `CICAM_player_capabilities_req` (`9F A0 02`, Table 52) — CICAM → Host, header-only.
//! - `CICAM_player_capabilities_reply` (`9F A0 03`, Table 53) — Host → CICAM.
//! - `CICAM_player_start_req` (`9F A0 04`, Table 54) — CICAM → Host.
//! - `CICAM_player_start_reply` (`9F A0 05`, Table 55) — Host → CICAM.
//! - `CICAM_player_play_req` (`9F A0 06`, Table 57) — Host → CICAM.
//! - `CICAM_player_status_error` (`9F A0 07`, Table 58) — CICAM → Host.
//! - `CICAM_player_control_req` (`9F A0 08`, Table 60) — Host → CICAM.
//! - `CICAM_player_info_req` (`9F A0 09`, Table 63) — Host → CICAM.
//! - `CICAM_player_info_reply` (`9F A0 0A`, Table 64) — CICAM → Host.
//! - `CICAM_player_stop` (`9F A0 0B`, Table 65) — Host → CICAM.
//! - `CICAM_player_end` (`9F A0 0C`, Table 66) — CICAM → Host.
//! - `CICAM_player_asset_end` (`9F A0 0D`, Table 67) — CICAM → Host.
//! - `CICAM_player_update_req` (`9F A0 0E`, Table 68) — CICAM → Host.
//! - `CICAM_player_update_reply` (`9F A0 0F`, Table 69) — Host → CICAM.
//!
//! ## Table 69 tag slip
//!
//! Table 69 (PDF p. 95) literally prints the first syntax row as
//! `CICAM_player_start_reply_tag` — an evident copy/paste slip from Table 55. The
//! field-list text gives the authoritative tag `0x9FA00F`
//! (`CICAM_player_update_reply_tag`); this module uses `0x9FA00F`.
//!
//! The `service_location_byte` bodies (`verify_req` / `play_req`) and the
//! `PMT_byte` bodies (`start_req` / `update_req`) are opaque to the wire parser —
//! carried as borrowed `&[u8]`.

use crate::error::{Error, Result};
use crate::objects;
use crate::tag::ApduTag;
use alloc::vec::Vec;
use dvb_common::{Parse, Serialize};

/// Resource-scoped `apdu_tag`s for the CICAM Player resource (Table 71).
pub mod tag {
    use crate::tag::ApduTag;
    /// `CICAM_player_verify_req_tag` = `9F A0 00`.
    pub const VERIFY_REQ: ApduTag = ApduTag::from_bytes(0x9F, 0xA0, 0x00);
    /// `CICAM_player_verify_reply_tag` = `9F A0 01`.
    pub const VERIFY_REPLY: ApduTag = ApduTag::from_bytes(0x9F, 0xA0, 0x01);
    /// `CICAM_player_capabilities_req_tag` = `9F A0 02`.
    pub const CAPABILITIES_REQ: ApduTag = ApduTag::from_bytes(0x9F, 0xA0, 0x02);
    /// `CICAM_player_capabilities_reply_tag` = `9F A0 03`.
    pub const CAPABILITIES_REPLY: ApduTag = ApduTag::from_bytes(0x9F, 0xA0, 0x03);
    /// `CICAM_player_start_req_tag` = `9F A0 04`.
    pub const START_REQ: ApduTag = ApduTag::from_bytes(0x9F, 0xA0, 0x04);
    /// `CICAM_player_start_reply_tag` = `9F A0 05`.
    pub const START_REPLY: ApduTag = ApduTag::from_bytes(0x9F, 0xA0, 0x05);
    /// `CICAM_player_play_req_tag` = `9F A0 06`.
    pub const PLAY_REQ: ApduTag = ApduTag::from_bytes(0x9F, 0xA0, 0x06);
    /// `CICAM_player_status_error_tag` = `9F A0 07`.
    pub const STATUS_ERROR: ApduTag = ApduTag::from_bytes(0x9F, 0xA0, 0x07);
    /// `CICAM_player_control_req_tag` = `9F A0 08`.
    pub const CONTROL_REQ: ApduTag = ApduTag::from_bytes(0x9F, 0xA0, 0x08);
    /// `CICAM_player_info_req_tag` = `9F A0 09`.
    pub const INFO_REQ: ApduTag = ApduTag::from_bytes(0x9F, 0xA0, 0x09);
    /// `CICAM_player_info_reply_tag` = `9F A0 0A`.
    pub const INFO_REPLY: ApduTag = ApduTag::from_bytes(0x9F, 0xA0, 0x0A);
    /// `CICAM_player_stop_tag` = `9F A0 0B`.
    pub const STOP: ApduTag = ApduTag::from_bytes(0x9F, 0xA0, 0x0B);
    /// `CICAM_player_end_tag` = `9F A0 0C`.
    pub const END: ApduTag = ApduTag::from_bytes(0x9F, 0xA0, 0x0C);
    /// `CICAM_player_asset_end_tag` = `9F A0 0D`.
    pub const ASSET_END: ApduTag = ApduTag::from_bytes(0x9F, 0xA0, 0x0D);
    /// `CICAM_player_update_req_tag` = `9F A0 0E`.
    pub const UPDATE_REQ: ApduTag = ApduTag::from_bytes(0x9F, 0xA0, 0x0E);
    /// `CICAM_player_update_reply_tag` = `9F A0 0F`. (Table 69 prints a
    /// copy/paste `CICAM_player_start_reply_tag` slip; the authoritative tag is
    /// `0x9FA00F`.)
    pub const UPDATE_REPLY: ApduTag = ApduTag::from_bytes(0x9F, 0xA0, 0x0F);
}

// --- player_verify_status (Table 51) ---

/// `player_verify_status` values (Table 51).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub enum PlayerVerifyStatus {
    /// `0x00` — OK, service playback is possible.
    Ok,
    /// `0x01` — Error, service playback is not possible.
    Error,
    /// Reserved (`0x02`–`0xFF`).
    Reserved(u8),
}
impl PlayerVerifyStatus {
    /// Decode a `player_verify_status` byte.
    #[must_use]
    pub fn from_u8(v: u8) -> Self {
        match v {
            0x00 => Self::Ok,
            0x01 => Self::Error,
            other => Self::Reserved(other),
        }
    }
    /// Wire byte.
    #[must_use]
    pub fn to_u8(self) -> u8 {
        match self {
            Self::Ok => 0x00,
            Self::Error => 0x01,
            Self::Reserved(v) => v,
        }
    }
    /// Spec token, or `"reserved"`.
    #[must_use]
    pub fn name(&self) -> &'static str {
        match self {
            Self::Ok => "ok",
            Self::Error => "error",
            Self::Reserved(_) => "reserved",
        }
    }
}
dvb_common::impl_spec_display!(PlayerVerifyStatus, Reserved);

// --- input_status (Table 56) ---

/// `input_status` values (Table 56).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub enum InputStatus {
    /// `0x00` — OK, a Local TS has switched to Input Mode.
    Ok,
    /// `0x01` — Request refused.
    RequestRefused,
    /// `0x02` — Insufficient bitrate available.
    InsufficientBitrate,
    /// `0x03` — No remaining player sessions available.
    NoSessionsAvailable,
    /// Reserved (`0x04`–`0xFF`).
    Reserved(u8),
}
impl InputStatus {
    /// Decode an `input_status` byte.
    #[must_use]
    pub fn from_u8(v: u8) -> Self {
        match v {
            0x00 => Self::Ok,
            0x01 => Self::RequestRefused,
            0x02 => Self::InsufficientBitrate,
            0x03 => Self::NoSessionsAvailable,
            other => Self::Reserved(other),
        }
    }
    /// Wire byte.
    #[must_use]
    pub fn to_u8(self) -> u8 {
        match self {
            Self::Ok => 0x00,
            Self::RequestRefused => 0x01,
            Self::InsufficientBitrate => 0x02,
            Self::NoSessionsAvailable => 0x03,
            Self::Reserved(v) => v,
        }
    }
    /// Spec token, or `"reserved"`.
    #[must_use]
    pub fn name(&self) -> &'static str {
        match self {
            Self::Ok => "ok",
            Self::RequestRefused => "request_refused",
            Self::InsufficientBitrate => "insufficient_bitrate",
            Self::NoSessionsAvailable => "no_sessions_available",
            Self::Reserved(_) => "reserved",
        }
    }
}
dvb_common::impl_spec_display!(InputStatus, Reserved);

// --- play_status (Table 59) ---

/// `play_status` values (Table 59), carried in `CICAM_player_status_error`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub enum PlayStatus {
    /// `0x01` — Error, content play is not possible (unsupported format/protocol).
    PlayNotPossible,
    /// `0x02` — Error, unrecoverable error.
    Unrecoverable,
    /// `0x03` — Error, content blocked (no content license available).
    ContentBlocked,
    /// Reserved (`0x00`, `0x04`–`0xFF`).
    Reserved(u8),
}
impl PlayStatus {
    /// Decode a `play_status` byte.
    #[must_use]
    pub fn from_u8(v: u8) -> Self {
        match v {
            0x01 => Self::PlayNotPossible,
            0x02 => Self::Unrecoverable,
            0x03 => Self::ContentBlocked,
            other => Self::Reserved(other),
        }
    }
    /// Wire byte.
    #[must_use]
    pub fn to_u8(self) -> u8 {
        match self {
            Self::PlayNotPossible => 0x01,
            Self::Unrecoverable => 0x02,
            Self::ContentBlocked => 0x03,
            Self::Reserved(v) => v,
        }
    }
    /// Spec token, or `"reserved"`.
    #[must_use]
    pub fn name(&self) -> &'static str {
        match self {
            Self::PlayNotPossible => "play_not_possible",
            Self::Unrecoverable => "unrecoverable_error",
            Self::ContentBlocked => "content_blocked",
            Self::Reserved(_) => "reserved",
        }
    }
}
dvb_common::impl_spec_display!(PlayStatus, Reserved);

// --- Command (Table 61) / seek_mode (Table 62) ---

/// `seek_mode` values (Table 62).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub enum SeekMode {
    /// `0x00` — Absolute.
    Absolute,
    /// `0x01` — Relative to current position.
    Relative,
    /// Reserved (`0x02`–`0xFF`).
    Reserved(u8),
}
impl SeekMode {
    /// Decode a `seek_mode` byte.
    #[must_use]
    pub fn from_u8(v: u8) -> Self {
        match v {
            0x00 => Self::Absolute,
            0x01 => Self::Relative,
            other => Self::Reserved(other),
        }
    }
    /// Wire byte.
    #[must_use]
    pub fn to_u8(self) -> u8 {
        match self {
            Self::Absolute => 0x00,
            Self::Relative => 0x01,
            Self::Reserved(v) => v,
        }
    }
    /// Spec token, or `"reserved"`.
    #[must_use]
    pub fn name(&self) -> &'static str {
        match self {
            Self::Absolute => "absolute",
            Self::Relative => "relative",
            Self::Reserved(_) => "reserved",
        }
    }
}
dvb_common::impl_spec_display!(SeekMode, Reserved);

// --- update_status (Table 70) ---

/// `update_status` values (Table 70).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub enum UpdateStatus {
    /// `0x00` — OK, Host processed the updated PMT and is ready to receive the Local TS.
    Ok,
    /// `0x01` — Request refused.
    RequestRefused,
    /// Reserved (`0x02`–`0xFF`).
    Reserved(u8),
}
impl UpdateStatus {
    /// Decode an `update_status` byte.
    #[must_use]
    pub fn from_u8(v: u8) -> Self {
        match v {
            0x00 => Self::Ok,
            0x01 => Self::RequestRefused,
            other => Self::Reserved(other),
        }
    }
    /// Wire byte.
    #[must_use]
    pub fn to_u8(self) -> u8 {
        match self {
            Self::Ok => 0x00,
            Self::RequestRefused => 0x01,
            Self::Reserved(v) => v,
        }
    }
    /// Spec token, or `"reserved"`.
    #[must_use]
    pub fn name(&self) -> &'static str {
        match self {
            Self::Ok => "ok",
            Self::RequestRefused => "request_refused",
            Self::Reserved(_) => "reserved",
        }
    }
}
dvb_common::impl_spec_display!(UpdateStatus, Reserved);

// ---------------------------------------------------------------------------
// CICAM_player_verify_req (Table 49)
// ---------------------------------------------------------------------------

/// `CICAM_player_verify_req()` (Table 49): Host → CICAM. A length-prefixed
/// `service_location` XML blob (annex D schema).
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct PlayerVerifyReq<'a> {
    /// `service_location_byte` body (`service_location_length` bytes) — opaque XML.
    #[cfg_attr(feature = "serde", serde(borrow, with = "crate::objects::bytes_serde"))]
    pub service_location: &'a [u8],
}

// service_location_length(2) + service_location bytes.
const SERVICE_LOCATION_PREFIX: usize = 2;

impl<'a> Parse<'a> for PlayerVerifyReq<'a> {
    type Error = Error;
    fn parse(bytes: &'a [u8]) -> Result<Self> {
        let body = objects::parse_apdu_header(bytes, tag::VERIFY_REQ, "CICAM_player_verify_req")?;
        let service_location = parse_service_location(body, "CICAM_player_verify_req")?;
        Ok(Self { service_location })
    }
}
impl Serialize for PlayerVerifyReq<'_> {
    type Error = Error;
    fn serialized_len(&self) -> usize {
        objects::apdu_len(SERVICE_LOCATION_PREFIX + self.service_location.len())
    }
    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        let body_len = SERVICE_LOCATION_PREFIX + self.service_location.len();
        let pos = objects::write_apdu_header(tag::VERIFY_REQ, body_len, buf)?;
        write_service_location(self.service_location, &mut buf[pos..]);
        Ok(pos + body_len)
    }
}

fn parse_service_location<'a>(body: &'a [u8], what: &'static str) -> Result<&'a [u8]> {
    if body.len() < SERVICE_LOCATION_PREFIX {
        return Err(Error::BufferTooShort {
            need: SERVICE_LOCATION_PREFIX,
            have: body.len(),
            what,
        });
    }
    let len = u16::from_be_bytes([body[0], body[1]]) as usize;
    let end = SERVICE_LOCATION_PREFIX + len;
    if body.len() < end {
        return Err(Error::LengthMismatch {
            what,
            declared: len,
            actual: body.len().saturating_sub(SERVICE_LOCATION_PREFIX),
        });
    }
    Ok(&body[SERVICE_LOCATION_PREFIX..end])
}

fn write_service_location(loc: &[u8], buf: &mut [u8]) {
    buf[0..2].copy_from_slice(&(loc.len() as u16).to_be_bytes());
    buf[2..2 + loc.len()].copy_from_slice(loc);
}

// ---------------------------------------------------------------------------
// CICAM_player_verify_reply (Table 50)
// ---------------------------------------------------------------------------

/// `CICAM_player_verify_reply()` (Table 50): CICAM → Host.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct PlayerVerifyReply {
    /// `player_verify_status` (8) — Table 51.
    pub player_verify_status: PlayerVerifyStatus,
}

const VERIFY_REPLY_BODY: usize = 1;

impl<'a> Parse<'a> for PlayerVerifyReply {
    type Error = Error;
    fn parse(bytes: &'a [u8]) -> Result<Self> {
        let body =
            objects::parse_apdu_header(bytes, tag::VERIFY_REPLY, "CICAM_player_verify_reply")?;
        if body.len() < VERIFY_REPLY_BODY {
            return Err(Error::BufferTooShort {
                need: VERIFY_REPLY_BODY,
                have: body.len(),
                what: "CICAM_player_verify_reply",
            });
        }
        Ok(Self {
            player_verify_status: PlayerVerifyStatus::from_u8(body[0]),
        })
    }
}
impl Serialize for PlayerVerifyReply {
    type Error = Error;
    fn serialized_len(&self) -> usize {
        objects::apdu_len(VERIFY_REPLY_BODY)
    }
    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        let pos = objects::write_apdu_header(tag::VERIFY_REPLY, VERIFY_REPLY_BODY, buf)?;
        buf[pos] = self.player_verify_status.to_u8();
        Ok(pos + VERIFY_REPLY_BODY)
    }
}

// ---------------------------------------------------------------------------
// CICAM_player_capabilities_req (Table 52)
// ---------------------------------------------------------------------------

/// `CICAM_player_capabilities_req()` (Table 52): CICAM → Host, header-only.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct PlayerCapabilitiesReq;

impl<'a> Parse<'a> for PlayerCapabilitiesReq {
    type Error = Error;
    fn parse(bytes: &'a [u8]) -> Result<Self> {
        objects::parse_empty_apdu(
            bytes,
            tag::CAPABILITIES_REQ,
            "CICAM_player_capabilities_req",
        )?;
        Ok(Self)
    }
}
impl Serialize for PlayerCapabilitiesReq {
    type Error = Error;
    fn serialized_len(&self) -> usize {
        objects::empty_apdu_len()
    }
    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        objects::serialize_empty_apdu(tag::CAPABILITIES_REQ, buf)
    }
}

// ---------------------------------------------------------------------------
// CICAM_player_capabilities_reply (Table 53)
// ---------------------------------------------------------------------------

/// One component-type entry of `CICAM_player_capabilities_reply` (Table 53).
/// `stream_content` / `component_type` are coded as in the EN 300 468 Component
/// descriptor \[10\].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct ComponentType {
    /// `stream_content` (4).
    pub stream_content: u8,
    /// `component_type` (8).
    pub component_type: u8,
}

/// `CICAM_player_capabilities_reply()` (Table 53): Host → CICAM.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct PlayerCapabilitiesReply {
    /// `component_type` list (loop count `number_of_component_types`).
    pub component_types: Vec<ComponentType>,
}

// number_of_component_types(2), then per-entry stream_content/reserved(1)+component_type(1).
const CAPABILITIES_PREFIX: usize = 2;
const COMPONENT_TYPE_LEN: usize = 2;
const STREAM_CONTENT_MASK: u8 = 0x0F;

impl<'a> Parse<'a> for PlayerCapabilitiesReply {
    type Error = Error;
    fn parse(bytes: &'a [u8]) -> Result<Self> {
        let body = objects::parse_apdu_header(
            bytes,
            tag::CAPABILITIES_REPLY,
            "CICAM_player_capabilities_reply",
        )?;
        if body.len() < CAPABILITIES_PREFIX {
            return Err(Error::BufferTooShort {
                need: CAPABILITIES_PREFIX,
                have: body.len(),
                what: "CICAM_player_capabilities_reply",
            });
        }
        let n = u16::from_be_bytes([body[0], body[1]]) as usize;
        let mut pos = CAPABILITIES_PREFIX;
        let mut component_types = Vec::with_capacity(n);
        for _ in 0..n {
            if pos + COMPONENT_TYPE_LEN > body.len() {
                return Err(Error::BufferTooShort {
                    need: pos + COMPONENT_TYPE_LEN,
                    have: body.len(),
                    what: "CICAM_player_capabilities_reply entry",
                });
            }
            component_types.push(ComponentType {
                // stream_content(4) + reserved(4).
                stream_content: (body[pos] >> 4) & STREAM_CONTENT_MASK,
                component_type: body[pos + 1],
            });
            pos += COMPONENT_TYPE_LEN;
        }
        Ok(Self { component_types })
    }
}
impl Serialize for PlayerCapabilitiesReply {
    type Error = Error;
    fn serialized_len(&self) -> usize {
        objects::apdu_len(CAPABILITIES_PREFIX + self.component_types.len() * COMPONENT_TYPE_LEN)
    }
    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        let body_len = CAPABILITIES_PREFIX + self.component_types.len() * COMPONENT_TYPE_LEN;
        let mut pos = objects::write_apdu_header(tag::CAPABILITIES_REPLY, body_len, buf)?;
        buf[pos..pos + 2].copy_from_slice(&(self.component_types.len() as u16).to_be_bytes());
        pos += CAPABILITIES_PREFIX;
        for c in &self.component_types {
            // stream_content(4) << 4 + reserved(4)='1111'.
            buf[pos] = ((c.stream_content & STREAM_CONTENT_MASK) << 4) | 0x0F;
            buf[pos + 1] = c.component_type;
            pos += COMPONENT_TYPE_LEN;
        }
        Ok(pos)
    }
}

// ---------------------------------------------------------------------------
// CICAM_player_start_req (Table 54)
// ---------------------------------------------------------------------------

/// `CICAM_player_start_req()` (Table 54): CICAM → Host.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct PlayerStartReq<'a> {
    /// `input_max_bitrate` (16) — Host→CICAM delivery, units of 10 kbps (rounded up).
    pub input_max_bitrate: u16,
    /// `output_max_bitrate` (16) — CICAM→Host delivery, units of 10 kbps.
    pub output_max_bitrate: u16,
    /// `linearChannel` (1) — set = linear channel with no timeshift.
    pub linear_channel: bool,
    /// `PMT_byte` body (`PMT_length` bytes) — opaque PMT (first byte = `table_id`).
    #[cfg_attr(feature = "serde", serde(borrow, with = "crate::objects::bytes_serde"))]
    pub pmt: &'a [u8],
}

// input_max_bitrate(2)+output_max_bitrate(2)+linearChannel/reserved(1)+PMT_length(2).
const START_REQ_PREFIX: usize = 2 + 2 + 1 + 2;
const LINEAR_CHANNEL_BIT: u8 = 0x80;

impl<'a> Parse<'a> for PlayerStartReq<'a> {
    type Error = Error;
    fn parse(bytes: &'a [u8]) -> Result<Self> {
        let body = objects::parse_apdu_header(bytes, tag::START_REQ, "CICAM_player_start_req")?;
        if body.len() < START_REQ_PREFIX {
            return Err(Error::BufferTooShort {
                need: START_REQ_PREFIX,
                have: body.len(),
                what: "CICAM_player_start_req",
            });
        }
        let input_max_bitrate = u16::from_be_bytes([body[0], body[1]]);
        let output_max_bitrate = u16::from_be_bytes([body[2], body[3]]);
        let linear_channel = body[4] & LINEAR_CHANNEL_BIT != 0;
        let pmt_length = u16::from_be_bytes([body[5], body[6]]) as usize;
        let end = START_REQ_PREFIX + pmt_length;
        if body.len() < end {
            return Err(Error::LengthMismatch {
                what: "CICAM_player_start_req PMT",
                declared: pmt_length,
                actual: body.len().saturating_sub(START_REQ_PREFIX),
            });
        }
        Ok(Self {
            input_max_bitrate,
            output_max_bitrate,
            linear_channel,
            pmt: &body[START_REQ_PREFIX..end],
        })
    }
}
impl Serialize for PlayerStartReq<'_> {
    type Error = Error;
    fn serialized_len(&self) -> usize {
        objects::apdu_len(START_REQ_PREFIX + self.pmt.len())
    }
    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        let body_len = START_REQ_PREFIX + self.pmt.len();
        let pos = objects::write_apdu_header(tag::START_REQ, body_len, buf)?;
        buf[pos..pos + 2].copy_from_slice(&self.input_max_bitrate.to_be_bytes());
        buf[pos + 2..pos + 4].copy_from_slice(&self.output_max_bitrate.to_be_bytes());
        // linearChannel(1) + reserved(7)='0000000'.
        buf[pos + 4] = if self.linear_channel {
            LINEAR_CHANNEL_BIT
        } else {
            0
        };
        buf[pos + 5..pos + 7].copy_from_slice(&(self.pmt.len() as u16).to_be_bytes());
        buf[pos + START_REQ_PREFIX..pos + body_len].copy_from_slice(self.pmt);
        Ok(pos + body_len)
    }
}

// ---------------------------------------------------------------------------
// CICAM_player_start_reply (Table 55)
// ---------------------------------------------------------------------------

/// `CICAM_player_start_reply()` (Table 55): Host → CICAM.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct PlayerStartReply {
    /// `LTS_id` (8) — Local TS allocated; uniquely identifies the player session.
    /// Ignored if `input_status` is non-zero.
    pub lts_id: u8,
    /// `input_status` (8) — Table 56.
    pub input_status: InputStatus,
}

const START_REPLY_BODY: usize = 2;

impl<'a> Parse<'a> for PlayerStartReply {
    type Error = Error;
    fn parse(bytes: &'a [u8]) -> Result<Self> {
        let body = objects::parse_apdu_header(bytes, tag::START_REPLY, "CICAM_player_start_reply")?;
        if body.len() < START_REPLY_BODY {
            return Err(Error::BufferTooShort {
                need: START_REPLY_BODY,
                have: body.len(),
                what: "CICAM_player_start_reply",
            });
        }
        Ok(Self {
            lts_id: body[0],
            input_status: InputStatus::from_u8(body[1]),
        })
    }
}
impl Serialize for PlayerStartReply {
    type Error = Error;
    fn serialized_len(&self) -> usize {
        objects::apdu_len(START_REPLY_BODY)
    }
    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        let pos = objects::write_apdu_header(tag::START_REPLY, START_REPLY_BODY, buf)?;
        buf[pos] = self.lts_id;
        buf[pos + 1] = self.input_status.to_u8();
        Ok(pos + START_REPLY_BODY)
    }
}

// ---------------------------------------------------------------------------
// CICAM_player_play_req (Table 57)
// ---------------------------------------------------------------------------

/// `CICAM_player_play_req()` (Table 57): Host → CICAM. A length-prefixed
/// `service_location` XML blob (annex D schema).
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct PlayerPlayReq<'a> {
    /// `service_location_byte` body (`service_location_length` bytes) — opaque XML.
    #[cfg_attr(feature = "serde", serde(borrow, with = "crate::objects::bytes_serde"))]
    pub service_location: &'a [u8],
}

impl<'a> Parse<'a> for PlayerPlayReq<'a> {
    type Error = Error;
    fn parse(bytes: &'a [u8]) -> Result<Self> {
        let body = objects::parse_apdu_header(bytes, tag::PLAY_REQ, "CICAM_player_play_req")?;
        let service_location = parse_service_location(body, "CICAM_player_play_req")?;
        Ok(Self { service_location })
    }
}
impl Serialize for PlayerPlayReq<'_> {
    type Error = Error;
    fn serialized_len(&self) -> usize {
        objects::apdu_len(SERVICE_LOCATION_PREFIX + self.service_location.len())
    }
    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        let body_len = SERVICE_LOCATION_PREFIX + self.service_location.len();
        let pos = objects::write_apdu_header(tag::PLAY_REQ, body_len, buf)?;
        write_service_location(self.service_location, &mut buf[pos..]);
        Ok(pos + body_len)
    }
}

// ---------------------------------------------------------------------------
// CICAM_player_status_error (Table 58)
// ---------------------------------------------------------------------------

/// `CICAM_player_status_error()` (Table 58): CICAM → Host.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct PlayerStatusError {
    /// `valid_LTS_id` (1) — `true` when `lts_id` relates to an established session.
    pub valid_lts_id: bool,
    /// `LTS_id` (8) — undefined when `valid_lts_id` is `false`.
    pub lts_id: u8,
    /// `player_status` (8) — Table 59.
    pub player_status: PlayStatus,
}

// reserved(7)+valid_LTS_id(1) + LTS_id(8) + player_status(8).
const STATUS_ERROR_BODY: usize = 3;
const VALID_LTS_ID_BIT: u8 = 0x01;

impl<'a> Parse<'a> for PlayerStatusError {
    type Error = Error;
    fn parse(bytes: &'a [u8]) -> Result<Self> {
        let body =
            objects::parse_apdu_header(bytes, tag::STATUS_ERROR, "CICAM_player_status_error")?;
        if body.len() < STATUS_ERROR_BODY {
            return Err(Error::BufferTooShort {
                need: STATUS_ERROR_BODY,
                have: body.len(),
                what: "CICAM_player_status_error",
            });
        }
        Ok(Self {
            valid_lts_id: body[0] & VALID_LTS_ID_BIT != 0,
            lts_id: body[1],
            player_status: PlayStatus::from_u8(body[2]),
        })
    }
}
impl Serialize for PlayerStatusError {
    type Error = Error;
    fn serialized_len(&self) -> usize {
        objects::apdu_len(STATUS_ERROR_BODY)
    }
    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        let pos = objects::write_apdu_header(tag::STATUS_ERROR, STATUS_ERROR_BODY, buf)?;
        // reserved(7)='0000000' + valid_LTS_id(1).
        buf[pos] = if self.valid_lts_id {
            VALID_LTS_ID_BIT
        } else {
            0
        };
        buf[pos + 1] = self.lts_id;
        buf[pos + 2] = self.player_status.to_u8();
        Ok(pos + STATUS_ERROR_BODY)
    }
}

// ---------------------------------------------------------------------------
// CICAM_player_control_req (Table 60)
// ---------------------------------------------------------------------------

/// The `Command`-selected payload of `CICAM_player_control_req` (Tables 60/61).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub enum ControlCommand {
    /// `command == 0x01` — Set position.
    SetPosition {
        /// `seek_mode` (8) — Table 62.
        seek_mode: SeekMode,
        /// `seek_position` (32, signed) — milliseconds; `0xFFFFFFFF` = jump to live / end.
        seek_position: i32,
    },
    /// `command == 0x02` — Set speed.
    SetSpeed {
        /// `Speed` (16, signed) — hundredths of nominal (100 = nominal, 0 = pause).
        speed: i16,
    },
    /// Reserved/unknown `command` value with no further payload (e.g. `0x00`).
    Reserved(u8),
}
impl ControlCommand {
    /// The `Command` byte (Table 61) this payload encodes.
    #[must_use]
    pub fn command(&self) -> u8 {
        match self {
            Self::SetPosition { .. } => 0x01,
            Self::SetSpeed { .. } => 0x02,
            Self::Reserved(v) => *v,
        }
    }
}

/// `CICAM_player_control_req()` (Table 60): Host → CICAM.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct PlayerControlReq {
    /// `LTS_id` (8).
    pub lts_id: u8,
    /// The `Command`-selected payload.
    pub command: ControlCommand,
}

// LTS_id(1) + Command(1) [+ command-specific].
const CONTROL_PREFIX: usize = 2;
const CONTROL_CMD_SET_POSITION: u8 = 0x01;
const CONTROL_CMD_SET_SPEED: u8 = 0x02;
// SetPosition: seek_mode(1) + seek_position(4); SetSpeed: Speed(2).
const SET_POSITION_EXTRA: usize = 1 + 4;
const SET_SPEED_EXTRA: usize = 2;

impl PlayerControlReq {
    fn body_len(&self) -> usize {
        CONTROL_PREFIX
            + match self.command {
                ControlCommand::SetPosition { .. } => SET_POSITION_EXTRA,
                ControlCommand::SetSpeed { .. } => SET_SPEED_EXTRA,
                ControlCommand::Reserved(_) => 0,
            }
    }
}

impl<'a> Parse<'a> for PlayerControlReq {
    type Error = Error;
    fn parse(bytes: &'a [u8]) -> Result<Self> {
        let body = objects::parse_apdu_header(bytes, tag::CONTROL_REQ, "CICAM_player_control_req")?;
        if body.len() < CONTROL_PREFIX {
            return Err(Error::BufferTooShort {
                need: CONTROL_PREFIX,
                have: body.len(),
                what: "CICAM_player_control_req",
            });
        }
        let lts_id = body[0];
        let command = match body[1] {
            CONTROL_CMD_SET_POSITION => {
                if body.len() < CONTROL_PREFIX + SET_POSITION_EXTRA {
                    return Err(Error::BufferTooShort {
                        need: CONTROL_PREFIX + SET_POSITION_EXTRA,
                        have: body.len(),
                        what: "CICAM_player_control_req set_position",
                    });
                }
                ControlCommand::SetPosition {
                    seek_mode: SeekMode::from_u8(body[2]),
                    seek_position: i32::from_be_bytes([body[3], body[4], body[5], body[6]]),
                }
            }
            CONTROL_CMD_SET_SPEED => {
                if body.len() < CONTROL_PREFIX + SET_SPEED_EXTRA {
                    return Err(Error::BufferTooShort {
                        need: CONTROL_PREFIX + SET_SPEED_EXTRA,
                        have: body.len(),
                        what: "CICAM_player_control_req set_speed",
                    });
                }
                ControlCommand::SetSpeed {
                    speed: i16::from_be_bytes([body[2], body[3]]),
                }
            }
            other => ControlCommand::Reserved(other),
        };
        Ok(Self { lts_id, command })
    }
}
impl Serialize for PlayerControlReq {
    type Error = Error;
    fn serialized_len(&self) -> usize {
        objects::apdu_len(self.body_len())
    }
    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        let body_len = self.body_len();
        let pos = objects::write_apdu_header(tag::CONTROL_REQ, body_len, buf)?;
        buf[pos] = self.lts_id;
        buf[pos + 1] = self.command.command();
        match self.command {
            ControlCommand::SetPosition {
                seek_mode,
                seek_position,
            } => {
                buf[pos + 2] = seek_mode.to_u8();
                buf[pos + 3..pos + 7].copy_from_slice(&seek_position.to_be_bytes());
            }
            ControlCommand::SetSpeed { speed } => {
                buf[pos + 2..pos + 4].copy_from_slice(&speed.to_be_bytes());
            }
            ControlCommand::Reserved(_) => {}
        }
        Ok(pos + body_len)
    }
}

// ---------------------------------------------------------------------------
// CICAM_player_info_req (Table 63)
// ---------------------------------------------------------------------------

/// `CICAM_player_info_req()` (Table 63): Host → CICAM.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct PlayerInfoReq {
    /// `LTS_id` (8).
    pub lts_id: u8,
}

const LTS_ID_BODY: usize = 1;

impl<'a> Parse<'a> for PlayerInfoReq {
    type Error = Error;
    fn parse(bytes: &'a [u8]) -> Result<Self> {
        Ok(Self {
            lts_id: parse_lts_id_body(bytes, tag::INFO_REQ, "CICAM_player_info_req")?,
        })
    }
}
impl Serialize for PlayerInfoReq {
    type Error = Error;
    fn serialized_len(&self) -> usize {
        objects::apdu_len(LTS_ID_BODY)
    }
    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        serialize_lts_id_body(self.lts_id, tag::INFO_REQ, buf)
    }
}

fn parse_lts_id_body(bytes: &[u8], expected: ApduTag, what: &'static str) -> Result<u8> {
    let body = objects::parse_apdu_header(bytes, expected, what)?;
    if body.len() < LTS_ID_BODY {
        return Err(Error::BufferTooShort {
            need: LTS_ID_BODY,
            have: body.len(),
            what,
        });
    }
    Ok(body[0])
}

fn serialize_lts_id_body(lts_id: u8, tag: ApduTag, buf: &mut [u8]) -> Result<usize> {
    let pos = objects::write_apdu_header(tag, LTS_ID_BODY, buf)?;
    buf[pos] = lts_id;
    Ok(pos + LTS_ID_BODY)
}

// ---------------------------------------------------------------------------
// CICAM_player_info_reply (Table 64)
// ---------------------------------------------------------------------------

/// `CICAM_player_info_reply()` (Table 64): CICAM → Host.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct PlayerInfoReply {
    /// `LTS_id` (8).
    pub lts_id: u8,
    /// `duration` (32) — total content duration in seconds; `0xFFFFFFFF` if unknown.
    pub duration: u32,
    /// `position` (32) — current play position in seconds; `0xFFFFFFFF` if unknown.
    pub position: u32,
    /// `speed` (16, signed) — current playout speed in hundredths of nominal.
    pub speed: i16,
}

// LTS_id(1) + duration(4) + position(4) + speed(2).
const INFO_REPLY_BODY: usize = 1 + 4 + 4 + 2;

impl<'a> Parse<'a> for PlayerInfoReply {
    type Error = Error;
    fn parse(bytes: &'a [u8]) -> Result<Self> {
        let body = objects::parse_apdu_header(bytes, tag::INFO_REPLY, "CICAM_player_info_reply")?;
        if body.len() < INFO_REPLY_BODY {
            return Err(Error::BufferTooShort {
                need: INFO_REPLY_BODY,
                have: body.len(),
                what: "CICAM_player_info_reply",
            });
        }
        Ok(Self {
            lts_id: body[0],
            duration: u32::from_be_bytes([body[1], body[2], body[3], body[4]]),
            position: u32::from_be_bytes([body[5], body[6], body[7], body[8]]),
            speed: i16::from_be_bytes([body[9], body[10]]),
        })
    }
}
impl Serialize for PlayerInfoReply {
    type Error = Error;
    fn serialized_len(&self) -> usize {
        objects::apdu_len(INFO_REPLY_BODY)
    }
    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        let pos = objects::write_apdu_header(tag::INFO_REPLY, INFO_REPLY_BODY, buf)?;
        buf[pos] = self.lts_id;
        buf[pos + 1..pos + 5].copy_from_slice(&self.duration.to_be_bytes());
        buf[pos + 5..pos + 9].copy_from_slice(&self.position.to_be_bytes());
        buf[pos + 9..pos + 11].copy_from_slice(&self.speed.to_be_bytes());
        Ok(pos + INFO_REPLY_BODY)
    }
}

// ---------------------------------------------------------------------------
// CICAM_player_stop (Table 65) / CICAM_player_end (Table 66)
// ---------------------------------------------------------------------------

/// `CICAM_player_stop()` (Table 65): Host → CICAM.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct PlayerStop {
    /// `LTS_id` (8).
    pub lts_id: u8,
}

impl<'a> Parse<'a> for PlayerStop {
    type Error = Error;
    fn parse(bytes: &'a [u8]) -> Result<Self> {
        Ok(Self {
            lts_id: parse_lts_id_body(bytes, tag::STOP, "CICAM_player_stop")?,
        })
    }
}
impl Serialize for PlayerStop {
    type Error = Error;
    fn serialized_len(&self) -> usize {
        objects::apdu_len(LTS_ID_BODY)
    }
    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        serialize_lts_id_body(self.lts_id, tag::STOP, buf)
    }
}

/// `CICAM_player_end()` (Table 66): CICAM → Host.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct PlayerEnd {
    /// `LTS_id` (8).
    pub lts_id: u8,
}

impl<'a> Parse<'a> for PlayerEnd {
    type Error = Error;
    fn parse(bytes: &'a [u8]) -> Result<Self> {
        Ok(Self {
            lts_id: parse_lts_id_body(bytes, tag::END, "CICAM_player_end")?,
        })
    }
}
impl Serialize for PlayerEnd {
    type Error = Error;
    fn serialized_len(&self) -> usize {
        objects::apdu_len(LTS_ID_BODY)
    }
    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        serialize_lts_id_body(self.lts_id, tag::END, buf)
    }
}

// ---------------------------------------------------------------------------
// CICAM_player_asset_end (Table 67)
// ---------------------------------------------------------------------------

/// `CICAM_player_asset_end()` (Table 67): CICAM → Host.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct PlayerAssetEnd {
    /// `LTS_id` (8).
    pub lts_id: u8,
    /// `beginning` (1) — `true` = start of asset reached; otherwise end reached.
    pub beginning: bool,
}

// LTS_id(1) + reserved(7)+beginning(1).
const ASSET_END_BODY: usize = 2;
const BEGINNING_BIT: u8 = 0x01;
// reserved field shall be 0x7F per §8.8.16.
const ASSET_END_RESERVED: u8 = 0x7F << 1;

impl<'a> Parse<'a> for PlayerAssetEnd {
    type Error = Error;
    fn parse(bytes: &'a [u8]) -> Result<Self> {
        let body = objects::parse_apdu_header(bytes, tag::ASSET_END, "CICAM_player_asset_end")?;
        if body.len() < ASSET_END_BODY {
            return Err(Error::BufferTooShort {
                need: ASSET_END_BODY,
                have: body.len(),
                what: "CICAM_player_asset_end",
            });
        }
        Ok(Self {
            lts_id: body[0],
            beginning: body[1] & BEGINNING_BIT != 0,
        })
    }
}
impl Serialize for PlayerAssetEnd {
    type Error = Error;
    fn serialized_len(&self) -> usize {
        objects::apdu_len(ASSET_END_BODY)
    }
    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        let pos = objects::write_apdu_header(tag::ASSET_END, ASSET_END_BODY, buf)?;
        buf[pos] = self.lts_id;
        // reserved(7) shall be 0x7F + beginning(1).
        buf[pos + 1] = ASSET_END_RESERVED | u8::from(self.beginning);
        Ok(pos + ASSET_END_BODY)
    }
}

// ---------------------------------------------------------------------------
// CICAM_player_update_req (Table 68)
// ---------------------------------------------------------------------------

/// `CICAM_player_update_req()` (Table 68): CICAM → Host.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct PlayerUpdateReq<'a> {
    /// `LTS_id` (8).
    pub lts_id: u8,
    /// `PMT_byte` body (`PMT_length` bytes, shall not be zero) — opaque PMT.
    #[cfg_attr(feature = "serde", serde(borrow, with = "crate::objects::bytes_serde"))]
    pub pmt: &'a [u8],
}

// LTS_id(1) + PMT_length(2) + PMT bytes.
const UPDATE_REQ_PREFIX: usize = 1 + 2;

impl<'a> Parse<'a> for PlayerUpdateReq<'a> {
    type Error = Error;
    fn parse(bytes: &'a [u8]) -> Result<Self> {
        let body = objects::parse_apdu_header(bytes, tag::UPDATE_REQ, "CICAM_player_update_req")?;
        if body.len() < UPDATE_REQ_PREFIX {
            return Err(Error::BufferTooShort {
                need: UPDATE_REQ_PREFIX,
                have: body.len(),
                what: "CICAM_player_update_req",
            });
        }
        let lts_id = body[0];
        let pmt_length = u16::from_be_bytes([body[1], body[2]]) as usize;
        let end = UPDATE_REQ_PREFIX + pmt_length;
        if body.len() < end {
            return Err(Error::LengthMismatch {
                what: "CICAM_player_update_req PMT",
                declared: pmt_length,
                actual: body.len().saturating_sub(UPDATE_REQ_PREFIX),
            });
        }
        Ok(Self {
            lts_id,
            pmt: &body[UPDATE_REQ_PREFIX..end],
        })
    }
}
impl Serialize for PlayerUpdateReq<'_> {
    type Error = Error;
    fn serialized_len(&self) -> usize {
        objects::apdu_len(UPDATE_REQ_PREFIX + self.pmt.len())
    }
    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        let body_len = UPDATE_REQ_PREFIX + self.pmt.len();
        let pos = objects::write_apdu_header(tag::UPDATE_REQ, body_len, buf)?;
        buf[pos] = self.lts_id;
        buf[pos + 1..pos + 3].copy_from_slice(&(self.pmt.len() as u16).to_be_bytes());
        buf[pos + UPDATE_REQ_PREFIX..pos + body_len].copy_from_slice(self.pmt);
        Ok(pos + body_len)
    }
}

// ---------------------------------------------------------------------------
// CICAM_player_update_reply (Table 69)
// ---------------------------------------------------------------------------

/// `CICAM_player_update_reply()` (Table 69): Host → CICAM.
///
/// NB: Table 69 prints a copy/paste `CICAM_player_start_reply_tag` slip; the
/// authoritative tag is `0x9FA00F` ([`tag::UPDATE_REPLY`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct PlayerUpdateReply {
    /// `LTS_id` (8).
    pub lts_id: u8,
    /// `update_status` (8) — Table 70.
    pub update_status: UpdateStatus,
}

const UPDATE_REPLY_BODY: usize = 2;

impl<'a> Parse<'a> for PlayerUpdateReply {
    type Error = Error;
    fn parse(bytes: &'a [u8]) -> Result<Self> {
        let body =
            objects::parse_apdu_header(bytes, tag::UPDATE_REPLY, "CICAM_player_update_reply")?;
        if body.len() < UPDATE_REPLY_BODY {
            return Err(Error::BufferTooShort {
                need: UPDATE_REPLY_BODY,
                have: body.len(),
                what: "CICAM_player_update_reply",
            });
        }
        Ok(Self {
            lts_id: body[0],
            update_status: UpdateStatus::from_u8(body[1]),
        })
    }
}
impl Serialize for PlayerUpdateReply {
    type Error = Error;
    fn serialized_len(&self) -> usize {
        objects::apdu_len(UPDATE_REPLY_BODY)
    }
    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        let pos = objects::write_apdu_header(tag::UPDATE_REPLY, UPDATE_REPLY_BODY, buf)?;
        buf[pos] = self.lts_id;
        buf[pos + 1] = self.update_status.to_u8();
        Ok(pos + UPDATE_REPLY_BODY)
    }
}

// ---------------------------------------------------------------------------
// Resource-scoped dispatch
// ---------------------------------------------------------------------------

/// Resource-scoped dispatch over the CICAM Player resource objects (Table 71).
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub enum CicamPlayerApdu<'a> {
    /// `CICAM_player_verify_req` (`9F A0 00`).
    VerifyReq(#[cfg_attr(feature = "serde", serde(borrow))] PlayerVerifyReq<'a>),
    /// `CICAM_player_verify_reply` (`9F A0 01`).
    VerifyReply(PlayerVerifyReply),
    /// `CICAM_player_capabilities_req` (`9F A0 02`).
    CapabilitiesReq(PlayerCapabilitiesReq),
    /// `CICAM_player_capabilities_reply` (`9F A0 03`).
    CapabilitiesReply(PlayerCapabilitiesReply),
    /// `CICAM_player_start_req` (`9F A0 04`).
    StartReq(#[cfg_attr(feature = "serde", serde(borrow))] PlayerStartReq<'a>),
    /// `CICAM_player_start_reply` (`9F A0 05`).
    StartReply(PlayerStartReply),
    /// `CICAM_player_play_req` (`9F A0 06`).
    PlayReq(#[cfg_attr(feature = "serde", serde(borrow))] PlayerPlayReq<'a>),
    /// `CICAM_player_status_error` (`9F A0 07`).
    StatusError(PlayerStatusError),
    /// `CICAM_player_control_req` (`9F A0 08`).
    ControlReq(PlayerControlReq),
    /// `CICAM_player_info_req` (`9F A0 09`).
    InfoReq(PlayerInfoReq),
    /// `CICAM_player_info_reply` (`9F A0 0A`).
    InfoReply(PlayerInfoReply),
    /// `CICAM_player_stop` (`9F A0 0B`).
    Stop(PlayerStop),
    /// `CICAM_player_end` (`9F A0 0C`).
    End(PlayerEnd),
    /// `CICAM_player_asset_end` (`9F A0 0D`).
    AssetEnd(PlayerAssetEnd),
    /// `CICAM_player_update_req` (`9F A0 0E`).
    UpdateReq(#[cfg_attr(feature = "serde", serde(borrow))] PlayerUpdateReq<'a>),
    /// `CICAM_player_update_reply` (`9F A0 0F`).
    UpdateReply(PlayerUpdateReply),
}

impl<'a> CicamPlayerApdu<'a> {
    /// Parse a CICAM Player APDU, dispatching on the leading `apdu_tag`.
    pub fn parse(body: &'a [u8]) -> Result<Self> {
        if body.len() < 3 {
            return Err(Error::BufferTooShort {
                need: 3,
                have: body.len(),
                what: "cicam_player apdu_tag",
            });
        }
        let t = ApduTag::from_bytes(body[0], body[1], body[2]);
        match t {
            tag::VERIFY_REQ => Ok(Self::VerifyReq(PlayerVerifyReq::parse(body)?)),
            tag::VERIFY_REPLY => Ok(Self::VerifyReply(PlayerVerifyReply::parse(body)?)),
            tag::CAPABILITIES_REQ => Ok(Self::CapabilitiesReq(PlayerCapabilitiesReq::parse(body)?)),
            tag::CAPABILITIES_REPLY => Ok(Self::CapabilitiesReply(PlayerCapabilitiesReply::parse(
                body,
            )?)),
            tag::START_REQ => Ok(Self::StartReq(PlayerStartReq::parse(body)?)),
            tag::START_REPLY => Ok(Self::StartReply(PlayerStartReply::parse(body)?)),
            tag::PLAY_REQ => Ok(Self::PlayReq(PlayerPlayReq::parse(body)?)),
            tag::STATUS_ERROR => Ok(Self::StatusError(PlayerStatusError::parse(body)?)),
            tag::CONTROL_REQ => Ok(Self::ControlReq(PlayerControlReq::parse(body)?)),
            tag::INFO_REQ => Ok(Self::InfoReq(PlayerInfoReq::parse(body)?)),
            tag::INFO_REPLY => Ok(Self::InfoReply(PlayerInfoReply::parse(body)?)),
            tag::STOP => Ok(Self::Stop(PlayerStop::parse(body)?)),
            tag::END => Ok(Self::End(PlayerEnd::parse(body)?)),
            tag::ASSET_END => Ok(Self::AssetEnd(PlayerAssetEnd::parse(body)?)),
            tag::UPDATE_REQ => Ok(Self::UpdateReq(PlayerUpdateReq::parse(body)?)),
            tag::UPDATE_REPLY => Ok(Self::UpdateReply(PlayerUpdateReply::parse(body)?)),
            _ => Err(Error::UnexpectedApduTag {
                got: t.as_u24(),
                expected: tag::VERIFY_REQ.as_u24(),
                what: "cicam_player",
            }),
        }
    }
}

impl Serialize for CicamPlayerApdu<'_> {
    type Error = Error;
    fn serialized_len(&self) -> usize {
        match self {
            Self::VerifyReq(o) => o.serialized_len(),
            Self::VerifyReply(o) => o.serialized_len(),
            Self::CapabilitiesReq(o) => o.serialized_len(),
            Self::CapabilitiesReply(o) => o.serialized_len(),
            Self::StartReq(o) => o.serialized_len(),
            Self::StartReply(o) => o.serialized_len(),
            Self::PlayReq(o) => o.serialized_len(),
            Self::StatusError(o) => o.serialized_len(),
            Self::ControlReq(o) => o.serialized_len(),
            Self::InfoReq(o) => o.serialized_len(),
            Self::InfoReply(o) => o.serialized_len(),
            Self::Stop(o) => o.serialized_len(),
            Self::End(o) => o.serialized_len(),
            Self::AssetEnd(o) => o.serialized_len(),
            Self::UpdateReq(o) => o.serialized_len(),
            Self::UpdateReply(o) => o.serialized_len(),
        }
    }
    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        match self {
            Self::VerifyReq(o) => o.serialize_into(buf),
            Self::VerifyReply(o) => o.serialize_into(buf),
            Self::CapabilitiesReq(o) => o.serialize_into(buf),
            Self::CapabilitiesReply(o) => o.serialize_into(buf),
            Self::StartReq(o) => o.serialize_into(buf),
            Self::StartReply(o) => o.serialize_into(buf),
            Self::PlayReq(o) => o.serialize_into(buf),
            Self::StatusError(o) => o.serialize_into(buf),
            Self::ControlReq(o) => o.serialize_into(buf),
            Self::InfoReq(o) => o.serialize_into(buf),
            Self::InfoReply(o) => o.serialize_into(buf),
            Self::Stop(o) => o.serialize_into(buf),
            Self::End(o) => o.serialize_into(buf),
            Self::AssetEnd(o) => o.serialize_into(buf),
            Self::UpdateReq(o) => o.serialize_into(buf),
            Self::UpdateReply(o) => o.serialize_into(buf),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn verify_req_round_trips_and_bites() {
        let req = PlayerVerifyReq {
            service_location: &[0x3C, 0x53, 0x4C, 0x3E], // "<SL>"
        };
        let bytes = req.to_bytes();
        // tag(9F A0 00) len(06) svc_len(00 04) 3C 53 4C 3E.
        assert_eq!(
            bytes,
            [0x9F, 0xA0, 0x00, 0x06, 0x00, 0x04, 0x3C, 0x53, 0x4C, 0x3E]
        );
        assert_eq!(PlayerVerifyReq::parse(&bytes).unwrap(), req);
        let other = PlayerVerifyReq {
            service_location: &[0x3C, 0x53, 0x4C, 0x00],
        };
        assert_ne!(bytes, other.to_bytes());
    }

    #[test]
    fn verify_reply_round_trips() {
        let r = PlayerVerifyReply {
            player_verify_status: PlayerVerifyStatus::Error,
        };
        let bytes = r.to_bytes();
        assert_eq!(bytes, [0x9F, 0xA0, 0x01, 0x01, 0x01]);
        assert_eq!(PlayerVerifyReply::parse(&bytes).unwrap(), r);
    }

    #[test]
    fn capabilities_req_round_trips() {
        let bytes = PlayerCapabilitiesReq.to_bytes();
        assert_eq!(bytes, [0x9F, 0xA0, 0x02, 0x00]);
        assert_eq!(
            PlayerCapabilitiesReq::parse(&bytes).unwrap(),
            PlayerCapabilitiesReq
        );
    }

    #[test]
    fn capabilities_reply_two_entries() {
        let r = PlayerCapabilitiesReply {
            component_types: alloc::vec![
                ComponentType {
                    stream_content: 0x01,
                    component_type: 0x03,
                },
                ComponentType {
                    stream_content: 0x02,
                    component_type: 0x05,
                },
            ],
        };
        let bytes = r.to_bytes();
        // len = 2 + 2*2 = 6. count(00 02) then 0x1F 03 / 0x2F 05.
        assert_eq!(bytes[0..4], [0x9F, 0xA0, 0x03, 0x06]);
        assert_eq!(&bytes[4..6], &[0x00, 0x02]);
        assert_eq!(&bytes[6..10], &[0x1F, 0x03, 0x2F, 0x05]);
        assert_eq!(PlayerCapabilitiesReply::parse(&bytes).unwrap(), r);
        let mut other = r.clone();
        other.component_types[0].component_type = 0x04;
        assert_ne!(bytes, other.to_bytes());
    }

    #[test]
    fn start_req_round_trips_and_bites() {
        // 512 kbps = 0x0034 input; output 0x0040; linear; 3-byte PMT.
        let req = PlayerStartReq {
            input_max_bitrate: 0x0034,
            output_max_bitrate: 0x0040,
            linear_channel: true,
            pmt: &[0x02, 0xB0, 0x12],
        };
        let bytes = req.to_bytes();
        // body = 7 + 3 = 10 = 0x0A.
        assert_eq!(bytes[0..4], [0x9F, 0xA0, 0x04, 0x0A]);
        assert_eq!(&bytes[4..6], &[0x00, 0x34]); // input_max_bitrate
        assert_eq!(&bytes[6..8], &[0x00, 0x40]); // output_max_bitrate
        assert_eq!(bytes[8], 0x80); // linearChannel set
        assert_eq!(&bytes[9..11], &[0x00, 0x03]); // PMT_length
        assert_eq!(&bytes[11..14], &[0x02, 0xB0, 0x12]);
        assert_eq!(PlayerStartReq::parse(&bytes).unwrap(), req);
        let mut other = req.clone();
        other.linear_channel = false;
        assert_eq!(other.to_bytes()[8], 0x00);
        assert_ne!(bytes, other.to_bytes());
    }

    #[test]
    fn start_reply_round_trips() {
        let r = PlayerStartReply {
            lts_id: 0x07,
            input_status: InputStatus::InsufficientBitrate,
        };
        let bytes = r.to_bytes();
        assert_eq!(bytes, [0x9F, 0xA0, 0x05, 0x02, 0x07, 0x02]);
        assert_eq!(PlayerStartReply::parse(&bytes).unwrap(), r);
    }

    #[test]
    fn play_req_round_trips() {
        let req = PlayerPlayReq {
            service_location: &[0xAA, 0xBB],
        };
        let bytes = req.to_bytes();
        assert_eq!(bytes, [0x9F, 0xA0, 0x06, 0x04, 0x00, 0x02, 0xAA, 0xBB]);
        assert_eq!(PlayerPlayReq::parse(&bytes).unwrap(), req);
    }

    #[test]
    fn status_error_round_trips_and_bites() {
        let e = PlayerStatusError {
            valid_lts_id: true,
            lts_id: 0x09,
            player_status: PlayStatus::ContentBlocked,
        };
        let bytes = e.to_bytes();
        // body=3: reserved+valid(01) LTS_id(09) play_status(03).
        assert_eq!(bytes, [0x9F, 0xA0, 0x07, 0x03, 0x01, 0x09, 0x03]);
        assert_eq!(PlayerStatusError::parse(&bytes).unwrap(), e);
        let mut other = e;
        other.valid_lts_id = false;
        assert_eq!(other.to_bytes()[4], 0x00);
        assert_ne!(bytes, other.to_bytes());
    }

    #[test]
    fn control_req_set_position_round_trips_and_bites() {
        let c = PlayerControlReq {
            lts_id: 0x03,
            command: ControlCommand::SetPosition {
                seek_mode: SeekMode::Relative,
                seek_position: -1000,
            },
        };
        let bytes = c.to_bytes();
        // body = LTS_id(03) cmd(01) seek_mode(01) seek_position(-1000 = FF FF FC 18).
        assert_eq!(bytes[0..4], [0x9F, 0xA0, 0x08, 0x07]);
        assert_eq!(bytes[4], 0x03);
        assert_eq!(bytes[5], 0x01); // Command set_position
        assert_eq!(bytes[6], 0x01); // seek_mode relative
        assert_eq!(&bytes[7..11], &(-1000i32).to_be_bytes());
        assert_eq!(PlayerControlReq::parse(&bytes).unwrap(), c);
        let mut other = c;
        other.command = ControlCommand::SetPosition {
            seek_mode: SeekMode::Absolute,
            seek_position: -1000,
        };
        assert_ne!(bytes, other.to_bytes());
    }

    #[test]
    fn control_req_set_speed_round_trips() {
        let c = PlayerControlReq {
            lts_id: 0x01,
            command: ControlCommand::SetSpeed { speed: -100 },
        };
        let bytes = c.to_bytes();
        // body = LTS_id(01) cmd(02) speed(-100 = FF 9C).
        assert_eq!(bytes[0..4], [0x9F, 0xA0, 0x08, 0x04]);
        assert_eq!(bytes[4], 0x01);
        assert_eq!(bytes[5], 0x02);
        assert_eq!(&bytes[6..8], &(-100i16).to_be_bytes());
        assert_eq!(PlayerControlReq::parse(&bytes).unwrap(), c);
    }

    #[test]
    fn info_req_reply_round_trip() {
        let req = PlayerInfoReq { lts_id: 0x05 };
        let rb = req.to_bytes();
        assert_eq!(rb, [0x9F, 0xA0, 0x09, 0x01, 0x05]);
        assert_eq!(PlayerInfoReq::parse(&rb).unwrap(), req);

        let reply = PlayerInfoReply {
            lts_id: 0x05,
            duration: 0xFFFF_FFFF,
            position: 0x0000_003C,
            speed: 100,
        };
        let bytes = reply.to_bytes();
        // body = 11 = 0x0B.
        assert_eq!(bytes[0..4], [0x9F, 0xA0, 0x0A, 0x0B]);
        assert_eq!(bytes[4], 0x05);
        assert_eq!(&bytes[5..9], &[0xFF, 0xFF, 0xFF, 0xFF]);
        assert_eq!(&bytes[9..13], &[0x00, 0x00, 0x00, 0x3C]);
        assert_eq!(&bytes[13..15], &100i16.to_be_bytes());
        assert_eq!(PlayerInfoReply::parse(&bytes).unwrap(), reply);
    }

    #[test]
    fn stop_end_round_trip() {
        let stop = PlayerStop { lts_id: 0x02 };
        assert_eq!(stop.to_bytes(), [0x9F, 0xA0, 0x0B, 0x01, 0x02]);
        assert_eq!(PlayerStop::parse(&stop.to_bytes()).unwrap(), stop);

        let end = PlayerEnd { lts_id: 0x02 };
        assert_eq!(end.to_bytes(), [0x9F, 0xA0, 0x0C, 0x01, 0x02]);
        assert_eq!(PlayerEnd::parse(&end.to_bytes()).unwrap(), end);
    }

    #[test]
    fn asset_end_round_trips_and_reserved_is_7f() {
        let e = PlayerAssetEnd {
            lts_id: 0x04,
            beginning: true,
        };
        let bytes = e.to_bytes();
        // reserved(7)=0x7F, beginning=1 -> 0xFE | 0x01 = 0xFF.
        assert_eq!(bytes, [0x9F, 0xA0, 0x0D, 0x02, 0x04, 0xFF]);
        assert_eq!(PlayerAssetEnd::parse(&bytes).unwrap(), e);
        let other = PlayerAssetEnd {
            lts_id: 0x04,
            beginning: false,
        };
        // reserved still 0x7F<<1 = 0xFE, beginning=0.
        assert_eq!(other.to_bytes(), [0x9F, 0xA0, 0x0D, 0x02, 0x04, 0xFE]);
    }

    #[test]
    fn update_req_round_trips_two_byte_pmt() {
        let req = PlayerUpdateReq {
            lts_id: 0x08,
            pmt: &[0x02, 0xB0],
        };
        let bytes = req.to_bytes();
        // body = 3 + 2 = 5. LTS_id(08) PMT_length(00 02) 02 B0.
        assert_eq!(
            bytes,
            [0x9F, 0xA0, 0x0E, 0x05, 0x08, 0x00, 0x02, 0x02, 0xB0]
        );
        assert_eq!(PlayerUpdateReq::parse(&bytes).unwrap(), req);
    }

    #[test]
    fn update_reply_uses_9fa00f_not_start_reply_slip() {
        let r = PlayerUpdateReply {
            lts_id: 0x06,
            update_status: UpdateStatus::RequestRefused,
        };
        let bytes = r.to_bytes();
        // The authoritative tag is 0x9FA00F, NOT the Table-69 0x9FA005 slip.
        assert_eq!(&bytes[0..3], &[0x9F, 0xA0, 0x0F]);
        assert_ne!(&bytes[0..3], &[0x9F, 0xA0, 0x05]); // not the start_reply slip tag
        assert_eq!(bytes, [0x9F, 0xA0, 0x0F, 0x02, 0x06, 0x01]);
        assert_eq!(PlayerUpdateReply::parse(&bytes).unwrap(), r);
        assert_eq!(tag::UPDATE_REPLY.as_u24(), 0x009F_A00F);
    }

    #[test]
    fn dispatch_routes_every_tag() {
        let cases: alloc::vec::Vec<alloc::vec::Vec<u8>> = alloc::vec![
            PlayerVerifyReq {
                service_location: &[]
            }
            .to_bytes(),
            PlayerVerifyReply {
                player_verify_status: PlayerVerifyStatus::Ok
            }
            .to_bytes(),
            PlayerCapabilitiesReq.to_bytes(),
            PlayerCapabilitiesReply {
                component_types: alloc::vec![]
            }
            .to_bytes(),
            PlayerStartReq {
                input_max_bitrate: 0,
                output_max_bitrate: 0,
                linear_channel: false,
                pmt: &[]
            }
            .to_bytes(),
            PlayerStartReply {
                lts_id: 0,
                input_status: InputStatus::Ok
            }
            .to_bytes(),
            PlayerPlayReq {
                service_location: &[]
            }
            .to_bytes(),
            PlayerStatusError {
                valid_lts_id: false,
                lts_id: 0,
                player_status: PlayStatus::Unrecoverable
            }
            .to_bytes(),
            PlayerControlReq {
                lts_id: 0,
                command: ControlCommand::SetSpeed { speed: 0 }
            }
            .to_bytes(),
            PlayerInfoReq { lts_id: 0 }.to_bytes(),
            PlayerInfoReply {
                lts_id: 0,
                duration: 0,
                position: 0,
                speed: 0
            }
            .to_bytes(),
            PlayerStop { lts_id: 0 }.to_bytes(),
            PlayerEnd { lts_id: 0 }.to_bytes(),
            PlayerAssetEnd {
                lts_id: 0,
                beginning: false
            }
            .to_bytes(),
            PlayerUpdateReq {
                lts_id: 0,
                pmt: &[0x00]
            }
            .to_bytes(),
            PlayerUpdateReply {
                lts_id: 0,
                update_status: UpdateStatus::Ok
            }
            .to_bytes(),
        ];
        for c in &cases {
            let parsed = CicamPlayerApdu::parse(c).unwrap();
            assert_eq!(&parsed.to_bytes(), c);
        }
        // Unknown tag in the 0x9FA0xx space.
        assert!(matches!(
            CicamPlayerApdu::parse(&[0x9F, 0xA0, 0x7E, 0x00]),
            Err(Error::UnexpectedApduTag { .. })
        ));
    }
}
