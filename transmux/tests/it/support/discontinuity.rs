//! Two-source inputs with a real timeline break, for the mux-stage
//! `MuxInput::Discontinuity` tests (issue #4154).
//!
//! One committed capture is demuxed twice, so every track's second run is the
//! same coded media on its own source timeline. [`forward`] places that second
//! source a gap after the first one's end (a forward jump); [`backwards`]
//! leaves its original timestamps, which start before the first source ended
//! (a backward jump). [`drive`] feeds first source, break, second source, so a
//! stage sees the exact order a live origin produces when it switches sources.

use broadcast_common::stage::{Stage, Timestamp};
use test_harness::collect::{Collected, CollectedTrack};
use transmux::ir::SampleTiming;
use transmux::mux_stage::MuxInput;
use transmux::{Sample, TrackId};

/// The real capture every two-source input is built from.
pub const SOURCE: &str = "ts/h264-aac.ts";

/// Media-time gap placed between the two sources by [`forward`].
const FORWARD_GAP_SECS: i64 = 10;

/// A joined two-source media with the per-track index where the second source
/// starts.
pub struct TwoSource {
    pub media: Collected,
    /// `split[track]` is the index of each track's first second-source sample.
    pub split: Vec<usize>,
}

/// Demux the capture once and use it as both sources, keeping each source's
/// own absolute timestamps.
fn sources() -> (Collected, Collected) {
    let bytes = test_fixtures::read(SOURCE);
    let first = test_harness::collect::media_from_ts(&bytes)
        .expect("demux the first source of the capture");
    let second = test_harness::collect::media_from_ts(&bytes)
        .expect("demux the second source of the capture");
    (first, second)
}

/// Shift one sample by `offset` ticks, keeping its span, flags, display facts
/// and protection.
fn shifted(sample: &Sample, offset: i64) -> Sample {
    let timing = sample.timing.map(|timing| match timing {
        SampleTiming::Timed { dts, pts, duration } => SampleTiming::Timed {
            dts: dts.saturating_add(offset),
            pts: pts.saturating_add(offset),
            duration,
        },
        other => other,
    });
    sample.clone().with_timing(timing)
}

/// Each track's first decode time and last decode-time end.
fn edges(track: &CollectedTrack) -> (i64, i64) {
    let first = track
        .samples
        .iter()
        .find_map(|s| s.timing.and_then(|t| t.dts()))
        .unwrap_or(0);
    let last = track
        .samples
        .last()
        .and_then(|s| {
            let dts = s.timing.and_then(|t| t.dts())?;
            let duration = s.timing.and_then(|t| t.duration()).unwrap_or(0);
            Some(dts.saturating_add(i64::from(duration)))
        })
        .unwrap_or(first);
    (first, last)
}

/// Join the two sources, giving the second one `offset_for(first, second)`
/// ticks.
fn join(mut offset_for: impl FnMut(&CollectedTrack, &CollectedTrack) -> i64) -> TwoSource {
    let (first, second) = sources();
    let mut split = Vec::with_capacity(first.tracks.len());
    let mut tracks = Vec::with_capacity(first.tracks.len());
    for (a, b) in first.tracks.iter().zip(&second.tracks) {
        let offset = offset_for(a, b);
        let mut samples = a.samples.clone();
        split.push(samples.len());
        samples.extend(b.samples.iter().map(|sample| shifted(sample, offset)));
        let start = samples
            .iter()
            .find_map(|s| s.timing.and_then(|t| t.dts()))
            .map(|dts| dts.max(0) as u64)
            .unwrap_or(a.start_decode_time);
        tracks.push(CollectedTrack::new_at(a.spec.clone(), samples, start));
    }
    TwoSource {
        media: Collected::new(tracks, first.movie_timescale),
        split,
    }
}

/// Two sources whose second run starts [`FORWARD_GAP_SECS`] after the first
/// run ends: a forward timeline break.
pub fn forward() -> TwoSource {
    join(|a, b| {
        let (_, end) = edges(a);
        let (start, _) = edges(b);
        end.saturating_add(FORWARD_GAP_SECS.saturating_mul(i64::from(a.spec.timescale.get())))
            .saturating_sub(start)
    })
}

/// Two sources whose second run keeps its own timestamps: it starts where the
/// first run started, so the seam moves media time backwards.
pub fn backwards() -> TwoSource {
    join(|_, _| 0)
}

/// Feed a two-source input to `stage`: every track, then the first source in
/// decode order, one [`MuxInput::Discontinuity`] naming `track` (`None` for
/// all tracks), then the second source.
pub fn drive<S, O>(
    stage: &mut S,
    input: &TwoSource,
    track: Option<TrackId>,
) -> Result<Vec<O>, S::Error>
where
    S: for<'a> Stage<In<'a> = MuxInput, Out = O>,
{
    drive_inner(stage, input, track, true)
}

/// [`drive`] for a stage whose track set is fixed at construction (a
/// segmenter): no `MuxInput::Track` announcements.
pub fn drive_fixed<S, O>(
    stage: &mut S,
    input: &TwoSource,
    track: Option<TrackId>,
) -> Result<Vec<O>, S::Error>
where
    S: for<'a> Stage<In<'a> = MuxInput, Out = O>,
{
    drive_inner(stage, input, track, false)
}

fn drive_inner<S, O>(
    stage: &mut S,
    input: &TwoSource,
    track: Option<TrackId>,
    announce: bool,
) -> Result<Vec<O>, S::Error>
where
    S: for<'a> Stage<In<'a> = MuxInput, Out = O>,
{
    let mut out = Vec::new();
    if announce {
        for track in &input.media.tracks {
            stage.feed(
                MuxInput::Track(Box::new(track.spec.clone())),
                Timestamp::ZERO,
            )?;
            drain(stage, &mut out);
        }
    }
    drive_range(stage, &input.media, &input.split, true, &mut out)?;
    stage.feed(MuxInput::Discontinuity { track }, Timestamp::ZERO)?;
    drain(stage, &mut out);
    drive_range(stage, &input.media, &input.split, false, &mut out)?;
    stage.finish()?;
    drain(stage, &mut out);
    Ok(out)
}

/// Feed a whole media to `stage` with no discontinuity at all.
pub fn drive_media_fixed<S, O>(stage: &mut S, media: &Collected) -> Result<Vec<O>, S::Error>
where
    S: for<'a> Stage<In<'a> = MuxInput, Out = O>,
{
    let mut out = Vec::new();
    let zeros = vec![0usize; media.tracks.len()];
    drive_range(stage, media, &zeros, false, &mut out)?;
    stage.finish()?;
    drain(stage, &mut out);
    Ok(out)
}

/// Feed one half of the joined media in decode-time order across tracks.
fn drive_range<S, O>(
    stage: &mut S,
    media: &Collected,
    split: &[usize],
    first_half: bool,
    out: &mut Vec<O>,
) -> Result<(), S::Error>
where
    S: for<'a> Stage<In<'a> = MuxInput, Out = O>,
{
    let bounds: Vec<(usize, usize)> = media
        .tracks
        .iter()
        .zip(split)
        .map(|(track, &split)| {
            if first_half {
                (0, split.min(track.samples.len()))
            } else {
                (split.min(track.samples.len()), track.samples.len())
            }
        })
        .collect();
    let mut next: Vec<usize> = bounds.iter().map(|&(lo, _)| lo).collect();
    loop {
        // Compare across tracks in nanoseconds: their decode times are in
        // their own timescales, so raw ticks are not comparable.
        let mut chosen: Option<(i128, usize)> = None;
        for (index, track) in media.tracks.iter().enumerate() {
            let (_, hi) = bounds[index];
            if next[index] >= hi {
                continue;
            }
            let Some(dts) = track.samples[next[index]].timing.and_then(|t| t.dts()) else {
                continue;
            };
            let nanos = i128::from(dts) * i128::from(broadcast_common::units::NANOS_PER_SEC)
                / i128::from(track.spec.timescale.get());
            if chosen.is_none_or(|(best, _)| nanos < best) {
                chosen = Some((nanos, index));
            }
        }
        let Some((_, index)) = chosen else {
            return Ok(());
        };
        let sample = media.tracks[index].samples[next[index]].clone();
        next[index] += 1;
        stage.feed(
            MuxInput::Sample {
                track: media.tracks[index].spec.track_id,
                sample,
            },
            Timestamp::ZERO,
        )?;
        drain(stage, out);
    }
}

fn drain<S, O>(stage: &mut S, out: &mut Vec<O>)
where
    S: for<'a> Stage<In<'a> = MuxInput, Out = O>,
{
    while let Some(item) = stage.poll() {
        out.push(item);
    }
}

/// Every `Cluster`'s absolute `Timestamp`, oldest first, read from the
/// muxer's bytes by the EBML syntax walker (not the muxer's own state).
///
/// # Panics
/// When the bytes are not a valid EBML document this test helper can walk.
pub fn cluster_timestamps(bytes: &[u8]) -> Vec<i64> {
    use broadcast_matroska::Limits;
    use broadcast_matroska::ebml::ElementIter;
    use broadcast_matroska::matroska::ids;

    let (_, _, ebml_end) = broadcast_matroska::ElementHeader::split(bytes).expect("EBML header");
    let rest = &bytes[ebml_end..];
    let (segment, used) = broadcast_matroska::ElementHeader::parse(rest).expect("Segment header");
    assert_eq!(
        segment.id().value(),
        ids::SEGMENT,
        "second element is the Segment"
    );
    let body = &rest[used..];
    let mut clusters = Vec::new();
    let mut walk = ElementIter::new(body, Limits::new());
    while let Some(element) = walk.next_element() {
        let element = element.expect("walk the Segment's children");
        if element.header().id().value() != ids::CLUSTER {
            continue;
        }
        let mut children = ElementIter::new(element.data(), Limits::new());
        let mut timestamp = None;
        while let Some(child) = children.next_element() {
            let child = child.expect("walk the Cluster's children");
            if child.header().id().value() == ids::TIMESTAMP {
                let mut value: i64 = 0;
                for byte in child.data() {
                    value = (value << u8::BITS) | i64::from(*byte);
                }
                timestamp = Some(value);
            }
        }
        clusters.push(timestamp.expect("a Cluster states its Timestamp"));
    }
    clusters
}

/// The earliest second-source Block timestamp in Matroska Ticks (1 ms), the
/// first Block a stage sees after the break.
///
/// # Panics
/// When a track's second source is empty in this fixture.
pub fn seam_matroska_ticks(input: &TwoSource) -> i64 {
    use broadcast_common::units::NANOS_PER_SEC;
    const MATROSKA_TICK_NS: i128 = 1_000_000;

    input
        .media
        .tracks
        .iter()
        .zip(&input.split)
        .filter_map(|(track, &split)| {
            let sample = track.samples.get(split)?;
            // A Block's timestamp is the sample's presentation time (falling
            // back to its decode time), which is what the writer orders by.
            let ticks = sample.timing.and_then(|t| t.pts().or_else(|| t.dts()))?;
            let nanos = i128::from(ticks) * i128::from(NANOS_PER_SEC)
                / i128::from(track.spec.timescale.get());
            Some(i64::try_from(nanos / MATROSKA_TICK_NS).unwrap_or(i64::MAX))
        })
        .min()
        .expect("every track has a second source")
}

/// Packet `dts_time` values ffprobe reads for one stream, or
/// `None` when ffprobe is not installed (the test then skips loudly).
pub fn ffprobe_dts(path: &std::path::Path, stream: &str) -> Option<Vec<f64>> {
    ffprobe_times(path, stream, "dts_time")
}

/// Packet `pts_time` values ffprobe reads for one stream. Matroska carries no
/// decode time, and ffprobe leaves a reordered frame's derived `dts` unset.
pub fn ffprobe_pts(path: &std::path::Path, stream: &str) -> Option<Vec<f64>> {
    ffprobe_times(path, stream, "pts_time")
}

fn ffprobe_times(path: &std::path::Path, stream: &str, field: &str) -> Option<Vec<f64>> {
    if !test_fixtures::process::tool_available("ffprobe", "-version") {
        eprintln!("skipping: ffprobe is not on PATH");
        return None;
    }
    let mut command = std::process::Command::new("ffprobe");
    command.args([
        "-v",
        "error",
        "-select_streams",
        stream,
        "-show_entries",
        &format!("packet={field}"),
        "-of",
        "csv=p=0",
    ]);
    command.arg(path);
    let output =
        test_fixtures::process::output_bounded(&mut command, std::time::Duration::from_secs(30))
            .expect("run ffprobe");
    assert!(
        output.status.success(),
        "ffprobe failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    Some(
        String::from_utf8_lossy(&output.stdout)
            .lines()
            .filter_map(|line| line.trim().trim_end_matches(',').parse().ok())
            .collect(),
    )
}

pub fn scratch_path(name: &str) -> std::path::PathBuf {
    let dir = std::env::var_os("CARGO_TARGET_TMPDIR")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| {
            std::env::current_dir()
                .expect("a working directory")
                .join("test-artifacts")
        });
    std::fs::create_dir_all(&dir).expect("create the scratch directory");
    dir.join(name)
}

pub fn assert_non_decreasing(name: &str, values: &[f64]) {
    for pair in values.windows(2) {
        assert!(
            pair[1] >= pair[0],
            "{name}: packet times must not step backwards: {} then {}",
            pair[0],
            pair[1]
        );
    }
}
