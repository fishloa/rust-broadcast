//! Static (VOD) DASH presentations through a `Session`: the real committed captures play to
//! completion, and the delivered samples equal what `Fmp4Demux` makes of the concatenated init and
//! segments (the oracle never sees the session).

#[path = "support/dash_common.rs"]
mod dash_common;
#[path = "support/dash_sim.rs"]
mod dash_sim;

use broadcast_client::{Config, Event, Status};
use dash_common::loopback::{Sig, oracle};
use dash_sim::{DashSim, SEC, assert_samples_equal_oracle};

const HOST: &str = "http://dash.test";

fn seq(prefix: &str, init: &str, pattern: &str, n: std::ops::RangeInclusive<usize>) -> Vec<String> {
    let mut out = vec![format!("{prefix}{init}")];
    for i in n {
        out.push(format!("{prefix}{}", pattern.replace("{n}", &format!("{i}"))));
    }
    out
}

fn seq5(prefix: &str, init: &str, stem: &str, n: std::ops::RangeInclusive<usize>) -> Vec<String> {
    let mut out = vec![format!("{prefix}{init}")];
    for i in n {
        out.push(format!("{prefix}{stem}{i:05}.m4s"));
    }
    out
}

fn seq3(prefix: &str, init: &str, stem: &str, n: std::ops::RangeInclusive<usize>) -> Vec<String> {
    let mut out = vec![format!("{prefix}{init}")];
    for i in n {
        out.push(format!("{prefix}{stem}{i:03}.m4s"));
    }
    out
}

fn want(sets: &[Vec<String>]) -> Vec<Vec<Sig>> {
    sets.iter()
        .flat_map(|files| {
            let refs: Vec<&str> = files.iter().map(String::as_str).collect();
            oracle(&refs)
        })
        .collect()
}

fn ffmpeg(prefix: &str) -> Vec<Vec<String>> {
    vec![
        seq5(prefix, "init-stream0.m4s", "chunk-stream0-", 1..=3),
        seq5(prefix, "init-stream1.m4s", "chunk-stream1-", 1..=4),
    ]
}

fn whole_files(dir: &str) -> Vec<Vec<Sig>> {
    let v = format!("{dir}/v_dashinit.mp4");
    let a = format!("{dir}/a_dashinit.mp4");
    let mut out = oracle(&[v.as_str()]);
    out.extend(oracle(&[a.as_str()]));
    out
}

fn play(mpd: &str) -> DashSim {
    let mut sim = DashSim::static_origin(&format!("{HOST}/{mpd}"), Config::default());
    sim.run_to_terminal(600 * SEC);
    sim
}

fn assert_clean(sim: &DashSim) {
    assert!(sim.errors().is_empty(), "{:?}", sim.errors());
    assert!(sim.ended(), "no Ended; statuses {:?}", sim.statuses());
    assert!(matches!(
        sim.events.last(),
        Some(Event::Status(Status::Ended))
    ));
}

#[test]
fn ffmpeg_timeline_manifest_equals_the_demux_oracle() {
    let sim = play("manifest.mpd");
    assert_clean(&sim);
    assert_samples_equal_oracle(&sim, &want(&ffmpeg("")));
}

#[test]
fn ffmpeg_quirk_timeline_equals_the_demux_oracle() {
    let sim = play("timeline-ffmpeg-quirk/manifest.mpd");
    assert_clean(&sim);
    assert_samples_equal_oracle(&sim, &want(&ffmpeg("timeline-ffmpeg-quirk/")));
}

#[test]
fn gpac_timeline_equals_the_demux_oracle() {
    let sets = vec![
        seq("timeline-gpac/", "seg-1-.mp4", "seg-1-{n}.m4s", 1..=6),
        seq("timeline-gpac/", "seg-2-.mp4", "seg-2-{n}.m4s", 1..=6),
    ];
    let sim = play("timeline-gpac/timeline.mpd");
    assert_clean(&sim);
    assert_samples_equal_oracle(&sim, &want(&sets));
}

#[test]
fn segment_list_equals_the_demux_oracle() {
    let sets = vec![
        seq3("segmentlist/", "init-0.m4s", "seg-0-", 1..=6),
        seq3("segmentlist/", "init-1.m4s", "seg-1-", 1..=6),
    ];
    let sim = play("segmentlist/list.mpd");
    assert_clean(&sim);
    assert_samples_equal_oracle(&sim, &want(&sets));
}

#[test]
fn on_demand_sidx_equals_the_demux_oracle_and_uses_byte_ranges() {
    let sim = play("ondemand-sidx/ondemand.mpd");
    assert_clean(&sim);
    assert_samples_equal_oracle(&sim, &whole_files("ondemand-sidx"));
    let ranged = sim.log.iter().filter(|l| l.byte_range.is_some()).count();
    assert!(
        ranged >= 2 * (1 + 1 + 6),
        "init + index + 6 subsegments per rep: {ranged}"
    );
    assert!(sim.log.iter().all(|l| l.method == "GET"));
}
