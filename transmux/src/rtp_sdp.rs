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
use crate::nal::{NalCodec, nal_unit_type};
use crate::nalu_types::{AvcPps, AvcSps};
use crate::rtp::base64_decode;
use alloc::vec::Vec;

/// Length prefix size transmux uses for coded NALs (4-byte).
const NAL_LENGTH_SIZE_MINUS_ONE: u8 = 3;
/// H.264 `nal_unit_type` for a sequence parameter set (SPS).
const AVC_NAL_SPS: u8 = 7;
/// H.264 `nal_unit_type` for a picture parameter set (PPS).
const AVC_NAL_PPS: u8 = 8;

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
        match nal_unit_type(NalCodec::Avc, &nal) {
            Some(AVC_NAL_SPS) => sps.push(AvcSps(nal)),
            Some(AVC_NAL_PPS) => pps.push(AvcPps(nal)),
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

    #[test]
    fn sprop_rejects_invalid_base64() {
        // Invalid base64 token → returns Err, not panic.
        let sprop = "not-valid-base64!!!";
        assert!(avc_config_from_sprop(sprop).is_err());
    }

    #[test]
    fn sprop_rejects_sps_too_short() {
        // An SPS shorter than 4 bytes (need bytes [1..4] for profile/level).
        let sps = alloc::vec![0x67u8, 0x42]; // type 7 (SPS), but only 2 bytes
        let sprop = base64_encode(&sps);
        let result = avc_config_from_sprop(&sprop);
        assert!(result.is_err());
        // Verify it's a BufferTooShort error.
        if let Err(Error::BufferTooShort { need, have, what }) = result {
            assert_eq!(need, 4);
            assert_eq!(have, 2);
            assert!(what.contains("SPS"));
        }
    }

    #[test]
    fn sprop_rejects_non_sps_non_pps_nal() {
        // A non-SPS/non-PPS NAL (e.g., SEI type 6) → returns Err.
        let sei = alloc::vec![0x06u8, 0x00]; // type 6 (SEI), not SPS or PPS
        let sprop = base64_encode(&sei);
        let result = avc_config_from_sprop(&sprop);
        assert!(result.is_err());
    }
}
