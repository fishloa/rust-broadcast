//! update_descriptor — ETSI TS 102 006 §9.5.2.6 Table 25 (UNT tag 0x02),
//! `update_flag` per Table 26, `update_method` per Table 27.
//!
//! Guides the SSU process. In the context of TS 102 006 the `update_flag` and
//! `update_method` fields are advisory. UNT common and operational loops
//! (Table 19).

use crate::descriptors::{descriptor_body, write_descriptor_header};
use crate::error::{Error, Result};
use broadcast_common::{Parse, Serialize};

/// Descriptor tag for update_descriptor (UNT namespace).
pub const TAG: u8 = 0x02;
const HEADER_LEN: usize = 2;
const FLAG_SHIFT: u8 = 6;
const FLAG_MASK: u8 = 0x03;
const METHOD_SHIFT: u8 = 2;
const METHOD_MASK: u8 = 0x0F;
const PRIORITY_MASK: u8 = 0x03;

/// `update_flag` — ETSI TS 102 006 §9.5.2.6 Table 26 (2 bits).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub enum UpdateFlag {
    /// `00` — the update has to be activated manually.
    Manual,
    /// `01` — the update may be performed automatically.
    Automatic,
    /// `10` / `11` — reserved for future use (the 2-bit value).
    Reserved(u8),
}

impl UpdateFlag {
    /// Decode from the 2-bit value (higher bits ignored).
    #[must_use]
    pub fn from_u8(v: u8) -> Self {
        match v & FLAG_MASK {
            0 => Self::Manual,
            1 => Self::Automatic,
            r => Self::Reserved(r),
        }
    }

    /// Encode to the 2-bit value.
    #[must_use]
    pub const fn to_u8(self) -> u8 {
        match self {
            Self::Manual => 0,
            Self::Automatic => 1,
            Self::Reserved(v) => v,
        }
    }

    /// Spec label (Table 26).
    #[must_use]
    pub fn name(self) -> &'static str {
        match self {
            Self::Manual => "manual",
            Self::Automatic => "automatic",
            Self::Reserved(_) => "reserved",
        }
    }
}
broadcast_common::impl_spec_display!(UpdateFlag, Reserved);

/// `update_method` — ETSI TS 102 006 §9.5.2.6 Table 27 (4 bits).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub enum UpdateMethod {
    /// 0 — immediate update: performed whatever the IRD state.
    Immediate,
    /// 1 — IRD available: taken into account when it does not interfere with
    /// the normal user operation.
    IrdAvailable,
    /// 2 — next restart: taken into account at the next IRD restart.
    NextRestart,
    /// 3 to 7 and 15 — reserved (the 4-bit value).
    Reserved(u8),
    /// 8 to 14 — private use (the 4-bit value).
    PrivateUse(u8),
}

impl UpdateMethod {
    /// Decode from the 4-bit value (higher bits ignored).
    #[must_use]
    pub fn from_u8(v: u8) -> Self {
        match v & METHOD_MASK {
            0 => Self::Immediate,
            1 => Self::IrdAvailable,
            2 => Self::NextRestart,
            p @ 8..=14 => Self::PrivateUse(p),
            r => Self::Reserved(r),
        }
    }

    /// Encode to the 4-bit value.
    #[must_use]
    pub const fn to_u8(self) -> u8 {
        match self {
            Self::Immediate => 0,
            Self::IrdAvailable => 1,
            Self::NextRestart => 2,
            Self::Reserved(v) | Self::PrivateUse(v) => v,
        }
    }

    /// Spec label (Table 27).
    #[must_use]
    pub fn name(self) -> &'static str {
        match self {
            Self::Immediate => "immediate update",
            Self::IrdAvailable => "IRD available",
            Self::NextRestart => "next restart",
            Self::Reserved(_) => "reserved",
            Self::PrivateUse(_) => "private use",
        }
    }
}
broadcast_common::impl_spec_display!(UpdateMethod, Reserved, PrivateUse);

/// update_descriptor.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[cfg_attr(feature = "yoke", derive(yoke::Yokeable))]
pub struct UpdateDescriptor<'a> {
    /// `update_flag` (Table 26).
    pub update_flag: UpdateFlag,
    /// `update_method` (Table 27).
    pub update_method: UpdateMethod,
    /// `update_priority` (2 bits): 0 is the highest priority, 3 the lowest.
    pub update_priority: u8,
    /// `private_data_byte`s — opaque operator data.
    pub private_data: &'a [u8],
}

impl<'a> Parse<'a> for UpdateDescriptor<'a> {
    type Error = Error;
    fn parse(bytes: &'a [u8]) -> Result<Self> {
        let body = descriptor_body(
            bytes,
            TAG,
            "UpdateDescriptor",
            "unexpected tag for update_descriptor",
        )?;
        let (b, private_data) = body.split_first().ok_or(Error::InvalidDescriptor {
            tag: TAG,
            reason: "update_descriptor body is empty",
        })?;
        Ok(Self {
            update_flag: UpdateFlag::from_u8(b >> FLAG_SHIFT),
            update_method: UpdateMethod::from_u8(b >> METHOD_SHIFT),
            update_priority: b & PRIORITY_MASK,
            private_data,
        })
    }
}

impl Serialize for UpdateDescriptor<'_> {
    type Error = Error;
    fn serialized_len(&self) -> usize {
        HEADER_LEN + 1 + self.private_data.len()
    }
    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        let len = self.serialized_len();
        if buf.len() < len {
            return Err(Error::OutputBufferTooSmall {
                need: len,
                have: buf.len(),
            });
        }
        let flag =
            broadcast_common::len::fit_mask(self.update_flag.to_u8(), FLAG_MASK, "update_flag")?;
        let method = broadcast_common::len::fit_mask(
            self.update_method.to_u8(),
            METHOD_MASK,
            "update_method",
        )?;
        let prio = broadcast_common::len::fit_mask(
            self.update_priority,
            PRIORITY_MASK,
            "update_priority",
        )?;
        write_descriptor_header(buf, TAG, len - HEADER_LEN)?;
        buf[2] = (flag << FLAG_SHIFT) | (method << METHOD_SHIFT) | prio;
        buf[3..len].copy_from_slice(self.private_data);
        Ok(len)
    }
}

impl<'a> crate::traits::DescriptorDef<'a> for UpdateDescriptor<'a> {
    const TAG: u8 = TAG;
    const NAME: &'static str = "UPDATE";
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_fields_and_round_trip() {
        // automatic(01) immediate(0000) priority 11 = 0b01_0000_11.
        let bytes = [TAG, 3, 0x43, 0xCA, 0xFE];
        let d = UpdateDescriptor::parse(&bytes).unwrap();
        assert_eq!(d.update_flag, UpdateFlag::Automatic);
        assert_eq!(d.update_method, UpdateMethod::Immediate);
        assert_eq!(d.update_priority, 3);
        assert_eq!(d.private_data, &[0xCA, 0xFE]);
        let mut out = vec![0u8; d.serialized_len()];
        d.serialize_into(&mut out).unwrap();
        assert_eq!(out, bytes);
    }

    #[test]
    fn every_byte_round_trips() {
        for b in 0..=u8::MAX {
            let bytes = [TAG, 1, b];
            let d = UpdateDescriptor::parse(&bytes).unwrap();
            let mut out = vec![0u8; d.serialized_len()];
            d.serialize_into(&mut out).unwrap();
            assert_eq!(out, bytes, "byte {b:#04x}");
        }
    }

    #[test]
    fn enums_lossless_and_labelled() {
        for v in 0..=3u8 {
            assert_eq!(UpdateFlag::from_u8(v).to_u8(), v);
        }
        for v in 0..=15u8 {
            assert_eq!(UpdateMethod::from_u8(v).to_u8(), v);
        }
        assert_eq!(UpdateMethod::from_u8(9), UpdateMethod::PrivateUse(9));
        assert_eq!(UpdateMethod::from_u8(15), UpdateMethod::Reserved(15));
        assert_eq!(UpdateMethod::NextRestart.to_string(), "next restart");
        assert_eq!(UpdateFlag::Reserved(2).to_string(), "reserved(0x02)");
    }

    #[test]
    fn rejects_empty_body_and_wrong_tag() {
        assert!(UpdateDescriptor::parse(&[TAG, 0]).is_err());
        assert!(UpdateDescriptor::parse(&[0x03, 1, 0]).is_err());
    }

    #[test]
    fn serialize_rejects_over_range_priority() {
        let d = UpdateDescriptor {
            update_flag: UpdateFlag::Manual,
            update_method: UpdateMethod::Immediate,
            update_priority: 4,
            private_data: &[],
        };
        let mut out = vec![0u8; d.serialized_len()];
        assert!(d.serialize_into(&mut out).is_err());
    }
}
