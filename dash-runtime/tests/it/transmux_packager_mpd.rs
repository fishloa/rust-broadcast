//! The MPDs `transmux`'s packagers write, read back with
//! `dash_runtime::mpd::Mpd::parse` (issue #2340): the writer's output is a
//! document dash-runtime's reader accepts, and it says what the packager was
//! told to emit.

use broadcast_common::{Package, Unpackage};
use dash_runtime::mpd::Mpd;
use dash_runtime::mpd::PresentationType;
use dash_runtime::mpd::inherit::SegmentInfo;
use dash_runtime::mpd::inherit::TemplateInfo;
use dash_runtime::mpd::inherit::resolve_segment_info;
use transmux::dash::{Addressing, DashPackager, TrackSegments};
use transmux::ts_demux::TsDemux;

fn media() -> transmux::media::Media {
    let ts = std::fs::read(test_fixtures::path("ts/h264-aac.ts")).expect("ts fixture");
    TsDemux::new().unpackage(&ts[..]).expect("demux")
}

fn parse(xml: &str) -> Mpd {
    Mpd::parse(xml.as_bytes())
        .unwrap_or_else(|e| panic!("reader must accept the writer: {e}\n{xml}"))
}

fn templates(mpd: &Mpd) -> Vec<(String, TemplateInfo)> {
    let mut out = Vec::new();
    for period in &mpd.periods {
        for set in &period.adaptation_sets {
            for rep in &set.representations {
                match resolve_segment_info(period, set, rep).expect("SegmentTemplate") {
                    SegmentInfo::Template(t) => out.push((rep.id.clone(), t)),
                    other => panic!("expected a SegmentTemplate, got {other:?}"),
                }
            }
        }
    }
    out
}

#[test]
fn number_addressing_round_trip() {
    let media = media();
    let mut pkg = DashPackager::default();
    let xml = pkg.package(&media).expect("package DASH MPD (Number)");
    let mpd = parse(&xml);
    assert_eq!(mpd.kind, PresentationType::Static);
    assert_eq!(mpd.periods.len(), 1);

    let mut expected_ids: Vec<String> = media
        .tracks
        .iter()
        .map(|t| t.spec.track_id.to_string())
        .collect();
    expected_ids.sort();
    let all = templates(&mpd);
    let mut found_ids: Vec<String> = all.iter().map(|(id, _)| id.clone()).collect();
    found_ids.sort();
    assert_eq!(
        found_ids, expected_ids,
        "every Representation@id must equal a track_id the packager was given"
    );

    for (id, st) in all {
        let rep = mpd.periods[0]
            .adaptation_sets
            .iter()
            .flat_map(|s| &s.representations)
            .find(|r| r.id == id)
            .expect("representation");
        assert!(rep.bandwidth > 0, "bandwidth must round-trip as positive");
        assert!(st.multi.timescale.get() > 0, "timescale must round-trip");
        assert_eq!(st.multi.start_number, pkg.start_number);
        assert!(
            st.multi.duration.is_some(),
            "Number addressing carries a nominal @duration"
        );
        assert!(
            st.multi.timeline.is_none(),
            "Number addressing has no SegmentTimeline"
        );
        let media_tpl = st.media.as_deref().expect("media template");
        assert!(
            media_tpl.contains("$Number$") && media_tpl.contains("$RepresentationID$"),
            "media template must carry $Number$ and $RepresentationID$: {media_tpl}"
        );
        let init_tpl = st.initialization.as_deref().expect("init template");
        assert!(
            init_tpl.contains("$RepresentationID$"),
            "init template must carry $RepresentationID$: {init_tpl}"
        );
    }
}

#[test]
fn timeline_addressing_round_trip() {
    let media = media();
    let durations = vec![1000u64, 1000, 500];
    let segments: Vec<TrackSegments> = media
        .tracks
        .iter()
        .map(|t| TrackSegments {
            track_id: t.spec.track_id,
            durations: durations.clone(),
        })
        .collect();
    let xml = DashPackager {
        addressing: Addressing::Timeline,
        segments,
        ..DashPackager::default()
    }
    .package(&media)
    .expect("package DASH MPD (Timeline)");
    let mpd = parse(&xml);

    let all = templates(&mpd);
    assert!(
        !all.is_empty(),
        "at least one Representation must be present"
    );
    for (id, st) in all {
        assert!(
            st.multi.duration.is_none(),
            "Timeline addressing must not carry @duration ({id})"
        );
        let media_tpl = st.media.as_deref().expect("media template");
        assert!(
            media_tpl.contains("$Time$") && media_tpl.contains("$RepresentationID$"),
            "media template must carry $Time$ and $RepresentationID$: {media_tpl}"
        );
        let runs: Vec<(u64, i64)> = st
            .multi
            .timeline
            .expect("SegmentTimeline present")
            .entries
            .iter()
            .map(|e| (e.d, e.r))
            .collect();
        assert_eq!(
            runs,
            vec![(1000, 1), (500, 0)],
            "run-length-encoded S list must match the durations the packager was given"
        );
    }
}

#[test]
fn packager_output_truncations_all_error() {
    let xml = DashPackager::default().package(&media()).expect("mpd");
    let complete = xml.trim_end().len();
    let mut errors = 0;
    for end in 1..xml.len() {
        if !xml.is_char_boundary(end) {
            continue;
        }
        let result = Mpd::parse(&xml.as_bytes()[..end]);
        if end < complete {
            assert!(
                result.is_err(),
                "DashPackager MPD: truncation at byte {end}/{} must be Err",
                xml.len()
            );
            errors += 1;
        } else {
            assert!(result.is_ok(), "the complete document must parse");
        }
    }
    assert!(errors > 1_000, "only {errors} truncations were cut");
}

#[test]
fn ll_utc_timing_escapes_scheme_and_value() {
    let mut pkg = transmux::ll_dash::LlDashPackager::new(
        2.0,
        0.5,
        std::time::Duration::from_secs(3),
        "2024-01-01T00:00:00Z",
    )
    .expect("packager");
    pkg.base.publish_time = Some("2024-01-01T00:00:00Z".to_string());
    let mut pkg = pkg.with_utc_timing("urn:x:a&b", "https://t.example/x?a=1&b=2");
    let xml = pkg.package(&media()).expect("LL MPD");
    let mpd = parse(&xml);
    let timing = mpd.utc_timings.first().expect("UTCTiming");
    assert_eq!(timing.scheme_id_uri, "urn:x:a&b");
    assert_eq!(timing.value, "https://t.example/x?a=1&b=2");
}
