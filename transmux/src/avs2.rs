//! AVS2 video in ISOBMFF — GY/T 420-2025 Annex A.2.
//!
//! | Type | FourCC | Spec |
//! |------|--------|------|
//! | [`Avs2ConfigurationBox`] | `av2c` | A.2.2.2 / A.2.2.4 `Avs2DecoderConfigurationRecord` |
//! | [`crate::avs3::LAvsConfigurationBox`] | `la2c` | A.2.2.3 / A.2.2.5 (same layout as AVS3's `lavc`) |
//! | [`crate::avs3::AvsVideoSampleEntry`] | `avst` / `lav2` | A.2.3.1 / A.2.3.2 |
//! | [`crate::avs3::TemporalLayerEntry`] | `telg` | A.2.3.4 (same as A.3.4.3) |
//!
//! The record is `configurationVersion(8)=1 sequence_header_length(16)
//! sequence_header[]`; the `sequence_header()` of GY/T 299.1-2016 is preserved
//! verbatim. The temporal-layer types are shared with [`crate::avs3`], the
//! entry/box layouts being identical field for field.
//!
//! Source note: A.2.3.1.1 prints the AVS2 sample entry type as `avsv` while its
//! syntax and the rest of the document use `avst`; `avst` is implemented.

use crate::avs3::parse_sequence_header_record;
use crate::error::{Error, Result};
use alloc::vec::Vec;
use broadcast_common::len::fit_u16;
use broadcast_common::{Parse, Serialize};

/// FourCC of the AVS2 configuration box.
pub const AV2C_FOURCC: [u8; 4] = *b"av2c";
/// FourCC of the LAVS2 (temporal layer) configuration box.
pub const LA2C_FOURCC: [u8; 4] = *b"la2c";
/// FourCC of the AVS2 sample entry.
pub const AVST_FOURCC: [u8; 4] = *b"avst";
/// FourCC of the LAVS2 sample entry.
pub const LAV2_FOURCC: [u8; 4] = *b"lav2";
const CONFIGURATION_VERSION: u8 = 1;

/// `Avs2DecoderConfigurationRecord` — the body of the `av2c` box (A.2.2.2).
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub struct Avs2ConfigurationBox {
    /// `configurationVersion` (8 bits; 1).
    pub configuration_version: u8,
    /// `sequence_header`: the `sequence_header()` of GY/T 299.1-2016, verbatim.
    pub sequence_header: Vec<u8>,
}

impl Avs2ConfigurationBox {
    /// Build a record (version 1).
    #[must_use]
    pub fn new(sequence_header: Vec<u8>) -> Self {
        Self {
            configuration_version: CONFIGURATION_VERSION,
            sequence_header,
        }
    }
}

impl<'a> Parse<'a> for Avs2ConfigurationBox {
    type Error = Error;
    fn parse(bytes: &'a [u8]) -> Result<Self> {
        let (version, header) = parse_sequence_header_record(bytes, "av2c body", 0)?;
        Ok(Self {
            configuration_version: version,
            sequence_header: header,
        })
    }
}

impl Serialize for Avs2ConfigurationBox {
    type Error = Error;
    fn serialized_len(&self) -> usize {
        3 + self.sequence_header.len()
    }
    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        let need = self.serialized_len();
        if buf.len() < need {
            return Err(Error::OutputBufferTooSmall {
                need,
                have: buf.len(),
            });
        }
        let len = fit_u16(self.sequence_header.len(), "sequence_header_length")?;
        buf[0] = self.configuration_version;
        buf[1..3].copy_from_slice(&len.to_be_bytes());
        buf[3..need].copy_from_slice(&self.sequence_header);
        Ok(need)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::avs3::{AvsVideoConfig, AvsVideoEntryKind, AvsVideoSampleEntry};
    use crate::sample_entries::{SampleEntryChild, VisualSampleEntryFields};
    use alloc::vec;

    /// A.2.2.2: version 1, 3-byte sequence header.
    #[test]
    fn av2c_record_vector() {
        let bytes = [1, 0, 3, 0x00, 0x00, 0xB0];
        let c = Avs2ConfigurationBox::parse(&bytes).unwrap();
        assert_eq!(c.configuration_version, 1);
        assert_eq!(c.sequence_header, [0x00, 0x00, 0xB0]);
        let mut out = vec![0u8; c.serialized_len()];
        c.serialize_into(&mut out).unwrap();
        assert_eq!(out, bytes);
    }

    #[test]
    fn av2c_rejects_truncation_and_overlong_header() {
        assert!(Avs2ConfigurationBox::parse(&[1, 0]).is_err());
        assert!(Avs2ConfigurationBox::parse(&[1, 0, 3, 0, 0]).is_err());
        let c = Avs2ConfigurationBox::new(vec![0; 70_000]);
        assert!(
            c.serialize_into(&mut vec![0u8; c.serialized_len()])
                .is_err()
        );
    }

    /// An `avst` entry with `av2c` round-trips; the fourcc constants agree with
    /// the entry kinds.
    #[test]
    fn avst_sample_entry_round_trips() {
        assert_eq!(AvsVideoEntryKind::Avs2.entry_fourcc(), AVST_FOURCC);
        assert_eq!(AvsVideoEntryKind::Avs2.config_fourcc(), AV2C_FOURCC);
        assert_eq!(AvsVideoEntryKind::LAvs2.entry_fourcc(), LAV2_FOURCC);
        assert_eq!(AvsVideoEntryKind::LAvs2.config_fourcc(), LA2C_FOURCC);
        let entry = AvsVideoSampleEntry {
            kind: AvsVideoEntryKind::Avs2,
            visual: VisualSampleEntryFields {
                width: 1920,
                height: 1080,
                ..VisualSampleEntryFields::default()
            },
            config: AvsVideoConfig::Avs2(Avs2ConfigurationBox::new(vec![0, 0, 1, 0xB0, 0x20])),
            children: vec![SampleEntryChild::Config],
        };
        let mut buf = vec![0u8; entry.serialized_len()];
        entry.serialize_into(&mut buf).unwrap();
        assert_eq!(&buf[4..8], b"avst");
        let back = AvsVideoSampleEntry::parse_entry(&buf).unwrap();
        assert_eq!(back, entry);
    }
}
