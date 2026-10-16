//! Application Storage Descriptor — ETSI TS 102 809 V1.3.1 §5.3.10.1, Table 37
//! (AIT tag 0x10; same layout as §5.2.11.2 Table 14, `storage_property` values
//! Table 15). PDF p.51 / p.28, transcribed in
//! `docs/descriptors/ts_102_809/37-*.md` and `docs/enums/ts_102_809/15-*.md`,
//! verified against the rendered pages.
//!
//! Fixed 7-byte body: `storage_property`(8), `not_launchable_from_broadcast`(1),
//! `launchable_completely_from_cache`(1), `is_launchable_with_older_version`(1),
//! `reserved`(5), `reserved`(1), `version`(31), `priority`(8).

use crate::descriptors::{descriptor_body, write_descriptor_header};
use crate::error::{Error, Result};
use broadcast_common::{Parse, Serialize};

/// Descriptor tag for application_storage_descriptor (AIT namespace).
pub const TAG: u8 = 0x10;
const HEADER_LEN: usize = 2;
const BODY_LEN: usize = 7;

const NOT_LAUNCHABLE_FROM_BROADCAST: u8 = 0x80;
const LAUNCHABLE_COMPLETELY_FROM_CACHE: u8 = 0x40;
const IS_LAUNCHABLE_WITH_OLDER_VERSION: u8 = 0x20;
const RESERVED_FLAGS_MASK: u8 = 0x1F;
const RESERVED_BIT_AFTER_FLAGS: u8 = 0x80;
const VERSION_MASK: u32 = 0x7FFF_FFFF;

/// Spec default of the 5-bit `reserved` after the three flags (all ones,
/// TS 102 809 V1.3.1 NOTE: reserved bits are set to "1").
pub const RESERVED_FLAGS_DEFAULT: u8 = 0x1F;

/// `storage_property` (TS 102 809 §5.2.11.2 Table 15).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub enum StorageProperty {
    /// 0 — broadcast related.
    BroadcastRelated,
    /// 1 — stand alone.
    StandAlone,
    /// 2..=255 — reserved.
    Reserved(u8),
}

impl StorageProperty {
    /// Decode from the 8-bit wire value.
    #[must_use]
    pub const fn from_u8(v: u8) -> Self {
        match v {
            0 => Self::BroadcastRelated,
            1 => Self::StandAlone,
            other => Self::Reserved(other),
        }
    }

    /// Encode to the 8-bit wire value.
    #[must_use]
    pub const fn to_u8(self) -> u8 {
        match self {
            Self::BroadcastRelated => 0,
            Self::StandAlone => 1,
            Self::Reserved(v) => v,
        }
    }

    /// Spec label.
    #[must_use]
    pub fn name(&self) -> &'static str {
        match self {
            Self::BroadcastRelated => "broadcast related",
            Self::StandAlone => "stand alone",
            Self::Reserved(_) => "reserved",
        }
    }
}
broadcast_common::impl_spec_display!(StorageProperty, Reserved);

/// Application Storage Descriptor (AIT tag 0x10).
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct ApplicationStorageDescriptor {
    /// `storage_property` — broadcast related or stand-alone.
    pub storage_property: StorageProperty,
    /// `not_launchable_from_broadcast` (§5.2.11.1.3).
    pub not_launchable_from_broadcast: bool,
    /// `launchable_completely_from_cache` (§5.2.11.1.3).
    pub launchable_completely_from_cache: bool,
    /// `is_launchable_with_older_version` (§5.2.11.1.2).
    pub is_launchable_with_older_version: bool,
    /// `reserved`(5) after the flags (right-aligned). Parsed value is kept so
    /// re-serialization is byte-identical; default [`RESERVED_FLAGS_DEFAULT`].
    pub reserved_flags: u8,
    /// The 1-bit `reserved` before `version`. Parsed value is kept; default `true`.
    pub reserved_before_version: bool,
    /// `version`(31) — application version number (§5.2.11.1.2).
    pub version: u32,
    /// `priority` — storage priority, higher is more important (§5.2.11.1.4).
    pub priority: u8,
}

impl Default for ApplicationStorageDescriptor {
    /// Broadcast-related, all flags clear, spec-default reserved bits.
    fn default() -> Self {
        Self {
            storage_property: StorageProperty::BroadcastRelated,
            not_launchable_from_broadcast: false,
            launchable_completely_from_cache: false,
            is_launchable_with_older_version: false,
            reserved_flags: RESERVED_FLAGS_DEFAULT,
            reserved_before_version: true,
            version: 0,
            priority: 0,
        }
    }
}

impl<'a> Parse<'a> for ApplicationStorageDescriptor {
    type Error = crate::error::Error;
    fn parse(bytes: &'a [u8]) -> Result<Self> {
        let body = descriptor_body(
            bytes,
            TAG,
            "ApplicationStorageDescriptor",
            "unexpected tag for application_storage_descriptor",
        )?;
        if body.len() != BODY_LEN {
            return Err(Error::InvalidDescriptor {
                tag: TAG,
                reason: "application_storage_descriptor body must be 7 bytes",
            });
        }
        let flags = body[1];
        let ver = u32::from_be_bytes([body[2], body[3], body[4], body[5]]);
        Ok(Self {
            storage_property: StorageProperty::from_u8(body[0]),
            not_launchable_from_broadcast: flags & NOT_LAUNCHABLE_FROM_BROADCAST != 0,
            launchable_completely_from_cache: flags & LAUNCHABLE_COMPLETELY_FROM_CACHE != 0,
            is_launchable_with_older_version: flags & IS_LAUNCHABLE_WITH_OLDER_VERSION != 0,
            reserved_flags: flags & RESERVED_FLAGS_MASK,
            reserved_before_version: ver & !VERSION_MASK != 0,
            version: ver & VERSION_MASK,
            priority: body[6],
        })
    }
}

impl Serialize for ApplicationStorageDescriptor {
    type Error = crate::error::Error;
    fn serialized_len(&self) -> usize {
        HEADER_LEN + BODY_LEN
    }

    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        let len = self.serialized_len();
        let reserved =
            broadcast_common::len::fit_mask(self.reserved_flags, RESERVED_FLAGS_MASK, "reserved")?;
        let version = broadcast_common::len::fit_mask(self.version, VERSION_MASK, "version")?;
        if buf.len() < len {
            return Err(Error::OutputBufferTooSmall {
                need: len,
                have: buf.len(),
            });
        }
        write_descriptor_header(buf, TAG, BODY_LEN)?;
        buf[2] = self.storage_property.to_u8();
        buf[3] = (u8::from(self.not_launchable_from_broadcast) * NOT_LAUNCHABLE_FROM_BROADCAST)
            | (u8::from(self.launchable_completely_from_cache) * LAUNCHABLE_COMPLETELY_FROM_CACHE)
            | (u8::from(self.is_launchable_with_older_version) * IS_LAUNCHABLE_WITH_OLDER_VERSION)
            | reserved;
        let word = version
            | (u32::from(self.reserved_before_version)
                * (u32::from(RESERVED_BIT_AFTER_FLAGS) << 24));
        buf[4..8].copy_from_slice(&word.to_be_bytes());
        buf[8] = self.priority;
        Ok(len)
    }
}

impl crate::traits::DescriptorDef<'_> for ApplicationStorageDescriptor {
    const TAG: u8 = TAG;
    const NAME: &'static str = "APPLICATION_STORAGE";
}

#[cfg(test)]
mod tests {
    use super::*;

    // Table 37 layout: property=1 (stand alone), flags 0b101 + reserved 0x1F,
    // reserved bit 1 + version 0x0000_0102, priority 7.
    const BYTES: [u8; 9] = [TAG, 7, 0x01, 0xBF, 0x80, 0x00, 0x01, 0x02, 0x07];

    #[test]
    fn parse_fields() {
        let d = ApplicationStorageDescriptor::parse(&BYTES).unwrap();
        assert_eq!(d.storage_property, StorageProperty::StandAlone);
        assert!(d.not_launchable_from_broadcast);
        assert!(!d.launchable_completely_from_cache);
        assert!(d.is_launchable_with_older_version);
        assert_eq!(d.reserved_flags, 0x1F);
        assert!(d.reserved_before_version);
        assert_eq!(d.version, 0x102);
        assert_eq!(d.priority, 7);
    }

    #[test]
    fn round_trip_byte_identical_including_zero_reserved() {
        for bytes in [
            BYTES,
            [TAG, 7, 0x02, 0x00, 0x7F, 0xFF, 0xFF, 0xFF, 0xFF], // reserved all zero, max version
        ] {
            let d = ApplicationStorageDescriptor::parse(&bytes).unwrap();
            let mut out = vec![0u8; d.serialized_len()];
            d.serialize_into(&mut out).unwrap();
            assert_eq!(out, bytes);
            assert_eq!(ApplicationStorageDescriptor::parse(&out).unwrap(), d);
        }
    }

    #[test]
    fn default_reserved_is_spec_value() {
        let d = ApplicationStorageDescriptor::default();
        let mut out = vec![0u8; d.serialized_len()];
        d.serialize_into(&mut out).unwrap();
        assert_eq!(out, [TAG, 7, 0x00, 0x1F, 0x80, 0, 0, 0, 0]);
    }

    #[test]
    fn wrong_length_and_overflow_rejected() {
        assert!(ApplicationStorageDescriptor::parse(&[TAG, 6, 0, 0, 0, 0, 0, 0]).is_err());
        let d = ApplicationStorageDescriptor {
            version: 0x8000_0000,
            ..Default::default()
        };
        let mut out = [0u8; 9];
        assert!(d.serialize_into(&mut out).is_err());
        let d = ApplicationStorageDescriptor {
            reserved_flags: 0x20,
            ..Default::default()
        };
        assert!(d.serialize_into(&mut out).is_err());
    }

    #[test]
    fn storage_property_labels_round_trip() {
        for v in 0..=255u8 {
            assert_eq!(StorageProperty::from_u8(v).to_u8(), v);
        }
        assert_eq!(StorageProperty::Reserved(9).name(), "reserved");
    }
}
