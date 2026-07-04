//! `FpsCadenceCheck` — VUI-declared frame rate vs. the measured sample
//! cadence (issue #567).
//!
//! Runs the TS through [`transmux::TsDemux`] (the same demuxer a TS→MP4
//! transmux would use) to get a [`transmux::Media`]. For every AVC/HEVC
//! video track it compares two independently-derived frame rates:
//!
//! - **declared**: the VUI `time_scale`/`num_units_in_tick` frame rate
//!   decoded from the track's own recovered `avcC`/`hvcC` SPS (ITU-T H.264
//!   §E.2.1 / ITU-T H.265 §E.2.1 — decoded via [`transmux::sps`], never
//!   re-parsed here).
//! - **measured**: `timescale / mean(sample.duration)`, where each sample's
//!   duration was resolved by `TsDemux` from the actual PES-timestamp delta
//!   between consecutive access units (ISO/IEC 13818-1 §2.4.3.7) — i.e. the
//!   cadence the stream is actually delivered at.
//!
//! A [`FpsCadenceMismatch`](Severity::Warning) finding fires when the two
//! disagree by more than [`FPS_TOLERANCE_FRACTION`] — wide enough to absorb
//! NTSC 1000/1001 rounding and the quantization noise of a short capture,
//! narrow enough to catch a real cadence fault (e.g. VUI claims 30 fps while
//! the stream is actually delivered at 25).
//!
//! No VUI/SPS parsing happens here — [`transmux::sps::AvcSpsInfo::fps`] /
//! [`transmux::sps::HevcSpsInfo::fps`] (via the recovered config's own SPS)
//! do that; this check only compares the two already-decoded numbers.

use alloc::format;

use broadcast_common::Unpackage;
use transmux::{CodecConfig, Media, TsDemux};

use crate::Diagnostic;
use crate::Report;
use crate::report::{Finding, Location, Severity};

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
    fn run(&self, ts: &[u8], report: &mut Report) {
        let Ok(media) = TsDemux::new().unpackage(ts) else {
            return;
        };
        check_media(&media, report);
    }
}

fn check_media(media: &Media, report: &mut Report) {
    for track in &media.tracks {
        let pid = track.spec.source_pid.unwrap_or(0);
        let declared_fps = match &track.spec.config {
            CodecConfig::Avc { config, .. } => config.config.sps.first().and_then(|s| {
                let info = s.decode().ok()?;
                // Interlaced coding (`frame_mbs_only_flag == 0`) codes field
                // pictures: `TsDemux` emits one sample per field, so the
                // measured cadence is ~2x the VUI frame rate by convention,
                // not by fault. That's a units mismatch this check cannot
                // resolve without knowing the demuxer's field/frame sample
                // convention, so it skips interlaced tracks rather than risk
                // a false positive (the `PtsCheck`-lesson hard gate).
                if !info.frame_mbs_only {
                    return None;
                }
                info.fps
            }),
            CodecConfig::Hevc { config, .. } => config
                .config
                .arrays
                .iter()
                .find(|a| a.nal_unit_type == HEVC_NAL_SPS)
                .and_then(|a| a.nalus.first())
                .and_then(|n| n.decode_sps().ok().flatten()?.fps),
            _ => None,
        };
        let Some(declared_fps) = declared_fps else {
            continue;
        };
        if declared_fps <= 0.0 {
            continue;
        }

        if track.samples.len() < MIN_SAMPLES_FOR_CADENCE {
            continue;
        }
        let total_duration: u64 = track.samples.iter().map(|s| s.duration as u64).sum();
        if total_duration == 0 {
            continue;
        }
        let mean_duration = total_duration as f64 / track.samples.len() as f64;
        let timescale = track.timescale();
        if timescale == 0 {
            continue;
        }
        let measured_fps = timescale as f64 / mean_duration;

        let relative_diff =
            (measured_fps - declared_fps as f64).abs() / declared_fps.max(1.0) as f64;
        if relative_diff > FPS_TOLERANCE_FRACTION as f64 {
            report.push(Finding::new(
                Severity::Warning,
                Location::new(0, pid),
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
}

#[cfg(test)]
mod tests {
    use super::*;

    /// No video tracks (or no VUI timing in the SPS) → no findings, never a
    /// panic — regression guard for the `None`/empty-track paths.
    #[test]
    fn empty_media_no_findings() {
        let media = Media::new(alloc::vec::Vec::new(), 90_000);
        let mut report = Report::new();
        check_media(&media, &mut report);
        assert!(report.is_empty());
    }
}
