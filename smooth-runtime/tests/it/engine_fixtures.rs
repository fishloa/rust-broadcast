use smooth_runtime::{Output, SmoothClientConfig, SmoothError};
use transmux::smooth_fragment::{FragmentState, fragment_state};

use crate::common::{Host, Reply, fixture, is_ended, is_fatal};

const FNV_OFFSET_BASIS: u64 = 0xcbf2_9ce4_8422_2325;
const FNV_PRIME: u64 = 0x0000_0100_0000_01b3;
const VIDEO_FRAGMENTS: usize = 3;

fn fnv1a(bytes: &[u8]) -> u64 {
    bytes.iter().fold(FNV_OFFSET_BASIS, |h, b| {
        (h ^ u64::from(*b)).wrapping_mul(FNV_PRIME)
    })
}

fn trace_without_track(host: &Host) -> Vec<String> {
    host.samples()
        .iter()
        .map(|(_, s)| {
            format!(
                "dts={:?} pts={:?} dur={:?} sync={} len={} fnv={:016x}",
                s.timing.and_then(|t| t.dts()),
                s.timing.and_then(|t| t.pts()),
                s.timing.and_then(|t| t.duration()),
                s.flags.is_sync,
                s.data.len(),
                fnv1a(&s.data)
            )
        })
        .collect()
}

fn golden_without_track(name: &str) -> Vec<String> {
    let text = std::fs::read_to_string(test_fixtures::path(&format!("golden/smooth/{name}")))
        .expect("golden");
    text.lines()
        .filter_map(|l| l.strip_prefix("sample "))
        .map(|l| l.split_once(' ').expect("fields").1.to_string())
        .collect()
}

#[test]
fn av_plays_every_sample_of_both_tracks_in_the_recorded_order() {
    let mut host = Host::new(SmoothClientConfig::default());
    host.serve_dir("av");
    host.run_until(|o| o.iter().any(is_ended));
    assert_eq!(
        trace_without_track(&host),
        golden_without_track("pull-broadcast-client-av.txt")
    );
}

#[test]
fn av_announces_one_track_per_stream_numbered_from_one() {
    let mut host = Host::new(SmoothClientConfig::default());
    host.serve_dir("av");
    host.run_until(|o| o.iter().any(is_ended));
    let tracks = host
        .outputs
        .iter()
        .find_map(|o| match o {
            Output::Tracks(t) => Some(t.iter().map(|s| s.track_id.value()).collect::<Vec<_>>()),
            _ => None,
        })
        .expect("tracks");
    assert_eq!(tracks, [1, 2]);
    let per_track = |id: u32| {
        host.samples()
            .iter()
            .filter(|(t, _)| t.value() == id)
            .count()
    };
    assert!(per_track(1) > 0 && per_track(2) > 0);
    assert_eq!(per_track(1) + per_track(2), host.samples().len());
}

#[test]
fn timestamps_are_rebased_onto_the_fragment_start_not_restarted_per_fragment() {
    let mut host = Host::new(SmoothClientConfig::default());
    host.serve_dir("av-low");
    host.run_until(|o| o.iter().any(is_fatal));
    let video: Vec<i64> = host
        .samples()
        .iter()
        .map(|(_, s)| s.timing.and_then(|t| t.dts()).expect("dts"))
        .collect();
    assert!(
        video.windows(2).all(|w| w[0] < w[1]),
        "dts strictly increasing across fragments"
    );
    assert!(
        video.iter().any(|&d| d >= 20_000_000),
        "third fragment starts at 2 s"
    );
}

#[test]
fn av_low_ends_fatally_when_the_fourth_chunk_has_no_fragment() {
    let mut host = Host::new(SmoothClientConfig::default());
    host.serve_dir("av-low");
    host.run_until(|o| o.iter().any(is_fatal));
    assert!(host.has(|o| matches!(
        o,
        Output::Fatal(SmoothError::Fragment {
            status: Some(http::StatusCode::NOT_FOUND),
        })
    )));
    assert_eq!(
        host.samples().len(),
        golden_without_track("pull-broadcast-client-av-low.txt").len()
    );
    assert!(!host.has(is_ended));
}

#[test]
fn a_manifest_with_a_protection_element_is_fatal_and_fetches_no_fragment() {
    let text = String::from_utf8(fixture("av/Manifest")).expect("utf8");
    let text = text.replace("<StreamIndex", "<Protection/>\n<StreamIndex");
    let mut host = Host::new(SmoothClientConfig::default());
    host.route("Manifest", vec![Reply::Body(text.into_bytes())]);
    host.run();
    assert!(host.has(|o| matches!(o, Output::Fatal(SmoothError::ManifestProtected))));
    assert_eq!(host.requested.len(), 1);
}

#[test]
fn a_piff_encrypted_fragment_is_fatal() {
    let probe = fixture("av/QualityLevels(64528)/Fragments(video=0)");
    assert_eq!(fragment_state(&probe), FragmentState::Clear);
    let encrypted = test_fixtures::read("mp4/piff-sample-encrypted.mp4");
    assert_eq!(fragment_state(&encrypted), FragmentState::Encrypted);
    let mut host = Host::new(SmoothClientConfig::default());
    host.serve_dir("av");
    host.route(
        "QualityLevels(64528)/Fragments(video=0)",
        vec![Reply::Body(encrypted)],
    );
    host.run();
    assert!(host.has(|o| matches!(o, Output::Fatal(SmoothError::FragmentEncrypted))));
    assert_eq!(host.samples().len(), 0);
}

#[test]
fn only_the_first_video_quality_is_offered_and_chosen_by_default() {
    let mut host = Host::new(SmoothClientConfig::default());
    host.serve_dir("av");
    host.run_until(|o| o.iter().any(is_ended));
    let (qualities, chosen) = host
        .outputs
        .iter()
        .find_map(|o| match o {
            Output::Qualities { qualities, chosen } => Some((qualities.clone(), *chosen)),
            _ => None,
        })
        .expect("qualities");
    assert_eq!(chosen, 0);
    assert_eq!(qualities[0].bitrate, 64_528);
    assert_eq!(qualities[0].resolution, Some((320, 240)));
    assert!(
        qualities[0]
            .codecs
            .as_deref()
            .is_some_and(|c| c.starts_with("avc1"))
    );
    let _ = VIDEO_FRAGMENTS;
}
