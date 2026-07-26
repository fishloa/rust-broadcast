//! `dash_parse` (issue #758 T1) gate — the MPD parser, exercised against a
//! real broadcast-derived fixture and cross-checked as the true structural
//! inverse of `DashPackager`'s writer.
//!
//! Two oracle sources:
//! - `fixtures/dash/manifest.mpd`: a real ffmpeg-generated DASH MPD (the same
//!   one `tests/dash.rs`/`tests/dash_mpd.rs` use as their writer oracle) —
//!   `SegmentTemplate`+`SegmentTimeline` addressing, two `AdaptationSet`s,
//!   `$Number%05d$` media templates. Parsing it and asserting the exact
//!   structural values is the real bite (never a bare substring check).
//! - `DashPackager::render` fed by a real `TsDemux`'d `Media`: rendered once
//!   under `Addressing::Number` and once under `Addressing::Timeline`, then
//!   re-parsed with `Mpd::parse` and asserted against what the packager was
//!   told to emit — proving `dash_parse` is the writer's true inverse for
//!   both addressing modes.

use std::path::PathBuf;

use broadcast_common::{Package, Unpackage};
use transmux::{Addressing, DashPackager, Mpd, MpdType, TrackSegments, TsDemux};

fn fixtures_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../fixtures")
}

fn demux_media() -> transmux::media::Media {
    let ts = std::fs::read(fixtures_dir().join("ts/h264_aac.ts")).expect("h264_aac.ts fixture");
    let mut demux = TsDemux::new();
    demux.unpackage(&ts[..]).expect("demux h264_aac.ts")
}

// ---------------------------------------------------------------------------
// Real fixture: fixtures/dash/manifest.mpd
// ---------------------------------------------------------------------------

#[test]
fn parses_real_fixture_structure() {
    let xml = std::fs::read_to_string(fixtures_dir().join("dash/manifest.mpd"))
        .expect("reference manifest.mpd must exist");
    let mpd = Mpd::parse(&xml).expect("parse real fixture MPD");

    assert_eq!(mpd.profiles, "urn:mpeg:dash:profile:isoff-live:2011");
    assert_eq!(mpd.mpd_type, MpdType::Static);
    assert_eq!(
        mpd.media_presentation_duration,
        Some(std::time::Duration::new(3, 0)),
        "mediaPresentationDuration=\"PT3.0S\""
    );
    assert_eq!(mpd.periods.len(), 1, "exactly one Period");

    let period = &mpd.periods[0];
    assert_eq!(period.id.as_deref(), Some("0"));
    assert_eq!(period.start, Some(std::time::Duration::new(0, 0)));
    assert_eq!(
        period.adaptation_sets.len(),
        2,
        "exactly two AdaptationSets (video + audio)"
    );

    let video_set = period
        .adaptation_sets
        .iter()
        .find(|s| s.content_type.as_deref() == Some("video"))
        .expect("video AdaptationSet");
    let audio_set = period
        .adaptation_sets
        .iter()
        .find(|s| s.content_type.as_deref() == Some("audio"))
        .expect("audio AdaptationSet");

    assert_eq!(video_set.representations.len(), 1);
    assert_eq!(audio_set.representations.len(), 1);

    // --- Video representation ---
    let v = &video_set.representations[0];
    assert_eq!(v.id, "0");
    assert_eq!(v.bandwidth, 58141);
    assert_eq!(v.codecs.as_deref(), Some("avc1.4d400d"));
    assert_eq!(v.mime_type.as_deref(), Some("video/mp4"));
    assert_eq!(v.width, Some(320));
    assert_eq!(v.height, Some(240));

    let v_st = v.segment_template.as_ref().expect("video SegmentTemplate");
    assert_eq!(v_st.timescale, 90000);
    assert_eq!(
        v_st.initialization.as_deref(),
        Some("init-stream$RepresentationID$.m4s")
    );
    assert_eq!(
        v_st.media.as_deref(),
        Some("chunk-stream$RepresentationID$-$Number%05d$.m4s")
    );
    assert_eq!(v_st.start_number, 1);
    assert_eq!(
        v_st.duration, None,
        "SegmentTimeline addressing, no @duration"
    );

    let v_timeline = v_st.timeline.as_ref().expect("video SegmentTimeline");
    assert_eq!(
        v_timeline.segments,
        vec![transmux::S {
            t: Some(2070),
            d: 90000,
            r: 2,
        }],
        "one S run, t=2070 d=90000 r=2"
    );
    assert_eq!(
        v_timeline.enumerate(v_st.start_number).expect("enumerate"),
        vec![(1, 2070), (2, 92070), (3, 182070)],
        "r=2 expands to 3 segments of duration 90000 each"
    );

    // --- Audio representation ---
    let a = &audio_set.representations[0];
    assert_eq!(a.id, "1");
    assert_eq!(a.bandwidth, 96013);
    assert_eq!(a.codecs.as_deref(), Some("mp4a.40.2"));
    assert_eq!(a.mime_type.as_deref(), Some("audio/mp4"));
    assert_eq!(a.audio_sampling_rate, Some(44100));

    let a_st = a.segment_template.as_ref().expect("audio SegmentTemplate");
    assert_eq!(a_st.timescale, 44100);
    assert_eq!(
        a_st.media.as_deref(),
        Some("chunk-stream$RepresentationID$-$Number%05d$.m4s")
    );

    let a_timeline = a_st.timeline.as_ref().expect("audio SegmentTimeline");
    assert_eq!(
        a_timeline.segments,
        vec![
            transmux::S {
                t: Some(0),
                d: 41984,
                r: 0
            },
            transmux::S {
                t: None,
                d: 44032,
                r: 0
            },
            transmux::S {
                t: None,
                d: 45056,
                r: 0
            },
            transmux::S {
                t: None,
                d: 3072,
                r: 0
            },
        ],
        "four unequal-duration S runs, only the first carrying @t"
    );
    assert_eq!(
        a_timeline.enumerate(a_st.start_number).expect("enumerate"),
        vec![(1, 0), (2, 41984), (3, 86016), (4, 131072)],
        "accumulated start times across S runs with no explicit @t"
    );

    // Template resolution end to end for both representations.
    assert_eq!(
        transmux::SegmentTemplate::resolve(
            v_st.media.as_ref().unwrap(),
            &v.id,
            Some(1),
            None,
            None
        ),
        "chunk-stream0-00001.m4s"
    );
    assert_eq!(
        transmux::SegmentTemplate::resolve(
            v_st.initialization.as_ref().unwrap(),
            &v.id,
            None,
            None,
            None
        ),
        "init-stream0.m4s"
    );
}

// ---------------------------------------------------------------------------
// Round-trip vs. the writer — Addressing::Number
// ---------------------------------------------------------------------------

#[test]
fn round_trip_against_writer_number_addressing() {
    let media = demux_media();
    let mut pkg = DashPackager::default(); // Addressing::Number is the default.
    let xml = pkg
        .package(&media)
        .expect("package DASH MPD (Number addressing)");

    let mpd = Mpd::parse(&xml).expect("parse packager output");
    assert_eq!(mpd.mpd_type, MpdType::Static);
    assert_eq!(mpd.periods.len(), 1);

    let mut expected_ids: Vec<String> = media
        .tracks
        .iter()
        .map(|t| t.spec.track_id.to_string())
        .collect();
    expected_ids.sort();

    let mut found_ids: Vec<String> = mpd.periods[0]
        .adaptation_sets
        .iter()
        .flat_map(|s| s.representations.iter())
        .map(|r| r.id.clone())
        .collect();
    found_ids.sort();
    assert_eq!(
        found_ids, expected_ids,
        "every Representation@id must equal a track_id the packager was given"
    );

    for set in &mpd.periods[0].adaptation_sets {
        for repr in &set.representations {
            assert!(repr.bandwidth > 0, "bandwidth must round-trip as positive");
            let st = repr
                .segment_template
                .as_ref()
                .expect("Number addressing always emits a SegmentTemplate");
            assert!(st.timescale > 0);
            assert_eq!(st.start_number, pkg_default_start_number());
            assert!(
                st.duration.is_some(),
                "Number addressing carries a nominal @duration"
            );
            assert!(
                st.timeline.is_none(),
                "Number addressing has no SegmentTimeline"
            );
            let media_tpl = st.media.as_ref().expect("media template");
            assert!(
                media_tpl.contains("$Number$") && media_tpl.contains("$RepresentationID$"),
                "media template must carry $Number$ and $RepresentationID$: {media_tpl}"
            );
            let init_tpl = st.initialization.as_ref().expect("init template");
            assert!(
                init_tpl.contains("$RepresentationID$"),
                "init template must carry $RepresentationID$: {init_tpl}"
            );

            // Resolve a concrete segment URL and check it's exactly what the
            // template + representation id predict.
            let resolved =
                transmux::SegmentTemplate::resolve(media_tpl, &repr.id, Some(3), None, None);
            let expected = media_tpl
                .replace("$RepresentationID$", &repr.id)
                .replace("$Number$", "3");
            assert_eq!(resolved, expected);
        }
    }
}

fn pkg_default_start_number() -> u64 {
    DashPackager::default().start_number
}

// ---------------------------------------------------------------------------
// Round-trip vs. the writer — Addressing::Timeline
// ---------------------------------------------------------------------------

#[test]
fn round_trip_against_writer_timeline_addressing() {
    let media = demux_media();

    // Give every track the same synthetic duration list: two equal 1000-tick
    // segments (run-length-encoded by the writer into one S with r=1) then
    // one 500-tick segment (a second S with no r).
    let durations = vec![1000u64, 1000, 500];
    let segments: Vec<TrackSegments> = media
        .tracks
        .iter()
        .map(|t| TrackSegments {
            track_id: t.spec.track_id,
            durations: durations.clone(),
        })
        .collect();

    let mut pkg = DashPackager {
        addressing: Addressing::Timeline,
        segments,
        ..DashPackager::default()
    };
    let xml = pkg
        .package(&media)
        .expect("package DASH MPD (Timeline addressing)");

    let mpd = Mpd::parse(&xml).expect("parse packager output");

    let mut any_checked = false;
    for set in &mpd.periods[0].adaptation_sets {
        for repr in &set.representations {
            any_checked = true;
            let st = repr
                .segment_template
                .as_ref()
                .expect("Timeline addressing always emits a SegmentTemplate");
            assert!(
                st.duration.is_none(),
                "Timeline addressing must not carry @duration (mutually exclusive, §5.3.9.4.4)"
            );
            let media_tpl = st.media.as_ref().expect("media template");
            assert!(
                media_tpl.contains("$Time$") && media_tpl.contains("$RepresentationID$"),
                "media template must carry $Time$ and $RepresentationID$: {media_tpl}"
            );

            let timeline = st.timeline.as_ref().expect("SegmentTimeline present");
            assert_eq!(
                timeline.segments,
                vec![
                    transmux::S {
                        t: Some(0),
                        d: 1000,
                        r: 1
                    },
                    transmux::S {
                        t: None,
                        d: 500,
                        r: 0
                    },
                ],
                "run-length-encoded S list must match the durations the packager was given"
            );

            // Enumeration must reproduce the exact original per-segment
            // duration list and cumulative start times.
            let pairs = timeline.enumerate(st.start_number).expect("enumerate");
            assert_eq!(pairs, vec![(1, 0), (2, 1000), (3, 2000)]);
        }
    }
    assert!(any_checked, "at least one Representation must be present");
}

// ---------------------------------------------------------------------------
// Remote alloc-DoS cap tests (issue #758 T1)
// ---------------------------------------------------------------------------

#[test]
fn timeline_enumerate_caps_unbounded_repeats() {
    // CRITICAL: a hostile MPD specifying a huge repeat count must be rejected
    // instantly, not allocated/looped. This test ensures the cap bites.
    let timeline = transmux::SegmentTimeline {
        segments: vec![transmux::S {
            t: Some(0),
            d: 1,
            r: 9_223_372_036_854_775_806, // i64::MAX - 1 → (as u64) + 1 = unbounded
        }],
    };
    let err = timeline
        .enumerate(1)
        .expect_err("must reject oversized repeat");
    assert!(
        matches!(err, transmux::DashParseError::TimelineTooLong { .. }),
        "error must be TimelineTooLong: {err:?}"
    );
}

#[test]
fn timeline_enumerate_accepts_under_cap() {
    // Valid timelines under the cap must still succeed.
    // E.g., 50k segments is well under the 100k cap.
    let timeline = transmux::SegmentTimeline {
        segments: vec![transmux::S {
            t: Some(0),
            d: 1,
            r: 49_999, // exactly 50k segments
        }],
    };
    let pairs = timeline
        .enumerate(1)
        .expect("timeline under cap must succeed");
    assert_eq!(
        pairs.len(),
        50_000,
        "50k segments must be enumerated when under cap"
    );
}

#[test]
fn segment_template_resolve_clamps_format_width() {
    // CRITICAL: a hostile @media template with $Number%9999999999d$ must not
    // allocate/loop unboundedly. The width is clamped to MAX_FORMAT_WIDTH.
    // Result is a sanely-sized zero-padded string, not OOM.
    let resolved = transmux::SegmentTemplate::resolve(
        "chunk-$Number%9999999999d$.m4s",
        "r0",
        Some(42),
        None,
        None,
    );
    assert!(
        resolved.starts_with("chunk-"),
        "template must resolve (not panic/OOM)"
    );
    // The number 42 padded to at most 20 digits is "00000000000000000042"
    // (20 chars). The resolved string should be reasonable in size.
    assert!(
        resolved.len() < 100,
        "resolved string must be small (clamped width): {resolved}"
    );
    assert!(
        resolved.contains("42"),
        "number must appear in resolved string: {resolved}"
    );
}

#[test]
fn mismatched_end_tag_causes_error() {
    // CRITICAL: a stray closing tag (malformed nesting) must error, not
    // silently truncate the structure. Example: <Period></Period></Period>
    // would previously accept the first </Period> and drop the second, now
    // it must error.
    let xml = r#"<MPD profiles="p">
        <Period id="0">
            <AdaptationSet contentType="video">
                <Representation id="0" bandwidth="1" />
            </AdaptationSet>
        </Period>
        </Period>
    </MPD>"#;
    let err = transmux::Mpd::parse(xml).expect_err("must reject stray </Period>");
    assert!(
        matches!(err, transmux::DashParseError::MismatchedEndTag { .. }),
        "error must be MismatchedEndTag: {err:?}"
    );
}
