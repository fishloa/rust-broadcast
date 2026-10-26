//! Multi-Period behaviour of `DashClient` on the real GPAC multi-period
//! fixtures: the restart (timeline reset) boundary, the continuous boundary
//! (23009-1 5.3.2.4 / A.9), and the 13 ms tail GPAC declares past its media.

mod common;

use common::loopback::{
    Drive, Fault, FaultKind, Order, Origin, Run, Sig, assert_matches_oracle, drive, mpd_text,
    oracle,
};
use dash_runtime::client::{DashConfig, FailureClass, FatalKind, Output};

const RESTART: &str = "http://dash.test/multiperiod-restart/multiperiod.mpd";
const CONTINUOUS: &str = "http://dash.test/multiperiod-continuous/multiperiod.mpd";

fn period_files(period: u32, track: u32, segments: usize) -> Vec<String> {
    let mut v = vec![format!(
        "multiperiod-restart/p{period}_dash_track{track}_init.mp4"
    )];
    for n in 1..=segments {
        v.push(format!(
            "multiperiod-restart/p{period}_dash_track{track}_{n}.m4s"
        ));
    }
    v
}

fn oracle_of(files: &[String]) -> Vec<Sig> {
    let refs: Vec<&str> = files.iter().map(String::as_str).collect();
    oracle(&refs).remove(0)
}

fn restart_expected() -> Vec<Vec<Sig>> {
    let mut video = oracle_of(&period_files(1, 1, 3));
    video.extend(oracle_of(&period_files(2, 1, 3)));
    let mut audio = oracle_of(&period_files(1, 2, 3));
    audio.extend(oracle_of(&period_files(2, 2, 3)));
    vec![video, audio]
}

fn run_restart(origin: &mut Origin, drive_opts: &Drive, cfg: DashConfig) -> Run {
    drive(RESTART, cfg, origin, drive_opts)
}

#[test]
fn restart_periods_play_the_files_that_exist_in_order() {
    let run = run_restart(&mut Origin::new(), &Drive::default(), DashConfig::default());
    assert!(run.errors.is_empty(), "{:?}", run.errors);
    assert!(run.ended && run.fatal.is_none());
    assert_matches_oracle(&run, &restart_expected());
}

#[test]
fn restart_boundary_is_one_discontinuity_before_the_new_periods_samples() {
    let run = run_restart(&mut Origin::new(), &Drive::default(), DashConfig::default());
    let changes: Vec<_> = run
        .outputs
        .iter()
        .filter_map(|o| match o {
            Output::PeriodChanged {
                index,
                id,
                start_us,
                continuous,
            } => Some((*index, id.clone(), *start_us, *continuous)),
            _ => None,
        })
        .collect();
    assert_eq!(
        changes,
        vec![
            (0, Some("1".to_owned()), 0, false),
            (1, Some("2".to_owned()), 6_000_000, false)
        ]
    );
    let manifests = run.count(|o| matches!(o, Output::Manifest(_)));
    assert_eq!(manifests, 2);
    assert_eq!(run.count(|o| matches!(o, Output::Discontinuity)), 1);
    let disc = run
        .outputs
        .iter()
        .position(|o| matches!(o, Output::Discontinuity))
        .unwrap();
    let second = run
        .outputs
        .iter()
        .rposition(|o| matches!(o, Output::PeriodChanged { index: 1, .. }))
        .unwrap();
    assert!(second < disc, "PeriodChanged then Discontinuity");
    let before: usize = run.outputs[..disc]
        .iter()
        .filter_map(|o| match o {
            Output::Samples { samples, .. } => Some(samples.len()),
            _ => None,
        })
        .sum();
    let total: usize = run.samples().values().map(Vec::len).sum();
    assert!(before > 0 && before < total, "samples on both sides");
    let inits = run.count(|o| matches!(o, Output::Init(_)));
    assert_eq!(inits, 4, "two inits per Period, each Period has its own");
}

#[test]
fn restart_period_two_inits_are_requested_while_period_one_is_in_flight() {
    use dash_runtime::client::{Action, DashClient};
    let mut cfg = DashConfig::default();
    cfg.max_inflight_segments = 8;
    let mut c = DashClient::new(RESTART, cfg);
    assert!(matches!(c.poll(0), Some(Action::FetchMpd { .. })));
    let mpd = test_fixtures::read("dash/multiperiod-restart/multiperiod.mpd");
    c.on_mpd(0, &mpd).unwrap();
    let mut urls = Vec::new();
    while let Some(a) = c.poll(0) {
        urls.push(a.url().to_owned());
    }
    assert!(
        urls.iter().any(|u| u.contains("p2_dash_track1_init")),
        "{urls:?}"
    );
    assert!(urls.iter().any(|u| u.contains("p1_dash_track1_1.m4s")));
}

#[test]
fn restart_with_the_window_of_one_and_reverse_completion_is_unchanged() {
    for order in [Order::Lifo, Order::Alternate] {
        let mut cfg = DashConfig::default();
        cfg.max_inflight_segments = 1;
        let run = run_restart(
            &mut Origin::new(),
            &Drive {
                order,
                ..Drive::default()
            },
            cfg,
        );
        assert!(run.ended && run.errors.is_empty());
        assert_matches_oracle(&run, &restart_expected());
    }
}

#[test]
fn the_gpac_tail_past_the_media_is_requested_once_and_ends_the_period() {
    let run = run_restart(&mut Origin::new(), &Drive::default(), DashConfig::default());
    for tail in ["p1_dash_track1_4.m4s", "p1_dash_track2_4.m4s"] {
        assert_eq!(run.requested(tail), 1, "{tail}: 404 once, never retried");
    }
    let tails = run
        .classes
        .iter()
        .filter(|c| matches!(c, FailureClass::EndTail))
        .count();
    assert_eq!(tails, 2);
    assert!(run.fatal.is_none());
}

#[test]
fn a_missing_full_length_last_segment_is_not_a_tail() {
    let mut cfg = DashConfig::default();
    cfg.max_segment_failures = 3;
    let mut origin = Origin::new().fault(Fault::always(
        "p2_dash_track1_3.m4s",
        FaultKind::Status(404),
    ));
    let run = run_restart(&mut origin, &Drive::default(), cfg);
    assert!(matches!(
        run.fatal,
        Some(FatalKind::SegmentFetchFailed {
            status: Some(404),
            ..
        })
    ));
    assert_eq!(run.requested("p2_dash_track1_3.m4s"), 3);
    assert!(!run.ended);
}

#[test]
fn a_missing_middle_segment_is_not_a_tail() {
    let mut cfg = DashConfig::default();
    cfg.max_segment_failures = 2;
    let mut origin = Origin::new().fault(Fault::always(
        "p1_dash_track1_2.m4s",
        FaultKind::Status(404),
    ));
    let run = run_restart(&mut origin, &Drive::default(), cfg);
    assert!(matches!(
        run.fatal,
        Some(FatalKind::SegmentFetchFailed { .. })
    ));
    assert_eq!(run.requested("p1_dash_track1_2.m4s"), 2);
}

fn continuous_expected() -> Vec<Vec<Sig>> {
    let mut files = vec!["timeline-gpac/seg-1-.mp4".to_owned()];
    for n in 1..=6 {
        files.push(format!("timeline-gpac/seg-1-{n}.m4s"));
    }
    vec![oracle_of(&files)]
}

fn continuous_origin(edit: impl Fn(String) -> String) -> Origin {
    let text = edit(mpd_text("multiperiod-continuous/multiperiod.mpd"));
    Origin::new().doc(CONTINUOUS, text.into_bytes())
}

#[test]
fn continuous_periods_join_without_a_discontinuity_or_a_second_init() {
    let run = drive(
        CONTINUOUS,
        DashConfig::default(),
        &mut Origin::new(),
        &Drive::default(),
    );
    assert!(run.errors.is_empty(), "{:?}", run.errors);
    assert!(run.ended && run.fatal.is_none());
    assert_matches_oracle(&run, &continuous_expected());
    assert_eq!(run.count(|o| matches!(o, Output::Discontinuity)), 0);
    assert_eq!(run.count(|o| matches!(o, Output::Init(_))), 1);
    assert_eq!(run.count(|o| matches!(o, Output::Tracks(_))), 1);
    assert_eq!(
        run.requested("seg-1-.mp4"),
        1,
        "the shared init is fetched once"
    );
    let flags: Vec<bool> = run
        .outputs
        .iter()
        .filter_map(|o| match o {
            Output::PeriodChanged { continuous, .. } => Some(*continuous),
            _ => None,
        })
        .collect();
    assert_eq!(flags, vec![false, true]);
}

#[test]
fn without_the_continuity_signal_the_same_boundary_is_a_discontinuity() {
    let mut origin = continuous_origin(|t| {
        let start = t.find("<SupplementalProperty").unwrap();
        let end = t[start..].find("/>").unwrap() + start + 2;
        format!("{}{}", &t[..start], &t[end..])
    });
    let run = drive(
        CONTINUOUS,
        DashConfig::default(),
        &mut origin,
        &Drive::default(),
    );
    assert!(run.ended && run.errors.is_empty());
    assert_matches_oracle(&run, &continuous_expected());
    assert_eq!(run.count(|o| matches!(o, Output::Discontinuity)), 1);
    assert_eq!(run.requested("seg-1-.mp4"), 1);
}

#[test]
fn continuity_naming_another_period_is_not_continuity() {
    let mut origin = continuous_origin(|t| t.replace("value=\"a\"", "value=\"zzz\""));
    let run = drive(
        CONTINUOUS,
        DashConfig::default(),
        &mut origin,
        &Drive::default(),
    );
    assert_eq!(run.count(|o| matches!(o, Output::Discontinuity)), 1);
}

#[test]
fn continuity_needs_the_same_adaptation_set_id() {
    let mut origin = continuous_origin(|t| {
        let at = t.rfind("<AdaptationSet id=\"1\"").unwrap();
        format!("{}<AdaptationSet id=\"7\"{}", &t[..at], &t[at + 21..])
    });
    let run = drive(
        CONTINUOUS,
        DashConfig::default(),
        &mut origin,
        &Drive::default(),
    );
    assert_eq!(run.count(|o| matches!(o, Output::Discontinuity)), 1);
}

#[test]
fn init_identity_ignores_the_query() {
    let mut origin = continuous_origin(|t| {
        let at = t
            .rfind("initialization=\"seg-$RepresentationID$-.mp4\"")
            .unwrap();
        format!(
            "{}initialization=\"seg-$RepresentationID$-.mp4?v=2\"{}",
            &t[..at],
            &t[at + 45..]
        )
    });
    let run = drive(
        CONTINUOUS,
        DashConfig::default(),
        &mut origin,
        &Drive::default(),
    );
    assert!(run.ended, "{:?}", run.fatal);
    assert_eq!(run.count(|o| matches!(o, Output::Discontinuity)), 0);
    assert_eq!(
        run.requested("seg-1-.mp4"),
        1,
        "same URL without query: same init"
    );
}

#[test]
fn continuity_with_a_different_init_url_is_not_continuous() {
    let init = test_fixtures::read("dash/timeline-gpac/seg-1-.mp4");
    let mut origin = continuous_origin(|t| {
        let at = t
            .rfind("initialization=\"seg-$RepresentationID$-.mp4\"")
            .unwrap();
        format!(
            "{}initialization=\"seg-$RepresentationID$-b.mp4\"{}",
            &t[..at],
            &t[at + 45..]
        )
    })
    .doc("http://dash.test/timeline-gpac/seg-1-b.mp4", init);
    let run = drive(
        CONTINUOUS,
        DashConfig::default(),
        &mut origin,
        &Drive::default(),
    );
    assert!(run.ended, "{:?} {:?}", run.fatal, run.errors);
    assert_matches_oracle(&run, &continuous_expected());
    assert_eq!(run.count(|o| matches!(o, Output::Discontinuity)), 1);
    assert_eq!(run.count(|o| matches!(o, Output::Init(_))), 2);
    assert_eq!(run.requested("seg-1-"), 8, "two inits and six segments");
}
