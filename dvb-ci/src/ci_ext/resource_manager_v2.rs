//! Resource Manager v2 objects — ETSI TS 101 699 V1.1.1 §4.2.1, Tables 3-7
//! (PDF pp. 13-17). See `docs/ci_plus/resource-manager-v2.md`.
//!
//! Resource ID `0x00010042`. Adds Module ID establishment to the EN 50221 v1
//! Resource Manager. The three v1 objects (Profile Enquiry, Profile Reply,
//! Profile Changed) are layout-identical to EN 50221 but are re-defined here so
//! the v2 resource owns its own object set:
//!
//! - `profile_enq` (`9F 80 10`, Table 3) — header-only enquiry.
//! - `profile_reply` (`9F 80 11`, Table 4) — list of `resource_identifier()`s.
//! - `profile_changed` (`9F 80 12`, Table 5) — header-only notification.
//! - `module_id_send` (`9F 80 13`, Table 6) — module returns its Module ID.
//! - `module_id_command` (`9F 80 14`, Table 7) — host ack / sets the Module ID.

use crate::error::{Error, Result};
use crate::objects;
use crate::resource::ResourceId;
use crate::tag::ApduTag;
use alloc::vec::Vec;
use dvb_common::{Parse, Serialize};

/// Resource-scoped `apdu_tag`s for the Resource Manager v2 (Table 87 / Tables 3-7).
pub mod tag {
    use crate::tag::ApduTag;
    /// `Tprofile_enq` = `9F 80 10`.
    pub const PROFILE_ENQ: ApduTag = ApduTag::from_bytes(0x9F, 0x80, 0x10);
    /// `Tprofile_reply` = `9F 80 11`.
    pub const PROFILE_REPLY: ApduTag = ApduTag::from_bytes(0x9F, 0x80, 0x11);
    /// `Tprofile_changed` = `9F 80 12`.
    pub const PROFILE_CHANGED: ApduTag = ApduTag::from_bytes(0x9F, 0x80, 0x12);
    /// `Tmodule_id_send` = `9F 80 13`.
    pub const MODULE_ID_SEND: ApduTag = ApduTag::from_bytes(0x9F, 0x80, 0x13);
    /// `Tmodule_id_command` = `9F 80 14`.
    pub const MODULE_ID_COMMAND: ApduTag = ApduTag::from_bytes(0x9F, 0x80, 0x14);
}

/// `profile_enq()` — Profile Enquiry, empty body (Table 3).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct ProfileEnq;

/// `profile_changed()` — Profile Changed, empty body (Table 5).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct ProfileChanged;

/// `profile_reply()` — the list of resources the sender provides (Table 4).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct ProfileReply {
    /// Advertised `resource_identifier()`s, in wire order (`length_field = N*4`).
    pub resources: Vec<ResourceId>,
}

/// `module_id_send()` — module returns its current Module ID (Table 6).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct ModuleIdSend {
    /// The 6-bit Module ID (`0` if the host has not allocated one). Only the low
    /// 6 bits are significant; the top 2 bits are reserved.
    pub module_id: u8,
}

/// `command` values for `module_id_command()` (Table 7).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub enum ModuleIdCommandKind {
    /// `0x01` — host accepts the Module ID; module continues to Profile.
    Acknowledgement,
    /// `0x02` — `module_id` carries a new ID to set.
    SetModuleId,
    /// Any other value (reserved).
    Reserved(u8),
}

impl ModuleIdCommandKind {
    /// Decode a `command` byte.
    #[must_use]
    pub fn from_u8(v: u8) -> Self {
        match v {
            0x01 => Self::Acknowledgement,
            0x02 => Self::SetModuleId,
            other => Self::Reserved(other),
        }
    }
    /// Wire byte.
    #[must_use]
    pub const fn to_u8(self) -> u8 {
        match self {
            Self::Acknowledgement => 0x01,
            Self::SetModuleId => 0x02,
            Self::Reserved(v) => v,
        }
    }
    /// Spec token, or `"reserved"`.
    #[must_use]
    pub fn name(&self) -> &'static str {
        match self {
            Self::Acknowledgement => "Acknowledgement",
            Self::SetModuleId => "Set_ModuleID",
            Self::Reserved(_) => "reserved",
        }
    }
}
dvb_common::impl_spec_display!(ModuleIdCommandKind, Reserved);

/// `module_id_command()` — host acknowledges or sets the Module ID (Table 7).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct ModuleIdCommand {
    /// `command`.
    pub command: ModuleIdCommandKind,
    /// The 6-bit Module ID (significant only for `Set_ModuleID`).
    pub module_id: u8,
}

// --- profile_enq / profile_changed (empty body) ---

impl<'a> Parse<'a> for ProfileEnq {
    type Error = Error;
    fn parse(bytes: &'a [u8]) -> Result<Self> {
        objects::parse_empty_apdu(bytes, tag::PROFILE_ENQ, "profile_enq")?;
        Ok(Self)
    }
}
impl Serialize for ProfileEnq {
    type Error = Error;
    fn serialized_len(&self) -> usize {
        objects::empty_apdu_len()
    }
    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        objects::serialize_empty_apdu(tag::PROFILE_ENQ, buf)
    }
}

impl<'a> Parse<'a> for ProfileChanged {
    type Error = Error;
    fn parse(bytes: &'a [u8]) -> Result<Self> {
        objects::parse_empty_apdu(bytes, tag::PROFILE_CHANGED, "profile_changed")?;
        Ok(Self)
    }
}
impl Serialize for ProfileChanged {
    type Error = Error;
    fn serialized_len(&self) -> usize {
        objects::empty_apdu_len()
    }
    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        objects::serialize_empty_apdu(tag::PROFILE_CHANGED, buf)
    }
}

// --- profile_reply (resource-id list) ---

impl<'a> Parse<'a> for ProfileReply {
    type Error = Error;
    fn parse(bytes: &'a [u8]) -> Result<Self> {
        let body = objects::parse_apdu_header(bytes, tag::PROFILE_REPLY, "profile_reply")?;
        if body.len() % ResourceId::LEN != 0 {
            return Err(Error::InvalidObject {
                what: "profile_reply",
                reason: "body length is not a multiple of 4",
            });
        }
        let mut resources = Vec::with_capacity(body.len() / ResourceId::LEN);
        for chunk in body.chunks_exact(ResourceId::LEN) {
            resources.push(ResourceId::parse(chunk)?);
        }
        Ok(Self { resources })
    }
}
impl Serialize for ProfileReply {
    type Error = Error;
    fn serialized_len(&self) -> usize {
        objects::apdu_len(self.resources.len() * ResourceId::LEN)
    }
    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        let body_len = self.resources.len() * ResourceId::LEN;
        let mut pos = objects::write_apdu_header(tag::PROFILE_REPLY, body_len, buf)?;
        for r in &self.resources {
            pos += r.serialize_into(&mut buf[pos..])?;
        }
        Ok(pos)
    }
}

// --- module_id_send ---

// reserved(2) + module_id(6).
const MODULE_ID_SEND_BODY: usize = 1;

impl<'a> Parse<'a> for ModuleIdSend {
    type Error = Error;
    fn parse(bytes: &'a [u8]) -> Result<Self> {
        let body = objects::parse_apdu_header(bytes, tag::MODULE_ID_SEND, "module_id_send")?;
        if body.len() < MODULE_ID_SEND_BODY {
            return Err(Error::BufferTooShort {
                need: MODULE_ID_SEND_BODY,
                have: body.len(),
                what: "module_id_send",
            });
        }
        Ok(Self {
            module_id: body[0] & 0x3F,
        })
    }
}
impl Serialize for ModuleIdSend {
    type Error = Error;
    fn serialized_len(&self) -> usize {
        objects::apdu_len(MODULE_ID_SEND_BODY)
    }
    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        let pos = objects::write_apdu_header(tag::MODULE_ID_SEND, MODULE_ID_SEND_BODY, buf)?;
        // reserved(2)='00', module_id(6).
        buf[pos] = self.module_id & 0x3F;
        Ok(pos + MODULE_ID_SEND_BODY)
    }
}

// --- module_id_command ---

// command(8) + reserved(2) + module_id(6).
const MODULE_ID_COMMAND_BODY: usize = 2;

impl<'a> Parse<'a> for ModuleIdCommand {
    type Error = Error;
    fn parse(bytes: &'a [u8]) -> Result<Self> {
        let body = objects::parse_apdu_header(bytes, tag::MODULE_ID_COMMAND, "module_id_command")?;
        if body.len() < MODULE_ID_COMMAND_BODY {
            return Err(Error::BufferTooShort {
                need: MODULE_ID_COMMAND_BODY,
                have: body.len(),
                what: "module_id_command",
            });
        }
        Ok(Self {
            command: ModuleIdCommandKind::from_u8(body[0]),
            module_id: body[1] & 0x3F,
        })
    }
}
impl Serialize for ModuleIdCommand {
    type Error = Error;
    fn serialized_len(&self) -> usize {
        objects::apdu_len(MODULE_ID_COMMAND_BODY)
    }
    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        let pos = objects::write_apdu_header(tag::MODULE_ID_COMMAND, MODULE_ID_COMMAND_BODY, buf)?;
        buf[pos] = self.command.to_u8();
        buf[pos + 1] = self.module_id & 0x3F;
        Ok(pos + MODULE_ID_COMMAND_BODY)
    }
}

/// Resource-scoped dispatch over the Resource Manager v2 objects (Tables 3-7).
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub enum ResourceManagerV2Apdu {
    /// `profile_enq` (`9F 80 10`).
    ProfileEnq(ProfileEnq),
    /// `profile_reply` (`9F 80 11`).
    ProfileReply(ProfileReply),
    /// `profile_changed` (`9F 80 12`).
    ProfileChanged(ProfileChanged),
    /// `module_id_send` (`9F 80 13`).
    ModuleIdSend(ModuleIdSend),
    /// `module_id_command` (`9F 80 14`).
    ModuleIdCommand(ModuleIdCommand),
}

impl ResourceManagerV2Apdu {
    /// Parse a Resource Manager v2 APDU, dispatching on the leading `apdu_tag`.
    pub fn parse(body: &[u8]) -> Result<Self> {
        if body.len() < 3 {
            return Err(Error::BufferTooShort {
                need: 3,
                have: body.len(),
                what: "resource_manager_v2 apdu_tag",
            });
        }
        let t = ApduTag::from_bytes(body[0], body[1], body[2]);
        match t {
            tag::PROFILE_ENQ => Ok(Self::ProfileEnq(ProfileEnq::parse(body)?)),
            tag::PROFILE_REPLY => Ok(Self::ProfileReply(ProfileReply::parse(body)?)),
            tag::PROFILE_CHANGED => Ok(Self::ProfileChanged(ProfileChanged::parse(body)?)),
            tag::MODULE_ID_SEND => Ok(Self::ModuleIdSend(ModuleIdSend::parse(body)?)),
            tag::MODULE_ID_COMMAND => Ok(Self::ModuleIdCommand(ModuleIdCommand::parse(body)?)),
            _ => Err(Error::UnexpectedApduTag {
                got: t.as_u24(),
                expected: tag::PROFILE_ENQ.as_u24(),
                what: "resource_manager_v2",
            }),
        }
    }
}

impl Serialize for ResourceManagerV2Apdu {
    type Error = Error;
    fn serialized_len(&self) -> usize {
        match self {
            Self::ProfileEnq(o) => o.serialized_len(),
            Self::ProfileReply(o) => o.serialized_len(),
            Self::ProfileChanged(o) => o.serialized_len(),
            Self::ModuleIdSend(o) => o.serialized_len(),
            Self::ModuleIdCommand(o) => o.serialized_len(),
        }
    }
    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        match self {
            Self::ProfileEnq(o) => o.serialize_into(buf),
            Self::ProfileReply(o) => o.serialize_into(buf),
            Self::ProfileChanged(o) => o.serialize_into(buf),
            Self::ModuleIdSend(o) => o.serialize_into(buf),
            Self::ModuleIdCommand(o) => o.serialize_into(buf),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn profile_enq_round_trips() {
        let bytes = ProfileEnq.to_bytes();
        assert_eq!(bytes, [0x9F, 0x80, 0x10, 0x00]);
        assert_eq!(ProfileEnq::parse(&bytes).unwrap(), ProfileEnq);
    }

    #[test]
    fn profile_changed_round_trips() {
        let bytes = ProfileChanged.to_bytes();
        assert_eq!(bytes, [0x9F, 0x80, 0x12, 0x00]);
        assert_eq!(ProfileChanged::parse(&bytes).unwrap(), ProfileChanged);
    }

    #[test]
    fn profile_reply_multi_round_trips_and_bites() {
        let p = ProfileReply {
            resources: alloc::vec![ResourceId(0x0001_0042), ResourceId(0x0002_0042)],
        };
        let bytes = p.to_bytes();
        // tag(3) + len(1) + 2*4 = 12; len = 0x08.
        assert_eq!(
            bytes,
            [0x9F, 0x80, 0x11, 0x08, 0x00, 0x01, 0x00, 0x42, 0x00, 0x02, 0x00, 0x42]
        );
        assert_eq!(ProfileReply::parse(&bytes).unwrap(), p);
        let mut other = p.clone();
        other.resources[1] = ResourceId(0x0022_0041);
        assert_ne!(bytes, other.to_bytes());
    }

    #[test]
    fn module_id_send_round_trips_and_bites() {
        let m = ModuleIdSend { module_id: 0x03 };
        let bytes = m.to_bytes();
        assert_eq!(bytes, [0x9F, 0x80, 0x13, 0x01, 0x03]);
        assert_eq!(ModuleIdSend::parse(&bytes).unwrap(), m);
        // top 2 bits ignored on read.
        let parsed = ModuleIdSend::parse(&[0x9F, 0x80, 0x13, 0x01, 0xC3]).unwrap();
        assert_eq!(parsed.module_id, 0x03);
        let other = ModuleIdSend { module_id: 0x04 };
        assert_ne!(bytes, other.to_bytes());
    }

    #[test]
    fn module_id_command_round_trips_and_bites() {
        let m = ModuleIdCommand {
            command: ModuleIdCommandKind::SetModuleId,
            module_id: 0x05,
        };
        let bytes = m.to_bytes();
        assert_eq!(bytes, [0x9F, 0x80, 0x14, 0x02, 0x02, 0x05]);
        assert_eq!(ModuleIdCommand::parse(&bytes).unwrap(), m);
        assert_eq!(m.command.name(), "Set_ModuleID");
        let mut other = m;
        other.command = ModuleIdCommandKind::Acknowledgement;
        assert_ne!(bytes, other.to_bytes());
    }

    #[test]
    fn dispatch_routes_each_tag() {
        let enq = ProfileEnq.to_bytes();
        assert!(matches!(
            ResourceManagerV2Apdu::parse(&enq).unwrap(),
            ResourceManagerV2Apdu::ProfileEnq(_)
        ));
        let mic = ModuleIdCommand {
            command: ModuleIdCommandKind::Acknowledgement,
            module_id: 1,
        }
        .to_bytes();
        let parsed = ResourceManagerV2Apdu::parse(&mic).unwrap();
        assert!(matches!(parsed, ResourceManagerV2Apdu::ModuleIdCommand(_)));
        // dispatch enum round-trips.
        assert_eq!(parsed.to_bytes(), mic);
    }
}
