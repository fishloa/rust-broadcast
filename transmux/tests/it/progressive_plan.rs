//! The plan pass and the plan-driven progressive muxer: a real TS capture whose
//! AAC is ADTS and whose H.264 is Annex B (the demuxer conditions both before
//! the plan sees them), a plan mismatch refused by track and index, and the
//! append-only sink.
//!
//! [`PlanPass`] must record the sizes the muxer will actually be fed. If the
//! plan came from the raw PES payloads rather than the conditioned samples, the
//! muxer would refuse the first sample with [`Error::PlanMismatch`]; feeding the
//! same `Media` to both passes is what makes the two agree, so every test here
//! drives both from one demux.
use broadcast_common::stage::{Stage, Timestamp};
use bytes::Bytes;
use isobmff::writer::{Interleave, Layout, MovieHeader, Sink};
use test_harness::collect::Collected as Media;
use test_harness::collect::drive;
use transmux::mux_stage::MuxInput;
use transmux::progressive::ProgressiveMuxStage;
use transmux::{Error, TrackId};

/// One interleaved chunk of the CLI's layout, in milliseconds.
const INTERLEAVE_MILLIS: u32 = 500;
/// A caller's append-only reorder window: comfortably more than one half-second
/// interleaved chunk of the two-track capture, so no sample is refused.
const REORDER_WINDOW: u64 = 1024 * 1024;

fn demux() -> Media {
    let data = test_fixtures::read("ts/h264-aac.ts");
    test_harness::collect::media_from_ts(data.as_slice()).expect("demux h264-aac.ts")
}

fn layout(faststart: bool, sink: Sink) -> Layout {
    Layout::new(
        faststart,
        Interleave::ByDuration {
            millis: INTERLEAVE_MILLIS,
        },
        sink,
    )
}

fn stage(
    plan: isobmff::plan::OwnedPlan,
    media: &Media,
    faststart: bool,
    sink: Sink,
) -> ProgressiveMuxStage {
    ProgressiveMuxStage::new(
        plan,
        layout(faststart, sink),
        MovieHeader::new(media.movie_timescale),
    )
}

// ---------------------------------------------------------------------------
// 1. The plan's sizes are the conditioned samples' sizes.
// ---------------------------------------------------------------------------

#[test]
fn plan_sizes_match_the_conditioned_samples() {
    let media = demux();
    let plan = test_harness::collect::plan_from_media(&media).expect("plan the capture");
    assert_eq!(
        plan.tracks().len(),
        media.tracks.len(),
        "one plan track per track"
    );
    for track in &media.tracks {
        let planned = plan
            .track(isobmff::TrackId::new(track.spec.track_id.value()))
            .expect("planned track");
        assert_eq!(
            usize::try_from(planned.tables.sample_count).expect("sample count fits"),
            track.samples.len(),
            "track {} sample count",
            track.spec.track_id
        );
        for (index, sample) in track.samples.iter().enumerate() {
            let index = u32::try_from(index).expect("index fits");
            let size = planned.tables.size(index).expect("planned size");
            assert_eq!(
                usize::try_from(size).expect("size fits"),
                sample.data.len(),
                "track {} sample {index}: the plan must see post-conditioning sizes",
                track.spec.track_id
            );
        }
    }
}

// ---------------------------------------------------------------------------
// 2. The muxer takes the plan and the file demuxes back to the same bytes.
// ---------------------------------------------------------------------------

#[test]
fn planned_output_demuxes_back_with_the_same_sample_bytes() {
    let media = demux();
    let out = crate::progressive_support::package(&media, true);
    let back =
        test_harness::collect::media_from_mp4(&out, test_harness::collect::CollectMode::Seekable)
            .expect("demux the progressive output");

    assert_eq!(back.tracks.len(), media.tracks.len(), "track count");
    for source in &media.tracks {
        let decoded = back
            .tracks
            .iter()
            .find(|t| t.spec.track_id == source.spec.track_id)
            .unwrap_or_else(|| panic!("track {} came back", source.spec.track_id));
        assert_eq!(
            decoded.samples.len(),
            source.samples.len(),
            "track {} sample count",
            source.spec.track_id
        );
        for (index, (a, b)) in source.samples.iter().zip(&decoded.samples).enumerate() {
            assert_eq!(
                a.data, b.data,
                "track {} sample {index} bytes",
                source.spec.track_id
            );
        }
    }
}

// ---------------------------------------------------------------------------
// 3. An append-only sink buffers at most one interleave window and writes the
//    same file.
// ---------------------------------------------------------------------------

#[test]
fn an_append_only_sink_writes_the_same_file() {
    let media = demux();
    for faststart in [true, false] {
        let seekable =
            crate::progressive_support::package_with_sink(&media, faststart, Sink::Seekable);
        let appended = crate::progressive_support::package_with_sink(
            &media,
            faststart,
            Sink::AppendOnly {
                reorder_bytes: REORDER_WINDOW,
            },
        );
        assert_eq!(
            seekable, appended,
            "faststart={faststart}: the sink must not change the file"
        );
    }
}

// ---------------------------------------------------------------------------
// 4. A sample that disagrees with the plan is refused, naming track + index.
// ---------------------------------------------------------------------------

#[test]
fn a_larger_sample_is_refused_naming_track_and_index() {
    let media = demux();
    let plan = test_harness::collect::plan_from_media(&media).expect("plan the capture");
    let mut mutated = media.clone();
    let track = mutated.tracks.first_mut().expect("a track");
    let id = track.spec.track_id;
    let sample = track.samples.first_mut().expect("a sample");
    let mut longer = Vec::new();
    longer.extend_from_slice(&sample.data);
    longer.push(0);
    sample.data = Bytes::from(longer);
    let mut stage = stage(plan, &media, true, Sink::Seekable);
    let error = drive(&mut stage, &mutated, Timestamp::ZERO).expect_err("a size mismatch");
    match error {
        Error::PlanMismatch {
            track: mismatch,
            index,
            what,
            planned,
            got,
        } => {
            assert_eq!(mismatch, id, "the mismatch names the track");
            assert_eq!(index, 0, "the mismatch names the sample index");
            assert_eq!(what, "size");
            assert_eq!(planned + 1, got);
        }
        other => panic!("expected PlanMismatch, got {other:?}"),
    }
}

#[test]
fn a_caller_side_short_sample_is_refused_naming_track_and_index() {
    let media = demux();
    let plan = test_harness::collect::plan_from_media(&media).expect("plan the capture");
    let mut mutated = media.clone();
    let track = mutated.tracks.first_mut().expect("a track");
    let id = track.spec.track_id;
    let sample = track.samples.get_mut(2).expect("three samples");
    let shorter = sample.data.slice(0..sample.data.len().saturating_sub(1));
    sample.data = shorter;
    let mut stage = stage(plan, &media, true, Sink::Seekable);
    let error = drive(&mut stage, &mutated, Timestamp::ZERO).expect_err("a size mismatch");
    match error {
        Error::PlanMismatch {
            track: mismatch,
            index,
            what,
            ..
        } => {
            assert_eq!(mismatch, id);
            assert_eq!(index, 2);
            assert_eq!(what, "size");
        }
        other => panic!("expected PlanMismatch, got {other:?}"),
    }
}

#[test]
fn a_missing_sample_is_refused_at_finish() {
    let media = demux();
    let plan = test_harness::collect::plan_from_media(&media).expect("plan the capture");
    let first = media.tracks.first().expect("a track");
    let id = first.spec.track_id;
    let missing = u32::try_from(first.samples.len().saturating_sub(1)).expect("index fits");
    let mut stage = stage(plan, &media, true, Sink::Seekable);
    for track in &media.tracks {
        stage
            .feed(
                MuxInput::Track(Box::new(track.spec.clone())),
                Timestamp::ZERO,
            )
            .expect("announce");
    }
    for (index, track) in media.tracks.iter().enumerate() {
        let skip = if index == 0 {
            Some(first.samples.len() - 1)
        } else {
            None
        };
        for (sample_index, sample) in track.samples.iter().enumerate() {
            if skip == Some(sample_index) {
                continue;
            }
            stage
                .feed(
                    MuxInput::Sample {
                        track: track.spec.track_id,
                        sample: sample.clone(),
                    },
                    Timestamp::ZERO,
                )
                .expect("push");
        }
    }
    let error = stage.finish().expect_err("a missing sample");
    match error {
        Error::PlanMismatch {
            track, index, what, ..
        } => {
            assert_eq!(track, id);
            assert_eq!(index, missing);
            assert_eq!(what, "missing sample");
        }
        other => panic!("expected PlanMismatch, got {other:?}"),
    }
}

#[test]
fn a_sample_past_the_plan_is_refused() {
    let media = demux();
    let plan = test_harness::collect::plan_from_media(&media).expect("plan the capture");
    let first = media.tracks.first().expect("a track");
    let id = first.spec.track_id;
    let mut stage = stage(plan, &media, true, Sink::Seekable);
    for track in &media.tracks {
        stage
            .feed(
                MuxInput::Track(Box::new(track.spec.clone())),
                Timestamp::ZERO,
            )
            .expect("announce");
    }
    for track in &media.tracks {
        for sample in &track.samples {
            stage
                .feed(
                    MuxInput::Sample {
                        track: track.spec.track_id,
                        sample: sample.clone(),
                    },
                    Timestamp::ZERO,
                )
                .expect("push");
        }
    }
    let duplicate = first.samples.first().expect("a sample").clone();
    let error = stage
        .feed(
            MuxInput::Sample {
                track: first.spec.track_id,
                sample: duplicate,
            },
            Timestamp::ZERO,
        )
        .expect_err("a sample past the plan");
    match error {
        Error::PlanMismatch {
            track, index, what, ..
        } => {
            assert_eq!(track, id);
            assert_eq!(
                index,
                u32::try_from(first.samples.len()).expect("index fits")
            );
            assert_eq!(what, "sample count");
        }
        other => panic!("expected PlanMismatch, got {other:?}"),
    }
}

#[test]
fn an_unplanned_track_is_refused() {
    let media = demux();
    let plan = test_harness::collect::plan_from_media(&media).expect("plan the capture");
    let mut stage = stage(plan, &media, true, Sink::Seekable);
    let sample = media.tracks.first().expect("a track").samples[0].clone();
    let error = stage
        .feed(
            MuxInput::Sample {
                track: TrackId::new(999),
                sample,
            },
            Timestamp::ZERO,
        )
        .expect_err("an unplanned track");
    assert!(
        matches!(error, Error::InvalidInput(_)),
        "expected InvalidInput, got {error:?}"
    );
}
