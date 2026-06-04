//! FTA Content Management Descriptor — ETSI EN 300 468 §6.2.18.1 (tag 0x7E, Table 57, PDF p. 82).
//!
//! Carried in NIT/BAT/SDT/EIT. Fixed 1-byte body packing five fields
//! (Table 57, MSB→LSB): `user_defined` (1), `reserved_future_use` (3),
//! `do_not_scramble` (1), `control_remote_access_over_internet` (2),
//! `do_not_apply_revocation` (1).
//! `control_remote_access_over_internet` coding is Table 58.

use crate::error::{Error, Result};
use crate::traits::Descriptor;
use dvb_common::{Parse, Serialize};

/// Descriptor tag for FTA_content_management_descriptor.
pub const TAG: u8 = 0x7E;
/// Length of the header (tag byte + length byte).
pub const HEADER_LEN: usize = 2;
/// Fixed body length: one packed flag byte.
pub const BODY_LEN: usize = 1;

const USER_DEFINED_MASK: u8 = 0b1000_0000;
const RESERVED_MASK: u8 = 0b0111_0000;
const DO_NOT_SCRAMBLE_MASK: u8 = 0b0000_1000;
const CONTROL_REMOTE_ACCESS_MASK: u8 = 0b0000_0110;
const CONTROL_REMOTE_ACCESS_SHIFT: u8 = 1;
const DO_NOT_APPLY_REVOCATION_MASK: u8 = 0b0000_0001;
/// Max value of the 2-bit control_remote_access_over_internet field.
pub const CONTROL_REMOTE_ACCESS_MAX: u8 = 0b11;

/// FTA Content Management Descriptor.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct FtaContentManagementDescriptor {
    /// 1-bit user_defined flag.
    pub user_defined: bool,
    /// 1-bit do_not_scramble flag.
    pub do_not_scramble: bool,
    /// 2-bit control_remote_access_over_internet field (Table 58).
    pub control_remote_access_over_internet: u8,
    /// 1-bit do_not_apply_revocation flag.
    pub do_not_apply_revocation: bool,
}

impl<'a> Parse<'a> for FtaContentManagementDescriptor {
    type Error = crate::error::Error;
    fn parse(bytes: &'a [u8]) -> Result<Self> {
        if bytes.len() < HEADER_LEN {
            return Err(Error::BufferTooShort {
                need: HEADER_LEN,
                have: bytes.len(),
                what: "FtaContentManagementDescriptor header",
            });
        }
        if bytes[0] != TAG {
            return Err(Error::InvalidDescriptor {
                tag: bytes[0],
                reason: "unexpected tag for FTA_content_management_descriptor",
            });
        }
        let length = bytes[1] as usize;
        if length != BODY_LEN {
            return Err(Error::InvalidDescriptor {
                tag: TAG,
                reason: "FTA_content_management_descriptor length must be exactly 1",
            });
        }
        let end = HEADER_LEN + length;
        if bytes.len() < end {
            return Err(Error::BufferTooShort {
                need: end,
                have: bytes.len(),
                what: "FtaContentManagementDescriptor body",
            });
        }
        let flags = bytes[HEADER_LEN];
        // reserved_future_use (3 bits) ignored on parse (§5.1).
        Ok(Self {
            user_defined: flags & USER_DEFINED_MASK != 0,
            do_not_scramble: flags & DO_NOT_SCRAMBLE_MASK != 0,
            control_remote_access_over_internet: (flags & CONTROL_REMOTE_ACCESS_MASK)
                >> CONTROL_REMOTE_ACCESS_SHIFT,
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
        if self.control_remote_access_over_internet > CONTROL_REMOTE_ACCESS_MAX {
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
        // reserved_future_use 3 bits emitted as 1s (§5.1).
        let mut flags = RESERVED_MASK;
        if self.user_defined {
            flags |= USER_DEFINED_MASK;
        }
        if self.do_not_scramble {
            flags |= DO_NOT_SCRAMBLE_MASK;
        }
        flags |= (self.control_remote_access_over_internet << CONTROL_REMOTE_ACCESS_SHIFT)
            & CONTROL_REMOTE_ACCESS_MASK;
        if self.do_not_apply_revocation {
            flags |= DO_NOT_APPLY_REVOCATION_MASK;
        }
        buf[0] = TAG;
        buf[1] = BODY_LEN as u8;
        buf[HEADER_LEN] = flags;
        Ok(len)
    }
}

impl<'a> Descriptor<'a> for FtaContentManagementDescriptor {
    const TAG: u8 = TAG;
    fn descriptor_length(&self) -> u8 {
        BODY_LEN as u8
    }
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
        assert_eq!(d.control_remote_access_over_internet, 0b10);
        assert!(d.do_not_apply_revocation);
    }

    #[test]
    fn parse_ignores_reserved_bits() {
        // reserved bits all set, everything else zero: 0 111 0 00 0 = 0x70.
        let bytes = [TAG, 1, 0x70];
        let d = FtaContentManagementDescriptor::parse(&bytes).unwrap();
        assert!(!d.user_defined);
        assert!(!d.do_not_scramble);
        assert_eq!(d.control_remote_access_over_internet, 0);
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
        let d = FtaContentManagementDescriptor {
            user_defined: true,
            do_not_scramble: true,
            control_remote_access_over_internet: 0b10,
            do_not_apply_revocation: true,
        };
        let mut buf = vec![0u8; d.serialized_len()];
        d.serialize_into(&mut buf).unwrap();
        // Reserved 3 bits emitted as 1s: 1 111 1 10 1 = 0xFD.
        assert_eq!(buf, [TAG, 1, 0xFD]);
        assert_eq!(FtaContentManagementDescriptor::parse(&buf).unwrap(), d);
    }

    #[test]
    fn serialize_rejects_too_small_buffer() {
        let d = FtaContentManagementDescriptor {
            user_defined: false,
            do_not_scramble: false,
            control_remote_access_over_internet: 0,
            do_not_apply_revocation: false,
        };
        let mut buf = vec![0u8; 2];
        assert!(matches!(
            d.serialize_into(&mut buf).unwrap_err(),
            Error::OutputBufferTooSmall { .. }
        ));
    }

    #[test]
    fn serialize_rejects_over_range_cra() {
        let d = FtaContentManagementDescriptor {
            user_defined: false,
            do_not_scramble: false,
            control_remote_access_over_internet: 0b100, // 3 bits
            do_not_apply_revocation: false,
        };
        let mut buf = vec![0u8; d.serialized_len()];
        assert!(matches!(
            d.serialize_into(&mut buf).unwrap_err(),
            Error::InvalidDescriptor { tag: TAG, .. }
        ));
    }

    #[cfg(feature = "serde")]
    #[test]
    fn serde_round_trip() {
        let d = FtaContentManagementDescriptor {
            user_defined: true,
            do_not_scramble: false,
            control_remote_access_over_internet: 0b01,
            do_not_apply_revocation: true,
        };
        let json = serde_json::to_string(&d).unwrap();
        let back: FtaContentManagementDescriptor = serde_json::from_str(&json).unwrap();
        assert_eq!(back, d);
    }
}
