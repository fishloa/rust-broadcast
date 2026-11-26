//! Codec-metadata extension methods over the moved configuration records.
//!
//! The records themselves are pure box syntax; formatting their RFC 6381 codec
//! strings and reading their SPS-derived geometry needs the bitstream parsers
//! (and, for `esds`, the AAC AudioSpecificConfig parser), which stay in
//! transmux.

use crate::aac_asc::AudioSpecificConfig;
use crate::error::Result;
use alloc::string::String;
use broadcast_bitstream::rfc6381::{rfc6381_hvc1, rfc6381_vvc1};
use broadcast_bitstream::sps::HevcSpsInfo;
use broadcast_core::Parse;
use isobmff::hevc_config::HEVCDecoderConfigurationRecord;
use isobmff::mp4esds::EsdsBox;
use isobmff::vvc_config::VvcDecoderConfigurationRecord;

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
        rfc6381_hvc1(&HevcSpsInfo::from_record(
            self.general_profile_space,
            self.general_tier_flag,
            self.general_profile_idc,
            self.general_profile_compatibility_flags,
            self.general_constraint_indicator_flags,
            self.general_level_idc,
            self.chroma_format_idc,
            self.bit_depth_luma_minus8 + BIT_DEPTH_OFFSET,
            self.bit_depth_chroma_minus8 + BIT_DEPTH_OFFSET,
        ))
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
        let info = broadcast_bitstream::sps::decode_vvc_sps(sps).ok()?;
        Some((info.width(), info.height()))
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

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::vec;

    fn fixture_vvcc_body() -> Vec<u8> {
        let hex = "00000000ff00655f010220800000014000f00000028f00010044\
007900ab02208000008028203c46a00527ffac2136563040827004a1164883521e8\
f56a4bc97a89422c81168412421c43425c8e54330463c50000003001000000300c1\
88900001000c0081000014101e22241f4100";
        test_fixtures::hex_bytes(hex)
    }

    #[test]
    fn vvcc_dimensions_come_from_the_first_sps() {
        let body = fixture_vvcc_body();
        let record = VvcDecoderConfigurationRecord::parse(&body[4..]).unwrap();
        assert_eq!(record.dimensions(), Some((320, 240)));
    }

    #[test]
    fn vvcc_rfc6381_cta_suffix_prints_the_constraint_payload() {
        let payload = [
            0x00u8, 0x11, 0x22, 0x33, 0x44, 0x55, 0x66, 0x77, 0x88, 0x99, 0xAA, 0xFC,
        ];
        assert_eq!(
            broadcast_bitstream::rfc6381::rfc6381_vvc1(1, false, 32, &payload, 12),
            "vvc1.1.L32.CTA-00112233445566778899AAFC"
        );
    }

    #[test]
    fn vvcc_record_rfc6381_uses_the_ptl_record() {
        let body = fixture_vvcc_body();
        let record = VvcDecoderConfigurationRecord::parse(&body[4..]).unwrap();
        assert!(record.rfc6381().starts_with("vvc1."));

        let mut without_ptl = record.clone();
        without_ptl.ptl = None;
        assert_eq!(without_ptl.rfc6381(), "vvc1");
    }

    #[test]
    fn esds_audio_specific_config_reads_the_decoder_specific_info() {
        let esds = EsdsBox::new(isobmff::mp4esds::ESDescriptor::new(
            1,
            0,
            Some(isobmff::mp4esds::DecoderConfigDescriptor::new(
                0x40,
                5,
                false,
                0,
                0,
                0,
                Some(isobmff::mp4esds::DecoderSpecificInfo::new(vec![0x12, 0x10])),
            )),
            Some(isobmff::mp4esds::SLConfigDescriptor::predefined_two()),
        ));
        assert_eq!(esds.decoder_specific_info_data(), Some(&[0x12u8, 0x10][..]));
        assert!(esds.audio_specific_config().is_ok());
    }
}
