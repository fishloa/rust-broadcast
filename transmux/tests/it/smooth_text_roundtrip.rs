//! `SmoothPackager` -> `SmoothManifest::parse` for a `Type="text"` stream
//! ([MS-SSTR] §2.2.2.3 / §2.2.2.5): the packager writes the text stream the
//! parser must read back (real artifact: ffmpeg CMAF `av-subtitle-frag.mp4`,
//! AVC + `stpp`/TTML).

use broadcast_common::{Package, Unpackage};
use broadcast_smooth::{SmoothManifest, StreamType};
use transmux::pipeline::CodecConfig;
use transmux::{Fmp4Demux, SmoothPackager};

#[test]
fn packager_text_stream_parses_back() {
    let path = test_fixtures::path("mp4/cmaf-av-subtitle/av-subtitle-frag.mp4");
    let bytes = std::fs::read(&path).unwrap_or_else(|e| panic!("read {}: {e}", path.display()));
    let media = Fmp4Demux::new().unpackage(&bytes).expect("demux fixture");
    let out = SmoothPackager::default().package(&media).expect("smooth");

    let parsed = SmoothManifest::parse(&out.manifest).expect("parse the packager's own manifest");
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
    let frags = out
        .fragments
        .iter()
        .filter(|f| f.track_id == sub.spec.track_id)
        .count();
    assert_eq!(text.chunks_list.len(), frags, "one <c> per text fragment");
    // ...and the text level resolves to a muxable subtitle TrackSpec.
    let spec = transmux::track_spec_from_quality_level(
        transmux::TrackId::new(7),
        Timescale::from_nonzero(90_000),
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
