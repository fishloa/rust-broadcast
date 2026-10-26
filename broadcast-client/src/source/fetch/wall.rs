//! The host wall clock as an anchor pair, for the places a segment-pull source needs "now" in UTC.

/// At the monotonic instant `mono_us` the host UTC clock read `unix_us` microseconds since
/// 1970-01-01T00:00:00Z (POSIX, no leap seconds). See [`crate::Session::set_wall_clock`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct WallClock {
    mono_us: u64,
    unix_us: u64,
}

impl WallClock {
    pub(crate) fn new(mono_us: u64, unix_us: u64) -> Self {
        Self { mono_us, unix_us }
    }

    pub(crate) fn unix_at(&self, now_us: u64) -> i128 {
        i128::from(self.unix_us) + i128::from(now_us) - i128::from(self.mono_us)
    }
}
