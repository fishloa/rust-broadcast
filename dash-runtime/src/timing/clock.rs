//! The UTC clock: an anchor pair mapping the caller's monotonic microseconds
//! to UTC, with a trust-ordered replacement rule.
//!
//! `docs/dash-runtime/DESIGN.md` 3.2/3.3. Pure: the clock never reads a time
//! source, it only maps instants it is given.

use broadcast_common::impl_spec_display;

use super::utc_timing::UtcSample;

/// Where an anchor came from, ordered by trust (lowest first).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[non_exhaustive]
pub enum ClockSource {
    /// The `Date` of the MPD response.
    MpdDate,
    /// The host's own wall clock.
    Host,
    /// A `UTCTiming` measurement.
    UtcTiming,
}

impl ClockSource {
    /// Stable snake_case label.
    #[must_use]
    pub fn name(&self) -> &'static str {
        match self {
            Self::MpdDate => "mpd_date",
            Self::Host => "host",
            Self::UtcTiming => "utc_timing",
        }
    }
}
impl_spec_display!(ClockSource);

/// One anchor: at monotonic `mono_us` UTC was `utc_us`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub struct Anchor {
    /// Monotonic microseconds.
    pub mono_us: u64,
    /// Unix microseconds.
    pub utc_us: i64,
    /// Where it came from.
    pub source: ClockSource,
    /// Half-width of the error bar.
    pub uncertainty_us: u64,
}

impl Anchor {
    /// Anchor from a measured sample.
    #[must_use]
    pub fn from_sample(sample: UtcSample, source: ClockSource) -> Self {
        Self {
            mono_us: sample.mono_us,
            utc_us: sample.utc_us,
            source,
            uncertainty_us: sample.uncertainty_us,
        }
    }

    /// UTC at `now_us`, saturating at the `i64` range.
    #[must_use]
    pub fn utc_at(&self, now_us: u64) -> i64 {
        let delta = i128::from(now_us) - i128::from(self.mono_us);
        saturate(i128::from(self.utc_us) + delta)
    }

    /// The monotonic instant at which UTC reaches `utc_us`, saturating at
    /// `0..=u64::MAX`.
    #[must_use]
    pub fn mono_for_utc(&self, utc_us: i64) -> u64 {
        let delta = i128::from(utc_us) - i128::from(self.utc_us);
        let mono = i128::from(self.mono_us) + delta;
        u64::try_from(mono.clamp(0, i128::from(u64::MAX))).unwrap_or(u64::MAX)
    }
}

fn saturate(v: i128) -> i64 {
    i64::try_from(v.clamp(i128::from(i64::MIN), i128::from(i64::MAX))).unwrap_or(i64::MAX)
}

/// Why an offered sample replaced the anchor.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum AdoptReason {
    /// There was no anchor.
    First,
    /// The source is more trusted than the current one.
    HigherTrust,
    /// Same source, at least as precise.
    Refined,
    /// Same source, the old anchor was older than the resync period.
    Stale,
    /// Same source, the time disagrees with the prediction by more than both
    /// error bars: a clock step.
    Step,
    /// A less trusted source, allowed because the better one failed.
    Downgrade,
}

impl AdoptReason {
    /// Stable snake_case label.
    #[must_use]
    pub fn name(&self) -> &'static str {
        match self {
            Self::First => "first",
            Self::HigherTrust => "higher_trust",
            Self::Refined => "refined",
            Self::Stale => "stale",
            Self::Step => "step",
            Self::Downgrade => "downgrade",
        }
    }
}
impl_spec_display!(AdoptReason);

/// Why an offered sample was not adopted.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum RejectReason {
    /// A more trusted anchor exists.
    LowerTrust,
    /// Same source but less precise and not stale.
    LessPrecise,
}

impl RejectReason {
    /// Stable snake_case label.
    #[must_use]
    pub fn name(&self) -> &'static str {
        match self {
            Self::LowerTrust => "lower_trust",
            Self::LessPrecise => "less_precise",
        }
    }
}
impl_spec_display!(RejectReason);

/// The result of [`UtcClock::offer`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum OfferOutcome {
    /// The sample is now the anchor.
    Adopted(AdoptReason),
    /// The existing anchor was kept.
    Rejected(RejectReason),
}

/// Parameters of the replacement rule.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub struct OfferPolicy {
    /// An anchor older than this is replaced by any same-source sample.
    pub resync_us: u64,
    /// Let a less trusted source replace a more trusted one (the better one
    /// has failed for the configured wait).
    pub allow_downgrade: bool,
}

impl OfferPolicy {
    /// A policy.
    #[must_use]
    pub fn new(resync_us: u64, allow_downgrade: bool) -> Self {
        Self {
            resync_us,
            allow_downgrade,
        }
    }
}

/// The clock: at most one anchor.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct UtcClock {
    anchor: Option<Anchor>,
}

impl UtcClock {
    /// A clock with no anchor.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// The current anchor.
    #[must_use]
    pub fn anchor(&self) -> Option<&Anchor> {
        self.anchor.as_ref()
    }

    /// UTC at `now_us`, or `None` before the first anchor.
    #[must_use]
    pub fn utc_at(&self, now_us: u64) -> Option<i64> {
        self.anchor.map(|a| a.utc_at(now_us))
    }

    /// The monotonic instant at which UTC reaches `utc_us`.
    #[must_use]
    pub fn mono_for_utc(&self, utc_us: i64) -> Option<u64> {
        self.anchor.map(|a| a.mono_for_utc(utc_us))
    }

    /// Offer a measurement; the anchor changes per the replacement rule.
    ///
    /// A more trusted source replaces at once; a less trusted one only when
    /// `policy.allow_downgrade`. For the same source: a reading that disagrees
    /// with the prediction by more than the two error bars together is a clock
    /// step and is adopted immediately; otherwise it replaces the anchor when
    /// it is at least as precise or the anchor is older than
    /// `policy.resync_us`.
    pub fn offer(
        &mut self,
        sample: UtcSample,
        source: ClockSource,
        now_us: u64,
        policy: OfferPolicy,
    ) -> OfferOutcome {
        let new = Anchor::from_sample(sample, source);
        let Some(old) = self.anchor else {
            self.anchor = Some(new);
            return OfferOutcome::Adopted(AdoptReason::First);
        };
        let reason = if new.source > old.source {
            AdoptReason::HigherTrust
        } else if new.source < old.source {
            if policy.allow_downgrade {
                AdoptReason::Downgrade
            } else {
                return OfferOutcome::Rejected(RejectReason::LowerTrust);
            }
        } else {
            let predicted = i128::from(old.utc_at(new.mono_us));
            let diff = (i128::from(new.utc_us) - predicted).unsigned_abs();
            let bars = u128::from(old.uncertainty_us) + u128::from(new.uncertainty_us);
            if diff > bars {
                AdoptReason::Step
            } else if now_us.saturating_sub(old.mono_us) >= policy.resync_us {
                AdoptReason::Stale
            } else if new.uncertainty_us <= old.uncertainty_us {
                AdoptReason::Refined
            } else {
                return OfferOutcome::Rejected(RejectReason::LessPrecise);
            }
        };
        self.anchor = Some(new);
        OfferOutcome::Adopted(reason)
    }
}
