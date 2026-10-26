//! Failure-injection matrix for `DashClient` on a static presentation: every
//! fault is injected transiently (the client recovers and the samples equal
//! the demux oracle) and permanently (the presentation ends with the typed
//! fatal after exactly the configured number of consecutive failures), for a
//! segment, an init, and the `sidx` and ranged subsegments of the on-demand
//! profile. Per cell: no panic, no hang, a bounded number of requests, and a
//! terminal state.

mod common;

use common::loopback::{
    Drive, Fault, FaultKind, Origin, Run, Sig, assert_matches_oracle, drive, oracle,
};
use dash_runtime::client::{
    DashConfig, Error, FailureClass, FailureKind, FatalKind, Rejection, ResourceId,
};

const FFMPEG: &str = "http://dash.test/timeline-ffmpeg-quirk/manifest.mpd";
const ONDEMAND: &str = "http://dash.test/ondemand-sidx/ondemand.mpd";
const SEGMENT: &str = "chunk-stream0-00002.m4s";
const INIT: &str = "init-stream0.m4s";
const INDEX_OFFSET: u64 = 918;
const FIRST_SUBSEGMENT: u64 = 1022;
const CAP: u32 = 4;
const TRANSIENT: usize = 2;

fn ffmpeg_expected() -> Vec<Vec<Sig>> {
    let dir = "timeline-ffmpeg-quirk";
    let mut v = vec![format!("{dir}/init-stream0.m4s")];
    for n in 1..=3 {
        v.push(format!("{dir}/chunk-stream0-{n:05}.m4s"));
    }
    let mut a = vec![format!("{dir}/init-stream1.m4s")];
    for n in 1..=4 {
        a.push(format!("{dir}/chunk-stream1-{n:05}.m4s"));
    }
    let mut out = oracle(&v.iter().map(String::as_str).collect::<Vec<_>>());
    out.extend(oracle(&a.iter().map(String::as_str).collect::<Vec<_>>()));
    out
}

fn ondemand_expected() -> Vec<Vec<Sig>> {
    let mut out = oracle(&["ondemand-sidx/v_dashinit.mp4"]);
    out.extend(oracle(&["ondemand-sidx/a_dashinit.mp4"]));
    out
}

fn cfg() -> DashConfig {
    let mut c = DashConfig::default();
    c.max_segment_failures = CAP;
    c.max_init_failures = CAP;
    c
}

fn play(url: &str, fault: Fault) -> Run {
    let mut origin = Origin::new().fault(fault);
    let run = drive(url, cfg(), &mut origin, &mut Drive::default());
    assert!(!run.stuck, "the client hung");
    assert!(run.ended ^ run.fatal.is_some(), "no terminal state");
    assert!(
        run.requests.len() < 200,
        "request storm: {}",
        run.requests.len()
    );
    run
}

fn kinds() -> Vec<(&'static str, FaultKind, Option<u16>)> {
    vec![
        ("404", FaultKind::Status(404), Some(404)),
        ("410", FaultKind::Status(410), Some(410)),
        ("425", FaultKind::Status(425), Some(425)),
        ("429", FaultKind::Status(429), Some(429)),
        ("500", FaultKind::Status(500), Some(500)),
        ("503", FaultKind::Status(503), Some(503)),
        ("transport", FaultKind::Transport, None),
        ("timeout", FaultKind::Timeout, None),
        ("truncated", FaultKind::Truncate(64), None),
        ("header-only", FaultKind::HeaderOnly, None),
        ("garbage-200", FaultKind::Garbage, None),
    ]
}

fn is_segment_fatal(f: &Option<FatalKind>, status: Option<u16>) -> bool {
    matches!(f, Some(FatalKind::SegmentFetchFailed { status: s, .. }) if *s == status)
}

fn is_init_fatal(f: &Option<FatalKind>, status: Option<u16>) -> bool {
    matches!(f, Some(FatalKind::InitFetchFailed { status: s, .. }) if *s == status)
}

#[test]
fn a_transient_segment_fault_recovers_with_every_sample() {
    for (name, kind, _) in kinds() {
        let run = play(FFMPEG, Fault::times(SEGMENT, kind, TRANSIENT));
        assert!(run.ended, "{name}: {:?}", run.fatal);
        assert_matches_oracle(&run, &ffmpeg_expected());
        assert_eq!(run.requested(SEGMENT), TRANSIENT + 1, "{name}");
    }
}

#[test]
fn a_permanent_segment_fault_ends_after_the_cap_with_the_typed_fatal() {
    for (name, kind, status) in kinds() {
        let run = play(FFMPEG, Fault::always(SEGMENT, kind));
        assert!(
            is_segment_fatal(&run.fatal, status),
            "{name}: {:?}",
            run.fatal
        );
        assert_eq!(run.requested(SEGMENT), CAP as usize, "{name}");
        let last = run.classes.last().copied();
        assert_eq!(last, Some(FailureClass::GiveUp), "{name}");
        let attempts: Vec<u32> = run
            .classes
            .iter()
            .filter_map(|c| match c {
                FailureClass::Retry { attempt, .. } => Some(*attempt),
                _ => None,
            })
            .collect();
        assert_eq!(attempts, vec![1, 2, 3], "{name}: attempts count up");
    }
}

#[test]
fn a_transient_init_fault_recovers_with_every_sample() {
    for (name, kind, _) in kinds() {
        let run = play(FFMPEG, Fault::times(INIT, kind, TRANSIENT));
        assert!(run.ended, "{name}: {:?}", run.fatal);
        assert_matches_oracle(&run, &ffmpeg_expected());
        assert_eq!(run.requested(INIT), TRANSIENT + 1, "{name}");
    }
}

#[test]
fn a_permanent_init_fault_ends_after_the_cap_with_the_typed_fatal() {
    for (name, kind, status) in kinds() {
        let run = play(FFMPEG, Fault::always(INIT, kind));
        assert!(is_init_fatal(&run.fatal, status), "{name}: {:?}", run.fatal);
        assert_eq!(run.requested(INIT), CAP as usize, "{name}");
        assert!(!run.ended);
    }
}

#[test]
fn nothing_is_delivered_after_a_fatal() {
    let run = play(FFMPEG, Fault::always(SEGMENT, FaultKind::Status(404)));
    let fatal_at = run
        .outputs
        .iter()
        .position(|o| matches!(o, dash_runtime::client::Output::Fatal(_)))
        .unwrap();
    assert_eq!(fatal_at, run.outputs.len() - 1, "Fatal is the last output");
}

#[test]
fn a_cap_of_zero_or_one_makes_the_first_failure_final() {
    for cap in [0u32, 1] {
        let mut c = cfg();
        c.max_segment_failures = cap;
        let mut origin = Origin::new().fault(Fault::always(SEGMENT, FaultKind::Status(503)));
        let run = drive(FFMPEG, c, &mut origin, &mut Drive::default());
        assert!(is_segment_fatal(&run.fatal, Some(503)), "cap {cap}");
        assert_eq!(run.requested(SEGMENT), 1, "cap {cap}");
    }
}

#[test]
fn an_unbounded_cap_retries_through_a_long_transient_fault() {
    let mut c = cfg();
    c.max_segment_failures = u32::MAX;
    let mut origin = Origin::new().fault(Fault::times(SEGMENT, FaultKind::Status(503), 30));
    let run = drive(FFMPEG, c, &mut origin, &mut Drive::default());
    assert!(run.ended);
    assert_matches_oracle(&run, &ffmpeg_expected());
    assert_eq!(run.requested(SEGMENT), 31);
}

#[test]
fn counted_failure_kinds_follow_the_status() {
    let run = play(FFMPEG, Fault::always(SEGMENT, FaultKind::Status(404)));
    assert!(run.classes.iter().any(|c| matches!(
        c,
        FailureClass::Retry {
            kind: FailureKind::Missing,
            ..
        }
    )));
    let run = play(FFMPEG, Fault::always(SEGMENT, FaultKind::Status(425)));
    assert!(run.classes.iter().any(|c| matches!(
        c,
        FailureClass::Retry {
            kind: FailureKind::TooEarly,
            ..
        }
    )));
    let run = play(FFMPEG, Fault::always(SEGMENT, FaultKind::Timeout));
    assert!(run.classes.iter().any(|c| matches!(
        c,
        FailureClass::Retry {
            kind: FailureKind::Transport,
            ..
        }
    )));
    let run = play(FFMPEG, Fault::always(SEGMENT, FaultKind::Garbage));
    assert!(run.classes.iter().any(|c| matches!(
        c,
        FailureClass::Retry {
            kind: FailureKind::Invalid,
            ..
        }
    )));
}

#[test]
fn rejection_reasons_are_typed() {
    let reasons = |run: &Run| -> Vec<Rejection> {
        run.errors
            .iter()
            .filter_map(|e| match e {
                Error::Rejected { reason, .. } => Some(*reason),
                _ => None,
            })
            .collect()
    };
    let run = play(FFMPEG, Fault::always(SEGMENT, FaultKind::Garbage));
    assert!(reasons(&run).iter().all(|r| *r == Rejection::NotMedia));
    let run = play(FFMPEG, Fault::always(SEGMENT, FaultKind::Truncate(64)));
    assert!(reasons(&run).iter().all(|r| *r == Rejection::NotMedia));
    let run = play(FFMPEG, Fault::always(SEGMENT, FaultKind::HeaderOnly));
    assert!(reasons(&run).iter().all(|r| *r == Rejection::NotMedia));
    let run = play(FFMPEG, Fault::always(INIT, FaultKind::Garbage));
    assert!(
        reasons(&run)
            .iter()
            .all(|r| *r == Rejection::InitUndecodable)
    );
    let run = play(FFMPEG, Fault::always(INIT, FaultKind::Truncate(100)));
    assert!(
        reasons(&run)
            .iter()
            .all(|r| *r == Rejection::InitUndecodable)
    );
}

#[test]
fn a_progressive_mp4_served_as_a_segment_is_fatal_at_once() {
    let progressive = test_fixtures::read("mp4/h264_high.mp4");
    let run = play(
        FFMPEG,
        Fault::always(SEGMENT, FaultKind::Replace(progressive)),
    );
    assert!(matches!(
        run.fatal,
        Some(FatalKind::NonFragmentedMp4 { .. })
    ));
    assert_eq!(run.requested(SEGMENT), 1);
}

#[test]
fn a_body_over_the_size_cap_is_a_counted_failure() {
    let mut c = cfg();
    c.max_segment_bytes = 1_000;
    let mut origin = Origin::new();
    let run = drive(FFMPEG, c, &mut origin, &mut Drive::default());
    assert!(
        matches!(run.fatal, Some(FatalKind::SegmentFetchFailed { .. })),
        "{:?}",
        run.fatal
    );
    assert!(run.errors.iter().any(|e| matches!(
        e,
        Error::Rejected {
            reason: Rejection::TooLarge,
            ..
        }
    )));
}

#[test]
fn a_caller_reported_oversize_is_counted_like_any_other_failure() {
    use dash_runtime::client::{Action, DashClient, Failure};
    let mut c = DashClient::new(FFMPEG, cfg());
    let Some(Action::FetchMpd { .. }) = c.poll(0) else {
        panic!("mpd first")
    };
    let mpd = test_fixtures::read("dash/timeline-ffmpeg-quirk/manifest.mpd");
    c.on_mpd(0, &mpd).unwrap();
    let id = loop {
        match c.poll(0) {
            Some(Action::FetchSegment { id, .. }) => break id,
            Some(_) => {}
            None => panic!("no segment requested"),
        }
    };
    let class = c.on_failure(0, id, Failure::TooLarge).unwrap();
    assert!(matches!(
        class,
        FailureClass::Retry {
            kind: FailureKind::Invalid,
            attempt: 1
        }
    ));
}

#[test]
fn sidx_faults_recover_or_end_with_the_init_counter() {
    for (name, kind, status) in kinds() {
        let run = play(
            ONDEMAND,
            Fault::times("v_dashinit.mp4", kind.clone(), TRANSIENT).at(INDEX_OFFSET),
        );
        assert!(run.ended, "{name}: {:?}", run.fatal);
        assert_matches_oracle(&run, &ondemand_expected());
        let run = play(
            ONDEMAND,
            Fault::always("v_dashinit.mp4", kind).at(INDEX_OFFSET),
        );
        assert!(is_init_fatal(&run.fatal, status), "{name}: {:?}", run.fatal);
        assert!(
            matches!(
                run.fatal,
                Some(FatalKind::InitFetchFailed {
                    id: ResourceId::Index { .. },
                    ..
                })
            ),
            "{name}"
        );
    }
}

#[test]
fn a_short_or_overlong_sidx_response_is_a_length_rejection() {
    for kind in [FaultKind::ShortRange(10), FaultKind::IgnoreRange] {
        let run = play(
            ONDEMAND,
            Fault::always("v_dashinit.mp4", kind.clone()).at(INDEX_OFFSET),
        );
        assert!(matches!(run.fatal, Some(FatalKind::InitFetchFailed { .. })));
        assert!(run.errors.iter().all(|e| matches!(
            e,
            Error::Rejected {
                reason: Rejection::WrongLength,
                ..
            }
        )));
        let run = play(
            ONDEMAND,
            Fault::times("v_dashinit.mp4", kind, TRANSIENT).at(INDEX_OFFSET),
        );
        assert!(run.ended);
        assert_matches_oracle(&run, &ondemand_expected());
    }
}

#[test]
fn a_server_that_ignores_the_range_header_is_caught_on_the_init() {
    let run = play(
        ONDEMAND,
        Fault::times("v_dashinit.mp4", FaultKind::IgnoreRange, TRANSIENT).at(0),
    );
    assert!(run.ended);
    assert_matches_oracle(&run, &ondemand_expected());
    assert!(run.errors.iter().any(|e| matches!(
        e,
        Error::Rejected {
            reason: Rejection::WrongLength,
            ..
        }
    )));
}

#[test]
fn subsegment_faults_recover_or_end_with_the_segment_counter() {
    for (name, kind, status) in kinds() {
        let run = play(
            ONDEMAND,
            Fault::times("v_dashinit.mp4", kind.clone(), TRANSIENT).at(FIRST_SUBSEGMENT),
        );
        assert!(run.ended, "{name}: {:?}", run.fatal);
        assert_matches_oracle(&run, &ondemand_expected());
        let run = play(
            ONDEMAND,
            Fault::always("v_dashinit.mp4", kind).at(FIRST_SUBSEGMENT),
        );
        assert!(
            is_segment_fatal(&run.fatal, status),
            "{name}: {:?}",
            run.fatal
        );
    }
}

#[test]
fn a_short_subsegment_is_a_length_rejection_not_a_demux_of_half_a_fragment() {
    let run = play(
        ONDEMAND,
        Fault::always("v_dashinit.mp4", FaultKind::ShortRange(5)).at(FIRST_SUBSEGMENT),
    );
    assert!(matches!(
        run.fatal,
        Some(FatalKind::SegmentFetchFailed { .. })
    ));
    assert!(run.errors.iter().all(|e| matches!(
        e,
        Error::Rejected {
            reason: Rejection::WrongLength,
            ..
        }
    )));
}

#[test]
fn the_mpd_is_never_given_up_on() {
    let mut origin = Origin::new().fault(Fault::always("manifest.mpd", FaultKind::Garbage));
    let run = drive(
        FFMPEG,
        cfg(),
        &mut origin,
        &mut Drive {
            max_steps: 50,
            ..Drive::default()
        },
    );
    assert!(!run.ended && run.fatal.is_none());
    assert!(run.errors.len() > 10);
    assert!(run.errors.iter().all(|e| matches!(e, Error::Mpd(_))));
}

#[test]
fn a_transient_mpd_fault_recovers() {
    for kind in [
        FaultKind::Garbage,
        FaultKind::Status(503),
        FaultKind::Transport,
        FaultKind::Truncate(200),
    ] {
        let mut origin = Origin::new().fault(Fault::times("manifest.mpd", kind, 3));
        let run = drive(FFMPEG, cfg(), &mut origin, &mut Drive::default());
        assert!(run.ended);
        assert_matches_oracle(&run, &ffmpeg_expected());
        assert_eq!(run.requested("manifest.mpd"), 4);
    }
}
