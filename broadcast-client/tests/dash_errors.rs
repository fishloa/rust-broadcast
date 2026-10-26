//! The error surface of a DASH `Session`: every `ErrorKind` the DASH source can emit is reachable
//! from a real or a minimally edited real input, is reported once until a success resets it, and
//! ends the session only when it is fatal.

#[path = "support/dash_checks.rs"]
mod dash_checks;
#[path = "support/dash_common.rs"]
mod dash_common;
#[path = "support/dash_sim.rs"]
mod dash_sim;

use broadcast_client::{BufferEvent, Config, ErrorKind, Event, Status};
use dash_common::loopback::Origin;
use dash_common::synth::{Addressing, MPD_URL, Synth};
use dash_common::world::{MICROS, World};
use dash_sim::{Answer, DashSim, Logged, SEC, StaticBackend, WorldBackend};

const HOST: &str = "http://dash.test";
const MANIFEST: &str = "manifest.mpd";
const AST_US: i64 = 1_700_000_000 * MICROS;

fn url_of(rel: &str) -> String {
    format!("{HOST}/{rel}")
}

fn text_sim(rel: &str, text: String, config: Config) -> DashSim {
    let url = url_of(rel);
    let origin = Origin::new().doc(&url, text.into_bytes());
    DashSim::new(
        &url,
        config,
        World::new(AST_US),
        Box::new(StaticBackend(origin)),
    )
}

fn manifest_text() -> String {
    test_fixtures::read_to_string(&format!("dash/{MANIFEST}"))
}

fn finished(mut sim: DashSim) -> DashSim {
    sim.run_to_terminal(900 * SEC);
    sim
}

fn kinds(sim: &DashSim) -> Vec<(ErrorKind, bool)> {
    sim.errors()
}

fn is_mpd(l: &Logged) -> bool {
    l.is_mpd()
}

fn serve_mpd_with(sim: &mut DashSim, body: &'static [u8]) {
    sim.fault = Some(Box::new(move |req: &Logged, _| {
        is_mpd(req).then(|| Answer::ok(body.to_vec()))
    }));
}

#[test]
fn an_mpd_that_is_not_xml_is_a_parse_error_reported_once_and_fatal_at_the_key_failure_limit() {
    let mut config = Config::default();
    config.max_key_failures = 3;
    let mut sim = DashSim::static_origin(&url_of(MANIFEST), config);
    serve_mpd_with(&mut sim, b"<html><body>captive portal</body></html>");
    let sim = finished(sim);
    assert_eq!(
        kinds(&sim),
        vec![
            (ErrorKind::MpdParse { reason: "not_mpd" }, false),
            (ErrorKind::MpdParse { reason: "not_mpd" }, true),
        ]
    );
    assert_eq!(sim.log.iter().filter(|l| l.is_mpd()).count(), 3);
    let gaps: Vec<u64> = sim
        .log
        .windows(2)
        .map(|w| (w[1].send_elapsed_us - w[0].send_elapsed_us) / SEC)
        .collect();
    assert_eq!(gaps, [2, 4], "the 2 s then 4 s backoff steps");
    assert_eq!(sim.count(|e| matches!(e, Event::Sample { .. })), 0);
}

#[test]
fn each_mpd_parse_failure_names_its_reason() {
    let cases: [(&'static [u8], &str); 3] = [
        (b"<MPD xmlns=\"urn:mpeg:dash:schema:mpd:2011\"", "xml"),
        (&[0xff, 0xfe, 0x00, 0x80], "invalid_utf8"),
        (b"<Other/>", "not_mpd"),
    ];
    for (body, reason) in cases {
        let mut config = Config::default();
        config.max_key_failures = 1;
        let mut sim = DashSim::static_origin(&url_of(MANIFEST), config);
        serve_mpd_with(&mut sim, body);
        let sim = finished(sim);
        assert_eq!(
            sim.fatal(),
            Some(ErrorKind::MpdParse { reason }),
            "{reason}"
        );
    }
}

#[test]
fn a_good_mpd_after_failures_resets_the_failure_count() {
    let mut config = Config::default();
    config.max_key_failures = 3;
    let mut sim = DashSim::static_origin(&url_of(MANIFEST), config);
    let mut bad = 2;
    sim.fault = Some(Box::new(move |req: &Logged, _| {
        if is_mpd(req) && bad > 0 {
            bad -= 1;
            return Some(Answer::ok(b"nonsense".to_vec()));
        }
        None
    }));
    let sim = finished(sim);
    assert!(sim.ended(), "{:?}", kinds(&sim));
    assert_eq!(
        kinds(&sim),
        vec![(ErrorKind::MpdParse { reason: "xml" }, false)],
        "two bad bodies stay below the limit and are reported once"
    );
}

#[test]
fn a_list_presentation_is_fatal_unsupported_addressing() {
    let text = manifest_text().replacen("type=\"static\"", "type=\"list\"", 1);
    let sim = finished(text_sim(MANIFEST, text, Config::default()));
    assert_eq!(
        kinds(&sim),
        vec![(
            ErrorKind::UnsupportedAddressing {
                reason: "presentation_type_list"
            },
            true
        )]
    );
    assert_eq!(sim.log.len(), 1, "nothing is fetched after the refusal");
}

#[test]
fn an_mpd_nothing_can_play_is_fatal_mpd_invalid() {
    let no_period = "<MPD xmlns=\"urn:mpeg:dash:schema:mpd:2011\" type=\"static\" mediaPresentationDuration=\"PT3S\"></MPD>";
    let sim = finished(text_sim(MANIFEST, no_period.to_owned(), Config::default()));
    assert_eq!(
        kinds(&sim),
        vec![(
            ErrorKind::MpdInvalid {
                reason: "no_period"
            },
            true
        )]
    );
    let dynamic_without_ast = manifest_text().replacen("type=\"static\"", "type=\"dynamic\"", 1);
    let sim = finished(text_sim(MANIFEST, dynamic_without_ast, Config::default()));
    assert_eq!(
        sim.fatal(),
        Some(ErrorKind::MpdInvalid {
            reason: "missing_availability_start_time"
        })
    );
}

#[test]
fn cross_origin_urls_off_refuses_a_base_url_on_another_origin() {
    let text = manifest_text().replacen(
        "<Period",
        "<BaseURL>http://cdn.test/media/</BaseURL><Period",
        1,
    );
    let mut config = Config::default();
    config.dash.cross_origin_urls = false;
    let sim = finished(text_sim(MANIFEST, text.clone(), config));
    assert_eq!(
        kinds(&sim),
        vec![
            (
                ErrorKind::UnsupportedAddressing {
                    reason: "cross_origin"
                },
                false
            ),
            (
                ErrorKind::MpdInvalid {
                    reason: "no_playable_representation"
                },
                true
            ),
        ],
        "reported once for both sets, then nothing is left to play"
    );
    assert!(
        sim.log.iter().all(|l| !l.url.contains("cdn.test")),
        "no request ever reaches the foreign origin"
    );
    let sim = finished(text_sim(MANIFEST, text, Config::default()));
    assert!(
        sim.log.iter().any(|l| l.url.contains("cdn.test")),
        "with the default the MPD may name a CDN"
    );
}

#[test]
fn unsupported_features_are_reported_once_each_and_playback_goes_on() {
    let text = manifest_text()
        .replacen(
            "<Period id=\"0\" start=\"PT0.0S\">",
            "<Period id=\"0\" start=\"PT0.0S\"><EventStream schemeIdUri=\"urn:x:events\"/><Preselection id=\"pre\" preselectionComponents=\"0 1\"/>",
            1,
        )
        .replacen(
            "<ProgramInformation>",
            "<PatchLocation ttl=\"60\">patch.mpp</PatchLocation><ProgramInformation>",
            1,
        );
    let sim = finished(text_sim(MANIFEST, text, Config::default()));
    assert!(sim.ended(), "{:?}", kinds(&sim));
    let features: Vec<&str> = kinds(&sim)
        .iter()
        .filter_map(|(k, fatal)| match k {
            ErrorKind::UnsupportedFeature { feature } if !fatal => Some(*feature),
            _ => None,
        })
        .collect();
    let mut sorted = features.clone();
    sorted.sort_unstable();
    assert_eq!(sorted, ["event_stream", "mpd_patch", "preselection"]);
    assert!(kinds(&sim).iter().all(|(_, fatal)| !fatal));
}

#[test]
fn a_low_latency_live_mpd_plays_at_segment_granularity_with_one_notice() {
    let ll = |at: i64| {
        let mut base = Synth::new(AST_US, Addressing::Number)
            .with_utc("http-xsdate", "http://utc.test/xsdate");
        let mpd = base.mpd_at(at);
        mpd.replacen(
            "<SegmentTemplate ",
            "<SegmentTemplate availabilityTimeOffset=\"1.5\" availabilityTimeComplete=\"false\" ",
            2,
        )
    };
    let mut synth =
        Synth::new(AST_US, Addressing::Number).with_utc("http-xsdate", "http://utc.test/xsdate");
    synth.mpd_override = Some(Box::new(move |at| Some(ll(at))));
    let mut sim = DashSim::new(
        MPD_URL,
        Config::default(),
        World::new(AST_US + 41 * MICROS + 120_000),
        Box::new(WorldBackend(synth)),
    );
    sim.run_for(30 * SEC);
    let low: Vec<_> = kinds(&sim)
        .into_iter()
        .filter(|(k, _)| {
            *k == ErrorKind::UnsupportedFeature {
                feature: "low_latency",
            }
        })
        .collect();
    assert_eq!(low.len(), 1, "{:?}", kinds(&sim));
    assert!(sim.count(|e| matches!(e, Event::Sample { .. })) > 0);
    assert!(sim.fatal().is_none());
}

#[test]
fn protected_content_is_reported_and_all_protected_content_is_fatal_unsupported_encryption() {
    let protection =
        "<ContentProtection schemeIdUri=\"urn:mpeg:dash:mp4protection:2011\" value=\"cenc\"/>";
    let both = manifest_text().replace("<Representation", &format!("{protection}<Representation"));
    let sim = finished(text_sim(MANIFEST, both, Config::default()));
    let method = "content_protection".to_owned();
    assert_eq!(
        kinds(&sim),
        vec![
            (
                ErrorKind::UnsupportedEncryption {
                    method: method.clone()
                },
                false
            ),
            (
                ErrorKind::UnsupportedEncryption {
                    method: method.clone()
                },
                true
            ),
        ]
    );
    let video_only = manifest_text().replacen(
        "<Representation",
        &format!("{protection}<Representation"),
        1,
    );
    let sim = finished(text_sim(MANIFEST, video_only, Config::default()));
    assert!(sim.ended(), "the clear audio plays: {:?}", kinds(&sim));
    assert_eq!(
        kinds(&sim),
        vec![(ErrorKind::UnsupportedEncryption { method }, false)]
    );
    let tracks: Vec<usize> = sim
        .events
        .iter()
        .filter_map(|e| match e {
            Event::Tracks(t) => Some(t.len()),
            _ => None,
        })
        .collect();
    assert_eq!(tracks, [1]);
}

#[test]
fn a_subtitle_set_is_dropped_once_with_a_track_dropped_error() {
    let text = manifest_text().replacen(
        "</Period>",
        "<AdaptationSet id=\"5\" contentType=\"text\" mimeType=\"application/mp4\" codecs=\"stpp\" lang=\"en\"><Representation id=\"t\" bandwidth=\"1000\"><SegmentTemplate timescale=\"1000\" duration=\"1000\" initialization=\"t-init.m4s\" media=\"t-$Number$.m4s\"/></Representation></AdaptationSet></Period>",
        1,
    );
    let sim = finished(text_sim(MANIFEST, text, Config::default()));
    assert!(sim.ended());
    assert_eq!(
        kinds(&sim),
        vec![(
            ErrorKind::TrackDropped {
                track_id: 0,
                stream_type: None,
                fourcc: None
            },
            false
        )]
    );
}

fn progressive() -> Vec<u8> {
    test_fixtures::read("mp4/progressive/av_prog.mp4")
}

#[test]
fn a_progressive_mp4_served_as_a_segment_is_a_fatal_unsupported_container() {
    let mut sim = DashSim::static_origin(&url_of(MANIFEST), Config::default());
    sim.fault = Some(Box::new(|req: &Logged, _| {
        req.url
            .ends_with("chunk-stream0-00001.m4s")
            .then(|| Answer::ok(progressive()))
    }));
    let sim = finished(sim);
    assert_eq!(
        sim.fatal(),
        Some(ErrorKind::UnsupportedContainer {
            reason: "non_fragmented_mp4"
        })
    );
    assert!(!sim.ended());
}

#[test]
fn a_vod_segment_that_keeps_failing_is_fatal_after_the_configured_count_with_backoff() {
    let mut config = Config::default();
    config.max_segment_failures = 3;
    let mut sim = DashSim::static_origin(&url_of(MANIFEST), config);
    sim.fault = Some(Box::new(|req: &Logged, _| {
        req.url
            .ends_with("chunk-stream0-00002.m4s")
            .then(|| Answer::status(404))
    }));
    let sim = finished(sim);
    assert_eq!(
        sim.fatal(),
        Some(ErrorKind::SegmentFetchFailed { status: Some(404) })
    );
    let at: Vec<u64> = sim
        .requests_to("chunk-stream0-00002")
        .iter()
        .map(|l| l.send_elapsed_us)
        .collect();
    assert_eq!(at.len(), 3);
    assert_eq!(
        [(at[1] - at[0]) / SEC, (at[2] - at[1]) / SEC],
        [2, 4],
        "the shared 2 s / 4 s backoff"
    );
    assert!(!sim.ended());
    let after = sim.log.len();
    let mut sim = sim;
    sim.run_for(60 * SEC);
    assert_eq!(sim.log.len(), after, "silent after a fatal error");
}

#[test]
fn a_live_segment_that_keeps_failing_is_skipped_with_one_error_and_a_discontinuity() {
    let mut config = Config::default();
    config.max_segment_failures = 2;
    config.backoff_ms = vec![100];
    config.backoff_cap_ms = 100;
    let synth =
        Synth::new(AST_US, Addressing::Number).with_utc("http-xsdate", "http://utc.test/xsdate");
    let geo = synth.geo();
    let mut sim = DashSim::new(
        MPD_URL,
        config,
        World::new(AST_US + 41 * MICROS + 120_000),
        Box::new(WorldBackend(synth)),
    );
    let mut victim: Option<String> = None;
    sim.fault = Some(Box::new(move |req: &Logged, _| {
        if req.url.contains("/V/") && req.url.ends_with(".m4s") && !req.url.contains("init") {
            let v = victim.get_or_insert_with(|| req.url.clone());
            if *v == req.url {
                return Some(Answer::status(500));
            }
        }
        None
    }));
    sim.run_for(30 * SEC);
    assert!(sim.fatal().is_none(), "{:?}", kinds(&sim));
    let skipped = kinds(&sim)
        .into_iter()
        .filter(|(k, _)| *k == ErrorKind::SegmentFetchFailed { status: Some(500) })
        .count();
    assert_eq!(skipped, 1);
    let order: Vec<&str> = sim
        .events
        .iter()
        .filter_map(|e| match e {
            Event::Error { kind, .. } if kind.name() == "segment_fetch_failed" => Some("error"),
            Event::Discontinuity => Some("discontinuity"),
            _ => None,
        })
        .collect();
    assert_eq!(&order[..2], ["error", "discontinuity"], "{order:?}");
    let _ = geo;
}

#[test]
fn an_init_that_keeps_failing_is_fatal_init_fetch_failed_and_retry_after_is_honoured() {
    let mut config = Config::default();
    config.max_key_failures = 3;
    let mut sim = DashSim::static_origin(&url_of(MANIFEST), config);
    sim.fault = Some(Box::new(|req: &Logged, _| {
        req.url
            .ends_with("init-stream0.m4s")
            .then(|| Answer::status_with(503, vec![("Retry-After".into(), "7".into())]))
    }));
    let sim = finished(sim);
    assert_eq!(
        sim.fatal(),
        Some(ErrorKind::InitFetchFailed { status: Some(503) })
    );
    let at: Vec<u64> = sim
        .requests_to("init-stream0")
        .iter()
        .map(|l| l.send_elapsed_us)
        .collect();
    assert_eq!(at.len(), 3);
    assert_eq!(
        (at[1] - at[0]) / SEC,
        7,
        "Retry-After 7 s beats the 2 s step"
    );
    assert_eq!((at[2] - at[1]) / SEC, 7);
}

#[test]
fn an_undecodable_init_is_init_decode_failed_then_fatal_at_the_limit() {
    let mut config = Config::default();
    config.max_key_failures = 3;
    let mut sim = DashSim::static_origin(&url_of(MANIFEST), config);
    sim.fault = Some(Box::new(|req: &Logged, _| {
        req.url
            .ends_with("init-stream0.m4s")
            .then(|| Answer::ok(vec![0u8; 400]))
    }));
    let sim = finished(sim);
    assert_eq!(
        kinds(&sim),
        vec![
            (ErrorKind::InitDecodeFailed, false),
            (ErrorKind::InitDecodeFailed, true)
        ]
    );
}

#[test]
fn an_oversize_segment_body_is_reported_once_and_counted_as_a_failure() {
    let mut config = Config::default();
    config.max_segment_bytes = 5_000;
    config.max_segment_failures = 2;
    let sim = finished(DashSim::static_origin(&url_of(MANIFEST), config));
    let names: Vec<ErrorKind> = kinds(&sim).into_iter().map(|(k, _)| k).collect();
    assert_eq!(
        names
            .iter()
            .filter(|k| **k
                == ErrorKind::BodyTooLarge {
                    field: "max_segment_bytes"
                })
            .count(),
        1
    );
    assert!(matches!(
        sim.fatal(),
        Some(ErrorKind::SegmentFetchFailed { status: None })
    ));
}

#[test]
fn an_oversize_mpd_is_reported_once_and_retried_forever_with_backoff() {
    let mut config = Config::default();
    config.max_manifest_bytes = 100;
    let mut sim = DashSim::static_origin(&url_of(MANIFEST), config);
    sim.run_for(300 * SEC);
    assert!(sim.fatal().is_none());
    assert_eq!(
        kinds(&sim),
        vec![(
            ErrorKind::BodyTooLarge {
                field: "max_manifest_bytes"
            },
            false
        )]
    );
    let n = sim.log.iter().filter(|l| l.is_mpd()).count();
    assert!(
        n > 10 && n < 60,
        "bounded retry rate: {n} requests in 300 s"
    );
}

#[test]
fn errors_are_reported_again_after_a_success_resets_them() {
    let mut config = Config::default();
    config.max_segment_failures = 2;
    config.backoff_ms = vec![50];
    config.backoff_cap_ms = 50;
    let synth =
        Synth::new(AST_US, Addressing::Number).with_utc("http-xsdate", "http://utc.test/xsdate");
    let mut sim = DashSim::new(
        MPD_URL,
        config,
        World::new(AST_US + 41 * MICROS + 120_000),
        Box::new(WorldBackend(synth)),
    );
    let mut victims: Vec<String> = Vec::new();
    sim.fault = Some(Box::new(move |req: &Logged, _| {
        if req.url.contains("/V/") && req.url.ends_with(".m4s") && !req.url.contains("init") {
            if victims.contains(&req.url) {
                return Some(Answer::status(500));
            }
            if victims.len() < 2 && (victims.is_empty() || req.send_elapsed_us > 14 * SEC) {
                victims.push(req.url.clone());
                return Some(Answer::status(500));
            }
        }
        None
    }));
    sim.run_for(40 * SEC);
    let skipped = kinds(&sim)
        .into_iter()
        .filter(|(k, _)| matches!(k, ErrorKind::SegmentFetchFailed { .. }))
        .count();
    assert_eq!(
        skipped, 2,
        "samples in between reset the once-per-kind rule"
    );
}

#[test]
fn the_vod_playout_gate_starts_at_the_vod_lead_and_the_live_gate_at_the_segment_based_lead() {
    let start_at = |sim: &DashSim| {
        sim.events
            .iter()
            .position(|e| matches!(e, Event::Buffer(BufferEvent::Start { .. })))
    };
    let samples_before_start = |sim: &DashSim| {
        let at = start_at(sim).expect("a Start");
        sim.events[..at]
            .iter()
            .filter(|e| matches!(e, Event::Sample { .. }))
            .count()
    };
    let mut short = Config::default();
    short.playout.vod_start_lead_us = 500_000;
    let mut long = Config::default();
    long.playout.vod_start_lead_us = 2_500_000;
    let a = finished(DashSim::static_origin(&url_of(MANIFEST), short));
    let b = finished(DashSim::static_origin(&url_of(MANIFEST), long));
    assert!(samples_before_start(&a) < samples_before_start(&b));

    let live = |lead_us: u64| {
        let mut config = Config::default();
        config.playout.live_start_lead_us = lead_us;
        let synth = Synth::new(AST_US, Addressing::Number)
            .with_utc("http-xsdate", "http://utc.test/xsdate");
        let mut sim = DashSim::new(
            MPD_URL,
            config,
            World::new(AST_US + 41 * MICROS + 120_000),
            Box::new(WorldBackend(synth)),
        );
        sim.run_for(30 * SEC);
        sim
    };
    let quick = live(1 * SEC);
    let slow = live(1_000 * SEC);
    let cap = live(6 * SEC);
    assert!(samples_before_start(&quick) < samples_before_start(&slow));
    assert_eq!(
        samples_before_start(&slow),
        samples_before_start(&cap),
        "the lead is capped at three times maxSegmentDuration (6 s): a larger setting changes nothing"
    );
}

#[test]
fn a_closed_session_ignores_everything_and_a_dash_session_reports_ended_once() {
    let sim = finished(DashSim::static_origin(&url_of(MANIFEST), Config::default()));
    assert_eq!(
        sim.events
            .iter()
            .filter(|e| matches!(e, Event::Status(Status::Ended)))
            .count(),
        1
    );
    let mut sim = sim;
    sim.session.reconnect(sim.world.mono());
    let after = sim.log.len();
    sim.run_for(30 * SEC);
    assert_eq!(
        sim.log.len(),
        after,
        "an ended presentation ignores reconnect"
    );
    sim.session.close();
    assert!(sim.session.poll(0).is_none());
}
