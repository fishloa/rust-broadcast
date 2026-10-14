//! FTA Content Management Descriptor — ETSI EN 300 468 §6.2.18.1 (tag 0x7E, Table 57, PDF p. 82).
//!
//! Carried in NIT/BAT/SDT/EIT. Fixed 1-byte body packing five fields
//! (Table 57, MSB→LSB): `user_defined` (1), `reserved_future_use` (3),
//! `do_not_scramble` (1), `control_remote_access_over_internet` (2),
//! `do_not_apply_revocation` (1).
//! `control_remote_access_over_internet` coding is Table 58.

use super::descriptor_body;
use crate::error::{Error, Result};
use broadcast_common::{Parse, Serialize};

/// Descriptor tag for FTA_content_management_descriptor.
pub const TAG: u8 = 0x7E;
/// Length of the header (tag byte + length byte).
pub const HEADER_LEN: usize = 2;
/// Fixed body length: one packed flag byte.
pub const BODY_LEN: usize = 1;

const USER_DEFINED_MASK: u8 = 0b1000_0000;
const RESERVED_MASK: u8 = 0b0111_0000;
const RESERVED_SHIFT: u8 = 4;
/// Spec default of `reserved_future_use`(3): all ones (EN 300 468 V1.12.1-V1.20.1
/// Table 57; audit `dvb-si/docs/reserved-bits-edition-audit.md` §4 row 3 / §6.1:
/// `user_defined`(1) was carved out of the old `reserved_future_use`(4) in V1.12.1,
/// the remaining 3 bits stay reserved = 1).
pub const RESERVED_FUTURE_USE_DEFAULT: u8 = 0b111;
const DO_NOT_SCRAMBLE_MASK: u8 = 0b0000_1000;
const CONTROL_REMOTE_ACCESS_MASK: u8 = 0b0000_0110;
const CONTROL_REMOTE_ACCESS_SHIFT: u8 = 1;
const DO_NOT_APPLY_REVOCATION_MASK: u8 = 0b0000_0001;
/// Max value of the 2-bit control_remote_access_over_internet field.
pub const CONTROL_REMOTE_ACCESS_MAX: u8 = 0b11;

/// Control remote access over internet — ETSI EN 300 468 Table 58.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub enum ControlRemoteAccess {
    /// 0b00 — redistribution over the Internet is enabled.
    Enabled,
    /// 0b01 — redistribution over the Internet is enabled but only within a
    /// managed domain.
    EnabledManagedDomain,
    /// 0b10 — redistribution over the Internet is enabled but only within a
    /// managed domain and after a certain short period of time.
    EnabledManagedDomainTimeLimited,
    /// 0b11 — redistribution over the Internet is not allowed.
    NotAllowed,
    /// Reserved/unallocated wire value, preserved verbatim for round-trip.
    Reserved(u8),
}

impl ControlRemoteAccess {
    #[must_use]
    /// Creates a value from a wire byte, preserving every possible
    /// byte value for lossless round-trip.
    pub fn from_u8(v: u8) -> Self {
        match v {
            0b00 => Self::Enabled,
            0b01 => Self::EnabledManagedDomain,
            0b10 => Self::EnabledManagedDomainTimeLimited,
            0b11 => Self::NotAllowed,
            v => Self::Reserved(v),
        }
    }

    #[must_use]
    /// Returns the wire byte for this value.
    pub const fn to_u8(self) -> u8 {
        match self {
            Self::Enabled => 0b00,
            Self::EnabledManagedDomain => 0b01,
            Self::EnabledManagedDomainTimeLimited => 0b10,
            Self::NotAllowed => 0b11,
            Self::Reserved(v) => v,
        }
    }

    #[must_use]
    /// Returns a human-readable spec name for this value.
    pub fn name(self) -> &'static str {
        match self {
            Self::Enabled => "redistribution enabled",
            Self::EnabledManagedDomain => "redistribution enabled (managed domain only)",
            Self::EnabledManagedDomainTimeLimited => {
                "redistribution enabled (managed domain, time-limited)"
            }
            Self::NotAllowed => "redistribution not allowed",
            Self::Reserved(_) => "reserved",
        }
    }
}
broadcast_common::impl_spec_display!(ControlRemoteAccess, Reserved);

/// FTA Content Management Descriptor.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct FtaContentManagementDescriptor {
    /// `reserved_future_use`(3) after `user_defined` (3 bits, right-aligned).
    /// Captured on parse and re-emitted verbatim;
    /// [`FtaContentManagementDescriptor::new`] defaults to
    /// [`RESERVED_FUTURE_USE_DEFAULT`]. (A pre-V1.12.1 all-ones sender reads as
    /// `user_defined = 1`; that bit is typed, the round-trip stays byte-exact.)
    pub reserved_future_use: u8,
    /// 1-bit user_defined flag.
    pub user_defined: bool,
    /// 1-bit do_not_scramble flag.
    pub do_not_scramble: bool,
    /// 2-bit control_remote_access_over_internet field (Table 58).
    pub control_remote_access_over_internet: ControlRemoteAccess,
    /// 1-bit do_not_apply_revocation flag.
    pub do_not_apply_revocation: bool,
}

impl FtaContentManagementDescriptor {
    /// Build a descriptor with the spec-default reserved bits (all ones).
    #[must_use]
    pub const fn new(
        user_defined: bool,
        do_not_scramble: bool,
        control_remote_access_over_internet: ControlRemoteAccess,
        do_not_apply_revocation: bool,
    ) -> Self {
        Self {
            reserved_future_use: RESERVED_FUTURE_USE_DEFAULT,
            user_defined,
            do_not_scramble,
            control_remote_access_over_internet,
            do_not_apply_revocation,
        }
    }
}

impl<'a> Parse<'a> for FtaContentManagementDescriptor {
    type Error = crate::error::Error;
    fn parse(bytes: &'a [u8]) -> Result<Self> {
        let body = descriptor_body(
            bytes,
            TAG,
            "FtaContentManagementDescriptor",
            "unexpected tag for FTA_content_management_descriptor",
        )?;
        if body.len() != BODY_LEN {
            return Err(Error::InvalidDescriptor {
                tag: TAG,
                reason: "FTA_content_management_descriptor length must be exactly 1",
            });
        }
        let flags = body[0];
        Ok(Self {
            reserved_future_use: (flags & RESERVED_MASK) >> RESERVED_SHIFT,
            user_defined: flags & USER_DEFINED_MASK != 0,
            do_not_scramble: flags & DO_NOT_SCRAMBLE_MASK != 0,
            control_remote_access_over_internet: ControlRemoteAccess::from_u8(
                (flags & CONTROL_REMOTE_ACCESS_MASK) >> CONTROL_REMOTE_ACCESS_SHIFT,
            ),
            do_not_apply_revocation: flags & DO_NOT_APPLY_REVOCATION_MASK != 0,
        })
    }
}

impl Serialize for FtaContentManagementDescriptor {
    type Error = crate::error::Error;
    fn serialized_len(&self) -> usize {
        HEADER_LEN + BODY_LEN
    }

    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        if self.control_remote_access_over_internet.to_u8() > CONTROL_REMOTE_ACCESS_MAX {
            return Err(Error::InvalidDescriptor {
                tag: TAG,
                reason: "control_remote_access_over_internet exceeds 2 bits",
            });
        }
        let len = self.serialized_len();
        if buf.len() < len {
            return Err(Error::OutputBufferTooSmall {
                need: len,
                have: buf.len(),
            });
        }
        // reserved_future_use(3) preserved from parse (default 111).
        let mut flags = broadcast_common::len::fit_mask(
            self.reserved_future_use,
            RESERVED_MASK >> RESERVED_SHIFT,
            "reserved_future_use",
        )? << RESERVED_SHIFT;
        if self.user_defined {
            flags |= USER_DEFINED_MASK;
        }
        if self.do_not_scramble {
            flags |= DO_NOT_SCRAMBLE_MASK;
        }
        flags |= broadcast_common::len::fit_mask(
            self.control_remote_access_over_internet.to_u8(),
            CONTROL_REMOTE_ACCESS_MASK >> CONTROL_REMOTE_ACCESS_SHIFT,
            "control_remote_access_over_internet",
        )? << CONTROL_REMOTE_ACCESS_SHIFT;
        if self.do_not_apply_revocation {
            flags |= DO_NOT_APPLY_REVOCATION_MASK;
        }
        crate::descriptors::write_descriptor_header(buf, TAG, BODY_LEN)?;
        buf[HEADER_LEN] = flags;
        Ok(len)
    }
}
impl crate::traits::DescriptorDef<'_> for FtaContentManagementDescriptor {
    const TAG: u8 = TAG;
    const NAME: &'static str = "FTA_CONTENT_MANAGEMENT";
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_extracts_all_fields() {
        // user_defined=1, reserved=000, do_not_scramble=1, cra=10, revocation=1
        // = 1 000 1 10 1 = 0b1000_1101 = 0x8D
        let bytes = [TAG, 1, 0x8D];
        let d = FtaContentManagementDescriptor::parse(&bytes).unwrap();
        assert!(d.user_defined);
        assert!(d.do_not_scramble);
        assert_eq!(
            d.control_remote_access_over_internet,
            ControlRemoteAccess::EnabledManagedDomainTimeLimited
        );
        assert!(d.do_not_apply_revocation);
    }

    /// Every flag-byte value (all 256) re-serializes byte-identically.
    #[test]
    fn reserved_preserved_for_every_byte() {
        for b in 0..=0xFFu8 {
            let bytes = [TAG, 1, b];
            let d = FtaContentManagementDescriptor::parse(&bytes).unwrap();
            assert_eq!(d.reserved_future_use, (b >> 4) & 0x07);
            let mut buf = vec![0u8; d.serialized_len()];
            d.serialize_into(&mut buf).unwrap();
            assert_eq!(buf, bytes);
        }
    }

    #[test]
    fn default_is_spec_value() {
        let d =
            FtaContentManagementDescriptor::new(false, false, ControlRemoteAccess::Enabled, false);
        let mut buf = vec![0u8; d.serialized_len()];
        d.serialize_into(&mut buf).unwrap();
        assert_eq!(buf, [TAG, 1, 0x70]);
    }

    #[test]
    fn parse_captures_reserved_bits() {
        // reserved bits all set, everything else zero: 0 111 0 00 0 = 0x70.
        let bytes = [TAG, 1, 0x70];
        let d = FtaContentManagementDescriptor::parse(&bytes).unwrap();
        assert_eq!(d.reserved_future_use, 0b111);
        assert!(!d.user_defined);
        assert!(!d.do_not_scramble);
        assert_eq!(
            d.control_remote_access_over_internet,
            ControlRemoteAccess::Enabled
        );
        assert!(!d.do_not_apply_revocation);
    }

    #[test]
    fn parse_rejects_wrong_tag() {
        assert!(matches!(
            FtaContentManagementDescriptor::parse(&[0x7F, 1, 0]).unwrap_err(),
            Error::InvalidDescriptor { tag: 0x7F, .. }
        ));
    }

    #[test]
    fn parse_rejects_wrong_length() {
        assert!(matches!(
            FtaContentManagementDescriptor::parse(&[TAG, 2, 0, 0]).unwrap_err(),
            Error::InvalidDescriptor { tag: TAG, .. }
        ));
    }

    #[test]
    fn parse_rejects_short_body() {
        assert!(matches!(
            FtaContentManagementDescriptor::parse(&[TAG, 1]).unwrap_err(),
            Error::BufferTooShort { .. }
        ));
    }

    #[test]
    fn serialize_round_trip() {
        let d = FtaContentManagementDescriptor::new(
            true,
            true,
            ControlRemoteAccess::EnabledManagedDomainTimeLimited,
            true,
        );
        let mut buf = vec![0u8; d.serialized_len()];
        d.serialize_into(&mut buf).unwrap();
        // Reserved 3 bits emitted as 1s: 1 111 1 10 1 = 0xFD.
        assert_eq!(buf, [TAG, 1, 0xFD]);
        assert_eq!(FtaContentManagementDescriptor::parse(&buf).unwrap(), d);
    }

    #[test]
    fn serialize_rejects_too_small_buffer() {
        let d =
            FtaContentManagementDescriptor::new(false, false, ControlRemoteAccess::Enabled, false);
        let mut buf = vec![0u8; 2];
        assert!(matches!(
            d.serialize_into(&mut buf).unwrap_err(),
            Error::OutputBufferTooSmall { .. }
        ));
    }

    #[test]
    fn serialize_rejects_over_range_cra() {
        let d = FtaContentManagementDescriptor::new(
            false,
            false,
            ControlRemoteAccess::Reserved(0b100),
            false,
        );
        let mut buf = vec![0u8; d.serialized_len()];
        assert!(matches!(
            d.serialize_into(&mut buf).unwrap_err(),
            Error::InvalidDescriptor { tag: TAG, .. }
        ));
    }

    #[cfg(feature = "serde")]
    #[test]
    fn serde_round_trip() {
        let d = FtaContentManagementDescriptor::new(
            true,
            false,
            ControlRemoteAccess::EnabledManagedDomain,
            true,
        );
        let json = serde_json::to_string(&d).unwrap();
        // Serialize-only: assert the emitted JSON re-parses (serialize-stable).
        let _v: serde_json::Value = serde_json::from_str(&json).unwrap();
    }

    #[test]
    fn control_remote_access_full_range_round_trip() {
        for b in 0..=0xFF_u8 {
            let cra = ControlRemoteAccess::from_u8(b);
            assert_eq!(cra.to_u8(), b, "round-trip failed for byte 0x{b:02X}");
        }
    }
}
