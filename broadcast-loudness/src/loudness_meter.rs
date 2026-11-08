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
//!   elapsed, one 3 s short-term LKFS value every 100 ms step. Each derived
//!   value is folded into a fixed-size histogram (issue #1729) and then
//!   dropped, so memory is O(1) in stream length and independent of sample
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
//! [`LoudnessMeter::loudness_range`] is computed from the 3 s short-term
//! histogram above; [`LoudnessMeter::integrated_lufs`] uses the 400 ms
//! gating-block histogram, which is the correct input for integrated
//! loudness (unaffected by this fix).
//!
//! ## Histogram accumulation (issue #1729)
//!
//! Blocks are binned by loudness at 0.01 LU resolution between the
//! −70 LKFS absolute gate (BS.1770-5 Annex 1, eq. 6; transcription
//! `private/specs-verified/loudness/broadcast-loudness/bs1770-algorithm.md`)
//! and a 30 LKFS ceiling. Each bin keeps its block count, the exact sum of
//! block mean-square energies, and its lowest and highest block energy, so
//! the gated mean of eq. (5)/(6) is computed from energies, not quantised
//! loudness. The two histograms take a fixed 640 000 bytes per meter
//! (2 × 10 000 bins × 32 bytes), allocated by [`LoudnessMeter::new`] and by
//! every `clone`, whatever the measurement length.
//!
//! The relative gates (−10 LU integrated; −20 LU LRA per EBU Tech 3342,
//! `private/specs-verified/loudness/broadcast-loudness/tech3342-lra.md`) are
//! applied at the exact threshold to every bin wholly above or below it, and
//! to the bin containing the threshold whenever its blocks all lie on one
//! side, so integrated loudness and the gated block set then equal the
//! block-list reference. When that one bin holds blocks on both sides of the
//! threshold, its loudest block is kept, its quietest dropped, and the count
//! in between estimated as if every block sat at the bin's lowest or highest
//! energy; this is exact when the bin holds at most two distinct loudnesses
//! (steady programme plus a transition block). Otherwise up to `count - 2`
//! blocks of that bin, all within 0.01 LU of the gate, can land on the wrong
//! side. Gating is discontinuous, so the worst case is the reference's own
//! jump at that gate: on a two-level test signal whose quieter level sits in
//! that bin, about 1.9 LU of integrated loudness or 21 LU of LRA, confined to
//! programme levels whose block loudnesses straddle the threshold inside one
//! bin.
//!
//! The Tech 3342 10th/95th percentiles are resolved to a bin and report the
//! loudness of that bin's mean energy, so each is within one bin width
//! (0.01 LU) of the reference and LRA within two.

use alloc::collections::VecDeque;
use alloc::vec::Vec;

use crate::channel_layout::ChannelLayout;
use crate::filter::{
    BiquadCoeffs, BiquadState, K_WEIGHTING_MAX_GAIN, MIN_SAMPLE_RATE, apply_biquad,
    k_weighting_coeffs,
};

/// Highest accepted sample rate in Hz (768 kHz, the top PCM rate in use: 16 x
/// 48 kHz). The sliding-window trackers hold `3.4 s * sample_rate` `f64`s, so
/// the cap bounds a meter built from an untrusted rate to about 21 MB instead
/// of letting a `u32` rate request tens of gigabytes.
pub const MAX_SAMPLE_RATE: u32 = 768_000;

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

/// Inverse of [`mean_sq_to_lkfs`].
fn lkfs_to_mean_sq(lkfs: f64) -> f64 {
    libm::pow(10.0, (lkfs - LOUDNESS_OFFSET) / 10.0)
}

/// Largest sample magnitude for `layout` that keeps every block below
/// [`HISTOGRAM_CEILING_LKFS`]: a K‑weighted sample is at most
/// [`K_WEIGHTING_MAX_GAIN`] times the largest input magnitude, so a block's
/// weighted mean square is at most `sum(|G_i|) * (K_WEIGHTING_MAX_GAIN * x)^2`.
fn max_sample_magnitude(layout: ChannelLayout) -> f64 {
    let weight_sum: f64 = (0..layout.channel_count())
        .map(|i| libm::fabs(layout.weight(i)))
        .sum();
    libm::sqrt(lkfs_to_mean_sq(HISTOGRAM_CEILING_LKFS) / weight_sum) / K_WEIGHTING_MAX_GAIN
}

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

/// Bin width of [`LoudnessHistogram`] in LU.
const BIN_WIDTH_LU: f64 = 0.01;

/// Upper edge of [`LoudnessHistogram`]; [`max_sample_magnitude`] keeps every
/// block below it.
const HISTOGRAM_CEILING_LKFS: f64 = 30.0;

/// Number of bins spanning [`ABSOLUTE_GATE`] to [`HISTOGRAM_CEILING_LKFS`].
const HISTOGRAM_BINS: usize =
    ((HISTOGRAM_CEILING_LKFS - ABSOLUTE_GATE) / BIN_WIDTH_LU + 0.5) as usize;

/// LRA relative gate in LU (EBU Tech 3342).
const LRA_RELATIVE_GATE: f64 = -20.0;

/// LRA lower percentile (EBU Tech 3342).
const LRA_LOW_PERCENT: f64 = 10.0;

/// LRA upper percentile (EBU Tech 3342).
const LRA_HIGH_PERCENT: f64 = 95.0;

/// How a block loudness is compared with a gate threshold: BS.1770-5 eq. (6)
/// keeps blocks strictly above each gate, the Tech 3342 reference keeps blocks
/// at or above it.
#[derive(Debug, Clone, Copy)]
enum GateComparison {
    Above,
    AtOrAbove,
}

impl GateComparison {
    fn passes(self, lkfs: f64, threshold: f64) -> bool {
        match self {
            Self::Above => lkfs > threshold,
            Self::AtOrAbove => lkfs >= threshold,
        }
    }
}

/// One histogram bin: block count, exact sum of block mean-square energies,
/// and the lowest and highest block mean-square energy it holds.
#[derive(Debug, Clone, Copy)]
struct Bin {
    count: u64,
    energy: f64,
    min_energy: f64,
    max_energy: f64,
}

impl Bin {
    const EMPTY: Self = Self {
        count: 0,
        energy: 0.0,
        min_energy: f64::INFINITY,
        max_energy: 0.0,
    };

    fn mean_lkfs(&self) -> f64 {
        mean_sq_to_lkfs(self.energy / self.count as f64)
    }

    fn min_lkfs(&self) -> f64 {
        mean_sq_to_lkfs(self.min_energy)
    }

    fn max_lkfs(&self) -> f64 {
        mean_sq_to_lkfs(self.max_energy)
    }
}

/// Fixed-size histogram of block loudness. Blocks failing the absolute gate
/// are not recorded.
#[derive(Debug, Clone)]
struct LoudnessHistogram {
    gate: GateComparison,
    bins: Vec<Bin>,
    #[cfg(test)]
    recorded: Vec<f64>,
}

impl LoudnessHistogram {
    fn new(gate: GateComparison) -> Self {
        Self {
            gate,
            bins: alloc::vec![Bin::EMPTY; HISTOGRAM_BINS],
            #[cfg(test)]
            recorded: Vec::new(),
        }
    }

    fn clear(&mut self) {
        self.bins.fill(Bin::EMPTY);
        #[cfg(test)]
        self.recorded.clear();
    }

    fn bin_of(lkfs: f64) -> usize {
        let idx = (lkfs - ABSOLUTE_GATE) / BIN_WIDTH_LU;
        (idx as usize).min(HISTOGRAM_BINS - 1)
    }

    fn record(&mut self, mean_sq: f64) {
        let lkfs = mean_sq_to_lkfs(mean_sq);
        if !self.gate.passes(lkfs, ABSOLUTE_GATE) {
            return;
        }
        #[cfg(test)]
        self.recorded.push(lkfs);
        let bin = &mut self.bins[Self::bin_of(lkfs)];
        bin.count += 1;
        bin.energy += mean_sq;
        bin.min_energy = bin.min_energy.min(mean_sq);
        bin.max_energy = bin.max_energy.max(mean_sq);
    }

    fn totals_from(&self, first_bin: usize) -> (u64, f64) {
        self.bins[first_bin..]
            .iter()
            .fold((0, 0.0), |(count, energy), bin| {
                (count + bin.count, energy + bin.energy)
            })
    }

    fn relative_threshold(&self, gate_lu: f64) -> Option<f64> {
        let (count, energy) = self.totals_from(0);
        (count > 0).then(|| mean_sq_to_lkfs(energy / count as f64) + gate_lu)
    }

    /// Where the gate at `threshold` cuts the histogram. Bin index is
    /// monotonic in loudness, so every bin below the threshold's bin is wholly
    /// below it and every bin above is wholly above it. The threshold's own
    /// bin is resolved exactly from its loudness range unless its blocks lie
    /// on both sides of the threshold. Then the blocks kept are estimated as
    /// if every block sat at the bin's lowest or highest energy, which the
    /// bin's energy sum then determines: exact when the bin holds at most two
    /// distinct block loudnesses, and never dropping its loudest block or
    /// keeping its quietest.
    fn cut(&self, threshold: f64) -> GateCut {
        let at = Self::bin_of(threshold);
        let bin = &self.bins[at];
        let (kept, kept_energy) = if bin.count == 0 || self.gate.passes(bin.min_lkfs(), threshold) {
            (bin.count, bin.energy)
        } else if !self.gate.passes(bin.max_lkfs(), threshold) {
            (0, 0.0)
        } else {
            let at_max = (bin.energy - bin.count as f64 * bin.min_energy)
                / (bin.max_energy - bin.min_energy);
            let kept = (libm::round(at_max) as u64).clamp(1, bin.count - 1);
            (kept, kept as f64 * bin.max_energy)
        };
        GateCut {
            bin: at,
            kept,
            kept_energy,
        }
    }

    fn totals_above(&self, cut: GateCut) -> (u64, f64) {
        let (count, energy) = self.totals_from(cut.bin + 1);
        (count + cut.kept, energy + cut.kept_energy)
    }

    fn integrated(&self) -> f64 {
        let Some(threshold) = self.relative_threshold(RELATIVE_GATE) else {
            return f64::NEG_INFINITY;
        };
        let (count, energy) = self.totals_above(self.cut(threshold));
        if count == 0 {
            return f64::NEG_INFINITY;
        }
        mean_sq_to_lkfs(energy / count as f64)
    }

    fn percentile_loudness(&self, cut: GateCut, rank: u64) -> f64 {
        let mut seen = 0u64;
        for (i, bin) in self.bins.iter().enumerate().skip(cut.bin) {
            seen += if i == cut.bin { cut.kept } else { bin.count };
            if seen > rank {
                return bin.mean_lkfs();
            }
        }
        f64::NEG_INFINITY
    }

    fn loudness_range(&self) -> f64 {
        let Some(threshold) = self.relative_threshold(LRA_RELATIVE_GATE) else {
            return 0.0;
        };
        let cut = self.cut(threshold);
        let (n, _) = self.totals_above(cut);
        if n == 0 {
            return 0.0;
        }
        // Tech 3342 MATLAB: round((n-1)*PRC/100 + 1), 1-based -> 0-based rank.
        let rank = |percent: f64| {
            let one_based = libm::round((n as f64 - 1.0) * percent / 100.0 + 1.0) as u64;
            one_based.saturating_sub(1).min(n - 1)
        };
        let low = self.percentile_loudness(cut, rank(LRA_LOW_PERCENT));
        let high = self.percentile_loudness(cut, rank(LRA_HIGH_PERCENT));
        high - low
    }
}

/// A relative gate's cut through a [`LoudnessHistogram`]: every bin above
/// `bin` is kept whole, and `kept` of the blocks in `bin`, holding
/// `kept_energy`, are kept.
#[derive(Debug, Clone, Copy)]
struct GateCut {
    bin: usize,
    kept: u64,
    kept_energy: f64,
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

    /// 400 ms gating blocks, one per completed 100 ms step — drives
    /// integrated loudness. Fixed size (issue #1729).
    gating_blocks: LoudnessHistogram,

    /// 3 s short-term values, one per completed 100 ms step once 3 s have
    /// elapsed — drives LRA (issue #1051). Fixed size (issue #1729).
    short_term_blocks: LoudnessHistogram,

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

    /// See [`Self::max_sample_magnitude`].
    max_sample_magnitude: f64,
}

impl LoudnessMeter {
    /// Create a new loudness meter.
    ///
    /// `sample_rate` — input sample rate in Hz. Any rate from
    /// [`MIN_SAMPLE_RATE`] up to [`MAX_SAMPLE_RATE`] is accepted (e.g. 44100, 48000, 96000, 192000). The K‑weighting filter
    /// coefficients are derived for the given rate by a bilinear transform of
    /// the analog prototype filters with frequency pre‑warping (the same
    /// derivation libebur128 uses); at 48 kHz they match the ITU‑R BS.1770‑5
    /// Annex 1 tabulated coefficients to within floating‑point epsilon.
    ///
    /// Returns `Error::InvalidSampleRate` if `sample_rate < MIN_SAMPLE_RATE` or
    /// `sample_rate > MAX_SAMPLE_RATE`.
    ///
    /// `layout` — channel configuration with per‑channel weights.
    ///
    /// Allocates all of the meter's state up front, whatever the measurement
    /// length: 640 000 bytes of loudness histograms plus 3.4 s of `f64`s at
    /// `sample_rate` for the sliding-window maxima. Pushing and finishing
    /// allocate nothing.
    pub fn new(sample_rate: u32, layout: ChannelLayout) -> Result<Self, crate::Error> {
        if !(MIN_SAMPLE_RATE..=MAX_SAMPLE_RATE).contains(&sample_rate) {
            return Err(crate::Error::InvalidSampleRate { got: sample_rate });
        }
        let (stage1, stage2) = k_weighting_coeffs(sample_rate)?;
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
            gating_blocks: LoudnessHistogram::new(GateComparison::Above),
            short_term_blocks: LoudnessHistogram::new(GateComparison::AtOrAbove),
            momentary_max: SlidingWindowMax::new(momentary_window_samples),
            short_term_max: SlidingWindowMax::new(short_term_window_samples),
            integrated: f64::NEG_INFINITY,
            lra: 0.0,
            max_momentary: f64::NEG_INFINITY,
            max_short_term: f64::NEG_INFINITY,
            finished: false,
            frame_count: 0,
            max_sample_magnitude: max_sample_magnitude(layout),
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

        if self.sub_block_ring.len() == SHORT_TERM_SUB_BLOCKS {
            self.sub_block_ring.pop_front();
        }
        self.sub_block_ring.push_back(mean_sq);

        let n = self.sub_block_ring.len();
        if n >= MOMENTARY_SUB_BLOCKS {
            let sum: f64 = self
                .sub_block_ring
                .iter()
                .rev()
                .take(MOMENTARY_SUB_BLOCKS)
                .sum();
            self.gating_blocks.record(sum / MOMENTARY_SUB_BLOCKS as f64);
        }
        if n >= SHORT_TERM_SUB_BLOCKS {
            let sum: f64 = self.sub_block_ring.iter().sum();
            self.short_term_blocks
                .record(sum / SHORT_TERM_SUB_BLOCKS as f64);
        }
    }

    /// Largest sample magnitude this meter accepts.
    ///
    /// The histograms span −70 to +30 LKFS, so a sample whose magnitude could
    /// carry a block to or above +30 LKFS is rejected with
    /// [`crate::Error::SampleTooLoud`]: about +19.8 dBFS for a mono
    /// layout, +16.8 dBFS for stereo, +12.2 dBFS for 5.1, given by the
    /// K‑weighting filter's largest gain over the accepted sample rates and
    /// the sum of the layout's channel weights.
    #[must_use]
    pub fn max_sample_magnitude(&self) -> f64 {
        self.max_sample_magnitude
    }

    fn check_sample(&self, index: usize, channel: usize, value: f64) -> Result<(), crate::Error> {
        if !value.is_finite() {
            return Err(crate::Error::NonFiniteSample {
                index,
                channel,
                value,
            });
        }
        if libm::fabs(value) > self.max_sample_magnitude {
            return Err(crate::Error::SampleTooLoud {
                index,
                channel,
                value,
                max: self.max_sample_magnitude,
            });
        }
        Ok(())
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
            self.check_sample(self.frame_count, i, sample_f64)?;
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
            self.check_sample(self.frame_count, i, sample)?;
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
            self.check_sample(self.frame_count + frame_idx, 0, l_f64)?;
            self.check_sample(self.frame_count + frame_idx, 1, r_f64)?;
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

        self.integrated = self.gating_blocks.integrated();
        self.lra = self.short_term_blocks.loudness_range();

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

            assert_eq!(meter.gating_blocks.bins.len(), super::HISTOGRAM_BINS);
            assert_eq!(meter.short_term_blocks.bins.len(), super::HISTOGRAM_BINS);
        }
    }

    mod gate_edge {
        use super::super::{
            BIN_WIDTH_LU, LRA_HIGH_PERCENT, LRA_LOW_PERCENT, LRA_RELATIVE_GATE, LoudnessHistogram,
            LoudnessMeter, RELATIVE_GATE, SUB_BLOCK_S, lkfs_to_mean_sq, mean_sq_to_lkfs,
        };
        use crate::channel_layout::ChannelLayout;
        use alloc::vec::Vec;

        const RATE: u32 = crate::filter::MIN_SAMPLE_RATE;
        const SEGMENT_S: f64 = 10.0;
        const LOUD_DBFS: f64 = -6.0;
        const STEP_DB: f64 = 0.0001;
        const STEPS_BELOW: i32 = 110;
        const STEPS_ABOVE: i32 = 10;

        /// 997 Hz: block loudness wanders over a few thousandths of an LU
        /// at this rate. The aligned tone repeats exactly every sub-block, so
        /// all its steady blocks share one loudness.
        #[derive(Clone, Copy)]
        enum Tone {
            Wandering,
            Aligned,
        }

        impl Tone {
            fn hz(self) -> f64 {
                const WANDERING_HZ: f64 = 997.0;
                const ALIGNED_CYCLES_PER_SUB_BLOCK: f64 = 100.0;
                match self {
                    Self::Wandering => WANDERING_HZ,
                    Self::Aligned => {
                        let sub_block = (SUB_BLOCK_S * f64::from(RATE)) as usize as f64;
                        ALIGNED_CYCLES_PER_SUB_BLOCK * f64::from(RATE) / sub_block
                    }
                }
            }
        }

        fn meter_for(tone: Tone, quiet_dbfs: f64) -> LoudnessMeter {
            let mut m = LoudnessMeter::new(RATE, ChannelLayout::Mono).unwrap();
            let n = (SEGMENT_S * f64::from(RATE)) as usize;
            for (dbfs, offset) in [(LOUD_DBFS, 0), (quiet_dbfs, n)] {
                let amp = libm::pow(10.0, dbfs / 20.0);
                for i in 0..n {
                    let t = (offset + i) as f64 / f64::from(RATE);
                    m.push_f64(&[amp * libm::sin(2.0 * core::f64::consts::PI * tone.hz() * t)])
                        .unwrap();
                }
            }
            m.finish();
            m
        }

        fn threshold(blocks: &[f64], gate_lu: f64) -> f64 {
            let energy: f64 = blocks.iter().map(|&l| lkfs_to_mean_sq(l)).sum();
            mean_sq_to_lkfs(energy / blocks.len() as f64) + gate_lu
        }

        fn integrated_of(kept: Vec<f64>) -> f64 {
            let energy: f64 = kept.iter().map(|&l| lkfs_to_mean_sq(l)).sum();
            mean_sq_to_lkfs(energy / kept.len() as f64)
        }

        fn lra_of(mut kept: Vec<f64>) -> f64 {
            kept.sort_by(f64::total_cmp);
            let n = kept.len();
            let at = |p: f64| kept[(libm::round((n as f64 - 1.0) * p / 100.0 + 1.0) as usize) - 1];
            at(LRA_HIGH_PERCENT) - at(LRA_LOW_PERCENT)
        }

        struct Gate {
            histogram: fn(&LoudnessMeter) -> &LoudnessHistogram,
            result: fn(&LoudnessMeter) -> f64,
            reference: fn(Vec<f64>) -> f64,
            gate_lu: f64,
            tolerance: f64,
        }

        const INTEGRATED: Gate = Gate {
            histogram: |m| &m.gating_blocks,
            result: LoudnessMeter::integrated_lufs,
            reference: integrated_of,
            gate_lu: RELATIVE_GATE,
            tolerance: 1e-9,
        };

        const LRA: Gate = Gate {
            histogram: |m| &m.short_term_blocks,
            result: LoudnessMeter::loudness_range,
            reference: lra_of,
            gate_lu: LRA_RELATIVE_GATE,
            tolerance: 2.0 * BIN_WIDTH_LU,
        };

        impl Gate {
            fn reference_with(&self, h: &LoudnessHistogram, in_bin: Option<bool>) -> f64 {
                let thr = threshold(&h.recorded, self.gate_lu);
                let at = LoudnessHistogram::bin_of(thr);
                let keep = |l: f64| match in_bin {
                    Some(forced) if LoudnessHistogram::bin_of(l) == at => forced,
                    _ => h.gate.passes(l, thr),
                };
                (self.reference)(h.recorded.iter().copied().filter(|&l| keep(l)).collect())
            }

            fn mostly_kept(&self, tone: Tone, quiet_dbfs: f64) -> bool {
                let m = meter_for(tone, quiet_dbfs);
                let h = (self.histogram)(&m);
                let thr = threshold(&h.recorded, self.gate_lu);
                let kept = h
                    .recorded
                    .iter()
                    .filter(|&&l| h.gate.passes(l, thr))
                    .count();
                4 * kept > 3 * h.recorded.len()
            }

            fn edge(&self, tone: Tone) -> f64 {
                let (mut lo, mut hi) = (self.gate_lu - 25.0, self.gate_lu + 5.0);
                assert!(!self.mostly_kept(tone, lo) && self.mostly_kept(tone, hi));
                while hi - lo > STEP_DB / 4.0 {
                    let mid = 0.5 * (lo + hi);
                    if self.mostly_kept(tone, mid) {
                        hi = mid;
                    } else {
                        lo = mid;
                    }
                }
                hi
            }

            /// Steps the quiet level down across the gate and checks every
            /// step against the exact block-list reference. Returns the
            /// number of steps whose threshold bin held blocks only below the
            /// threshold (the case a bin-resolution gate wrongly admits) and
            /// the number whose threshold bin held blocks on both sides.
            fn sweep(&self, tone: Tone) -> (usize, usize) {
                let jump = self.edge(tone);
                let (mut wholly_below, mut straddling) = (0, 0);
                for k in -STEPS_BELOW..=STEPS_ABOVE {
                    let q = jump + f64::from(k) * STEP_DB;
                    let m = meter_for(tone, q);
                    let h = (self.histogram)(&m);
                    let thr = threshold(&h.recorded, self.gate_lu);
                    let bin = &h.bins[LoudnessHistogram::bin_of(thr)];
                    let got = (self.result)(&m);
                    let want = self.reference_with(h, None);
                    if bin.count > 0 && !h.gate.passes(bin.max_lkfs(), thr) {
                        wholly_below += 1;
                    }
                    if !h.gate.passes(bin.min_lkfs(), thr) && h.gate.passes(bin.max_lkfs(), thr) {
                        straddling += 1;
                        let all_out = self.reference_with(h, Some(false));
                        let all_in = self.reference_with(h, Some(true));
                        assert!(
                            (got - want).abs() <= (all_in - all_out).abs() + self.tolerance,
                            "q = {q}: got {got}, reference {want}, whole-bin range {all_out}..{all_in}"
                        );
                    } else {
                        assert!(
                            (got - want).abs() < self.tolerance,
                            "q = {q}: got {got}, reference {want}"
                        );
                    }
                }
                (wholly_below, straddling)
            }
        }

        fn check(gate: &Gate) {
            let (wholly_below, straddling) = gate.sweep(Tone::Aligned);
            assert!(
                wholly_below > 0,
                "sweep never reached the bin-resolution case"
            );
            assert_eq!(straddling, 0, "aligned tone: every step must be exact");
            let (_, straddling) = gate.sweep(Tone::Wandering);
            assert!(straddling > 0, "sweep never reached a straddling bin");
        }

        #[test]
        fn integrated_gate_matches_the_reference_across_the_edge() {
            check(&INTEGRATED);
        }

        #[test]
        fn lra_gate_matches_the_reference_across_the_edge() {
            check(&LRA);
        }
    }

    #[test]
    fn absolute_gate_keeps_a_block_at_exactly_minus_70_for_lra_only() {
        use super::{
            ABSOLUTE_GATE, GateComparison, LoudnessHistogram, lkfs_to_mean_sq, mean_sq_to_lkfs,
        };
        let estimate = lkfs_to_mean_sq(ABSOLUTE_GATE);
        let at_gate = (-64i64..=64)
            .map(|ulps| f64::from_bits((estimate.to_bits() as i64 + ulps) as u64))
            .find(|&m| mean_sq_to_lkfs(m) == ABSOLUTE_GATE)
            .unwrap();
        let mut lra = LoudnessHistogram::new(GateComparison::AtOrAbove);
        let mut integrated = LoudnessHistogram::new(GateComparison::Above);
        lra.record(at_gate);
        integrated.record(at_gate);
        assert_eq!(lra.totals_from(0).0, 1);
        assert_eq!(integrated.totals_from(0).0, 0);
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
        let (stage1, stage2) = crate::filter::k_weighting_coeffs(48_000).unwrap();

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
