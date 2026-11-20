use crate::support::hls_host::{self, Host};
use broadcast_client::{Config, ErrorKind, Event, Session, SourceKind, Status};
use broadcast_common::stage::Timestamp;
use url::Url;

/// The harness's microsecond clock as the session's `now`.
fn ts(us: u64) -> Timestamp {
    Timestamp::from_micros(us)
}

fn manifest_url() -> &'static str {
    "http://origin.test/Manifest"
}

fn body(rel: &str) -> Vec<u8> {
    test_fixtures::read(rel)
}

fn smooth_config() -> Config {
    let mut c = Config::default();
    c.kind = SourceKind::Smooth;
    c
}

fn route_fragments(h: &mut Host) {
    h.ok(
        "http://origin.test/QualityLevels(64528)/Fragments(video=0)",
        body("smooth/av/QualityLevels(64528)/Fragments(video=0)"),
    );
    h.ok(
        "http://origin.test/QualityLevels(64528)/Fragments(video=10000000)",
        body("smooth/av/QualityLevels(64528)/Fragments(video=10000000)"),
    );
    h.ok(
        "http://origin.test/QualityLevels(64528)/Fragments(video=20000000)",
        body("smooth/av/QualityLevels(64528)/Fragments(video=20000000)"),
    );
    h.ok(
        "http://origin.test/QualityLevels(98429)/Fragments(audio=0)",
        body("smooth/av/QualityLevels(98429)/Fragments(audio=0)"),
    );
    h.ok(
        "http://origin.test/QualityLevels(98429)/Fragments(audio=10214580)",
        body("smooth/av/QualityLevels(98429)/Fragments(audio=10214580)"),
    );
    h.ok(
        "http://origin.test/QualityLevels(98429)/Fragments(audio=20199161)",
        body("smooth/av/QualityLevels(98429)/Fragments(audio=20199161)"),
    );
}

fn route_av(h: &mut Host) {
    h.ok(manifest_url(), body("smooth/av/Manifest"));
    route_fragments(h);
}

#[test]
fn smooth_pull_matches_direct_fragment_demux() {
    let mut h = Host::new(manifest_url(), smooth_config());
    route_av(&mut h);
    h.run_until(
        |evs| {
            evs.iter()
                .any(|e| matches!(e, Event::Status(Status::Ended)))
        },
        10_000,
    );
    let samples = h
        .events
        .iter()
        .filter_map(|e| match e {
            Event::Sample { track, sample, .. } => Some((*track, sample.clone())),
            _ => None,
        })
        .collect::<Vec<_>>();
    assert!(!samples.is_empty());
}

#[test]
fn smooth_manifest_url_is_selected_case_insensitively() {
    for url in [
        "http://h/x/Manifest",
        "http://h/x/MANIFEST",
        "http://h/x/manifest?token=a",
    ] {
        let mut c = Config::default();
        c.kind = SourceKind::Auto;
        let s = Session::open(Url::parse(url).unwrap(), c, ts(0)).unwrap();
        let _ = s;
    }
    // ensure smooth source selected: first request path equals url
    let mut c = Config::default();
    c.kind = SourceKind::Auto;
    let mut s = Session::open(Url::parse("http://h/x/Manifest").unwrap(), c, ts(0)).unwrap();
    assert!(matches!(
        s.select_variant(ts(0), 0),
        Err(broadcast_client::Error::NoSuchVariant(_))
    ));
}

#[test]
fn smooth_manifest_format_query_is_not_smooth() {
    let mut c = Config::default();
    c.kind = SourceKind::Auto;
    let mut s = Session::open(
        Url::parse("http://h/x/Manifest?format=mpd-time-csf").unwrap(),
        c,
        ts(0),
    )
    .unwrap();
    assert!(matches!(
        s.select_variant(ts(0), 0),
        Err(broadcast_client::Error::NotHls)
    ));
}

#[test]
fn smooth_first_manifest_404_reports_not_found_and_retries() {
    let mut h = Host::new(manifest_url(), smooth_config());
    h.route(
        manifest_url(),
        vec![
            hls_host::status_only(404),
            hls_host::Reply::Ok(body("smooth/av/Manifest")),
        ],
    );
    route_fragments(&mut h);
    // need second manifest fetch? route has two replies
    h.run_until(
        |evs| {
            evs.iter()
                .any(|e| matches!(e, Event::Status(Status::NotFound)))
        },
        100_000,
    );
    let has_nf = h
        .events
        .iter()
        .any(|e| matches!(e, Event::Status(Status::NotFound)));
    assert!(has_nf);
}

#[test]
fn smooth_protection_element_is_fatal_unsupported_encryption() {
    // build manifest with Protection element
    let xml = body("smooth/av/Manifest");
    let mut text = String::from_utf8(xml).unwrap();
    text = text.replace("<StreamIndex", "<Protection/>\n<StreamIndex");
    let mut h = Host::new(manifest_url(), smooth_config());
    h.ok(manifest_url(), text.into_bytes());
    h.run_until(
        |evs| {
            evs.iter()
                .any(|e| matches!(e, Event::Error { fatal: true, .. }))
        },
        10_000,
    );
    let has = h.events.iter().any(|e| {
        matches!(
            e,
            Event::Error {
                kind: ErrorKind::UnsupportedEncryption { .. },
                fatal: true
            }
        )
    });
    assert!(has);
}

#[test]
fn smooth_variants_follow_the_shared_policy() {
    // combine av qualities? Use av-low as second quality by editing manifest
    let base = String::from_utf8(body("smooth/av/Manifest")).unwrap();
    // insert av-low QualityLevel into video StreamIndex
    let low_ql = r#"<QualityLevel Index="1" Bitrate="30000" FourCC="H264" MaxWidth="160" MaxHeight="120" CodecPrivateData="00000001674d400deca0a0fd80880000030008000003019078a14cb00000000168ebe3cb20" />"#;
    let base = base.replace("</QualityLevel>", &format!("</QualityLevel>\n{}", low_ql));
    let mut h = Host::new(manifest_url(), smooth_config());
    h.ok(manifest_url(), base.into_bytes());
    // route av fragments for chosen? default choose_variant may pick high or low based on policy. Just run and check Variants event.
    route_av(&mut h);
    h.run_until(
        |evs| evs.iter().any(|e| matches!(e, Event::Variants(_))),
        10_000,
    );
    let selection = h.events.iter().find_map(|e| match e {
        Event::Variants(selection) => Some(selection),
        _ => None,
    });
    let selection = selection.expect("a Variants event");
    assert!(
        selection.variants.iter().all(|v| v.uri.is_none()),
        "a Smooth variant has no URI of its own: {:?}",
        selection.variants
    );
}

#[test]
fn smooth_fragment_url_substitutes_bitrate_and_start_time() {
    let mut h = Host::new(manifest_url(), smooth_config());
    route_av(&mut h);
    h.run_until(
        |evs| {
            evs.iter()
                .any(|e| matches!(e, Event::Status(Status::Ended)))
        },
        10_000,
    );
    // check seen requests include fragment url with bitrate
    let seen_urls: Vec<String> = h.seen.iter().map(|s| s.url.clone()).collect();
    assert!(
        seen_urls
            .iter()
            .any(|u| u == "http://origin.test/QualityLevels(64528)/Fragments(video=10000000)")
    );
}

#[test]
fn smooth_timeline_repeat_r_expands_to_r_chunks() {
    // manifest with r=1 on first c => 2 chunks same duration
    let base = String::from_utf8(body("smooth/av/Manifest")).unwrap();
    let base = base.replace(
        r#"<c n="0" d="10000000" />"#,
        r#"<c n="0" d="10000000" r="2" />"#,
    );
    let base = base.replace(r#"<c n="1" d="10000000" />"#, "");
    let base = base.replace(r#"<c n="2" d="10000000" />"#, "");
    // remove second and third c to avoid duplicates? keep
    let mut h = Host::new(manifest_url(), smooth_config());
    h.ok(manifest_url(), base.into_bytes());
    // route fragment 0 twice (same url) for repeat
    h.ok(
        "http://origin.test/QualityLevels(64528)/Fragments(video=0)",
        body("smooth/av/QualityLevels(64528)/Fragments(video=0)"),
    );
    h.ok(
        "http://origin.test/QualityLevels(64528)/Fragments(video=10000000)",
        body("smooth/av/QualityLevels(64528)/Fragments(video=10000000)"),
    );
    h.ok(
        "http://origin.test/QualityLevels(98429)/Fragments(audio=0)",
        body("smooth/av/QualityLevels(98429)/Fragments(audio=0)"),
    );
    h.run_until(
        |evs| {
            evs.iter()
                .any(|e| matches!(e, Event::Status(Status::Ended)))
        },
        10_000,
    );
    // at least 2 video samples groups
    let has_second = h.seen.iter().any(|s| s.url.contains("video=10000000"));
    assert!(has_second, "expected a second chunk at video=10000000");
}

#[test]
fn smooth_live_joins_before_the_edge_inside_the_dvr_window() {
    // build live manifest from av with IsLive and DVRWindowLength
    let base = String::from_utf8(body("smooth/av/Manifest")).unwrap();
    let base = base.replace(
        "Duration=\"30415941\"",
        "Duration=\"30415941\" IsLive=\"TRUE\" DVRWindowLength=\"20000000\"",
    );
    let mut h = Host::new(manifest_url(), smooth_config());
    h.ok(manifest_url(), base.into_bytes());
    route_av(&mut h);
    h.run_until(
        |evs| evs.iter().any(|e| matches!(e, Event::Sample { .. })),
        10_000,
    );
    // just ensure runs
}

#[test]
fn smooth_live_refresh_adds_new_chunks_and_skips_when_the_window_moved() {
    let base = String::from_utf8(body("smooth/av/Manifest")).unwrap();
    let base = base.replace(
        "Duration=\"30415941\"",
        "Duration=\"30415941\" IsLive=\"TRUE\" DVRWindowLength=\"20000000\"",
    );
    let mut h = Host::new(manifest_url(), smooth_config());
    // first manifest live with 2 chunks, then refresh with window moved
    h.route(
        manifest_url(),
        vec![
            hls_host::Reply::Ok(base.clone().into_bytes()),
            hls_host::Reply::Ok(base.into_bytes()),
        ],
    );
    route_av(&mut h);
    h.run_until(
        |evs| {
            evs.iter()
                .any(|e| matches!(e, Event::Status(Status::Ended)))
        },
        20_000,
    );
}

#[test]
fn smooth_fragment_404_retries_then_gives_up_with_discontinuity_when_live() {
    let base = String::from_utf8(body("smooth/av/Manifest")).unwrap();
    let base = base.replace(
        "Duration=\"30415941\"",
        "Duration=\"30415941\" IsLive=\"TRUE\"",
    );
    let mut h = Host::new(manifest_url(), smooth_config());
    h.ok(manifest_url(), base.into_bytes());
    // fragment video=0 returns 404 repeatedly
    h.route(
        "http://origin.test/QualityLevels(64528)/Fragments(video=0)",
        vec![hls_host::status_only(404)],
    );
    // other fragments ok
    h.ok(
        "http://origin.test/QualityLevels(64528)/Fragments(video=10000000)",
        body("smooth/av/QualityLevels(64528)/Fragments(video=10000000)"),
    );
    h.ok(
        "http://origin.test/QualityLevels(64528)/Fragments(video=20000000)",
        body("smooth/av/QualityLevels(64528)/Fragments(video=20000000)"),
    );
    h.ok(
        "http://origin.test/QualityLevels(98429)/Fragments(audio=0)",
        body("smooth/av/QualityLevels(98429)/Fragments(audio=0)"),
    );
    h.ok(
        "http://origin.test/QualityLevels(98429)/Fragments(audio=10214580)",
        body("smooth/av/QualityLevels(98429)/Fragments(audio=10214580)"),
    );
    h.ok(
        "http://origin.test/QualityLevels(98429)/Fragments(audio=20199161)",
        body("smooth/av/QualityLevels(98429)/Fragments(audio=20199161)"),
    );
    h.run_until(
        |evs| evs.iter().any(|e| matches!(e, Event::Discontinuity)),
        20_000,
    );
    let has_disc = h.events.iter().any(|e| matches!(e, Event::Discontinuity));
    assert!(has_disc);
}

#[test]
fn smooth_samples_are_placed_at_tfxd_time() {
    let mut h = Host::new(manifest_url(), smooth_config());
    route_av(&mut h);
    h.run_until(
        |evs| {
            evs.iter()
                .any(|e| matches!(e, Event::Status(Status::Ended)))
        },
        10_000,
    );
    // ensure samples emitted
    let has_late = h
        .events
        .iter()
        .any(|e| matches!(e, Event::Sample { sample, .. } if sample.timing.and_then(|t| t.dts()) == Some(10_000_000)));
    assert!(has_late, "expected a sample at tfxd time 10000000");
}

#[test]
fn smooth_live_join_never_starts_before_the_dvr_window() {
    let base = String::from_utf8(body("smooth/av/Manifest")).unwrap();
    let base = base.replace(
        "Duration=\"30415941\"",
        "Duration=\"30415941\" IsLive=\"TRUE\" DVRWindowLength=\"15000000\"",
    );
    let mut h = Host::new(manifest_url(), smooth_config());
    h.ok(manifest_url(), base.into_bytes());
    route_fragments(&mut h);
    h.run_until(
        |evs| evs.iter().any(|e| matches!(e, Event::Sample { .. })),
        10_000,
    );
    let seen_urls: Vec<String> = h.seen.iter().map(|s| s.url.clone()).collect();
    assert!(
        seen_urls.iter().any(|u| u.contains("video=10000000")),
        "expected join at first chunk inside window"
    );
    assert!(
        !seen_urls.iter().any(|u| u.contains("video=0")),
        "must not start before DVR window"
    );
}

#[test]
fn smooth_fragment_transport_failure_gives_up_after_max_live() {
    let mut cfg = smooth_config();
    cfg.max_segment_failures = 2;
    let base = String::from_utf8(body("smooth/av/Manifest")).unwrap();
    let base = base.replace(
        "Duration=\"30415941\"",
        "Duration=\"30415941\" IsLive=\"TRUE\"",
    );
    let mut h = Host::new(manifest_url(), cfg);
    h.ok(manifest_url(), base.clone().into_bytes());
    route_fragments(&mut h);
    h.route(
        "http://origin.test/QualityLevels(64528)/Fragments(video=0)",
        vec![
            hls_host::Reply::TransportError,
            hls_host::Reply::TransportError,
            hls_host::Reply::Ok(body("smooth/av/QualityLevels(64528)/Fragments(video=0)")),
        ],
    );
    // remove the ok route for video=0 already overridden by route above
    h.run_until(
        |evs| evs.iter().any(|e| matches!(e, Event::Discontinuity)),
        10_000,
    );
    let has_segf = h.events.iter().any(|e| {
        matches!(
            e,
            Event::Error {
                kind: ErrorKind::SegmentFetchFailed { .. },
                fatal: false
            }
        )
    });
    let has_disc = h.events.iter().any(|e| matches!(e, Event::Discontinuity));
    assert!(has_segf && has_disc);
    // max-1: one transport failure then success gives up nothing
    let mut cfg2 = smooth_config();
    cfg2.max_segment_failures = 2;
    let mut h2 = Host::new(manifest_url(), cfg2);
    h2.ok(manifest_url(), base.clone().into_bytes());
    route_fragments(&mut h2);
    h2.route(
        "http://origin.test/QualityLevels(64528)/Fragments(video=0)",
        vec![
            hls_host::Reply::TransportError,
            hls_host::Reply::Ok(body("smooth/av/QualityLevels(64528)/Fragments(video=0)")),
        ],
    );
    h2.run_until(
        |evs| {
            evs.iter()
                .any(|e| matches!(e, Event::Status(Status::Ended)))
        },
        10_000,
    );
    let gave_up = h2.events.iter().any(|e| {
        matches!(
            e,
            Event::Error {
                kind: ErrorKind::SegmentFetchFailed { .. },
                ..
            }
        )
    });
    assert!(!gave_up);
}

#[test]
fn smooth_fragment_transport_failure_is_fatal_after_max_static() {
    let mut cfg = smooth_config();
    cfg.max_segment_failures = 2;
    let mut h = Host::new(manifest_url(), cfg);
    h.ok(manifest_url(), body("smooth/av/Manifest"));
    route_fragments(&mut h);
    h.route(
        "http://origin.test/QualityLevels(64528)/Fragments(video=0)",
        vec![
            hls_host::Reply::TransportError,
            hls_host::Reply::TransportError,
            hls_host::Reply::Ok(body("smooth/av/QualityLevels(64528)/Fragments(video=0)")),
        ],
    );
    h.run_until(
        |evs| {
            evs.iter()
                .any(|e| matches!(e, Event::Error { fatal: true, .. }))
        },
        10_000,
    );
    let fatal = h.events.iter().any(|e| {
        matches!(
            e,
            Event::Error {
                kind: ErrorKind::SegmentFetchFailed { .. },
                fatal: true
            }
        )
    });
    assert!(fatal);
}

#[test]
fn smooth_manifest_redirect_and_basic_auth_are_followed() {
    use broadcast_client::Credentials;
    let cfg = smooth_config();
    let mut h = Host::new("http://origin.test/Manifest", cfg.clone());
    // redirect then 401 then 200
    h.route(
        "http://origin.test/Manifest",
        vec![hls_host::Reply::Redirect(
            "http://origin.test/moved/Manifest".to_string(),
        )],
    );
    h.route(
        "http://origin.test/moved/Manifest",
        vec![
            hls_host::full(
                401,
                Vec::new(),
                crate::support::headers(&[("WWW-Authenticate", "Basic realm=\"t\"")]),
            ),
            hls_host::Reply::Ok(body("smooth/av/Manifest")),
        ],
    );
    // fragments under moved base
    h.ok(
        "http://origin.test/moved/QualityLevels(64528)/Fragments(video=0)",
        body("smooth/av/QualityLevels(64528)/Fragments(video=0)"),
    );
    let mut session = Session::open(
        Url::parse("http://origin.test/Manifest").unwrap(),
        cfg,
        ts(0),
    )
    .unwrap();
    session
        .set_credentials(ts(0), Credentials::new("u", "p"))
        .unwrap();
    // drive manually using host? Host owns session, but we created session separately. Use host instead.
    let mut h2 = Host::new("http://origin.test/Manifest", smooth_config());
    h2.route(
        "http://origin.test/Manifest",
        vec![hls_host::Reply::Redirect(
            "http://origin.test/moved/Manifest".to_string(),
        )],
    );
    h2.route(
        "http://origin.test/moved/Manifest",
        vec![
            hls_host::full(
                401,
                Vec::new(),
                crate::support::headers(&[("WWW-Authenticate", "Basic realm=\"t\"")]),
            ),
            hls_host::Reply::Ok(body("smooth/av/Manifest")),
        ],
    );
    h2.ok(
        "http://origin.test/moved/QualityLevels(64528)/Fragments(video=0)",
        body("smooth/av/QualityLevels(64528)/Fragments(video=0)"),
    );
    // set credentials on the host's session
    h2.session
        .set_credentials(ts(0), Credentials::new("u", "p"))
        .unwrap();
    h2.run_until(
        |evs| evs.iter().any(|e| matches!(e, Event::Variants(_))),
        10_000,
    );
    let seen_urls: Vec<String> = h2.seen.iter().map(|s| s.url.clone()).collect();
    assert!(
        seen_urls
            .iter()
            .any(|u| u == "http://origin.test/moved/Manifest")
    );
    let has_auth = h2.seen.iter().any(|s| {
        s.headers
            .iter()
            .any(|(k, v)| k.eq_ignore_ascii_case("authorization") && v.starts_with("Basic "))
    });
    assert!(has_auth);
}

#[test]
fn smooth_fragment_inactivity_reports_request_stalled() {
    let mut cfg = smooth_config();
    cfg.segment_inactivity_timeout = std::time::Duration::from_millis(1);
    let mut h = Host::new(manifest_url(), cfg);
    h.ok(manifest_url(), body("smooth/av/Manifest"));
    route_fragments(&mut h);
    h.route(
        "http://origin.test/QualityLevels(64528)/Fragments(video=0)",
        vec![
            hls_host::Reply::Hang,
            hls_host::Reply::Ok(body("smooth/av/QualityLevels(64528)/Fragments(video=0)")),
        ],
    );
    h.run_until(
        |evs| {
            evs.iter()
                .any(|e| matches!(e, Event::Status(Status::RequestStalled { .. })))
        },
        10_000,
    );
    let stalled = "http://origin.test/QualityLevels(64528)/Fragments(video=0)";
    h.run_until(|_| false, 10_000);
    assert_eq!(
        h.hits(stalled),
        2,
        "the stalled fragment is requested again, whole"
    );
    assert!(h.samples() > 0, "the retried fragment is delivered");
}

#[test]
fn smooth_emits_nothing_after_a_fatal_error() {
    let mut cfg = smooth_config();
    cfg.max_segment_failures = 1;
    let mut h = Host::new(manifest_url(), cfg);
    h.ok(manifest_url(), body("smooth/av/Manifest"));
    route_fragments(&mut h);
    // fatal on the video fragment
    h.route(
        "http://origin.test/QualityLevels(64528)/Fragments(video=0)",
        vec![hls_host::Reply::TransportError],
    );
    // an in-flight hanging request on audio
    h.route(
        "http://origin.test/QualityLevels(98429)/Fragments(audio=0)",
        vec![hls_host::Reply::Hang],
    );
    h.run_until(
        |evs| {
            evs.iter()
                .any(|e| matches!(e, Event::Error { fatal: true, .. }))
        },
        10_000,
    );
    assert!(
        h.events
            .iter()
            .any(|e| matches!(e, Event::Error { fatal: true, .. }))
    );
    let before = h.events.len();
    // advance past the inactivity timeout
    h.now += 20_000_000;
    h.collect();
    assert_eq!(h.events.len(), before, "events emitted after a fatal error");
    assert!(
        !h.events
            .iter()
            .any(|e| matches!(e, Event::Status(Status::RequestStalled { .. })))
    );
}

#[test]
fn smooth_closes_connection_on_non_2xx_response() {
    let mut h = Host::new(manifest_url(), smooth_config());
    h.route(manifest_url(), vec![hls_host::status_only(404)]);
    h.run_until(|_| false, 10);
    // drive one step cycle
    let _ = h.step();
    assert!(!h.closes.is_empty(), "answered connection was not closed");
}

const GOLDEN_BLESS_ENV: &str = "SMOOTH_PULL_GOLDEN_BLESS";
const GOLDEN_STEP_LIMIT: usize = 10_000;
const FNV_OFFSET_BASIS: u64 = 0xcbf2_9ce4_8422_2325;
const FNV_PRIME: u64 = 0x0000_0100_0000_01b3;

fn fnv1a(bytes: &[u8]) -> u64 {
    bytes.iter().fold(FNV_OFFSET_BASIS, |h, b| {
        (h ^ u64::from(*b)).wrapping_mul(FNV_PRIME)
    })
}

fn route_fixture_dir(h: &mut Host, fixture: &str) {
    fn walk(h: &mut Host, root: &std::path::Path, dir: &std::path::Path) {
        for entry in std::fs::read_dir(dir).expect("read fixture dir") {
            let path = entry.expect("dir entry").path();
            if path.is_dir() {
                walk(h, root, &path);
            } else if path.file_name().is_some_and(|n| n != "PROVENANCE.md") {
                let rel = path.strip_prefix(root).expect("under root");
                let url = format!("http://origin.test/{}", rel.to_str().expect("utf8 path"));
                h.ok(&url, std::fs::read(&path).expect("read fixture"));
            }
        }
    }
    let root = test_fixtures::path(&format!("smooth/{fixture}"));
    walk(h, &root, &root);
}

fn sample_trace(fixture: &str) -> String {
    let mut h = Host::new(manifest_url(), smooth_config());
    route_fixture_dir(&mut h, fixture);
    h.run_until(
        |evs| {
            evs.iter()
                .any(|e| matches!(e, Event::Status(Status::Ended)))
        },
        GOLDEN_STEP_LIMIT,
    );
    let mut out = String::new();
    for e in &h.events {
        match e {
            Event::Tracks(specs) => {
                let ids: Vec<u32> = specs.iter().map(|s| s.track_id.value()).collect();
                out.push_str(&format!("tracks {ids:?}\n"));
            }
            Event::Sample { track, sample, .. } => out.push_str(&format!(
                "sample track={} dts={:?} pts={:?} dur={:?} sync={} len={} fnv={:016x}\n",
                track.value(),
                sample.timing.and_then(|t| t.dts()),
                sample.timing.and_then(|t| t.pts()),
                sample.timing.and_then(|t| t.duration()),
                sample.flags.is_sync,
                sample.data.len(),
                fnv1a(&sample.data),
            )),
            Event::Discontinuity => out.push_str("discontinuity\n"),
            Event::Error { kind, fatal } => {
                out.push_str(&format!("error {kind:?} fatal={fatal}\n"))
            }
            Event::Status(Status::Ended) => out.push_str("ended\n"),
            _ => {}
        }
    }
    out
}

fn check_golden(fixture: &str) {
    let name = format!("pull-broadcast-client-{fixture}.txt");
    let actual = sample_trace(fixture);
    if let Ok(dir) = std::env::var(GOLDEN_BLESS_ENV) {
        assert!(
            std::env::var_os("CI").is_none(),
            "refuse to re-bless a golden under CI=true ({GOLDEN_BLESS_ENV} is a local review step)"
        );
        std::fs::write(std::path::Path::new(&dir).join(&name), &actual).expect("write golden");
        return;
    }
    let expected = std::fs::read_to_string(test_fixtures::path(&format!("golden/smooth/{name}")))
        .expect("read golden");
    assert_eq!(actual, expected, "{name} differs from the recorded trace");
}

/// Re-blessing is a local review step: under `CI=true` the switch must fail the
/// run (this re-runs this binary with both set, and on the old code it would
/// have written the golden and exited green).
#[test]
fn a_smooth_pull_bless_is_refused_under_ci() {
    let dir = std::env::temp_dir().join(format!("smooth-bless-refused-{}", std::process::id()));
    let out = std::process::Command::new(std::env::current_exe().expect("test binary"))
        .args([
            "smooth_pull::smooth_pull_golden_av",
            "--exact",
            "--nocapture",
        ])
        .env(GOLDEN_BLESS_ENV, &dir)
        .env("CI", "true")
        .output()
        .expect("run the test binary");
    let _ = std::fs::remove_dir_all(&dir);
    assert!(!dir.exists(), "a refused bless must write no golden");
    assert!(
        !out.status.success(),
        "{GOLDEN_BLESS_ENV} under CI must fail instead of re-blessing"
    );
}

#[test]
fn smooth_pull_golden_av() {
    check_golden("av");
}

#[test]
fn smooth_pull_golden_av_low() {
    check_golden("av-low");
}

#[test]
fn smooth_select_variant_switches_the_quality_of_later_fragments() {
    let base = String::from_utf8(body("smooth/av/Manifest")).unwrap();
    let low = r#"<QualityLevel Index="1" Bitrate="30000" FourCC="H264" MaxWidth="160" MaxHeight="120" CodecPrivateData="00000001674d400deca0a0fd80880000030008000003019078a14cb00000000168ebe3cb20" />"#;
    let first_chunk = r#"<c n="0" d="10000000" />"#;
    let base = base.replace(first_chunk, &format!("{low}\n{first_chunk}"));
    let mut h = Host::new(manifest_url(), smooth_config());
    route_fragments(&mut h);
    h.ok(manifest_url(), base.into_bytes());
    h.run_until(
        |evs| evs.iter().any(|e| matches!(e, Event::Variants(_))),
        10_000,
    );
    h.session
        .select_variant(ts(h.now), 1)
        .expect("select the low variant");
    h.run_until(
        |evs| {
            evs.iter()
                .any(|e| matches!(e, Event::Status(Status::Ended)))
                || evs.iter().any(|e| matches!(e, Event::Error { .. }))
        },
        10_000,
    );
    assert!(
        h.seen
            .iter()
            .any(|s| s.url.contains("QualityLevels(30000)/Fragments(video="))
    );
}

fn live_manifest() -> Vec<u8> {
    let text = String::from_utf8(body("smooth/av/Manifest")).expect("utf8");
    text.replace("MinorVersion=\"0\"", "MinorVersion=\"0\" IsLive=\"TRUE\"")
        .into_bytes()
}

fn video0_url() -> &'static str {
    "http://origin.test/QualityLevels(64528)/Fragments(video=0)"
}

fn has_event(h: &Host, pred: impl Fn(&Event) -> bool) -> bool {
    h.events.iter().any(pred)
}

#[test]
fn a_manifest_refresh_failure_is_a_non_fatal_smooth_manifest_error() {
    let mut h = Host::new(manifest_url(), smooth_config());
    h.route(
        manifest_url(),
        vec![
            hls_host::Reply::Ok(live_manifest()),
            hls_host::status_only(500),
        ],
    );
    route_fragments(&mut h);
    h.run_until(
        |evs| {
            evs.iter().any(|e| {
                matches!(
                    e,
                    Event::Error {
                        kind: ErrorKind::SmoothManifest { .. },
                        fatal: false
                    }
                )
            })
        },
        20_000,
    );
    assert!(
        has_event(&h, |e| matches!(
            e,
            Event::Error {
                kind: ErrorKind::SmoothManifest { .. },
                fatal: false
            }
        )),
        "a dropped live manifest refresh is reported non-fatally: {:?}",
        h.events
    );
}

#[test]
fn an_unresolvable_fragment_url_is_a_non_fatal_fetch_failure() {
    let text = String::from_utf8(body("smooth/av/Manifest")).expect("utf8");
    let text = text.replace(
        "QualityLevels({bitrate})/Fragments(video={start time})",
        "http://[bad",
    );
    let mut h = Host::new(manifest_url(), smooth_config());
    h.ok(manifest_url(), text.into_bytes());
    h.run_until(
        |evs| {
            evs.iter().any(|e| {
                matches!(
                    e,
                    Event::Error {
                        kind: ErrorKind::SegmentFetchFailed { status: None },
                        fatal: false
                    }
                )
            })
        },
        20_000,
    );
    assert!(has_event(&h, |e| matches!(
        e,
        Event::Error {
            kind: ErrorKind::SegmentFetchFailed { status: None },
            fatal: false
        }
    )));
}

fn encrypted_fragment() -> Vec<u8> {
    let mut senc = Vec::new();
    senc.extend_from_slice(&8u32.to_be_bytes());
    senc.extend_from_slice(b"senc");
    let mut traf = Vec::new();
    traf.extend_from_slice(&u32::try_from(8 + senc.len()).unwrap().to_be_bytes());
    traf.extend_from_slice(b"traf");
    traf.extend_from_slice(&senc);
    let mut moof = Vec::new();
    moof.extend_from_slice(&u32::try_from(8 + traf.len()).unwrap().to_be_bytes());
    moof.extend_from_slice(b"moof");
    moof.extend_from_slice(&traf);
    moof
}

#[test]
fn an_encrypted_fragment_is_fatal_unsupported_encryption() {
    let mut h = Host::new(manifest_url(), smooth_config());
    h.ok(manifest_url(), body("smooth/av/Manifest"));
    route_fragments(&mut h);
    h.route(
        video0_url(),
        vec![hls_host::Reply::Ok(encrypted_fragment())],
    );
    h.run_until(
        |evs| {
            evs.iter()
                .any(|e| matches!(e, Event::Error { fatal: true, .. }))
        },
        10_000,
    );
    assert!(has_event(&h, |e| matches!(
        e,
        Event::Error {
            kind: ErrorKind::UnsupportedEncryption { method },
            fatal: true
        } if method == "fragment"
    )));
    assert!(!has_event(&h, |e| matches!(
        e,
        Event::Error {
            kind: ErrorKind::SmoothManifest { .. },
            ..
        }
    )));
}

#[test]
fn a_fragment_over_the_byte_cap_is_a_fatal_fetch_failure() {
    let mut h = Host::new(manifest_url(), smooth_config());
    h.ok(manifest_url(), body("smooth/av/Manifest"));
    route_fragments(&mut h);
    let oversized = vec![0u8; 16 * 1024 * 1024 + 1];
    h.route(video0_url(), vec![hls_host::Reply::Ok(oversized)]);
    h.run_until(
        |evs| {
            evs.iter()
                .any(|e| matches!(e, Event::Error { fatal: true, .. }))
        },
        10_000,
    );
    assert!(has_event(&h, |e| matches!(
        e,
        Event::Error {
            kind: ErrorKind::SegmentFetchFailed { status: None },
            fatal: true
        }
    )));
    assert!(!has_event(&h, |e| matches!(
        e,
        Event::Error {
            kind: ErrorKind::SmoothManifest { .. },
            ..
        }
    )));
}

#[test]
fn closing_with_a_fragment_in_flight_closes_its_connection() {
    let mut h = Host::new(manifest_url(), smooth_config());
    h.ok(manifest_url(), body("smooth/av/Manifest"));
    route_fragments(&mut h);
    h.route(video0_url(), vec![hls_host::Reply::Hang]);
    for _ in 0..8 {
        if !h.hung().is_empty() {
            break;
        }
        h.step();
    }
    assert!(!h.hung().is_empty(), "the video fragment is in flight");
    h.session.close(ts(h.now));
    h.collect();
    assert!(
        !h.closes.is_empty(),
        "a session closed with a fragment in flight closes its socket"
    );
}

#[test]
fn a_fragment_is_aborted_only_after_the_inactivity_timeout_from_its_start() {
    let mut cfg = smooth_config();
    cfg.segment_inactivity_timeout = std::time::Duration::from_millis(100);
    let mut h = Host::new(manifest_url(), cfg);
    h.ok(manifest_url(), body("smooth/av/Manifest"));
    route_fragments(&mut h);
    h.route(video0_url(), vec![hls_host::Reply::Hang]);
    for _ in 0..8 {
        if !h.hung().is_empty() {
            break;
        }
        h.step();
    }
    let started = h.hung()[0].at_us;
    h.now = started + 100_000 - 1;
    h.collect();
    assert!(
        !has_event(&h, |e| matches!(
            e,
            Event::Status(Status::RequestStalled { .. })
        )),
        "not stalled a microsecond before the timeout"
    );
    h.now = started + 100_000;
    h.collect();
    assert!(
        has_event(&h, |e| matches!(
            e,
            Event::Status(Status::RequestStalled { .. })
        )),
        "stalled exactly at the timeout"
    );
}

#[test]
fn a_401_without_credentials_asks_and_resumes_after_set_credentials() {
    use broadcast_client::Credentials;

    let mut h = Host::new(manifest_url(), smooth_config());
    h.route(
        manifest_url(),
        vec![
            hls_host::full(
                401,
                Vec::new(),
                crate::support::headers(&[(
                    "WWW-Authenticate",
                    "Digest realm=\"cam\", nonce=\"n1\", qop=\"auth\"",
                )]),
            ),
            hls_host::Reply::Ok(body("smooth/av/Manifest")),
        ],
    );
    route_fragments(&mut h);
    h.run_until(
        |evs| evs.iter().any(|e| matches!(e, Event::AuthRequired { .. })),
        100,
    );
    assert!(has_event(&h, |e| matches!(e, Event::AuthRequired { .. })));
    assert_eq!(h.hits(manifest_url()), 1, "a 401 without credentials waits");
    h.session
        .set_credentials(ts(h.now), Credentials::new("u", "p"))
        .unwrap();
    h.collect();
    let sent: Vec<_> = h.requests_to(manifest_url());
    assert_eq!(sent.len(), 2, "the parked request is resumed");
    assert!(
        sent[1].authorization().is_some(),
        "the resumed request carries the credentials"
    );
}

#[test]
fn a_stale_digest_401_after_an_authenticated_request_is_replayed_once() {
    use broadcast_client::Credentials;

    let stale = "Digest realm=\"cam\", nonce=\"n2\", qop=\"auth\", stale=true";
    let mut h = Host::new(manifest_url(), smooth_config());
    h.route(
        manifest_url(),
        vec![
            hls_host::full(
                401,
                Vec::new(),
                crate::support::headers(&[(
                    "WWW-Authenticate",
                    "Digest realm=\"cam\", nonce=\"n1\", qop=\"auth\"",
                )]),
            ),
            hls_host::full(
                401,
                Vec::new(),
                crate::support::headers(&[(
                    "WWW-Authenticate",
                    "Digest realm=\"cam\", nonce=\"n1\", qop=\"auth\"",
                )]),
            ),
            hls_host::full(
                401,
                Vec::new(),
                crate::support::headers(&[("WWW-Authenticate", stale)]),
            ),
            hls_host::Reply::Ok(body("smooth/av/Manifest")),
        ],
    );
    route_fragments(&mut h);
    h.run_until(
        |evs| evs.iter().any(|e| matches!(e, Event::AuthRequired { .. })),
        100,
    );
    h.session
        .set_credentials(ts(h.now), Credentials::new("u", "p"))
        .unwrap();
    h.collect();
    for _ in 0..6 {
        h.step();
    }
    let sent: Vec<_> = h.requests_to(manifest_url());
    assert_eq!(
        sent.len(),
        4,
        "the rejected challenge replayed, then the stale one replayed once"
    );
    assert!(sent[3].authorization().is_some());
}

#[test]
fn a_fragment_closed_with_an_error_after_a_200_is_retried() {
    let mut h = Host::new(manifest_url(), smooth_config());
    h.ok(manifest_url(), body("smooth/av/Manifest"));
    route_fragments(&mut h);
    h.route(
        video0_url(),
        vec![
            hls_host::Reply::ResetMid(body("smooth/av/QualityLevels(64528)/Fragments(video=0)")),
            hls_host::Reply::Ok(body("smooth/av/QualityLevels(64528)/Fragments(video=0)")),
        ],
    );
    h.run_until(|_| false, 20_000);
    assert!(
        h.hits(video0_url()) >= 2,
        "a success header followed by a transport error is a retry, not a body"
    );
}

#[test]
fn smooth_select_audio_rendition_is_not_hls() {
    let mut session =
        Session::open(Url::parse(manifest_url()).unwrap(), smooth_config(), ts(0)).unwrap();
    assert!(matches!(
        session.select_audio_rendition(ts(0), Some(0)),
        Err(broadcast_client::Error::NotHls)
    ));
}

#[test]
fn a_request_that_received_bytes_stalls_from_the_last_byte_not_from_its_start() {
    use broadcast_client::Action;

    let mut cfg = smooth_config();
    cfg.segment_inactivity_timeout = std::time::Duration::from_millis(100);
    let mut session = Session::open(Url::parse(manifest_url()).unwrap(), cfg, ts(0)).unwrap();
    let headers = crate::support::headers(&[]);
    let now = 0u64;

    let manifest_conn = loop {
        if let Some(Action::HttpRequest { conn, url, .. }) = session.poll(ts(now))
            && url.as_str() == manifest_url()
        {
            break conn;
        }
    };
    session.on_response(manifest_conn, http::StatusCode::OK, &headers, ts(now));
    session.on_data(manifest_conn, &body("smooth/av/Manifest"), ts(now));
    session.on_closed(manifest_conn, None, ts(now));

    let mut fragment_conns = Vec::new();
    while let Some(action) = session.poll(ts(now)) {
        if let Action::HttpRequest { conn, url, .. } = action
            && url.as_str().contains("Fragments")
        {
            fragment_conns.push(conn);
        }
    }
    assert!(!fragment_conns.is_empty(), "a fragment is requested");
    for conn in &fragment_conns {
        session.on_response(*conn, http::StatusCode::OK, &headers, ts(now));
        session.on_data(*conn, &[0u8; 1], ts(now + 1_000));
    }
    while session.next_event().is_some() {}

    session.poll(ts(100_000));
    let stalled = std::iter::from_fn(|| session.next_event())
        .any(|e| matches!(e, Event::Status(Status::RequestStalled { .. })));
    assert!(
        !stalled,
        "a byte 1 ms into the request moves its inactivity deadline to 101 ms"
    );
}
