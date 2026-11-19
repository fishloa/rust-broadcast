use smooth_runtime::{Action, Dropped, FetchFailure, Output, SmoothClientConfig, StreamId};

use crate::common::{Host, Reply, fixture, is_fatal, now};

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
    assert!(host.samples().iter().any(|(_, s)| {
        s.timing
            .and_then(|t| t.dts())
            .is_some_and(|d| d >= 30_000_000)
    }));
}

#[test]
fn a_refresh_past_the_plan_cap_drops_the_oldest_chunks_with_a_discontinuity() {
    let mut grown = String::from("<c n=\"2\" d=\"10000000\" />");
    for n in 3..=10 {
        grown.push_str(&format!("\n<c n=\"{n}\" d=\"10000000\" />"));
    }
    let refreshed = String::from_utf8(live_manifest(""))
        .expect("utf8")
        .replacen("<c n=\"2\" d=\"10000000\" />", &grown, 1);
    let mut host = Host::new(SmoothClientConfig::default().with_max_plan_len(3));
    host.serve_dir("av");
    host.route(
        "Manifest",
        vec![
            Reply::Body(live_manifest("")),
            Reply::Body(refreshed.into_bytes()),
        ],
    );
    for start in [30_000_000, 40_000_000, 50_000_000, 60_000_000, 70_000_000] {
        host.route(
            &format!("QualityLevels(64528)/Fragments(video={start})"),
            vec![Reply::Body(fixture(
                "av/QualityLevels(64528)/Fragments(video=20000000)",
            ))],
        );
    }
    for start in [80_000_000, 90_000_000, 100_000_000] {
        host.route(
            &format!("QualityLevels(64528)/Fragments(video={start})"),
            vec![Reply::Body(fixture(
                "av/QualityLevels(64528)/Fragments(video=20000000)",
            ))],
        );
    }
    host.run_until(|o| o.iter().any(|o| matches!(o, Output::Discontinuity)));
    assert!(host.has(|o| matches!(o, Output::Discontinuity)));
    for start in [30_000_000, 40_000_000, 50_000_000, 60_000_000, 70_000_000] {
        assert_eq!(
            host.count_requests(&format!("{VIDEO_URL}{start})")),
            0,
            "the plan dropped chunk {start}"
        );
    }
    for start in [80_000_000, 90_000_000, 100_000_000] {
        assert_eq!(
            host.count_requests(&format!("{VIDEO_URL}{start})")),
            1,
            "the plan kept chunk {start}"
        );
    }
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
fn held_fragments_are_bounded_by_abandoning_a_stream_that_never_announces() {
    let held =
        |start: &str| fixture(&format!("av/QualityLevels(64528)/Fragments(video={start})")).len();
    let cap = held("0") + held("10000000");
    let mut host = Host::new(SmoothClientConfig::default().with_max_held_bytes(cap));
    host.serve_dir("av");
    host.route("Manifest", vec![Reply::Body(live_manifest(""))]);
    for start in ["0", "10214580", "20199161"] {
        host.route(
            &format!("QualityLevels(98429)/Fragments(audio={start})"),
            vec![Reply::Status(404)],
        );
    }
    host.run_until(|o| o.iter().any(|o| matches!(o, Output::Tracks(_))));
    assert!(
        host.has(|o| matches!(o, Output::Dropped(Dropped::StreamLost { stream }) if stream.0 == 1)),
        "the stream that never announced its track id is abandoned"
    );
    assert!(
        sample_count(&host.outputs) > 0,
        "the held fragments of the announced streams are released"
    );
    assert!(
        !host.has(|o| matches!(o, Output::Fatal(_))),
        "the announced stream keeps the session alive"
    );
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
        t.value() == 1
            && s.timing
                .and_then(|t| t.dts())
                .is_some_and(|d| (20_000_000..30_000_000).contains(&d))
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

fn step_until_idle(host: &mut Host) {
    for _ in 0..64 {
        if !host.step() {
            return;
        }
    }
    panic!("the client kept scheduling without a clock advance");
}

fn fragment_body(stream: usize, start: u64) -> Vec<u8> {
    match stream {
        0 => fixture(&format!("av/QualityLevels(64528)/Fragments(video={start})")),
        _ => fixture(&format!("av/QualityLevels(98429)/Fragments(audio={start})")),
    }
}

fn fetch_manifest(client: &mut smooth_runtime::SmoothClient, at: u64) -> smooth_runtime::RequestId {
    match client.poll(now(at)) {
        Some(Action::FetchManifest { id, .. }) => id,
        other => panic!("expected a manifest fetch, got {other:?}"),
    }
}

fn drain_fragment_actions(client: &mut smooth_runtime::SmoothClient, at: u64) {
    while let Some(action) = client.poll(now(at)) {
        match action {
            Action::FetchFragment {
                id, stream, start, ..
            } => {
                let body = fragment_body(stream.0, start);
                client.on_fragment(now(at), id, &body);
            }
            other => panic!("unexpected action {other:?}"),
        }
    }
}

fn live_manifest_with_last_duration(last: &str) -> Vec<u8> {
    String::from_utf8(live_manifest(""))
        .expect("utf8")
        .replacen(
            "<c n=\"2\" d=\"10000000\" />",
            &format!("<c n=\"2\" d=\"{last}\" />"),
            1,
        )
        .into_bytes()
}

fn deadline_after_drain(last: &str) -> Option<u64> {
    let mut client = smooth_runtime::SmoothClient::new(SmoothClientConfig::default());
    let id = fetch_manifest(&mut client, 0);
    client.on_manifest(now(0), id, &live_manifest_with_last_duration(last));
    drain_fragment_actions(&mut client, 0);
    client.next_deadline()
}

#[test]
fn the_live_refresh_waits_half_the_last_chunk_clamped_to_the_bounds() {
    assert_eq!(deadline_after_drain("20000000"), Some(1_000_000));
    assert_eq!(deadline_after_drain("1000000"), Some(500_000));
    assert_eq!(deadline_after_drain("600000000"), Some(5_000_000));
}

#[test]
fn an_in_flight_manifest_refresh_schedules_no_second_one() {
    let mut client = smooth_runtime::SmoothClient::new(SmoothClientConfig::default());
    let id = fetch_manifest(&mut client, 0);
    client.on_manifest(now(0), id, &live_manifest(""));
    drain_fragment_actions(&mut client, 0);
    let deadline = client.next_deadline().expect("a refresh is due");
    let _refreshing = fetch_manifest(&mut client, deadline);
    assert_eq!(client.next_deadline(), None);
    assert!(client.poll(now(deadline)).is_none());
}

#[test]
fn a_live_404_schedules_the_next_manifest_fetch_at_once() {
    let mut client = smooth_runtime::SmoothClient::new(SmoothClientConfig::default());
    let id = fetch_manifest(&mut client, 0);
    client.on_manifest(now(0), id, &live_manifest(""));
    let mut video = None;
    while let Some(action) = client.poll(now(0)) {
        if let Action::FetchFragment {
            id,
            stream: StreamId(0),
            ..
        } = action
        {
            video = Some(id);
        }
    }
    client.on_failure(
        now(0),
        video.expect("a video fragment fetch"),
        FetchFailure {
            status: Some(http::StatusCode::NOT_FOUND),
            retry_after: std::time::Duration::ZERO,
        },
    );
    let _refreshing = fetch_manifest(&mut client, 0);
}

#[test]
fn a_live_500_does_not_force_a_manifest_refresh() {
    let mut host = Host::new(SmoothClientConfig::default());
    host.serve_dir("av");
    host.route("Manifest", vec![Reply::Body(live_manifest(""))]);
    for path in [
        "QualityLevels(64528)/Fragments(video=0)",
        "QualityLevels(98429)/Fragments(audio=0)",
    ] {
        host.route(path, vec![Reply::Status(500)]);
    }
    host.run_for(2 * SECOND_MICROS);
    assert_eq!(host.count_requests("Manifest"), 1);
}

#[test]
fn a_static_404_does_not_force_a_manifest_refresh() {
    let mut host = Host::new(SmoothClientConfig::default());
    host.serve_dir("av");
    host.route("Manifest", vec![Reply::Body(fixture("av/Manifest"))]);
    host.route(
        "QualityLevels(64528)/Fragments(video=0)",
        vec![Reply::Status(404)],
    );
    host.run_for(2 * SECOND_MICROS);
    assert_eq!(host.count_requests("Manifest"), 1);
}

#[test]
fn a_fatal_after_announcing_schedules_no_refresh() {
    let mut host = Host::new(SmoothClientConfig::default().with_max_manifest_failures(1));
    host.serve_dir("av");
    host.route(
        "Manifest",
        vec![
            Reply::Body(live_manifest("")),
            Reply::Body(b"not xml".to_vec()),
        ],
    );
    host.run_until(|o| o.iter().any(is_fatal));
    assert!(host.client.is_fatal());
    assert_eq!(host.client.next_deadline(), None);
}

#[test]
fn a_fatal_session_polls_nothing_and_has_no_deadline() {
    let mut client = smooth_runtime::SmoothClient::new(
        SmoothClientConfig::default().with_max_manifest_failures(1),
    );
    let id = fetch_manifest(&mut client, 0);
    client.on_manifest(now(0), id, &live_manifest(""));
    let mut fragments = Vec::new();
    while let Some(action) = client.poll(now(0)) {
        if let Action::FetchFragment { id, .. } = action {
            fragments.push(id);
        }
    }
    assert_eq!(fragments.len(), 2);
    for id in fragments {
        client.on_failure(
            now(0),
            id,
            FetchFailure {
                status: Some(http::StatusCode::NOT_FOUND),
                retry_after: std::time::Duration::ZERO,
            },
        );
    }
    let id = fetch_manifest(&mut client, 0);
    client.on_manifest(now(0), id, b"not xml");
    assert!(client.is_fatal());
    assert_eq!(client.next_deadline(), None);
    assert!(client.poll(now(10 * SECOND_MICROS)).is_none());
}

fn two_audio_manifest_without_fra() -> Vec<u8> {
    let named = AUDIO_STREAM.replace("Type=\"audio\"", "Type=\"audio\" Name=\"eng\"");
    String::from_utf8(live_manifest(""))
        .expect("utf8")
        .replace(AUDIO_STREAM, &named)
        .into_bytes()
}

#[test]
fn a_stream_missing_from_five_refreshes_is_abandoned() {
    let chunks = "<c t=\"0\" d=\"10214580\" />\n";
    let mut host = Host::new(SmoothClientConfig::default());
    host.serve_dir("av");
    host.route(
        "Manifest",
        vec![
            Reply::Body(two_audio_manifest(chunks)),
            Reply::Body(two_audio_manifest_without_fra()),
        ],
    );
    host.route(
        "fra/QualityLevels(98429)/Fragments(audio=0)",
        vec![Reply::Body(fixture(
            "av/QualityLevels(98429)/Fragments(audio=0)",
        ))],
    );
    for _ in 0..4 {
        step_until_idle(&mut host);
        assert!(!host.has(|o| matches!(o, Output::Dropped(Dropped::StreamLost { .. }))));
        host.now = host.client.next_deadline().expect("a refresh is due");
    }
    step_until_idle(&mut host);
    assert!(!host.has(|o| matches!(o, Output::Dropped(Dropped::StreamLost { .. }))));
    host.now = host.client.next_deadline().expect("a refresh is due");
    step_until_idle(&mut host);
    assert!(host.has(|o| matches!(
        o,
        Output::Dropped(Dropped::StreamLost { stream }) if stream.0 == 2
    )));
    assert_eq!(host.count_requests("Manifest"), 6);
    let fra_requests = host.count_requests("fra/");
    host.now = host.client.next_deadline().expect("a refresh is due");
    step_until_idle(&mut host);
    assert_eq!(host.count_requests("fra/"), fra_requests);
}

#[test]
fn held_bytes_at_the_cap_do_not_abandon_a_stream() {
    let first = fixture("av/QualityLevels(64528)/Fragments(video=0)");
    let mut client = smooth_runtime::SmoothClient::new(
        SmoothClientConfig::default().with_max_held_bytes(first.len()),
    );
    let id = fetch_manifest(&mut client, 0);
    client.on_manifest(now(0), id, &live_manifest(""));
    let mut video = None;
    while let Some(action) = client.poll(now(0)) {
        if let Action::FetchFragment {
            id,
            stream: StreamId(0),
            ..
        } = action
        {
            video = Some(id);
        }
    }
    client.on_fragment(now(0), video.expect("a video fragment fetch"), &first);
    let mut announced = false;
    let mut lost = false;
    while let Some(output) = client.next_output() {
        if matches!(output, Output::Tracks(_)) {
            announced = true;
        }
        if matches!(output, Output::Dropped(Dropped::StreamLost { .. })) {
            lost = true;
        }
    }
    assert!(!lost, "the cap is spent only past its value");
    assert!(
        !announced,
        "the audio stream has not announced its track id"
    );
}

fn live_manifest_without_video() -> Vec<u8> {
    let text = String::from_utf8(live_manifest("")).expect("utf8");
    let start = text
        .find("<StreamIndex Type=\"video\"")
        .expect("a video stream");
    let end =
        start + text[start..].find("</StreamIndex>").expect("its close") + "</StreamIndex>".len();
    format!("{}{}", &text[..start], &text[end..]).into_bytes()
}

#[test]
fn a_held_fragment_with_every_stream_abandoned_fails_without_announcing() {
    let mut client = smooth_runtime::SmoothClient::new(
        SmoothClientConfig::default()
            .with_max_fragment_failures(10)
            .with_max_held_bytes(0),
    );
    let id = fetch_manifest(&mut client, 0);
    client.on_manifest(now(0), id, &live_manifest(""));
    let mut video = None;
    while let Some(action) = client.poll(now(0)) {
        if let Action::FetchFragment {
            id,
            stream: StreamId(0),
            ..
        } = action
        {
            video = Some(id);
        }
    }
    let mut video = video.expect("a video fragment fetch");
    let mut at = 0;
    for _ in 0..5 {
        client.on_failure(
            now(at),
            video,
            FetchFailure {
                status: Some(http::StatusCode::NOT_FOUND),
                retry_after: std::time::Duration::ZERO,
            },
        );
        let id = fetch_manifest(&mut client, at);
        client.on_manifest(now(at), id, &live_manifest_without_video());
        at = client.next_deadline().expect("a fragment retry is due");
        match client.poll(now(at)) {
            Some(Action::FetchFragment { id, .. }) => video = id,
            other => panic!("expected a fragment retry, got {other:?}"),
        }
    }
    client.on_fragment(now(at), video, &fragment_body(0, 0));
    assert!(client.is_fatal());
    let mut tracks = false;
    while let Some(output) = client.next_output() {
        if matches!(output, Output::Tracks(_)) {
            tracks = true;
        }
    }
    assert!(
        !tracks,
        "no track set is announced after the stream gave up"
    );
}
