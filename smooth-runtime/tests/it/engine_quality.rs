use smooth_runtime::{Action, Output, SmoothClientConfig};
use transmux::TrackSpec;

use crate::common::{Host, fixture, is_ended, is_fatal, now};

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
    let mut host = Host::new(SmoothClientConfig::default());
    host.serve_dir("av");
    let Some(Action::FetchManifest { id, .. }) = host.client.poll(now(0)) else {
        panic!("first action is the manifest fetch");
    };
    host.client.on_manifest(now(0), id, &two_quality_manifest());
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

const SKIPPED_STREAM: &str = "<StreamIndex Type=\"video\" QualityLevels=\"0\" Chunks=\"1\" Url=\"Skipped({bitrate})/Fragments(video={start time})\">\n<c n=\"0\" d=\"10000000\" />\n</StreamIndex>\n";

const LOW_LEVEL_BITRATE: &str = "30000";
const FIRST_LEVEL_WIDTH: u32 = 320;
const LOW_LEVEL_WIDTH: u32 = 160;

fn ladder_manifest(second_level: &str) -> Vec<u8> {
    let text = String::from_utf8(fixture("av/Manifest")).expect("utf8");
    let first = text
        .lines()
        .find(|l| l.contains("Bitrate=\"64528\""))
        .expect("video quality level");
    text.replace(first, &format!("{first}\n{second_level}"))
        .into_bytes()
}

fn real_low_level() -> String {
    String::from_utf8(fixture("av-low/Manifest"))
        .expect("utf8")
        .lines()
        .find(|l| l.contains(&format!("Bitrate=\"{LOW_LEVEL_BITRATE}\"")))
        .expect("low quality level")
        .replace("Index=\"0\"", "Index=\"1\"")
}

fn announced_host(manifest: &[u8]) -> Host {
    let mut host = Host::new(SmoothClientConfig::default());
    host.serve_dir("av");
    host.serve_dir("av-low");
    let Some(Action::FetchManifest { id, .. }) = host.client.poll(now(0)) else {
        panic!("first action is the manifest fetch");
    };
    host.client.on_manifest(now(0), id, manifest);
    while !host.has(|o| matches!(o, Output::Tracks(_))) {
        assert!(host.step(), "tracks are announced");
    }
    host
}

fn coded_width(spec: &TrackSpec) -> u32 {
    spec.display.as_ref().expect("display geometry").coded_width
}

#[test]
fn a_ladder_with_a_different_resolution_switches_and_re_announces_before_its_samples() {
    let mut host = announced_host(&ladder_manifest(&real_low_level()));
    let announced = host
        .outputs
        .iter()
        .find_map(|o| match o {
            Output::Tracks(specs) => Some(specs.clone()),
            _ => None,
        })
        .expect("tracks");
    assert_eq!(coded_width(&announced[0]), FIRST_LEVEL_WIDTH);
    assert!(host.client.select_video_quality(1));
    host.run_until(|o| o.iter().any(|o| is_fatal(o) || is_ended(o)));
    let changed: Vec<usize> = (0..host.outputs.len())
        .filter(|&i| matches!(host.outputs[i], Output::TracksChanged(_)))
        .collect();
    assert_eq!(changed.len(), 1);
    let Output::TracksChanged(specs) = &host.outputs[changed[0]] else {
        panic!("tracks changed");
    };
    assert_eq!(specs.len(), announced.len());
    assert_eq!(specs[0].track_id, announced[0].track_id);
    assert_eq!(coded_width(&specs[0]), LOW_LEVEL_WIDTH);
    let samples_after = host.outputs[changed[0]..]
        .iter()
        .filter(|o| matches!(o, Output::Sample { track, .. } if *track == announced[0].track_id))
        .count();
    let samples_before = host.outputs[..changed[0]]
        .iter()
        .filter(|o| matches!(o, Output::Sample { track, .. } if *track == announced[0].track_id))
        .count();
    assert!(samples_before > 0);
    assert!(samples_after > 0);
    let low = format!("QualityLevels({LOW_LEVEL_BITRATE})/{FIRST_FRAGMENT}");
    assert_eq!(host.count_requests(&low), 0);
    assert!(
        host.count_requests(&format!(
            "QualityLevels({LOW_LEVEL_BITRATE})/Fragments(video=20000000)"
        )) > 0
    );
}

#[test]
fn a_level_of_another_codec_family_is_refused_once_announced() {
    let other_family = real_low_level().replace("H264", "H265");
    let mut host = announced_host(&ladder_manifest(&other_family));
    assert!(!host.client.select_video_quality(1));
    host.run_until(|o| o.iter().any(|o| is_fatal(o) || is_ended(o)));
    assert!(!host.has(|o| matches!(o, Output::TracksChanged(_))));
}

#[test]
fn a_level_with_the_same_config_switches_without_re_announcing() {
    let mut host = announced_host(&two_quality_manifest());
    assert!(host.client.select_video_quality(1));
    host.run_until(|o| o.iter().any(|o| is_fatal(o) || is_ended(o)));
    assert!(!host.has(|o| matches!(o, Output::TracksChanged(_))));
}
#[test]
fn the_offered_levels_index_the_stream_that_select_video_quality_switches() {
    let manifest = String::from_utf8(two_quality_manifest())
        .expect("utf8")
        .replace(
            "<StreamIndex Type=\"video\" QualityLevels=\"1\"",
            &format!("{SKIPPED_STREAM}<StreamIndex Type=\"video\" QualityLevels=\"1\""),
        );
    let mut host = Host::new(SmoothClientConfig::default());
    host.serve_dir("av");
    let Some(Action::FetchManifest { id, .. }) = host.client.poll(now(0)) else {
        panic!("first action is the manifest fetch");
    };
    host.client.on_manifest(now(0), id, manifest.as_bytes());
    assert_eq!(host.client.video_quality_count(), 2);
    assert!(host.client.select_video_quality(1));
    host.run_until(|o| o.iter().any(|o| is_fatal(o) || is_ended(o)));
    let selected = format!("QualityLevels({SECOND_VIDEO_BITRATE})/{FIRST_FRAGMENT}");
    assert!(host.count_requests(&selected) > 0);
}
