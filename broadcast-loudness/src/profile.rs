//! Chinese loudness target profiles and monitoring helpers (NRTA GY/T 282-2014,
//! GY/T 377-2023, GY/T 379-2023).
//!
//! A [`LoudnessProfile`] is a named target: programme-average loudness
//! `target_lkfs` with a symmetric `tolerance_lu`, and a maximum true-peak level
//! `max_true_peak_dbtp`. [`LoudnessProfile::check`] / [`LoudnessProfile::check_meters`]
//! compare a measurement against it.
//!
//! | Profile | Standard | Target | Tolerance | Max true peak |
//! |---|---|---|---|---|
//! | [`GY_T_282_2014`] | GY/T 282-2014 clauses 4.3, 4.4, 4.6 (digital TV programmes) | -24 LKFS | +-2 LU | -2 dBTP |
//! | [`GY_T_377_2023_QUIET`] | GY/T 377-2023 clause 5.1 a), c) (online programmes, quiet environments) | -24 LKFS | +-2 LU | -1 dBTP |
//! | [`GY_T_377_2023_NOISY`] | GY/T 377-2023 clause 5.1 a), c) (online programmes, noisy environments) | -15 LKFS | +-2 LU | -1 dBTP |
//!
//! The standards measure the **whole-programme** average over all channels per
//! GY/T 262-2012 (ITU-R BS.1770-2 with its gating); this crate's
//! [`LoudnessMeter`](crate::LoudnessMeter) implements BS.1770-5 gating and K-weighting
//! (the standards state a 2 dB measurement-device tolerance as acceptable), so use
//! the integrated value as the "average loudness". LKFS and LUFS are the same
//! unit.
//!
//! GY/T 379-2023 channel monitoring (clauses 9 and 10): programme units of
//! 10 minutes or 3 seconds are measured; a unit is *over limit* when its average
//! loudness exceeds [`MONITOR_OVER_LIMIT_LKFS`] (-22 LKFS, the upper edge of
//! -24 +-2). The monthly share of over-limit units is `M10` (10-minute units) or
//! `S3` (3-second units; only units that are part of a run of two or more
//! consecutive over-limit units count), and Table 1 classifies it as
//! [`MonitorVerdict::Acceptable`] (<= 20%), [`MonitorVerdict::OverLimit`]
//! (<= 40%) or [`MonitorVerdict::SeverelyOverLimit`] (> 40%).

use crate::LoudnessMeter;

/// A named loudness target (see the module table).
#[derive(Debug, Clone, Copy, PartialEq)]
#[non_exhaustive]
pub struct LoudnessProfile {
    /// Short profile name.
    pub name: &'static str,
    /// Defining standard and clauses.
    pub standard: &'static str,
    /// Target programme-average loudness in LKFS.
    pub target_lkfs: f64,
    /// Tolerance around the target in LU (symmetric, inclusive).
    pub tolerance_lu: f64,
    /// Maximum permitted true-peak level in dBTP (inclusive).
    pub max_true_peak_dbtp: f64,
}

/// GY/T 282-2014 (digital TV programmes): -24 LKFS +-2 LU, true peak <= -2 dBTP
/// (clauses 4.3, 4.4, 4.6).
pub const GY_T_282_2014: LoudnessProfile = LoudnessProfile {
    name: "GY/T 282-2014",
    standard: "GY/T 282-2014 clauses 4.3, 4.4, 4.6",
    target_lkfs: -24.0,
    tolerance_lu: 2.0,
    max_true_peak_dbtp: -2.0,
};

/// GY/T 377-2023 quiet-environment version of an online programme: -24 LKFS
/// +-2 LU, true peak <= -1 dBTP (clause 5.1 a), c)).
pub const GY_T_377_2023_QUIET: LoudnessProfile = LoudnessProfile {
    name: "GY/T 377-2023 (-24 LKFS)",
    standard: "GY/T 377-2023 clause 5.1 a), c)",
    target_lkfs: -24.0,
    tolerance_lu: 2.0,
    max_true_peak_dbtp: -1.0,
};

/// GY/T 377-2023 noisy-environment version of an online programme: -15 LKFS
/// +-2 LU, true peak <= -1 dBTP (clause 5.1 a), c)).
pub const GY_T_377_2023_NOISY: LoudnessProfile = LoudnessProfile {
    name: "GY/T 377-2023 (-15 LKFS)",
    standard: "GY/T 377-2023 clause 5.1 a), c)",
    target_lkfs: -15.0,
    tolerance_lu: 2.0,
    max_true_peak_dbtp: -1.0,
};

/// Every Chinese profile in this module.
pub const CHINESE_PROFILES: &[LoudnessProfile] =
    &[GY_T_282_2014, GY_T_377_2023_QUIET, GY_T_377_2023_NOISY];

/// Result of comparing a measurement with a [`LoudnessProfile`].
#[derive(Debug, Clone, Copy, PartialEq)]
#[non_exhaustive]
pub struct ProfileReport {
    /// Measured loudness minus the target, in LU (positive = louder).
    pub deviation_lu: f64,
    /// Whether the loudness is within `target +- tolerance` (inclusive).
    pub loudness_ok: bool,
    /// Limit minus the measured true peak, in dB (negative = over the limit).
    pub true_peak_headroom_db: f64,
    /// Whether the true peak is at or below the limit.
    pub true_peak_ok: bool,
}

impl ProfileReport {
    /// Both the loudness and the true-peak requirement are met.
    #[must_use]
    pub fn is_compliant(&self) -> bool {
        self.loudness_ok && self.true_peak_ok
    }
}

impl LoudnessProfile {
    /// Compare a programme-average loudness (LKFS/LUFS) and a maximum true-peak
    /// level (dBTP) with this profile. A non-finite measurement (NaN, or the
    /// -infinity of a silent programme) never complies for loudness; a true
    /// peak of -infinity (silence) complies.
    #[must_use]
    pub fn check(&self, integrated_lkfs: f64, true_peak_dbtp: f64) -> ProfileReport {
        let deviation_lu = integrated_lkfs - self.target_lkfs;
        let loudness_ok = integrated_lkfs.is_finite() && deviation_lu.abs() <= self.tolerance_lu;
        let true_peak_ok = !true_peak_dbtp.is_nan() && true_peak_dbtp <= self.max_true_peak_dbtp;
        ProfileReport {
            deviation_lu,
            loudness_ok,
            true_peak_headroom_db: self.max_true_peak_dbtp - true_peak_dbtp,
            true_peak_ok,
        }
    }

    /// [`LoudnessProfile::check`] over a finished [`LoudnessMeter`] and the
    /// result of a true-peak meter (`TruePeakMeter::finish`, dBTP).
    #[must_use]
    pub fn check_meters(&self, loudness: &LoudnessMeter, true_peak_dbtp: f64) -> ProfileReport {
        self.check(loudness.integrated_lufs(), true_peak_dbtp)
    }
}

/// GY/T 379-2023 clause 10 (and 9.2): the loudness above which a monitoring
/// unit is *over limit* (the upper edge of -24 +-2 LKFS).
pub const MONITOR_OVER_LIMIT_LKFS: f64 = -22.0;
/// GY/T 379-2023 Table 1: largest share of over-limit units that is *acceptable*.
pub const ACCEPTABLE_MAX_RATIO: f64 = 0.20;
/// GY/T 379-2023 Table 1: largest share of over-limit units that is merely
/// *over limit* (above it: severely over limit).
pub const OVER_LIMIT_MAX_RATIO: f64 = 0.40;

/// A monitoring unit length of a channel without EPG (GY/T 379-2023 clause 10 c).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum MonitorUnit {
    /// 10-minute units; the over-limit share is `M10`.
    TenMinutes,
    /// 3-second units; the over-limit share is `S3`.
    ThreeSeconds,
}

impl MonitorUnit {
    /// Unit length in seconds.
    #[must_use]
    pub const fn seconds(self) -> u32 {
        match self {
            Self::TenMinutes => 600,
            Self::ThreeSeconds => 3,
        }
    }

    /// Spec label.
    #[must_use]
    pub fn name(&self) -> &'static str {
        match self {
            Self::TenMinutes => "10 min (M10)",
            Self::ThreeSeconds => "3 s (S3)",
        }
    }
}
broadcast_common::impl_spec_display!(MonitorUnit);

/// GY/T 379-2023 Table 1 verdict for a channel's monthly over-limit share.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum MonitorVerdict {
    /// Share <= 20%.
    Acceptable,
    /// 20% < share <= 40%.
    OverLimit,
    /// Share > 40%.
    SeverelyOverLimit,
}

impl MonitorVerdict {
    /// Spec label.
    #[must_use]
    pub fn name(&self) -> &'static str {
        match self {
            Self::Acceptable => "acceptable",
            Self::OverLimit => "over limit",
            Self::SeverelyOverLimit => "severely over limit",
        }
    }
}
broadcast_common::impl_spec_display!(MonitorVerdict);

/// Classify an over-limit share (0.0..=1.0) per GY/T 379-2023 Table 1 (the same
/// 20% / 40% thresholds apply to `M10` and `S3`). A NaN share is treated as
/// severely over limit.
#[must_use]
pub fn classify_ratio(ratio: f64) -> MonitorVerdict {
    if ratio <= ACCEPTABLE_MAX_RATIO {
        MonitorVerdict::Acceptable
    } else if ratio <= OVER_LIMIT_MAX_RATIO {
        MonitorVerdict::OverLimit
    } else {
        MonitorVerdict::SeverelyOverLimit
    }
}

fn is_over(lkfs: f64) -> bool {
    lkfs > MONITOR_OVER_LIMIT_LKFS
}

/// `M10`: the share of units whose average loudness exceeds
/// [`MONITOR_OVER_LIMIT_LKFS`] (clause 10 d 2). `None` for no units.
#[must_use]
pub fn over_limit_ratio(unit_lkfs: &[f64]) -> Option<f64> {
    if unit_lkfs.is_empty() {
        return None;
    }
    let over = unit_lkfs.iter().filter(|&&l| is_over(l)).count();
    Some(over as f64 / unit_lkfs.len() as f64)
}

/// `S3`: the share of units that belong to a run of two or more consecutive
/// over-limit units (clause 10 d 2), over all units. `None` for no units.
#[must_use]
pub fn consecutive_over_limit_ratio(unit_lkfs: &[f64]) -> Option<f64> {
    if unit_lkfs.is_empty() {
        return None;
    }
    let mut counted = 0usize;
    let mut run = 0usize;
    for &l in unit_lkfs {
        if is_over(l) {
            run += 1;
        } else {
            if run >= 2 {
                counted += run;
            }
            run = 0;
        }
    }
    if run >= 2 {
        counted += run;
    }
    Some(counted as f64 / unit_lkfs.len() as f64)
}

/// Share and verdict for a channel's units of the given length: `M10`
/// ([`over_limit_ratio`]) for 10-minute units, `S3`
/// ([`consecutive_over_limit_ratio`]) for 3-second units, classified by
/// [`classify_ratio`]. `None` for no units.
#[must_use]
pub fn classify_units(unit: MonitorUnit, unit_lkfs: &[f64]) -> Option<(f64, MonitorVerdict)> {
    let ratio = match unit {
        MonitorUnit::TenMinutes => over_limit_ratio(unit_lkfs),
        MonitorUnit::ThreeSeconds => consecutive_over_limit_ratio(unit_lkfs),
    }?;
    Some((ratio, classify_ratio(ratio)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::string::ToString;
    use alloc::vec;
    use alloc::vec::Vec;

    #[test]
    fn profile_constants_match_the_standards() {
        // GY/T 282-2014 4.3/4.4/4.6
        assert_eq!(
            (
                GY_T_282_2014.target_lkfs,
                GY_T_282_2014.tolerance_lu,
                GY_T_282_2014.max_true_peak_dbtp
            ),
            (-24.0, 2.0, -2.0)
        );
        // GY/T 377-2023 5.1 a), c)
        assert_eq!(
            (
                GY_T_377_2023_QUIET.target_lkfs,
                GY_T_377_2023_QUIET.max_true_peak_dbtp
            ),
            (-24.0, -1.0)
        );
        assert_eq!(
            (
                GY_T_377_2023_NOISY.target_lkfs,
                GY_T_377_2023_NOISY.tolerance_lu,
                GY_T_377_2023_NOISY.max_true_peak_dbtp
            ),
            (-15.0, 2.0, -1.0)
        );
        assert_eq!(CHINESE_PROFILES.len(), 3);
    }

    #[test]
    fn tolerance_edges_are_inclusive() {
        let p = GY_T_282_2014;
        for l in [-26.0, -24.0, -22.0] {
            assert!(p.check(l, -3.0).loudness_ok, "{l}");
        }
        for l in [-26.01, -21.99, f64::NAN, f64::NEG_INFINITY, f64::INFINITY] {
            assert!(!p.check(l, -3.0).loudness_ok, "{l}");
        }
        let r = p.check(-23.0, -3.0);
        assert_eq!(r.deviation_lu, 1.0);
        assert!(r.is_compliant());
    }

    #[test]
    fn true_peak_limits_differ_between_standards() {
        // -1.5 dBTP: over GY/T 282's -2 dBTP, within GY/T 377's -1 dBTP.
        let tp = -1.5;
        assert!(!GY_T_282_2014.check(-24.0, tp).true_peak_ok);
        assert!(GY_T_377_2023_QUIET.check(-24.0, tp).true_peak_ok);
        assert!(
            GY_T_282_2014.check(-24.0, -2.0).true_peak_ok,
            "limit is inclusive"
        );
        let r = GY_T_282_2014.check(-24.0, -0.5);
        assert!(!r.true_peak_ok && !r.is_compliant());
        assert_eq!(r.true_peak_headroom_db, -1.5);
        assert!(
            GY_T_282_2014.check(-24.0, f64::NEG_INFINITY).true_peak_ok,
            "silence"
        );
        assert!(!GY_T_282_2014.check(-24.0, f64::NAN).true_peak_ok);
    }

    #[test]
    fn noisy_and_quiet_online_versions() {
        assert!(GY_T_377_2023_NOISY.check(-15.5, -1.0).is_compliant());
        assert!(!GY_T_377_2023_NOISY.check(-24.0, -1.0).loudness_ok);
        assert!(GY_T_377_2023_QUIET.check(-25.0, -1.0).is_compliant());
        assert!(!GY_T_377_2023_QUIET.check(-15.0, -1.0).loudness_ok);
    }

    /// A 1 kHz full-scale-relative sine whose integrated loudness lands near a
    /// known value: check_meters reads the meter's integrated_lufs().
    #[test]
    fn check_meters_uses_the_integrated_value() {
        use crate::ChannelLayout;
        let mut m = LoudnessMeter::new(48_000, ChannelLayout::Mono).unwrap();
        // -20 dBFS 997 Hz sine mono = about -23.0 LUFS (BS.1770 mono, K-weighting).
        let amp = 10f64.powf(-20.0 / 20.0);
        let samples: Vec<f32> = (0..48_000 * 5)
            .map(|n| {
                (amp * (2.0 * core::f64::consts::PI * 997.0 * n as f64 / 48_000.0).sin()) as f32
            })
            .collect();
        for s in &samples {
            m.push_f32(&[*s]).unwrap();
        }
        m.finish();
        let lufs = m.integrated_lufs();
        assert!(lufs.is_finite() && lufs < -20.0 && lufs > -26.0, "{lufs}");
        let r = GY_T_282_2014.check_meters(&m, -1.0);
        assert!((r.deviation_lu - (lufs + 24.0)).abs() < 1e-9);
        assert!(r.loudness_ok);
        assert!(!r.true_peak_ok);
    }

    /// GY/T 379-2023 Table 1 thresholds: 20% and 40%, boundaries inclusive.
    #[test]
    fn table_1_classification() {
        assert_eq!(classify_ratio(0.0), MonitorVerdict::Acceptable);
        assert_eq!(classify_ratio(0.20), MonitorVerdict::Acceptable);
        assert_eq!(classify_ratio(0.2001), MonitorVerdict::OverLimit);
        assert_eq!(classify_ratio(0.40), MonitorVerdict::OverLimit);
        assert_eq!(classify_ratio(0.4001), MonitorVerdict::SeverelyOverLimit);
        assert_eq!(classify_ratio(1.0), MonitorVerdict::SeverelyOverLimit);
        assert_eq!(classify_ratio(f64::NAN), MonitorVerdict::SeverelyOverLimit);
    }

    /// M10: strictly above -22 LKFS counts; -22.0 itself does not.
    #[test]
    fn m10_counts_units_above_minus_22() {
        let units = [-24.0, -22.0, -21.9, -20.0, -30.0];
        assert_eq!(over_limit_ratio(&units), Some(0.4));
        assert_eq!(over_limit_ratio(&[]), None);
        let (ratio, v) = classify_units(MonitorUnit::TenMinutes, &units).unwrap();
        assert_eq!((ratio, v), (0.4, MonitorVerdict::OverLimit));
    }

    /// S3: only over-limit units in runs of >= 2 consecutive units count.
    #[test]
    fn s3_counts_only_consecutive_runs() {
        // over: 3,4 (run of 2), 7 (isolated), 9,10,11 (run of 3) -> 5 of 12 count
        let mut u = vec![-30.0; 12];
        for i in [3, 4, 7, 9, 10, 11] {
            u[i] = -20.0;
        }
        let r = consecutive_over_limit_ratio(&u).unwrap();
        assert!((r - 5.0 / 12.0).abs() < 1e-12);
        // a run at the very end still counts
        assert_eq!(
            consecutive_over_limit_ratio(&[-30.0, -20.0, -20.0]),
            Some(2.0 / 3.0)
        );
        // all isolated -> 0
        assert_eq!(
            consecutive_over_limit_ratio(&[-20.0, -30.0, -20.0]),
            Some(0.0)
        );
        assert_eq!(consecutive_over_limit_ratio(&[]), None);
        let (ratio, v) = classify_units(MonitorUnit::ThreeSeconds, &u).unwrap();
        assert!((ratio - 5.0 / 12.0).abs() < 1e-12);
        assert_eq!(v, MonitorVerdict::SeverelyOverLimit);
        assert!(classify_units(MonitorUnit::ThreeSeconds, &[]).is_none());
    }

    #[test]
    fn labels_and_unit_lengths() {
        assert_eq!(MonitorUnit::TenMinutes.seconds(), 600);
        assert_eq!(MonitorUnit::ThreeSeconds.seconds(), 3);
        assert_eq!(MonitorUnit::ThreeSeconds.to_string(), "3 s (S3)");
        assert_eq!(
            MonitorVerdict::SeverelyOverLimit.to_string(),
            "severely over limit"
        );
        assert_eq!(
            MONITOR_OVER_LIMIT_LKFS,
            GY_T_282_2014.target_lkfs + GY_T_282_2014.tolerance_lu
        );
    }
}
