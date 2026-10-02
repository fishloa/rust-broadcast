//! Loudness meter implementing ITU-R BS.1770‑5 and EBU R 128 / Tech 3341.
//!
//! Provides momentary (400 ms), short‑term (3 s), and integrated (gated)
//! loudness in LUFS, plus Loudness Range (LRA) per EBU Tech 3342.
//!
//! ## Streaming, bounded memory (issue #1072 / audit LOUD-C2)
//!
//! Earlier versions stored one `f64` of weighted power per **sample
//! frame** for the whole measurement (~1.4 GB/hour at 48 kHz), re-scanned
//! in full on `finish()`. This meter now keeps two independent, bounded
//! pieces of state instead, chosen per what each result actually needs:
//!
//! - **Integrated loudness + LRA** — samples accumulate into 100 ms energy
//!   sub-blocks (a fixed handful of running totals, O(1) per sample); a
//!   small ring of the last 30 completed sub-blocks (bounded, never grows)
//!   derives one 400 ms gating-block LKFS value and, once at least 3 s have
//!   elapsed, one 3 s short-term LKFS value every 100 ms step. Only those
//!   derived values are retained (`gating_blocks`/`short_term_blocks`) —
//!   about 36 000 `f64`s/hour, not 172 million, and independent of sample
//!   rate. This 100 ms grid is exactly what BS.1770-5's own 75%-overlap
//!   gating blocks, and Tech 3342's "≥10 Hz" short-term sampling, specify.
//! - **Max momentary / max short-term** — Tech 3341 §Table 1 cases 9-14
//!   test the exact, continuously-slid maximum (not merely sampled every
//!   100 ms), so these use [`SlidingWindowMax`]: an exact circular-buffer
//!   sliding window sized to its own fixed window length (0.4 s / 3 s
//!   worth of samples — a constant multiple of the sample rate, never of
//!   measurement duration), updated in O(1) amortized per sample.
//!
//! ## LRA from short-term values, not momentary blocks (issue #1051 / audit
//! LOUD-C1)
//!
//! EBU Tech 3342 defines Loudness Range over the distribution of
//! **short-term (3 s window)** loudness values, sampled at ≥10 Hz — NOT the
//! 400 ms gating blocks BS.1770-5's own integrated-loudness gating uses.
//! [`LoudnessMeter::loudness_range`] is computed from the 3 s
//! `short_term_blocks` history above; [`LoudnessMeter::integrated_lufs`]
//! uses the 400 ms `gating_blocks` history, which is the correct input for
//! integrated loudness (unaffected by this fix).

use alloc::collections::VecDeque;
use alloc::vec::Vec;

use crate::channel_layout::ChannelLayout;
use crate::filter::{BiquadCoeffs, BiquadState, apply_biquad, k_weighting_coeffs};

/// —70 LUFS absolute gating threshold (ITU‑R BS.1770‑5 §Annex 1, eq. 6).
const ABSOLUTE_GATE: f64 = -70.0;

/// —10 LU relative gating threshold (ITU‑R BS.1770‑5 §Annex 1, eq. 6).
const RELATIVE_GATE: f64 = -10.0;

/// Sub-block / step duration in seconds: the 100 ms common denominator of
/// the 400 ms momentary window (4 sub-blocks) and the 3 s short-term
/// window (30 sub-blocks), and BS.1770-5's own 75%-overlap 400 ms gating
/// block step (issue #1072).
const SUB_BLOCK_S: f64 = 0.1;

/// Momentary / BS.1770-5 gating-block window, in sub-blocks (400 ms).
const MOMENTARY_SUB_BLOCKS: usize = 4;

/// Short-term window, in sub-blocks (3 s) — EBU Tech 3341 §2.2.2 / the
/// input EBU Tech 3342 LRA is defined over (issue #1051).
const SHORT_TERM_SUB_BLOCKS: usize = 30;

/// The constant —0.691 in BS.1770‑5 eq. (2), cancelling the K‑weighting
/// gain for a 997 Hz tone.
const LOUDNESS_OFFSET: f64 = -0.691;

/// BS.1770‑5 Annex 1 eq. (2): convert mean‑square to LKFS.
///
/// `mean_sq` is the K‑weighted, channel‑weighted mean square.
#[inline]
fn mean_sq_to_lkfs(mean_sq: f64) -> f64 {
    if mean_sq <= 0.0 {
        f64::NEG_INFINITY
    } else {
        LOUDNESS_OFFSET + 10.0 * libm::log10(mean_sq)
    }
}

/// BS.1770‑5 Annex 1 inverse: LKFS → mean square.
#[inline]
fn lkfs_to_mean_sq(lkfs: f64) -> f64 {
    libm::pow(10.0, (lkfs - LOUDNESS_OFFSET) / 10.0)
}

/// Per‑channel K‑weighting filter state.
#[derive(Debug, Clone)]
struct ChannelFilter {
    stage1: BiquadState,
    stage2: BiquadState,
    coeffs: BiquadCoeffsPair,
}

/// The two K‑weighting biquad coefficient sets (shelving + high‑pass),
/// shared by all channels.
#[derive(Debug, Clone, Copy)]
struct BiquadCoeffsPair {
    stage1: BiquadCoeffs,
    stage2: BiquadCoeffs,
}

impl ChannelFilter {
    fn new(stage1: BiquadCoeffs, stage2: BiquadCoeffs) -> Self {
        Self {
            stage1: BiquadState::new(),
            stage2: BiquadState::new(),
            coeffs: BiquadCoeffsPair { stage1, stage2 },
        }
    }

    /// Apply the cascaded K‑weighting to one sample.
    #[inline]
    fn process(&mut self, sample: f64) -> f64 {
        let y1 = apply_biquad(sample, &self.coeffs.stage1, &mut self.stage1);
        apply_biquad(y1, &self.coeffs.stage2, &mut self.stage2)
    }
}

/// Exact sliding-window maximum tracker: an `O(window_samples)` circular
/// buffer of raw per-frame weighted power plus a running sum, giving the
/// exact per-SAMPLE-aligned sliding-window mean (matching what a full
/// re-scan of the raw samples would find) at `O(1)` amortized cost per
/// pushed sample.
///
/// This is deliberately kept separate from the 100 ms sub-block history
/// used for integrated loudness/LRA (BS.1770-5's own gating blocks are
/// defined on a fixed 100 ms grid, so grid alignment is correct there) —
/// `Max M`/`Max S` (EBU Tech 3341 cases 9-14) are specified and tested
/// against the true continuous sliding maximum, which a 100 ms-grid
/// approximation under-reports whenever the loudest window isn't aligned
/// to that grid. Memory is bounded by `window_samples` (a fixed multiple
/// of the sample rate), never by measurement duration (issue #1072).
#[derive(Debug, Clone)]
struct SlidingWindowMax {
    /// Circular buffer, length `window_samples`.
    ring: Vec<f64>,
    window_samples: usize,
    write_pos: usize,
    filled: usize,
    running_sum: f64,
    max_lkfs: f64,
}

impl SlidingWindowMax {
    fn new(window_samples: usize) -> Self {
        let window_samples = window_samples.max(1);
        Self {
            ring: alloc::vec![0.0; window_samples],
            window_samples,
            write_pos: 0,
            filled: 0,
            running_sum: 0.0,
            max_lkfs: f64::NEG_INFINITY,
        }
    }

    fn reset(&mut self) {
        for v in &mut self.ring {
            *v = 0.0;
        }
        self.write_pos = 0;
        self.filled = 0;
        self.running_sum = 0.0;
        self.max_lkfs = f64::NEG_INFINITY;
    }

    /// Fold one new sample's weighted power in, evicting the oldest once
    /// the ring is full, and update the running max if this window is now
    /// (still) fully populated.
    fn push(&mut self, weighted_power: f64) {
        let outgoing = self.ring[self.write_pos];
        self.ring[self.write_pos] = weighted_power;
        self.write_pos = (self.write_pos + 1) % self.window_samples;

        if self.filled < self.window_samples {
            self.running_sum += weighted_power;
            self.filled += 1;
            if self.filled == self.window_samples {
                self.update_max();
            }
        } else {
            self.running_sum += weighted_power - outgoing;
            self.update_max();
        }
    }

    fn update_max(&mut self) {
        let lkfs = mean_sq_to_lkfs(self.running_sum / self.window_samples as f64);
        if lkfs > self.max_lkfs {
            self.max_lkfs = lkfs;
        }
    }
}

/// A pre‑computed block loudness value — either a 400 ms gating block (for
/// the integrated measurement) or a 3 s short-term value (for LRA); the two
/// histories are the same shape, kept as separate `Vec`s (see
/// [`LoudnessMeter::gating_blocks`] / `short_term_blocks`).
#[derive(Debug, Clone, Copy)]
struct LkfsSample {
    /// Loudness in LKFS of this block/window.
    lkfs: f64,
}

/// EBU R 128 / ITU‑R BS.1770‑5 loudness meter.
///
/// Feed planar or interleaved PCM samples, then query momentary, short‑term,
/// integrated loudness (LUFS), loudness range (LU), and per‑channel true‑peak.
///
/// ## Measurement flow
///
/// ```text
/// input samples → K‑weighting → channel weighting → mean square
///   → 100 ms sub-blocks → a bounded 30-entry ring
///   → gating blocks (400 ms = 4 sub-blocks, 75% overlap) → integrated, max momentary
///   → short-term values (3 s = 30 sub-blocks) → max short-term, LRA
/// ```
///
/// See the module docs for why this is streaming/bounded rather than
/// buffering every sample (issue #1072) and why LRA is computed from the
/// short-term history rather than the gating-block one (issue #1051).
#[derive(Debug, Clone)]
pub struct LoudnessMeter {
    sample_rate: u32,
    layout: ChannelLayout,
    channel_count: usize,

    /// Per‑channel K‑weighting filter state.
    filters: Vec<ChannelFilter>,

    /// The K‑weighting biquad coefficients (derived for `sample_rate`).
    coeffs: BiquadCoeffsPair,

    /// Samples per 100 ms sub-block.
    sub_block_samples: usize,
    /// Weighted-power sum accumulating for the sub-block in progress.
    current_sub_block_sum: f64,
    /// Sample frames accumulated into `current_sub_block_sum` so far.
    current_sub_block_count: usize,

    /// Ring of the last up to [`SHORT_TERM_SUB_BLOCKS`] completed
    /// sub-blocks' mean-square energy — bounded, never grows past that
    /// capacity. The momentary/gating window is the mean of its last
    /// [`MOMENTARY_SUB_BLOCKS`] entries; the short-term window is the mean
    /// of all of it once full.
    sub_block_ring: VecDeque<f64>,

    /// One 400 ms gating-block LKFS value per completed 100 ms step —
    /// drives integrated loudness. O(duration / 100 ms), not O(sample
    /// count).
    gating_blocks: Vec<LkfsSample>,

    /// One 3 s short-term LKFS value per completed 100 ms step (once at
    /// least 3 s have elapsed) — drives LRA (issue #1051). Same bound as
    /// `gating_blocks`.
    short_term_blocks: Vec<LkfsSample>,

    /// Exact per-sample sliding-window max tracker for the 400 ms
    /// momentary window (bounded to `0.4 * sample_rate` samples — see
    /// [`SlidingWindowMax`]).
    momentary_max: SlidingWindowMax,

    /// Exact per-sample sliding-window max tracker for the 3 s short-term
    /// window (bounded to `3.0 * sample_rate` samples).
    short_term_max: SlidingWindowMax,

    /// Integrated loudness result (computed on `finish()`).
    integrated: f64,

    /// Loudness range result (computed on `finish()`).
    lra: f64,

    /// Maximum momentary loudness (cached from `momentary_max` on `finish()`).
    max_momentary: f64,

    /// Maximum short‑term loudness (cached from `short_term_max` on `finish()`).
    max_short_term: f64,

    /// Whether `finish()` has been called.
    finished: bool,

    /// Number of sample frames pushed.
    frame_count: usize,
}

impl LoudnessMeter {
    /// Create a new loudness meter.
    ///
    /// `sample_rate` — input sample rate in Hz. Any rate greater than zero is
    /// accepted (e.g. 44100, 48000, 96000, 192000). The K‑weighting filter
    /// coefficients are derived for the given rate by a bilinear transform of
    /// the analog prototype filters with frequency pre‑warping (the same
    /// derivation libebur128 uses); at 48 kHz they match the ITU‑R BS.1770‑5
    /// Annex 1 tabulated coefficients to within floating‑point epsilon.
    ///
    /// Returns `Error::InvalidSampleRate` if `sample_rate == 0`.
    ///
    /// `layout` — channel configuration with per‑channel weights.
    pub fn new(sample_rate: u32, layout: ChannelLayout) -> Result<Self, crate::Error> {
        if sample_rate == 0 {
            return Err(crate::Error::InvalidSampleRate { got: sample_rate });
        }
        let (stage1, stage2) = k_weighting_coeffs(sample_rate);
        let coeffs = BiquadCoeffsPair { stage1, stage2 };
        let channel_count = layout.channel_count();
        let sub_block_samples = ((SUB_BLOCK_S * f64::from(sample_rate)) as usize).max(1);
        let momentary_window_samples =
            ((MOMENTARY_SUB_BLOCKS as f64 * SUB_BLOCK_S * f64::from(sample_rate)) as usize).max(1);
        let short_term_window_samples =
            ((SHORT_TERM_SUB_BLOCKS as f64 * SUB_BLOCK_S * f64::from(sample_rate)) as usize).max(1);
        Ok(Self {
            sample_rate,
            layout,
            channel_count,
            filters: (0..channel_count)
                .map(|_| ChannelFilter::new(stage1, stage2))
                .collect(),
            coeffs,
            sub_block_samples,
            current_sub_block_sum: 0.0,
            current_sub_block_count: 0,
            sub_block_ring: VecDeque::with_capacity(SHORT_TERM_SUB_BLOCKS),
            gating_blocks: Vec::new(),
            short_term_blocks: Vec::new(),
            momentary_max: SlidingWindowMax::new(momentary_window_samples),
            short_term_max: SlidingWindowMax::new(short_term_window_samples),
            integrated: f64::NEG_INFINITY,
            lra: 0.0,
            max_momentary: f64::NEG_INFINITY,
            max_short_term: f64::NEG_INFINITY,
            finished: false,
            frame_count: 0,
        })
    }

    /// Reset the meter for a new measurement.
    pub fn reset(&mut self) {
        for f in &mut self.filters {
            *f = ChannelFilter::new(self.coeffs.stage1, self.coeffs.stage2);
        }
        self.current_sub_block_sum = 0.0;
        self.current_sub_block_count = 0;
        self.sub_block_ring.clear();
        self.gating_blocks.clear();
        self.short_term_blocks.clear();
        self.momentary_max.reset();
        self.short_term_max.reset();
        self.integrated = f64::NEG_INFINITY;
        self.lra = 0.0;
        self.max_momentary = f64::NEG_INFINITY;
        self.max_short_term = f64::NEG_INFINITY;
        self.finished = false;
        self.frame_count = 0;
    }

    /// Fold one sample frame's K-weighted, channel-weighted power into (a)
    /// the exact sliding-window max trackers (O(1) amortized, bounded to
    /// each window's fixed sample count — issue #1072) and (b) the 100 ms
    /// sub-block accumulator, completing it (see
    /// [`Self::complete_sub_block`]) once `sub_block_samples` frames have
    /// been folded in. No per-sample storage proportional to measurement
    /// duration.
    fn accumulate_sub_block(&mut self, weighted_power: f64) {
        self.momentary_max.push(weighted_power);
        self.short_term_max.push(weighted_power);

        self.current_sub_block_sum += weighted_power;
        self.current_sub_block_count += 1;
        if self.current_sub_block_count >= self.sub_block_samples {
            self.complete_sub_block();
        }
    }

    /// Finalize a *full* in-progress sub-block into the bounded ring
    /// (evicting the oldest once past [`SHORT_TERM_SUB_BLOCKS`]), then
    /// derive any gating-block (400 ms) / short-term (3 s) value it newly
    /// completes. Called once per `sub_block_samples` frames from
    /// `accumulate_sub_block`, so `current_sub_block_count` is always
    /// exactly `sub_block_samples` here — never partial (#1108/LOUD-W2:
    /// `finish()` used to flush a leftover PARTIAL sub-block through this
    /// same path, averaging fewer-than-`sub_block_samples` frames as if
    /// they were a full 100 ms and feeding that into the 400 ms/3 s
    /// windows on equal footing with genuinely complete sub-blocks, which
    /// biases the last up-to-3 gating/short-term values of every
    /// measurement — BS.1770-5 gating uses complete blocks only.
    /// `finish()` now discards that remainder instead of flushing it).
    fn complete_sub_block(&mut self) {
        if self.current_sub_block_count == 0 {
            return;
        }
        let mean_sq = self.current_sub_block_sum / self.current_sub_block_count as f64;
        self.current_sub_block_sum = 0.0;
        self.current_sub_block_count = 0;

        self.sub_block_ring.push_back(mean_sq);
        if self.sub_block_ring.len() > SHORT_TERM_SUB_BLOCKS {
            self.sub_block_ring.pop_front();
        }

        let n = self.sub_block_ring.len();
        if n >= MOMENTARY_SUB_BLOCKS {
            let sum: f64 = self
                .sub_block_ring
                .iter()
                .rev()
                .take(MOMENTARY_SUB_BLOCKS)
                .sum();
            let lkfs = mean_sq_to_lkfs(sum / MOMENTARY_SUB_BLOCKS as f64);
            self.gating_blocks.push(LkfsSample { lkfs });
        }
        if n >= SHORT_TERM_SUB_BLOCKS {
            let sum: f64 = self.sub_block_ring.iter().sum();
            let lkfs = mean_sq_to_lkfs(sum / SHORT_TERM_SUB_BLOCKS as f64);
            self.short_term_blocks.push(LkfsSample { lkfs });
        }
    }

    /// Push one frame of planar f32 samples.
    ///
    /// `channels` must have length equal to `self.channel_count`.
    /// Each entry is the sample for one channel at this time instant.
    pub fn push_f32(&mut self, channels: &[f32]) -> Result<(), crate::Error> {
        if self.finished {
            return Err(crate::Error::Finished);
        }
        if channels.len() != self.channel_count {
            return Err(crate::Error::ChannelMismatch {
                expected: self.channel_count,
                got: channels.len(),
            });
        }
        let mut sum_sq = 0.0f64;
        for (i, &sample) in channels.iter().enumerate() {
            let sample_f64 = f64::from(sample);
            if !sample_f64.is_finite() {
                return Err(crate::Error::NonFiniteSample {
                    index: self.frame_count,
                    channel: i,
                    value: sample_f64,
                });
            }
            let weight = self.layout.weight(i);
            if weight == 0.0 {
                continue;
            }
            let filtered = self.filters[i].process(sample_f64);
            sum_sq += weight * filtered * filtered;
        }
        self.accumulate_sub_block(sum_sq);
        self.frame_count += 1;
        Ok(())
    }

    /// Push one frame of planar f64 samples.
    pub fn push_f64(&mut self, channels: &[f64]) -> Result<(), crate::Error> {
        if self.finished {
            return Err(crate::Error::Finished);
        }
        if channels.len() != self.channel_count {
            return Err(crate::Error::ChannelMismatch {
                expected: self.channel_count,
                got: channels.len(),
            });
        }
        let mut sum_sq = 0.0f64;
        for (i, &sample) in channels.iter().enumerate() {
            if !sample.is_finite() {
                return Err(crate::Error::NonFiniteSample {
                    index: self.frame_count,
                    channel: i,
                    value: sample,
                });
            }
            let weight = self.layout.weight(i);
            if weight == 0.0 {
                continue;
            }
            let filtered = self.filters[i].process(sample);
            sum_sq += weight * filtered * filtered;
        }
        self.accumulate_sub_block(sum_sq);
        self.frame_count += 1;
        Ok(())
    }

    /// Push two planar (not interleaved) f32 channel buffers for a
    /// **stereo** (2-channel) layout.
    ///
    /// `left` and `right` must have equal length. Returns
    /// [`crate::Error::ChannelMismatch`] if the configured
    /// [`crate::ChannelLayout`] isn't exactly 2 channels (#1108/LOUD-W1):
    /// pre-fix, calling this on e.g. a `Mono` layout indexed `self.
    /// filters[1]`, a 1-element `Vec`, and panicked.
    ///
    /// ```
    /// use broadcast_loudness::{ChannelLayout, LoudnessMeter};
    ///
    /// // One second of planar stereo silence at 48 kHz.
    /// let left_samples = vec![0.0_f32; 48_000];
    /// let right_samples = vec![0.0_f32; 48_000];
    ///
    /// let mut meter = LoudnessMeter::new(48_000, ChannelLayout::Stereo).unwrap();
    /// meter.push_stereo_planar_f32(&left_samples, &right_samples).unwrap();
    /// meter.finish();
    ///
    /// println!("Integrated: {:.1} LUFS", meter.integrated_lufs());
    /// println!("LRA:       {:.1} LU",   meter.loudness_range());
    /// println!("Max M:     {:.1} LUFS", meter.max_momentary_lufs());
    /// println!("Max S:     {:.1} LUFS", meter.max_short_term_lufs());
    /// ```
    pub fn push_stereo_planar_f32(
        &mut self,
        left: &[f32],
        right: &[f32],
    ) -> Result<(), crate::Error> {
        if self.finished {
            return Err(crate::Error::Finished);
        }
        if self.layout.channel_count() != 2 {
            return Err(crate::Error::ChannelMismatch {
                expected: 2,
                got: self.layout.channel_count(),
            });
        }
        if left.len() != right.len() {
            return Err(crate::Error::ChannelMismatch {
                expected: left.len(),
                got: right.len(),
            });
        }
        for (frame_idx, (&l, &r)) in left.iter().zip(right.iter()).enumerate() {
            let l_f64 = f64::from(l);
            let r_f64 = f64::from(r);
            if !l_f64.is_finite() {
                return Err(crate::Error::NonFiniteSample {
                    index: self.frame_count + frame_idx,
                    channel: 0,
                    value: l_f64,
                });
            }
            if !r_f64.is_finite() {
                return Err(crate::Error::NonFiniteSample {
                    index: self.frame_count + frame_idx,
                    channel: 1,
                    value: r_f64,
                });
            }
            let weight_l = self.layout.weight(0);
            let weight_r = self.layout.weight(1);
            let mut sum_sq = 0.0;
            if weight_l != 0.0 {
                let f = self.filters[0].process(l_f64);
                sum_sq += weight_l * f * f;
            }
            if weight_r != 0.0 {
                let f = self.filters[1].process(r_f64);
                sum_sq += weight_r * f * f;
            }
            self.accumulate_sub_block(sum_sq);
        }
        self.frame_count += left.len();
        Ok(())
    }

    /// Finish measurement and compute integrated loudness + LRA.
    ///
    /// After calling this, no more samples are accepted. Query results via
    /// `integrated_lufs()`, `loudness_range()`, `max_momentary_lufs()`,
    /// and `max_short_term_lufs()`.
    pub fn finish(&mut self) {
        if self.finished {
            return;
        }
        self.finished = true;

        // #1108/LOUD-W2: a final in-progress sub-block with fewer than
        // `sub_block_samples` frames is discarded, not flushed — BS.1770-5
        // gating uses complete 400 ms blocks (built from complete 100 ms
        // sub-blocks); averaging a short remainder as if it were a full
        // sub-block and feeding it into the 400 ms/3 s windows on equal
        // footing with genuinely complete ones biased the last up-to-3
        // gating/short-term values of every measurement. See
        // `complete_sub_block`'s doc comment.
        self.current_sub_block_sum = 0.0;
        self.current_sub_block_count = 0;

        // --- Integrated loudness (two‑stage gating), from the 400 ms
        // gating-block history ---
        // Stage 1: absolute gate at —70 LKFS
        let abs_gated: Vec<f64> = self
            .gating_blocks
            .iter()
            .filter(|b| b.lkfs > ABSOLUTE_GATE)
            .map(|b| b.lkfs)
            .collect();

        let integrated = if abs_gated.is_empty() {
            f64::NEG_INFINITY
        } else {
            let abs_gated_loudness = mean_of_lkfs(&abs_gated);
            let rel_threshold = abs_gated_loudness + RELATIVE_GATE;

            // Stage 2: relative gate
            let rel_gated: Vec<f64> = abs_gated
                .iter()
                .filter(|&&l| l > rel_threshold)
                .copied()
                .collect();

            if rel_gated.is_empty() {
                f64::NEG_INFINITY
            } else {
                mean_of_lkfs(&rel_gated)
            }
        };
        self.integrated = integrated;

        // --- LRA (EBU Tech 3342), from the 3 s short-term history
        // (issue #1051 — NOT the 400 ms gating-block history) ---
        self.lra = compute_lra(&self.short_term_blocks);

        // --- Max momentary and max short‑term: read from the exact
        // per-sample sliding-window trackers (updated incrementally on
        // every pushed sample — see `accumulate_sub_block`/
        // `SlidingWindowMax`), not the 100 ms-grid block histories above.
        self.max_momentary = self.momentary_max.max_lkfs;
        self.max_short_term = self.short_term_max.max_lkfs;
    }

    // ---- Query methods ----

    /// Integrated loudness in LUFS (gated, two‑stage, per BS.1770‑5).
    ///
    /// Returns `f64::NEG_INFINITY` if the measurement has no valid blocks.
    #[must_use]
    pub fn integrated_lufs(&self) -> f64 {
        self.integrated
    }

    /// Integrated loudness relative to —23 LUFS target level, in LU.
    #[must_use]
    pub fn integrated_lu(&self) -> f64 {
        if self.integrated.is_finite() {
            self.integrated + 23.0
        } else {
            f64::NEG_INFINITY
        }
    }

    /// Maximum momentary loudness (400 ms window) in LUFS.
    #[must_use]
    pub fn max_momentary_lufs(&self) -> f64 {
        self.max_momentary
    }

    /// Maximum short‑term loudness (3 s window) in LUFS.
    #[must_use]
    pub fn max_short_term_lufs(&self) -> f64 {
        self.max_short_term
    }

    /// Loudness Range in LU (EBU Tech 3342).
    #[must_use]
    pub fn loudness_range(&self) -> f64 {
        self.lra
    }

    /// Number of sample frames processed.
    #[must_use]
    pub fn frame_count(&self) -> usize {
        self.frame_count
    }

    /// Duration in seconds of the measurement so far.
    #[must_use]
    pub fn duration_seconds(&self) -> f64 {
        self.frame_count as f64 / self.sample_rate as f64
    }
}

/// Compute the mean loudness from a list of LKFS values.
fn mean_of_lkfs(values: &[f64]) -> f64 {
    if values.is_empty() {
        return f64::NEG_INFINITY;
    }
    let n = values.len() as f64;
    let sum_power: f64 = values.iter().map(|&l| lkfs_to_mean_sq(l)).sum();
    mean_sq_to_lkfs(sum_power / n)
}

/// Compute Loudness Range per EBU Tech 3342.
///
/// Input: short‑term (3 s sliding window) loudness values, sampled at
/// 10 Hz (issue #1051 — NOT 400 ms momentary/gating blocks, which have
/// far higher variance on real programme material and badly inflate LRA;
/// see the module docs). The algorithm:
/// 1. Absolute gate: keep blocks ≥ —70 LUFS.
/// 2. Compute absolute‑gated integrated loudness.
/// 3. Relative gate: keep blocks ≥ (integrated —20 LU).
/// 4. Compute 10th and 95th percentiles of the distribution.
/// 5. LRA = 95th percentile — 10th percentile.
fn compute_lra(blocks: &[LkfsSample]) -> f64 {
    // Absolute gate
    let abs_gated: Vec<f64> = blocks
        .iter()
        .filter(|b| b.lkfs >= ABSOLUTE_GATE)
        .map(|b| b.lkfs)
        .collect();

    if abs_gated.is_empty() {
        return 0.0;
    }

    let abs_integrated = mean_of_lkfs(&abs_gated);
    let rel_threshold = abs_integrated - 20.0; // —20 LU relative gate

    // Relative gate
    let mut rel_gated: Vec<f64> = abs_gated
        .iter()
        .filter(|&&l| l >= rel_threshold)
        .copied()
        .collect();

    if rel_gated.is_empty() {
        return 0.0;
    }

    // Sort for percentile computation
    rel_gated.sort_unstable_by(|a, b| a.partial_cmp(b).unwrap_or(core::cmp::Ordering::Equal));

    let n = rel_gated.len();
    // Tech 3342 MATLAB: round((n-1)*PRC/100 + 1) — 1‑based indexing → 0‑based
    let low_idx = (libm::round((n as f64 - 1.0) * 10.0 / 100.0 + 1.0) as usize).saturating_sub(1);
    let high_idx = (libm::round((n as f64 - 1.0) * 95.0 / 100.0 + 1.0) as usize).saturating_sub(1);
    let low_idx = low_idx.min(n - 1);
    let high_idx = high_idx.min(n - 1);

    let perc_low = rel_gated[low_idx];
    let perc_high = rel_gated[high_idx];

    perc_high - perc_low
}

#[cfg(test)]
mod tests {
    use super::{LoudnessMeter, SHORT_TERM_SUB_BLOCKS};
    use crate::channel_layout::ChannelLayout;

    /// Issue #1072 (audit LOUD-C2): the bounded ring/tracker state must
    /// never grow with sample RATE or sample COUNT, only with measurement
    /// duration — a 2 s measurement at 192 kHz must carry the exact same
    /// bounded-collection sizes as one at 48 kHz, even though it folds in
    /// 4x as many raw samples.
    #[test]
    fn state_is_bounded_by_duration_not_sample_rate_or_count() {
        for &sample_rate in &[48_000u32, 192_000] {
            let mut meter = LoudnessMeter::new(sample_rate, ChannelLayout::Mono).unwrap();
            let duration_s = 2.0;
            let n = (duration_s * sample_rate as f64) as usize;
            for i in 0..n {
                let t = i as f64 / sample_rate as f64;
                let v = (0.1 * (2.0 * core::f64::consts::PI * 1000.0 * t).sin()) as f32;
                meter.push_f32(&[v]).unwrap();
            }

            // The sub-block ring and both sliding-window rings are
            // fixed-capacity, never reallocated to grow with input size.
            assert!(
                meter.sub_block_ring.len() <= SHORT_TERM_SUB_BLOCKS,
                "sample_rate {sample_rate}: sub_block_ring grew past its {SHORT_TERM_SUB_BLOCKS}-entry cap"
            );
            assert_eq!(
                meter.momentary_max.ring.len(),
                meter.momentary_max.window_samples,
                "sample_rate {sample_rate}: momentary ring is not fixed-size"
            );
            assert_eq!(
                meter.short_term_max.ring.len(),
                meter.short_term_max.window_samples,
                "sample_rate {sample_rate}: short-term ring is not fixed-size"
            );

            // The per-100ms-step histories grow with DURATION (~20 entries
            // for a 2 s measurement), never with sample count — 192 kHz's
            // 4x the raw samples must NOT produce 4x the entries.
            assert!(
                meter.gating_blocks.len() <= 25,
                "sample_rate {sample_rate}: gating_blocks scaled with sample count \
                 ({} entries for a 2 s measurement)",
                meter.gating_blocks.len()
            );
        }
    }

    #[test]
    fn stereo_1khz_minus_23_lufs_is_minus_23() {
        let sample_rate = 48_000;
        let duration = 2.0;
        let n = (duration * sample_rate as f64) as usize;
        let amplitude = 10.0f64.powf(-23.0 / 20.0) as f32;
        let mut left = alloc::vec![0.0f32; n];
        let mut right = alloc::vec![0.0f32; n];
        for i in 0..n {
            let t = i as f64 / sample_rate as f64;
            let val = (amplitude as f64 * (2.0 * core::f64::consts::PI * 1000.0 * t).sin()) as f32;
            left[i] = val;
            right[i] = val;
        }
        let mut meter = LoudnessMeter::new(sample_rate, ChannelLayout::Stereo).unwrap();
        meter.push_stereo_planar_f32(&left, &right).unwrap();
        meter.finish();
        let lufs = meter.integrated_lufs();
        assert!((lufs - (-23.0)).abs() < 0.2, "got {lufs}, expected -23.0");
    }

    /// LOUD-W1 (#1108): calling `push_stereo_planar_f32` on a non-stereo
    /// layout used to panic (`Mono` has a 1-element `self.filters`, and
    /// this indexed `self.filters[1]` unconditionally); it now returns
    /// `ChannelMismatch`.
    #[test]
    fn push_stereo_planar_rejects_non_stereo_layout() {
        let mut meter = LoudnessMeter::new(48_000, ChannelLayout::Mono).unwrap();
        let samples = [0.1f32, 0.2, 0.3];
        let err = meter
            .push_stereo_planar_f32(&samples, &samples)
            .unwrap_err();
        assert!(matches!(
            err,
            crate::Error::ChannelMismatch {
                expected: 2,
                got: 1
            }
        ));
    }

    /// LOUD-W2 (#1108): a final, in-progress sub-block with fewer than
    /// `sub_block_samples` frames must be discarded on `finish()`, not
    /// averaged as if it were a full sub-block and folded into the last
    /// gating-block window on equal footing with complete sub-blocks.
    /// Appending a short (partial-sub-block) burst at a very different
    /// level must not move `integrated_lufs()` at all, since it's
    /// discarded outright — pre-fix, it was flushed and biased the last
    /// gating-block value.
    #[test]
    fn trailing_partial_sub_block_does_not_affect_integrated_loudness() {
        let sample_rate = 48_000u32;
        let tone_amplitude = 10.0f64.powf(-23.0 / 20.0) as f32;
        // 700 ms = exactly 7 complete 100 ms sub-blocks; no remainder.
        let n_base = (0.7 * f64::from(sample_rate)) as usize;
        let make_tone = |n: usize, amp: f32, freq: f64| -> Vec<f32> {
            (0..n)
                .map(|i| {
                    let t = i as f64 / f64::from(sample_rate);
                    (f64::from(amp) * (2.0 * core::f64::consts::PI * freq * t).sin()) as f32
                })
                .collect()
        };
        let base = make_tone(n_base, tone_amplitude, 1000.0);

        let mut meter_a = LoudnessMeter::new(sample_rate, ChannelLayout::Stereo).unwrap();
        meter_a.push_stereo_planar_f32(&base, &base).unwrap();
        meter_a.finish();
        let baseline_lufs = meter_a.integrated_lufs();

        // Same 700 ms, plus a 20 ms burst at a much louder level (partial
        // sub-block — 20 ms << the 100 ms sub_block_samples threshold).
        let loud_amplitude = 10.0f64.powf(-3.0 / 20.0) as f32;
        let n_tail = (0.02 * f64::from(sample_rate)) as usize;
        let tail = make_tone(n_tail, loud_amplitude, 1000.0);
        let mut left = base.clone();
        let mut right = base.clone();
        left.extend_from_slice(&tail);
        right.extend_from_slice(&tail);

        let mut meter_b = LoudnessMeter::new(sample_rate, ChannelLayout::Stereo).unwrap();
        meter_b.push_stereo_planar_f32(&left, &right).unwrap();
        meter_b.finish();
        let with_tail_lufs = meter_b.integrated_lufs();

        assert!(
            (with_tail_lufs - baseline_lufs).abs() < 1e-9,
            "a discarded partial trailing sub-block must not change integrated_lufs: \
             baseline={baseline_lufs}, with_tail={with_tail_lufs}"
        );
    }

    #[test]
    fn absolute_gate_excludes_silence() {
        // 2 s of silence, then 3 s at —23 LUFS.
        let sample_rate = 48_000;
        let silence_s = 2.0;
        let tone_s = 3.0;
        let n_silence = (silence_s * sample_rate as f64) as usize;
        let n_tone = (tone_s * sample_rate as f64) as usize;
        let amplitude = 10.0f64.powf(-23.0 / 20.0) as f32;

        let mut left = alloc::vec![0.0f32; n_silence + n_tone];
        let mut right = alloc::vec![0.0f32; n_silence + n_tone];
        for i in n_silence..(n_silence + n_tone) {
            let t = (i - n_silence) as f64 / sample_rate as f64;
            let val = (amplitude as f64 * (2.0 * core::f64::consts::PI * 1000.0 * t).sin()) as f32;
            left[i] = val;
            right[i] = val;
        }
        let mut meter = LoudnessMeter::new(sample_rate, ChannelLayout::Stereo).unwrap();
        meter.push_stereo_planar_f32(&left, &right).unwrap();
        meter.finish();
        let lufs = meter.integrated_lufs();
        assert!(
            (lufs - (-23.0)).abs() < 0.5,
            "got {lufs}, expected ~-23.0 (gating should exclude silence)"
        );
    }

    #[test]
    fn low_signal_below_absolute_gate_does_not_drag_integrated() {
        // 2 s at —80 LUFS, then 3 s at —23 LUFS, then 2 s at —80.
        let sample_rate = 48_000;
        let tone_amplitude = 10.0f64.powf(-23.0 / 20.0) as f32;
        let low_amplitude = 10.0f64.powf(-80.0 / 20.0) as f32;

        let seg_low1_s = 2.0;
        let seg_tone_s = 3.0;
        let seg_low2_s = 2.0;
        let total = (seg_low1_s + seg_tone_s + seg_low2_s) * sample_rate as f64;
        let n = total as usize;
        let mut left = alloc::vec![0.0f32; n];
        let mut right = alloc::vec![0.0f32; n];

        let n_low1 = (seg_low1_s * sample_rate as f64) as usize;
        let n_tone = (seg_tone_s * sample_rate as f64) as usize;

        for i in 0..n_low1 {
            let t = i as f64 / sample_rate as f64;
            let val =
                (low_amplitude as f64 * (2.0 * core::f64::consts::PI * 1000.0 * t).sin()) as f32;
            left[i] = val;
            right[i] = val;
        }
        for i in 0..n_tone {
            let t = i as f64 / sample_rate as f64;
            let val =
                (tone_amplitude as f64 * (2.0 * core::f64::consts::PI * 1000.0 * t).sin()) as f32;
            left[n_low1 + i] = val;
            right[n_low1 + i] = val;
        }
        let offset = n_low1 + n_tone;
        for i in 0..(n - offset) {
            let t = i as f64 / sample_rate as f64;
            let val =
                (low_amplitude as f64 * (2.0 * core::f64::consts::PI * 1000.0 * t).sin()) as f32;
            left[offset + i] = val;
            right[offset + i] = val;
        }

        let mut meter = LoudnessMeter::new(sample_rate, ChannelLayout::Stereo).unwrap();
        meter.push_stereo_planar_f32(&left, &right).unwrap();
        meter.finish();
        let lufs = meter.integrated_lufs();
        assert!(
            (lufs - (-23.0)).abs() < 0.5,
            "got {lufs}, expected ~-23.0 (low segments should be gated out)"
        );
    }

    #[test]
    fn accepts_441_khz() {
        // 44100 Hz is now accepted; coefficients are derived via bilinear
        // transform rather than restricted to 48 kHz.
        assert!(LoudnessMeter::new(44_100, ChannelLayout::Stereo).is_ok());
    }

    #[test]
    fn rejects_zero_rate() {
        let err = LoudnessMeter::new(0, ChannelLayout::Stereo).unwrap_err();
        let msg = format!("{err}");
        assert!(
            msg.contains("sample rate") && msg.contains("0"),
            "expected invalid sample rate error, got: {msg}"
        );
    }

    #[test]
    fn coeffs_at_48k_match_tabulated() {
        // The bilinear transform at 48 kHz must reproduce the BS.1770-5 Annex 1
        // tabulated coefficients to within floating-point epsilon.
        let (stage1, stage2) = crate::filter::k_weighting_coeffs(48_000);

        let shelf_ref = crate::filter::BiquadCoeffs {
            b0: 1.535_124_859_586_97,
            b1: -2.691_696_189_406_38,
            b2: 1.198_392_810_852_85,
            a1: -1.690_659_293_182_41,
            a2: 0.732_480_774_215_85,
        };
        let hp_ref = crate::filter::BiquadCoeffs {
            b0: 1.0,
            b1: -2.0,
            b2: 1.0,
            a1: -1.990_047_454_833_98,
            a2: 0.990_072_250_366_21,
        };

        for (got, want) in [
            (stage1.b0, shelf_ref.b0),
            (stage1.b1, shelf_ref.b1),
            (stage1.b2, shelf_ref.b2),
            (stage1.a1, shelf_ref.a1),
            (stage1.a2, shelf_ref.a2),
            (stage2.b0, hp_ref.b0),
            (stage2.b1, hp_ref.b1),
            (stage2.b2, hp_ref.b2),
            (stage2.a1, hp_ref.a1),
            (stage2.a2, hp_ref.a2),
        ] {
            assert!(
                (got - want).abs() < 1e-12,
                "expected {want}, got {got} (diff {})",
                (got - want).abs()
            );
        }
    }

    #[test]
    fn rejects_nan_in_planar_f32() {
        let mut meter = LoudnessMeter::new(48_000, ChannelLayout::Stereo).unwrap();
        let err = meter.push_f32(&[f32::NAN, 0.5]).unwrap_err();
        let msg = format!("{err}");
        assert!(msg.contains("non-finite"), "got: {msg}");
    }

    #[test]
    fn rejects_inf_in_planar_f32() {
        let mut meter = LoudnessMeter::new(48_000, ChannelLayout::Stereo).unwrap();
        let err = meter.push_f32(&[0.5, f32::INFINITY]).unwrap_err();
        let msg = format!("{err}");
        assert!(msg.contains("non-finite"), "got: {msg}");
    }

    #[test]
    fn rejects_non_finite_in_planar_f64() {
        let mut meter = LoudnessMeter::new(48_000, ChannelLayout::Stereo).unwrap();
        let err = meter.push_f64(&[f64::NEG_INFINITY, 0.5]).unwrap_err();
        let msg = format!("{err}");
        assert!(msg.contains("non-finite"), "got: {msg}");
    }

    #[test]
    fn rejects_non_finite_in_interleaved_f32() {
        let mut meter = LoudnessMeter::new(48_000, ChannelLayout::Stereo).unwrap();
        let err = meter
            .push_stereo_planar_f32(&[0.5], &[f32::NAN])
            .unwrap_err();
        let msg = format!("{err}");
        assert!(msg.contains("non-finite"), "got: {msg}");
    }

    #[test]
    fn meter_not_poisoned_after_non_finite_rejection() {
        let mut meter = LoudnessMeter::new(48_000, ChannelLayout::Stereo).unwrap();

        meter.push_f32(&[0.5, 0.5]).unwrap();

        let _ = meter.push_f32(&[f32::NAN, 0.5]);

        for _ in 0..192_000 {
            meter.push_f32(&[0.1, 0.1]).unwrap();
        }
        meter.finish();
        let lufs = meter.integrated_lufs();
        assert!(lufs.is_finite(), "meter was poisoned: got {lufs}");
        assert!(lufs < -10.0, "unexpectedly loud: {lufs}");
    }

    #[test]
    fn non_finite_sample_error_carries_metadata() {
        let mut meter = LoudnessMeter::new(48_000, ChannelLayout::Stereo).unwrap();
        meter.push_f32(&[1.0, 1.0]).unwrap();
        let err = meter.push_f32(&[f32::NEG_INFINITY, 0.5]).unwrap_err();
        let msg = format!("{err}");
        assert!(msg.contains("channel 0"), "got: {msg}");
        assert!(msg.contains("index 1"), "got: {msg}");
    }
}
