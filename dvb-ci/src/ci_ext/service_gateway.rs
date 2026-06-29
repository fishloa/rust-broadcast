//! Generic Service Gateway objects — ETSI TS 101 699 V1.1.1 §6.1.3, Tables 21-31
//! (PDF pp. 32-37). See `docs/ci_plus/input-modules.md`.
//!
//! A Type 'B' input module presents a **ServiceGateway** (Generic Service
//! Gateway) resource for **service-level** access. These calls are inherited by
//! all network-specific gateway resources (e.g. [`super::broadcast_service_gateway`]).
//! The Generic Service Gateway is never instantiated on its own (Table 87 NOTE);
//! its objects are modelled here so the network-specific gateways can route them.
//!
//! A service reference is the `{OriginalNetworkID, ServiceID}` pair (Figure 12).
//!
//! - `ServiceListReq` (`9F 80 00`, Table 22) — A → R: header-only.
//! - `ServiceListAck` (`9F 80 01`, Table 23) — R → A: version + service list.
//! - `ServiceListVersionReq` (`9F 80 02`, Table 24) — A → R: header-only.
//! - `ServiceListVersionAck` (`9F 80 03`, Table 25) — R → A: version number.
//! - `ServiceListChanged` (`9F 80 04`, Table 26) — R → A: new version number.
//! - `ServiceDescReq` (`9F 80 05`, Table 27) — A → R: a service reference.
//! - `ServiceDescAck` (`9F 80 06`, Table 28) — R → A: SDT-modelled service params.
//! - `GetServiceReq` (`9F 80 07`, Table 29) — A → R: a service reference.
//! - `GetServiceAck` (`9F 80 08`, Table 30) — R → A: service availability.

use crate::error::{Error, Result};
use crate::objects;
use crate::tag::ApduTag;
use alloc::vec::Vec;
use broadcast_common::{Parse, Serialize};

/// Resource-scoped `apdu_tag`s for the Generic Service Gateway (Tables 22-30).
pub mod tag {
    use crate::tag::ApduTag;
    /// `ServiceListReqTag` = `9F 80 00`.
    pub const SERVICE_LIST_REQ: ApduTag = ApduTag::from_bytes(0x9F, 0x80, 0x00);
    /// `ServiceListAckTag` = `9F 80 01`.
    pub const SERVICE_LIST_ACK: ApduTag = ApduTag::from_bytes(0x9F, 0x80, 0x01);
    /// `ServiceListVersionReqTag` = `9F 80 02`.
    pub const SERVICE_LIST_VERSION_REQ: ApduTag = ApduTag::from_bytes(0x9F, 0x80, 0x02);
    /// `ServiceListVersionAckTag` = `9F 80 03`.
    pub const SERVICE_LIST_VERSION_ACK: ApduTag = ApduTag::from_bytes(0x9F, 0x80, 0x03);
    /// `ServiceListChangedTag` = `9F 80 04`.
    pub const SERVICE_LIST_CHANGED: ApduTag = ApduTag::from_bytes(0x9F, 0x80, 0x04);
    /// `ServiceDescReqTag` = `9F 80 05`.
    pub const SERVICE_DESC_REQ: ApduTag = ApduTag::from_bytes(0x9F, 0x80, 0x05);
    /// `ServiceDescAckTag` = `9F 80 06`.
    pub const SERVICE_DESC_ACK: ApduTag = ApduTag::from_bytes(0x9F, 0x80, 0x06);
    /// `GetServiceReqTag` = `9F 80 07`.
    pub const GET_SERVICE_REQ: ApduTag = ApduTag::from_bytes(0x9F, 0x80, 0x07);
    /// `GetServiceAckTag` = `9F 80 08`.
    pub const GET_SERVICE_ACK: ApduTag = ApduTag::from_bytes(0x9F, 0x80, 0x08);
}

/// A service reference — the `{OriginalNetworkID, ServiceID}` pair (Figure 12).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct ServiceReference {
    /// `OriginalNetworkID` (16-bit, allocated within ETR 162).
    pub original_network_id: u16,
    /// `ServiceID` (16-bit, unique within the original network).
    pub service_id: u16,
}

/// `ServiceListReq()` (Table 22) — A → R: header-only.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct ServiceListReq;

/// `ServiceListVersionReq()` (Table 24) — A → R: header-only.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct ServiceListVersionReq;

/// `ServiceListAck()` (Table 23) — R → A: the services the resource can supply.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct ServiceListAck {
    /// `VersionNumber` — increments each time the service list is updated.
    pub version_number: u8,
    /// The service references (`NumberOfServices` of them, may be 0).
    pub services: Vec<ServiceReference>,
}

/// `ServiceListVersionAck()` (Table 25) — R → A: the service-list version.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct ServiceListVersionAck {
    /// `VersionNumber`.
    pub version_number: u8,
}

/// `ServiceListChanged()` (Table 26) — R → A: the service list changed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct ServiceListChanged {
    /// `VersionNumber` — the new version.
    pub version_number: u8,
}

/// `ServiceDescReq()` (Table 27) — A → R: request info on a particular service.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct ServiceDescReq {
    /// The service reference being queried.
    pub service: ServiceReference,
}

/// `ServiceDescAck()` (Table 28) — R → A: SDT-modelled service parameters and a
/// descriptor loop. The parameters mirror the SDT in ETS 300 468.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct ServiceDescAck<'a> {
    /// The service reference being described.
    pub service: ServiceReference,
    /// `EIT_schedule_flag` — SDT meaning (ETS 300 468).
    pub eit_schedule_flag: bool,
    /// `EIT_present_following_flag` — SDT meaning.
    pub eit_present_following_flag: bool,
    /// `running_status` — 3-bit SDT running status (the spec prose "6 bit" is a
    /// typo; 3 bits is authoritative — see `docs/ci_plus/input-modules.md`).
    pub running_status: u8,
    /// `free_CA_mode` — SDT meaning.
    pub free_ca_mode: bool,
    /// The SDT descriptor loop (`descriptors_loop_length` bytes), carried verbatim;
    /// walk it with the dvb-si descriptor parsers.
    #[cfg_attr(feature = "serde", serde(borrow, with = "crate::objects::bytes_serde"))]
    pub descriptors: &'a [u8],
}

/// `GetServiceReq()` (Table 29) — A → R: request the resource to provide a
/// service. An absent service reference (zero following bytes) requests a
/// network disconnect.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct GetServiceReq {
    /// The requested service reference, or `None` for a network-disconnect request.
    pub service: Option<ServiceReference>,
}

/// `GetServiceAck()` (Table 30) — R → A: availability of a requested service.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct GetServiceAck {
    /// The service reference replied about.
    pub service: ServiceReference,
    /// `ServiceTerminated` — `1`: the service has finished (or a disconnect was
    /// requested); navigation reverts to the host.
    pub service_terminated: bool,
    /// `ServiceNotAvailable` — `1`: the requested service is not available.
    pub service_not_available: bool,
    /// `CAServiceFlag` — `1`: conditional-access restrictions apply.
    pub ca_service_flag: bool,
    /// `ActualService` — the actual service id (MPEG program number) delivered;
    /// `0` = no valid TS (the host should not attempt to decode).
    pub actual_service: u16,
}

// --- header-only objects ---

macro_rules! empty_object {
    ($ty:ty, $tag:expr, $what:literal) => {
        impl<'a> Parse<'a> for $ty {
            type Error = Error;
            fn parse(bytes: &'a [u8]) -> Result<Self> {
                objects::parse_empty_apdu(bytes, $tag, $what)?;
                Ok(Self)
            }
        }
        impl Serialize for $ty {
            type Error = Error;
            fn serialized_len(&self) -> usize {
                objects::empty_apdu_len()
            }
            fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
                objects::serialize_empty_apdu($tag, buf)
            }
        }
    };
}

empty_object!(ServiceListReq, tag::SERVICE_LIST_REQ, "ServiceListReq");
empty_object!(
    ServiceListVersionReq,
    tag::SERVICE_LIST_VERSION_REQ,
    "ServiceListVersionReq"
);

// --- single version-byte objects ---

macro_rules! version_byte_object {
    ($ty:ty, $tag:expr, $what:literal) => {
        impl<'a> Parse<'a> for $ty {
            type Error = Error;
            fn parse(bytes: &'a [u8]) -> Result<Self> {
                let body = objects::parse_apdu_header(bytes, $tag, $what)?;
                if body.is_empty() {
                    return Err(Error::BufferTooShort {
                        need: 1,
                        have: 0,
                        what: $what,
                    });
                }
                Ok(Self {
                    version_number: body[0],
                })
            }
        }
        impl Serialize for $ty {
            type Error = Error;
            fn serialized_len(&self) -> usize {
                objects::apdu_len(1)
            }
            fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
                let pos = objects::write_apdu_header($tag, 1, buf)?;
                buf[pos] = self.version_number;
                Ok(pos + 1)
            }
        }
    };
}

version_byte_object!(
    ServiceListVersionAck,
    tag::SERVICE_LIST_VERSION_ACK,
    "ServiceListVersionAck"
);
version_byte_object!(
    ServiceListChanged,
    tag::SERVICE_LIST_CHANGED,
    "ServiceListChanged"
);

// --- ServiceListAck ---

/// Width of one service reference on the wire (`OriginalNetworkID` + `ServiceID`).
const SERVICE_REF_LEN: usize = 4;
// VersionNumber(1) + NumberOfServices(2).
const SERVICE_LIST_ACK_PREFIX: usize = 3;

impl<'a> Parse<'a> for ServiceListAck {
    type Error = Error;
    fn parse(bytes: &'a [u8]) -> Result<Self> {
        let body = objects::parse_apdu_header(bytes, tag::SERVICE_LIST_ACK, "ServiceListAck")?;
        if body.len() < SERVICE_LIST_ACK_PREFIX {
            return Err(Error::BufferTooShort {
                need: SERVICE_LIST_ACK_PREFIX,
                have: body.len(),
                what: "ServiceListAck",
            });
        }
        let version_number = body[0];
        let count = u16::from_be_bytes([body[1], body[2]]) as usize;
        let list = &body[SERVICE_LIST_ACK_PREFIX..];
        if list.len() < count * SERVICE_REF_LEN {
            return Err(Error::LengthMismatch {
                what: "ServiceListAck services",
                declared: count * SERVICE_REF_LEN,
                actual: list.len(),
            });
        }
        let mut services = Vec::with_capacity(count);
        for chunk in list[..count * SERVICE_REF_LEN].chunks_exact(SERVICE_REF_LEN) {
            services.push(ServiceReference {
                original_network_id: u16::from_be_bytes([chunk[0], chunk[1]]),
                service_id: u16::from_be_bytes([chunk[2], chunk[3]]),
            });
        }
        Ok(Self {
            version_number,
            services,
        })
    }
}
impl Serialize for ServiceListAck {
    type Error = Error;
    fn serialized_len(&self) -> usize {
        objects::apdu_len(SERVICE_LIST_ACK_PREFIX + self.services.len() * SERVICE_REF_LEN)
    }
    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        if self.services.len() > u16::MAX as usize {
            return Err(Error::InvalidObject {
                what: "ServiceListAck",
                reason: "more than 65535 services",
            });
        }
        let body_len = SERVICE_LIST_ACK_PREFIX + self.services.len() * SERVICE_REF_LEN;
        let mut pos = objects::write_apdu_header(tag::SERVICE_LIST_ACK, body_len, buf)?;
        buf[pos] = self.version_number;
        buf[pos + 1..pos + 3].copy_from_slice(&(self.services.len() as u16).to_be_bytes());
        pos += SERVICE_LIST_ACK_PREFIX;
        for s in &self.services {
            buf[pos..pos + 2].copy_from_slice(&s.original_network_id.to_be_bytes());
            buf[pos + 2..pos + 4].copy_from_slice(&s.service_id.to_be_bytes());
            pos += SERVICE_REF_LEN;
        }
        Ok(pos)
    }
}

// --- ServiceDescReq (service reference) ---

impl<'a> Parse<'a> for ServiceDescReq {
    type Error = Error;
    fn parse(bytes: &'a [u8]) -> Result<Self> {
        let body = objects::parse_apdu_header(bytes, tag::SERVICE_DESC_REQ, "ServiceDescReq")?;
        if body.len() < SERVICE_REF_LEN {
            return Err(Error::BufferTooShort {
                need: SERVICE_REF_LEN,
                have: body.len(),
                what: "ServiceDescReq",
            });
        }
        Ok(Self {
            service: ServiceReference {
                original_network_id: u16::from_be_bytes([body[0], body[1]]),
                service_id: u16::from_be_bytes([body[2], body[3]]),
            },
        })
    }
}
impl Serialize for ServiceDescReq {
    type Error = Error;
    fn serialized_len(&self) -> usize {
        objects::apdu_len(SERVICE_REF_LEN)
    }
    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        let pos = objects::write_apdu_header(tag::SERVICE_DESC_REQ, SERVICE_REF_LEN, buf)?;
        buf[pos..pos + 2].copy_from_slice(&self.service.original_network_id.to_be_bytes());
        buf[pos + 2..pos + 4].copy_from_slice(&self.service.service_id.to_be_bytes());
        Ok(pos + SERVICE_REF_LEN)
    }
}

// --- ServiceDescAck ---

// OriginalNetworkID(2) + ServiceID(2) + flags/status/loop_len(3) = 7 fixed bytes.
const SERVICE_DESC_ACK_PREFIX: usize = SERVICE_REF_LEN + 3;

impl<'a> Parse<'a> for ServiceDescAck<'a> {
    type Error = Error;
    fn parse(bytes: &'a [u8]) -> Result<Self> {
        let body = objects::parse_apdu_header(bytes, tag::SERVICE_DESC_ACK, "ServiceDescAck")?;
        if body.len() < SERVICE_DESC_ACK_PREFIX {
            return Err(Error::BufferTooShort {
                need: SERVICE_DESC_ACK_PREFIX,
                have: body.len(),
                what: "ServiceDescAck",
            });
        }
        // byte 4: reserved_future_use(6) + EIT_schedule(1) + EIT_present_following(1)
        let flags = body[4];
        // byte 5..6: running_status(3) + free_CA_mode(1) + descriptors_loop_length(12)
        let b5 = body[5];
        let b6 = body[6];
        let running_status = (b5 >> 5) & 0x07;
        let free_ca_mode = (b5 & 0x10) != 0;
        let loop_len = ((u16::from(b5 & 0x0F) << 8) | u16::from(b6)) as usize;
        let desc_start = SERVICE_DESC_ACK_PREFIX;
        let desc_end = desc_start + loop_len;
        if body.len() < desc_end {
            return Err(Error::LengthMismatch {
                what: "ServiceDescAck descriptors",
                declared: loop_len,
                actual: body.len() - desc_start,
            });
        }
        Ok(Self {
            service: ServiceReference {
                original_network_id: u16::from_be_bytes([body[0], body[1]]),
                service_id: u16::from_be_bytes([body[2], body[3]]),
            },
            eit_schedule_flag: (flags & 0x02) != 0,
            eit_present_following_flag: (flags & 0x01) != 0,
            running_status,
            free_ca_mode,
            descriptors: &body[desc_start..desc_end],
        })
    }
}
impl Serialize for ServiceDescAck<'_> {
    type Error = Error;
    fn serialized_len(&self) -> usize {
        objects::apdu_len(SERVICE_DESC_ACK_PREFIX + self.descriptors.len())
    }
    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        if self.descriptors.len() > 0x0FFF {
            return Err(Error::InvalidObject {
                what: "ServiceDescAck",
                reason: "descriptors loop longer than 4095 bytes",
            });
        }
        let body_len = SERVICE_DESC_ACK_PREFIX + self.descriptors.len();
        let mut pos = objects::write_apdu_header(tag::SERVICE_DESC_ACK, body_len, buf)?;
        buf[pos..pos + 2].copy_from_slice(&self.service.original_network_id.to_be_bytes());
        buf[pos + 2..pos + 4].copy_from_slice(&self.service.service_id.to_be_bytes());
        // reserved_future_use(6)=0b111111, EIT_schedule(1), EIT_present_following(1).
        buf[pos + 4] = 0xFC
            | (u8::from(self.eit_schedule_flag) << 1)
            | u8::from(self.eit_present_following_flag);
        let loop_len = self.descriptors.len() as u16;
        // running_status(3), free_CA_mode(1), descriptors_loop_length(12).
        buf[pos + 5] = ((self.running_status & 0x07) << 5)
            | (u8::from(self.free_ca_mode) << 4)
            | ((loop_len >> 8) as u8 & 0x0F);
        buf[pos + 6] = loop_len as u8;
        pos += SERVICE_DESC_ACK_PREFIX;
        buf[pos..pos + self.descriptors.len()].copy_from_slice(self.descriptors);
        Ok(pos + self.descriptors.len())
    }
}

// --- GetServiceReq ---

impl<'a> Parse<'a> for GetServiceReq {
    type Error = Error;
    fn parse(bytes: &'a [u8]) -> Result<Self> {
        let body = objects::parse_apdu_header(bytes, tag::GET_SERVICE_REQ, "GetServiceReq")?;
        let service = if body.is_empty() {
            None
        } else {
            if body.len() < SERVICE_REF_LEN {
                return Err(Error::BufferTooShort {
                    need: SERVICE_REF_LEN,
                    have: body.len(),
                    what: "GetServiceReq",
                });
            }
            Some(ServiceReference {
                original_network_id: u16::from_be_bytes([body[0], body[1]]),
                service_id: u16::from_be_bytes([body[2], body[3]]),
            })
        };
        Ok(Self { service })
    }
}
impl Serialize for GetServiceReq {
    type Error = Error;
    fn serialized_len(&self) -> usize {
        objects::apdu_len(if self.service.is_some() {
            SERVICE_REF_LEN
        } else {
            0
        })
    }
    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        match self.service {
            None => objects::write_apdu_header(tag::GET_SERVICE_REQ, 0, buf),
            Some(s) => {
                let pos = objects::write_apdu_header(tag::GET_SERVICE_REQ, SERVICE_REF_LEN, buf)?;
                buf[pos..pos + 2].copy_from_slice(&s.original_network_id.to_be_bytes());
                buf[pos + 2..pos + 4].copy_from_slice(&s.service_id.to_be_bytes());
                Ok(pos + SERVICE_REF_LEN)
            }
        }
    }
}

// --- GetServiceAck ---

// OriginalNetworkID(2) + ServiceID(2) + flags(1) + ActualService(2).
const GET_SERVICE_ACK_BODY: usize = SERVICE_REF_LEN + 1 + 2;

impl<'a> Parse<'a> for GetServiceAck {
    type Error = Error;
    fn parse(bytes: &'a [u8]) -> Result<Self> {
        let body = objects::parse_apdu_header(bytes, tag::GET_SERVICE_ACK, "GetServiceAck")?;
        if body.len() < GET_SERVICE_ACK_BODY {
            return Err(Error::BufferTooShort {
                need: GET_SERVICE_ACK_BODY,
                have: body.len(),
                what: "GetServiceAck",
            });
        }
        // byte 4: Reserved(5) + ServiceTerminated(1) + ServiceNotAvailable(1) + CAServiceFlag(1)
        let flags = body[4];
        Ok(Self {
            service: ServiceReference {
                original_network_id: u16::from_be_bytes([body[0], body[1]]),
                service_id: u16::from_be_bytes([body[2], body[3]]),
            },
            service_terminated: (flags & 0x04) != 0,
            service_not_available: (flags & 0x02) != 0,
            ca_service_flag: (flags & 0x01) != 0,
            actual_service: u16::from_be_bytes([body[5], body[6]]),
        })
    }
}
impl Serialize for GetServiceAck {
    type Error = Error;
    fn serialized_len(&self) -> usize {
        objects::apdu_len(GET_SERVICE_ACK_BODY)
    }
    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        let pos = objects::write_apdu_header(tag::GET_SERVICE_ACK, GET_SERVICE_ACK_BODY, buf)?;
        buf[pos..pos + 2].copy_from_slice(&self.service.original_network_id.to_be_bytes());
        buf[pos + 2..pos + 4].copy_from_slice(&self.service.service_id.to_be_bytes());
        // Reserved(5)=0, then the three flag bits.
        buf[pos + 4] = (u8::from(self.service_terminated) << 2)
            | (u8::from(self.service_not_available) << 1)
            | u8::from(self.ca_service_flag);
        buf[pos + 5..pos + 7].copy_from_slice(&self.actual_service.to_be_bytes());
        Ok(pos + GET_SERVICE_ACK_BODY)
    }
}

/// Resource-scoped dispatch over the Generic Service Gateway objects (Tables 22-30).
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub enum ServiceGatewayApdu<'a> {
    /// `ServiceListReq` (`9F 80 00`).
    ServiceListReq(ServiceListReq),
    /// `ServiceListAck` (`9F 80 01`).
    ServiceListAck(ServiceListAck),
    /// `ServiceListVersionReq` (`9F 80 02`).
    ServiceListVersionReq(ServiceListVersionReq),
    /// `ServiceListVersionAck` (`9F 80 03`).
    ServiceListVersionAck(ServiceListVersionAck),
    /// `ServiceListChanged` (`9F 80 04`).
    ServiceListChanged(ServiceListChanged),
    /// `ServiceDescReq` (`9F 80 05`).
    ServiceDescReq(ServiceDescReq),
    /// `ServiceDescAck` (`9F 80 06`).
    ServiceDescAck(ServiceDescAck<'a>),
    /// `GetServiceReq` (`9F 80 07`).
    GetServiceReq(GetServiceReq),
    /// `GetServiceAck` (`9F 80 08`).
    GetServiceAck(GetServiceAck),
}

impl<'a> ServiceGatewayApdu<'a> {
    /// Parse a Generic Service Gateway APDU, dispatching on the leading `apdu_tag`.
    pub fn parse(body: &'a [u8]) -> Result<Self> {
        if body.len() < 3 {
            return Err(Error::BufferTooShort {
                need: 3,
                have: body.len(),
                what: "service_gateway apdu_tag",
            });
        }
        let t = ApduTag::from_bytes(body[0], body[1], body[2]);
        match t {
            tag::SERVICE_LIST_REQ => Ok(Self::ServiceListReq(ServiceListReq::parse(body)?)),
            tag::SERVICE_LIST_ACK => Ok(Self::ServiceListAck(ServiceListAck::parse(body)?)),
            tag::SERVICE_LIST_VERSION_REQ => Ok(Self::ServiceListVersionReq(
                ServiceListVersionReq::parse(body)?,
            )),
            tag::SERVICE_LIST_VERSION_ACK => Ok(Self::ServiceListVersionAck(
                ServiceListVersionAck::parse(body)?,
            )),
            tag::SERVICE_LIST_CHANGED => {
                Ok(Self::ServiceListChanged(ServiceListChanged::parse(body)?))
            }
            tag::SERVICE_DESC_REQ => Ok(Self::ServiceDescReq(ServiceDescReq::parse(body)?)),
            tag::SERVICE_DESC_ACK => Ok(Self::ServiceDescAck(ServiceDescAck::parse(body)?)),
            tag::GET_SERVICE_REQ => Ok(Self::GetServiceReq(GetServiceReq::parse(body)?)),
            tag::GET_SERVICE_ACK => Ok(Self::GetServiceAck(GetServiceAck::parse(body)?)),
            _ => Err(Error::UnexpectedApduTag {
                got: t.as_u24(),
                expected: tag::SERVICE_LIST_REQ.as_u24(),
                what: "service_gateway",
            }),
        }
    }
}

impl Serialize for ServiceGatewayApdu<'_> {
    type Error = Error;
    fn serialized_len(&self) -> usize {
        match self {
            Self::ServiceListReq(o) => o.serialized_len(),
            Self::ServiceListAck(o) => o.serialized_len(),
            Self::ServiceListVersionReq(o) => o.serialized_len(),
            Self::ServiceListVersionAck(o) => o.serialized_len(),
            Self::ServiceListChanged(o) => o.serialized_len(),
            Self::ServiceDescReq(o) => o.serialized_len(),
            Self::ServiceDescAck(o) => o.serialized_len(),
            Self::GetServiceReq(o) => o.serialized_len(),
            Self::GetServiceAck(o) => o.serialized_len(),
        }
    }
    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        match self {
            Self::ServiceListReq(o) => o.serialize_into(buf),
            Self::ServiceListAck(o) => o.serialize_into(buf),
            Self::ServiceListVersionReq(o) => o.serialize_into(buf),
            Self::ServiceListVersionAck(o) => o.serialize_into(buf),
            Self::ServiceListChanged(o) => o.serialize_into(buf),
            Self::ServiceDescReq(o) => o.serialize_into(buf),
            Self::ServiceDescAck(o) => o.serialize_into(buf),
            Self::GetServiceReq(o) => o.serialize_into(buf),
            Self::GetServiceAck(o) => o.serialize_into(buf),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn header_only_objects_round_trip() {
        assert_eq!(ServiceListReq.to_bytes(), [0x9F, 0x80, 0x00, 0x00]);
        assert_eq!(ServiceListVersionReq.to_bytes(), [0x9F, 0x80, 0x02, 0x00]);
        assert_eq!(
            ServiceListReq::parse(&[0x9F, 0x80, 0x00, 0x00]).unwrap(),
            ServiceListReq
        );
    }

    #[test]
    fn service_list_ack_multi_round_trips_and_bites() {
        let ack = ServiceListAck {
            version_number: 0x07,
            services: alloc::vec![
                ServiceReference {
                    original_network_id: 0x0001,
                    service_id: 0x0064,
                },
                ServiceReference {
                    original_network_id: 0x0001,
                    service_id: 0x0065,
                },
            ],
        };
        let bytes = ack.to_bytes();
        // tag(3) + len(1) + ver(1) + count(2) + 2*4 = 15; body = 11 = 0x0B.
        assert_eq!(
            bytes,
            [
                0x9F, 0x80, 0x01, 0x0B, 0x07, 0x00, 0x02, 0x00, 0x01, 0x00, 0x64, 0x00, 0x01, 0x00,
                0x65
            ]
        );
        assert_eq!(ServiceListAck::parse(&bytes).unwrap(), ack);
        let mut other = ack.clone();
        other.services[1].service_id = 0x0066;
        assert_ne!(bytes, other.to_bytes());
    }

    #[test]
    fn service_list_ack_empty_round_trips() {
        let ack = ServiceListAck {
            version_number: 0x02,
            services: alloc::vec![],
        };
        let bytes = ack.to_bytes();
        assert_eq!(bytes, [0x9F, 0x80, 0x01, 0x03, 0x02, 0x00, 0x00]);
        assert_eq!(ServiceListAck::parse(&bytes).unwrap(), ack);
    }

    #[test]
    fn version_ack_and_changed_round_trip_and_bite() {
        let v = ServiceListVersionAck { version_number: 9 };
        let bytes = v.to_bytes();
        assert_eq!(bytes, [0x9F, 0x80, 0x03, 0x01, 0x09]);
        assert_eq!(ServiceListVersionAck::parse(&bytes).unwrap(), v);
        assert_ne!(
            bytes,
            ServiceListVersionAck { version_number: 10 }.to_bytes()
        );

        let c = ServiceListChanged { version_number: 3 };
        let cbytes = c.to_bytes();
        assert_eq!(cbytes, [0x9F, 0x80, 0x04, 0x01, 0x03]);
        assert_eq!(ServiceListChanged::parse(&cbytes).unwrap(), c);
    }

    #[test]
    fn service_desc_req_round_trips_and_bites() {
        let r = ServiceDescReq {
            service: ServiceReference {
                original_network_id: 0x1234,
                service_id: 0x5678,
            },
        };
        let bytes = r.to_bytes();
        assert_eq!(bytes, [0x9F, 0x80, 0x05, 0x04, 0x12, 0x34, 0x56, 0x78]);
        assert_eq!(ServiceDescReq::parse(&bytes).unwrap(), r);
        let mut other = r;
        other.service.service_id = 0x5679;
        assert_ne!(bytes, other.to_bytes());
    }

    #[test]
    fn service_desc_ack_round_trips_and_bites() {
        // 2 descriptors: 0x48 (service descriptor) len 2, and 0x52 len 1.
        let desc = [0x48, 0x02, 0xAA, 0xBB, 0x52, 0x01, 0x03];
        let ack = ServiceDescAck {
            service: ServiceReference {
                original_network_id: 0x0001,
                service_id: 0x0064,
            },
            eit_schedule_flag: true,
            eit_present_following_flag: false,
            running_status: 4, // running
            free_ca_mode: true,
            descriptors: &desc,
        };
        let bytes = ack.to_bytes();
        // prefix(7) + 7 desc = 14 body; tag(3)+len(1)+14 = 18.
        // byte4 = 0xFC | (1<<1) | 0 = 0xFE
        // byte5 = (4<<5) | (1<<4) | (loop_len>>8) = 0x80 | 0x10 | 0x00 = 0x90
        // byte6 = loop_len = 7
        assert_eq!(
            bytes,
            [
                0x9F, 0x80, 0x06, 0x0E, 0x00, 0x01, 0x00, 0x64, 0xFE, 0x90, 0x07, 0x48, 0x02, 0xAA,
                0xBB, 0x52, 0x01, 0x03
            ]
        );
        let parsed = ServiceDescAck::parse(&bytes).unwrap();
        assert_eq!(parsed, ack);
        assert_eq!(parsed.running_status, 4);
        assert!(parsed.eit_schedule_flag);
        assert!(!parsed.eit_present_following_flag);
        assert!(parsed.free_ca_mode);
        let mut other = ack.clone();
        other.running_status = 1;
        assert_ne!(bytes, other.to_bytes());
    }

    #[test]
    fn service_desc_ack_empty_loop() {
        let ack = ServiceDescAck {
            service: ServiceReference {
                original_network_id: 0xAAAA,
                service_id: 0xBBBB,
            },
            eit_schedule_flag: false,
            eit_present_following_flag: true,
            running_status: 0,
            free_ca_mode: false,
            descriptors: &[],
        };
        let bytes = ack.to_bytes();
        // byte4 = 0xFC | 0 | 1 = 0xFD ; byte5 = 0 ; byte6 = 0
        assert_eq!(
            bytes,
            [0x9F, 0x80, 0x06, 0x07, 0xAA, 0xAA, 0xBB, 0xBB, 0xFD, 0x00, 0x00]
        );
        assert_eq!(ServiceDescAck::parse(&bytes).unwrap(), ack);
    }

    #[test]
    fn get_service_req_with_and_without_ref() {
        let r = GetServiceReq {
            service: Some(ServiceReference {
                original_network_id: 0x0001,
                service_id: 0x0064,
            }),
        };
        let bytes = r.to_bytes();
        assert_eq!(bytes, [0x9F, 0x80, 0x07, 0x04, 0x00, 0x01, 0x00, 0x64]);
        assert_eq!(GetServiceReq::parse(&bytes).unwrap(), r);

        let disconnect = GetServiceReq { service: None };
        let dbytes = disconnect.to_bytes();
        assert_eq!(dbytes, [0x9F, 0x80, 0x07, 0x00]);
        assert_eq!(GetServiceReq::parse(&dbytes).unwrap(), disconnect);
        assert_ne!(bytes, dbytes);
    }

    #[test]
    fn get_service_ack_round_trips_and_bites() {
        let ack = GetServiceAck {
            service: ServiceReference {
                original_network_id: 0x0001,
                service_id: 0x0064,
            },
            service_terminated: false,
            service_not_available: false,
            ca_service_flag: true,
            actual_service: 0x0064,
        };
        let bytes = ack.to_bytes();
        // byte4 = CAServiceFlag = 0x01 ; ActualService = 0x0064
        assert_eq!(
            bytes,
            [0x9F, 0x80, 0x08, 0x07, 0x00, 0x01, 0x00, 0x64, 0x01, 0x00, 0x64]
        );
        assert_eq!(GetServiceAck::parse(&bytes).unwrap(), ack);
        // Table 31: a "not available" combination.
        let mut other = ack;
        other.ca_service_flag = false;
        other.service_not_available = true;
        other.actual_service = 0;
        assert_ne!(bytes, other.to_bytes());
        assert_eq!(other.to_bytes()[8], 0x02);
    }

    #[test]
    fn dispatch_routes_each_tag() {
        let req = ServiceListReq.to_bytes();
        assert!(matches!(
            ServiceGatewayApdu::parse(&req).unwrap(),
            ServiceGatewayApdu::ServiceListReq(_)
        ));
        let gs = GetServiceAck {
            service: ServiceReference::default(),
            service_terminated: true,
            service_not_available: false,
            ca_service_flag: false,
            actual_service: 0,
        }
        .to_bytes();
        let parsed = ServiceGatewayApdu::parse(&gs).unwrap();
        assert!(matches!(parsed, ServiceGatewayApdu::GetServiceAck(_)));
        assert_eq!(parsed.to_bytes(), gs);
    }
}
