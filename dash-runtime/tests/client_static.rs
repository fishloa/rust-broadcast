//! Every static fixture plays to completion through the loopback host, and the
//! samples equal what `Fmp4Demux` makes of the concatenated init and segments
//! (the oracle never sees the client).

mod common;

use common::loopback::{
    Drive, Order, Origin, Run, Sig, assert_matches_oracle, drive, oracle, play,
};
use dash_runtime::client::{DashConfig, Output};

fn seq(prefix: &str, init: &str, pattern: &str, n: std::ops::RangeInclusive<usize>) -> Vec<String> {
    let mut out = vec![format!("{prefix}{init}")];
    for i in n {
        out.push(format!(
            "{prefix}{}",
            pattern.replace("{n}", &format!("{i}"))
        ));
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

pub fn assert_clean(run: &Run) {
    assert!(run.errors.is_empty(), "{:?}", run.errors);
    assert!(run.fatal.is_none(), "{:?}", run.fatal);
    assert!(run.ended, "no EndOfStream");
    assert!(!run.stuck);
    assert!(matches!(run.outputs.last(), Some(Output::EndOfStream)));
}

fn ffmpeg(prefix: &str) -> Vec<Vec<String>> {
    vec![
        seq5(prefix, "init-stream0.m4s", "chunk-stream0-", 1..=3),
        seq5(prefix, "init-stream1.m4s", "chunk-stream1-", 1..=4),
    ]
}

#[test]
fn ffmpeg_timeline_manifest_equals_the_demux_oracle() {
    let run = play("manifest.mpd", DashConfig::default());
    assert_clean(&run);
    assert_matches_oracle(&run, &want(&ffmpeg("")));
}

#[test]
fn ffmpeg_quirk_timeline_equals_the_demux_oracle() {
    let run = play("timeline-ffmpeg-quirk/manifest.mpd", DashConfig::default());
    assert_clean(&run);
    assert_matches_oracle(&run, &want(&ffmpeg("timeline-ffmpeg-quirk/")));
}

#[test]
fn gpac_timeline_equals_the_demux_oracle() {
    let sets = vec![
        seq("timeline-gpac/", "seg-1-.mp4", "seg-1-{n}.m4s", 1..=6),
        seq("timeline-gpac/", "seg-2-.mp4", "seg-2-{n}.m4s", 1..=6),
    ];
    let run = play("timeline-gpac/timeline.mpd", DashConfig::default());
    assert_clean(&run);
    assert_matches_oracle(&run, &want(&sets));
}

#[test]
fn segment_list_equals_the_demux_oracle() {
    let sets = vec![
        seq3("segmentlist/", "init-0.m4s", "seg-0-", 1..=6),
        seq3("segmentlist/", "init-1.m4s", "seg-1-", 1..=6),
    ];
    let run = play("segmentlist/list.mpd", DashConfig::default());
    assert_clean(&run);
    assert_matches_oracle(&run, &want(&sets));
}

fn whole_files(dir: &str) -> Vec<Vec<Sig>> {
    let v = format!("{dir}/v_dashinit.mp4");
    let a = format!("{dir}/a_dashinit.mp4");
    let mut out = oracle(&[v.as_str()]);
    out.extend(oracle(&[a.as_str()]));
    out
}

#[test]
fn on_demand_sidx_equals_the_demux_oracle() {
    let run = play("ondemand-sidx/ondemand.mpd", DashConfig::default());
    assert_clean(&run);
    assert_matches_oracle(&run, &whole_files("ondemand-sidx"));
    let ranged = run.requests.iter().filter(|r| r.range.is_some()).count();
    assert!(
        ranged >= 2 * (1 + 1 + 6),
        "init + index + 6 subsegments per rep: {ranged}"
    );
}

#[test]
fn on_demand_gpac_pto_quirk_equals_the_demux_oracle() {
    let run = play(
        "ondemand-gpac-pto-quirk/ondemand.mpd",
        DashConfig::default(),
    );
    assert_clean(&run);
    assert_matches_oracle(&run, &whole_files("ondemand-gpac-pto-quirk"));
}

#[test]
fn completion_order_and_window_do_not_change_the_samples() {
    let expected = want(&ffmpeg("timeline-ffmpeg-quirk/"));
    for order in [Order::Fifo, Order::Lifo, Order::Alternate] {
        for window in [1usize, 2, 3, 8] {
            let mut cfg = DashConfig::default();
            cfg.max_inflight_segments = window;
            let run = drive(
                "http://dash.test/timeline-ffmpeg-quirk/manifest.mpd",
                cfg,
                &mut Origin::new(),
                &mut Drive {
                    order,
                    ..Drive::default()
                },
            );
            assert_clean(&run);
            assert_matches_oracle(&run, &expected);
            assert!(
                run.max_segments_outstanding <= window,
                "{order:?} window {window}: {} outstanding",
                run.max_segments_outstanding
            );
        }
    }
}

#[test]
fn window_zero_behaves_as_one() {
    let mut cfg = DashConfig::default();
    cfg.max_inflight_segments = 0;
    let run = drive(
        "http://dash.test/manifest.mpd",
        cfg,
        &mut Origin::new(),
        &mut Drive::default(),
    );
    assert_clean(&run);
    assert_eq!(run.max_segments_outstanding, 1);
}

#[test]
fn the_window_fills_when_the_cap_allows_it() {
    let mut cfg = DashConfig::default();
    cfg.max_inflight_segments = 4;
    let run = drive(
        "http://dash.test/timeline-gpac/timeline.mpd",
        cfg,
        &mut Origin::new(),
        &mut Drive::default(),
    );
    assert_clean(&run);
    assert_eq!(run.max_segments_outstanding, 4);
}

#[test]
fn track_ids_are_one_based_video_then_audio_and_announced_once() {
    let run = play("manifest.mpd", DashConfig::default());
    let tracks: Vec<_> = run
        .outputs
        .iter()
        .filter_map(|o| match o {
            Output::Tracks(t) => Some(t),
            _ => None,
        })
        .collect();
    assert_eq!(tracks.len(), 1);
    let ids: Vec<u32> = tracks[0].iter().map(|t| t.track_id.value()).collect();
    assert_eq!(ids, vec![1, 2]);
    assert_eq!(tracks[0][0].timescale, 90_000);
    assert_eq!(tracks[0][1].timescale, 44_100);
}

#[test]
fn tracks_and_init_precede_every_sample() {
    let run = play("manifest.mpd", DashConfig::default());
    let first_sample = run
        .outputs
        .iter()
        .position(|o| matches!(o, Output::Samples { .. }))
        .expect("samples");
    let tracks = run
        .outputs
        .iter()
        .position(|o| matches!(o, Output::Tracks(_)))
        .expect("tracks");
    let init = run
        .outputs
        .iter()
        .position(|o| matches!(o, Output::Init(_)))
        .expect("init");
    assert!(init < tracks && tracks < first_sample);
    assert!(matches!(run.outputs[0], Output::Manifest(_)));
    assert!(matches!(
        run.outputs[1],
        Output::PeriodChanged {
            continuous: false,
            index: 0,
            ..
        }
    ));
    assert_eq!(run.count(|o| matches!(o, Output::Discontinuity)), 0);
}

#[test]
fn samples_of_a_track_never_go_back_in_decode_time() {
    let run = play("timeline-gpac/timeline.mpd", DashConfig::default());
    for (id, samples) in run.samples() {
        let mut last: Option<i64> = None;
        for (n, s) in samples.iter().enumerate() {
            if let (Some(l), Some(d)) = (last, s.dts) {
                assert!(d >= l, "track {id} sample {n}: dts {d} after {l}");
            }
            last = s.dts.or(last);
        }
    }
}
