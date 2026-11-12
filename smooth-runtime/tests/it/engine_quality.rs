use smooth_runtime::{Action, SmoothConfig};

use crate::common::{Host, fixture, is_ended, is_fatal};

const SECOND_VIDEO_BITRATE: &str = "99999";
const FIRST_FRAGMENT: &str = "Fragments(video=0)";
const FIRST_BITRATE: &str = "64528";

fn two_quality_manifest() -> Vec<u8> {
    let text = String::from_utf8(fixture("av/Manifest")).expect("utf8");
    let first = text
        .lines()
        .find(|l| l.contains("Bitrate=\"64528\""))
        .expect("video quality level");
    let second = first
        .replace("64528", SECOND_VIDEO_BITRATE)
        .replace("Index=\"0\"", "Index=\"1\"");
    text.replace(first, &format!("{first}\n{second}"))
        .into_bytes()
}

fn host_after_manifest() -> Host {
    let mut host = Host::new(SmoothConfig::default());
    host.serve_dir("av");
    let Some(Action::FetchManifest { id, .. }) = host.client.poll(0) else {
        panic!("first action is the manifest fetch");
    };
    host.client.on_manifest(0, id, &two_quality_manifest());
    host
}

#[test]
fn a_selected_quality_names_the_later_fragment_urls() {
    let mut host = host_after_manifest();
    assert!(host.client.select_video_quality(1));
    host.run_until(|o| o.iter().any(|o| is_fatal(o) || is_ended(o)));
    let selected = format!("QualityLevels({SECOND_VIDEO_BITRATE})/{FIRST_FRAGMENT}");
    let original = format!("QualityLevels({FIRST_BITRATE})/{FIRST_FRAGMENT}");
    assert!(host.count_requests(&selected) > 0);
    assert_eq!(host.count_requests(&original), 0);
}

#[test]
fn an_unknown_quality_index_is_refused() {
    let mut host = host_after_manifest();
    assert!(!host.client.select_video_quality(2));
}

const ORIGINAL_PPS_TAIL: &str = "ebe3cb20";
const OTHER_PPS_TAIL: &str = "ebe3cb21";
const SKIPPED_STREAM: &str = "<StreamIndex Type=\"video\" TimeScale=\"0\" QualityLevels=\"1\" Chunks=\"1\" Url=\"Skipped({bitrate})/Fragments(video={start time})\">\n<QualityLevel Index=\"0\" Bitrate=\"11111\" FourCC=\"H264\" MaxWidth=\"320\" MaxHeight=\"240\" CodecPrivateData=\"00\" />\n<c n=\"0\" d=\"10000000\" />\n</StreamIndex>\n";

fn manifest_with_other_pps_second_level() -> Vec<u8> {
    String::from_utf8(two_quality_manifest())
        .expect("utf8")
        .lines()
        .map(|l| {
            if l.contains(SECOND_VIDEO_BITRATE) {
                l.replace(ORIGINAL_PPS_TAIL, OTHER_PPS_TAIL)
            } else {
                l.to_string()
            }
        })
        .collect::<Vec<_>>()
        .join("\n")
        .into_bytes()
}

#[test]
fn a_level_with_a_different_codec_config_is_refused_once_announced() {
    let mut host = Host::new(SmoothConfig::default());
    host.serve_dir("av");
    let Some(Action::FetchManifest { id, .. }) = host.client.poll(0) else {
        panic!("first action is the manifest fetch");
    };
    host.client
        .on_manifest(0, id, &manifest_with_other_pps_second_level());
    host.run_until(|o| {
        o.iter()
            .any(|o| matches!(o, smooth_runtime::Output::Tracks(_)))
    });
    assert!(!host.client.select_video_quality(1));
}

#[test]
fn the_offered_levels_index_the_stream_that_select_video_quality_switches() {
    let manifest = String::from_utf8(two_quality_manifest())
        .expect("utf8")
        .replace(
            "<StreamIndex Type=\"video\" QualityLevels=\"1\"",
            &format!("{SKIPPED_STREAM}<StreamIndex Type=\"video\" QualityLevels=\"1\""),
        );
    let mut host = Host::new(SmoothConfig::default());
    host.serve_dir("av");
    let Some(Action::FetchManifest { id, .. }) = host.client.poll(0) else {
        panic!("first action is the manifest fetch");
    };
    host.client.on_manifest(0, id, manifest.as_bytes());
    assert_eq!(host.client.video_quality_count(), 2);
    assert!(host.client.select_video_quality(1));
    host.run_until(|o| o.iter().any(|o| is_fatal(o) || is_ended(o)));
    let selected = format!("QualityLevels({SECOND_VIDEO_BITRATE})/{FIRST_FRAGMENT}");
    assert!(host.count_requests(&selected) > 0);
}
