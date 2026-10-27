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
use dash_matrix::{
    Cap, Class, F, Limits, Matcher, Src, Stage, distinct_urls, matrix, peak_rate_per_url,
    served_segments, terminal,
};
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
const MATRIX_SEGMENT_FAILURES: u32 = 3;
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
    config.max_segment_failures = MATRIX_SEGMENT_FAILURES;
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

fn check_stage(src: Src, stage: &str) {
    let (bad, cells) = matrix(&src, Some(stage));
    assert!(!cells.is_empty(), "{}: {stage}: no cell was run", src.name);
    assert!(
        bad.is_empty(),
        "{} cells off the expected states:\n{}",
        bad.len(),
        bad.join("\n")
    );
}

/// One test per (source, stage): nextest caps a test at 90 s and a CI runner is several times
/// slower than a workstation, so a whole source in one test does not fit.
macro_rules! stage_tests {
    ($($name:ident => ($source:ident, $source_name:literal, $stage:literal)),+ $(,)?) => {
        $(
            #[test]
            fn $name() {
                check_stage($source(), $stage);
            }
        )+

        const COVERED: &[(&str, &str)] = &[$(($source_name, $stage)),+];
    };
}

stage_tests! {
    static_mpd => (static_src, "static", "mpd"),
    static_init_v => (static_src, "static", "init-v"),
    static_init_a => (static_src, "static", "init-a"),
    static_seg_v2 => (static_src, "static", "seg-v2"),
    static_seg_a3 => (static_src, "static", "seg-a3"),
    template_mpd => (static_template, "static-template", "mpd"),
    template_init_p1_v => (static_template, "static-template", "init-p1-v"),
    template_init_p1_a => (static_template, "static-template", "init-p1-a"),
    template_init_p2_v => (static_template, "static-template", "init-p2-v"),
    template_seg_p1_v => (static_template, "static-template", "seg-p1-v"),
    template_seg_p2_a => (static_template, "static-template", "seg-p2-a"),
    list_mpd => (static_list, "static-list", "mpd"),
    list_init_v => (static_list, "static-list", "init-v"),
    list_init_a => (static_list, "static-list", "init-a"),
    list_seg_v => (static_list, "static-list", "seg-v"),
    list_seg_a => (static_list, "static-list", "seg-a"),
    sidx_mpd => (sidx_src, "ondemand-sidx", "mpd"),
    sidx_init_v => (sidx_src, "ondemand-sidx", "init-v"),
    sidx_index_v => (sidx_src, "ondemand-sidx", "index-v"),
    sidx_seg_v => (sidx_src, "ondemand-sidx", "seg-v"),
    sidx_init_a => (sidx_src, "ondemand-sidx", "init-a"),
    sidx_index_a => (sidx_src, "ondemand-sidx", "index-a"),
    sidx_seg_a => (sidx_src, "ondemand-sidx", "seg-a"),
    number_mpd => (live_number, "live-number", "mpd"),
    number_mpd_refresh => (live_number, "live-number", "mpd-refresh"),
    number_utc => (live_number, "live-number", "utc"),
    number_init_v => (live_number, "live-number", "init-v"),
    number_init_a => (live_number, "live-number", "init-a"),
    number_seg_v => (live_number, "live-number", "seg-v"),
    number_seg_a => (live_number, "live-number", "seg-a"),
    timeline_mpd => (live_timeline, "live-timeline", "mpd"),
    timeline_mpd_refresh => (live_timeline, "live-timeline", "mpd-refresh"),
    timeline_utc => (live_timeline, "live-timeline", "utc"),
    timeline_init_v => (live_timeline, "live-timeline", "init-v"),
    timeline_init_a => (live_timeline, "live-timeline", "init-a"),
    timeline_seg_v => (live_timeline, "live-timeline", "seg-v"),
    timeline_seg_a => (live_timeline, "live-timeline", "seg-a"),
    head_mpd => (live_head, "live-head-utc", "mpd"),
    head_mpd_refresh => (live_head, "live-head-utc", "mpd-refresh"),
    head_utc => (live_head, "live-head-utc", "utc"),
    head_init_v => (live_head, "live-head-utc", "init-v"),
    head_init_a => (live_head, "live-head-utc", "init-a"),
    head_seg_v => (live_head, "live-head-utc", "seg-v"),
    head_seg_a => (live_head, "live-head-utc", "seg-a"),
    periods_mpd => (live_multiperiod, "live-multiperiod", "mpd"),
    periods_mpd_refresh => (live_multiperiod, "live-multiperiod", "mpd-refresh"),
    periods_utc => (live_multiperiod, "live-multiperiod", "utc"),
    periods_init_v => (live_multiperiod, "live-multiperiod", "init-v"),
    periods_init_a => (live_multiperiod, "live-multiperiod", "init-a"),
    periods_seg_v => (live_multiperiod, "live-multiperiod", "seg-v"),
    periods_seg_a => (live_multiperiod, "live-multiperiod", "seg-a"),
    periods_init_v_next => (live_multiperiod, "live-multiperiod", "init-v-next"),
    periods_seg_v_next => (live_multiperiod, "live-multiperiod", "seg-v-next"),
    periods_mpd_boundary => (live_multiperiod, "live-multiperiod", "mpd-boundary"),
}

/// A stage without a test above would silently never run: every stage of every source must be
/// listed in `stage_tests!`, and nothing listed may name a stage that no longer exists.
#[test]
fn every_stage_of_every_source_has_a_test() {
    let mut offered: Vec<(String, String)> = Vec::new();
    for src in all_sources() {
        let (_, stages) = (src.build)();
        offered.extend(stages.into_iter().map(|s| (src.name.to_owned(), s.name)));
    }
    let mut problems = Vec::new();
    for (source, stage) in &offered {
        if !COVERED.contains(&(source.as_str(), stage.as_str())) {
            problems.push(format!("{source} / {stage} has no test"));
        }
    }
    for (source, stage) in COVERED {
        if !offered.iter().any(|(s, g)| s == source && g == stage) {
            problems.push(format!("{source} / {stage} is tested but not offered"));
        }
    }
    assert!(problems.is_empty(), "{}", problems.join("\n"));
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

const CLOCK_SKEWS_US: [i64; 4] = [5 * MICROS, -5 * MICROS, 60 * MICROS, -60 * MICROS];
const CLOCK_STEPS_US: [i64; 2] = [30 * MICROS, -30 * MICROS];
const CLOCK_RUN_BEFORE_STEP_US: u64 = 30 * SEC;
const CLOCK_RUN_AFTER_STEP_US: u64 = 150 * SEC;
const TOLERATED_SKEW_US: i64 = 5 * MICROS;
const HTTP_TOO_EARLY: u16 = 425;

fn clock_run(skew_us: i64, step_us: i64, with_utc: bool) -> DashSim {
    let mut synth = Synth::new(AST_US, Addressing::Number);
    if with_utc {
        synth = synth.with_utc("http-xsdate", UTC_URL);
    }
    synth.send_date = true;
    synth.early_status = HTTP_TOO_EARLY;
    let mut world = World::new(AST_US + 41 * MICROS + 120_000);
    world.host_skew_us = skew_us;
    let mut sim = DashSim::new(
        MPD_URL,
        matrix_config(),
        world,
        Box::new(WorldBackend(synth)),
    );
    sim.give_host_clock();
    sim.run_for(CLOCK_RUN_BEFORE_STEP_US);
    if step_us != 0 {
        sim.world.host_step_us = step_us;
        sim.give_host_clock();
    }
    sim.run_for(CLOCK_RUN_AFTER_STEP_US);
    sim
}

/// A host clock that is wrong: with UTCTiming the origin's clock wins and the host's error is
/// irrelevant; with the host clock as the only source, an error within the tolerated skew (and
/// any error ahead of true time, which the origin's 425 answers absorb) must still play, and a
/// larger error behind true time may lose the stream but never ends the session, loops on the
/// wire, or panics.
fn host_clock_faults(with_utc: bool) {
    let mut bad = Vec::new();
    let mut cases: Vec<(i64, i64)> = CLOCK_SKEWS_US.iter().map(|s| (*s, 0)).collect();
    cases.extend(CLOCK_STEPS_US.iter().map(|st| (0, *st)));
    for (skew, step) in cases {
        let sim = clock_run(skew, step, with_utc);
        let label = format!("utc={with_utc} skew={skew} step={step}");
        let applied = (skew != 0 || step != 0)
            && sim.world.host_skew_us == skew
            && sim.world.host_step_us == step
            && sim.host_clock_calls.len() > usize::from(step != 0);
        if !applied {
            bad.push(format!("{label}: the clock fault was not applied"));
        }
        if let Some(kind) = sim.fatal() {
            bad.push(format!("{label}: fatal {kind:?}"));
        }
        if terminal(&sim.events) {
            bad.push(format!("{label}: the session ended"));
        }
        let samples: usize = sim.samples().values().map(Vec::len).sum();
        let must_play = with_utc || skew >= -TOLERATED_SKEW_US;
        if must_play && samples == 0 {
            bad.push(format!("{label}: no samples"));
        }
        let served = served_segments(&sim.log, &[]);
        if let Some(why) = served.repeated {
            bad.push(format!("{label}: {why}"));
        }
        let (peak, url) = peak_rate_per_url(&sim.log);
        if peak > dash_matrix::RATE_CAP_PER_WINDOW {
            bad.push(format!("{label}: {url} {peak}/s"));
        }
        println!(
            "clock {label}: samples={samples} reqs={} gap={:?}",
            sim.log.len(),
            served.gap.is_some()
        );
    }
    assert!(bad.is_empty(), "{}", bad.join("\n"));
}

#[test]
fn host_clock_faults_with_utc_timing() {
    host_clock_faults(true);
}

#[test]
fn host_clock_faults_without_utc_timing() {
    host_clock_faults(false);
}

const ALT_HOST: &str = "http://alt.test";
const PRIMARY_SEGMENTS: &str = "http://dash.test/live/seg/";

struct AltHost<B>(B);

impl<B: dash_sim::Backend> dash_sim::Backend for AltHost<B> {
    fn serve(&mut self, at_utc_us: i64, req: &Logged) -> dash_sim::Answer {
        let mut mapped = req.clone();
        mapped.url = req.url.replacen(ALT_HOST, HOST, 1);
        self.0.serve(at_utc_us, &mapped)
    }
}

fn live_alt_base_url() -> Src {
    let config = matrix_config();
    let build_config = config.clone();
    let base = move || {
        let mut synth = Synth::new(AST_US, Addressing::Number).with_utc("http-xsdate", UTC_URL);
        synth.extra_mpd_children =
            format!("<BaseURL>{HOST}/live/</BaseURL><BaseURL>{ALT_HOST}/live/</BaseURL>");
        DashSim::new(
            MPD_URL,
            build_config.clone(),
            World::new(AST_US + 41 * MICROS + 120_000),
            Box::new(AltHost(WorldBackend(synth))),
        )
    };
    let stages = vec![stage(
        "primary-segments",
        Class::Media,
        Matcher::Prefix(PRIMARY_SEGMENTS.to_owned()),
    )];
    Src {
        name: "live-alt-base-url",
        live: true,
        caps: &[Cap::Live],
        config,
        limits,
        build: Box::new(move || (base(), stages.clone())),
        horizon_us: LIVE_HORIZON_US,
    }
}

/// The primary BaseURL refusing every segment (the design's "alternate-BaseURL 503"): the client
/// rotates to the alternate and keeps playing, instead of retrying the dead host.
#[test]
fn an_alternate_base_url_takes_over_when_the_primary_refuses() {
    let src = live_alt_base_url();
    let baseline = dash_matrix::run_cell(&src, None);
    let (_, stages) = (src.build)();
    for fault in [F::S503, F::ConnFail, F::S404] {
        let plan = dash_matrix::Plan {
            stage: stages[0].clone(),
            fault,
            permanent: true,
        };
        let out = dash_matrix::run_cell(&src, Some(plan));
        assert!(
            !out.faulted.is_empty(),
            "{fault:?}: the fault was not applied"
        );
        assert!(!out.panicked, "{fault:?}: panic");
        let alternate = out
            .log
            .iter()
            .filter(|l| l.url.starts_with(ALT_HOST))
            .count();
        assert!(alternate > 0, "{fault:?}: the alternate was never used");
        let played = dash_matrix::samples(&out.events).len();
        let wanted = dash_matrix::samples(&baseline.events).len() / 2;
        assert!(
            played >= wanted,
            "{fault:?}: {played} samples, wanted {wanted}"
        );
        assert!(out.log.len() < 600, "{fault:?}: {} requests", out.log.len());
    }
}
