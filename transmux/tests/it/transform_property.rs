//! The pipeline property a stage inserted between a demuxer and a muxer must
//! be able to rely on, and the seam that makes it true.
//!
//! The property: a stage may replace a track's [`TrackSpec`], emit a different
//! number of samples than it took, delay its output by any number of inputs,
//! and flush at end of stream — and every consumer downstream accepts that
//! without special knowledge.
//!
//! The stage in this file does exactly that and holds no codec: it waits for
//! three inputs before announcing or emitting anything, announces a *different*
//! `TrackSpec` for one track (a different media timescale, with its samples
//! rescaled), emits one extra sample per batch, and flushes at `finish`. It is
//! inserted where a caller would insert one: `Repackage::run_transformed` and
//! the CLI assembly `run_bytes_with`.
//!
//! `media-plane`'s ingest path is the third site; its evidence lives in that
//! crate (`media_plane::segmenter::program`'s
//! `a_delayed_track_set_reaches_the_trunk_and_its_segmenter`), because
//! `ProgramSegmenter` is crate-private.

use core::time::Duration;
use std::collections::VecDeque;

use broadcast_common::stage::{Stage, Timestamp};
use broadcast_core::timescale::{Rounding, Timescale, rescale_i64};
use test_harness::collect::{CollectMode, Collected as Media, media_from_mp4};
use transmux::ir::SampleTiming;
use transmux::mux_stage::MuxInput;
use transmux::{Repackage, Sample, TrackId, TrackSpec};

/// Inputs the stage holds before it announces or emits anything.
const HOLD: usize = 3;

/// The media timescale the stage announces for the track it rewrites: whole
/// milliseconds, so a rescaled tick is plainly a different time base.
fn announced_timescale() -> Timescale {
    Timescale::try_new(1000).expect("a non-zero timescale")
}

fn samples_of(media: &Media, track: TrackId) -> Vec<Sample> {
    media
        .tracks
        .iter()
        .find(|t| t.spec.track_id == track)
        .map(|t| t.samples.clone())
        .expect("the track is in the output")
}

fn spec_of(media: &Media, track: TrackId) -> TrackSpec {
    media
        .tracks
        .iter()
        .find(|t| t.spec.track_id == track)
        .map(|t| t.spec.clone())
        .expect("the track is in the output")
}

/// The test-only stage: holds [`HOLD`] inputs, announces a different
/// `TrackSpec` for `target`, emits one extra sample per batch, flushes at
/// `finish`. It holds no codec knowledge and never inspects a coded payload.
struct ReTiming {
    target: TrackId,
    source: Option<TrackSpec>,
    held: VecDeque<(TrackId, Sample)>,
    ready: VecDeque<MuxInput>,
    announced: bool,
}

impl ReTiming {
    fn new(target: TrackId) -> Self {
        Self {
            target,
            source: None,
            held: VecDeque::new(),
            ready: VecDeque::new(),
            announced: false,
        }
    }

    /// The announcement: the source spec with the announced media timescale,
    /// so the muxer builds its track description from *this*, not the source's.
    fn spec(&self) -> Option<TrackSpec> {
        let source = self.source.as_ref()?;
        Some(TrackSpec::new(
            source.track_id,
            announced_timescale(),
            source.config.clone(),
        ))
    }

    /// One sample in the announced timescale, with its timing rescaled.
    fn rescale(&self, sample: &Sample) -> Option<Sample> {
        let source = self.source.as_ref()?.timescale;
        let timing = sample.timing?;
        let dts = rescale_i64(
            timing.dts()?,
            source,
            announced_timescale(),
            Rounding::Floor,
        );
        let pts = rescale_i64(
            timing.pts()?,
            source,
            announced_timescale(),
            Rounding::Floor,
        );
        let duration = timing
            .duration()
            .map(|d| rescaled_duration(d, source))
            .filter(|d| *d > 0);
        Some(sample.clone().with_timing(Some(match duration {
            Some(duration) => SampleTiming::timed_with_duration(dts, pts, duration),
            None => SampleTiming::timed(dts, pts),
        })))
    }

    /// Announce once, then release every held input; the first batch also
    /// gains one extra sample between its first two, so a batch of three
    /// becomes four outputs.
    fn release(&mut self) {
        if !self.announced {
            self.announced = true;
            if let Some(spec) = self.spec() {
                self.ready.push_back(MuxInput::Track(Box::new(spec)));
            }
        }
        let held: Vec<(TrackId, Sample)> = self.held.drain(..).collect();
        let mut extra_done = false;
        let mut emitted: Option<Sample> = None;
        for (track, sample) in held {
            let Some(rescaled) = self.rescale(&sample) else {
                self.ready.push_back(MuxInput::Sample { track, sample });
                continue;
            };
            if track == self.target {
                if let (false, Some(previous)) = (extra_done, emitted.as_ref())
                    && let (Some(a), Some(b)) = (
                        previous.timing.and_then(|t| t.dts()),
                        rescaled.timing.and_then(|t| t.dts()),
                    )
                    && b > a + 1
                {
                    let middle = a + (b - a).div_euclid(2);
                    let copy = previous.clone().with_timing(Some(SampleTiming::timed(
                        middle,
                        middle + (previous.timing.and_then(|t| t.pts()).unwrap_or(middle) - a),
                    )));
                    self.ready.push_back(MuxInput::Sample {
                        track,
                        sample: copy,
                    });
                    extra_done = true;
                }
                emitted = Some(rescaled.clone());
                self.ready.push_back(MuxInput::Sample {
                    track,
                    sample: rescaled,
                });
            } else {
                self.ready.push_back(MuxInput::Sample {
                    track,
                    sample: rescaled,
                });
            }
        }
    }
}

fn rescaled_duration(ticks: u32, from: Timescale) -> u32 {
    let ticks = rescale_i64(
        i64::from(ticks),
        from,
        announced_timescale(),
        Rounding::Floor,
    );
    u32::try_from(ticks).unwrap_or(u32::MAX)
}

impl Stage for ReTiming {
    type In<'a> = MuxInput;
    type Out = MuxInput;
    type Error = transmux::Error;

    fn feed(&mut self, input: MuxInput, _now: Timestamp) -> transmux::Result<()> {
        match input {
            MuxInput::Track(spec) => {
                if spec.track_id != self.target {
                    self.ready.push_back(MuxInput::Track(spec));
                    return Ok(());
                }
                self.source = Some(*spec);
                Ok(())
            }
            MuxInput::Sample { track, sample } => {
                if track != self.target {
                    self.ready.push_back(MuxInput::Sample { track, sample });
                    return Ok(());
                }
                self.held.push_back((track, sample));
                if self.held.len() >= HOLD {
                    self.release();
                }
                Ok(())
            }
            other => {
                self.ready.push_back(other);
                Ok(())
            }
        }
    }

    fn poll(&mut self) -> Option<MuxInput> {
        self.ready.pop_front()
    }

    fn finish(&mut self) -> transmux::Result<()> {
        if !self.held.is_empty() || !self.announced {
            self.release();
        }
        Ok(())
    }
}

/// The oracle: `fixtures/ts/h264-aac.ts` has 75 video and 131 audio samples.
fn oracle_ir() -> Media {
    let data = test_fixtures::read("ts/h264-aac.ts");
    test_harness::collect::media_from_ts(&data).expect("ts demux")
}

/// `Repackage::run_transformed` takes a stage that rewrites a track's spec,
/// delays its output and emits more samples than it took, and the resegmented
/// output describes the *stage's* track.
#[test]
fn repackage_takes_a_spec_rewriting_stage() {
    let ir = oracle_ir();
    let source = test_harness::collect::cmaf_from_media(&ir, Duration::from_secs(2))
        .expect("build the source CMAF")
        .to_contiguous();
    let audio = ir.tracks[1].spec.track_id;

    let plain = Repackage::new(Duration::from_secs(1))
        .run(&source)
        .expect("repackage without a stage");
    let transformed = Repackage::new(Duration::from_secs(1))
        .run_transformed(&source, ReTiming::new(audio))
        .expect("repackage with a stage");

    let plain = media_from_mp4(&plain.to_contiguous(), CollectMode::Seekable).expect("re-demux");
    let transformed =
        media_from_mp4(&transformed.to_contiguous(), CollectMode::Seekable).expect("re-demux");

    assert_eq!(
        spec_of(&transformed, audio).timescale,
        announced_timescale(),
        "the output track carries the stage's timescale, not the source's"
    );
    assert_eq!(
        spec_of(&plain, audio).timescale,
        ir.tracks[1].spec.timescale,
        "without the stage the source timescale stands"
    );

    let before = samples_of(&plain, audio);
    let after = samples_of(&transformed, audio);
    assert_eq!(
        after.len(),
        before.len() + before.len().div_ceil(HOLD),
        "every batch of {HOLD} inputs gains one output, and nothing is lost"
    );

    let mut last: Option<i64> = None;
    for sample in &after {
        let dts = sample.timing.expect("a timed sample").dts().expect("a dts");
        assert!(
            last.is_none_or(|previous| dts >= previous),
            "timestamps stay monotonic across the stage"
        );
        last = Some(dts);
    }
}

/// A transform that never settles the track set is bounded by the caller's
/// cap rather than left to grow: `MuxChain` holds whole samples, so it says so.
#[test]
fn a_chain_that_never_settles_is_bounded_by_its_callers_cap() {
    use transmux::mux_stage::MuxChain;

    let (specs, samples) = {
        let data = test_fixtures::read("ts/h264-aac.ts");
        let media = test_harness::collect::media_from_ts(&data).expect("demux");
        let specs: Vec<TrackSpec> = media.tracks.iter().map(|t| t.spec.clone()).collect();
        let mut samples = Vec::new();
        for track in &media.tracks {
            for sample in &track.samples {
                samples.push((track.spec.track_id, sample.clone()));
            }
        }
        (specs, samples)
    };
    let target = specs[1].track_id;
    // Holds everything and never announces: the chain has no way to settle.
    struct Queue(TrackId, VecDeque<MuxInput>);
    impl Stage for Queue {
        type In<'a> = MuxInput;
        type Out = MuxInput;
        type Error = transmux::Error;

        fn feed(&mut self, input: MuxInput, _now: Timestamp) -> transmux::Result<()> {
            match input {
                MuxInput::Track(spec) if spec.track_id == self.0 => {}
                other => self.1.push_back(other),
            }
            Ok(())
        }

        fn poll(&mut self) -> Option<MuxInput> {
            self.1.pop_front()
        }

        fn finish(&mut self) -> transmux::Result<()> {
            Ok(())
        }
    }
    let expected: Vec<TrackId> = specs.iter().map(|spec| spec.track_id).collect();
    let mut chain = MuxChain::new(Queue(target, VecDeque::new()), expected).hold_limit(16);
    let mut err = None;
    for (track, sample) in &samples {
        if *track == target {
            continue;
        }
        if let Err(e) = chain.feed(MuxInput::Sample {
            track: *track,
            sample: sample.clone(),
        }) {
            err = Some(e);
            break;
        }
    }
    let err = err.expect("the chain refuses to hold more than its caller allows");
    assert!(
        matches!(err, transmux::Error::TransformHoldExceeded { limit: 16 }),
        "the cap is the caller's, got {err:?}"
    );
}

/// The untouched track's coded samples are byte-identical with and without the
/// stage: a transform of one track leaves the others alone.
#[test]
fn a_spec_rewriting_stage_leaves_the_other_track_byte_identical() {
    let ir = oracle_ir();
    let source = test_harness::collect::cmaf_from_media(&ir, Duration::from_secs(2))
        .expect("build the source CMAF")
        .to_contiguous();
    let audio = ir.tracks[1].spec.track_id;
    let video = ir.tracks[0].spec.track_id;

    let plain = Repackage::new(Duration::from_secs(1))
        .run(&source)
        .expect("repackage without a stage");
    let transformed = Repackage::new(Duration::from_secs(1))
        .run_transformed(&source, ReTiming::new(audio))
        .expect("repackage with a stage");

    let plain = media_from_mp4(&plain.to_contiguous(), CollectMode::Seekable).expect("re-demux");
    let transformed =
        media_from_mp4(&transformed.to_contiguous(), CollectMode::Seekable).expect("re-demux");

    let plain_video = samples_of(&plain, video);
    let transformed_video = samples_of(&transformed, video);
    assert_eq!(
        plain_video.len(),
        transformed_video.len(),
        "the untouched track keeps its sample count"
    );
    for (a, b) in plain_video.iter().zip(transformed_video.iter()) {
        assert_eq!(
            a.data, b.data,
            "the untouched track's payloads are unchanged"
        );
        assert_eq!(
            a.timing, b.timing,
            "the untouched track's timing is unchanged"
        );
    }
}

/// The CLI assembly takes the same stage: the output demuxes back with the
/// stage's track description and more audio samples than the input carried.
#[cfg(feature = "cli")]
#[test]
fn the_cli_assembly_takes_a_spec_rewriting_stage() {
    use transmux::cli::{Container, Opts, OutputFormat, run_bytes_with};

    let data = test_fixtures::read("ts/h264-aac.ts");
    let opts = Opts {
        format: OutputFormat::Cmaf,
        segment_duration: 1,
        ..Opts::default()
    };

    let plain = match transmux::cli::run_bytes(&data, &opts).expect("plain run") {
        transmux::cli::Output::Bytes(bytes) => bytes,
        other => panic!("CMAF output is binary, got {other:?}"),
    };
    let plain = media_from_mp4(&plain, CollectMode::Seekable).expect("re-demux the plain output");
    let audio = plain
        .tracks
        .iter()
        .find(|t| matches!(t.spec.config, transmux::CodecConfig::Aac { .. }))
        .map(|t| t.spec.track_id)
        .expect("the fixture has an AAC track");

    let transformed =
        match run_bytes_with(&data, &opts, ReTiming::new(audio)).expect("run with a stage") {
            transmux::cli::Output::Bytes(bytes) => bytes,
            other => panic!("CMAF output is binary, got {other:?}"),
        };
    let transformed = media_from_mp4(&transformed, CollectMode::Seekable)
        .expect("re-demux the transformed output");

    assert_eq!(
        spec_of(&transformed, audio).timescale,
        announced_timescale(),
        "the CLI's output describes the stage's track"
    );
    assert_eq!(
        samples_of(&transformed, audio).len(),
        samples_of(&plain, audio).len() + samples_of(&plain, audio).len().div_ceil(HOLD),
        "every batch gains one output and nothing is lost"
    );
    assert_eq!(
        transmux::cli::detect_container(&data).expect("detect"),
        Container::MpegTs,
        "the fixture is the container the CLI autodetects"
    );
}
