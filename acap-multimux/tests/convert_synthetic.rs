//! Synthetic-H.264 conversion gate for `acap_multimux::convert` (Task 2 of the
//! acap-multimux plan, docs/superpowers/plans/2026-07-16-acap-multimux.md).
//!
//! Hand-builds a minimal Annex B H.264 access unit (SPS + PPS + IDR slice,
//! each prefixed with a 4-byte start code) and drives the pure `convert` API
//! against it — no `vdo` dependency, host-buildable.
//!
//! H.265 coverage here is limited to [`extract_param_sets`] against a
//! hand-built minimal VPS/SPS/PPS/IDR access unit: the NAL header bytes are
//! real (ITU-T H.265 §7.3.1.2), but the VPS/SPS/PPS *payloads* are placeholder
//! bytes, not a decodable `profile_tier_level()` — HEVC's profile/tier/level
//! fields are bit-packed (not byte-positioned like AVC's `profile_idc`), so a
//! conformant `hvcC` requires a real camera SPS to decode. `track_spec` for
//! H.265 is exercised against a real capture in the Task 7 hardware verify.

use acap_multimux::convert::{Codec, au_to_sample, duration_ticks, extract_param_sets, track_spec};

/// Real minimal H.264 SPS bytes (profile 0x42 = Baseline, level 0x1E = 3.0),
/// the same known-good bytes transmux's `rtp_sdp` round-trip test uses
/// (transmux/src/rtp_sdp.rs `sprop_round_trips_sps_pps_and_profile`). NAL
/// header byte 0x67 = nal_ref_idc 3 | nal_unit_type 7 (SPS).
const SPS: [u8; 5] = [0x67, 0x42, 0xC0, 0x1E, 0xAB];
/// Real minimal H.264 PPS bytes (same fixture as above). NAL header byte
/// 0x68 = nal_ref_idc 3 | nal_unit_type 8 (PPS).
const PPS: [u8; 4] = [0x68, 0xCE, 0x3C, 0x80];
/// A tiny placeholder IDR slice NAL (type 5) — payload bytes are irrelevant to
/// `convert`, which treats sample data as opaque.
const IDR: [u8; 3] = [0x65, 0x88, 0x99];

/// 4-byte Annex B start code (ITU-T H.264 Annex B / ISO/IEC 14496-15 §5.3.4).
const START_CODE: [u8; 4] = [0x00, 0x00, 0x00, 0x01];

fn h264_au() -> Vec<u8> {
    let mut au = Vec::new();
    au.extend_from_slice(&START_CODE);
    au.extend_from_slice(&SPS);
    au.extend_from_slice(&START_CODE);
    au.extend_from_slice(&PPS);
    au.extend_from_slice(&START_CODE);
    au.extend_from_slice(&IDR);
    au
}

#[test]
fn extract_param_sets_finds_h264_sps_and_pps() {
    let au = h264_au();
    let params = extract_param_sets(Codec::H264, &au).expect("SPS+PPS present");
    assert_eq!(params.sps, SPS);
    assert_eq!(params.pps, PPS);
    assert!(params.vps.is_none(), "H.264 has no VPS");
}

#[test]
fn extract_param_sets_none_without_both_sets() {
    // SPS only, no PPS.
    let mut au = Vec::new();
    au.extend_from_slice(&START_CODE);
    au.extend_from_slice(&SPS);
    au.extend_from_slice(&START_CODE);
    au.extend_from_slice(&IDR);
    assert!(extract_param_sets(Codec::H264, &au).is_none());
}

#[test]
fn track_spec_h264_has_timescale_and_non_empty_avcc() {
    let au = h264_au();
    let params = extract_param_sets(Codec::H264, &au).expect("SPS+PPS present");
    let spec = track_spec(Codec::H264, &params, 1, 90_000).expect("valid H.264 param sets");

    assert_eq!(spec.timescale, 90_000);
    assert_eq!(spec.track_id, 1);
    match spec.config {
        transmux::pipeline::CodecConfig::Avc { config, .. } => {
            assert_eq!(config.config.sps.len(), 1, "avcC must carry the SPS");
            assert_eq!(config.config.pps.len(), 1, "avcC must carry the PPS");
            assert_eq!(config.config.sps[0].0, SPS);
            assert_eq!(config.config.pps[0].0, PPS);
            assert_eq!(config.config.profile_indication, 0x42);
            assert_eq!(config.config.level_indication, 0x1E);
        }
        other => panic!("expected CodecConfig::Avc, got {other:?}"),
    }
}

#[test]
fn au_to_sample_h264_carries_duration_sync_and_data() {
    let au = h264_au();
    let sample = au_to_sample(Codec::H264, &au, 3000, true);

    assert_eq!(sample.duration, 3000);
    assert!(sample.is_sync);
    assert!(!sample.data.is_empty());
    // The Annex B start codes are stripped in favour of 4-byte length
    // prefixes (transmux::annexb::annexb_to_length_prefixed); the SPS bytes
    // must still be present verbatim just after their length prefix.
    assert!(sample.data.windows(SPS.len()).any(|w| w == SPS));
}

#[test]
fn duration_ticks_converts_micros_delta_to_90khz_ticks() {
    // 33.333 ms @ 90 kHz ~= 3000 ticks (integer division truncates slightly).
    let ticks = duration_ticks(1_000_000, 1_033_333, 90_000);
    assert!(
        (2999..=3000).contains(&ticks),
        "expected ~3000 ticks, got {ticks}"
    );
}

#[test]
fn duration_ticks_saturates_on_backwards_timestamps() {
    // A timestamp that goes backwards must not panic or wrap; delta clamps to 0.
    assert_eq!(duration_ticks(1_000_000, 999_000, 90_000), 0);
}

// --- H.265 (VPS/SPS/PPS extraction only; see module doc) -------------------

/// H.265 NAL header bytes are real (ITU-T H.265 §7.3.1.2): byte0 carries
/// `nal_unit_type` in bits `[6:1]` (`(byte0 >> 1) & 0x3F`), byte1 carries
/// `nuh_layer_id`/`nuh_temporal_id_plus1`. VPS=32, SPS=33, PPS=34 (Table 7-1).
const H265_VPS_HEADER: [u8; 2] = [0x40, 0x01]; // type 32 << 1
const H265_SPS_HEADER: [u8; 2] = [0x42, 0x01]; // type 33 << 1
const H265_PPS_HEADER: [u8; 2] = [0x44, 0x01]; // type 34 << 1
const H265_IDR_HEADER: [u8; 2] = [0x26, 0x01]; // type 19 (IDR_W_RADL) << 1

fn h265_au() -> Vec<u8> {
    let mut au = Vec::new();
    au.extend_from_slice(&START_CODE);
    au.extend_from_slice(&H265_VPS_HEADER);
    au.extend_from_slice(&[0x0C, 0x01, 0xFF]);
    au.extend_from_slice(&START_CODE);
    au.extend_from_slice(&H265_SPS_HEADER);
    au.extend_from_slice(&[0x01, 0x02, 0x03, 0x04]);
    au.extend_from_slice(&START_CODE);
    au.extend_from_slice(&H265_PPS_HEADER);
    au.extend_from_slice(&[0xAA, 0xBB]);
    au.extend_from_slice(&START_CODE);
    au.extend_from_slice(&H265_IDR_HEADER);
    au.extend_from_slice(&[0x99, 0x99]);
    au
}

#[test]
fn extract_param_sets_finds_h265_vps_sps_pps() {
    let au = h265_au();
    let params = extract_param_sets(Codec::H265, &au).expect("VPS+SPS+PPS present");

    assert_eq!(params.sps[..2], H265_SPS_HEADER);
    assert_eq!(params.pps[..2], H265_PPS_HEADER);
    let vps = params.vps.expect("H.265 must carry a VPS");
    assert_eq!(vps[..2], H265_VPS_HEADER);
}

#[test]
fn extract_param_sets_h265_none_without_vps() {
    // SPS + PPS only, no VPS.
    let mut au = Vec::new();
    au.extend_from_slice(&START_CODE);
    au.extend_from_slice(&H265_SPS_HEADER);
    au.extend_from_slice(&[0x01, 0x02, 0x03, 0x04]);
    au.extend_from_slice(&START_CODE);
    au.extend_from_slice(&H265_PPS_HEADER);
    au.extend_from_slice(&[0xAA, 0xBB]);
    assert!(extract_param_sets(Codec::H265, &au).is_none());
}
