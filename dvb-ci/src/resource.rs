//! `resource_identifier()` — the 4-octet resource identity — ETSI EN 50221
//! §8.2.2, Table 15 (PDF p. 24) + §8.8.1, Table 57 (PDF p. 54).
//!
//! The two MSBs of the first octet are `resource_id_type`. Type 0/1/2 indicate a
//! public resource laid out as `resource_class` (14b) + `resource_type` (10b) +
//! `resource_version` (6b); type 3 indicates a private resource laid out as
//! `private_resource_definer` (10b) + `private_resource_identity` (20b). Both
//! layouts pack to exactly 32 bits, so we carry the identifier verbatim as a
//! `u32` and expose typed views over it — no information is lost on round-trip.

use crate::error::{Error, Result};
use dvb_common::{Parse, Serialize};

/// A `resource_identifier()` — 4 octets, carried verbatim as a big-endian `u32`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[cfg_attr(feature = "serde", serde(transparent))]
pub struct ResourceId(pub u32);

impl ResourceId {
    /// Wire length of a `resource_identifier()` in bytes.
    pub const LEN: usize = 4;

    /// `resource_id_type` — the two MSBs of the first octet (0/1/2 = public,
    /// 3 = private).
    #[must_use]
    pub const fn id_type(self) -> u8 {
        (self.0 >> 30) as u8
    }

    /// True if this is a private resource (`resource_id_type == 3`).
    #[must_use]
    pub const fn is_private(self) -> bool {
        self.id_type() == 3
    }

    /// `resource_class` (14 bits) for a public resource — meaningless if
    /// [`is_private`](Self::is_private).
    #[must_use]
    pub const fn resource_class(self) -> u16 {
        ((self.0 >> 16) & 0x3FFF) as u16
    }

    /// `resource_type` (10 bits) for a public resource.
    #[must_use]
    pub const fn resource_type(self) -> u16 {
        ((self.0 >> 6) & 0x03FF) as u16
    }

    /// `resource_version` (6 bits) for a public resource.
    #[must_use]
    pub const fn resource_version(self) -> u8 {
        (self.0 & 0x3F) as u8
    }

    /// Diagnostic name for the well-known public resources (Table 57), matched
    /// on the full identifier; `"unknown"` otherwise.
    #[must_use]
    pub fn name(self) -> &'static str {
        match self {
            RESOURCE_MANAGER => "resource_manager",
            APPLICATION_INFORMATION => "application_information",
            CONDITIONAL_ACCESS_SUPPORT => "conditional_access_support",
            HOST_CONTROL => "host_control",
            DATE_TIME => "date_time",
            MMI => "mmi",
            _ => "unknown",
        }
    }
}

impl core::fmt::Display for ResourceId {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self.name() {
            "unknown" => write!(f, "resource_id(0x{:08X})", self.0),
            n => write!(f, "{n}(0x{:08X})", self.0),
        }
    }
}

impl<'a> Parse<'a> for ResourceId {
    type Error = Error;
    fn parse(bytes: &'a [u8]) -> Result<Self> {
        let chunk = bytes.first_chunk::<4>().ok_or(Error::BufferTooShort {
            need: 4,
            have: bytes.len(),
            what: "resource_identifier",
        })?;
        Ok(Self(u32::from_be_bytes(*chunk)))
    }
}

impl Serialize for ResourceId {
    type Error = Error;
    fn serialized_len(&self) -> usize {
        Self::LEN
    }
    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        if buf.len() < Self::LEN {
            return Err(Error::OutputBufferTooSmall {
                need: Self::LEN,
                have: buf.len(),
            });
        }
        buf[..Self::LEN].copy_from_slice(&self.0.to_be_bytes());
        Ok(Self::LEN)
    }
}

/// Resource Manager — `00010041` (Table 57).
pub const RESOURCE_MANAGER: ResourceId = ResourceId(0x0001_0041);
/// Application Information — `00020041`.
pub const APPLICATION_INFORMATION: ResourceId = ResourceId(0x0002_0041);
/// Conditional Access Support — `00030041`.
pub const CONDITIONAL_ACCESS_SUPPORT: ResourceId = ResourceId(0x0003_0041);
/// Host Control — `00200041`.
pub const HOST_CONTROL: ResourceId = ResourceId(0x0020_0041);
/// Date-Time — `00240041`.
pub const DATE_TIME: ResourceId = ResourceId(0x0024_0041);
/// MMI — `00400041`.
pub const MMI: ResourceId = ResourceId(0x0040_0041);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn packs_per_table_57() {
        // Resource Manager: class=1, type=1, version=1 -> 0x00010041.
        let rm = RESOURCE_MANAGER;
        assert_eq!(rm.id_type(), 0);
        assert!(!rm.is_private());
        assert_eq!(rm.resource_class(), 1);
        assert_eq!(rm.resource_type(), 1);
        assert_eq!(rm.resource_version(), 1);
        assert_eq!(rm.name(), "resource_manager");
    }

    #[test]
    fn ca_support_fields() {
        assert_eq!(CONDITIONAL_ACCESS_SUPPORT.resource_class(), 3);
        assert_eq!(
            CONDITIONAL_ACCESS_SUPPORT.name(),
            "conditional_access_support"
        );
    }

    #[test]
    fn private_resource_type() {
        let p = ResourceId(0xC000_0000);
        assert!(p.is_private());
        assert_eq!(p.id_type(), 3);
        assert_eq!(p.name(), "unknown");
    }

    #[test]
    fn round_trip() {
        let r = DATE_TIME;
        let bytes = r.to_bytes();
        assert_eq!(bytes, [0x00, 0x24, 0x00, 0x41]);
        assert_eq!(ResourceId::parse(&bytes).unwrap(), r);
    }

    #[test]
    fn mutating_changes_bytes() {
        let mut bytes = MMI.to_bytes();
        let a = ResourceId::parse(&bytes).unwrap();
        bytes[0] ^= 0xFF;
        let b = ResourceId::parse(&bytes).unwrap();
        assert_ne!(a, b);
    }
}
