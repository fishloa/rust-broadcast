//! Real-fixture gate for the streaming RTP depayloader (#700).
//!
//! Demuxes the real `h264_aac.ts` fixture, packetizes it to RTP (the same
//! `TsDemux` + `RtpPacketizer::package` calls `tests/rtp.rs` uses), then feeds
//! the video stream's packets through [`RtpStreamDepacketizer`] fed with the
//! codec config recovered from the *generated SDP* (exercising the P2
//! `avc_config_from_sprop`/`aac_config_from_fmtp` round-trip), and checks the
//! recovered timing/config/sync against the demuxed oracle, then builds a
//! valid fMP4 init + media segment from the recovered samples.
#![cfg(feature = "std")]

use broadcast_common::{Package, Unpackage};
use transmux::pipeline::CodecConfig;
use transmux::rtp::RtpMediaKind;
use transmux::rtp_sdp::{aac_config_from_fmtp, avc_config_from_sprop};
use transmux::{
    FragmentTrackData, Media, RtpOutput, RtpPacketizer, RtpStream, RtpStreamDepacketizer,
    RtpStreamTrack, Severity, TsDemux, build_init_segment, build_media_segment,
    validate_init_segment, validate_media_segment,
};

const MTU: usize = 1400;
const SSRC: u32 = 0x1234_5678;

// ── Step 0 plumbing, copied verbatim from tests/rtp.rs ──────────────────────

fn demux_fixture() -> Media {
    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../fixtures/ts/h264_aac.ts");
    let data = std::fs::read(path).expect("h264_aac.ts fixture must exist");
    let mut demux = TsDemux::new();
    demux.unpackage(&data[..]).expect("demux TS → IR")
}

fn packetize(media: &Media) -> RtpOutput {
    let mut p = RtpPacketizer {
        mtu: MTU,
        ssrc: SSRC,
        ..RtpPacketizer::default()
    };
    p.package(media).expect("packetize IR → RTP")
}

fn video_stream(out: &RtpOutput) -> &RtpStream {
    out.streams
        .iter()
        .find(|s| s.kind == RtpMediaKind::H264)
        .unwrap()
}

// Pull one fmtp attribute value ("sprop-parameter-sets=" / "config=") out of an
// SDP string. Test-only crude extraction (no sdp-types dependency in transmux).
fn fmtp_value<'a>(sdp: &'a str, key: &str) -> Option<&'a str> {
    for line in sdp.lines() {
        if let Some(idx) = line.find(key) {
            let rest = &line[idx + key.len()..];
            let end = rest.find([';', ' ', '\r', '\n']).unwrap_or(rest.len());
            return Some(&rest[..end]);
        }
    }
    None
}

fn errors(issues: &[transmux::ConformanceIssue]) -> Vec<&str> {
    issues
        .iter()
        .filter(|i| i.severity == Severity::Error)
        .map(|i| i.code)
        .collect()
}

#[test]
fn ts_round_trip_recovers_timing_config_and_builds_fmp4() {
    let media = demux_fixture();
    let out = packetize(&media);

    // Original per-track truth from the demuxed Media.
    let orig_video = media
        .tracks
        .iter()
        .find(|t| matches!(t.spec.config, CodecConfig::Avc { .. }))
        .expect("video track");
    let orig_video_syncs = orig_video.samples.iter().filter(|s| s.is_sync).count();
    let orig_video_total: u64 = orig_video
        .samples
        .iter()
        .map(|s| u64::from(s.duration))
        .sum();

    // Build codec config from the generated SDP (exercises P2).
    let sprop = fmtp_value(&out.sdp, "sprop-parameter-sets=").expect("sprop");
    let avc = avc_config_from_sprop(sprop).expect("avc from sprop");
    // SPS/PPS bytes recovered from SDP must equal the fixture's.
    if let CodecConfig::Avc { config, .. } = &orig_video.spec.config {
        assert_eq!(avc.config.sps.len(), config.config.sps.len());
        assert_eq!(
            avc.config.sps[0].0, config.config.sps[0].0,
            "SPS bytes round-trip"
        );
        assert_eq!(
            avc.config.pps[0].0, config.config.pps[0].0,
            "PPS bytes round-trip"
        );
    } else {
        panic!("expected video track to carry CodecConfig::Avc");
    }

    // Feed the packetized RTP for the video stream through the streaming depayloader.
    let video_stream = video_stream(&out);
    let mut d = RtpStreamDepacketizer::new(vec![RtpStreamTrack::new(
        1,
        RtpMediaKind::H264,
        CodecConfig::Avc {
            config: avc.clone(),
            width: 0,
            height: 0,
        },
        90_000,
    )]);
    let mut recovered = Vec::new();
    for pkt in &video_stream.packets {
        recovered.extend(d.push(1, pkt).unwrap());
    }
    recovered.extend(d.flush(1).unwrap());

    // Recovered sample count within 1 of the original (last-AU flush edge).
    assert!(
        (recovered.len() as i64 - orig_video.samples.len() as i64).abs() <= 1,
        "recovered {} vs original {}",
        recovered.len(),
        orig_video.samples.len()
    );
    // Sync points preserved.
    let rec_syncs = recovered.iter().filter(|s| s.is_sync).count();
    assert_eq!(rec_syncs, orig_video_syncs, "keyframe count preserved");
    // Total duration within one frame of the original (one-AU flush tolerance).
    let rec_total: u64 = recovered.iter().map(|s| u64::from(s.duration)).sum();
    let frame = orig_video
        .samples
        .first()
        .map(|s| u64::from(s.duration))
        .unwrap_or(3000);
    assert!(
        rec_total.abs_diff(orig_video_total) <= frame,
        "total duration {rec_total} vs {orig_video_total}"
    );

    // AAC: SDP config= → CodecConfig::Aac, rate/channels sane.
    let cfg_hex = fmtp_value(&out.sdp, "config=")
        .expect("SDP must carry AAC config= for the fixture's AAC track");
    let aac = aac_config_from_fmtp(cfg_hex).expect("aac from config");
    match aac {
        CodecConfig::Aac {
            sample_rate,
            channel_count,
            ..
        } => {
            assert!((8_000..=96_000).contains(&sample_rate));
            assert!((1..=8).contains(&channel_count));
        }
        _ => panic!("expected AAC"),
    }

    // Recovered video samples build a valid fMP4 init + media segment + part.
    let specs = d.track_specs();
    let init = build_init_segment(&specs, 90_000).expect("build_init_segment must succeed");
    assert!(!init.is_empty(), "init segment non-empty");
    let init_issues = validate_init_segment(&init);
    assert!(
        errors(&init_issues).is_empty(),
        "init segment must validate clean: {:?}",
        errors(&init_issues)
    );

    // Split the recovered samples across two "parts" to also exercise a
    // multi-segment (part) build, as CMAF/LL-* tests in this crate do.
    let mid = recovered.len() / 2;
    let (part1, part2) = recovered.split_at(mid);
    let seg1 = build_media_segment(
        1,
        &[FragmentTrackData {
            track_id: 1,
            base_media_decode_time: 0,
            samples: part1,
        }],
    )
    .expect("build_media_segment (part 1) must succeed");
    let part1_total: u64 = part1.iter().map(|s| u64::from(s.duration)).sum();
    let seg2 = build_media_segment(
        2,
        &[FragmentTrackData {
            track_id: 1,
            base_media_decode_time: part1_total,
            samples: part2,
        }],
    )
    .expect("build_media_segment (part 2) must succeed");

    for (label, seg) in [("part 1", &seg1), ("part 2", &seg2)] {
        assert!(!seg.is_empty(), "{label} segment non-empty");
        let issues = validate_media_segment(seg);
        assert!(
            errors(&issues).is_empty(),
            "{label} segment must validate clean: {:?}",
            errors(&issues)
        );
    }
}
