//! K-weighting IIR biquad filter (ITU-R BS.1770-5 §Annex 1, Fig. 3, Tables 1–2).
//!
//! The K-weighting filter is a cascade of two 2nd‑order IIR sections:
//!
//! 1. **Stage 1** — shelving filter modelling the acoustic effects of the head
//!    (a rigid sphere).
//! 2. **Stage 2** — high-pass filter (revised low-frequency B-curve, RLB).
//!
//! BS.1770‑5 Annex 1 tabulates the coefficients **only at 48 kHz** and gives no
//! derivation for other sample rates. Rather than hard-code a single rate, the
//! coefficients here are derived for any sample rate by bilinear-transforming
//! the *analog* prototype filters with frequency pre-warping — the same
//! derivation libebur128 uses. The analog parameters (corner frequencies and
//! quality factors) were chosen so that the 48 kHz bilinear transform
//! reproduces the BS.1770‑5 tabulated coefficients exactly (to within
//! floating‑point epsilon).
//!
//! The filter is applied independently to each input channel before
//! mean‑square computation.
//!
//! ## Filter structure
//!
//! Each stage is a Direct Form I biquad:
//!
//! ```text
//! y[n] = b0·x[n] + b1·x[n-1] + b2·x[n-2] - a1·y[n-1] - a2·y[n-2]
//! ```
//!
//! Coefficients are unquantized (f64), derived via the bilinear transform.

/// Biquad filter state (delay elements).
#[derive(Debug, Clone, Copy)]
pub struct BiquadState {
    x1: f64, // x[n-1]
    x2: f64, // x[n-2]
    y1: f64, // y[n-1]
    y2: f64, // y[n-2]
}

impl BiquadState {
    /// Create a zero‑initialized biquad state.
    #[must_use]
    pub fn new() -> Self {
        Self {
            x1: 0.0,
            x2: 0.0,
            y1: 0.0,
            y2: 0.0,
        }
    }
}

impl Default for BiquadState {
    fn default() -> Self {
        Self::new()
    }
}

/// Coefficients for a 2nd‑order IIR section (Direct Form I).
#[derive(Debug, Clone, Copy)]
pub struct BiquadCoeffs {
    /// Feedforward coefficient for the current sample.
    pub b0: f64,
    /// Feedforward coefficient for x[n-1].
    pub b1: f64,
    /// Feedforward coefficient for x[n-2].
    pub b2: f64,
    /// Feedback coefficient for y[n-1].
    pub a1: f64,
    /// Feedback coefficient for y[n-2].
    pub a2: f64,
}

/// Apply one sample through a biquad filter.
///
/// `y[n] = b0·x + b1·x1 + b2·x2 - a1·y1 - a2·y2`
#[inline]
pub fn apply_biquad(x: f64, coeffs: &BiquadCoeffs, state: &mut BiquadState) -> f64 {
    let y = coeffs.b0 * x + coeffs.b1 * state.x1 + coeffs.b2 * state.x2
        - coeffs.a1 * state.y1
        - coeffs.a2 * state.y2;
    state.x2 = state.x1;
    state.x1 = x;
    state.y2 = state.y1;
    state.y1 = y;
    y
}

/// Derive the two K‑weighting stage coefficients for a given sample rate.
///
/// Returns `(stage1_shelving, stage2_highpass)`.
///
/// Each stage is produced by bilinear-transforming an analog prototype with
/// frequency pre‑warping. At `sample_rate == 48_000` the result matches the
/// ITU‑R BS.1770‑5 Annex 1 tabulated coefficients to within floating‑point
/// epsilon. Other rates use the same derivation (the approach in libebur128).
///
/// # Panics
///
/// Panics if `sample_rate == 0` (the caller validates the rate before calling).
///
/// ## Stage 1 — head‑related high‑shelf filter
///
/// Analog prototype: high shelf, ≈ +4 dB gain, corner 1681.974450955533 Hz,
/// Q = 0.7071752369554196.
///
/// ## Stage 2 — RLB high‑pass filter
///
/// Analog prototype: 2nd‑order Butterworth high‑pass, corner
/// 38.13547087602444 Hz, Q = 0.5003270373238773.
#[must_use]
pub fn k_weighting_coeffs(sample_rate: u32) -> (BiquadCoeffs, BiquadCoeffs) {
    // --- Stage 1: head-related high-shelf filter ---
    // Gain in dB (libebur128: 3.999843853973347, ≈ +4 dB). The exact constant
    // reproduces the BS.1770-5 tabulated coefficients at 48 kHz exactly.
    const SHELF_GAIN_DB: f64 = 3.999_843_853_973_347;
    const SHELF_FC: f64 = 1_681.974_450_955_533;
    const SHELF_Q: f64 = 0.707_175_236_955_419_6;
    // libebur128 uses the exact exponent Vb = Vh^0.4996667741545416,
    // not sqrt(Vh).
    const SHELF_VB_EXP: f64 = 0.499_666_774_154_541_6;

    let vh = libm::pow(10.0, SHELF_GAIN_DB / 20.0);
    let vb = libm::pow(vh, SHELF_VB_EXP);
    let k = libm::tan(core::f64::consts::PI * SHELF_FC / sample_rate as f64);
    let a0 = 1.0 + k / SHELF_Q + k * k;
    let stage1 = BiquadCoeffs {
        b0: (vh + vb * (k / SHELF_Q) + k * k) / a0,
        b1: (2.0 * (k * k - vh)) / a0,
        b2: (vh - vb * (k / SHELF_Q) + k * k) / a0,
        a1: (2.0 * (k * k - 1.0)) / a0,
        a2: (1.0 - k / SHELF_Q + k * k) / a0,
    };

    // --- Stage 2: RLB high-pass filter ---
    const HIGH_PASS_FC: f64 = 38.135_470_876_024_44;
    const HIGH_PASS_Q: f64 = 0.500_327_037_323_877_3;

    let k = libm::tan(core::f64::consts::PI * HIGH_PASS_FC / sample_rate as f64);
    let a0 = 1.0 + k / HIGH_PASS_Q + k * k;
    let stage2 = BiquadCoeffs {
        // libebur128 keeps the canonical `1 - 2z^-1 + z^-2` numerator (not
        // normalized by a0); only the denominator is normalized. This matches
        // the BS.1770-5 tabulated values exactly.
        b0: 1.0,
        b1: -2.0,
        b2: 1.0,
        a1: (2.0 * (k * k - 1.0)) / a0,
        a2: (1.0 - k / HIGH_PASS_Q + k * k) / a0,
    };

    (stage1, stage2)
}
