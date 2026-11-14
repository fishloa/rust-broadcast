use smooth_runtime::{Dropped, Output, SmoothClientConfig};

use crate::common::{Host, Reply, fixture};

const STATIC_HEADER: &str = "Duration=\"30415941\"";
const VIDEO_URL: &str = "QualityLevels(64528)/Fragments(video=";
const SECOND_MICROS: u64 = 1_000_000;

fn live_manifest(attrs: &str) -> Vec<u8> {
    let text = String::from_utf8(fixture("av/Manifest")).expect("utf8");
    text.replace(
        STATIC_HEADER,
        &format!("{STATIC_HEADER} IsLive=\"TRUE\" {attrs}"),
    )
    .into_bytes()
}

fn live_host(attrs: &str) -> Host {
    let mut host = Host::new(SmoothClientConfig::default());
    host.serve_dir("av");
    host.route("Manifest", vec![Reply::Body(live_manifest(attrs))]);
    host
}

fn requested_video(host: &Host, start: u64) -> bool {
    host.requested
        .iter()
        .any(|u| u.ends_with(&format!("{VIDEO_URL}{start})")))
}

fn sample_count(o: &[Output]) -> usize {
    o.iter()
        .filter(|o| matches!(o, Output::Sample { .. }))
        .count()
}

#[test]
fn live_join_starts_look_ahead_fragments_behind_the_edge() {
    let mut host = live_host("");
    host.run_until(|o| sample_count(o) > 100);
    assert!(
        requested_video(&host, 0),
        "two fragments behind the edge of three is the first"
    );
}

#[test]
fn live_join_honours_a_zero_look_ahead_by_starting_at_the_edge() {
    let mut host = Host::new(SmoothClientConfig::default().with_live_join_fragments(0));
    host.serve_dir("av");
    host.route("Manifest", vec![Reply::Body(live_manifest(""))]);
    host.run_until(|o| sample_count(o) > 0 && o.iter().any(|o| matches!(o, Output::Tracks(_))));
    assert!(!requested_video(&host, 0));
    assert!(!requested_video(&host, 10_000_000));
    assert!(requested_video(&host, 20_000_000));
}

#[test]
fn live_join_is_clamped_to_the_dvr_window() {
    let mut host = Host::new(SmoothClientConfig::default().with_live_join_fragments(10));
    host.serve_dir("av");
    host.route(
        "Manifest",
        vec![Reply::Body(live_manifest("DVRWindowLength=\"10000000\""))],
    );
    host.run_until(|o| o.iter().any(|o| matches!(o, Output::Tracks(_))));
    assert!(!requested_video(&host, 0));
    assert!(requested_video(&host, 10_000_000));
}

#[test]
fn a_refresh_that_adds_a_chunk_fetches_exactly_that_chunk() {
    let refreshed = String::from_utf8(live_manifest(""))
        .expect("utf8")
        .replacen(
            "<c n=\"2\" d=\"10000000\" />",
            "<c n=\"2\" d=\"10000000\" />\n<c n=\"3\" d=\"10000000\" />",
            1,
        );
    let mut host = live_host("");
    host.route(
        "Manifest",
        vec![
            Reply::Body(live_manifest("")),
            Reply::Body(refreshed.into_bytes()),
        ],
    );
    host.route(
        "QualityLevels(64528)/Fragments(video=30000000)",
        vec![Reply::Body(fixture(
            "av/QualityLevels(64528)/Fragments(video=20000000)",
        ))],
    );
    host.run_for(20 * SECOND_MICROS);
    assert_eq!(host.count_requests(&format!("{VIDEO_URL}30000000)")), 1);
    assert_eq!(host.count_requests(&format!("{VIDEO_URL}20000000)")), 1);
    assert!(
        host.samples()
            .iter()
            .any(|(_, s)| s.dts.is_some_and(|d| d >= 30_000_000))
    );
}

#[test]
fn a_live_window_that_moves_past_the_last_fragment_is_a_discontinuity() {
    let moved = String::from_utf8(live_manifest("")).expect("utf8").replace(
        "<c n=\"0\" d=\"10000000\" />",
        "<c t=\"90000000\" d=\"10000000\" />",
    );
    let mut host = live_host("");
    host.route(
        "Manifest",
        vec![
            Reply::Body(live_manifest("")),
            Reply::Body(moved.into_bytes()),
        ],
    );
    host.run_until(|o| o.iter().any(|o| matches!(o, Output::Discontinuity)));
    assert!(host.has(|o| matches!(o, Output::Discontinuity)));
}

#[test]
fn a_live_fragment_that_keeps_failing_is_skipped_with_a_discontinuity() {
    let mut host = live_host("");
    host.route(
        "QualityLevels(64528)/Fragments(video=10000000)",
        vec![Reply::Status(404)],
    );
    host.run_until(|o| {
        o.iter()
            .any(|o| matches!(o, Output::Dropped(Dropped::Fragment { .. })))
    });
    assert!(host.has(|o| matches!(o, Output::Discontinuity)));
    host.run_for(20 * SECOND_MICROS);
    let video_after_gap = host.samples().iter().any(|(t, s)| {
        t.value() == 1 && s.dts.is_some_and(|d| (20_000_000..30_000_000).contains(&d))
    });
    assert!(
        video_after_gap,
        "playback continues past the skipped fragment"
    );
}

#[test]
fn a_live_manifest_is_refetched_repeatedly_while_nothing_new_appears() {
    let mut host = live_host("");
    host.run_for(20 * SECOND_MICROS);
    assert!(host.count_requests("Manifest") >= 4);
}

const AUDIO_STREAM: &str = r#"<StreamIndex Type="audio" QualityLevels="1" Chunks="3" Url="QualityLevels({bitrate})/Fragments(audio={start time})">"#;

fn two_audio_manifest(fra_chunks: &str) -> Vec<u8> {
    let named = AUDIO_STREAM.replace("Type=\"audio\"", "Type=\"audio\" Name=\"eng\"");
    let fra = format!(
        "<StreamIndex Type=\"audio\" Name=\"fra\" QualityLevels=\"1\" Url=\"fra/QualityLevels({{bitrate}})/Fragments(audio={{start time}})\">\n<QualityLevel Index=\"0\" Bitrate=\"98429\" FourCC=\"AACL\" SamplingRate=\"44100\" Channels=\"1\" BitsPerSample=\"16\" PacketSize=\"4\" AudioTag=\"255\" CodecPrivateData=\"1208\" />\n{fra_chunks}</StreamIndex>\n"
    );
    String::from_utf8(live_manifest(""))
        .expect("utf8")
        .replace(AUDIO_STREAM, &named)
        .replace(
            "</SmoothStreamingMedia>",
            &format!("{fra}</SmoothStreamingMedia>"),
        )
        .into_bytes()
}

#[test]
fn a_refresh_matches_each_stream_by_its_own_identity() {
    let chunks = "<c t=\"0\" d=\"10214580\" />\n";
    let grown = format!("{chunks}<c d=\"9984581\" />\n");
    let mut host = live_host("");
    host.route(
        "Manifest",
        vec![
            Reply::Body(two_audio_manifest(chunks)),
            Reply::Body(two_audio_manifest(&grown)),
        ],
    );
    let audio = fixture("av/QualityLevels(98429)/Fragments(audio=0)");
    for path in [
        "fra/QualityLevels(98429)/Fragments(audio=0)",
        "fra/QualityLevels(98429)/Fragments(audio=10214580)",
    ] {
        host.route(path, vec![Reply::Body(audio.clone())]);
    }
    host.run_for(20 * SECOND_MICROS);
    assert_eq!(
        host.count_requests("fra/QualityLevels(98429)/Fragments(audio=10214580)"),
        1,
        "the stream whose timeline grew fetches its new chunk"
    );
    assert_eq!(
        host.count_requests("QualityLevels(98429)/Fragments(audio=10214580)"),
        2,
        "once for eng's own timeline and once for fra's"
    );
}
