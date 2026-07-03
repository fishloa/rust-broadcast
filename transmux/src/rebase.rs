//! Timeline-conditioning transforms over the [`Media`] IR — PTS/DTS rebase,
//! offset, 33-bit MPEG wrap-unroll, and discontinuity-gap insertion (issue #476).
//!
//! [`Sample`](crate::pipeline::Sample) timing in the IR is *relative*: each
//! sample carries a `duration`, a sample's DTS is the running sum of the
//! preceding durations, and its PTS is `DTS + composition_offset`. The only
//! *absolute* datum is [`Track::start_decode_time`] — the decode time of the
//! track's first sample, in that track's media timescale, which maps onto the
//! fragment `tfdt` `baseMediaDecodeTime` (ISO/IEC 14496-12:2015 §8.8.12) written
//! by [`CmafMux`](crate::media::CmafMux).
//!
//! These transforms all operate on that anchor (and, for wrap-unroll and gap
//! insertion, on per-sample durations), leaving the coded sample bytes
//! untouched:
//!
//! - [`rebase_to_zero`] — re-origin each track so its first sample starts at
//!   decode time 0 (per-track).
//! - [`apply_offset`] — shift every track's anchor by a signed delta (saturating
//!   at 0 on underflow).
//! - [`unroll_33bit_wraps`] — undo MPEG-2 Systems 33-bit timestamp wraps
//!   (ISO/IEC 13818-1 §2.4.3.6: PTS/DTS are 33-bit values that wrap modulo
//!   2^33 ≈ 26.5 h at 90 kHz) so the timeline is monotonic.
//! - [`insert_discontinuity_gap`] — extend the timeline by a gap at a given
//!   sample index (splice / concat conditioning).
//!
//! This pairs with issue #475 (splice/concat) as the next consumer: rebase a
//! source to zero, offset it onto the target timeline, unroll any wrap, then
//! concatenate.

use crate::media::{Media, Track};

/// The MPEG-2 Systems timestamp modulus: PTS/DTS are 33-bit fields, so they
/// wrap every `2^33` ticks (ISO/IEC 13818-1 §2.4.3.6). At the 90 kHz system
/// clock that is ≈ 26.5 hours.
pub const MPEG_TS_WRAP: u64 = 1 << 33;

/// Re-origin each track so its first sample starts at decode time 0.
///
/// For every track, subtracts its own [`Track::start_decode_time`] from the
/// anchor, leaving it at 0. This is done **per track** (each track is rebased to
/// its own zero), not by a single common minimum — so tracks that already shared
/// a common origin stay aligned, and tracks with independent origins are each
/// pulled to 0. Per-sample durations are relative and so are unchanged.
///
/// Idempotent: a second call is a no-op (every anchor is already 0).
pub fn rebase_to_zero(media: &mut Media) {
    for track in &mut media.tracks {
        track.start_decode_time = 0;
    }
}

/// Shift every track's decode-time anchor by a signed `delta_ticks`.
///
/// `delta_ticks` is interpreted in each track's own media-timescale ticks (the
/// same units as [`Track::start_decode_time`]). A positive delta moves the
/// timeline later; a negative delta moves it earlier, **saturating at 0** — a
/// track whose anchor would go negative is clamped to 0 (the earliest
/// representable decode time) rather than wrapping.
pub fn apply_offset(media: &mut Media, delta_ticks: i64) {
    for track in &mut media.tracks {
        track.start_decode_time = track.start_decode_time.saturating_add_signed(delta_ticks);
    }
}

/// Half the MPEG modulus — the classic wrap-detection threshold: a jump of more
/// than half the range is read as a wrap across the boundary, not a real jump.
const MPEG_TS_WRAP_HALF: u64 = MPEG_TS_WRAP / 2;

/// Undo MPEG-2 Systems 33-bit timestamp wraps so each track's decode timeline is
/// monotonic non-decreasing across the `2^33` boundary.
///
/// MPEG PTS/DTS are 33-bit values that wrap modulo [`MPEG_TS_WRAP`] (`2^33`,
/// ISO/IEC 13818-1 §2.4.3.6 — ≈ 26.5 h at 90 kHz). In the IR the timeline is
/// reconstructed as `start_decode_time + Σ durations`; a source captured across
/// a wrap folds that timeline back into the low 33 bits, so a step that should
/// have advanced past `2^33` instead lands near 0 — a large *backward* jump.
///
/// This transform, per track, reconstructs each sample's **folded** decode time
/// (`(start_decode_time + cumulative_duration) mod 2^33`, mirroring how the
/// value appears on the wire) and unwraps the sequence: whenever the folded
/// value jumps backward by more than half of [`MPEG_TS_WRAP`] it adds `2^33` to
/// stay monotonic (the same rule the TS demuxer applies inter-sample). It then
/// writes the unwrapped first value back to [`Track::start_decode_time`] and the
/// unwrapped inter-sample deltas back to each [`Sample::duration`](crate::pipeline::Sample::duration).
///
/// For an already-monotonic timeline (no wrap) this is an exact identity. Sample
/// bytes are never changed.
pub fn unroll_33bit_wraps(media: &mut Media) {
    for track in &mut media.tracks {
        if track.samples.is_empty() {
            continue;
        }
        // 1. Fold the reconstructed timeline into the 33-bit wire range.
        let mut folded: alloc::vec::Vec<u64> = alloc::vec::Vec::with_capacity(track.samples.len());
        let mut acc = track.start_decode_time % MPEG_TS_WRAP;
        folded.push(acc);
        for s in track.samples.iter().take(track.samples.len() - 1) {
            acc = (acc + s.duration as u64) % MPEG_TS_WRAP;
            folded.push(acc);
        }
        // 2. Unwrap the folded sequence to a monotonic timeline.
        let mut unwrapped: alloc::vec::Vec<u64> = alloc::vec::Vec::with_capacity(folded.len());
        unwrapped.push(folded[0]);
        for i in 1..folded.len() {
            let prev = unwrapped[i - 1];
            let cur_folded = folded[i];
            // How many whole wraps `prev` already carries above the wire range.
            let base = (prev / MPEG_TS_WRAP) * MPEG_TS_WRAP;
            let mut cur = base + cur_folded;
            // If placing `cur` in the same wrap window steps backward by more
            // than half the range, it wrapped forward: advance one window.
            if cur + MPEG_TS_WRAP_HALF < prev {
                cur += MPEG_TS_WRAP;
            }
            unwrapped.push(cur);
        }
        // 3. Write the unwrapped anchor + deltas back into the IR.
        track.start_decode_time = unwrapped[0];
        for i in 0..track.samples.len() {
            let delta = if i + 1 < unwrapped.len() {
                unwrapped[i + 1] - unwrapped[i]
            } else {
                // Final sample keeps its original duration (no successor to
                // remeasure against).
                track.samples[i].duration as u64
            };
            track.samples[i].duration = delta as u32;
        }
    }
}

/// Insert a discontinuity gap of `gap_ticks` into a track's decode timeline
/// immediately before the sample at `at_sample_index`.
///
/// The gap is realised by extending the duration of the sample **preceding** the
/// insertion point (index `at_sample_index - 1`) by `gap_ticks`, so the sample
/// at `at_sample_index` — and every sample after it — starts `gap_ticks` later.
/// This keeps the relative-duration IR self-consistent: the total timeline span
/// grows by exactly `gap_ticks` and no coded bytes move.
///
/// Behaviour at the edges:
/// - `at_sample_index == 0`: there is no preceding sample to extend, so the gap
///   is applied to the **anchor** ([`Track::start_decode_time`] is bumped by
///   `gap_ticks`, saturating), shifting the whole track later.
/// - `at_sample_index >= track.samples.len()`: the gap extends the final
///   sample's duration (a trailing gap), so the timeline span still grows by
///   `gap_ticks`.
///
/// Samples before the insertion point are otherwise unchanged.
pub fn insert_discontinuity_gap(track: &mut Track, at_sample_index: usize, gap_ticks: u32) {
    if track.samples.is_empty() {
        // No samples: the only timeline datum is the anchor.
        track.start_decode_time = track.start_decode_time.saturating_add(gap_ticks as u64);
        return;
    }
    if at_sample_index == 0 {
        // Gap before the first sample = shift the whole track later.
        track.start_decode_time = track.start_decode_time.saturating_add(gap_ticks as u64);
        return;
    }
    // Extend the sample just before the insertion point (clamp a past-the-end
    // index to the final sample, making it a trailing gap).
    let idx = at_sample_index.min(track.samples.len()) - 1;
    let dur = &mut track.samples[idx].duration;
    *dur = dur.saturating_add(gap_ticks);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::media::Track;
    use crate::pipeline::{Sample, TrackSpec};
    use alloc::vec;
    use alloc::vec::Vec;

    fn sample(duration: u32) -> Sample {
        Sample {
            data: vec![0u8; 4],
            duration,
            is_sync: true,
            composition_offset: 0,
            source_timing: None,
        }
    }

    fn track_at(start: u64, durs: &[u32]) -> Track {
        let samples = durs.iter().map(|&d| sample(d)).collect();
        Track::new_at(spec(), samples, start)
    }

    // Codec config is irrelevant to the transforms; build a minimal AVC spec.
    fn spec() -> TrackSpec {
        use crate::avc_config::{AVCConfigurationBox, AVCDecoderConfigurationRecord};
        use crate::nalu_types::{AvcPps, AvcSps};
        use crate::pipeline::CodecConfig;
        let record = AVCDecoderConfigurationRecord {
            configuration_version: 1,
            profile_indication: 66,
            profile_compatibility: 0,
            level_indication: 30,
            length_size_minus_one: 3,
            sps: vec![AvcSps(vec![0x67, 0x42, 0x00, 0x1e])],
            pps: vec![AvcPps(vec![0x68, 0xce, 0x3c, 0x80])],
            chroma_format: None,
            bit_depth_luma_minus8: None,
            bit_depth_chroma_minus8: None,
            sps_ext: vec![],
        };
        TrackSpec {
            track_id: 1,
            timescale: 90_000,
            config: CodecConfig::Avc {
                config: AVCConfigurationBox::new(record),
                width: 16,
                height: 16,
            },
        }
    }

    #[test]
    fn rebase_to_zero_clears_anchor() {
        let mut m = Media::new(vec![track_at(90_000, &[3000, 3000])], 1000);
        rebase_to_zero(&mut m);
        assert_eq!(m.tracks[0].start_decode_time, 0);
        // Idempotent.
        rebase_to_zero(&mut m);
        assert_eq!(m.tracks[0].start_decode_time, 0);
    }

    #[test]
    fn apply_offset_saturates() {
        let mut m = Media::new(vec![track_at(100, &[10])], 1000);
        apply_offset(&mut m, 50);
        assert_eq!(m.tracks[0].start_decode_time, 150);
        apply_offset(&mut m, -1000);
        assert_eq!(m.tracks[0].start_decode_time, 0, "underflow saturates at 0");
    }

    #[test]
    fn unroll_identity_on_monotonic() {
        // No wrap → exact identity in anchor and durations.
        let mut m = Media::new(vec![track_at(1_000_000, &[3000, 3000, 3000])], 1000);
        unroll_33bit_wraps(&mut m);
        assert_eq!(m.tracks[0].start_decode_time, 1_000_000);
        assert_eq!(
            m.tracks[0]
                .samples
                .iter()
                .map(|s| s.duration)
                .collect::<Vec<_>>(),
            vec![3000, 3000, 3000]
        );
    }

    #[test]
    fn unroll_lifts_timeline_across_boundary() {
        // Anchor just below 2^33; three 3000-tick samples cross the boundary.
        // Folded wire DTS: [2^33-3000, 2^33-0→0, 3000]. Unwrapped must be
        // monotonic: [2^33-3000, 2^33, 2^33+3000].
        let start = MPEG_TS_WRAP - 3000;
        let mut m = Media::new(vec![track_at(start, &[3000, 3000, 3000])], 1000);
        unroll_33bit_wraps(&mut m);
        let t = &m.tracks[0];
        assert_eq!(t.start_decode_time, MPEG_TS_WRAP - 3000);
        // Reconstruct DTS and assert monotonic + exact.
        let mut dts = t.start_decode_time;
        let expected = [MPEG_TS_WRAP - 3000, MPEG_TS_WRAP, MPEG_TS_WRAP + 3000];
        for (i, s) in t.samples.iter().enumerate() {
            assert_eq!(dts, expected[i], "sample {i} DTS");
            dts += s.duration as u64;
        }
    }

    #[test]
    fn gap_extends_preceding_sample() {
        let mut t = track_at(0, &[100, 100, 100]);
        let span_before: u32 = t.samples.iter().map(|s| s.duration).sum();
        insert_discontinuity_gap(&mut t, 2, 500);
        let span_after: u32 = t.samples.iter().map(|s| s.duration).sum();
        assert_eq!(span_after - span_before, 500);
        assert_eq!(t.samples[1].duration, 600, "preceding sample extended");
        assert_eq!(t.samples[0].duration, 100, "earlier sample unchanged");
    }

    #[test]
    fn gap_at_index_zero_shifts_anchor() {
        let mut t = track_at(1000, &[100]);
        insert_discontinuity_gap(&mut t, 0, 250);
        assert_eq!(t.start_decode_time, 1250);
        assert_eq!(t.samples[0].duration, 100);
    }
}
