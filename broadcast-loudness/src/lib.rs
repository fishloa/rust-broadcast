#![cfg_attr(not(feature = "std"), no_std)]
#![cfg_attr(docsrs, feature(doc_cfg))]
#![warn(missing_docs)]
#![doc = "\n## Runnable examples\n"]
#![doc = "Run with `cargo run -p broadcast-loudness --example <name>`.\n"]
#![doc = "\n### `measure_loudness`\n\n```rust,ignore"]
#![doc = include_str!("../examples/measure_loudness.rs")]
#![doc = "```\n\n### `measure_true_peak`\n\n```rust,ignore"]
#![doc = include_str!("../examples/measure_true_peak.rs")]
#![doc = "```"]

extern crate alloc;

mod channel_layout;
mod error;
mod filter;
mod loudness_meter;
mod profile;
mod true_peak;

pub use channel_layout::ChannelLayout;
pub use error::{Error, Result};
pub use loudness_meter::LoudnessMeter;
pub use profile::{
    ACCEPTABLE_MAX_RATIO, CHINESE_PROFILES, GY_T_282_2014, GY_T_377_2023_NOISY,
    GY_T_377_2023_QUIET, LoudnessProfile, MONITOR_OVER_LIMIT_LKFS, MonitorUnit, MonitorVerdict,
    OVER_LIMIT_MAX_RATIO, ProfileReport, classify_ratio, classify_units,
    consecutive_over_limit_ratio, over_limit_ratio,
};
pub use true_peak::TruePeakMeter;

pub mod kfilter {
    //! K-weighting IIR filter coefficients + state (ITU-R BS.1770-5 §Annex 1).
    //!
    //! [`k_weighting_coeffs`] derives the two stage coefficients for any
    //! sample rate via a bilinear transform; at 48 kHz they match the
    //! BS.1770-5 Annex 1 tabulated coefficients. The [`crate::LoudnessMeter`]
    //! applies them internally; re-exported here for consumers who need the
    //! raw filter (e.g. visualising the frequency response).

    pub use crate::filter::{BiquadCoeffs, BiquadState, apply_biquad, k_weighting_coeffs};
}
