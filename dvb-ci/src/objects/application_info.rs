//! Application Information objects — ETSI EN 50221 §8.4.2, Tables 20-22
//! (PDF pp. 27-28).
//!
//! - `application_info_enq` (`9F 80 20`, Table 20) — header-only enquiry.
//! - `application_info` (`9F 80 21`, Table 21) — type/manufacturer + menu string.
//! - `enter_menu` (`9F 80 22`, Table 22) — header-only command.

use crate::error::{Error, Result};
use crate::tag::{self, ApduTag};
use crate::traits::ApduDef;
use dvb_common::{Parse, Serialize};

/// `application_type` (Table, p. 28).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub enum ApplicationType {
    /// `01` — Conditional Access.
    ConditionalAccess,
    /// `02` — Electronic Programme Guide.
    ElectronicProgrammeGuide,
    /// Any other value (reserved).
    Reserved(u8),
}

impl ApplicationType {
    /// Decode an `application_type` byte.
    #[must_use]
    pub fn from_u8(v: u8) -> Self {
        match v {
            0x01 => Self::ConditionalAccess,
            0x02 => Self::ElectronicProgrammeGuide,
            other => Self::Reserved(other),
        }
    }
    /// Wire byte for this `application_type`.
    #[must_use]
    pub fn to_u8(self) -> u8 {
        match self {
            Self::ConditionalAccess => 0x01,
            Self::ElectronicProgrammeGuide => 0x02,
            Self::Reserved(v) => v,
        }
    }
    /// Spec token, or `"reserved"`.
    #[must_use]
    pub fn name(&self) -> &'static str {
        match self {
            Self::ConditionalAccess => "Conditional_Access",
            Self::ElectronicProgrammeGuide => "Electronic_Programme_Guide",
            Self::Reserved(_) => "reserved",
        }
    }
}
dvb_common::impl_spec_display!(ApplicationType, Reserved);

/// `application_info_enq()` — header-only enquiry (Table 20).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct ApplicationInfoEnq;

/// `enter_menu()` — header-only command (Table 22).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct EnterMenu;

/// `application_info()` reply (Table 21). The `menu_string` is the raw text
/// bytes of the top-level menu title (EN 300 468 Annex A coding); carried
/// verbatim so it round-trips.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct ApplicationInfo<'a> {
    /// `application_type`.
    pub application_type: ApplicationType,
    /// `application_manufacturer` (16-bit).
    pub application_manufacturer: u16,
    /// `manufacturer_code` (16-bit).
    pub manufacturer_code: u16,
    /// `text_char` bytes of the top-level menu title (length is the
    /// `menu_string_length` field, max 255).
    #[cfg_attr(feature = "serde", serde(borrow, with = "super::bytes_serde"))]
    pub menu_string: &'a [u8],
}

// --- application_info_enq (empty body) ---

impl<'a> Parse<'a> for ApplicationInfoEnq {
    type Error = Error;
    fn parse(bytes: &'a [u8]) -> Result<Self> {
        super::parse_empty_apdu(bytes, tag::APPLICATION_INFO_ENQ, "application_info_enq")?;
        Ok(Self)
    }
}
impl Serialize for ApplicationInfoEnq {
    type Error = Error;
    fn serialized_len(&self) -> usize {
        super::empty_apdu_len()
    }
    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        super::serialize_empty_apdu(tag::APPLICATION_INFO_ENQ, buf)
    }
}
impl<'a> ApduDef<'a> for ApplicationInfoEnq {
    const TAG: ApduTag = tag::APPLICATION_INFO_ENQ;
    const NAME: &'static str = "APPLICATION_INFO_ENQ";
}

// --- enter_menu (empty body) ---

impl<'a> Parse<'a> for EnterMenu {
    type Error = Error;
    fn parse(bytes: &'a [u8]) -> Result<Self> {
        super::parse_empty_apdu(bytes, tag::ENTER_MENU, "enter_menu")?;
        Ok(Self)
    }
}
impl Serialize for EnterMenu {
    type Error = Error;
    fn serialized_len(&self) -> usize {
        super::empty_apdu_len()
    }
    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        super::serialize_empty_apdu(tag::ENTER_MENU, buf)
    }
}
impl<'a> ApduDef<'a> for EnterMenu {
    const TAG: ApduTag = tag::ENTER_MENU;
    const NAME: &'static str = "ENTER_MENU";
}

// --- application_info ---

/// Fixed-prefix length: application_type(1) + manufacturer(2) + code(2) +
/// menu_string_length(1).
const APP_INFO_PREFIX: usize = 6;

impl<'a> Parse<'a> for ApplicationInfo<'a> {
    type Error = Error;
    fn parse(bytes: &'a [u8]) -> Result<Self> {
        let body = super::parse_apdu_header(bytes, tag::APPLICATION_INFO, "application_info")?;
        if body.len() < APP_INFO_PREFIX {
            return Err(Error::BufferTooShort {
                need: APP_INFO_PREFIX,
                have: body.len(),
                what: "application_info",
            });
        }
        let application_type = ApplicationType::from_u8(body[0]);
        let application_manufacturer = u16::from_be_bytes([body[1], body[2]]);
        let manufacturer_code = u16::from_be_bytes([body[3], body[4]]);
        let menu_string_length = body[5] as usize;
        let menu_end = APP_INFO_PREFIX + menu_string_length;
        if body.len() < menu_end {
            return Err(Error::LengthMismatch {
                what: "application_info menu_string",
                declared: menu_string_length,
                actual: body.len() - APP_INFO_PREFIX,
            });
        }
        Ok(Self {
            application_type,
            application_manufacturer,
            manufacturer_code,
            menu_string: &body[APP_INFO_PREFIX..menu_end],
        })
    }
}

impl Serialize for ApplicationInfo<'_> {
    type Error = Error;
    fn serialized_len(&self) -> usize {
        super::apdu_len(APP_INFO_PREFIX + self.menu_string.len())
    }
    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        if self.menu_string.len() > u8::MAX as usize {
            return Err(Error::InvalidObject {
                what: "application_info",
                reason: "menu_string longer than 255 bytes",
            });
        }
        let body_len = APP_INFO_PREFIX + self.menu_string.len();
        let mut pos = super::write_apdu_header(tag::APPLICATION_INFO, body_len, buf)?;
        buf[pos] = self.application_type.to_u8();
        buf[pos + 1..pos + 3].copy_from_slice(&self.application_manufacturer.to_be_bytes());
        buf[pos + 3..pos + 5].copy_from_slice(&self.manufacturer_code.to_be_bytes());
        buf[pos + 5] = self.menu_string.len() as u8;
        pos += APP_INFO_PREFIX;
        buf[pos..pos + self.menu_string.len()].copy_from_slice(self.menu_string);
        Ok(pos + self.menu_string.len())
    }
}

impl<'a> ApduDef<'a> for ApplicationInfo<'a> {
    const TAG: ApduTag = tag::APPLICATION_INFO;
    const NAME: &'static str = "APPLICATION_INFO";
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn enq_round_trip() {
        let bytes = ApplicationInfoEnq.to_bytes();
        assert_eq!(bytes, [0x9F, 0x80, 0x20, 0x00]);
        assert_eq!(
            ApplicationInfoEnq::parse(&bytes).unwrap(),
            ApplicationInfoEnq
        );
    }

    #[test]
    fn enter_menu_round_trip() {
        let bytes = EnterMenu.to_bytes();
        assert_eq!(bytes, [0x9F, 0x80, 0x22, 0x00]);
        assert_eq!(EnterMenu::parse(&bytes).unwrap(), EnterMenu);
    }

    #[test]
    fn application_info_round_trip() {
        let info = ApplicationInfo {
            application_type: ApplicationType::ConditionalAccess,
            application_manufacturer: 0x1234,
            manufacturer_code: 0x5678,
            menu_string: b"CA Module",
        };
        let bytes = info.to_bytes();
        // tag(3)+len(1)+prefix(6)+9
        assert_eq!(bytes.len(), 19);
        assert_eq!(&bytes[..4], &[0x9F, 0x80, 0x21, 0x0F]); // len 15
        let parsed = ApplicationInfo::parse(&bytes).unwrap();
        assert_eq!(parsed, info);
        assert_eq!(parsed.application_type.name(), "Conditional_Access");
    }

    #[test]
    fn mutating_field_changes_bytes() {
        let info = ApplicationInfo {
            application_type: ApplicationType::ConditionalAccess,
            application_manufacturer: 0x1234,
            manufacturer_code: 0x5678,
            menu_string: b"abc",
        };
        let a = info.to_bytes();
        let mut other = info.clone();
        other.application_manufacturer = 0x9999;
        assert_ne!(a, other.to_bytes());
    }

    #[test]
    fn empty_menu_string() {
        let info = ApplicationInfo {
            application_type: ApplicationType::ElectronicProgrammeGuide,
            application_manufacturer: 0,
            manufacturer_code: 0,
            menu_string: b"",
        };
        let bytes = info.to_bytes();
        assert_eq!(ApplicationInfo::parse(&bytes).unwrap(), info);
    }
}
