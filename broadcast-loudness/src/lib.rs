//! EBU R 128 / ITU-R BS.1770-5 loudness measurement: K-weighting, momentary /
//! short-term / integrated loudness (LUFS), loudness range (LU), true-peak
//! (dBTP), and the Chinese NRTA GY/T loudness target profiles.
//!
//! # What it does / what it does not
//!
//! **Does:** measures PCM the way broadcast loudness standards define it
//! ([`LoudnessMeter`], [`TruePeakMeter`]) and compares a measurement with the
//! GY/T 282 / 377 / 379 targets ([`LoudnessProfile`]).
//!
//! **Does not:** decode or resample audio, normalise audio, implement ATSC A/85,
//! encode the EBU R 128 targets as a profile, or implement GY/T 262-2012 /
//! 263-2012 as such (BS.1770-5 is used; the GY/T standards accept a 2 dB
//! tolerance). There is no wire format, so the workspace round-trip invariant
//! does not apply; the EBU compliance signals are the hard gate.
//!
//! # Standards supported
//!
//! | Standard | Edition | Scope | Status |
//! |---|---|---|---|
//! | ITU-R BS.1770 | BS.1770-5 (11/2023) | K-weighting (Annex 1), channel weights (mono, stereo, 5.1, custom), gating (-70 LUFS absolute, -10 LU relative), true peak by 4x oversampling with a 48-tap polyphase FIR (Annex 2) | Full |
//! | EBU R 128 | v5 (Nov 2023) | momentary, short-term, integrated, LRA, true peak | Partial (targets not encoded) |
//! | EBU Tech 3341 | v4 (Nov 2023) | compliance cases 1-6, 9-12, 15-19 pass | Partial (7, 8, 13, 14 not exercised; 20-23 need EBU reference WAVs) |
//! | EBU Tech 3342 | v4 (Nov 2023) | LRA; cases 1-4 pass | Partial (5-6 need real programme files) |
//! | NRTA GY/T 282-2014 | 2014 | [`GY_T_282_2014`]: -24 LKFS +-2 LU, <= -2 dBTP | Full (constants) |
//! | NRTA GY/T 377-2023 | 2023 | [`GY_T_377_2023_QUIET`] / [`GY_T_377_2023_NOISY`] | Full (constants) |
//! | NRTA GY/T 379-2023 | 2023 | channel monitoring: [`over_limit_ratio`], [`consecutive_over_limit_ratio`], [`classify_units`] | Full (helpers) |
//!
//! # Features
//!
//! | Feature | Default | Effect |
//! |---|---|---|
//! | `std` | yes | Links `std`; off gives `no_std` + `alloc` (math via `libm`) |
//!
//! # Quick start
//!
//! ```rust
//! use broadcast_loudness::{ChannelLayout, GY_T_282_2014, LoudnessMeter, TruePeakMeter};
//!
//! // 20 s of a 1 kHz stereo sine at -23 dBFS (EBU Tech 3341 case 1), 48 kHz.
//! let rate = 48_000_u32;
//! let amplitude = 10.0_f64.powf(-23.0 / 20.0);
//! let sine: Vec<f32> = (0..20 * rate as usize)
//!     .map(|i| {
//!         let t = i as f64 / f64::from(rate);
//!         (amplitude * (2.0 * std::f64::consts::PI * 1000.0 * t).sin()) as f32
//!     })
//!     .collect();
//!
//! // Loudness: planar stereo in, LUFS / LU out.
//! let mut meter = LoudnessMeter::new(rate, ChannelLayout::Stereo).unwrap();
//! meter.push_stereo_planar_f32(&sine, &sine).unwrap();
//! meter.finish();
//! assert!((meter.integrated_lufs() - (-23.0)).abs() <= 0.1);
//!
//! // True peak (one meter per channel), then check against a national target.
//! let mut tp = TruePeakMeter::new();
//! tp.push_f32_slice(&sine).unwrap();
//! let peak_dbtp = tp.finish();
//! assert!(peak_dbtp < -22.0 && peak_dbtp > -24.0);
//!
//! let report = GY_T_282_2014.check_meters(&meter, peak_dbtp);
//! assert!(report.is_compliant()); // -24 LKFS +-2 LU, <= -2 dBTP
//! ```
//!
//! # What is implemented
//!
//! - [`LoudnessMeter`]: `push_f32` / `push_f64` / `push_stereo_planar_f32`,
//!   `finish`, then `integrated_lufs`, `max_momentary_lufs`,
//!   `max_short_term_lufs`, `loudness_range`.
//! - [`TruePeakMeter`]: per channel; `finish` / `current_level`.
//! - [`ChannelLayout`]: `Mono`, `Stereo`, `Surround51`, `Custom`.
//! - [`kfilter`]: the raw K-weighting biquads.
//! - Profile items re-exported at the root: [`LoudnessProfile`],
//!   [`ProfileReport`], [`MonitorUnit`], [`MonitorVerdict`], [`classify_ratio`].
//!
//! # Design notes
//!
//! Streaming and bounded memory (30 minutes of audio under a 16 MiB cap);
//! non-finite input rejected ([`Error::NonFiniteSample`]); `no_std` + `alloc`;
//! depends only on `broadcast-common`.
//!
//! # Related crates
//!
//! Leaf crate over `broadcast-common`. MSRV 1.95.0; MIT OR Apache-2.0.
//!
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
