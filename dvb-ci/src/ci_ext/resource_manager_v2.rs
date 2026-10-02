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
use alloc::vec::Vec;
use broadcast_common::{Parse, Serialize};

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
broadcast_common::impl_spec_display!(ModuleIdCommandKind, Reserved);

/// `module_id_command()` — host acknowledges or sets the Module ID (Table 7).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct ModuleIdCommand {
    /// `command`.
    pub command: ModuleIdCommandKind,
    /// The 6-bit Module ID (significant only for `Set_ModuleID`).
    pub module_id: u8,
}

// --- profile_enq / profile_changed / profile_reply ---
//
// Layout-identical to EN 50221 §8.4.1 (module doc): delegate to
// `objects::resource_manager` rather than re-implementing the framing and the
// resource-id list walk (audit r10-O-5).

use crate::objects::resource_manager as v1;

impl<'a> Parse<'a> for ProfileEnq {
    type Error = Error;
    fn parse(bytes: &'a [u8]) -> Result<Self> {
        v1::ProfileEnq::parse(bytes)?;
        Ok(Self)
    }
}
impl Serialize for ProfileEnq {
    type Error = Error;
    fn serialized_len(&self) -> usize {
        v1::ProfileEnq.serialized_len()
    }
    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        v1::ProfileEnq.serialize_into(buf)
    }
}

impl<'a> Parse<'a> for ProfileChanged {
    type Error = Error;
    fn parse(bytes: &'a [u8]) -> Result<Self> {
        v1::ProfileChange::parse(bytes)?;
        Ok(Self)
    }
}
impl Serialize for ProfileChanged {
    type Error = Error;
    fn serialized_len(&self) -> usize {
        v1::ProfileChange.serialized_len()
    }
    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        v1::ProfileChange.serialize_into(buf)
    }
}

impl<'a> Parse<'a> for ProfileReply {
    type Error = Error;
    fn parse(bytes: &'a [u8]) -> Result<Self> {
        Ok(Self {
            resources: v1::Profile::parse(bytes)?.resources,
        })
    }
}
impl Serialize for ProfileReply {
    type Error = Error;
    fn serialized_len(&self) -> usize {
        objects::apdu_len(self.resources.len() * ResourceId::LEN)
    }
    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        // `Profile` owns its list, so this clones the (4-byte) ids once.
        v1::Profile {
            resources: self.resources.clone(),
        }
        .serialize_into(buf)
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
        crate::objects::reject_trailing_body(body, MODULE_ID_SEND_BODY, "module_id_send")?;
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
        objects::fit_bits(u64::from(self.module_id), 6, "module_id_send module_id")?;
        let pos = objects::write_apdu_header(tag::MODULE_ID_SEND, MODULE_ID_SEND_BODY, buf)?;
        // reserved(2)='00', module_id(6). Range-checked above.
        buf[pos] = self.module_id;
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
        crate::objects::reject_trailing_body(body, MODULE_ID_COMMAND_BODY, "module_id_command")?;
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
        objects::fit_bits(u64::from(self.module_id), 6, "module_id_command module_id")?;
        let pos = objects::write_apdu_header(tag::MODULE_ID_COMMAND, MODULE_ID_COMMAND_BODY, buf)?;
        buf[pos] = self.command.to_u8();
        // reserved(2)='00', module_id(6). Range-checked above.
        buf[pos + 1] = self.module_id;
        Ok(pos + MODULE_ID_COMMAND_BODY)
    }
}

crate::dispatch::declare_resource_apdus! {
    /// Resource-scoped dispatch over the Resource Manager v2 objects (Tables 3-7).
    #[derive(Debug, Clone, PartialEq, Eq)]
    #[cfg_attr(feature = "serde", derive(serde::Serialize))]
    #[non_exhaustive]
    pub enum ResourceManagerV2Apdu ("resource_manager_v2") {
        /// `profile_enq` (`9F 80 10`).
        ProfileEnq(ProfileEnq) = tag::PROFILE_ENQ,
        /// `profile_reply` (`9F 80 11`).
        ProfileReply(ProfileReply) = tag::PROFILE_REPLY,
        /// `profile_changed` (`9F 80 12`).
        ProfileChanged(ProfileChanged) = tag::PROFILE_CHANGED,
        /// `module_id_send` (`9F 80 13`).
        ModuleIdSend(ModuleIdSend) = tag::MODULE_ID_SEND,
        /// `module_id_command` (`9F 80 14`).
        ModuleIdCommand(ModuleIdCommand) = tag::MODULE_ID_COMMAND,
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
            [
                0x9F, 0x80, 0x11, 0x08, 0x00, 0x01, 0x00, 0x42, 0x00, 0x02, 0x00, 0x42
            ]
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

    #[test]
    fn oversized_module_id_send_is_rejected_not_wrapped() {
        // Before the fix, 0x43 (exceeds 6 bits) silently wrapped to 0x03 in
        // the module_id field and returned Ok.
        let m = ModuleIdSend { module_id: 0x43 };
        let mut buf = [0u8; 16];
        assert!(matches!(
            m.serialize_into(&mut buf),
            Err(Error::InvalidObject { .. })
        ));
    }

    #[test]
    fn max_module_id_send_still_serializes_and_round_trips() {
        let m = ModuleIdSend { module_id: 0x3F }; // the 6-bit maximum
        let bytes = m.to_bytes();
        assert_eq!(ModuleIdSend::parse(&bytes).unwrap(), m);
    }

    #[test]
    fn oversized_module_id_command_is_rejected_not_wrapped() {
        let m = ModuleIdCommand {
            command: ModuleIdCommandKind::SetModuleId,
            module_id: 0x43,
        };
        let mut buf = [0u8; 16];
        assert!(matches!(
            m.serialize_into(&mut buf),
            Err(Error::InvalidObject { .. })
        ));
    }
}
