//! SDP fmtp → transmux `CodecConfig` (RFC 6184 §8.1 / RFC 3640 §4.1).
//!
//! Turns the media-format parameters carried in an RTSP DESCRIBE SDP into the
//! codec configuration transmux muxers need: H.264 `sprop-parameter-sets`
//! (base64 SPS/PPS) → `avcC`, AAC `config` (hex AudioSpecificConfig) → `esds`.
//! The caller (e.g. multimux) extracts the raw fmtp attribute strings via an
//! SDP parser; this module owns only the codec-config construction, because
//! transmux owns `AVCConfigurationBox`/`EsdsBox`.

use crate::avc_config::{AVCConfigurationBox, AVCDecoderConfigurationRecord};
use crate::error::{Error, Result};
use crate::nalu_types::{AvcPps, AvcSps};
use crate::rtp::base64_decode;
use alloc::vec::Vec;

/// H.264 NAL unit type mask (RFC 6184) and the SPS/PPS type values.
const NAL_TYPE_MASK: u8 = 0x1F;
const NAL_TYPE_SPS: u8 = 7;
const NAL_TYPE_PPS: u8 = 8;
/// Length prefix size transmux uses for coded NALs (4-byte).
const NAL_LENGTH_SIZE_MINUS_ONE: u8 = 3;

/// Parse an SDP `sprop-parameter-sets` value (RFC 6184 §8.1: comma-separated
/// base64 parameter-set NAL units) into an `avcC` configuration box.
///
/// SPS units (nal_unit_type 7) supply `profile_indication` /
/// `profile_compatibility` / `level_indication` (SPS bytes `[1..4]` after the
/// NAL header). At least one SPS is required.
pub fn avc_config_from_sprop(sprop_parameter_sets: &str) -> Result<AVCConfigurationBox> {
    let mut sps: Vec<AvcSps> = Vec::new();
    let mut pps: Vec<AvcPps> = Vec::new();
    for token in sprop_parameter_sets.split(',') {
        let token = token.trim();
        if token.is_empty() {
            continue;
        }
        let nal = base64_decode(token)?;
        if nal.is_empty() {
            return Err(Error::InvalidInput("empty sprop parameter set"));
        }
        match nal[0] & NAL_TYPE_MASK {
            NAL_TYPE_SPS => sps.push(AvcSps(nal)),
            NAL_TYPE_PPS => pps.push(AvcPps(nal)),
            _ => return Err(Error::InvalidInput("sprop NAL is neither SPS nor PPS")),
        }
    }
    let first_sps = sps
        .first()
        .ok_or(Error::InvalidInput("sprop-parameter-sets contained no SPS"))?;
    if first_sps.0.len() < 4 {
        return Err(Error::BufferTooShort {
            need: 4,
            have: first_sps.0.len(),
            what: "SPS profile/level bytes",
        });
    }
    let record = AVCDecoderConfigurationRecord {
        configuration_version: 1,
        profile_indication: first_sps.0[1],
        profile_compatibility: first_sps.0[2],
        level_indication: first_sps.0[3],
        length_size_minus_one: NAL_LENGTH_SIZE_MINUS_ONE,
        sps,
        pps,
        chroma_format: None,
        bit_depth_luma_minus8: None,
        bit_depth_chroma_minus8: None,
        sps_ext: Vec::new(),
    };
    Ok(AVCConfigurationBox::new(record))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::rtp::base64_encode;

    #[test]
    fn sprop_round_trips_sps_pps_and_profile() {
        // A minimal but real SPS (type 7) and PPS (type 8).
        // SPS bytes after the NAL header byte: profile_idc, constraints, level_idc.
        let sps = alloc::vec![0x67u8, 0x42, 0xC0, 0x1E, 0xAB]; // profile 0x42, level 0x1E
        let pps = alloc::vec![0x68u8, 0xCE, 0x3C, 0x80];
        let sprop = alloc::format!("{},{}", base64_encode(&sps), base64_encode(&pps));

        let boxed = avc_config_from_sprop(&sprop).unwrap();
        let r = &boxed.config;
        assert_eq!(r.sps.len(), 1);
        assert_eq!(r.pps.len(), 1);
        assert_eq!(r.sps[0].0, sps);
        assert_eq!(r.pps[0].0, pps);
        assert_eq!(r.profile_indication, 0x42);
        assert_eq!(r.profile_compatibility, 0xC0);
        assert_eq!(r.level_indication, 0x1E);
        assert_eq!(r.length_size_minus_one, 3);
    }

    #[test]
    fn sprop_rejects_when_no_sps() {
        // Only a PPS (type 8) present → no SPS → error.
        let pps = alloc::vec![0x68u8, 0xCE];
        let sprop = base64_encode(&pps);
        assert!(avc_config_from_sprop(&sprop).is_err());
    }
}
