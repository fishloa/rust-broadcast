//! The decimal time-unit ratios every crate that converts between seconds, milliseconds,
//! microseconds and nanoseconds uses, declared once.
//!
//! All are `u64`; a caller that works in another integer or float width casts at the use
//! (`NANOS_PER_SEC as i128`). [`crate::timescale::MICROS_PER_SECOND`] is the `u32` spelling of
//! [`MICROS_PER_SEC`] the rate-conversion functions take.

/// Milliseconds in a second.
pub const MILLIS_PER_SEC: u64 = 1_000;
/// Microseconds in a millisecond.
pub const MICROS_PER_MILLI: u64 = 1_000;
/// Microseconds in a second.
pub const MICROS_PER_SEC: u64 = MILLIS_PER_SEC * MICROS_PER_MILLI;
/// Nanoseconds in a microsecond.
pub const NANOS_PER_MICRO: u64 = 1_000;
/// Nanoseconds in a millisecond.
pub const NANOS_PER_MILLI: u64 = NANOS_PER_MICRO * MICROS_PER_MILLI;
/// Nanoseconds in a second.
pub const NANOS_PER_SEC: u64 = NANOS_PER_MILLI * MILLIS_PER_SEC;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_ratios_are_the_decimal_units() {
        assert_eq!(MICROS_PER_SEC, 1_000_000);
        assert_eq!(NANOS_PER_MILLI, 1_000_000);
        assert_eq!(NANOS_PER_SEC, 1_000_000_000);
        assert_eq!(
            MICROS_PER_SEC,
            u64::from(crate::timescale::MICROS_PER_SECOND)
        );
    }
}
