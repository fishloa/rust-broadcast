//! SSU Module Type Descriptor — ETSI TS 102 006 §8.2.1, Table 9 (data-carousel
//! tag 0x0A, allocated by EN 301 192 Table 48; module types Table 10). PDF p.18
//! of the vendored V1.4.1 text, transcribed in
//! `docs/descriptors/ts_102_006/9-*.md` and `docs/enums/ts_102_006/10-*.md`,
//! verified against the rendered page.
//!
//! `SSU_module_type`(8): the type of the SSU module (DII `moduleInfo` only).

use crate::descriptors::{descriptor_body, write_descriptor_header};
use crate::error::{Error, Result};
use broadcast_common::{Parse, Serialize};

/// Descriptor tag for SSU_module_type_descriptor (data-carousel namespace).
pub const TAG: u8 = 0x0A;
const HEADER_LEN: usize = 2;
const BODY_LEN: usize = 1;

/// `SSU_module_type` (TS 102 006 Table 10).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub enum SsuModuleType {
    /// 0x00 — executable module type.
    Executable,
    /// 0x01 — memory mapped code module type.
    MemoryMappedCode,
    /// 0x02 — data module type.
    Data,
    /// 0x03..=0xFF — reserved for future use.
    Reserved(u8),
}

impl SsuModuleType {
    /// Decode from the 8-bit wire value.
    #[must_use]
    pub const fn from_u8(v: u8) -> Self {
        match v {
            0x00 => Self::Executable,
            0x01 => Self::MemoryMappedCode,
            0x02 => Self::Data,
            other => Self::Reserved(other),
        }
    }

    /// Encode to the 8-bit wire value.
    #[must_use]
    pub const fn to_u8(self) -> u8 {
        match self {
            Self::Executable => 0x00,
            Self::MemoryMappedCode => 0x01,
            Self::Data => 0x02,
            Self::Reserved(v) => v,
        }
    }

    /// Spec label.
    #[must_use]
    pub fn name(&self) -> &'static str {
        match self {
            Self::Executable => "executable module type",
            Self::MemoryMappedCode => "memory mapped code module type",
            Self::Data => "data module type",
            Self::Reserved(_) => "reserved",
        }
    }
}
broadcast_common::impl_spec_display!(SsuModuleType, Reserved);

/// SSU Module Type Descriptor (carousel tag 0x0A).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct SsuModuleTypeDescriptor {
    /// `SSU_module_type`.
    pub ssu_module_type: SsuModuleType,
}

impl<'a> Parse<'a> for SsuModuleTypeDescriptor {
    type Error = crate::error::Error;
    fn parse(bytes: &'a [u8]) -> Result<Self> {
        let body = descriptor_body(
            bytes,
            TAG,
            "SsuModuleTypeDescriptor",
            "unexpected tag for SSU_module_type_descriptor",
        )?;
        match body {
            [t] => Ok(Self {
                ssu_module_type: SsuModuleType::from_u8(*t),
            }),
            _ => Err(Error::InvalidDescriptor {
                tag: TAG,
                reason: "SSU_module_type_descriptor body must be 1 byte",
            }),
        }
    }
}

impl Serialize for SsuModuleTypeDescriptor {
    type Error = crate::error::Error;
    fn serialized_len(&self) -> usize {
        HEADER_LEN + BODY_LEN
    }

    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        let len = self.serialized_len();
        if buf.len() < len {
            return Err(Error::OutputBufferTooSmall {
                need: len,
                have: buf.len(),
            });
        }
        write_descriptor_header(buf, TAG, BODY_LEN)?;
        buf[2] = self.ssu_module_type.to_u8();
        Ok(len)
    }
}

impl crate::traits::DescriptorDef<'_> for SsuModuleTypeDescriptor {
    const TAG: u8 = TAG;
    const NAME: &'static str = "SSU_MODULE_TYPE";
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_and_round_trip_every_value() {
        for v in 0..=255u8 {
            let bytes = [TAG, 1, v];
            let d = SsuModuleTypeDescriptor::parse(&bytes).unwrap();
            assert_eq!(d.ssu_module_type, SsuModuleType::from_u8(v));
            let mut out = [0u8; 3];
            d.serialize_into(&mut out).unwrap();
            assert_eq!(out, bytes);
        }
        assert_eq!(SsuModuleType::from_u8(2), SsuModuleType::Data);
    }

    #[test]
    fn wrong_length_rejected() {
        assert!(SsuModuleTypeDescriptor::parse(&[TAG, 0]).is_err());
        assert!(SsuModuleTypeDescriptor::parse(&[TAG, 2, 0, 0]).is_err());
    }
}
