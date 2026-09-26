//! `PsDemux` MPEG-2 video gate (C6, #1009) — a Program Stream carrying MPEG-2
//! video (not H.264) must come out as `CodecConfig::Mpeg2Video`, not a garbage
//! `avc1` track built from MPEG-2 slice start codes misread as H.264 NALs.
//!
//! Fixture: `fixtures/ps/ffmpeg-mpeg2video-mp2.ps` — see `fixtures/ps/README.md`
//! for the exact ffmpeg command. Oracle: ffprobe on that same file.

use broadcast_common::Unpackage;
use transmux::PsDemux;
use transmux::media::Media;
use transmux::pipeline::CodecConfig;

fn load_ps() -> Vec<u8> {
    let path = format!(
        "{}/../fixtures/ps/ffmpeg-mpeg2video-mp2.ps",
        env!("CARGO_MANIFEST_DIR")
    );
    std::fs::read(&path).expect("ffmpeg-mpeg2video-mp2.ps fixture must exist")
}

/// Before the C6 fix, `Codec::from_stream_id` mapped every 0xE0-0xEF
/// `stream_id` to H.264 unconditionally: MPEG-2 slice start codes (`0x01`,
/// `0x09`, ...) were misread as an H.264 AUD/SPS/PPS and this test's track 0
/// would come back `CodecConfig::Avc` with nonsense `profile_indication`/
/// `level_indication` bytes lifted from MPEG-2 slice data, or the whole
/// stream would fail to demux at all.
#[test]
fn mpeg2_video_is_not_misidentified_as_h264() {
    let ps = load_ps();
    let media: Media = PsDemux::new().unpackage(&ps).expect("demux must succeed");

    let video = media
        .tracks
        .iter()
        .find(|t| matches!(t.spec.config, CodecConfig::Mpeg2Video { .. }))
        .unwrap_or_else(|| {
            panic!(
                "expected a Mpeg2Video track, got: {:?}",
                media
                    .tracks
                    .iter()
                    .map(|t| &t.spec.config)
                    .collect::<Vec<_>>()
            )
        });

    // Oracle (ffprobe -show_streams -select_streams v): width=352 height=288.
    let CodecConfig::Mpeg2Video { width, height, .. } = video.spec.config else {
        unreachable!()
    };
    assert_eq!(width, 352, "picture width must match ffprobe oracle");
    assert_eq!(height, 288, "picture height must match ffprobe oracle");

    // Oracle (ffprobe -show_packets -select_streams v | grep -c codec_type=video): 25.
    assert_eq!(
        video.samples.len(),
        25,
        "25 video access units (ffprobe oracle)"
    );

    // Oracle (ffprobe -show_packets -select_streams v | grep -c flags=K): 3.
    let syncs = video.samples.iter().filter(|s| s.flags.is_sync).count();
    assert_eq!(syncs, 3, "3 keyframes (ffprobe oracle)");

    // No H.264/AVC track was built from this MPEG-2 video stream_id.
    assert!(
        !media
            .tracks
            .iter()
            .any(|t| matches!(t.spec.config, CodecConfig::Avc { .. })),
        "must not also emit a garbage AVC track for the MPEG-2 video stream"
    );

    // The audio track (mp2, private_stream_1 is not used here — MPEG audio
    // uses the 0xC0-0xDF stream_id range, which this demuxer does not carry;
    // see the module doc's "skipped, never fatal" policy) is out of scope for
    // this fix and is not asserted here.
}
