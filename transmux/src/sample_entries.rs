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
use alloc::vec::Vec;
use broadcast_common::Serialize;

/// Length of the VisualSampleEntry fixed fields (ISO/IEC 14496-12:2015 §12.1.3).
/// 6+2(data_ref_index)+8+2(width)+2(height)+4(horiz)+4(vert)+4(reserved)+2(frame_count)+
/// 32(compressorname)+2(depth)+2(predefined) = 78
const VISUAL_SAMPLE_ENTRY_SIZE: usize = 78;

// ---------------------------------------------------------------------------
// VisualSampleEntry fields (common to all visual sample entries)
// ---------------------------------------------------------------------------

/// Fixed fields shared by all visual sample entries, per ISO/IEC 14496-12:2015 §12.1.3.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct VisualSampleEntryFields {
    /// Data reference index (SampleEntry field at body offset 6-7).
    pub data_reference_index: u16,
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
            data_reference_index: 1,
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

        // 6 bytes reserved (zero) — SampleEntry part
        cursor += 6;
        // data_reference_index (16)
        buf[cursor..cursor + 2].copy_from_slice(&self.data_reference_index.to_be_bytes());
        cursor += 2;
        // pre_defined(16) + reserved(16) + pre_defined[3]×32 = 16 bytes (zero) — §12.1.3
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

/// An opaque box captured verbatim for round-trip.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct OpaqueBox {
    pub box_type: [u8; 4],
    pub data: Vec<u8>,
}

impl OpaqueBox {
    fn serialized_len(&self) -> usize {
        8 + self.data.len()
    }
    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        let need = self.serialized_len();
        if buf.len() < need {
            return Err(Error::OutputBufferTooSmall {
                need,
                have: buf.len(),
            });
        }
        buf[..4].copy_from_slice(&(need as u32).to_be_bytes());
        buf[4..8].copy_from_slice(&self.box_type);
        buf[8..8 + self.data.len()].copy_from_slice(&self.data);
        Ok(need)
    }
}

/// Find a config box (e.g. `avcC`) inside the sample entry's config region.
/// Returns the full box bytes (including 8-byte header).
fn find_config_box<'a>(region: &'a [u8], fourcc: &[u8; 4]) -> Option<&'a [u8]> {
    let mut off = 0usize;
    while off + 8 <= region.len() {
        let size = u32::from_be_bytes([
            region[off],
            region[off + 1],
            region[off + 2],
            region[off + 3],
        ]) as usize;
        if size < 8 {
            break;
        }
        let ty = &region[off + 4..off + 8];
        if ty == fourcc {
            return Some(&region[off..off + size]);
        }
        off += size;
    }
    None
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
    /// Additional config boxes (e.g. `pasp`, `btrt`) preserved verbatim.
    pub extra_boxes: Vec<OpaqueBox>,
}

impl AVCSampleEntry {
    /// Parse an AVC sample entry from full box bytes (including 8-byte header).
    pub fn bare_parse(bytes: &[u8]) -> Result<Self> {
        use crate::avc_config::AVCConfigurationBox;
        if bytes.len() < 8 + 78 + 8 {
            return Err(Error::BufferTooShort {
                need: 8 + 80 + 8,
                have: bytes.len(),
                what: "avc1 bare",
            });
        }
        let codec_type = [bytes[4], bytes[5], bytes[6], bytes[7]];
        let body = &bytes[8..];
        // SampleEntry: reserved(6) + data_reference_index(2) = body[0..7]
        let data_reference_index = u16::from_be_bytes([body[6], body[7]]);
        // VisualSampleEntry (§12.1.3): 16 reserved bytes (pre_defined(16) +
        // reserved(16) + pre_defined[3]) precede width → width at body[24].
        let width = u16::from_be_bytes([body[24], body[25]]);
        let height = u16::from_be_bytes([body[26], body[27]]);
        let horizontal_resolution = u32::from_be_bytes([body[28], body[29], body[30], body[31]]);
        let vertical_resolution = u32::from_be_bytes([body[32], body[33], body[34], body[35]]);
        let data_size = u32::from_be_bytes([body[36], body[37], body[38], body[39]]);
        let frame_count = u16::from_be_bytes([body[40], body[41]]);
        let mut compressorname = [0u8; 32];
        compressorname.copy_from_slice(&body[42..74]);
        let depth = u16::from_be_bytes([body[74], body[75]]);
        let predefined = u16::from_be_bytes([body[76], body[77]]);
        let visual = VisualSampleEntryFields {
            data_reference_index,
            width,
            height,
            horizontal_resolution,
            vertical_resolution,
            data_size,
            frame_count,
            compressorname,
            depth,
            predefined,
        };
        // Find avcC in the config region (starting at body[78] since visual fixed = 78 bytes)
        let config_region = &body[78..];
        if let Some(avcc) = find_config_box(config_region, b"avcC") {
            // avcc is the full box (header+body); parse_body expects only the body
            let config = if avcc.len() > 8 {
                AVCConfigurationBox::parse_body(&avcc[8..])?
            } else {
                return Err(Error::BufferTooShort {
                    need: 8,
                    have: avcc.len(),
                    what: "avcC body",
                });
            };
            // Capture extra boxes after avcC (e.g. pasp, btrt)
            let avcc_start = avcc.as_ptr() as usize - config_region.as_ptr() as usize;
            let avcc_end = avcc_start + avcc.len();
            let mut extra_boxes = Vec::new();
            let mut eb_off = avcc_end;
            while eb_off + 8 <= config_region.len() {
                let eb_sz = u32::from_be_bytes([
                    config_region[eb_off],
                    config_region[eb_off + 1],
                    config_region[eb_off + 2],
                    config_region[eb_off + 3],
                ]) as usize;
                if eb_sz < 8 {
                    break;
                }
                let bt = [
                    config_region[eb_off + 4],
                    config_region[eb_off + 5],
                    config_region[eb_off + 6],
                    config_region[eb_off + 7],
                ];
                let d = config_region[eb_off + 8..eb_off + eb_sz.min(config_region.len() - eb_off)]
                    .to_vec();
                extra_boxes.push(OpaqueBox {
                    box_type: bt,
                    data: d,
                });
                eb_off += eb_sz;
            }
            Ok(Self {
                codec_type,
                visual,
                config,
                extra_boxes,
            })
        } else {
            Err(Error::BufferTooShort {
                need: 0,
                have: 0,
                what: "avc1 missing avcC",
            })
        }
    }

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
            extra_boxes: Vec::new(),
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
        let mut n = 8 + VisualSampleEntryFields::serialized_len() + self.config.serialized_len();
        for eb in &self.extra_boxes {
            n += eb.serialized_len();
        }
        n
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
        for eb in &self.extra_boxes {
            cursor += eb.serialize_into(&mut buf[cursor..])?;
        }
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
    /// Additional config boxes preserved verbatim.
    pub extra_boxes: Vec<OpaqueBox>,
}

impl HEVCSampleEntry {
    /// Parse an HEVC sample entry from full box bytes (including 8-byte header).
    pub fn bare_parse(bytes: &[u8]) -> Result<Self> {
        use crate::hevc_config::HEVCConfigurationBox;
        if bytes.len() < 8 + 78 + 8 {
            return Err(Error::BufferTooShort {
                need: 8 + 80 + 8,
                have: bytes.len(),
                what: "hvc1 bare",
            });
        }
        let codec_type = [bytes[4], bytes[5], bytes[6], bytes[7]];
        let body = &bytes[8..];
        let data_reference_index = u16::from_be_bytes([body[6], body[7]]);
        let width = u16::from_be_bytes([body[16], body[17]]);
        let height = u16::from_be_bytes([body[18], body[19]]);
        let horizontal_resolution = u32::from_be_bytes([body[20], body[21], body[22], body[23]]);
        let vertical_resolution = u32::from_be_bytes([body[24], body[25], body[26], body[27]]);
        let data_size = u32::from_be_bytes([body[28], body[29], body[30], body[31]]);
        let frame_count = u16::from_be_bytes([body[32], body[33]]);
        let mut compressorname = [0u8; 32];
        compressorname.copy_from_slice(&body[34..66]);
        let depth = u16::from_be_bytes([body[72], body[73]]);
        let predefined = u16::from_be_bytes([body[74], body[75]]);
        let visual = VisualSampleEntryFields {
            data_reference_index,
            width,
            height,
            horizontal_resolution,
            vertical_resolution,
            data_size,
            frame_count,
            compressorname,
            depth,
            predefined,
        };
        let config_region = &body[78..];
        if let Some(hvcc) = find_config_box(config_region, b"hvcC") {
            let config = if hvcc.len() > 8 {
                HEVCConfigurationBox::parse_body(&hvcc[8..])?
            } else {
                return Err(Error::BufferTooShort {
                    need: 8,
                    have: hvcc.len(),
                    what: "hvcC body",
                });
            };
            Ok(Self {
                codec_type,
                visual,
                config,
                extra_boxes: Vec::new(),
            })
        } else {
            Err(Error::BufferTooShort {
                need: 0,
                have: 0,
                what: "hvc1 missing hvcC",
            })
        }
    }

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
            extra_boxes: Vec::new(),
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
        let mut n = 8 + VisualSampleEntryFields::serialized_len() + self.config.serialized_len();
        for eb in &self.extra_boxes {
            n += eb.serialized_len();
        }
        n
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
        for eb in &self.extra_boxes {
            cursor += eb.serialize_into(&mut buf[cursor..])?;
        }
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
