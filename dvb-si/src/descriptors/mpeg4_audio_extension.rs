//! MPEG-4 audio extension descriptor — Rec. ITU-T H.222.0 (08/2023) |
//! ISO/IEC 13818-1:2023 §2.6.72–2.6.73, Table 2-97 (tag 0x2E); syntax in
//! `docs/descriptors/iso_13818_1/2d-2e-mpeg-4-text-and-audio-extension-descriptors.md`.
//!
//! ```text
//! ASC_flag(1) reserved(3) num_of_loops(4) { audioProfileLevelIndication(8) }*
//! [ ASC_size(8) audioSpecificConfig() ]      -- iff ASC_flag
//! ```
//!
//! `audioSpecificConfig()` is ISO/IEC 14496-3 §1.6.2.1 (not vendored): it is
//! carried as an opaque byte run of `ASC_size` bytes, by design.

use crate::error::Result;
use crate::h222_bits::{BitReader, BitWriter, impl_h222_descriptor};
use alloc::vec::Vec;

/// Descriptor tag of `MPEG-4_audio_extension_descriptor` (Table 2-45: 46).
pub const TAG: u8 = 0x2E;
/// Default of the 3-bit `reserved` (H.222.0 clause 2.1: all ones).
pub const RESERVED_DEFAULT: u8 = 0x07;

/// `MPEG-4_audio_extension_descriptor()` — Table 2-97.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[cfg_attr(feature = "yoke", derive(yoke::Yokeable))]
#[non_exhaustive]
pub struct Mpeg4AudioExtensionDescriptor {
    /// `reserved(3)` after `ASC_flag`; re-emitted verbatim, default 7.
    pub reserved: u8,
    /// `audioProfileLevelIndication` values (`num_of_loops` is 4 bits: at most
    /// 15); coded per ISO/IEC 14496-3 §1.5.2.1 (external code, not vendored).
    pub audio_profile_level_indications: Vec<u8>,
    /// `audioSpecificConfig()` (`ASC_size` bytes), present iff `ASC_flag == '1'`;
    /// opaque (ISO/IEC 14496-3 is not vendored).
    pub audio_specific_config: Option<Vec<u8>>,
    /// Bytes after the syntax, preserved verbatim.
    pub trailing: Vec<u8>,
}

impl Mpeg4AudioExtensionDescriptor {
    /// Build with the spec `reserved` fill and no trailing bytes.
    #[must_use]
    pub fn new(
        audio_profile_level_indications: Vec<u8>,
        audio_specific_config: Option<Vec<u8>>,
    ) -> Self {
        Self {
            reserved: RESERVED_DEFAULT,
            audio_profile_level_indications,
            audio_specific_config,
            trailing: Vec::new(),
        }
    }

    fn parse_body(body: &[u8]) -> Result<Self> {
        let mut r = BitReader::new(body, "Mpeg4AudioExtensionDescriptor");
        let asc_flag = r.flag()?;
        let reserved = r.u8(3)?;
        let n = r.bits(4)?;
        let mut indications = Vec::new();
        for _ in 0..n {
            indications.push(r.u8(8)?);
        }
        let audio_specific_config = if asc_flag {
            let size = usize::from(r.u8(8)?);
            Some(r.bytes(size)?.to_vec())
        } else {
            None
        };
        Ok(Self {
            reserved,
            audio_profile_level_indications: indications,
            audio_specific_config,
            trailing: r.rest()?.to_vec(),
        })
    }

    fn write_body(&self) -> Result<Vec<u8>> {
        let mut w = BitWriter::new();
        w.flag(self.audio_specific_config.is_some(), "ASC_flag")?;
        w.put(u64::from(self.reserved), 3, "reserved")?;
        w.put(
            self.audio_profile_level_indications.len() as u64,
            4,
            "num_of_loops",
        )?;
        w.bytes(&self.audio_profile_level_indications)?;
        if let Some(asc) = &self.audio_specific_config {
            w.put(asc.len() as u64, 8, "ASC_size")?;
            w.bytes(asc)?;
        }
        w.bytes(&self.trailing)?;
        Ok(w.into_vec())
    }
}

impl_h222_descriptor!(
    Mpeg4AudioExtensionDescriptor,
    TAG,
    "Mpeg4AudioExtensionDescriptor",
    "MPEG4_AUDIO_EXTENSION"
);

#[cfg(test)]
#[allow(clippy::unusual_byte_groupings)] // bit-field-wise literals mirror the syntax tables
mod tests {
    use super::*;
    use broadcast_common::{Parse, Serialize};

    fn rt(b: &[u8]) -> Mpeg4AudioExtensionDescriptor {
        let d = Mpeg4AudioExtensionDescriptor::parse(b).unwrap();
        let mut out = vec![0u8; d.serialized_len()];
        d.serialize_into(&mut out).unwrap();
        assert_eq!(out, b);
        d
    }

    #[test]
    fn with_and_without_asc() {
        // ASC_flag=1 reserved=111 loops=2, indications 0x2F 0x30, ASC_size=2 + bytes
        let d = rt(&[TAG, 6, 0b1_111_0010, 0x2F, 0x30, 2, 0x12, 0x10]);
        assert_eq!(d.audio_profile_level_indications, [0x2F, 0x30]);
        assert_eq!(d.audio_specific_config.as_deref(), Some(&[0x12, 0x10][..]));
        // ASC_flag=0, zero loops, non-default reserved + trailing byte
        let d = rt(&[TAG, 2, 0b0_000_0000, 0x55]);
        assert_eq!(d.reserved, 0);
        assert_eq!(d.trailing, [0x55]);
    }

    #[test]
    fn rejects_truncation_and_overflow() {
        assert!(Mpeg4AudioExtensionDescriptor::parse(&[TAG, 3, 0b1_111_0001, 0x2F, 5]).is_err());
        let d = Mpeg4AudioExtensionDescriptor::new(vec![0; 16], None);
        assert!(d.serialize_into(&mut [0u8; 64]).is_err());
        let d = Mpeg4AudioExtensionDescriptor::new(vec![], Some(vec![0; 256]));
        assert!(d.serialize_into(&mut [0u8; 300]).is_err());
    }
}
