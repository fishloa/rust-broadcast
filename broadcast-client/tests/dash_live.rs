//! Live (dynamic MPD) DASH through a `Session`: the DESIGN 3.6 `World` (true UTC, a host clock with
//! skew, drift and steps, an origin that decides availability from the true time) drives the
//! session through its callbacks; segment availability, the join and the clock ladder are asserted
//! on the wire and on the events.

#[path = "support/dash_checks.rs"]
mod dash_checks;
#[path = "support/dash_common.rs"]
mod dash_common;
#[path = "support/dash_sim.rs"]
mod dash_sim;

use std::cell::RefCell;
use std::collections::BTreeMap;
use std::rc::Rc;

use broadcast_client::{ClockSource, Config, DashClockPolicy, ErrorKind, Event, Status};
use dash_checks::*;
use dash_common::capture::{CaptureOrigin, Livesim2Pair};
use dash_common::loopback::{Sig, oracle, sigs};
use dash_common::synth::{Addressing, MPD_URL, Synth};
use dash_common::world::{MICROS, Shared, World};
use dash_runtime::mpd::Mpd;
use dash_runtime::timing::datetime::parse_xs_datetime;
use dash_sim::{DashSim, SEC, WorldBackend};

const AST_US: i64 = 1_700_000_000 * MICROS;
const UTC_XSDATE: &str = "http://utc.test/xsdate";
const TOLERANCE_US: i64 = 2_000;

fn ts(text: &str) -> i64 {
    parse_xs_datetime(text).unwrap().unix_us
}

fn live(
    synth: Synth,
    start_after_ast_us: i64,
    config: Config,
) -> (DashSim, dash_common::synth::Geo) {
    let geo = synth.geo();
    let world = World::new(AST_US + start_after_ast_us);
    let sim = DashSim::new(MPD_URL, config, world, Box::new(WorldBackend(synth)));
    (sim, geo)
}

fn with_utc(addressing: Addressing) -> Synth {
    Synth::new(AST_US, addressing).with_utc("http-xsdate", UTC_XSDATE)
}

fn clock_events(sim: &DashSim) -> Vec<(ClockSource, u64)> {
    sim.events
        .iter()
        .filter_map(|e| match e {
            Event::ClockSynced {
                source,
                uncertainty_us,
            } => Some((*source, *uncertainty_us)),
            _ => None,
        })
        .collect()
}

fn run_checked(sim: &mut DashSim, geo: dash_common::synth::Geo, secs: u64) -> Vec<SegReq> {
    sim.run_for(secs * SEC);
    assert!(sim.fatal().is_none(), "fatal: {:?}", sim.fatal());
    assert!(sim.errors().is_empty(), "{:?}", sim.errors());
    let reqs = segment_requests(sim, geo);
    assert!(!reqs.is_empty());
    assert_contiguous(&reqs);
    assert_no_duplicates(&reqs);
    reqs
}

fn sample_count(sim: &DashSim) -> usize {
    sim.count(|e| matches!(e, Event::Sample { .. }))
}

fn first_sample_position(sim: &DashSim) -> usize {
    sim.events
        .iter()
        .position(|e| matches!(e, Event::Sample { .. }))
        .expect("a sample")
}

#[test]
fn host_clock_number_template_joins_near_the_edge_and_plays_without_gaps() {
    let synth = Synth::new(AST_US, Addressing::Number);
    let (mut sim, geo) = live(synth, 40 * MICROS + 300_000, Config::default());
    sim.give_host_clock();
    let start_utc = sim.world.true_utc();
    let reqs = run_checked(&mut sim, geo, 40);
    let sources = clock_events(&sim);
    assert_eq!(sources.first().map(|s| s.0), Some(ClockSource::Host));
    assert_eq!(sources.first().map(|s| s.1), Some(500_000));
    let first = first_index(&reqs, "V").unwrap();
    let edge = geo.complete_count(start_utc) - 1;
    assert!((first - (edge - 3)).abs() <= 1, "join {first} edge {edge}");
    assert_eq!(sim.samples().len(), 2);
    assert!(sim.samples().values().all(|s| !s.is_empty()));
    assert!(
        sim.events
            .iter()
            .position(|e| matches!(e, Event::ClockSynced { .. }))
            < Some(first_sample_position(&sim)),
        "ClockSynced precedes the first sample that depends on it"
    );
}

#[test]
fn every_addressing_joins_at_the_delay_and_plays_on_utc_timing() {
    for a in [
        Addressing::Number,
        Addressing::TimelineNumber,
        Addressing::TimelineTime,
    ] {
        let ast = if a == Addressing::Number { AST_US } else { 0 };
        let mut synth = Synth::new(ast, a).with_utc("http-xsdate", UTC_XSDATE);
        synth.start_number = if a == Addressing::TimelineNumber {
            895_967_208
        } else {
            1
        };
        let geo = synth.geo();
        let world = World::new(AST_US + 41 * MICROS + 120_000);
        let mut sim = DashSim::new(
            MPD_URL,
            Config::default(),
            world,
            Box::new(WorldBackend(synth)),
        );
        let start_utc = sim.world.true_utc();
        let reqs = run_checked(&mut sim, geo, 30);
        assert!(early_requests(&reqs, geo, TOLERANCE_US).is_empty(), "{a:?}");
        let first = first_index(&reqs, "V").unwrap();
        let edge = geo.complete_count(start_utc) - 1;
        assert!(
            (first - (edge - 3)).abs() <= 1,
            "{a:?}: join {first} edge {edge}"
        );
        assert_eq!(
            clock_events(&sim).first().map(|s| s.0),
            Some(ClockSource::UtcTiming),
            "{a:?}"
        );
        assert!(!sim.requests_to("utc.test").is_empty());
    }
}

#[test]
fn host_skew_with_utc_timing_never_requests_early() {
    for skew in [
        -60 * MICROS,
        -5 * MICROS,
        -500_000,
        500_000,
        5 * MICROS,
        60 * MICROS,
    ] {
        let (mut sim, geo) = live(
            with_utc(Addressing::Number),
            41 * MICROS + 120_000,
            Config::default(),
        );
        sim.world.host_skew_us = skew;
        sim.give_host_clock();
        let reqs = run_checked(&mut sim, geo, 25);
        assert!(
            early_requests(&reqs, geo, TOLERANCE_US).is_empty(),
            "skew {skew}"
        );
        assert_eq!(
            clock_events(&sim).first().map(|s| s.0),
            Some(ClockSource::UtcTiming)
        );
    }
}

#[test]
fn host_skew_without_utc_timing_is_absorbed_as_early_requests() {
    for skew in [500_000i64, 5 * MICROS] {
        let mut synth = Synth::new(AST_US, Addressing::Number);
        synth.early_status = 425;
        let (mut sim, geo) = live(synth, 41 * MICROS + 120_000, Config::default());
        sim.world.host_skew_us = skew;
        sim.give_host_clock();
        sim.run_for(90 * SEC);
        assert!(sim.fatal().is_none());
        let reqs = segment_requests(&sim, geo);
        assert_contiguous(&reqs);
        assert_no_duplicates(&reqs);
        assert!(
            !early_requests(&reqs, geo, TOLERANCE_US).is_empty(),
            "skew {skew}: the host clock ahead requests early"
        );
        assert!(
            !sim.error_names().contains(&"segment_fetch_failed"),
            "early answers are never counted: {:?}",
            sim.error_names()
        );
        assert!(
            !sim.statuses()
                .iter()
                .any(|s| matches!(s, Status::Reconnecting { .. })),
            "an early answer is not a reconnect"
        );
        assert!(sample_count(&sim) > 0);
    }
}

#[test]
fn the_date_header_is_the_last_clock_rung() {
    let mut synth = Synth::new(AST_US, Addressing::Number);
    synth.send_date = true;
    let (mut sim, geo) = live(synth, 41 * MICROS + 120_000, Config::default());
    sim.run_for(25 * SEC);
    assert!(sim.fatal().is_none(), "{:?}", sim.fatal());
    let sources = clock_events(&sim);
    assert_eq!(sources.first().map(|s| s.0), Some(ClockSource::MpdDate));
    assert!(sources.first().is_some_and(|s| s.1 >= 500_000));
    assert_contiguous(&segment_requests(&sim, geo));
}

#[test]
fn no_clock_source_at_all_is_a_fatal_clock_unavailable() {
    let synth = Synth::new(AST_US, Addressing::Number);
    let (mut sim, _) = live(synth, 41 * MICROS, Config::default());
    sim.run_to_terminal(30 * SEC);
    assert_eq!(sim.fatal(), Some(ErrorKind::ClockUnavailable));
    assert!(clock_events(&sim).is_empty());
    assert_eq!(sample_count(&sim), 0);
}

#[test]
fn prefer_host_takes_the_host_clock_before_utc_timing() {
    let mut config = Config::default();
    config.dash.clock_policy = DashClockPolicy::PreferHost;
    let (mut sim, geo) = live(with_utc(Addressing::Number), 41 * MICROS + 120_000, config);
    sim.give_host_clock();
    run_checked(&mut sim, geo, 20);
    assert_eq!(
        clock_events(&sim).first().map(|s| s.0),
        Some(ClockSource::Host)
    );
    assert!(
        sim.requests_to("utc.test").is_empty(),
        "a trusted host clock saves the UTCTiming round trip"
    );
}

fn run_with_host_step(step_us: i64) -> (DashSim, dash_common::synth::Geo) {
    let (mut sim, geo) = live(
        with_utc(Addressing::Number),
        41 * MICROS + 120_000,
        Config::default(),
    );
    sim.give_host_clock();
    sim.run_for(10 * SEC);
    sim.world.host_step_us = step_us;
    sim.give_host_clock();
    sim.run_for(20 * SEC);
    (sim, geo)
}

fn shape(sim: &DashSim) -> Vec<&'static str> {
    sim.events
        .iter()
        .map(|e| match e {
            Event::Sample { .. } => "sample",
            Event::Discontinuity => "discontinuity",
            Event::Tracks(_) => "tracks",
            Event::ClockSynced { .. } => "clock",
            Event::Error { .. } => "error",
            _ => "other",
        })
        .collect()
}

#[test]
fn a_host_clock_step_does_not_disturb_a_utc_timing_anchored_session() {
    let (steady, _) = run_with_host_step(0);
    let (stepped, geo) = run_with_host_step(30 * MICROS);
    assert!(stepped.fatal().is_none());
    let reqs = segment_requests(&stepped, geo);
    assert!(early_requests(&reqs, geo, TOLERANCE_US).is_empty());
    assert_contiguous(&reqs);
    assert_no_duplicates(&reqs);
    assert_eq!(
        shape(&stepped),
        shape(&steady),
        "the event sequence is the one of a session whose host clock never stepped"
    );
    assert_eq!(
        clock_events(&stepped).len(),
        1,
        "one anchor, one ClockSynced"
    );
    assert_eq!(stepped.host_clock_calls.len(), 2);
}

#[test]
fn falling_behind_the_window_reports_live_edge_lost_and_a_discontinuity_then_recovers() {
    let (mut sim, geo) = live(
        with_utc(Addressing::Number),
        41 * MICROS + 120_000,
        Config::default(),
    );
    sim.run_for(12 * SEC);
    let before = sample_count(&sim);
    assert!(before > 0);
    sim.sleep(120 * SEC);
    sim.run_for(30 * SEC);
    assert!(sim.fatal().is_none(), "{:?}", sim.fatal());
    let lost = sim
        .errors()
        .iter()
        .filter(|(k, _)| *k == ErrorKind::LiveEdgeLost)
        .count();
    assert_eq!(lost, 1, "one LiveEdgeLost per run: {:?}", sim.errors());
    assert!(sim.count(|e| matches!(e, Event::Discontinuity)) >= 1);
    assert!(
        sample_count(&sim) > before,
        "playback resumes after the skip"
    );
    let reqs = segment_requests(&sim, geo);
    assert_no_duplicates(&reqs);
}

#[test]
fn the_status_sequence_is_establishing_then_live_once() {
    let (mut sim, _) = live(
        with_utc(Addressing::Number),
        41 * MICROS + 120_000,
        Config::default(),
    );
    sim.run_for(20 * SEC);
    let statuses: Vec<Status> = sim.statuses();
    assert_eq!(statuses, vec![Status::Establishing, Status::Live]);
}

#[test]
fn utc_head_is_sent_with_the_head_method_and_a_refused_head_is_retried_as_get() {
    let synth =
        Synth::new(AST_US, Addressing::Number).with_utc("http-head", "http://utc.test/head");
    let (mut sim, _) = live(synth, 41 * MICROS + 120_000, Config::default());
    sim.run_for(10 * SEC);
    let utc: Vec<_> = sim.requests_to("utc.test");
    assert!(!utc.is_empty());
    assert!(utc.iter().all(|r| r.method == "HEAD"), "{utc:?}");
    assert_eq!(
        clock_events(&sim).first().map(|s| s.0),
        Some(ClockSource::UtcTiming)
    );

    let mut synth =
        Synth::new(AST_US, Addressing::Number).with_utc("http-head", "http://utc.test/head");
    synth.utc.head_status = Some(405);
    let (mut sim, _) = live(synth, 41 * MICROS + 120_000, Config::default());
    sim.run_for(10 * SEC);
    let methods: Vec<&str> = sim
        .requests_to("utc.test")
        .iter()
        .map(|r| r.method.as_str())
        .collect();
    assert_eq!(methods.first(), Some(&"HEAD"));
    assert!(methods.contains(&"GET"), "{methods:?}");
    assert_eq!(
        clock_events(&sim).first().map(|s| s.0),
        Some(ClockSource::UtcTiming)
    );
}

#[test]
fn the_mpd_refresh_is_conditional_and_a_304_is_accepted() {
    let mut synth = with_utc(Addressing::Number);
    synth.send_etag = true;
    let fixed = {
        let mut probe = Synth::new(AST_US, Addressing::Number);
        probe.utc_elements = synth.utc_elements.clone();
        probe.mpd_at(AST_US + 41 * MICROS + 120_000)
    };
    synth.mpd_override = Some(Box::new(move |_| Some(fixed.clone())));
    let shared = Rc::new(RefCell::new(synth));
    let geo = shared.borrow().geo();
    let world = World::new(AST_US + 41 * MICROS + 120_000);
    let mut sim = DashSim::new(
        MPD_URL,
        Config::default(),
        world,
        Box::new(WorldBackend(Shared(shared.clone()))),
    );
    sim.run_for(30 * SEC);
    assert!(sim.fatal().is_none());
    let mpds: Vec<_> = sim.log.iter().filter(|l| l.is_mpd()).collect();
    assert!(mpds.len() >= 10);
    assert!(mpds[0].header("if-none-match").is_none());
    assert!(
        mpds.iter().skip(1).all(|r| r.header("if-none-match").is_some()),
        "every refresh carries the validator"
    );
    assert!(sim.errors().is_empty(), "{:?}", sim.errors());
    assert_contiguous(&segment_requests(&sim, geo));
    assert!(
        shared.borrow().if_none_match_hits >= 10,
        "the origin answered conditional requests with 304 and the session carried on"
    );
}

fn ffmpeg_number_origin(ast: i64) -> CaptureOrigin {
    let mut origin = CaptureOrigin::new("ffmpeg-number", "live-number");
    for n in 5..=8i64 {
        let at = ast + 2 * MICROS * n;
        origin = origin
            .available(&format!("seg-0-{n:03}.m4s"), at)
            .available(&format!("seg-1-{n:03}.m4s"), at);
    }
    origin
}

fn samples_by_track(sim: &DashSim) -> BTreeMap<u32, Vec<Sig>> {
    sim.samples()
        .into_iter()
        .map(|(k, v)| (k, sigs(&v)))
        .collect()
}

fn requested_files(sim: &DashSim) -> Vec<String> {
    sim.log
        .iter()
        .filter(|l| l.url.ends_with(".m4s") && !l.url.contains("init"))
        .map(|l| l.url.rsplit('/').next().unwrap().to_owned())
        .collect()
}

#[test]
fn the_ffmpeg_number_capture_plays_exactly_the_segments_the_clock_selects() {
    let ast = ts("2026-10-26T09:18:04.725Z");
    let origin = ffmpeg_number_origin(ast);
    let url = origin.url("live.mpd");
    let world = World::new(ast + 15_300_000);
    let mut sim = DashSim::new(
        &url,
        Config::default(),
        world,
        Box::new(WorldBackend(origin)),
    );
    sim.give_host_clock();
    sim.run_until(1_300_000);
    assert!(sim.fatal().is_none(), "{:?}", sim.fatal());
    let mut segments = requested_files(&sim);
    segments.sort();
    assert_eq!(
        segments,
        [
            "seg-0-007.m4s",
            "seg-0-008.m4s",
            "seg-1-007.m4s",
            "seg-1-008.m4s"
        ],
        "T_join = utc - SPD - AST = 13.34 s lies in segment 7; 8 completes at AST + 16 s"
    );
    let seg8 = sim
        .log
        .iter()
        .find(|l| l.url.ends_with("seg-0-008.m4s"))
        .unwrap();
    assert!(
        seg8.send_utc_us >= ast + 16 * MICROS,
        "segment 8 is not requested before it is complete"
    );
    let got = samples_by_track(&sim);
    let video = oracle(&[
        "live-number/init-0.m4s",
        "live-number/seg-0-007.m4s",
        "live-number/seg-0-008.m4s",
    ]);
    let audio = oracle(&[
        "live-number/init-1.m4s",
        "live-number/seg-1-007.m4s",
        "live-number/seg-1-008.m4s",
    ]);
    assert_eq!(
        got.get(&1),
        Some(&video[0]),
        "video samples equal the demux oracle"
    );
    assert_eq!(
        got.get(&2),
        Some(&audio[0]),
        "audio samples equal the demux oracle"
    );
}

#[test]
fn the_ffmpeg_timeline_capture_joins_on_s_t_not_on_tfdt() {
    let ast = ts("2026-10-26T09:18:20.766Z");
    let publish = ts("2026-10-26T09:18:36.438Z");
    let mut origin = CaptureOrigin::new("ffmpeg-timeline", "live-timeline");
    let micros = |ticks: i64, scale: i64| ticks * MICROS / scale;
    for k in 0..4i64 {
        let start = 365_853 + k * 273_763;
        let at = ast + micros(start + 273_763, 90_000);
        origin = origin.available(&format!("seg-0-{:03}.m4s", 3 + k), at);
    }
    let audio = [
        (353_280i64, 89_087i64),
        (442_367, 89_088),
        (531_455, 89_089),
        (620_544, 89_088),
    ];
    for (k, (t, d)) in audio.iter().enumerate() {
        let at = ast + micros(t + d, 44_100);
        origin = origin.available(&format!("seg-1-{:03}.m4s", 5 + k), at);
    }
    let url = origin.url("live.mpd");
    let mut sim = DashSim::new(
        &url,
        Config::default(),
        World::new(publish),
        Box::new(WorldBackend(origin)),
    );
    sim.give_host_clock();
    sim.run_until(1_700_000);
    assert!(sim.fatal().is_none(), "{:?}", sim.fatal());
    let mut segments = requested_files(&sim);
    segments.sort();
    assert_eq!(
        segments,
        [
            "seg-0-005.m4s",
            "seg-0-006.m4s",
            "seg-1-007.m4s",
            "seg-1-008.m4s"
        ],
        "video joins in 5 and audio in 7: boundaries of each Representation, not by number"
    );
    let got = samples_by_track(&sim);
    let video = oracle(&[
        "live-timeline/init-0.m4s",
        "live-timeline/seg-0-005.m4s",
        "live-timeline/seg-0-006.m4s",
    ]);
    let aud = oracle(&[
        "live-timeline/init-1.m4s",
        "live-timeline/seg-1-007.m4s",
        "live-timeline/seg-1-008.m4s",
    ]);
    assert_eq!(got.get(&1), Some(&video[0]));
    assert_eq!(got.get(&2), Some(&aud[0]));
}

fn livesim2_pair() -> (Livesim2Pair, i64, i64) {
    let a = test_fixtures::read_to_string("dash/livesim2/segtimeline_time_a.mpd");
    let b = test_fixtures::read_to_string("dash/livesim2/segtimeline_time_b.mpd");
    let mut avail = BTreeMap::new();
    for text in [&a, &b] {
        let mpd = Mpd::parse(text.as_bytes()).unwrap();
        for (set, name) in [(0usize, "A48"), (1usize, "V300")] {
            let planned = dash_runtime::address::plan_representation(
                &mpd,
                dash_runtime::selection::RepRef::at(0, set, 0),
                &dash_runtime::address::PlanOptions::default(),
            )
            .unwrap();
            let scale = planned.plan.timescale() as i64;
            for s in planned.plan.iter() {
                let end = s.time_ticks as i64 + s.duration_ticks as i64;
                avail.insert(
                    (name.to_owned(), s.time_ticks),
                    (i128::from(end) * i128::from(MICROS) / i128::from(scale)) as i64,
                );
            }
        }
    }
    let publish_a = Mpd::parse(a.as_bytes()).unwrap().publish_time_us.unwrap();
    let publish_b = Mpd::parse(b.as_bytes()).unwrap().publish_time_us.unwrap();
    (
        Livesim2Pair {
            a,
            b,
            switch_utc_us: publish_b,
            avail_utc_us: avail,
            pool: dash_common::synth::Pool::load(),
            segment_requests: Vec::new(),
        },
        publish_a,
        publish_b,
    )
}

#[test]
fn the_real_livesim2_refresh_pair_plays_across_the_update_without_gap_or_early_request() {
    let (origin, publish_a, publish_b) = livesim2_pair();
    let shared = Rc::new(RefCell::new(origin));
    let world = World::new(publish_a + 4 * MICROS);
    let mut sim = DashSim::new(
        Livesim2Pair::url(),
        Config::default(),
        world,
        Box::new(WorldBackend(Shared(shared.clone()))),
    );
    let span = (publish_b - publish_a) as u64 + 12 * SEC;
    sim.run_for(span);
    assert!(sim.fatal().is_none(), "{:?}", sim.fatal());
    let reqs = shared.borrow().segment_requests.clone();
    assert!(!reqs.is_empty());
    for (rep, t, at, status) in &reqs {
        assert_ne!(*status, 425, "{rep} {t} requested before it exists at {at}");
    }
    let mut by_rep: BTreeMap<String, Vec<u64>> = BTreeMap::new();
    for (rep, t, _, status) in &reqs {
        if *status == 200 {
            let v = by_rep.entry(rep.clone()).or_default();
            if !v.contains(t) {
                v.push(*t);
            }
        }
    }
    for (rep, mut times) in by_rep {
        times.sort_unstable();
        assert!(times.len() >= 2, "{rep}: {times:?}");
    }
    assert!(sample_count(&sim) > 0);
}
