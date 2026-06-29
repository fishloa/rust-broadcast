//! Resource Manager objects — ETSI EN 50221 §8.4.1, Tables 17-19 (PDF pp. 26-27).
//!
//! - `profile_enq` (`9F 80 10`, Table 17) — header-only enquiry.
//! - `profile` reply (`9F 80 11`, Table 18) — list of `resource_identifier()`s.
//! - `profile_change` (`9F 80 12`, Table 19) — header-only notification.

use crate::error::{Error, Result};
use crate::resource::ResourceId;
use crate::tag::{self, ApduTag};
use crate::traits::ApduDef;
use alloc::vec::Vec;
use broadcast_common::{Parse, Serialize};

/// `profile_enq()` — Profile Enquiry, an empty-body object (Table 17).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct ProfileEnq;

/// `profile_changed()` — Profile Changed, an empty-body object (Table 19).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct ProfileChange;

/// `profile()` reply — the list of resources the sender provides (Table 18).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct Profile {
    /// The advertised `resource_identifier()`s, in wire order.
    pub resources: Vec<ResourceId>,
}

// --- profile_enq (empty body) ---

impl<'a> Parse<'a> for ProfileEnq {
    type Error = Error;
    fn parse(bytes: &'a [u8]) -> Result<Self> {
        super::parse_empty_apdu(bytes, tag::PROFILE_ENQ, "profile_enq")?;
        Ok(Self)
    }
}
impl Serialize for ProfileEnq {
    type Error = Error;
    fn serialized_len(&self) -> usize {
        super::empty_apdu_len()
    }
    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        super::serialize_empty_apdu(tag::PROFILE_ENQ, buf)
    }
}
impl<'a> ApduDef<'a> for ProfileEnq {
    const TAG: ApduTag = tag::PROFILE_ENQ;
    const NAME: &'static str = "PROFILE_ENQ";
}

// --- profile_change (empty body) ---

impl<'a> Parse<'a> for ProfileChange {
    type Error = Error;
    fn parse(bytes: &'a [u8]) -> Result<Self> {
        super::parse_empty_apdu(bytes, tag::PROFILE_CHANGE, "profile_change")?;
        Ok(Self)
    }
}
impl Serialize for ProfileChange {
    type Error = Error;
    fn serialized_len(&self) -> usize {
        super::empty_apdu_len()
    }
    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        super::serialize_empty_apdu(tag::PROFILE_CHANGE, buf)
    }
}
impl<'a> ApduDef<'a> for ProfileChange {
    const TAG: ApduTag = tag::PROFILE_CHANGE;
    const NAME: &'static str = "PROFILE_CHANGE";
}

// --- profile (reply, resource-id list) ---

impl<'a> Parse<'a> for Profile {
    type Error = Error;
    fn parse(bytes: &'a [u8]) -> Result<Self> {
        let body = super::parse_apdu_header(bytes, tag::PROFILE, "profile")?;
        if body.len() % ResourceId::LEN != 0 {
            return Err(Error::InvalidObject {
                what: "profile",
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

impl Serialize for Profile {
    type Error = Error;
    fn serialized_len(&self) -> usize {
        let body = self.resources.len() * ResourceId::LEN;
        super::apdu_len(body)
    }
    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        let body_len = self.resources.len() * ResourceId::LEN;
        let mut pos = super::write_apdu_header(tag::PROFILE, body_len, buf)?;
        for r in &self.resources {
            pos += r.serialize_into(&mut buf[pos..])?;
        }
        Ok(pos)
    }
}

impl<'a> ApduDef<'a> for Profile {
    const TAG: ApduTag = tag::PROFILE;
    const NAME: &'static str = "PROFILE";
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::resource::{APPLICATION_INFORMATION, CONDITIONAL_ACCESS_SUPPORT, RESOURCE_MANAGER};

    #[test]
    fn profile_enq_round_trip() {
        let bytes = ProfileEnq.to_bytes();
        assert_eq!(bytes, [0x9F, 0x80, 0x10, 0x00]);
        assert_eq!(ProfileEnq::parse(&bytes).unwrap(), ProfileEnq);
    }

    #[test]
    fn profile_change_round_trip() {
        let bytes = ProfileChange.to_bytes();
        assert_eq!(bytes, [0x9F, 0x80, 0x12, 0x00]);
        assert_eq!(ProfileChange::parse(&bytes).unwrap(), ProfileChange);
    }

    #[test]
    fn profile_reply_multi_resource_round_trips() {
        let p = Profile {
            resources: alloc::vec![
                RESOURCE_MANAGER,
                APPLICATION_INFORMATION,
                CONDITIONAL_ACCESS_SUPPORT,
            ],
        };
        let bytes = p.to_bytes();
        // header(3) + length(1) + 3*4 = 16
        assert_eq!(bytes.len(), 16);
        assert_eq!(&bytes[..4], &[0x9F, 0x80, 0x11, 0x0C]); // length 12
        let parsed = Profile::parse(&bytes).unwrap();
        assert_eq!(parsed, p);
        assert_eq!(parsed.resources.len(), 3);
    }

    #[test]
    fn mutating_resource_changes_bytes() {
        let p = Profile {
            resources: alloc::vec![RESOURCE_MANAGER],
        };
        let bytes = p.to_bytes();
        let mut other = p.clone();
        other.resources[0] = MMI_RES;
        assert_ne!(bytes, other.to_bytes());
    }

    const MMI_RES: ResourceId = crate::resource::MMI;
}
