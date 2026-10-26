//! The fault matrix (design 8.3, harness v1): the round-8 fault kinds x request stage x
//! transient/permanent, for a static presentation, a live `$Number$` presentation and a live
//! timeline presentation. Every cell must classify as the state the fault calls for; the table is
//! printed with `--nocapture`.

#[path = "support/dash_common.rs"]
mod dash_common;
#[path = "support/dash_matrix.rs"]
mod dash_matrix;
#[path = "support/dash_sim.rs"]
mod dash_sim;

use broadcast_client::Config;
use dash_common::loopback::Origin;
use dash_common::synth::{Addressing, MPD_URL, Synth};
use dash_common::world::{MICROS, World};
use dash_matrix::{Limits, Matcher, Src, Stage, distinct_urls, matrix};
use dash_sim::{DashSim, SEC, StaticBackend, WorldBackend};

const HOST: &str = "http://dash.test";
const AST_US: i64 = 1_700_000_000 * MICROS;
const UTC_URL: &str = "http://utc.test/xsdate";

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
    config.dash.utc_timeout_us = 2 * SEC;
    config
}

fn url_stage(name: &str, url: &str) -> Stage {
    Stage {
        name: name.to_owned(),
        matcher: Matcher::Url(url.to_owned()),
    }
}

fn static_src() -> Src {
    let config = matrix_config();
    Src {
        name: "static",
        live: false,
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
                Stage {
                    name: "mpd".into(),
                    matcher: Matcher::MpdAll,
                },
                url_stage("init-v", &format!("{HOST}/init-stream0.m4s")),
                url_stage("init-a", &format!("{HOST}/init-stream1.m4s")),
                url_stage("seg-v2", &format!("{HOST}/chunk-stream0-00002.m4s")),
                url_stage("seg-a3", &format!("{HOST}/chunk-stream1-00003.m4s")),
            ];
            (sim, stages)
        }),
        horizon_us: 900 * SEC,
    }
}

fn live_src(name: &'static str, addressing: Addressing, start_number: u64, ast: i64) -> Src {
    let config = matrix_config();
    let build_config = config.clone();
    let base = move || {
        let mut synth = Synth::new(ast, addressing).with_utc("http-xsdate", UTC_URL);
        synth.start_number = start_number;
        synth.send_date = true;
        let world = World::new(AST_US + 41 * MICROS + 120_000);
        DashSim::new(
            MPD_URL,
            build_config.clone(),
            world,
            Box::new(WorldBackend(synth)),
        )
    };
    let probe = {
        let mut sim = base();
        sim.run_for(40 * SEC);
        sim.log
    };
    let urls = distinct_urls(&probe);
    let pick = |needle: &str, nth: usize| -> String {
        urls.iter()
            .filter(|u| u.contains(needle))
            .nth(nth)
            .unwrap_or_else(|| panic!("{name}: no {needle} #{nth} in {urls:?}"))
            .clone()
    };
    let stages = vec![
        Stage {
            name: "mpd".into(),
            matcher: Matcher::MpdAll,
        },
        Stage {
            name: "mpd-refresh".into(),
            matcher: Matcher::MpdRefresh,
        },
        url_stage("utc", UTC_URL),
        url_stage("init-v", &pick("/V/init.mp4", 0)),
        url_stage("init-a", &pick("/A/init.mp4", 0)),
        url_stage("seg-v", &pick("/V/", 3)),
        url_stage("seg-a", &pick("/A/", 3)),
    ];
    Src {
        name,
        live: true,
        config,
        limits,
        build: Box::new(move || (base(), stages.clone())),
        horizon_us: 120 * SEC,
    }
}

fn check(src: Src) {
    let bad = matrix(&src);
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
fn live_number_presentation() {
    check(live_src("live-number", Addressing::Number, 1, AST_US));
}

#[test]
fn live_timeline_presentation() {
    check(live_src("live-timeline", Addressing::TimelineTime, 1, 0));
}
