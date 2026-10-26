//! The fault matrix (design 8.3): every fault kind x request class x transient/permanent, over
//! static (timeline, template, on-demand sidx) and live (`$Number$`, timeline, HEAD UTC,
//! multi-Period) presentations. Each cell asserts that the fault actually changed an answer, and
//! must classify as the state the fault calls for; the table is printed with `--nocapture`.
//!
//! The fault kinds and request classes are exhaustive enums: a fault does not compile without
//! stating its request classes, and `every_fault_and_class_has_a_row` fails when a fault/class
//! pair has no source and stage that can exercise it.

#[path = "support/dash_common.rs"]
mod dash_common;
#[path = "support/dash_matrix.rs"]
mod dash_matrix;
#[path = "support/dash_sim.rs"]
mod dash_sim;

use broadcast_client::Config;
use dash_common::loopback::Origin;
use dash_common::synth::{Addressing, MPD_URL, Synth, parse_segment_url};
use dash_common::world::{MICROS, World};
use dash_matrix::{Cap, Class, F, Limits, Matcher, Src, Stage, distinct_urls, matrix};
use dash_sim::{DashSim, Logged, SEC, StaticBackend, WorldBackend};

const HOST: &str = "http://dash.test";
const AST_US: i64 = 1_700_000_000 * MICROS;
const UTC_URL: &str = "http://utc.test/xsdate";
const UTC_HEAD_URL: &str = "http://utc.test/head";
const SIDX_MPD: &str = "http://dash.test/ondemand-sidx/ondemand.mpd";
const SIDX_VIDEO: &str = "http://dash.test/ondemand-sidx/v_dashinit.mp4";
const SIDX_AUDIO: &str = "http://dash.test/ondemand-sidx/a_dashinit.mp4";
const RESTART_MPD: &str = "http://dash.test/multiperiod-restart/multiperiod.mpd";
const LIST_MPD: &str = "http://dash.test/segmentlist/list.mpd";
const LIVE_HORIZON_US: u64 = 120 * SEC;
const PROBE_US: u64 = 60 * SEC;
const PERIOD_STARTS_US: [i64; 3] = [0, 40 * MICROS, 80 * MICROS];
const PERIOD_ANNOUNCE_AHEAD_US: i64 = 10 * MICROS;
const PERIOD_TSB_US: i64 = 20 * MICROS;
const LATE_REFRESH_INDEX: usize = 16;

fn limits() -> Limits {
    Limits {
        manifest: 256 * 1024,
        segment: 1024 * 1024,
    }
}

fn matrix_config() -> Config {
    let mut config = Config::default();
    config.max_manifest_bytes = 256 * 1024;
    config.max_segment_bytes = 1024 * 1024;
    config.request_timeout_us = 5 * SEC;
    config.segment_timeout_us = 10 * SEC;
    config.dash.utc_timeout_us = 4 * SEC;
    config
}

fn stage(name: &str, class: Class, matcher: Matcher) -> Stage {
    Stage {
        name: name.to_owned(),
        class,
        matcher,
    }
}

fn url_stage(name: &str, class: Class, url: &str) -> Stage {
    stage(name, class, Matcher::Url(url.to_owned()))
}

fn init_url(urls: &[String], rep_prefix: &str, nth: usize) -> String {
    urls.iter()
        .filter(|u| u.ends_with("/init.mp4"))
        .filter(|u| {
            u.rsplit('/')
                .nth(1)
                .is_some_and(|rep| rep.starts_with(rep_prefix))
        })
        .nth(nth)
        .unwrap_or_else(|| panic!("no init #{nth} for {rep_prefix} in {urls:?}"))
        .clone()
}

fn segment_url(urls: &[String], rep_prefix: &str, nth: usize) -> String {
    urls.iter()
        .filter(|u| parse_segment_url(u).is_some_and(|(rep, _)| rep.starts_with(rep_prefix)))
        .nth(nth)
        .unwrap_or_else(|| panic!("no segment #{nth} for {rep_prefix} in {urls:?}"))
        .clone()
}

fn static_src() -> Src {
    let config = matrix_config();
    Src {
        name: "static",
        live: false,
        caps: &[],
        config: config.clone(),
        limits,
        build: Box::new(move || {
            let sim = DashSim::new(
                &format!("{HOST}/manifest.mpd"),
                config.clone(),
                World::new(AST_US),
                Box::new(StaticBackend(Origin::new())),
            );
            let stages = vec![
                stage("mpd", Class::Mpd, Matcher::MpdAll),
                url_stage("init-v", Class::Init, &format!("{HOST}/init-stream0.m4s")),
                url_stage("init-a", Class::Init, &format!("{HOST}/init-stream1.m4s")),
                url_stage(
                    "seg-v2",
                    Class::Media,
                    &format!("{HOST}/chunk-stream0-00002.m4s"),
                ),
                url_stage(
                    "seg-a3",
                    Class::Media,
                    &format!("{HOST}/chunk-stream1-00003.m4s"),
                ),
            ];
            (sim, stages)
        }),
        horizon_us: 900 * SEC,
    }
}

fn static_fixture_src(name: &'static str, mpd_url: &'static str, pick: StaticPick) -> Src {
    let config = matrix_config();
    let build_config = config.clone();
    let base = move || {
        DashSim::new(
            mpd_url,
            build_config.clone(),
            World::new(AST_US),
            Box::new(StaticBackend(Origin::new())),
        )
    };
    let urls = {
        let mut sim = base();
        sim.run_to_terminal(900 * SEC);
        distinct_urls(&sim.log)
    };
    let stages = pick(&urls);
    Src {
        name,
        live: false,
        caps: &[],
        config,
        limits,
        build: Box::new(move || (base(), stages.clone())),
        horizon_us: 900 * SEC,
    }
}

type StaticPick = fn(&[String]) -> Vec<Stage>;

fn contains_nth(urls: &[String], needle: &str, nth: usize) -> String {
    urls.iter()
        .filter(|u| u.contains(needle))
        .nth(nth)
        .unwrap_or_else(|| panic!("no #{nth} {needle} in {urls:?}"))
        .clone()
}

fn static_template() -> Src {
    static_fixture_src("static-template", RESTART_MPD, |urls| {
        vec![
            stage("mpd", Class::Mpd, Matcher::MpdAll),
            url_stage(
                "init-p1-v",
                Class::Init,
                &contains_nth(urls, "p1_dash_track1_init", 0),
            ),
            url_stage(
                "init-p1-a",
                Class::Init,
                &contains_nth(urls, "p1_dash_track2_init", 0),
            ),
            url_stage(
                "init-p2-v",
                Class::Init,
                &contains_nth(urls, "p2_dash_track1_init", 0),
            ),
            url_stage(
                "seg-p1-v",
                Class::Media,
                &contains_nth(urls, "p1_dash_track1_2.m4s", 0),
            ),
            url_stage(
                "seg-p2-a",
                Class::Media,
                &contains_nth(urls, "p2_dash_track2_2.m4s", 0),
            ),
        ]
    })
}

fn static_list() -> Src {
    static_fixture_src("static-list", LIST_MPD, |urls| {
        vec![
            stage("mpd", Class::Mpd, Matcher::MpdAll),
            url_stage("init-v", Class::Init, &contains_nth(urls, "init-0", 0)),
            url_stage("init-a", Class::Init, &contains_nth(urls, "init-1", 0)),
            url_stage("seg-v", Class::Media, &contains_nth(urls, "seg-0-002", 0)),
            url_stage("seg-a", Class::Media, &contains_nth(urls, "seg-1-003", 0)),
        ]
    })
}

fn sidx_src() -> Src {
    let config = matrix_config();
    let build_config = config.clone();
    let base = move || {
        DashSim::new(
            SIDX_MPD,
            build_config.clone(),
            World::new(AST_US),
            Box::new(StaticBackend(Origin::new())),
        )
    };
    let log = {
        let mut sim = base();
        sim.run_to_terminal(900 * SEC);
        sim.log
    };
    let ranges_of = |url: &str| -> Vec<u64> {
        let mut starts: Vec<u64> = Vec::new();
        for l in log.iter().filter(|l| l.url == url) {
            if let Some((from, _)) = l.byte_range
                && !starts.contains(&from)
            {
                starts.push(from);
            }
        }
        starts
    };
    let ranged = |name: &str, class: Class, url: &str, from: u64| {
        stage(
            name,
            class,
            Matcher::Ranged {
                url: url.to_owned(),
                from,
            },
        )
    };
    let (video, audio) = (ranges_of(SIDX_VIDEO), ranges_of(SIDX_AUDIO));
    assert!(
        video.len() >= 3 && audio.len() >= 3,
        "init, index and media ranges expected: {video:?} {audio:?}"
    );
    let stages = vec![
        stage("mpd", Class::Mpd, Matcher::MpdAll),
        ranged("init-v", Class::Init, SIDX_VIDEO, video[0]),
        ranged("index-v", Class::Index, SIDX_VIDEO, video[1]),
        ranged("seg-v", Class::Media, SIDX_VIDEO, video[2]),
        ranged("init-a", Class::Init, SIDX_AUDIO, audio[0]),
        ranged("index-a", Class::Index, SIDX_AUDIO, audio[1]),
        ranged("seg-a", Class::Media, SIDX_AUDIO, audio[2]),
    ];
    Src {
        name: "ondemand-sidx",
        live: false,
        caps: &[Cap::Sidx],
        config,
        limits,
        build: Box::new(move || (base(), stages.clone())),
        horizon_us: 900 * SEC,
    }
}

struct Live {
    name: &'static str,
    addressing: Addressing,
    start_number: u64,
    ast: i64,
    utc: (&'static str, &'static str),
    periods: &'static [i64],
}

fn live_src(spec: Live) -> Src {
    let Live {
        name,
        addressing,
        start_number,
        ast,
        utc,
        periods,
    } = spec;
    let multi = periods.len() > 1;
    let config = matrix_config();
    let build_config = config.clone();
    let base = move || {
        let mut synth = Synth::new(ast, addressing).with_utc(utc.0, utc.1);
        synth.start_number = start_number;
        synth.send_date = true;
        if multi {
            synth.period_starts_us = periods.to_vec();
            synth.announce_ahead_us = PERIOD_ANNOUNCE_AHEAD_US;
            synth.tsb_us = Some(PERIOD_TSB_US);
        }
        let world = World::new(AST_US + 41 * MICROS + 120_000);
        DashSim::new(
            MPD_URL,
            build_config.clone(),
            world,
            Box::new(WorldBackend(synth)),
        )
    };
    let probe: Vec<Logged> = {
        let mut sim = base();
        sim.run_for(PROBE_US);
        sim.log
    };
    let urls = distinct_urls(&probe);
    let (video, audio) = if multi { ("V1", "A1") } else { ("V", "A") };
    let mut stages = vec![
        stage("mpd", Class::Mpd, Matcher::MpdAll),
        stage("mpd-refresh", Class::MpdRefresh, Matcher::MpdRefresh),
        url_stage("utc", Class::Utc, utc.1),
        url_stage("init-v", Class::Init, &init_url(&urls, video, 0)),
        url_stage("init-a", Class::Init, &init_url(&urls, audio, 0)),
        url_stage("seg-v", Class::Media, &segment_url(&urls, video, 3)),
        url_stage("seg-a", Class::Media, &segment_url(&urls, audio, 3)),
    ];
    let mut caps: Vec<Cap> = vec![Cap::Live];
    if utc.0 == "http-head" {
        caps.push(Cap::HeadUtc);
    }
    if multi {
        caps.push(Cap::MultiPeriod);
        stages.push(url_stage(
            "init-v-next",
            Class::Init,
            &init_url(&urls, "V2", 0),
        ));
        stages.push(url_stage(
            "seg-v-next",
            Class::Media,
            &segment_url(&urls, "V2", 1),
        ));
        stages.push(stage(
            "mpd-boundary",
            Class::MpdBoundary,
            Matcher::MpdFrom(LATE_REFRESH_INDEX),
        ));
    }
    Src {
        name,
        live: true,
        caps: Box::leak(caps.into_boxed_slice()),
        config,
        limits,
        build: Box::new(move || (base(), stages.clone())),
        horizon_us: LIVE_HORIZON_US,
    }
}

fn live_number() -> Src {
    live_src(Live {
        name: "live-number",
        addressing: Addressing::Number,
        start_number: 1,
        ast: AST_US,
        utc: ("http-xsdate", UTC_URL),
        periods: &[0],
    })
}

fn live_timeline() -> Src {
    live_src(Live {
        name: "live-timeline",
        addressing: Addressing::TimelineTime,
        start_number: 1,
        ast: 0,
        utc: ("http-xsdate", UTC_URL),
        periods: &[0],
    })
}

fn live_head() -> Src {
    live_src(Live {
        name: "live-head-utc",
        addressing: Addressing::Number,
        start_number: 1,
        ast: AST_US,
        utc: ("http-head", UTC_HEAD_URL),
        periods: &[0],
    })
}

fn live_multiperiod() -> Src {
    live_src(Live {
        name: "live-multiperiod",
        addressing: Addressing::Number,
        start_number: 1,
        ast: AST_US,
        utc: ("http-xsdate", UTC_URL),
        periods: &PERIOD_STARTS_US,
    })
}

fn all_sources() -> Vec<Src> {
    vec![
        static_src(),
        static_template(),
        static_list(),
        sidx_src(),
        live_number(),
        live_timeline(),
        live_head(),
        live_multiperiod(),
    ]
}

fn check(src: Src) {
    let (bad, cells) = matrix(&src);
    assert!(!cells.is_empty(), "{}: no cell was run", src.name);
    assert!(
        bad.is_empty(),
        "{} cells off the expected states:\n{}",
        bad.len(),
        bad.join("\n")
    );
}

#[test]
fn static_presentation() {
    check(static_src());
}

#[test]
fn static_template_presentation() {
    check(static_template());
}

#[test]
fn static_segment_list_presentation() {
    check(static_list());
}

#[test]
fn on_demand_sidx_presentation() {
    check(sidx_src());
}

#[test]
fn live_number_presentation() {
    check(live_number());
}

#[test]
fn live_timeline_presentation() {
    check(live_timeline());
}

#[test]
fn live_head_utc_presentation() {
    check(live_head());
}

#[test]
fn live_multiperiod_presentation() {
    check(live_multiperiod());
}

#[test]
fn every_fault_and_class_has_a_row() {
    let sources = all_sources();
    let mut stage_classes: Vec<(&str, &[Cap], Class)> = Vec::new();
    for src in &sources {
        let (_, stages) = (src.build)();
        for s in stages {
            stage_classes.push((src.name, src.caps, s.class));
        }
    }
    let mut missing = Vec::new();
    for &class in Class::ALL {
        if !stage_classes.iter().any(|(_, _, c)| *c == class) {
            missing.push(format!(
                "request class {} has no stage in any source",
                class.name()
            ));
        }
    }
    for &fault in F::ALL {
        assert!(
            !fault.classes().is_empty(),
            "{fault:?} names no request class"
        );
        for &class in fault.classes() {
            let reachable = stage_classes
                .iter()
                .any(|(_, caps, c)| *c == class && fault.applies_to(*c, caps));
            if !reachable {
                missing.push(format!(
                    "{fault:?} x {} has no source/stage that can exercise it",
                    class.name()
                ));
            }
        }
    }
    assert!(
        missing.is_empty(),
        "matrix rows missing:\n{}",
        missing.join("\n")
    );
}

#[test]
fn the_fault_list_has_no_duplicates() {
    let mut shorts: Vec<&str> = F::ALL.iter().map(|f| f.short()).collect();
    shorts.sort_unstable();
    let before = shorts.len();
    shorts.dedup();
    assert_eq!(before, shorts.len(), "two faults share a table label");
}
