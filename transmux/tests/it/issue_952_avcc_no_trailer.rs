//! Regression for issue #952: `Mp4Demux` dropped an entire H.264 track
//! because `avc_config::AVCDecoderConfigurationRecord::parse` treated the
//! optional ISO/IEC 14496-15:2017 §5.3.3.1.2 high-profile trailer
//! (`chroma_format`/`bit_depth_*`/`sps_ext`) as mandatory whenever
//! `profile_indication` was in the High-profile family (100/110/122/244).
//!
//! Fixture: `fixtures/scte35/dash/v300/{init.mp4,3.m4s}` — a real
//! DASH-IF `livesim2` capture (Apache-2.0, see
//! `fixtures/scte35/PROVENANCE.md`). Its `avcC` is exactly 41 bytes,
//! profile 100 (High), and the SPS+PPS arrays consume every byte — there is
//! no trailer on the wire at all, which a conformant High-profile encoder is
//! free to do. ffmpeg reads this file without complaint; before the fix,
//! this crate's `Mp4Demux` recovered zero tracks from it.
//!
//! This test must FAIL on pre-fix code with `tracks=0, skipped=1`.

use broadcast_common::timescale::Timescale;
use broadcast_common::{Parse, Serialize};
use isobmff::avc_config::{AVCConfigurationBox, AVCDecoderConfigurationRecord};
use isobmff::nalu_types::{AvcPps, AvcSps};
use isobmff::{MovieBox, StblChild};
use transmux::TrackId;
use transmux::pipeline::{CodecConfig, TrackSpec, build_init_segment};

fn timescale(value: u32) -> Timescale {
    Timescale::try_new(value).expect("a non-zero timescale")
}

fn fixture_bytes() -> Vec<u8> {
    let mut buf = test_fixtures::read("scte35/dash/v300/init.mp4");
    buf.extend_from_slice(&test_fixtures::read("scte35/dash/v300/3.m4s"));
    buf
}

#[test]
fn trailer_less_high_profile_avcc_recovers_the_video_track() {
    let buf = fixture_bytes();
    let media =
        test_harness::collect::media_from_mp4(&buf, test_harness::collect::CollectMode::Seekable)
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
/// high-profile trailer — `Mp4Demux` must still drop only that track, but
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
        TrackId::new(1),
        timescale(90_000),
        CodecConfig::Avc {
            config: AVCConfigurationBox::new(record),
            width: 320,
            height: 240,
        },
    );
    let mut moov =
        build_init_segment(&[spec], timescale(90_000)).expect("build a valid init segment");

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

    let media =
        test_harness::collect::media_from_mp4(&moov, test_harness::collect::CollectMode::Seekable)
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

// ── The `stsd` fallback and its guard (#3465) ─────────────────────────────
//
// `parse_stbl_children` keeps a `stsd` that failed its typed parse as
// `StblChild::Opaque` bytes (issue #952). Both readers of that list — the
// track builder and the in-band probe — re-parse *only* the opaque child whose
// four-CC really is `stsd`; any other opaque child (a `free` box, `sgpd`, a
// vendor box) is somebody else's data and must not be read as a sample
// description.

const FTYP_HEADER_LEN: usize = 8;

fn avc3_spec() -> TrackSpec {
    let record = AVCDecoderConfigurationRecord {
        configuration_version: 1,
        profile_indication: 66,
        profile_compatibility: 0,
        level_indication: 30,
        length_size_minus_one: 3,
        sps: vec![AvcSps(vec![
            0x67, 0x42, 0xc0, 0x1e, 0xd9, 0x00, 0x80, 0x1e, 0x24,
        ])],
        pps: vec![AvcPps(vec![0x68, 0xce, 0x3c, 0x80])],
        sps_ext: vec![],
        chroma_format: None,
        bit_depth_luma_minus8: None,
        bit_depth_chroma_minus8: None,
    };
    TrackSpec::new(
        TrackId::new(1),
        timescale(90_000),
        CodecConfig::Avc {
            config: AVCConfigurationBox::new(record),
            width: 320,
            height: 240,
        },
    )
}

/// A `free` box of `body_len` body bytes, as it sits inside `stbl`.
fn free_box(body_len: usize) -> Vec<u8> {
    let size = u32::try_from(FTYP_HEADER_LEN + body_len).expect("a small box");
    let mut b = size.to_be_bytes().to_vec();
    b.extend_from_slice(b"free");
    b.extend(std::iter::repeat_n(0u8, body_len));
    b
}

/// The `avc3` init segment (an in-band entry) with `child` inserted into its
/// `stbl` immediately before the `stsd`.
fn init_with_stbl_child(child: Vec<u8>) -> Vec<u8> {
    let mut init = build_init_segment(&[avc3_spec()], timescale(90_000)).expect("init segment");
    let entry = init
        .windows(4)
        .position(|w| w == b"avc1")
        .expect("avc1 sample entry");
    init[entry..entry + 4].copy_from_slice(b"avc3");
    let ftyp_len = u32::from_be_bytes([init[0], init[1], init[2], init[3]]) as usize;
    let mut moov = MovieBox::parse(&init[ftyp_len..]).expect("moov parses");
    let stbl = moov.tracks[0]
        .mdia
        .as_mut()
        .and_then(|m| m.minf.as_mut())
        .and_then(|m| m.stbl.as_mut())
        .expect("stbl");
    let at = stbl
        .children
        .iter()
        .position(|c| matches!(c, StblChild::Stsd(_)))
        .expect("stsd child");
    stbl.children.insert(at, StblChild::Opaque(child));
    let mut out = init[..ftyp_len].to_vec();
    let mut buf = vec![0u8; moov.serialized_len()];
    let written = moov.serialize_into(&mut buf).expect("moov serializes");
    out.extend_from_slice(&buf[..written]);
    out
}

/// One access unit carrying an in-band SPS (type 7) as the first NAL.
fn inband_sample_segment() -> Vec<u8> {
    use transmux::ir::{FragmentTrackData, Sample};
    let sps = [0x67u8, 0x42, 0xc0, 0x1e, 0xd9, 0x00, 0x80, 0x1e, 0x24];
    let mut data = u32::try_from(sps.len()).unwrap().to_be_bytes().to_vec();
    data.extend_from_slice(&sps);
    let sample =
        Sample::new(data, Some(0), Some(0), Some(3_000), true).expect("valid sample timing");
    transmux::pipeline::build_media_segment(
        1,
        &[FragmentTrackData::new(
            TrackId::new(1),
            0,
            std::slice::from_ref(&sample),
        )],
    )
    .expect("media segment")
}

/// A `free` box ahead of the `stsd` is not a sample description: the track
/// must still be recovered, and its in-band (`avc3`) entry still recognised.
#[test]
fn an_unknown_stbl_child_before_the_stsd_does_not_hide_the_track() {
    let mut bytes = init_with_stbl_child(free_box(4));
    bytes.extend_from_slice(&inband_sample_segment());
    let media =
        test_harness::collect::media_from_mp4(&bytes, test_harness::collect::CollectMode::Seekable)
            .expect("an unknown stbl child must not fail the file");
    assert_eq!(media.skipped.len(), 0, "skipped = {:?}", media.skipped);
    assert_eq!(media.tracks.len(), 1, "the AVC track must be recovered");
    let track = &media.tracks[0];
    assert_eq!(track.samples.len(), 1);
    assert!(
        track.samples[0].display.is_some(),
        "the avc3 sample entry is in-band, so its access units must be tagged"
    );
}

/// A `stsd` whose typed parse fails names the real error for an in-band
/// (`avc3`) entry too, exactly as it does for `avc1`.
#[test]
fn a_failed_stsd_on_an_avc3_entry_reports_the_real_reason() {
    let mut moov = build_init_segment(&[avc3_spec()], timescale(90_000)).expect("init segment");
    let entry = moov
        .windows(4)
        .position(|w| w == b"avc1")
        .expect("avc1 sample entry");
    moov[entry..entry + 4].copy_from_slice(b"avc3");
    let avcc_at = moov
        .windows(4)
        .position(|w| w == b"avcC")
        .expect("avcC box");
    let sps_len_at = avcc_at + 4 + 6;
    moov[sps_len_at] = 0xFF;
    moov[sps_len_at + 1] = 0xFF;

    let media =
        test_harness::collect::media_from_mp4(&moov, test_harness::collect::CollectMode::Seekable)
            .expect("a malformed sample entry must skip its track, not fail the whole file");
    assert_eq!(media.tracks.len(), 0, "the malformed track must be skipped");
    assert_eq!(media.skipped.len(), 1);
    let reason = &media.skipped[0].reason;
    assert!(
        reason.contains("buffer too short") && reason.contains("SPS"),
        "reason must name the real cause, got {reason:?}"
    );
}

/// A container child too short to hold a box header is a framing error to
/// report, never an index out of range.
#[test]
fn a_stbl_child_shorter_than_a_box_header_does_not_panic() {
    let bytes = init_with_stbl_child(vec![0, 0, 0, 6, b'f', b'r']);
    match test_harness::collect::media_from_mp4(
        &bytes,
        test_harness::collect::CollectMode::Seekable,
    ) {
        Err(_) => {}
        Ok(media) => assert!(
            media.tracks.is_empty(),
            "a 6-byte child cannot be framed, so no track may be built: {:?}",
            media.tracks
        ),
    }
}
