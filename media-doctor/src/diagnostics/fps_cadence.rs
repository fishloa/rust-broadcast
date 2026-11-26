//! `FpsCadenceCheck` — VUI-declared frame rate vs. the measured sample
//! cadence (issue #567).
//!
//! Streams the TS through [`transmux::StreamingTsDemux`] (the same demuxer a
//! TS→MP4 transmux would use), accumulating each track's sample count and
//! total duration as the packets pass, so a multi-gigabyte capture is never
//! held whole. For every AVC/HEVC video track it compares two
//! independently-derived frame rates:
//!
//! - **declared**: the VUI `time_scale`/`num_units_in_tick` frame rate
//!   decoded from the track's own recovered `avcC`/`hvcC` SPS (ITU-T H.264
//!   §E.2.1 / ITU-T H.265 §E.2.1 — decoded via `broadcast_bitstream::sps`, never
//!   re-parsed here).
//! - **measured**: `timescale / mean(sample.duration)`, where each sample's
//!   duration was resolved by the demuxer from the actual PES-timestamp delta
//!   between consecutive access units (ISO/IEC 13818-1 §2.4.3.7) — i.e. the
//!   cadence the stream is actually delivered at.
//!
//! A [`FpsCadenceMismatch`](Severity::Warning) finding fires when the two
//! disagree by more than [`FPS_TOLERANCE_FRACTION`] — wide enough to absorb
//! NTSC 1000/1001 rounding and the quantization noise of a short capture,
//! narrow enough to catch a real cadence fault (e.g. VUI claims 30 fps while
//! the stream is actually delivered at 25).
//!
//! No VUI/SPS parsing happens here — [`broadcast_bitstream::sps::AvcSpsInfo::fps`] /
//! [`broadcast_bitstream::sps::HevcSpsInfo::fps`] (via the recovered config's own SPS)
//! do that; this check only compares the two already-decoded numbers.

use alloc::boxed::Box;
use alloc::collections::btree_map::BTreeMap;
use alloc::format;
use mpeg_ts::pid::Pid;

use transmux::{CodecConfig, DemuxEvent, StreamingTsDemux, TrackId, TrackSpec};

use crate::report::{Finding, Location, Severity};
use crate::{Diagnostic, DiagnosticScan, PmtStreamIndex, Report};
use transmux::nalu_ext::{AvcSpsCodecInfo, HevcNalUnitCodecInfo};

/// HEVC `NAL_unit_type` for a sequence parameter set (ITU-T H.265 §7.3.1.2
/// Table 7-1) — used to pick the SPS array out of `hvcC`'s `numOfArrays`
/// loop (ISO/IEC 14496-15:2017 §8.3.3).
const HEVC_NAL_SPS: u8 = 33;

/// Maximum relative difference between declared VUI fps and measured sample
/// cadence before it is flagged — wide enough to absorb NTSC 1000/1001
/// rounding and short-capture quantization noise.
const FPS_TOLERANCE_FRACTION: f32 = 0.10;

/// Minimum sample count before a measured cadence is trusted (too few
/// samples makes the mean duration noisy — avoids a false positive on a
/// clip with only 1-2 access units on the track).
const MIN_SAMPLES_FOR_CADENCE: usize = 4;

/// Cross-validates VUI-declared frame rate against the measured PES-timestamp
/// sample cadence for every AVC/HEVC video track.
#[derive(Debug, Clone, Copy)]
pub struct FpsCadenceCheck;

impl Diagnostic for FpsCadenceCheck {
    fn scan(&self, _pmt: &PmtStreamIndex) -> Box<dyn DiagnosticScan> {
        Box::new(FpsCadenceScan {
            demux: StreamingTsDemux::new().with_strict_pmt_order(),
            tracks: BTreeMap::new(),
        })
    }
}

/// One track's accumulated sample cadence.
struct TrackCadence {
    spec: TrackSpec,
    count: usize,
    total_duration: u64,
}

/// One streaming pass of [`FpsCadenceCheck`].
struct FpsCadenceScan {
    demux: StreamingTsDemux,
    tracks: BTreeMap<TrackId, TrackCadence>,
}

impl FpsCadenceScan {
    /// Fold one demux event into the per-track cadence.
    fn observe(&mut self, event: &DemuxEvent) {
        match event {
            DemuxEvent::TrackAdded(spec) | DemuxEvent::TrackUpdated(spec) => {
                self.tracks.insert(
                    spec.track_id,
                    TrackCadence {
                        spec: spec.clone(),
                        count: 0,
                        total_duration: 0,
                    },
                );
            }
            DemuxEvent::Sample {
                track_id, sample, ..
            } => {
                if let Some(track) = self.tracks.get_mut(track_id) {
                    track.count += 1;
                    track.total_duration = add_sample_duration(
                        track.total_duration,
                        sample.timing.and_then(|t| t.duration()),
                    );
                }
            }
            _ => {}
        }
    }
}

impl DiagnosticScan for FpsCadenceScan {
    fn feed(&mut self, packet: &[u8], _report: &mut Report) {
        if self.demux.feed(packet).is_err() {
            return;
        }
        while let Some(event) = self.demux.poll_event() {
            self.observe(&event);
        }
    }

    fn finish(&mut self, report: &mut Report) {
        let _ = self.demux.finish();
        while let Some(event) = self.demux.poll_event() {
            self.observe(&event);
        }
        for track in self.tracks.values() {
            check_track(track, report);
        }
    }
}

/// Add one sample's optional duration to a running total, in the track's
/// media timescale.
///
/// `Sample::duration` is `Option<u32>`, so each term widens rather than
/// truncating — `u64::from` rather than `as`, which keeps that a checked
/// claim. The accumulation saturates: a pathologically long track must not
/// wrap the total into a small, plausible-looking value that then reads as a
/// wildly fast cadence rather than as the overflow it is.
///
/// Split out so the arithmetic can be tested without fabricating a codec
/// configuration.
fn add_sample_duration(total: u64, duration: Option<u32>) -> u64 {
    total.saturating_add(u64::from(duration.unwrap_or(0)))
}

fn check_track(track: &TrackCadence, report: &mut Report) {
    let pid = track.spec.source_pid.unwrap_or(Pid::from_const(0));
    let declared_fps = match &track.spec.config {
        CodecConfig::Avc { config, .. } => config.config.sps.first().and_then(|s| {
            let info = s.decode().ok()?;
            // Interlaced coding (`frame_mbs_only_flag == 0`) codes field
            // pictures: the demuxer emits one sample per field, so the
            // measured cadence is ~2x the VUI frame rate by convention,
            // not by fault. That's a units mismatch this check cannot
            // resolve without knowing the demuxer's field/frame sample
            // convention, so it skips interlaced tracks rather than risk
            // a false positive (the `PtsCheck`-lesson hard gate).
            if !info.frame_mbs_only() {
                return None;
            }
            info.fps()
        }),
        CodecConfig::Hevc { config, .. } => config
            .config
            .arrays
            .iter()
            .find(|a| a.nal_unit_type == HEVC_NAL_SPS)
            .and_then(|a| a.nalus.first())
            .and_then(|n| n.decode_sps().ok().flatten()?.fps()),
        _ => None,
    };
    let Some(declared_fps) = declared_fps else {
        return;
    };
    if declared_fps <= 0.0 {
        return;
    }

    if track.count < MIN_SAMPLES_FOR_CADENCE {
        return;
    }
    let total_duration = track.total_duration;
    if total_duration == 0 {
        return;
    }
    let mean_duration = total_duration as f64 / track.count as f64;
    // A `Timescale` is non-zero by construction, so there is no
    // zero-timescale track to skip.
    let timescale = track.spec.timescale.get();
    let measured_fps = f64::from(timescale) / mean_duration;

    let relative_diff = (measured_fps - declared_fps as f64).abs() / declared_fps.max(1.0) as f64;
    if relative_diff > FPS_TOLERANCE_FRACTION as f64 {
        report.push(Finding::new(
            Severity::Warning,
            Location::at_pid(0, pid),
            "fps-cadence-mismatch",
            format!(
                "Track (PID 0x{pid:04X}) VUI-declared frame rate {declared_fps:.3} fps \
                 disagrees with the measured sample cadence {measured_fps:.3} fps \
                 (mean sample duration {mean_duration:.1} ticks @ {timescale} Hz) — \
                 ITU-T H.264 §E.2.1 / ITU-T H.265 §E.2.1 VUI timing vs. \
                 ISO/IEC 13818-1 §2.4.3.7 PES-timestamp cadence"
            ),
        ));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The sample-duration sum must widen from `u32` and saturate rather than
    /// wrap.
    ///
    /// A wrapped total is small and plausible, so it would read as a wildly
    /// fast cadence rather than as the overflow it is.
    /// Fold `durations` through [`add_sample_duration`].
    fn total_duration_of(durations: impl Iterator<Item = Option<u32>>) -> u64 {
        durations.fold(0u64, add_sample_duration)
    }

    #[test]
    fn sample_duration_sum_widens_and_saturates() {
        // Widening: `u32::MAX` per sample must survive several samples,
        // which a `u32` accumulator would not.
        assert_eq!(
            total_duration_of([Some(u32::MAX); 4].into_iter()),
            u64::from(u32::MAX) * 4,
        );
        // `None` contributes nothing.
        assert_eq!(
            total_duration_of([None, Some(1_000), None, Some(2_000)].into_iter()),
            3_000,
        );
        // Saturation: the total never decreases and never wraps below a term.
        let terms = [
            Some(u32::MAX),
            Some(u32::MAX),
            Some(u32::MAX),
            Some(u32::MAX),
            Some(u32::MAX),
        ];
        let total = total_duration_of(terms.into_iter());
        assert!(
            total >= u64::from(u32::MAX),
            "the total must never wrap below a single term; got {total}",
        );
        assert_eq!(total, u64::from(u32::MAX) * 5);
    }

    /// No video tracks (or no VUI timing in the SPS) → no findings, never a
    /// panic — regression guard for the `None`/empty-track paths.
    #[test]
    fn empty_stream_no_findings() {
        let mut report = Report::new();
        FpsCadenceCheck.run(&[], &mut report);
        assert!(report.is_empty());
    }
}
