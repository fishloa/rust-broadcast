//! Application Descriptor — ETSI TS 102 809 §5.3.5.3, Table 20
//! (AIT tag 0x00).
//!
//! Carried in the AIT per-application descriptor loop. Lists the
//! application's profile/version triplets, a visibility/service-bound flags
//! byte, a priority, and a list of transport protocol labels.

use crate::descriptors::descriptor_body;
use crate::error::{Error, Result};
use alloc::vec::Vec;
use dvb_common::{Parse, Serialize};

/// Descriptor tag for application_descriptor (AIT namespace).
pub const TAG: u8 = 0x00;
const HEADER_LEN: usize = 2;
const PROFILE_ENTRY_LEN: usize = 5;
const FLAGS_LEN: usize = 1;
const PRIORITY_LEN: usize = 1;

/// 2-bit visibility field — ETSI TS 102 809 §5.2.6.1 Table 5.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub enum Visibility {
    /// 0 — not visible to users or applications.
    NotVisibleAll,
    /// 1 — not visible to users, visible to applications.
    NotVisibleUsers,
    /// 2 — reserved_future_use.
    ReservedFutureUse,
    /// 3 — visible to users and applications.
    VisibleAll,
    /// Catch-all for any other 2-bit value (should not occur).
    Other(u8),
}

impl Visibility {
    /// Decode from the 2-bit wire value.
    #[must_use]
    pub fn from_u8(v: u8) -> Self {
        match v & 0x03 {
            0 => Self::NotVisibleAll,
            1 => Self::NotVisibleUsers,
            2 => Self::ReservedFutureUse,
            3 => Self::VisibleAll,
            other => Self::Other(other),
        }
    }

    /// Encode to the 2-bit wire value.
    #[must_use]
    pub fn to_u8(self) -> u8 {
        match self {
            Self::NotVisibleAll => 0,
            Self::NotVisibleUsers => 1,
            Self::ReservedFutureUse => 2,
            Self::VisibleAll => 3,
            Self::Other(v) => v & 0x03,
        }
    }

    /// Spec name.
    #[must_use]
    pub fn name(self) -> &'static str {
        match self {
            Self::NotVisibleAll => "NOT_VISIBLE_ALL",
            Self::NotVisibleUsers => "NOT_VISIBLE_USERS",
            Self::ReservedFutureUse => "reserved_future_use",
            Self::VisibleAll => "VISIBLE_ALL",
            Self::Other(_) => "reserved",
        }
    }
}
dvb_common::impl_spec_display!(Visibility, Other);

/// One application profile entry — Table 4.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct ApplicationProfile {
    /// 16-bit application profile identifier.
    pub profile: u16,
    /// Major version.
    pub version_major: u8,
    /// Minor version.
    pub version_minor: u8,
    /// Micro version.
    pub version_micro: u8,
}

/// Application Descriptor (AIT tag 0x00).
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct ApplicationDescriptor {
    /// Profile/version entries.
    pub profiles: Vec<ApplicationProfile>,
    /// 1-bit service_bound_flag.
    pub service_bound_flag: bool,
    /// 2-bit visibility (Table 5).
    pub visibility: Visibility,
    /// Application priority.
    pub application_priority: u8,
    /// Transport protocol labels (one per transport protocol descriptor).
    pub transport_protocol_labels: Vec<u8>,
}

impl<'a> Parse<'a> for ApplicationDescriptor {
    type Error = crate::error::Error;
    fn parse(bytes: &'a [u8]) -> Result<Self> {
        let body = descriptor_body(
            bytes,
            TAG,
            "ApplicationDescriptor",
            "unexpected tag for application_descriptor",
        )?;
        if body.is_empty() {
            return Err(Error::InvalidDescriptor {
                tag: TAG,
                reason: "application_descriptor body is empty",
            });
        }
        let profiles_length = body[0] as usize;
        let profiles_end = 1 + profiles_length;
        if profiles_end > body.len() {
            return Err(Error::InvalidDescriptor {
                tag: TAG,
                reason: "application_profiles_length runs past descriptor end",
            });
        }
        if profiles_length % PROFILE_ENTRY_LEN != 0 {
            return Err(Error::InvalidDescriptor {
                tag: TAG,
                reason: "application_profiles_length not a multiple of 5",
            });
        }
        let mut profiles = Vec::with_capacity(profiles_length / PROFILE_ENTRY_LEN);
        let mut pos = 1;
        while pos < profiles_end {
            let profile = u16::from_be_bytes([body[pos], body[pos + 1]]);
            let version_major = body[pos + 2];
            let version_minor = body[pos + 3];
            let version_micro = body[pos + 4];
            profiles.push(ApplicationProfile {
                profile,
                version_major,
                version_minor,
                version_micro,
            });
            pos += PROFILE_ENTRY_LEN;
        }
        if profiles_end + FLAGS_LEN + PRIORITY_LEN > body.len() {
            return Err(Error::InvalidDescriptor {
                tag: TAG,
                reason: "flags/priority bytes missing after profiles",
            });
        }
        let flags_byte = body[profiles_end];
        let service_bound_flag = (flags_byte & 0x80) != 0;
        let visibility = Visibility::from_u8((flags_byte >> 5) & 0x03);
        let application_priority = body[profiles_end + FLAGS_LEN];
        let labels_start = profiles_end + FLAGS_LEN + PRIORITY_LEN;
        let transport_protocol_labels = body[labels_start..].to_vec();
        Ok(Self {
            profiles,
            service_bound_flag,
            visibility,
            application_priority,
            transport_protocol_labels,
        })
    }
}

impl Serialize for ApplicationDescriptor {
    type Error = crate::error::Error;
    fn serialized_len(&self) -> usize {
        HEADER_LEN
            + 1
            + self.profiles.len() * PROFILE_ENTRY_LEN
            + FLAGS_LEN
            + PRIORITY_LEN
            + self.transport_protocol_labels.len()
    }

    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        let profiles_length = self.profiles.len() * PROFILE_ENTRY_LEN;
        if profiles_length > u8::MAX as usize {
            return Err(Error::InvalidDescriptor {
                tag: TAG,
                reason: "application_profiles_length exceeds 255 bytes",
            });
        }
        let len = self.serialized_len();
        let body_len = len - HEADER_LEN;
        if body_len > u8::MAX as usize {
            return Err(Error::InvalidDescriptor {
                tag: TAG,
                reason: "application_descriptor body exceeds 255 bytes",
            });
        }
        if buf.len() < len {
            return Err(Error::OutputBufferTooSmall {
                need: len,
                have: buf.len(),
            });
        }
        buf[0] = TAG;
        buf[1] = body_len as u8;
        buf[2] = profiles_length as u8;
        let mut pos = 3;
        for p in &self.profiles {
            buf[pos..pos + 2].copy_from_slice(&p.profile.to_be_bytes());
            buf[pos + 2] = p.version_major;
            buf[pos + 3] = p.version_minor;
            buf[pos + 4] = p.version_micro;
            pos += PROFILE_ENTRY_LEN;
        }
        let flags_byte = (u8::from(self.service_bound_flag) << 7)
            | ((self.visibility.to_u8() & 0x03) << 5)
            | 0x1F;
        buf[pos] = flags_byte;
        buf[pos + 1] = self.application_priority;
        pos += FLAGS_LEN + PRIORITY_LEN;
        for (i, &label) in self.transport_protocol_labels.iter().enumerate() {
            buf[pos + i] = label;
        }
        Ok(len)
    }
}

impl<'a> crate::traits::DescriptorDef<'a> for ApplicationDescriptor {
    const TAG: u8 = TAG;
    const NAME: &'static str = "APPLICATION";
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Body layout: profiles_length(1) + 1 profile(5) + flags(1) + priority(1) + 2 labels(2) = 10.
    fn build_single_profile_two_labels() -> [u8; 12] {
        [
            TAG, 10, // header
            5,  // profiles_length = 1 entry × 5
            0x00, 0x01, // profile = 1
            2, 3, 4,    // major=2, minor=3, micro=4
            0x9F, // service_bound=1, visibility=NOT_VISIBLE_ALL(0), rfu=11111
            0x0A, // priority=10
            0x01, 0x02, // two transport labels
        ]
    }

    #[test]
    fn parse_single_profile_with_labels() {
        let bytes = build_single_profile_two_labels();
        let d = ApplicationDescriptor::parse(&bytes).unwrap();
        assert_eq!(d.profiles.len(), 1);
        assert_eq!(d.profiles[0].profile, 1);
        assert_eq!(d.profiles[0].version_major, 2);
        assert_eq!(d.profiles[0].version_minor, 3);
        assert_eq!(d.profiles[0].version_micro, 4);
        assert!(d.service_bound_flag);
        assert_eq!(d.visibility, Visibility::NotVisibleAll);
        assert_eq!(d.application_priority, 0x0A);
        assert_eq!(d.transport_protocol_labels, [0x01, 0x02]);
    }

    #[test]
    fn parse_visible_all() {
        // Body: profiles_length(1) + 1 profile(5) + flags(1) + priority(1) + 1 label(1) = 9.
        let bytes = [
            TAG, 9, 5, 0x00, 0x10, 1, 0, 0,
            0x6F, // service_bound=0, visibility=VISIBLE_ALL(3), rfu=11111
            0xFF, // priority
            0x42,
        ];
        let d = ApplicationDescriptor::parse(&bytes).unwrap();
        assert!(!d.service_bound_flag);
        assert_eq!(d.visibility, Visibility::VisibleAll);
    }

    #[test]
    fn parse_rejects_short_body() {
        let err = ApplicationDescriptor::parse(&[TAG]).unwrap_err();
        assert!(matches!(err, Error::BufferTooShort { .. }));
    }

    #[test]
    fn serialize_round_trip() {
        let d = ApplicationDescriptor {
            profiles: alloc::vec![ApplicationProfile {
                profile: 0x0010,
                version_major: 1,
                version_minor: 2,
                version_micro: 3,
            }],
            service_bound_flag: true,
            visibility: Visibility::VisibleAll,
            application_priority: 5,
            transport_protocol_labels: alloc::vec![0x01],
        };
        let mut buf = vec![0u8; d.serialized_len()];
        d.serialize_into(&mut buf).unwrap();
        let re = ApplicationDescriptor::parse(&buf).unwrap();
        assert_eq!(d, re);
        assert_eq!(buf[0], TAG);
    }

    #[test]
    fn serialize_byte_identical() {
        let bytes = build_single_profile_two_labels();
        let d = ApplicationDescriptor::parse(&bytes).unwrap();
        let mut buf = vec![0u8; d.serialized_len()];
        d.serialize_into(&mut buf).unwrap();
        assert_eq!(buf.as_slice(), &bytes[..]);
    }

    #[test]
    fn visibility_round_trip() {
        for v in [
            Visibility::NotVisibleAll,
            Visibility::NotVisibleUsers,
            Visibility::ReservedFutureUse,
            Visibility::VisibleAll,
        ] {
            assert_eq!(Visibility::from_u8(v.to_u8()), v);
        }
    }
}
