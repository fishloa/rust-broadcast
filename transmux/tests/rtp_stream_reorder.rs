//! `RtpStreamDepacketiser` durations on the real B-frame stream
//! `fixtures/ts/h264_aac.ts`, packetised in decode order with RTP timestamps
//! equal to the presentation time (RFC 6184 §5.1).

use broadcast_common::{Package, Unpackage};
use transmux::pipeline::CodecConfig;
use transmux::{
    RtpMediaKind, RtpPacketiser, RtpStreamDepacketiser, RtpStreamTrack, TrackId, VIDEO_CLOCK_RATE,
};

fn video_durations() -> (Vec<(i64, u32)>, Vec<i64>) {
    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../fixtures/ts/h264_aac.ts");
    let data = std::fs::read(path).expect("h264_aac.ts fixture must exist");
    let media = transmux::TsDemux::new().unpackage(&data[..]).unwrap();
    let track = media
        .tracks
        .iter()
        .find(|t| matches!(t.spec.config, CodecConfig::Avc { .. }))
        .unwrap();
    let source_pts: Vec<i64> = track.samples.iter().map(|s| s.pts.unwrap()).collect();
    let mut p = RtpPacketiser {
        mtu: 1400,
        ssrc: 0x1234_5678,
        ..RtpPacketiser::default()
    };
    let out = p.package(&media).unwrap();
    let stream = out
        .streams
        .iter()
        .find(|s| s.kind == RtpMediaKind::H264)
        .unwrap();
    let id = TrackId::new(1);
    let mut d = RtpStreamDepacketiser::new(vec![RtpStreamTrack::new(
        id,
        RtpMediaKind::H264,
        track.spec.config.clone(),
        VIDEO_CLOCK_RATE,
    )]);
    let mut samples = Vec::new();
    for pkt in &stream.packets {
        samples.extend(d.push(id, &pkt.as_contiguous()).unwrap());
    }
    samples.extend(d.flush(id).unwrap());
    (
        samples
            .iter()
            .map(|s| (s.pts.unwrap(), s.duration.unwrap_or(0)))
            .collect(),
        source_pts,
    )
}

#[test]
fn fixture_is_reordered() {
    let (_, pts) = video_durations();
    assert!(
        pts.windows(2).any(|w| w[1] < w[0]),
        "fixture has no B-frame reorder"
    );
}

#[test]
fn reordered_video_never_gets_zero_duration() {
    let (got, _) = video_durations();
    let zero: Vec<_> = got
        .iter()
        .enumerate()
        .filter(|(_, (_, d))| *d == 0)
        .collect();
    assert!(
        zero.is_empty(),
        "zero-duration samples: {zero:?} of {}",
        got.len()
    );
}
