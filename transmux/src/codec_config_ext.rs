//! Codec-metadata extension methods over the moved configuration records.
//!
//! The records themselves are pure box syntax; formatting their RFC 6381 codec
//! strings and reading their SPS-derived geometry needs the bitstream parsers
//! (and, for `esds`, the AAC AudioSpecificConfig parser), which stay in
//! transmux.

use crate::aac_asc::AudioSpecificConfig;
use crate::error::Result;
use crate::hevc_config::HEVCDecoderConfigurationRecord;
use crate::mp4esds::EsdsBox;
use crate::sps::rfc6381_vvc1;
use crate::vvc_config::VvcDecoderConfigurationRecord;
use alloc::string::String;
use broadcast_common::Parse;

/// `pcmSampleDepth` is `bit_depth_minus8 + 8` — ISO/IEC 14496-15:2017 §E.3.
const BIT_DEPTH_OFFSET: u8 = 8;

/// RFC 6381 codec string for an `hvcC` record.
pub trait HevcRecordCodecInfo {
    /// RFC 6381 codec string (`hvc1.A.B.C.constraints`) built from the
    /// profile/tier/level fields carried in this record — ISO/IEC 14496-15:2017
    /// §E.3.
    fn rfc6381(&self) -> String;
}

impl HevcRecordCodecInfo for HEVCDecoderConfigurationRecord {
    fn rfc6381(&self) -> String {
        crate::sps::rfc6381_hvc1(&crate::sps::HevcSpsInfo {
            general_profile_space: self.general_profile_space,
            general_tier_flag: self.general_tier_flag,
            general_profile_idc: self.general_profile_idc,
            general_profile_compatibility_flags: self.general_profile_compatibility_flags,
            general_constraint_indicator_flags: self.general_constraint_indicator_flags,
            general_level_idc: self.general_level_idc,
            chroma_format_idc: self.chroma_format_idc,
            bit_depth_luma: self.bit_depth_luma_minus8 + BIT_DEPTH_OFFSET,
            bit_depth_chroma: self.bit_depth_chroma_minus8 + BIT_DEPTH_OFFSET,
            width: 0,
            height: 0,
            num_units_in_tick: None,
            time_scale: None,
            fps: None,
        })
    }
}

/// Coded dimensions and RFC 6381 codec string for a `vvcC` record.
pub trait VvcRecordCodecInfo {
    /// Coded dimensions from the first SPS, if one is present and decodable.
    fn dimensions(&self) -> Option<(u32, u32)>;

    /// RFC 6381 codec string (`vvc1.…`) built from the profile/tier/level
    /// fields carried in this record.
    fn rfc6381(&self) -> String;
}

impl VvcRecordCodecInfo for VvcDecoderConfigurationRecord {
    fn dimensions(&self) -> Option<(u32, u32)> {
        let sps = self.sps()?;
        let info = crate::sps::decode_vvc_sps(sps).ok()?;
        Some((info.width, info.height))
    }

    fn rfc6381(&self) -> String {
        match &self.ptl {
            Some(ptl) => rfc6381_vvc1(
                ptl.general_profile_idc,
                ptl.general_tier_flag,
                ptl.general_level_idc,
                &ptl.general_constraint_info,
                ptl.num_bytes_constraint_info,
            ),
            // No PTL record: emit the bare sample-entry FourCC (RFC 6381 §3.3).
            None => String::from("vvc1"),
        }
    }
}

/// AudioSpecificConfig lookup for an `esds` box.
pub trait EsdsBoxCodecInfo {
    /// Parse the `AudioSpecificConfig` carried in the `DecoderSpecificInfo`.
    fn audio_specific_config(&self) -> Result<AudioSpecificConfig>;
}

impl EsdsBoxCodecInfo for EsdsBox {
    fn audio_specific_config(&self) -> Result<AudioSpecificConfig> {
        AudioSpecificConfig::parse(self.require_decoder_specific_info()?)
    }
}
