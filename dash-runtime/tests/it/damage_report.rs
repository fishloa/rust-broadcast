//! The damage a DASH presentation reports: what the client passed over is one
//! report in the output order, the caller's policy decides whether it stops
//! the presentation, and a clean presentation reports nothing.

use common::loopback::{Drive, Origin, Run, drive, play};
use dash_runtime::client::{FatalKind, Output, PullLimits};

use crate::common;

fn damage(run: &Run) -> Vec<&broadcast_core::damage::Damage> {
    run.outputs
        .iter()
        .filter_map(|output| match output {
            Output::Damage(damage) => Some(damage),
            _ => None,
        })
        .collect()
}

/// `example-F4.mpd` carries an `EventStream` at Period level and an
/// `InbandEventStream` on its video set: neither is surfaced, which is a
/// report, not a silent drop.
#[test]
fn an_event_stream_the_client_ignores_is_reported() {
    let run = play("annex-g/example-F4.mpd", PullLimits::default());
    let reported = damage(&run);
    assert!(
        reported
            .iter()
            .any(|damage| damage.what() == "DASH EventStream"),
        "reports: {reported:?}"
    );
    assert!(
        run.outputs
            .iter()
            .any(|output| matches!(output, Output::Notice(_))),
        "the typed notice is still published alongside the report"
    );
    assert!(
        reported
            .iter()
            .all(|damage| damage.severity() == broadcast_core::damage::Severity::Informational),
        "an uninterpreted extension costs the viewer nothing"
    );
}

/// A policy that fails on informational reports stops the presentation at the
/// first one, carrying the same report.
#[test]
fn a_failing_policy_stops_at_the_first_report() {
    let mut origin = Origin::new();
    let run = drive(
        "http://dash.test/annex-g/example-F4.mpd",
        PullLimits::default(),
        &mut origin,
        &mut Drive {
            damage_policy: broadcast_core::damage::DamagePolicy::new()
                .fail_on(broadcast_core::damage::Severity::Informational),
            ..Drive::default()
        },
    );
    let FatalKind::Damage(ref refused) = *run.fatal.as_ref().expect("a fatal output") else {
        panic!("expected a damage refusal, got {:?}", run.fatal);
    };
    assert_eq!(refused.damage().what(), "DASH EventStream");
}

/// A presentation with nothing to skip reports nothing.
#[test]
fn a_clean_presentation_reports_nothing() {
    let run = play("manifest.mpd", PullLimits::default());
    assert!(
        damage(&run).is_empty(),
        "a clean presentation reports nothing: {:?}",
        damage(&run)
    );
}

/// Every notice converts to exactly one report, with the severity its cost to
/// the viewer implies.
#[test]
fn every_notice_converts_to_one_report() {
    use broadcast_core::damage::{DamageKind, Severity};
    use dash_runtime::client::Notice;
    let cases: &[(Notice, DamageKind, Severity)] = &[
        (
            Notice::UnsupportedEncryption,
            DamageKind::MetadataSkipped,
            Severity::Degraded,
        ),
        (
            Notice::EventStreamIgnored,
            DamageKind::MetadataUninterpreted,
            Severity::Informational,
        ),
        (
            Notice::PreselectionIgnored,
            DamageKind::MetadataUninterpreted,
            Severity::Informational,
        ),
        (
            Notice::XlinkSkipped,
            DamageKind::MetadataSkipped,
            Severity::Degraded,
        ),
        (
            Notice::PatchIgnored,
            DamageKind::MetadataUninterpreted,
            Severity::Informational,
        ),
        (
            Notice::LowLatencyIgnored,
            DamageKind::MetadataUninterpreted,
            Severity::Informational,
        ),
        (
            Notice::FellBehind,
            DamageKind::MediaUnitDropped,
            Severity::Degraded,
        ),
        (
            Notice::UtcSchemeUnsupported,
            DamageKind::MetadataUninterpreted,
            Severity::Informational,
        ),
        (
            Notice::UtcCrossOrigin,
            DamageKind::MetadataUninterpreted,
            Severity::Informational,
        ),
    ];
    for (notice, kind, severity) in cases {
        let report = notice.damage();
        assert_eq!(report.kind(), *kind, "{}", notice.name());
        assert_eq!(report.severity(), *severity, "{}", notice.name());
        assert!(!report.what().is_empty());
    }
}

/// A `SegmentTimeline` that declares a gap between two `S` runs is bridged
/// when the plan is built, and the plan says so; a contiguous one says
/// nothing. An `@t` that moves *backwards* is not a bridge at all — the plan
/// fails with `TimelineNotIncreasing` — which is what the addressing tests
/// pin.
#[test]
fn a_timeline_break_is_reported_by_the_plan() {
    use dash_runtime::address::{PlanOptions, plan_representation};
    use dash_runtime::selection::RepRef;

    fn boundaries(mpd: &str) -> Vec<dash_runtime::address::timeline::TimelineBoundary> {
        let mpd = dash_runtime::mpd::Mpd::parse(mpd.as_bytes()).expect("MPD");
        let planned =
            plan_representation(&mpd, RepRef::at(0, 0, 0), &PlanOptions::default()).expect("plan");
        planned.plan.timeline_boundaries()
    }

    fn mpd(first_d: u64, run: i64, second_t: u64) -> String {
        format!(
            r#"<?xml version="1.0"?>
<MPD xmlns="urn:mpeg:dash:schema:mpd:2011" type="static" mediaPresentationDuration="PT10S" minBufferTime="PT1S">
  <Period>
    <AdaptationSet mimeType="video/mp4" segmentAlignment="true">
      <SegmentTemplate timescale="10" initialization="i.mp4" media="$Time$.m4s">
        <SegmentTimeline>
          <S t="0" d="{first_d}" r="{run}"/>
          <S t="{second_t}" d="10" r="1"/>
        </SegmentTimeline>
      </SegmentTemplate>
      <Representation id="v" bandwidth="1000" codecs="avc1.4d401f" width="64" height="64"/>
    </AdaptationSet>
  </Period>
</MPD>"#
        )
    }

    // 0,10,20 then 40: 30..40 is media the MPD never addresses.
    let gap = boundaries(&mpd(10, 2, 40));
    assert_eq!(gap.len(), 1, "{gap:?}");
    assert_eq!(gap[0].ticks, 10);
    assert_eq!(gap[0].index, 3);

    // 0,10,20 then 30: the runs meet exactly.
    assert!(boundaries(&mpd(10, 2, 30)).is_empty());
}
