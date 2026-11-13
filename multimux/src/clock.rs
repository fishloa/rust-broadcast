//! The one place a pull source reads the machine's clocks.
//!
//! A sans-IO core takes every instant as an argument; the drive loop that owns
//! it asks a [`HostClock`] for them. Keeping the reads behind this trait means
//! a source's own module never calls `Instant::now`/`SystemTime::now`, and a
//! test can hand it a clock with a known skew.

use std::time::{Instant, SystemTime, UNIX_EPOCH};

/// A pair of clocks sampled back to back: a monotonic one that counts from the
/// moment the source opened, and the host's UTC clock.
pub(crate) trait HostClock: Send + Sync + 'static {
    /// Microseconds on the monotonic clock since the source opened.
    fn mono_us(&self) -> u64;
    /// Microseconds since 1970-01-01T00:00:00Z (POSIX, no leap seconds) on the
    /// host's own wall clock; `0` for a clock set before the epoch.
    fn unix_us(&self) -> u64;
    /// The wall-clock wait that spans `session` of this clock's own time; a clock that runs at
    /// real speed returns it unchanged.
    fn real_duration(&self, session: std::time::Duration) -> std::time::Duration {
        session
    }
}

/// The operating system's clocks.
pub(crate) struct SystemHostClock {
    opened: Instant,
}

impl SystemHostClock {
    pub(crate) fn new() -> Self {
        Self {
            opened: Instant::now(),
        }
    }

    /// The host's wall clock read once, for a caller that needs a single
    /// instant and owns no clock of its own.
    pub(crate) fn now_us() -> u64 {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |d| u64::try_from(d.as_micros()).unwrap_or(u64::MAX))
    }
}

#[cfg(test)]
impl SystemHostClock {
    /// Microseconds since 1970-01-01T00:00:00Z on the host's own wall clock.
    pub(crate) fn unix_us(&self) -> u64 {
        <Self as HostClock>::unix_us(self)
    }
}

impl HostClock for SystemHostClock {
    fn mono_us(&self) -> u64 {
        u64::try_from(self.opened.elapsed().as_micros()).unwrap_or(u64::MAX)
    }

    fn unix_us(&self) -> u64 {
        Self::now_us()
    }
}

/// A clock running `factor` times faster than real time, so a test sees a session's retry
/// backoff elapse in a fraction of the wall time: the host divides every wait it derives from the
/// session's instants by the same factor ([`HostClock::real_duration`]). The UTC clock is a fixed
/// epoch plus the scaled monotonic count.
#[cfg(test)]
pub(crate) struct ScaledHostClock {
    opened: Instant,
    factor: u32,
}

#[cfg(test)]
impl ScaledHostClock {
    const EPOCH_US: u64 = 1_700_000_000 * 1_000_000;

    pub(crate) fn new(factor: u32) -> Self {
        Self {
            opened: Instant::now(),
            factor,
        }
    }
}

#[cfg(test)]
impl HostClock for ScaledHostClock {
    fn mono_us(&self) -> u64 {
        u64::try_from(self.opened.elapsed().as_micros())
            .unwrap_or(u64::MAX)
            .saturating_mul(u64::from(self.factor))
    }

    fn unix_us(&self) -> u64 {
        Self::EPOCH_US.saturating_add(self.mono_us())
    }

    fn real_duration(&self, session: std::time::Duration) -> std::time::Duration {
        session / self.factor
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_monotonic_clock_starts_near_zero_and_never_goes_back() {
        let clock = SystemHostClock::new();
        let a = clock.mono_us();
        let b = clock.mono_us();
        assert!(a < 5_000_000, "counts from open, not from boot: {a}");
        assert!(b >= a);
    }

    #[test]
    fn the_wall_clock_reads_a_plausible_date() {
        const YEAR_2020_US: u64 = 1_577_836_800 * 1_000_000;
        assert!(SystemHostClock::new().unix_us() > YEAR_2020_US);
    }
}
