//! Transport profile descriptor — Rec. ITU-T H.222.0 (08/2023) | ISO/IEC
//! 13818-1:2023 §2.6.93–2.6.94, Tables 2-111/2-112 (tag 0x37); syntax in
//! `docs/descriptors/iso_13818_1/33-37-mvc-operation-point-stereoscopic-and-transport-profile-descriptors.md`.
//!
//! ```text
//! transport_profile(8) { private_data(8) }*
//! ```

use crate::error::{Error, Result};
use crate::h222_bits::impl_h222_descriptor;
use alloc::vec::Vec;

/// Descriptor tag of `Transport_profile_descriptor` (Table 2-45: 55).
pub const TAG: u8 = 0x37;

/// `transport_profile` (Table 2-112).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub enum TransportProfile {
    /// 0x00 — unspecified.
    Unspecified,
    /// 0x01 — complete profile.
    Complete,
    /// 0x02 — adaptive profile.
    Adaptive,
    /// 0x03..=0x0E — reserved (value carried).
    Reserved(u8),
    /// 0x0F..=0xFF — user private (value carried).
    UserPrivate(u8),
}

impl TransportProfile {
    /// Decode the 8-bit field (lossless).
    #[must_use]
    pub const fn from_u8(v: u8) -> Self {
        match v {
            0x00 => Self::Unspecified,
            0x01 => Self::Complete,
            0x02 => Self::Adaptive,
            0x03..=0x0E => Self::Reserved(v),
            other => Self::UserPrivate(other),
        }
    }

    /// The wire value.
    #[must_use]
    pub const fn to_u8(self) -> u8 {
        match self {
            Self::Unspecified => 0x00,
            Self::Complete => 0x01,
            Self::Adaptive => 0x02,
            Self::Reserved(v) | Self::UserPrivate(v) => v,
        }
    }

    /// Spec label (Table 2-112).
    #[must_use]
    pub fn name(&self) -> &'static str {
        match self {
            Self::Unspecified => "unspecified",
            Self::Complete => "complete profile",
            Self::Adaptive => "adaptive profile",
            Self::Reserved(_) => "reserved",
            Self::UserPrivate(_) => "user_private",
        }
    }
}
broadcast_common::impl_spec_display!(TransportProfile, Reserved, UserPrivate);

/// `Transport_profile_descriptor()` — Table 2-111.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[cfg_attr(feature = "yoke", derive(yoke::Yokeable))]
pub struct TransportProfileDescriptor {
    /// `transport_profile` (Table 2-112).
    pub transport_profile: TransportProfile,
    /// `private_data` bytes (private by definition: user-private profiles may
    /// identify themselves with a registration descriptor).
    pub private_data: Vec<u8>,
}

impl TransportProfileDescriptor {
    /// Build a descriptor with no private data.
    #[must_use]
    pub fn new(transport_profile: TransportProfile) -> Self {
        Self {
            transport_profile,
            private_data: Vec::new(),
        }
    }

    fn parse_body(body: &[u8]) -> Result<Self> {
        let (&profile, rest) = body.split_first().ok_or(Error::BufferTooShort {
            need: 1,
            have: 0,
            what: "TransportProfileDescriptor",
        })?;
        Ok(Self {
            transport_profile: TransportProfile::from_u8(profile),
            private_data: rest.to_vec(),
        })
    }

    fn write_body(&self) -> Result<Vec<u8>> {
        let mut v = Vec::with_capacity(1 + self.private_data.len());
        v.push(self.transport_profile.to_u8());
        v.extend_from_slice(&self.private_data);
        Ok(v)
    }
}

impl_h222_descriptor!(
    TransportProfileDescriptor,
    TAG,
    "TransportProfileDescriptor",
    "TRANSPORT_PROFILE"
);

#[cfg(test)]
mod tests {
    use super::*;
    use broadcast_common::{Parse, Serialize};

    #[test]
    fn profiles_and_private_data() {
        for (b, p) in [
            (0u8, TransportProfile::Unspecified),
            (1, TransportProfile::Complete),
            (2, TransportProfile::Adaptive),
            (0x0E, TransportProfile::Reserved(0x0E)),
            (0x0F, TransportProfile::UserPrivate(0x0F)),
        ] {
            let bytes = [TAG, 3, b, 0xDE, 0xAD];
            let d = TransportProfileDescriptor::parse(&bytes).unwrap();
            assert_eq!(d.transport_profile, p);
            assert_eq!(d.private_data, [0xDE, 0xAD]);
            let mut out = vec![0u8; d.serialized_len()];
            d.serialize_into(&mut out).unwrap();
            assert_eq!(out, bytes);
        }
        assert!(TransportProfileDescriptor::parse(&[TAG, 0]).is_err());
        let mut d = TransportProfileDescriptor::new(TransportProfile::Complete);
        d.private_data = vec![0; 255];
        assert!(d.serialize_into(&mut [0u8; 300]).is_err());
    }
}
