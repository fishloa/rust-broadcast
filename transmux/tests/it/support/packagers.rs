//! The whole-[`Media`] bridge the manifest tests hold their input through: a
//! track-spec list plus a [`SegmentSummaryLog`], exactly what the renderers
//! take ([`crate::segment_log`]).

use test_harness::collect::Collected as Media;
use transmux::dash::DashPackager;
use transmux::ir::TrackSpec;
use transmux::ll_dash::LlDashPackager;
use transmux::segment_log::SegmentSummaryLog;
use transmux::{HlsPackager, SmoothPackager};

/// Every track's spec, in presentation order.
pub fn specs(media: &Media) -> Vec<TrackSpec> {
    media.tracks.iter().map(|t| t.spec.clone()).collect()
}

/// The summaries one multi-track CMAF artifact per track describes.
pub fn log(media: &Media) -> SegmentSummaryLog {
    test_harness::collect::segment_log(media)
}

/// Drive a segmenter stage over `media` **without** announcing its tracks
/// (a segmenter that fixes its track set at construction refuses a
/// [`MuxInput`] track announcement): samples go in decode-time order across
/// tracks, then the stage is finished.
pub fn drive_samples<S>(
    stage: &mut S,
    media: &Media,
) -> Result<Vec<transmux::mux_stage::MuxOutput>, S::Error>
where
    S: for<'a> broadcast_common::Stage<
            In<'a> = transmux::mux_stage::MuxInput,
            Out = transmux::mux_stage::MuxOutput,
        >,
{
    use transmux::mux_stage::MuxInput;
    let mut out = Vec::new();
    let mut cursors = vec![0usize; media.tracks.len()];
    // This merge compares duration-accumulated positions from 0, not each
    // sample's absolute `dts`, so a track whose first sample carries an offset
    // still starts at the head of the merge.
    let mut dts: Vec<u128> = vec![0; media.tracks.len()];
    loop {
        let mut best: Option<usize> = None;
        for (i, track) in media.tracks.iter().enumerate() {
            if cursors[i] >= track.samples.len() {
                continue;
            }
            best = Some(match best {
                None => i,
                Some(b) => {
                    let lhs = dts[i] * u128::from(media.tracks[b].spec.timescale.get());
                    let rhs = dts[b] * u128::from(track.spec.timescale.get());
                    // Ties go to the later track, so a segment-boundary
                    // keyframe on the first video track sees that instant's
                    // audio already buffered.
                    if lhs <= rhs { i } else { b }
                }
            });
        }
        let Some(i) = best else { break };
        let track = &media.tracks[i];
        let sample = track.samples[cursors[i]].clone();
        dts[i] += u128::from(sample.timing.and_then(|t| t.duration()).unwrap_or(0));
        cursors[i] += 1;
        stage.feed(
            MuxInput::Sample {
                track: track.spec.track_id,
                sample,
            },
            broadcast_common::Timestamp::ZERO,
        )?;
        while let Some(item) = stage.poll() {
            out.push(item);
        }
    }
    stage.finish()?;
    while let Some(item) = stage.poll() {
        out.push(item);
    }
    Ok(out)
}

/// Render a DASH MPD for a whole [`Media`]'s one artifact per track.
pub fn dash(packager: &DashPackager, media: &Media) -> transmux::Result<String> {
    dash_with(packager, media, &log(media))
}

/// Render a DASH MPD for a whole [`Media`] from an explicit summary log.
pub fn dash_with(
    packager: &DashPackager,
    media: &Media,
    log: &SegmentSummaryLog,
) -> transmux::Result<String> {
    packager.render(&specs(media), log)
}

/// Render a low-latency DASH MPD for a whole [`Media`]'s one artifact per
/// track.
pub fn ll_dash(packager: &LlDashPackager, media: &Media) -> transmux::Result<String> {
    ll_dash_with(packager, media, &log(media))
}

/// Render a low-latency DASH MPD for a whole [`Media`] from an explicit
/// summary log.
pub fn ll_dash_with(
    packager: &LlDashPackager,
    media: &Media,
    log: &SegmentSummaryLog,
) -> transmux::Result<String> {
    packager.render(&specs(media), log)
}

/// Render a CMAF-HLS media playlist for a whole [`Media`].
pub fn hls(packager: &HlsPackager, media: &Media) -> transmux::Result<String> {
    packager.render(&specs(media), &log(media))
}

/// Render a Smooth client Manifest for a whole [`Media`]'s one fragment per
/// track cut by [`SmoothSegmenter`](transmux::SmoothSegmenter) at the default
/// two-second target.
pub fn smooth(packager: &SmoothPackager, media: &Media) -> transmux::Result<String> {
    let (log, _) = smooth_parts(media, core::time::Duration::from_secs(2))?;
    packager.render(&specs(media), &log)
}

/// The summaries and fragment bytes a [`SmoothSegmenter`](transmux::SmoothSegmenter)
/// produces for a whole [`Media`] at `target`.
pub fn smooth_parts(
    media: &Media,
    target: core::time::Duration,
) -> transmux::Result<(SegmentSummaryLog, Vec<transmux::mux_stage::SegmentOut>)> {
    use test_harness::collect::drive;
    use transmux::mux_stage::MuxOutput;
    let origin = transmux::SmoothOrigin::from_tracks(
        media
            .tracks
            .iter()
            .map(|t| (t.spec.timescale, t.samples.as_slice())),
    );
    let mut segmenter = transmux::SmoothSegmenter::new(specs(media), origin, target)?;
    let outputs = drive(&mut segmenter, media, broadcast_common::Timestamp::ZERO)?;
    let mut log = SegmentSummaryLog::new();
    let mut segments = Vec::new();
    for output in outputs {
        if let MuxOutput::Segment(segment) = output {
            log.push(&segment)?;
            segments.push(*segment);
        }
    }
    Ok((log, segments))
}
