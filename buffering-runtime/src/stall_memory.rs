//! Stall memory: a max-hold of the PDV with a hold then a linear decay.

use core::time::Duration;

use crate::time::{duration_add, duration_micros, micros_as_duration};

/// Plain-data snapshot of the stall memory, for session persistence.
///
/// **Aging semantics.** `peak` is the raw held peak; `age` is how long ago
/// (on the controller's arrival clock) that peak was observed. The
/// decayed value is `peak` while `age <= hold`, then falls linearly to 0 over
/// `decay`. A snapshot restored after `N` seconds of wall time must be aged by
/// `N` first: [`StallMemorySnapshot::aged`]. The restored memory is anchored
/// at the first clock reading the new controller sees, so a restore adds no
/// extra age of its own.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[non_exhaustive]
pub struct StallMemorySnapshot {
    /// Raw held peak PDV.
    pub peak: Duration,
    /// Age of that peak.
    pub age: Duration,
}

impl StallMemorySnapshot {
    /// Build a snapshot. The struct is `#[non_exhaustive]`, so a foreign
    /// crate or FFI wrapper cannot use a struct literal: this `const fn` is
    /// the constructor (usable in `const`/`static` context; `Default` is the
    /// empty memory and the fields stay public for reading).
    #[must_use]
    pub const fn new(peak: Duration, age: Duration) -> Self {
        Self { peak, age }
    }

    /// The same memory `elapsed` of wall time later (saturating at the end of
    /// the microsecond range).
    #[must_use]
    pub fn aged(self, elapsed: Duration) -> Self {
        Self {
            peak: self.peak,
            age: duration_add(self.age, elapsed),
        }
    }
}

/// Decayed value of a peak of the given age.
pub(crate) fn decayed(peak: u64, age: u64, hold: u64, decay: u64) -> u64 {
    if age <= hold {
        return peak;
    }
    let into = age - hold;
    if into >= decay {
        return 0;
    }
    let v = u128::from(peak) * u128::from(decay - into) / u128::from(decay.max(1));
    u64::try_from(v).unwrap_or(u64::MAX)
}

fn age_at(now: u64, at: i128) -> u64 {
    u64::try_from((i128::from(now) - at).max(0)).unwrap_or(u64::MAX)
}

#[derive(Clone, Debug)]
pub(crate) struct MaxHold {
    peak: u64,
    at: Option<i128>,
    pending_age: Option<u64>,
    hold: u64,
    decay: u64,
}

impl MaxHold {
    pub(crate) fn new(hold: u64, decay: u64) -> Self {
        Self {
            peak: 0,
            at: None,
            pending_age: None,
            hold,
            decay,
        }
    }

    /// Anchor a restored-but-unplaced snapshot at the clock reading `now`.
    pub(crate) fn anchor(&mut self, now: u64) {
        if let Some(age) = self.pending_age.take() {
            self.at = Some(i128::from(now) - i128::from(age));
        }
    }

    pub(crate) fn value(&self, now: u64) -> u64 {
        match (self.at, self.pending_age) {
            (Some(at), _) => decayed(self.peak, age_at(now, at), self.hold, self.decay),
            (None, Some(age)) => decayed(self.peak, age, self.hold, self.decay),
            (None, None) => 0,
        }
    }

    /// Sample-driven: a sample at least as large as the current decayed
    /// value replaces the peak (so the memory rises immediately).
    pub(crate) fn observe(&mut self, now: u64, v: u64) {
        self.anchor(now);
        if self.at.is_none() || v >= self.value(now) {
            self.peak = v;
            self.at = Some(i128::from(now));
        }
    }

    pub(crate) fn snapshot(&self, now: u64) -> StallMemorySnapshot {
        match (self.at, self.pending_age) {
            (Some(at), _) => StallMemorySnapshot::new(
                micros_as_duration(self.peak),
                micros_as_duration(age_at(now, at)),
            ),
            (None, Some(age)) => {
                StallMemorySnapshot::new(micros_as_duration(self.peak), micros_as_duration(age))
            }
            (None, None) => StallMemorySnapshot::default(),
        }
    }

    pub(crate) fn restore(&mut self, s: StallMemorySnapshot) {
        self.peak = duration_micros(s.peak);
        self.at = None;
        self.pending_age = Some(duration_micros(s.age));
    }

    pub(crate) fn clear(&mut self) {
        self.peak = 0;
        self.at = None;
        self.pending_age = None;
    }
}
