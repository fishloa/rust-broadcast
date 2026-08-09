//! Regression for issue #952: `Fmp4Demux` dropped an entire H.264 track
//! because `avc_config::AVCDecoderConfigurationRecord::parse` treated the
//! optional ISO/IEC 14496-15:2017 §5.3.3.1.2 high-profile trailer
//! (`chroma_format`/`bit_depth_*`/`sps_ext`) as mandatory whenever
//! `profile_indication` was in the High-profile family (100/110/122/244).
//!
//! Fixture: `fixtures/scte35-ssai/dash/V300/{init.mp4,3.m4s}` — a real
//! DASH-IF `livesim2` capture (Apache-2.0, see
//! `fixtures/scte35-ssai/PROVENANCE.md`). Its `avcC` is exactly 41 bytes,
//! profile 100 (High), and the SPS+PPS arrays consume every byte — there is
//! no trailer on the wire at all, which a conformant High-profile encoder is
//! free to do. ffmpeg reads this file without complaint; before the fix,
//! this crate's `Fmp4Demux` recovered zero tracks from it.
//!
//! This test must FAIL on pre-fix code with `tracks=0, skipped=1`.

use broadcast_common::Unpackage;
use transmux::Fmp4Demux;
use transmux::avc_config::{AVCConfigurationBox, AVCDecoderConfigurationRecord};
use transmux::nalu_types::{AvcPps, AvcSps};
use transmux::pipeline::{CodecConfig, TrackSpec, build_init_segment};

fn fixture_bytes() -> Vec<u8> {
    let init = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../fixtures/scte35-ssai/dash/V300/init.mp4"
    );
    let seg = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../fixtures/scte35-ssai/dash/V300/3.m4s"
    );
    let mut buf = std::fs::read(init).expect("fixtures/scte35-ssai/dash/V300/init.mp4 must exist");
    buf.extend_from_slice(
        &std::fs::read(seg).expect("fixtures/scte35-ssai/dash/V300/3.m4s must exist"),
    );
    buf
}

#[test]
fn trailer_less_high_profile_avcc_recovers_the_video_track() {
    let buf = fixture_bytes();
    let mut demux = Fmp4Demux::new();
    let media = demux
        .unpackage(&buf)
        .expect("a well-formed init+fragment pair must demux, not error");

    assert_eq!(
        media.skipped.len(),
        0,
        "no track should be skipped; skipped = {:?}",
        media.skipped
    );
    assert_eq!(
        media.tracks.len(),
        1,
        "the real H.264 video track must be recovered"
    );

    let track = &media.tracks[0];
    match &track.spec.config {
        CodecConfig::Avc {
            config,
            width,
            height,
        } => {
            let rec = &config.config;
            assert_eq!(rec.profile_indication, 100, "High profile per issue #952");
            assert_eq!(rec.sps.len(), 1);
            assert_eq!(rec.pps.len(), 1);
            // The real record's trailer is genuinely absent on the wire —
            // must stay `None`, never an invented default.
            assert_eq!(rec.chroma_format, None);
            assert_eq!(rec.bit_depth_luma_minus8, None);
            assert_eq!(rec.bit_depth_chroma_minus8, None);
            assert!(rec.sps_ext.is_empty());
            assert_eq!(*width, 640, "coded width from the real SPS");
            assert_eq!(*height, 360, "coded height from the real SPS");
        }
        other => panic!("expected CodecConfig::Avc, got {other:?}"),
    }

    assert!(
        !track.samples.is_empty(),
        "the recovered track must carry the fragment's samples"
    );
}

/// Defect #2 (the "worse half" per issue #952): when a `stsd` entry genuinely
/// fails to parse — for a reason that has nothing to do with the optional
/// high-profile trailer — `Fmp4Demux` must still drop only that track, but
/// the recorded [`transmux::SkippedTrack::reason`] must name the *real*
/// failure (here, the SPS NALU length overrunning the buffer), not the
/// generic "expected box: stsd entry" a blanked-out placeholder used to
/// produce. This proves `init_segment::parse_stbl_children`'s swallow is now
/// visible rather than merely worked around by the #1 fix above.
#[test]
fn genuinely_malformed_stsd_names_the_real_reason() {
    let record = AVCDecoderConfigurationRecord {
        configuration_version: 1,
        profile_indication: 66, // Baseline: no high-profile trailer in play here at all.
        profile_compatibility: 0,
        level_indication: 30,
        length_size_minus_one: 3,
        sps: vec![AvcSps(vec![0x67, 0x42, 0x00, 0x1e])],
        pps: vec![AvcPps(vec![0x68, 0xce, 0x3c, 0x80])],
        chroma_format: None,
        bit_depth_luma_minus8: None,
        bit_depth_chroma_minus8: None,
        sps_ext: vec![],
    };
    let spec = TrackSpec::new(
        1,
        90_000,
        CodecConfig::Avc {
            config: AVCConfigurationBox::new(record),
            width: 320,
            height: 240,
        },
    );
    let mut moov = build_init_segment(&[spec], 90_000).expect("build a valid init segment");

    // Corrupt the SPS NALU length field inside the `avcC` body (the 2 bytes
    // right after configurationVersion/profile/compat/level/lenSize/numSPS)
    // to a value that overruns the buffer — genuinely unparseable, unrelated
    // to the high-profile trailer.
    let avcc_at = moov
        .windows(4)
        .position(|w| w == b"avcC")
        .expect("build_init_segment must emit an avcC box");
    let sps_len_at = avcc_at + 4 + 6; // fourcc(4) + version/profile/compat/level/lenSize/numSPS(6)
    moov[sps_len_at] = 0xFF;
    moov[sps_len_at + 1] = 0xFF;

    let mut demux = Fmp4Demux::new();
    let media = demux
        .unpackage(&moov)
        .expect("a malformed sample entry must skip its track, not fail the whole file");

    assert_eq!(media.tracks.len(), 0, "the malformed track must be skipped");
    assert_eq!(media.skipped.len(), 1);
    let reason = &media.skipped[0].reason;
    assert!(
        reason.contains("buffer too short") && reason.contains("SPS"),
        "reason must name the real cause (SPS NALU overrun), not a generic \
         'expected box: stsd entry'; got {reason:?}"
    );
}
