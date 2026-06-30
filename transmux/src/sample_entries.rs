//! AVC/HEVC sample entries and their config boxes — ISO/IEC 14496-15:2017 §5.4, §8.4.
//!
//! Sample entries live inside `stsd` and extend `VisualSampleEntry`. Each carries
//! a configuration box (`avcC` or `hvcC`) containing the decoder configuration record
//! with parameter sets (SPS/PPS/VPS).
//!
//! # Types
//!
//! | Sample entry | Four-CC | Params in sample entry | Params in-band |
//! |---------------|---------|----------------------|----------------|
//! | `avc1`        | avc1    | yes                   | no             |
//! | `avc3`        | avc3    | yes                   | yes            |
//! | `avc2`        | avc2    | yes (extractors)      | no             |
//! | `avc4`        | avc4    | yes (extractors)      | yes            |
//! | `hvc1`        | hvc1    | yes (`array_completeness=1`) | no      |
//! | `hev1`        | hev1    | yes                   | yes            |

use crate::avc_config::{AVCConfigurationBox, AVCDecoderConfigurationRecord};
use crate::error::{Error, Result};
use crate::hevc_config::{HEVCConfigurationBox, HEVCDecoderConfigurationRecord};
use broadcast_common::Serialize;

/// Length of the VisualSampleEntry fixed fields (ISO/IEC 14496-12:2015 §12.1.3).
/// 16 bytes reserved + 2x4 (width/height) + 4x2 (horiz/vert resolution) +
/// 4 (data_size/entries) + 2 (frame_count) + 32 (compressorname) + 2 (depth) + 2 (predefined) = 78
const VISUAL_SAMPLE_ENTRY_SIZE: usize = 78;

// ---------------------------------------------------------------------------
// VisualSampleEntry fields (common to all visual sample entries)
// ---------------------------------------------------------------------------

/// Fixed fields shared by all visual sample entries, per ISO/IEC 14496-12:2015 §12.1.3.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct VisualSampleEntryFields {
    /// Width in pixels.
    pub width: u16,
    /// Height in pixels.
    pub height: u16,
    /// Horizontal resolution (fixed-point 16.16, default 0x00480000 = 72 dpi).
    pub horizontal_resolution: u32,
    /// Vertical resolution (fixed-point 16.16).
    pub vertical_resolution: u32,
    /// Data size (reserved, 0).
    pub data_size: u32,
    /// Frame count (typically 1).
    pub frame_count: u16,
    /// Compressor name (32 bytes, zero-padded).
    pub compressorname: [u8; 32],
    /// Depth (typically 0x0018 = 24).
    pub depth: u16,
    /// Pre-defined (reserved, -1 = 0xFFFF).
    pub predefined: u16,
}

impl Default for VisualSampleEntryFields {
    fn default() -> Self {
        Self {
            width: 0,
            height: 0,
            horizontal_resolution: 0x00480000,
            vertical_resolution: 0x00480000,
            data_size: 0,
            frame_count: 1,
            compressorname: [0u8; 32],
            depth: 0x0018,
            predefined: 0xFFFF,
        }
    }
}

impl VisualSampleEntryFields {
    /// Number of bytes serialized (constant).
    pub const fn serialized_len() -> usize {
        VISUAL_SAMPLE_ENTRY_SIZE
    }

    /// Serialize the fixed fields into a buffer.
    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        if buf.len() < VISUAL_SAMPLE_ENTRY_SIZE {
            return Err(Error::OutputBufferTooSmall {
                need: VISUAL_SAMPLE_ENTRY_SIZE,
                have: buf.len(),
            });
        }
        let mut cursor = 0usize;

        // 16 bytes reserved (zero)
        cursor += 16;
        // width (16)
        buf[cursor..cursor + 2].copy_from_slice(&self.width.to_be_bytes());
        cursor += 2;
        // height (16)
        buf[cursor..cursor + 2].copy_from_slice(&self.height.to_be_bytes());
        cursor += 2;
        // horizontal resolution (32, fixed-16.16)
        buf[cursor..cursor + 4].copy_from_slice(&self.horizontal_resolution.to_be_bytes());
        cursor += 4;
        // vertical resolution (32, fixed-16.16)
        buf[cursor..cursor + 4].copy_from_slice(&self.vertical_resolution.to_be_bytes());
        cursor += 4;
        // data size (32, reserved = 0)
        buf[cursor..cursor + 4].copy_from_slice(&self.data_size.to_be_bytes());
        cursor += 4;
        // frame count (16)
        buf[cursor..cursor + 2].copy_from_slice(&self.frame_count.to_be_bytes());
        cursor += 2;
        // compressorname (32 bytes)
        buf[cursor..cursor + 32].copy_from_slice(&self.compressorname);
        cursor += 32;
        // depth (16)
        buf[cursor..cursor + 2].copy_from_slice(&self.depth.to_be_bytes());
        cursor += 2;
        // predefined (16)
        buf[cursor..cursor + 2].copy_from_slice(&self.predefined.to_be_bytes());
        cursor += 2;

        Ok(cursor)
    }
}

// ---------------------------------------------------------------------------
// AVC Sample Entries: avc1 and avc3
// ---------------------------------------------------------------------------

/// H.264/AVC sample entry — ISO/IEC 14496-15:2017 §5.4.2.
///
/// Fields common to `avc1` and `avc3`.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct AVCSampleEntry {
    /// The four-CC (avc1 or avc3).
    pub codec_type: [u8; 4],
    /// Visual sample entry fields.
    pub visual: VisualSampleEntryFields,
    /// AVC configuration box (avcC).
    pub config: AVCConfigurationBox,
}

impl AVCSampleEntry {
    /// Create a new avc1 sample entry.
    pub fn new_avc1(config: AVCDecoderConfigurationRecord) -> Self {
        let compressorname = {
            let mut c = [0u8; 32];
            c[0] = 11;
            c[1..11].copy_from_slice(b"AVC Coding");
            c
        };
        Self {
            codec_type: *b"avc1",
            visual: VisualSampleEntryFields {
                compressorname,
                ..VisualSampleEntryFields::default()
            },
            config: AVCConfigurationBox::new(config),
        }
    }

    /// Create a new avc3 sample entry.
    pub fn new_avc3(config: AVCDecoderConfigurationRecord) -> Self {
        let mut s = Self::new_avc1(config);
        s.codec_type = *b"avc3";
        s
    }
}

impl Serialize for AVCSampleEntry {
    type Error = Error;

    fn serialized_len(&self) -> usize {
        8 + VisualSampleEntryFields::serialized_len() + self.config.serialized_len()
    }

    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        let need = self.serialized_len();
        if buf.len() < need {
            return Err(Error::OutputBufferTooSmall {
                need,
                have: buf.len(),
            });
        }
        let mut cursor = 0usize;
        let size32 = need as u32;
        buf[cursor..cursor + 4].copy_from_slice(&size32.to_be_bytes());
        cursor += 4;
        buf[cursor..cursor + 4].copy_from_slice(&self.codec_type);
        cursor += 4;
        cursor += self.visual.serialize_into(&mut buf[cursor..])?;
        cursor += self.config.serialize_into(&mut buf[cursor..])?;
        Ok(cursor)
    }
}

// ---------------------------------------------------------------------------
// HEVC Sample Entries: hvc1 and hev1
// ---------------------------------------------------------------------------

/// H.265/HEVC sample entry — ISO/IEC 14496-15:2017 §8.4.1.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct HEVCSampleEntry {
    /// The four-CC (hvc1 or hev1).
    pub codec_type: [u8; 4],
    /// Visual sample entry fields.
    pub visual: VisualSampleEntryFields,
    /// HEVC configuration box (hvcC).
    pub config: HEVCConfigurationBox,
}

impl HEVCSampleEntry {
    /// Create a new hvc1 sample entry.
    pub fn new_hvc1(config: HEVCDecoderConfigurationRecord) -> Self {
        let compressorname = {
            let mut c = [0u8; 32];
            c[0] = 12;
            c[1..12].copy_from_slice(b"HEVC Coding");
            c
        };
        Self {
            codec_type: *b"hvc1",
            visual: VisualSampleEntryFields {
                compressorname,
                ..VisualSampleEntryFields::default()
            },
            config: HEVCConfigurationBox::new(config),
        }
    }

    /// Create a new hev1 sample entry.
    pub fn new_hev1(config: HEVCDecoderConfigurationRecord) -> Self {
        let mut s = Self::new_hvc1(config);
        s.codec_type = *b"hev1";
        s
    }
}

impl Serialize for HEVCSampleEntry {
    type Error = Error;

    fn serialized_len(&self) -> usize {
        8 + VisualSampleEntryFields::serialized_len() + self.config.serialized_len()
    }

    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        let need = self.serialized_len();
        if buf.len() < need {
            return Err(Error::OutputBufferTooSmall {
                need,
                have: buf.len(),
            });
        }
        let mut cursor = 0usize;
        let size32 = need as u32;
        buf[cursor..cursor + 4].copy_from_slice(&size32.to_be_bytes());
        cursor += 4;
        buf[cursor..cursor + 4].copy_from_slice(&self.codec_type);
        cursor += 4;
        cursor += self.visual.serialize_into(&mut buf[cursor..])?;
        cursor += self.config.serialize_into(&mut buf[cursor..])?;
        Ok(cursor)
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::avc_config::AVCDecoderConfigurationRecord;
    use crate::hevc_config::HEVCDecoderConfigurationRecord;
    use alloc::vec;
    use broadcast_common::Serialize;

    fn make_avc_config() -> AVCDecoderConfigurationRecord {
        AVCDecoderConfigurationRecord {
            configuration_version: 1,
            profile_indication: 100,
            profile_compatibility: 0,
            level_indication: 30,
            length_size_minus_one: 3,
            sps: alloc::vec![crate::nalu_types::AvcSps(vec![0x67, 0x64, 0x00, 0x1E])],
            pps: alloc::vec![crate::nalu_types::AvcPps(vec![0x68, 0xEE, 0x3C])],
            chroma_format: Some(1),
            bit_depth_luma_minus8: Some(0),
            bit_depth_chroma_minus8: Some(0),
            sps_ext: alloc::vec![],
        }
    }

    fn make_hevc_config() -> HEVCDecoderConfigurationRecord {
        HEVCDecoderConfigurationRecord {
            configuration_version: 1,
            general_profile_space: 0,
            general_tier_flag: false,
            general_profile_idc: 1,
            general_profile_compatibility_flags: 0,
            general_constraint_indicator_flags: 0,
            general_level_idc: 93,
            min_spatial_segmentation_idc: 0,
            parallelism_type: 0,
            chroma_format_idc: 1,
            bit_depth_luma_minus8: 0,
            bit_depth_chroma_minus8: 0,
            avg_frame_rate: 0,
            constant_frame_rate: 0,
            num_temporal_layers: 1,
            temporal_id_nested: false,
            length_size_minus_one: 3,
            arrays: alloc::vec![crate::nalu_types::HevcNalArray {
                array_completeness: true,
                nal_unit_type: 32,
                nalus: alloc::vec![crate::nalu_types::HevcNalUnit(vec![
                    0x40, 0x01, 0x0C, 0x01, 0xFF
                ])],
            }],
        }
    }

    #[test]
    fn test_avc1_serialize() {
        let config = make_avc_config();
        let entry = AVCSampleEntry::new_avc1(config);
        let bytes = entry.to_bytes();
        assert!(bytes.len() > 8);
        assert_eq!(&bytes[4..8], b"avc1");
        assert!(bytes.windows(4).any(|w| w == b"avcC"));
    }

    #[test]
    fn test_avc3_serialize() {
        let config = make_avc_config();
        let entry = AVCSampleEntry::new_avc3(config);
        let bytes = entry.to_bytes();
        assert_eq!(&bytes[4..8], b"avc3");
    }

    #[test]
    fn test_hvc1_serialize() {
        let config = make_hevc_config();
        let entry = HEVCSampleEntry::new_hvc1(config);
        let bytes = entry.to_bytes();
        assert_eq!(&bytes[4..8], b"hvc1");
        assert!(bytes.windows(4).any(|w| w == b"hvcC"));
    }

    #[test]
    fn test_hev1_serialize() {
        let config = make_hevc_config();
        let entry = HEVCSampleEntry::new_hev1(config);
        let bytes = entry.to_bytes();
        assert_eq!(&bytes[4..8], b"hev1");
    }
}
