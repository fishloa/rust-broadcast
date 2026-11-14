//! The UTC clock: an anchor pair mapping the caller's monotonic microseconds
//! to UTC, with a trust-ordered replacement rule.
//!
//! `docs/dash-runtime/DESIGN.md` 3.2/3.3. Pure: the clock never reads a time
//! source, it only maps instants it is given.

use core::time::Duration;

use broadcast_common::impl_spec_display;
use broadcast_common::stage::Timestamp;
use broadcast_common::time::MonoUtc;

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

/// One anchor: at monotonic `mono` UTC was `utc_us`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub struct Anchor {
    /// Monotonic instant.
    pub mono: Timestamp,
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
            mono: Timestamp::from_micros(sample.mono_us),
            utc_us: sample.utc_us,
            source,
            uncertainty_us: sample.uncertainty_us,
        }
    }

    /// UTC at `now`, saturating at the `i64` range.
    #[must_use]
    pub fn utc_at(&self, now: Timestamp) -> i64 {
        let m = MonoUtc::new(self.mono.as_micros(), i128::from(self.utc_us));
        saturate(m.utc_at(now.as_micros()))
    }

    /// The monotonic instant at which UTC reaches `utc_us`, saturating at
    /// `0..=u64::MAX`.
    #[must_use]
    pub fn mono_for_utc(&self, utc_us: i64) -> Option<Timestamp> {
        let m = MonoUtc::new(self.mono.as_micros(), i128::from(self.utc_us));
        let mono = m.mono_for_utc(i128::from(utc_us));
        u64::try_from(mono.clamp(0, i128::from(u64::MAX)))
            .ok()
            .map(Timestamp::from_micros)
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
    pub resync: Duration,
    /// Let a less trusted source replace a more trusted one (the better one
    /// has failed for the configured wait).
    pub allow_downgrade: bool,
}

impl OfferPolicy {
    /// A policy.
    #[must_use]
    pub fn new(resync: Duration, allow_downgrade: bool) -> Self {
        Self {
            resync,
            allow_downgrade,
        }
    }

    /// A policy whose anchor never goes stale: a same-source sample replaces
    /// it only on a clock step or to refine its error bar.
    #[must_use]
    pub fn new_or_never(resync: Option<Duration>, allow_downgrade: bool) -> Self {
        Self {
            resync: resync.unwrap_or(Duration::MAX),
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

    /// UTC at `now`, or `None` before the first anchor.
    #[must_use]
    pub fn utc_at(&self, now: Timestamp) -> Option<i64> {
        self.anchor.map(|a| a.utc_at(now))
    }

    /// The monotonic instant at which UTC reaches `utc_us`.
    #[must_use]
    pub fn mono_for_utc(&self, utc_us: i64) -> Option<Timestamp> {
        self.anchor.and_then(|a| a.mono_for_utc(utc_us))
    }

    /// Offer a measurement; the anchor changes per the replacement rule.
    ///
    /// A more trusted source replaces at once; a less trusted one only when
    /// `policy.allow_downgrade`. For the same source: a reading that disagrees
    /// with the prediction by more than the two error bars together is a clock
    /// step and is adopted immediately; otherwise it replaces the anchor when
    /// it is at least as precise or the anchor is older than
    /// `policy.resync`.
    pub fn offer(
        &mut self,
        sample: UtcSample,
        source: ClockSource,
        now: Timestamp,
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
            let predicted = i128::from(old.utc_at(new.mono));
            let diff = (i128::from(new.utc_us) - predicted).unsigned_abs();
            let bars = u128::from(old.uncertainty_us) + u128::from(new.uncertainty_us);
            if diff > bars {
                AdoptReason::Step
            } else if now.saturating_sub(old.mono) >= policy.resync {
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
