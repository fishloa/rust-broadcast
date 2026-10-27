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
}

impl HostClock for SystemHostClock {
    fn mono_us(&self) -> u64 {
        u64::try_from(self.opened.elapsed().as_micros()).unwrap_or(u64::MAX)
    }

    fn unix_us(&self) -> u64 {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |d| u64::try_from(d.as_micros()).unwrap_or(u64::MAX))
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
