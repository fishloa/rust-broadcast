//! Application Information v2 objects — ETSI TS 101 699 V1.1.1 §5, Table 11
//! (PDF p. 21). See `docs/ci_plus/application-info-v2.md`.
//!
//! Resource ID `0x00020042`. The object layouts (Application Info Enquiry,
//! Application Info, Enter Menu) are **unchanged** from EN 50221 §8.4 — v2 only
//! extends the `application_type` value set (§5.1.1) and adds the
//! "unrecognized type → Unclassified" rule (§5.1.2). This module re-defines the
//! objects with the v2 [`ApplicationTypeV2`] enum so the v2 resource owns its set.
//!
//! - `application_info_enq` (`9F 80 20`, EN 50221 Table 20) — header-only.
//! - `application_info` (`9F 80 21`, EN 50221 Table 21) — type/manufacturer + menu.
//! - `enter_menu` (`9F 80 22`, EN 50221 Table 22) — header-only.

use crate::error::{Error, Result};
use crate::objects;
use crate::tag::ApduTag;
use dvb_common::{Parse, Serialize};

/// Resource-scoped `apdu_tag`s for Application Information v2 (Table 87).
pub mod tag {
    use crate::tag::ApduTag;
    /// `Tapplication_info_enq` = `9F 80 20`.
    pub const APPLICATION_INFO_ENQ: ApduTag = ApduTag::from_bytes(0x9F, 0x80, 0x20);
    /// `Tapplication_info` = `9F 80 21`.
    pub const APPLICATION_INFO: ApduTag = ApduTag::from_bytes(0x9F, 0x80, 0x21);
    /// `Tenter_menu` = `9F 80 22`.
    pub const ENTER_MENU: ApduTag = ApduTag::from_bytes(0x9F, 0x80, 0x22);
}

/// `application_type` — the v2 value set (TS 101 699 Table 11).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub enum ApplicationTypeV2 {
    /// `01` — Conditional Access (inherited from EN 50221 §8.4).
    ConditionalAccess,
    /// `02` — Electronic Programme Guide (inherited from EN 50221 §8.4).
    ElectronicProgrammeGuide,
    /// `03` — Software upgrade.
    SoftwareUpgrade,
    /// `04` — Network interface.
    NetworkInterface,
    /// `05` — Accessibility aids.
    AccessibilityAids,
    /// `06` — Unclassified.
    Unclassified,
    /// Any other value (reserved). §5.1.2: a v2 host treats an unrecognized type
    /// as Unclassified, but the wire value is retained for lossless round-trip.
    Reserved(u8),
}

impl ApplicationTypeV2 {
    /// Decode an `application_type` byte.
    #[must_use]
    pub fn from_u8(v: u8) -> Self {
        match v {
            0x01 => Self::ConditionalAccess,
            0x02 => Self::ElectronicProgrammeGuide,
            0x03 => Self::SoftwareUpgrade,
            0x04 => Self::NetworkInterface,
            0x05 => Self::AccessibilityAids,
            0x06 => Self::Unclassified,
            other => Self::Reserved(other),
        }
    }
    /// Wire byte.
    #[must_use]
    pub fn to_u8(self) -> u8 {
        match self {
            Self::ConditionalAccess => 0x01,
            Self::ElectronicProgrammeGuide => 0x02,
            Self::SoftwareUpgrade => 0x03,
            Self::NetworkInterface => 0x04,
            Self::AccessibilityAids => 0x05,
            Self::Unclassified => 0x06,
            Self::Reserved(v) => v,
        }
    }
    /// Spec token, or `"reserved"`.
    #[must_use]
    pub fn name(&self) -> &'static str {
        match self {
            Self::ConditionalAccess => "Conditional_Access",
            Self::ElectronicProgrammeGuide => "Electronic_Programme_Guide",
            Self::SoftwareUpgrade => "Software_upgrade",
            Self::NetworkInterface => "Network_interface",
            Self::AccessibilityAids => "Accessibility_aids",
            Self::Unclassified => "Unclassified",
            Self::Reserved(_) => "reserved",
        }
    }
    /// The effective type a v2 host applies (§5.1.2): an unrecognized
    /// (`Reserved`) type is treated as [`Unclassified`](Self::Unclassified).
    #[must_use]
    pub fn effective(self) -> Self {
        match self {
            Self::Reserved(_) => Self::Unclassified,
            known => known,
        }
    }
}
dvb_common::impl_spec_display!(ApplicationTypeV2, Reserved);

/// `application_info_enq()` — header-only enquiry.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct ApplicationInfoEnq;

/// `enter_menu()` — header-only command.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct EnterMenu;

/// `application_info()` reply (EN 50221 Table 21, with the v2 type set). The
/// `menu_string` is the raw top-level menu title text (EN 300 468 Annex A),
/// carried verbatim so it round-trips.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct ApplicationInfo<'a> {
    /// `application_type`.
    pub application_type: ApplicationTypeV2,
    /// `application_manufacturer` (16-bit).
    pub application_manufacturer: u16,
    /// `manufacturer_code` (16-bit).
    pub manufacturer_code: u16,
    /// `text_char` bytes of the menu title (length is `menu_string_length`, max 255).
    #[cfg_attr(feature = "serde", serde(borrow, with = "crate::objects::bytes_serde"))]
    pub menu_string: &'a [u8],
}

// --- empty-body objects ---

impl<'a> Parse<'a> for ApplicationInfoEnq {
    type Error = Error;
    fn parse(bytes: &'a [u8]) -> Result<Self> {
        objects::parse_empty_apdu(bytes, tag::APPLICATION_INFO_ENQ, "application_info_enq")?;
        Ok(Self)
    }
}
impl Serialize for ApplicationInfoEnq {
    type Error = Error;
    fn serialized_len(&self) -> usize {
        objects::empty_apdu_len()
    }
    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        objects::serialize_empty_apdu(tag::APPLICATION_INFO_ENQ, buf)
    }
}

impl<'a> Parse<'a> for EnterMenu {
    type Error = Error;
    fn parse(bytes: &'a [u8]) -> Result<Self> {
        objects::parse_empty_apdu(bytes, tag::ENTER_MENU, "enter_menu")?;
        Ok(Self)
    }
}
impl Serialize for EnterMenu {
    type Error = Error;
    fn serialized_len(&self) -> usize {
        objects::empty_apdu_len()
    }
    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        objects::serialize_empty_apdu(tag::ENTER_MENU, buf)
    }
}

// --- application_info ---

// application_type(1) + manufacturer(2) + code(2) + menu_string_length(1).
const APP_INFO_PREFIX: usize = 6;

impl<'a> Parse<'a> for ApplicationInfo<'a> {
    type Error = Error;
    fn parse(bytes: &'a [u8]) -> Result<Self> {
        let body = objects::parse_apdu_header(bytes, tag::APPLICATION_INFO, "application_info")?;
        if body.len() < APP_INFO_PREFIX {
            return Err(Error::BufferTooShort {
                need: APP_INFO_PREFIX,
                have: body.len(),
                what: "application_info",
            });
        }
        let menu_len = body[5] as usize;
        let menu_end = APP_INFO_PREFIX + menu_len;
        if body.len() < menu_end {
            return Err(Error::LengthMismatch {
                what: "application_info menu_string",
                declared: menu_len,
                actual: body.len() - APP_INFO_PREFIX,
            });
        }
        Ok(Self {
            application_type: ApplicationTypeV2::from_u8(body[0]),
            application_manufacturer: u16::from_be_bytes([body[1], body[2]]),
            manufacturer_code: u16::from_be_bytes([body[3], body[4]]),
            menu_string: &body[APP_INFO_PREFIX..menu_end],
        })
    }
}
impl Serialize for ApplicationInfo<'_> {
    type Error = Error;
    fn serialized_len(&self) -> usize {
        objects::apdu_len(APP_INFO_PREFIX + self.menu_string.len())
    }
    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        if self.menu_string.len() > u8::MAX as usize {
            return Err(Error::InvalidObject {
                what: "application_info",
                reason: "menu_string longer than 255 bytes",
            });
        }
        let body_len = APP_INFO_PREFIX + self.menu_string.len();
        let mut pos = objects::write_apdu_header(tag::APPLICATION_INFO, body_len, buf)?;
        buf[pos] = self.application_type.to_u8();
        buf[pos + 1..pos + 3].copy_from_slice(&self.application_manufacturer.to_be_bytes());
        buf[pos + 3..pos + 5].copy_from_slice(&self.manufacturer_code.to_be_bytes());
        buf[pos + 5] = self.menu_string.len() as u8;
        pos += APP_INFO_PREFIX;
        buf[pos..pos + self.menu_string.len()].copy_from_slice(self.menu_string);
        Ok(pos + self.menu_string.len())
    }
}

/// Resource-scoped dispatch over the Application Information v2 objects.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub enum ApplicationInfoV2Apdu<'a> {
    /// `application_info_enq` (`9F 80 20`).
    ApplicationInfoEnq(ApplicationInfoEnq),
    /// `application_info` (`9F 80 21`).
    ApplicationInfo(ApplicationInfo<'a>),
    /// `enter_menu` (`9F 80 22`).
    EnterMenu(EnterMenu),
}

impl<'a> ApplicationInfoV2Apdu<'a> {
    /// Parse an Application Information v2 APDU, dispatching on the `apdu_tag`.
    pub fn parse(body: &'a [u8]) -> Result<Self> {
        if body.len() < 3 {
            return Err(Error::BufferTooShort {
                need: 3,
                have: body.len(),
                what: "application_info_v2 apdu_tag",
            });
        }
        let t = ApduTag::from_bytes(body[0], body[1], body[2]);
        match t {
            tag::APPLICATION_INFO_ENQ => {
                Ok(Self::ApplicationInfoEnq(ApplicationInfoEnq::parse(body)?))
            }
            tag::APPLICATION_INFO => Ok(Self::ApplicationInfo(ApplicationInfo::parse(body)?)),
            tag::ENTER_MENU => Ok(Self::EnterMenu(EnterMenu::parse(body)?)),
            _ => Err(Error::UnexpectedApduTag {
                got: t.as_u24(),
                expected: tag::APPLICATION_INFO.as_u24(),
                what: "application_info_v2",
            }),
        }
    }
}

impl Serialize for ApplicationInfoV2Apdu<'_> {
    type Error = Error;
    fn serialized_len(&self) -> usize {
        match self {
            Self::ApplicationInfoEnq(o) => o.serialized_len(),
            Self::ApplicationInfo(o) => o.serialized_len(),
            Self::EnterMenu(o) => o.serialized_len(),
        }
    }
    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        match self {
            Self::ApplicationInfoEnq(o) => o.serialize_into(buf),
            Self::ApplicationInfo(o) => o.serialize_into(buf),
            Self::EnterMenu(o) => o.serialize_into(buf),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn enq_and_enter_menu_round_trip() {
        assert_eq!(ApplicationInfoEnq.to_bytes(), [0x9F, 0x80, 0x20, 0x00]);
        assert_eq!(EnterMenu.to_bytes(), [0x9F, 0x80, 0x22, 0x00]);
        assert_eq!(
            ApplicationInfoEnq::parse(&[0x9F, 0x80, 0x20, 0x00]).unwrap(),
            ApplicationInfoEnq
        );
    }

    #[test]
    fn application_info_round_trips_and_bites() {
        let info = ApplicationInfo {
            application_type: ApplicationTypeV2::AccessibilityAids,
            application_manufacturer: 0x1234,
            manufacturer_code: 0x5678,
            menu_string: b"Audio",
        };
        let bytes = info.to_bytes();
        // tag(3) + len(1) + prefix(6) + 5 = 15; body len = 11 = 0x0B.
        assert_eq!(
            bytes,
            [
                0x9F, 0x80, 0x21, 0x0B, 0x05, 0x12, 0x34, 0x56, 0x78, 0x05, b'A', b'u', b'd', b'i',
                b'o'
            ]
        );
        assert_eq!(ApplicationInfo::parse(&bytes).unwrap(), info);
        assert_eq!(info.application_type.name(), "Accessibility_aids");
        let mut other = info.clone();
        other.application_type = ApplicationTypeV2::Unclassified;
        assert_ne!(bytes, other.to_bytes());
    }

    #[test]
    fn unrecognized_type_treated_as_unclassified() {
        let t = ApplicationTypeV2::from_u8(0x7F);
        assert_eq!(t, ApplicationTypeV2::Reserved(0x7F));
        assert_eq!(t.effective(), ApplicationTypeV2::Unclassified);
        // wire value preserved for round-trip.
        assert_eq!(t.to_u8(), 0x7F);
    }

    #[test]
    fn dispatch_routes_each_tag() {
        let info = ApplicationInfo {
            application_type: ApplicationTypeV2::SoftwareUpgrade,
            application_manufacturer: 0,
            manufacturer_code: 0,
            menu_string: b"",
        }
        .to_bytes();
        assert!(matches!(
            ApplicationInfoV2Apdu::parse(&info).unwrap(),
            ApplicationInfoV2Apdu::ApplicationInfo(_)
        ));
    }
}
