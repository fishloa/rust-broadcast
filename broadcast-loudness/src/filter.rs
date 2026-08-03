//! K-weighting IIR biquad filter (ITU-R BS.1770-5 §Annex 1, Fig. 3, Tables 1–2).
//!
//! The K-weighting filter is a cascade of two 2nd‑order IIR sections:
//!
//! 1. **Stage 1** — shelving filter modelling the acoustic effects of the head
//!    (a rigid sphere), coefficients from Table 1.
//! 2. **Stage 2** — high-pass filter (revised low-frequency B-curve, RLB),
//!    coefficients from Table 2.
//!
//! Both are specified at 48 kHz. The filter is applied independently to each
//! input channel before mean‑square computation.
//!
//! ## Filter structure
//!
//! Each stage is a Direct Form I biquad:
//!
//! ```text
//! y[n] = b0·x[n] + b1·x[n-1] + b2·x[n-2] - a1·y[n-1] - a2·y[n-2]
//! ```
//!
//! Coefficients from Tables 1 and 2 of BS.1770‑5, unquantized (f64).

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
    pub b0: f64,
    pub b1: f64,
    pub b2: f64,
    pub a1: f64,
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

/// Stage 1 shelving filter coefficients (BS.1770-5 Table 1, 48 kHz).
///
/// Models the acoustic effects of the head as a rigid sphere.
#[must_use]
pub fn shelving_coeffs() -> BiquadCoeffs {
    BiquadCoeffs {
        b0: 1.535_124_859_586_97,
        b1: -2.691_696_189_406_38,
        b2: 1.198_392_810_852_85,
        a1: -1.690_659_293_182_41,
        a2: 0.732_480_774_215_85,
    }
}

/// Stage 2 high-pass filter coefficients (BS.1770-5 Table 2, 48 kHz).
///
/// Revised low-frequency B-curve (RLB) weighting.
#[must_use]
pub fn high_pass_coeffs() -> BiquadCoeffs {
    BiquadCoeffs {
        b0: 1.0,
        b1: -2.0,
        b2: 1.0,
        a1: -1.990_047_454_833_98,
        a2: 0.990_072_250_366_21,
    }
}
