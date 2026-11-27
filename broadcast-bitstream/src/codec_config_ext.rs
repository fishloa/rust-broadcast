//! Codec-metadata extension methods over the ISOBMFF configuration records
//! (`isobmff` feature), and the parameter-set builder behind an `avcC`.
//!
//! The records themselves are pure box syntax; formatting their RFC 6381 codec
//! strings and reading their SPS-derived geometry needs this crate's bitstream
//! parsers. [`avc_config_from_sps_pps`] is the value-level building block an
//! SDP `sprop-parameter-sets` interpreter, or any caller holding parameter-set
//! NAL units, uses to produce the record.

use crate::annexb::NAL_LENGTH_SIZE_MINUS_ONE;
use crate::error::{Error, Result};
#[cfg(feature = "hevc")]
use crate::rfc6381::rfc6381_hvc1;
#[cfg(feature = "vvc")]
use crate::rfc6381::rfc6381_vvc1;
#[cfg(feature = "hevc")]
use crate::sps::HevcSpsInfo;
use crate::sps::decode_avc_sps;
#[cfg(any(feature = "hevc", feature = "vvc"))]
use alloc::string::String;
use alloc::vec::Vec;
use broadcast_core::len::need_in;
use isobmff::avc_config::{AVCConfigurationBox, AVCDecoderConfigurationRecord};
#[cfg(feature = "hevc")]
use isobmff::hevc_config::HEVCDecoderConfigurationRecord;
use isobmff::nalu_types::{AvcPps, AvcSps};
#[cfg(feature = "vvc")]
use isobmff::vvc_config::VvcDecoderConfigurationRecord;

/// `pcmSampleDepth` is `bit_depth_minus8 + 8` — ISO/IEC 14496-15:2017 §E.3.
#[cfg(feature = "hevc")]
const BIT_DEPTH_OFFSET: u8 = 8;

/// Byte length of an SPS's `profile_idc`/`constraint_flags`/`level_idc` triple,
/// including the 1-byte NAL header (ISO/IEC 14496-10 §7.3.2.1.1).
const SPS_PROFILE_LEVEL_LEN: usize = 4;
/// Offset of `profile_idc` after the NAL header.
const SPS_PROFILE_IDC_OFFSET: usize = 1;
/// Offset of `constraint_set0..5_flag` + reserved bits after the NAL header.
const SPS_PROFILE_COMPATIBILITY_OFFSET: usize = 2;
/// Offset of `level_idc` after the NAL header.
const SPS_LEVEL_IDC_OFFSET: usize = 3;
/// `bit_depth_luma_minus8` is `bit_depth_luma - 8` (H.264 §7.3.2.1.1).
const BIT_DEPTH_LUMA_BIAS: u8 = 8;
/// `bit_depth_chroma_minus8` is `bit_depth_chroma - 8` (H.264 §7.3.2.1.1).
const BIT_DEPTH_CHROMA_BIAS: u8 = 8;

/// Build an `avcC` configuration box from already-classified SPS/PPS NAL
/// units (the value-level building block behind an SDP
/// `sprop-parameter-sets` interpreter).
///
/// `profile_indication`/`profile_compatibility`/`level_indication` are taken
/// from the first SPS's bytes `[1..4]` (after the NAL header) — at least one
/// SPS is required.
///
/// For a High profile SPS the record's `chroma_format`/
/// `bit_depth_luma_minus8`/`bit_depth_chroma_minus8` trailer is filled in
/// from the SPS itself: ISO/IEC 14496-15:2017 §5.3.3.1.2 makes those fields
/// **conditionally present** on `AVCProfileIndication ∈ {100, 110, 122, 244}`,
/// so an `avcC` built for a High profile stream without them is a
/// configuration record that a strict reader parses differently from the wire
/// (and an encoder's own SPS carries the values — [`decode_avc_sps`] already
/// decodes them). A non-High profile keeps `None` fields (the trailer is
/// absent from the wire).
pub fn avc_config_from_sps_pps(sps: Vec<AvcSps>, pps: Vec<AvcPps>) -> Result<AVCConfigurationBox> {
    let first_sps = sps
        .first()
        .ok_or(Error::InvalidInput("no SPS NAL unit supplied"))?;
    need_in(
        &first_sps.0,
        SPS_PROFILE_LEVEL_LEN,
        "SPS profile/level bytes",
    )?;
    let profile_indication = first_sps.0[SPS_PROFILE_IDC_OFFSET];
    // Gate on the serializer's own emission set (the ISO/IEC 14496-15:2017
    // §5.3.3.1.2 condition for `profile_idc` 100/110/122/244), reached through
    // the shared `sps::is_high_profile` source of truth so the record and the
    // wire can never disagree about the trailer's presence. The SPS is decoded
    // rather than ignored on failure: it *is* the only place those three
    // values exist, so a decode error means the trailer cannot be written, and
    // writing the record without it would produce a configuration that a
    // strict reader parses differently from the stream (the defect the gate
    // exists to prevent). `is_high_profile` is the wider H.264 Table A-1 list
    // that governs *parsing* the SPS syntax; the trailer's own condition is
    // the narrower §5.3.3.1.2 set, so a profile in the wider list but outside
    // the trailer set keeps `None` without decoding anything.
    let ext = if AVCDecoderConfigurationRecord::has_high_profile_ext(profile_indication) {
        Some(decode_avc_sps(&first_sps.0)?)
    } else {
        None
    };
    let record = AVCDecoderConfigurationRecord {
        configuration_version: 1,
        profile_indication,
        profile_compatibility: first_sps.0[SPS_PROFILE_COMPATIBILITY_OFFSET],
        level_indication: first_sps.0[SPS_LEVEL_IDC_OFFSET],
        length_size_minus_one: NAL_LENGTH_SIZE_MINUS_ONE,
        sps,
        pps,
        chroma_format: ext.as_ref().map(|i| i.chroma_format_idc()),
        bit_depth_luma_minus8: ext
            .as_ref()
            .map(|i| i.bit_depth_luma().saturating_sub(BIT_DEPTH_LUMA_BIAS)),
        bit_depth_chroma_minus8: ext
            .as_ref()
            .map(|i| i.bit_depth_chroma().saturating_sub(BIT_DEPTH_CHROMA_BIAS)),
        sps_ext: Vec::new(),
    };
    Ok(AVCConfigurationBox::new(record))
}

/// RFC 6381 codec string for an `hvcC` record.
#[cfg(feature = "hevc")]
pub trait HevcRecordCodecInfo {
    /// RFC 6381 codec string (`hvc1.A.B.C.constraints`) built from the
    /// profile/tier/level fields carried in this record — ISO/IEC 14496-15:2017
    /// §E.3.
    fn rfc6381(&self) -> String;
}

#[cfg(feature = "hevc")]
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
#[cfg(feature = "vvc")]
pub trait VvcRecordCodecInfo {
    /// Coded dimensions from the first SPS, if one is present and decodable.
    fn dimensions(&self) -> Option<(u32, u32)>;

    /// RFC 6381 codec string (`vvc1.…`) built from the profile/tier/level
    /// fields carried in this record.
    fn rfc6381(&self) -> String;
}

#[cfg(feature = "vvc")]
impl VvcRecordCodecInfo for VvcDecoderConfigurationRecord {
    fn dimensions(&self) -> Option<(u32, u32)> {
        let sps = self.sps()?;
        let info = crate::sps::decode_vvc_sps(sps).ok()?;
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

#[cfg(test)]
mod tests {
    use super::*;
    #[cfg(feature = "vvc")]
    use broadcast_core::Parse;
    use isobmff::nalu_types::{AvcPps, AvcSps};

    #[cfg(feature = "vvc")]
    fn fixture_vvcc_body() -> Vec<u8> {
        let hex = "00000000ff00655f010220800000014000f00000028f00010044\
007900ab02208000008028203c46a00527ffac2136563040827004a1164883521e8\
f56a4bc97a89422c81168412421c43425c8e54330463c50000003001000000300c1\
88900001000c0081000014101e22241f4100";
        test_fixtures::hex_bytes(hex)
    }

    #[cfg(feature = "vvc")]
    #[test]
    fn vvcc_dimensions_come_from_the_first_sps() {
        let body = fixture_vvcc_body();
        let record = VvcDecoderConfigurationRecord::parse(&body[4..]).unwrap();
        assert_eq!(record.dimensions(), Some((320, 240)));
    }

    #[cfg(feature = "vvc")]
    #[test]
    fn vvcc_rfc6381_cta_suffix_prints_the_constraint_payload() {
        let payload = [
            0x00u8, 0x11, 0x22, 0x33, 0x44, 0x55, 0x66, 0x77, 0x88, 0x99, 0xAA, 0xFC,
        ];
        assert_eq!(
            rfc6381_vvc1(1, false, 32, &payload, 12),
            "vvc1.1.L32.CTA-00112233445566778899AAFC"
        );
    }

    #[cfg(feature = "vvc")]
    #[test]
    fn vvcc_record_rfc6381_uses_the_ptl_record() {
        let body = fixture_vvcc_body();
        let record = VvcDecoderConfigurationRecord::parse(&body[4..]).unwrap();
        assert!(record.rfc6381().starts_with("vvc1."));

        let mut without_ptl = record.clone();
        without_ptl.ptl = None;
        assert_eq!(without_ptl.rfc6381(), "vvc1");
    }
    /// A High profile SPS whose body cannot be decoded must be an **error**,
    /// not a record silently missing §5.3.3.1.2's trailer: the trailer's three
    /// values exist only in the SPS, and a record written without them is one
    /// a strict reader parses differently from the stream.
    #[test]
    fn high_profile_sps_that_cannot_be_decoded_is_an_error() {
        // A High profile SPS (profile_idc 100) truncated immediately after its
        // profile/level bytes: everything the *record* copies is present, so
        // the record could be built — but the chroma/bit-depth fields after it
        // cannot be read.
        let sps = alloc::vec![0x67u8, 100, 0x00, 0x1E];
        let pps = alloc::vec![0x68u8, 0xCE, 0x3C, 0x80];
        let err = avc_config_from_sps_pps(alloc::vec![AvcSps(sps)], alloc::vec![AvcPps(pps)])
            .expect_err("a High profile SPS with no decodable body must not yield a record");
        assert!(
            matches!(err, Error::BufferTooShort(..) | Error::InvalidValue(..)),
            "must be a structured decode error, got {err:?}"
        );
    }

    #[test]
    fn a_sps_is_required() {
        assert!(matches!(
            avc_config_from_sps_pps(Vec::new(), Vec::new()),
            Err(Error::InvalidInput(..))
        ));
    }
}
