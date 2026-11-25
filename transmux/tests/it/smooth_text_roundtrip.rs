//! `SmoothPackager` -> `SmoothManifest::parse` for a `Type="text"` stream
//! ([MS-SSTR] §2.2.2.3 / §2.2.2.5): the packager writes the text stream the
//! parser must read back (real artifact: ffmpeg CMAF `av-subtitle-frag.mp4`,
//! AVC + `stpp`/TTML).

use broadcast_common::timescale::Timescale;
use broadcast_smooth::{SmoothManifest, StreamType};
use transmux::SmoothPackager;
use transmux::pipeline::CodecConfig;

#[test]
fn packager_text_stream_parses_back() {
    fn timescale(value: u32) -> Timescale {
        Timescale::try_new(value).expect("a non-zero timescale")
    }

    let path = test_fixtures::path("mp4/cmaf-av-subtitle/av-subtitle-frag.mp4");
    let bytes = std::fs::read(&path).unwrap_or_else(|e| panic!("read {}: {e}", path.display()));
    let media =
        test_harness::collect::media_from_mp4(&bytes, test_harness::collect::CollectMode::Seekable)
            .expect("demux fixture");
    let (_log, segments) =
        crate::packagers::smooth_parts(&media, core::time::Duration::from_secs(2))
            .expect("smooth fragments");
    let manifest = crate::packagers::smooth(&SmoothPackager, &media).expect("smooth");

    let parsed = SmoothManifest::parse(&manifest).expect("parse the packager's own manifest");
    let text = parsed
        .streams
        .iter()
        .find(|s| s.stream_type == StreamType::Text)
        .expect("text StreamIndex");
    assert_eq!(text.subtype.as_deref(), Some("SUBT"));
    assert_eq!(text.qualities.len(), 1);
    assert_eq!(text.qualities[0].four_cc, "stpp");
    let sub = media
        .tracks
        .iter()
        .find(|t| matches!(t.spec.config, CodecConfig::Subtitle { .. }))
        .unwrap();
    let frags = segments
        .iter()
        .filter(|f| f.summary.track == sub.spec.track_id)
        .count();
    assert_eq!(text.chunks_list.len(), frags, "one <c> per text fragment");
    // ...and the text level resolves to a muxable subtitle TrackSpec.
    let spec = transmux::track_spec_from_quality_level(
        transmux::TrackId::new(7),
        timescale(90_000),
        text.stream_type,
        &text.qualities[0],
    )
    .expect("text track spec");
    assert!(matches!(
        spec.config,
        CodecConfig::Subtitle {
            format: transmux::pipeline::SubtitleFormat::Ttml,
            sample_entry: Some(_),
        }
    ));
    // The same manifest still carries the video stream.
    assert!(
        parsed
            .streams
            .iter()
            .any(|s| s.stream_type == StreamType::Video)
    );
}
