//! Timeline-conditioning transforms over the [`Media`] IR — PTS/DTS rebase,
//! offset, and discontinuity-gap insertion (issue #476).
//!
//! Media plane step 2c: [`Sample`](crate::pipeline::Sample) `dts`/`pts` are
//! now **absolute** ticks in the track's media timescale, not a running sum
//! anchored on [`Track::start_decode_time`]. These transforms shift that
//! absolute pair directly, keeping [`Track::start_decode_time`] in lockstep
//! as the track-level anchor (equal to the first sample's `dts` for a timed
//! track; the only datum available at all for an empty track):
//!
//! - [`rebase_to_zero`] — re-origin each track so its first sample starts at
//!   decode time 0 (per-track).
//! - [`apply_offset`] — shift every track's anchor (and every sample's
//!   `dts`/`pts`) by a signed delta (saturating at 0 on underflow).
//! - [`insert_discontinuity_gap`] — push every sample from a given index
//!   onward (or the whole track, at index 0) later by a gap (splice / concat
//!   conditioning).
//!
//! **33-bit MPEG-2 Systems wrap-unrolling moved to the demux edge** (this
//! step's own mandate): [`crate::ts_demux`] unwraps the wire's 33-bit
//! PTS/DTS once, incrementally, as it demuxes, so every `Sample::dts`/`pts`
//! this crate produces is already absolute — there is nothing left to
//! re-derive downstream, and re-deriving it here (as this module did before
//! step 2c, folding `start_decode_time + Σ duration` back into the 33-bit
//! range and unwrapping again) is exactly the anti-pattern this step
//! removes. The public `unroll_33bit_wraps`/`MPEG_TS_WRAP` this module used
//! to export are gone with it.
//!
//! This pairs with issue #475 (splice/concat) as the next consumer: rebase a
//! source to zero, offset it onto the target timeline, then concatenate.

use crate::media::{Media, Track};

/// Re-origin each track so its first sample starts at decode time 0.
///
/// For every track, subtracts its own [`Track::start_decode_time`] from the
/// anchor and from every sample's `dts`/`pts` (leaving a `None` sample
/// untouched — never fabricating a timestamp for a section-carried track).
/// Done **per track** (each track is rebased to its own zero), not by a
/// single common minimum — so tracks that already shared a common origin
/// stay aligned, and tracks with independent origins are each pulled to 0.
///
/// Idempotent: a second call is a no-op (every anchor is already 0).
pub fn rebase_to_zero(media: &mut Media) {
    for track in &mut media.tracks {
        let anchor = track.start_decode_time as i64;
        if anchor == 0 {
            continue;
        }
        for s in &mut track.samples {
            if let Some(d) = s.dts {
                s.dts = Some(d - anchor);
            }
            if let Some(p) = s.pts {
                s.pts = Some(p - anchor);
            }
        }
        track.start_decode_time = 0;
    }
}

/// Shift every track's decode-time anchor — and every sample's `dts`/`pts` —
/// by a signed `delta_ticks`.
///
/// `delta_ticks` is interpreted in each track's own media-timescale ticks (the
/// same units as [`Track::start_decode_time`]). A positive delta moves the
/// timeline later; a negative delta moves it earlier, **saturating at 0** — a
/// track whose anchor would go negative is clamped to 0 (the earliest
/// representable decode time) rather than wrapping. The *same* effective
/// delta (which may be smaller in magnitude than `delta_ticks` when the
/// anchor saturates) is applied to every sample's `dts`/`pts`, so the anchor
/// and the sample timeline never disagree.
pub fn apply_offset(media: &mut Media, delta_ticks: i64) {
    for track in &mut media.tracks {
        let new_anchor = track.start_decode_time.saturating_add_signed(delta_ticks);
        let effective_delta = new_anchor as i64 - track.start_decode_time as i64;
        for s in &mut track.samples {
            if let Some(d) = s.dts {
                s.dts = Some(d + effective_delta);
            }
            if let Some(p) = s.pts {
                s.pts = Some(p + effective_delta);
            }
        }
        track.start_decode_time = new_anchor;
    }
}

/// Insert a discontinuity gap of `gap_ticks` into a track's decode timeline
/// immediately before the sample at `at_sample_index`.
///
/// Every sample from `at_sample_index` onward has its `dts`/`pts` (when
/// `Some`) pushed later by `gap_ticks` — so the sample at `at_sample_index`,
/// and every sample after it, starts `gap_ticks` later, and the total
/// timeline span grows by exactly `gap_ticks`. Samples before the insertion
/// point (and their durations) are unchanged.
///
/// At `at_sample_index == 0` (or an empty track) there is no preceding
/// sample: every sample (the whole track) shifts later, and the anchor
/// ([`Track::start_decode_time`]) is bumped by `gap_ticks` (saturating) to
/// stay in lockstep. An `at_sample_index` past the end of the track is
/// clamped to `track.samples.len()` — a no-op on the samples (there is
/// nothing after the end to shift), matching "gap at the very end" having no
/// observable effect on this track's own samples.
pub fn insert_discontinuity_gap(track: &mut Track, at_sample_index: usize, gap_ticks: u32) {
    let gap = gap_ticks as i64;
    if at_sample_index == 0 {
        track.start_decode_time = track.start_decode_time.saturating_add(gap_ticks as u64);
    }
    let idx = at_sample_index.min(track.samples.len());
    for s in &mut track.samples[idx..] {
        if let Some(d) = s.dts {
            s.dts = Some(d + gap);
        }
        if let Some(p) = s.pts {
            s.pts = Some(p + gap);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ir::SampleFlags;
    use crate::media::Track;
    use crate::pipeline::{Sample, TrackSpec};
    use alloc::vec;

    fn sample(dts: i64, duration: u32) -> Sample {
        Sample {
            data: vec![0u8; 4].into(),
            dts: Some(dts),
            pts: Some(dts),
            duration: Some(duration),
            flags: SampleFlags::SYNC,
            provenance: None,
        }
    }

    /// Build a track whose samples carry consecutive absolute `dts`/`pts`
    /// starting at `start`, stepping by each duration in `durs`.
    fn track_at(start: i64, durs: &[u32]) -> Track {
        let mut dts = start;
        let mut samples = alloc::vec::Vec::with_capacity(durs.len());
        for &d in durs {
            samples.push(sample(dts, d));
            dts += d as i64;
        }
        Track::new_at(spec(), samples, start as u64)
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
        TrackSpec::new(
            1,
            90_000,
            CodecConfig::Avc {
                config: AVCConfigurationBox::new(record),
                width: 16,
                height: 16,
            },
        )
    }

    #[test]
    fn rebase_to_zero_clears_anchor_and_samples() {
        let mut m = Media::new(vec![track_at(90_000, &[3000, 3000])], 1000);
        rebase_to_zero(&mut m);
        assert_eq!(m.tracks[0].start_decode_time, 0);
        assert_eq!(m.tracks[0].samples[0].dts, Some(0));
        assert_eq!(m.tracks[0].samples[1].dts, Some(3000));
        assert_eq!(m.tracks[0].samples[1].pts, Some(3000));
        // Idempotent.
        rebase_to_zero(&mut m);
        assert_eq!(m.tracks[0].start_decode_time, 0);
        assert_eq!(m.tracks[0].samples[0].dts, Some(0));
    }

    #[test]
    fn rebase_to_zero_leaves_none_untouched() {
        let mut m = Media::new(vec![track_at(1000, &[10])], 1000);
        m.tracks[0].samples[0].dts = None;
        m.tracks[0].samples[0].pts = None;
        rebase_to_zero(&mut m);
        assert_eq!(m.tracks[0].samples[0].dts, None, "never fabricate a dts");
        assert_eq!(m.tracks[0].samples[0].pts, None, "never fabricate a pts");
    }

    #[test]
    fn apply_offset_shifts_anchor_and_samples() {
        let mut m = Media::new(vec![track_at(100, &[10])], 1000);
        apply_offset(&mut m, 50);
        assert_eq!(m.tracks[0].start_decode_time, 150);
        assert_eq!(m.tracks[0].samples[0].dts, Some(150));
        apply_offset(&mut m, -1000);
        assert_eq!(m.tracks[0].start_decode_time, 0, "underflow saturates at 0");
        assert_eq!(
            m.tracks[0].samples[0].dts,
            Some(0),
            "sample shift matches the saturated (clamped) anchor delta, not the raw delta"
        );
    }

    #[test]
    fn gap_extends_timeline_and_shifts_later_samples() {
        let mut t = track_at(0, &[100, 100, 100]);
        insert_discontinuity_gap(&mut t, 2, 500);
        assert_eq!(t.samples[0].dts, Some(0), "earlier sample unchanged");
        assert_eq!(t.samples[1].dts, Some(100), "earlier sample unchanged");
        assert_eq!(
            t.samples[2].dts,
            Some(700),
            "sample at the insertion point pushed out by the gap (200 + 500)"
        );
        assert_eq!(t.start_decode_time, 0, "anchor unaffected mid-track");
    }

    #[test]
    fn gap_at_index_zero_shifts_anchor_and_every_sample() {
        let mut t = track_at(1000, &[100]);
        insert_discontinuity_gap(&mut t, 0, 250);
        assert_eq!(t.start_decode_time, 1250);
        assert_eq!(t.samples[0].dts, Some(1250));
    }
}
