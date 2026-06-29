//! Multi-stream Host Control resource objects — ETSI TS 103 205 V1.4.1 §6.4.5,
//! Tables 17-22 (PDF pp. 40-44); base DVB Host Control v3 tune APDUs §13.2,
//! Tables 97-102 (PDF pp. 126-130). See
//! `docs/ts_103_205/multi-stream-host-control.md`.
//!
//! The multi-stream Host Control resource (**resource ID `0x00200081`**) is based
//! on DVB Host Control version 3 (**resource ID `0x00200043`**). The multi-stream
//! tune APDUs add a `background_tune_flag` to the base-v3 layouts; all other APDUs
//! retain the v3 syntax.
//!
//! Tune-request APDUs implemented (tags inherited from the v3 resource, §13.2):
//!
//! - `tune_triplet_req` (`9F 84 09`, Tables 19/98) — CICAM → Host.
//! - `tune_lcn_req` (`9F 84 07`, Tables 20/99) — CICAM → Host.
//! - `tune_ip_req` (`9F 84 08`, Tables 21/100) — CICAM → Host.
//! - `tuner_status_req` (`9F 84 0A`, Table 101) — CICAM → Host, header-only.
//! - `tuner_status_reply` (`9F 84 0B`, Table 102) — Host → CICAM.
//!
//! ## The two resource ids and the `tune_ip_req` divergence
//!
//! The same tune objects appear under two resource ids that differ only in the bit
//! budget of the byte preceding the three tune flags:
//!
//! - **Multi-stream** (`0x00200081`, [`HostControlMode::MultiStream`]) — Table 21
//!   `tune_ip_req` is `reserved(1)` + `background_tune_flag(1)` + `tune_quietly(1)`
//!   + `keep_app_running(1)` + `service_location_length(12)`.
//! - **Base v3** (`0x00200043`, [`HostControlMode::BaseV3`]) — Table 100
//!   `tune_ip_req` is `reserved(2)` + `tune_quietly(1)` + `keep_app_running(1)` +
//!   `service_location_length(12)` (no `background_tune_flag`).
//!
//! Both layouts pack the four flag/reserved bits into the high nibble of the byte
//! that begins the 12-bit `service_location_length`. The two render to **distinct**
//! byte patterns whenever `background_tune_flag` is set, so the mode is carried on
//! [`TuneIpReq::mode`] and the parser is told which resource it arrived on.
//!
//! `tune_broadcast_req`, `tune_reply`, `ask_release` and `ask_release_reply` tags
//! and bodies are **deferred to CI Plus V1.3** (their tags are not numerically
//! printed in TS 103 205) — they are intentionally NOT implemented here.

use crate::error::{Error, Result};
use crate::objects;
use crate::tag::ApduTag;
use alloc::vec::Vec;
use broadcast_common::{Parse, Serialize};

/// Resource-scoped `apdu_tag`s, inherited from the DVB Host Control v3 resource
/// (§13.2). These live in the `0x9F84xx` namespace alongside EN 50221's Host
/// Control, but are distinct tag values.
pub mod tag {
    use crate::tag::ApduTag;
    /// `tune_lcn_req_tag` = `9F 84 07` (Tables 20/99).
    pub const TUNE_LCN_REQ: ApduTag = ApduTag::from_bytes(0x9F, 0x84, 0x07);
    /// `tune_ip_req_tag` = `9F 84 08` (Tables 21/100).
    pub const TUNE_IP_REQ: ApduTag = ApduTag::from_bytes(0x9F, 0x84, 0x08);
    /// `tune_triplet_req_tag` = `9F 84 09` (Tables 19/98).
    pub const TUNE_TRIPLET_REQ: ApduTag = ApduTag::from_bytes(0x9F, 0x84, 0x09);
    /// `tuner_status_req_tag` = `9F 84 0A` (Table 101).
    pub const TUNER_STATUS_REQ: ApduTag = ApduTag::from_bytes(0x9F, 0x84, 0x0A);
    /// `tuner_status_reply_tag` = `9F 84 0B` (Table 102).
    pub const TUNER_STATUS_REPLY: ApduTag = ApduTag::from_bytes(0x9F, 0x84, 0x0B);
}

/// Which resource layout a tune APDU uses — the multi-stream resource
/// (`0x00200081`) or the base DVB Host Control v3 resource (`0x00200043`). Only
/// affects [`TuneIpReq`]'s reserved-bit budget (Table 21 vs Table 100).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub enum HostControlMode {
    /// Multi-stream Host Control (`0x00200081`) — Table 21 layout
    /// (`reserved(1)` + `background_tune_flag`).
    MultiStream,
    /// Base DVB Host Control v3 (`0x00200043`) — Table 100 layout
    /// (`reserved(2)`, no `background_tune_flag`).
    BaseV3,
}

impl HostControlMode {
    /// Diagnostic spec token.
    #[must_use]
    pub fn name(&self) -> &'static str {
        match self {
            Self::MultiStream => "multi_stream",
            Self::BaseV3 => "base_v3",
        }
    }
}
broadcast_common::impl_spec_display!(HostControlMode);

// --- The three tune flag bits, packed into the high bits of the prefix byte. ---
//
// In every tune APDU the three flags occupy the low three bits of a flag/reserved
// byte: background_tune_flag, tune_quietly_flag, keep_app_running_flag — laid out
// from the most-significant flag bit downward. Their exact bit positions depend on
// how much reserved padding precedes them, so each struct defines its own masks.

/// `tune_triplet_req()` (Tables 19/98): CICAM → Host. The §6.4.5.3 multi-stream
/// layout, which adds `background_tune_flag` (the base-v3 Table 98 layout differs
/// only by having `reserved(6)` and no `background_tune_flag` — not modelled here
/// as a separate type; for v3 set `background_tune` = `false`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct TuneTripletReq {
    /// `background_tune_flag` (1) — `true` = background (not presented).
    pub background_tune: bool,
    /// `tune_quietly_flag` (1).
    pub tune_quietly: bool,
    /// `keep_app_running_flag` (1).
    pub keep_app_running: bool,
    /// `original_network_id` (16).
    pub original_network_id: u16,
    /// `transport_stream_id` (16).
    pub transport_stream_id: u16,
    /// `service_id` (16).
    pub service_id: u16,
    /// `delivery_system_descriptor_tag` (8).
    pub delivery_system_descriptor_tag: u8,
    /// `descriptor_tag_extension` (8) when
    /// `delivery_system_descriptor_tag == 0x7F`, else the trailing `reserved(8)`
    /// byte is present but ignored (modeled as `None`).
    pub descriptor_tag_extension: Option<u8>,
}

// flags/reserved(1) + onid(2) + tsid(2) + sid(2) + dsd_tag(1) + ext/reserved(1).
const TRIPLET_BODY: usize = 1 + 2 + 2 + 2 + 1 + 1;
// The descriptor_tag value that signals descriptor_tag_extension follows.
const DSD_TAG_EXTENSION: u8 = 0x7F;

// tune_triplet_req flag byte: reserved(5) + background(1) + quietly(1) + keep(1).
const TRIPLET_BACKGROUND_BIT: u8 = 0x04;
const TRIPLET_QUIETLY_BIT: u8 = 0x02;
const TRIPLET_KEEP_BIT: u8 = 0x01;

impl<'a> Parse<'a> for TuneTripletReq {
    type Error = Error;
    fn parse(bytes: &'a [u8]) -> Result<Self> {
        let body = objects::parse_apdu_header(bytes, tag::TUNE_TRIPLET_REQ, "tune_triplet_req")?;
        if body.len() < TRIPLET_BODY {
            return Err(Error::BufferTooShort {
                need: TRIPLET_BODY,
                have: body.len(),
                what: "tune_triplet_req",
            });
        }
        let flags = body[0];
        let dsd_tag = body[7];
        let descriptor_tag_extension = if dsd_tag == DSD_TAG_EXTENSION {
            Some(body[8])
        } else {
            None
        };
        Ok(Self {
            background_tune: flags & TRIPLET_BACKGROUND_BIT != 0,
            tune_quietly: flags & TRIPLET_QUIETLY_BIT != 0,
            keep_app_running: flags & TRIPLET_KEEP_BIT != 0,
            original_network_id: u16::from_be_bytes([body[1], body[2]]),
            transport_stream_id: u16::from_be_bytes([body[3], body[4]]),
            service_id: u16::from_be_bytes([body[5], body[6]]),
            delivery_system_descriptor_tag: dsd_tag,
            descriptor_tag_extension,
        })
    }
}

impl Serialize for TuneTripletReq {
    type Error = Error;
    fn serialized_len(&self) -> usize {
        objects::apdu_len(TRIPLET_BODY)
    }
    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        let pos = objects::write_apdu_header(tag::TUNE_TRIPLET_REQ, TRIPLET_BODY, buf)?;
        let mut flags = 0u8;
        if self.background_tune {
            flags |= TRIPLET_BACKGROUND_BIT;
        }
        if self.tune_quietly {
            flags |= TRIPLET_QUIETLY_BIT;
        }
        if self.keep_app_running {
            flags |= TRIPLET_KEEP_BIT;
        }
        buf[pos] = flags;
        buf[pos + 1..pos + 3].copy_from_slice(&self.original_network_id.to_be_bytes());
        buf[pos + 3..pos + 5].copy_from_slice(&self.transport_stream_id.to_be_bytes());
        buf[pos + 5..pos + 7].copy_from_slice(&self.service_id.to_be_bytes());
        buf[pos + 7] = self.delivery_system_descriptor_tag;
        // descriptor_tag_extension when tag==0x7F, else reserved(8) = 0.
        buf[pos + 8] = self.descriptor_tag_extension.unwrap_or(0);
        Ok(pos + TRIPLET_BODY)
    }
}

/// `tune_lcn_req()` (Tables 20/99): CICAM → Host. The §6.4.5.4 multi-stream
/// layout, which adds `background_tune_flag`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct TuneLcnReq {
    /// `background_tune_flag` (1).
    pub background_tune: bool,
    /// `tune_quietly_flag` (1).
    pub tune_quietly: bool,
    /// `keep_app_running_flag` (1).
    pub keep_app_running: bool,
    /// `logical_channel_number` (14) — range 0..9999.
    pub logical_channel_number: u16,
}

// flags/reserved(1) + flags/lcn-hi(1) + lcn-lo(1).
const LCN_BODY: usize = 3;

// tune_lcn_req: reserved(7) + background(1) | quietly(1)+keep(1)+lcn(14).
// First byte: reserved(7) + background_tune_flag(1 = LSB).
const LCN_BACKGROUND_BIT: u8 = 0x01;
// Second byte high bits: quietly(1)=bit7, keep(1)=bit6, then lcn[13:8].
const LCN_QUIETLY_BIT: u8 = 0x80;
const LCN_KEEP_BIT: u8 = 0x40;
// logical_channel_number occupies the low 14 bits of the last two bytes.
const LCN_MASK: u16 = 0x3FFF;

impl<'a> Parse<'a> for TuneLcnReq {
    type Error = Error;
    fn parse(bytes: &'a [u8]) -> Result<Self> {
        let body = objects::parse_apdu_header(bytes, tag::TUNE_LCN_REQ, "tune_lcn_req")?;
        if body.len() < LCN_BODY {
            return Err(Error::BufferTooShort {
                need: LCN_BODY,
                have: body.len(),
                what: "tune_lcn_req",
            });
        }
        let background_tune = body[0] & LCN_BACKGROUND_BIT != 0;
        let tune_quietly = body[1] & LCN_QUIETLY_BIT != 0;
        let keep_app_running = body[1] & LCN_KEEP_BIT != 0;
        let logical_channel_number = u16::from_be_bytes([body[1], body[2]]) & LCN_MASK;
        Ok(Self {
            background_tune,
            tune_quietly,
            keep_app_running,
            logical_channel_number,
        })
    }
}

impl Serialize for TuneLcnReq {
    type Error = Error;
    fn serialized_len(&self) -> usize {
        objects::apdu_len(LCN_BODY)
    }
    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        let pos = objects::write_apdu_header(tag::TUNE_LCN_REQ, LCN_BODY, buf)?;
        buf[pos] = if self.background_tune {
            LCN_BACKGROUND_BIT
        } else {
            0
        };
        let mut hi = (self.logical_channel_number >> 8) as u8 & (LCN_MASK >> 8) as u8;
        if self.tune_quietly {
            hi |= LCN_QUIETLY_BIT;
        }
        if self.keep_app_running {
            hi |= LCN_KEEP_BIT;
        }
        buf[pos + 1] = hi;
        buf[pos + 2] = self.logical_channel_number as u8;
        Ok(pos + LCN_BODY)
    }
}

/// `tune_ip_req()` (Tables 21/100): CICAM → Host. The reserved-bit budget before
/// the flags differs by [`HostControlMode`] (Table 21 = 1 reserved bit +
/// `background_tune_flag`; Table 100 = 2 reserved bits, no `background_tune_flag`).
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct TuneIpReq<'a> {
    /// Which resource layout this APDU uses (selects the reserved-bit budget).
    pub mode: HostControlMode,
    /// `background_tune_flag` (1) — only present in [`HostControlMode::MultiStream`];
    /// for [`HostControlMode::BaseV3`] this field is not on the wire and must be
    /// `false`.
    pub background_tune: bool,
    /// `tune_quietly_flag` (1).
    pub tune_quietly: bool,
    /// `keep_app_running_flag` (1).
    pub keep_app_running: bool,
    /// `service_location_data` (the `service_location_length`-byte body) — opaque.
    #[cfg_attr(feature = "serde", serde(borrow, with = "crate::objects::bytes_serde"))]
    pub service_location_data: &'a [u8],
}

// flags/reserved+length(2) prefix, then service_location_data.
const TUNE_IP_PREFIX: usize = 2;
// Multi-stream (Table 21): byte0 = reserved(1) | background(bit6) | quietly(bit5)
// | keep(bit4) | service_location_length[11:8].
const IP_MS_BACKGROUND_BIT: u8 = 0x40;
const IP_MS_QUIETLY_BIT: u8 = 0x20;
const IP_MS_KEEP_BIT: u8 = 0x10;
// Base v3 (Table 100): byte0 = reserved(2) | quietly(bit5) | keep(bit4) |
// service_location_length[11:8]. (No background_tune_flag.)
const IP_V3_QUIETLY_BIT: u8 = 0x20;
const IP_V3_KEEP_BIT: u8 = 0x10;
// service_location_length is 12 bits: high nibble in byte0, low 8 in byte1.
const SLL_HI_MASK: u8 = 0x0F;

impl<'a> TuneIpReq<'a> {
    /// Parse a `tune_ip_req` body under the given [`HostControlMode`].
    pub fn parse_mode(bytes: &'a [u8], mode: HostControlMode) -> Result<Self> {
        let body = objects::parse_apdu_header(bytes, tag::TUNE_IP_REQ, "tune_ip_req")?;
        if body.len() < TUNE_IP_PREFIX {
            return Err(Error::BufferTooShort {
                need: TUNE_IP_PREFIX,
                have: body.len(),
                what: "tune_ip_req",
            });
        }
        let (background_tune, tune_quietly, keep_app_running) = match mode {
            HostControlMode::MultiStream => (
                body[0] & IP_MS_BACKGROUND_BIT != 0,
                body[0] & IP_MS_QUIETLY_BIT != 0,
                body[0] & IP_MS_KEEP_BIT != 0,
            ),
            HostControlMode::BaseV3 => (
                false,
                body[0] & IP_V3_QUIETLY_BIT != 0,
                body[0] & IP_V3_KEEP_BIT != 0,
            ),
        };
        let sll = (((body[0] & SLL_HI_MASK) as usize) << 8) | body[1] as usize;
        let data = &body[TUNE_IP_PREFIX..];
        if data.len() < sll {
            return Err(Error::BufferTooShort {
                need: sll,
                have: data.len(),
                what: "tune_ip_req service_location_data",
            });
        }
        Ok(Self {
            mode,
            background_tune,
            tune_quietly,
            keep_app_running,
            service_location_data: &data[..sll],
        })
    }
}

impl Serialize for TuneIpReq<'_> {
    type Error = Error;
    fn serialized_len(&self) -> usize {
        objects::apdu_len(TUNE_IP_PREFIX + self.service_location_data.len())
    }
    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        let sll = self.service_location_data.len();
        let body_len = TUNE_IP_PREFIX + sll;
        let pos = objects::write_apdu_header(tag::TUNE_IP_REQ, body_len, buf)?;
        let mut byte0 = (sll >> 8) as u8 & SLL_HI_MASK;
        match self.mode {
            HostControlMode::MultiStream => {
                if self.background_tune {
                    byte0 |= IP_MS_BACKGROUND_BIT;
                }
                if self.tune_quietly {
                    byte0 |= IP_MS_QUIETLY_BIT;
                }
                if self.keep_app_running {
                    byte0 |= IP_MS_KEEP_BIT;
                }
            }
            HostControlMode::BaseV3 => {
                if self.tune_quietly {
                    byte0 |= IP_V3_QUIETLY_BIT;
                }
                if self.keep_app_running {
                    byte0 |= IP_V3_KEEP_BIT;
                }
            }
        }
        buf[pos] = byte0;
        buf[pos + 1] = sll as u8;
        buf[pos + 2..pos + 2 + sll].copy_from_slice(self.service_location_data);
        Ok(pos + body_len)
    }
}

/// `tuner_status_req()` (Table 101): CICAM → Host, header-only (`length_field = 0`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct TunerStatusReq;

impl<'a> Parse<'a> for TunerStatusReq {
    type Error = Error;
    fn parse(bytes: &'a [u8]) -> Result<Self> {
        objects::parse_empty_apdu(bytes, tag::TUNER_STATUS_REQ, "tuner_status_req")?;
        Ok(Self)
    }
}

impl Serialize for TunerStatusReq {
    type Error = Error;
    fn serialized_len(&self) -> usize {
        objects::empty_apdu_len()
    }
    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        objects::serialize_empty_apdu(tag::TUNER_STATUS_REQ, buf)
    }
}

/// One DSD entry of [`TunerStatusReply`] (Table 102): `reserved(7)` +
/// `connected_flag(1)` + `delivery_system_descriptor_tag(8)` + (extension or
/// reserved)(8).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct TunerStatusDsd {
    /// `connected_flag` (1) — Host believes at least one tuner of this DSD type is
    /// connected.
    pub connected: bool,
    /// `delivery_system_descriptor_tag` (8).
    pub delivery_system_descriptor_tag: u8,
    /// `descriptor_tag_extension` (8) when
    /// `delivery_system_descriptor_tag == 0x7F`, else the trailing `reserved(8)`
    /// byte is ignored (`None`).
    pub descriptor_tag_extension: Option<u8>,
}

/// `tuner_status_reply()` (Table 102): Host → CICAM.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct TunerStatusReply {
    /// `IP_tune_capable_flag` (1) — Host can accept `tune_ip_req()` APDUs.
    pub ip_tune_capable: bool,
    /// The DSD entries (loop count = `num_dsd`).
    pub dsds: Vec<TunerStatusDsd>,
}

// Each DSD entry is 3 bytes on the wire (flags+tag+ext/reserved).
const DSD_ENTRY_LEN: usize = 3;
// First byte: IP_tune_capable_flag(1 = bit7) + num_dsd(7).
const IP_TUNE_CAPABLE_BIT: u8 = 0x80;
const NUM_DSD_MASK: u8 = 0x7F;
// Within a DSD entry, connected_flag is the LSB of the reserved(7)+connected(1) byte.
const DSD_CONNECTED_BIT: u8 = 0x01;

impl<'a> Parse<'a> for TunerStatusReply {
    type Error = Error;
    fn parse(bytes: &'a [u8]) -> Result<Self> {
        let body =
            objects::parse_apdu_header(bytes, tag::TUNER_STATUS_REPLY, "tuner_status_reply")?;
        if body.is_empty() {
            return Err(Error::BufferTooShort {
                need: 1,
                have: 0,
                what: "tuner_status_reply",
            });
        }
        let ip_tune_capable = body[0] & IP_TUNE_CAPABLE_BIT != 0;
        let num_dsd = (body[0] & NUM_DSD_MASK) as usize;
        let mut rest = &body[1..];
        let mut dsds = Vec::with_capacity(num_dsd);
        for _ in 0..num_dsd {
            if rest.len() < DSD_ENTRY_LEN {
                return Err(Error::BufferTooShort {
                    need: DSD_ENTRY_LEN,
                    have: rest.len(),
                    what: "tuner_status_reply dsd",
                });
            }
            let connected = rest[0] & DSD_CONNECTED_BIT != 0;
            let dsd_tag = rest[1];
            let descriptor_tag_extension = if dsd_tag == DSD_TAG_EXTENSION {
                Some(rest[2])
            } else {
                None
            };
            dsds.push(TunerStatusDsd {
                connected,
                delivery_system_descriptor_tag: dsd_tag,
                descriptor_tag_extension,
            });
            rest = &rest[DSD_ENTRY_LEN..];
        }
        Ok(Self {
            ip_tune_capable,
            dsds,
        })
    }
}

impl Serialize for TunerStatusReply {
    type Error = Error;
    fn serialized_len(&self) -> usize {
        objects::apdu_len(1 + self.dsds.len() * DSD_ENTRY_LEN)
    }
    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        let body_len = 1 + self.dsds.len() * DSD_ENTRY_LEN;
        let mut pos = objects::write_apdu_header(tag::TUNER_STATUS_REPLY, body_len, buf)?;
        let mut byte0 = self.dsds.len() as u8 & NUM_DSD_MASK;
        if self.ip_tune_capable {
            byte0 |= IP_TUNE_CAPABLE_BIT;
        }
        buf[pos] = byte0;
        pos += 1;
        for dsd in &self.dsds {
            buf[pos] = if dsd.connected { DSD_CONNECTED_BIT } else { 0 };
            buf[pos + 1] = dsd.delivery_system_descriptor_tag;
            buf[pos + 2] = dsd.descriptor_tag_extension.unwrap_or(0);
            pos += DSD_ENTRY_LEN;
        }
        Ok(pos)
    }
}

/// Resource-scoped dispatch over the Multi-stream Host Control resource objects.
///
/// `tune_broadcast_req` / `tune_reply` / `ask_release` / `ask_release_reply` are
/// deferred to CI Plus V1.3 and are not members of this enum.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub enum MultistreamHostControlApdu<'a> {
    /// `tune_triplet_req` (`9F 84 09`).
    TuneTripletReq(TuneTripletReq),
    /// `tune_lcn_req` (`9F 84 07`).
    TuneLcnReq(TuneLcnReq),
    /// `tune_ip_req` (`9F 84 08`).
    TuneIpReq(TuneIpReq<'a>),
    /// `tuner_status_req` (`9F 84 0A`).
    TunerStatusReq(TunerStatusReq),
    /// `tuner_status_reply` (`9F 84 0B`).
    TunerStatusReply(TunerStatusReply),
}

impl<'a> MultistreamHostControlApdu<'a> {
    /// Parse a Multi-stream Host Control APDU, dispatching on the leading
    /// `apdu_tag`. `mode` selects the [`TuneIpReq`] reserved-bit budget.
    pub fn parse_mode(body: &'a [u8], mode: HostControlMode) -> Result<Self> {
        if body.len() < 3 {
            return Err(Error::BufferTooShort {
                need: 3,
                have: body.len(),
                what: "multistream_host_control apdu_tag",
            });
        }
        let t = ApduTag::from_bytes(body[0], body[1], body[2]);
        match t {
            tag::TUNE_TRIPLET_REQ => Ok(Self::TuneTripletReq(TuneTripletReq::parse(body)?)),
            tag::TUNE_LCN_REQ => Ok(Self::TuneLcnReq(TuneLcnReq::parse(body)?)),
            tag::TUNE_IP_REQ => Ok(Self::TuneIpReq(TuneIpReq::parse_mode(body, mode)?)),
            tag::TUNER_STATUS_REQ => Ok(Self::TunerStatusReq(TunerStatusReq::parse(body)?)),
            tag::TUNER_STATUS_REPLY => Ok(Self::TunerStatusReply(TunerStatusReply::parse(body)?)),
            _ => Err(Error::UnexpectedApduTag {
                got: t.as_u24(),
                expected: tag::TUNE_TRIPLET_REQ.as_u24(),
                what: "multistream_host_control",
            }),
        }
    }
}

impl Serialize for MultistreamHostControlApdu<'_> {
    type Error = Error;
    fn serialized_len(&self) -> usize {
        match self {
            Self::TuneTripletReq(o) => o.serialized_len(),
            Self::TuneLcnReq(o) => o.serialized_len(),
            Self::TuneIpReq(o) => o.serialized_len(),
            Self::TunerStatusReq(o) => o.serialized_len(),
            Self::TunerStatusReply(o) => o.serialized_len(),
        }
    }
    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        match self {
            Self::TuneTripletReq(o) => o.serialize_into(buf),
            Self::TuneLcnReq(o) => o.serialize_into(buf),
            Self::TuneIpReq(o) => o.serialize_into(buf),
            Self::TunerStatusReq(o) => o.serialize_into(buf),
            Self::TunerStatusReply(o) => o.serialize_into(buf),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tune_triplet_round_trips_and_bites() {
        let t = TuneTripletReq {
            background_tune: true,
            tune_quietly: false,
            keep_app_running: true,
            original_network_id: 0x1122,
            transport_stream_id: 0x3344,
            service_id: 0x5566,
            delivery_system_descriptor_tag: 0x44,
            descriptor_tag_extension: None,
        };
        let bytes = t.to_bytes();
        // tag(3) + len(0x09) + flags(background|keep = 0x04|0x01 = 0x05)
        //   + onid(11 22) + tsid(33 44) + sid(55 66) + dsd_tag(44) + reserved(00).
        assert_eq!(
            bytes,
            [0x9F, 0x84, 0x09, 0x09, 0x05, 0x11, 0x22, 0x33, 0x44, 0x55, 0x66, 0x44, 0x00]
        );
        assert_eq!(TuneTripletReq::parse(&bytes).unwrap(), t);
        let mut other = t;
        other.background_tune = false;
        assert_eq!(other.to_bytes()[4], 0x01);
        assert_ne!(bytes, other.to_bytes());
    }

    #[test]
    fn tune_triplet_with_descriptor_extension() {
        let t = TuneTripletReq {
            background_tune: false,
            tune_quietly: true,
            keep_app_running: false,
            original_network_id: 0x0001,
            transport_stream_id: 0x0002,
            service_id: 0x0003,
            delivery_system_descriptor_tag: 0x7F,
            descriptor_tag_extension: Some(0x79),
        };
        let bytes = t.to_bytes();
        // flags = quietly = 0x02; dsd_tag=0x7F; ext=0x79.
        assert_eq!(
            bytes,
            [0x9F, 0x84, 0x09, 0x09, 0x02, 0x00, 0x01, 0x00, 0x02, 0x00, 0x03, 0x7F, 0x79]
        );
        assert_eq!(TuneTripletReq::parse(&bytes).unwrap(), t);
    }

    #[test]
    fn tune_lcn_round_trips_and_bites() {
        let t = TuneLcnReq {
            background_tune: true,
            tune_quietly: true,
            keep_app_running: false,
            logical_channel_number: 0x0123,
        };
        let bytes = t.to_bytes();
        // tag(3) + len(0x03) + byte0(background = 0x01)
        //   + byte1(quietly=0x80 | lcn[13:8]=0x01 = 0x81) + byte2(lcn lo = 0x23).
        assert_eq!(bytes, [0x9F, 0x84, 0x07, 0x03, 0x01, 0x81, 0x23]);
        assert_eq!(TuneLcnReq::parse(&bytes).unwrap(), t);
        let mut other = t;
        other.logical_channel_number = 0x3FFE;
        assert_ne!(bytes, other.to_bytes());
        // 14-bit value preserved.
        assert_eq!(
            TuneLcnReq::parse(&other.to_bytes())
                .unwrap()
                .logical_channel_number,
            0x3FFE
        );
    }

    #[test]
    fn tune_ip_multistream_vs_basev3_distinct_bytes() {
        // Same logical request, both with background_tune set where applicable.
        let data: &[u8] = &[0xAA, 0xBB];
        let ms = TuneIpReq {
            mode: HostControlMode::MultiStream,
            background_tune: true,
            tune_quietly: true,
            keep_app_running: false,
            service_location_data: data,
        };
        let v3 = TuneIpReq {
            mode: HostControlMode::BaseV3,
            background_tune: false, // not on the wire in v3
            tune_quietly: true,
            keep_app_running: false,
            service_location_data: data,
        };
        let ms_bytes = ms.to_bytes();
        let v3_bytes = v3.to_bytes();
        // Multi-stream byte0: background(0x40) | quietly(0x20) | sll_hi(0) = 0x60.
        assert_eq!(ms_bytes, [0x9F, 0x84, 0x08, 0x04, 0x60, 0x02, 0xAA, 0xBB]);
        // Base v3 byte0: quietly(0x20) | sll_hi(0) = 0x20 (no background bit).
        assert_eq!(v3_bytes, [0x9F, 0x84, 0x08, 0x04, 0x20, 0x02, 0xAA, 0xBB]);
        // Distinct byte patterns even though quietly is set in both.
        assert_ne!(ms_bytes, v3_bytes);
        assert_eq!(
            TuneIpReq::parse_mode(&ms_bytes, HostControlMode::MultiStream).unwrap(),
            ms
        );
        assert_eq!(
            TuneIpReq::parse_mode(&v3_bytes, HostControlMode::BaseV3).unwrap(),
            v3
        );
    }

    #[test]
    fn tune_ip_multistream_background_bit_is_v3_reserved() {
        // The multi-stream background bit (0x40) sits in a v3 reserved position:
        // parsing the multi-stream bytes as v3 must NOT see a flag there, and the
        // background flag is dropped (v3 has no background_tune_flag).
        let ms = TuneIpReq {
            mode: HostControlMode::MultiStream,
            background_tune: true,
            tune_quietly: false,
            keep_app_running: false,
            service_location_data: &[],
        };
        let bytes = ms.to_bytes();
        // byte0 = 0x40 (background only).
        assert_eq!(bytes[4], 0x40);
        let as_v3 = TuneIpReq::parse_mode(&bytes, HostControlMode::BaseV3).unwrap();
        assert!(!as_v3.background_tune);
        assert!(!as_v3.tune_quietly);
        assert!(!as_v3.keep_app_running);
    }

    #[test]
    fn tune_ip_empty_location() {
        let t = TuneIpReq {
            mode: HostControlMode::MultiStream,
            background_tune: false,
            tune_quietly: false,
            keep_app_running: false,
            service_location_data: &[],
        };
        let bytes = t.to_bytes();
        assert_eq!(bytes, [0x9F, 0x84, 0x08, 0x02, 0x00, 0x00]);
        assert_eq!(
            TuneIpReq::parse_mode(&bytes, HostControlMode::MultiStream).unwrap(),
            t
        );
    }

    #[test]
    fn tuner_status_req_round_trips() {
        let bytes = TunerStatusReq.to_bytes();
        assert_eq!(bytes, [0x9F, 0x84, 0x0A, 0x00]);
        assert_eq!(TunerStatusReq::parse(&bytes).unwrap(), TunerStatusReq);
    }

    #[test]
    fn tuner_status_reply_round_trips_with_two_dsds() {
        let r = TunerStatusReply {
            ip_tune_capable: true,
            dsds: alloc::vec![
                TunerStatusDsd {
                    connected: true,
                    delivery_system_descriptor_tag: 0x44, // DVB-S2 dsd, say
                    descriptor_tag_extension: None,
                },
                TunerStatusDsd {
                    connected: false,
                    delivery_system_descriptor_tag: 0x7F,
                    descriptor_tag_extension: Some(0x79),
                },
            ],
        };
        let bytes = r.to_bytes();
        // tag(3) + len(0x07) + byte0(ip_capable=0x80 | num_dsd=2 = 0x82)
        //   dsd0: connected=1 -> 0x01, tag=0x44, reserved=0x00
        //   dsd1: connected=0 -> 0x00, tag=0x7F, ext=0x79.
        assert_eq!(
            bytes,
            [0x9F, 0x84, 0x0B, 0x07, 0x82, 0x01, 0x44, 0x00, 0x00, 0x7F, 0x79]
        );
        assert_eq!(TunerStatusReply::parse(&bytes).unwrap(), r);
        let mut other = r.clone();
        other.ip_tune_capable = false;
        assert_eq!(other.to_bytes()[4], 0x02);
        assert_ne!(bytes, other.to_bytes());
    }

    #[test]
    fn tuner_status_reply_empty() {
        let r = TunerStatusReply {
            ip_tune_capable: false,
            dsds: Vec::new(),
        };
        let bytes = r.to_bytes();
        assert_eq!(bytes, [0x9F, 0x84, 0x0B, 0x01, 0x00]);
        assert_eq!(TunerStatusReply::parse(&bytes).unwrap(), r);
    }

    #[test]
    fn dispatch_routes_each_tag() {
        let triplet = TuneTripletReq {
            background_tune: false,
            tune_quietly: false,
            keep_app_running: false,
            original_network_id: 1,
            transport_stream_id: 2,
            service_id: 3,
            delivery_system_descriptor_tag: 0,
            descriptor_tag_extension: None,
        }
        .to_bytes();
        assert!(matches!(
            MultistreamHostControlApdu::parse_mode(&triplet, HostControlMode::MultiStream).unwrap(),
            MultistreamHostControlApdu::TuneTripletReq(_)
        ));
        let ip = TuneIpReq {
            mode: HostControlMode::MultiStream,
            background_tune: true,
            tune_quietly: false,
            keep_app_running: false,
            service_location_data: &[0x01],
        }
        .to_bytes();
        let parsed =
            MultistreamHostControlApdu::parse_mode(&ip, HostControlMode::MultiStream).unwrap();
        assert!(matches!(parsed, MultistreamHostControlApdu::TuneIpReq(_)));
        assert_eq!(parsed.to_bytes(), ip);
        // Unknown tag rejected.
        assert!(matches!(
            MultistreamHostControlApdu::parse_mode(
                &[0x9F, 0x84, 0x00, 0x00],
                HostControlMode::MultiStream
            ),
            Err(Error::UnexpectedApduTag { .. })
        ));
    }
}
