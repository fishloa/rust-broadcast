//! Crate-internal unit and rounding constants shared by the modules that convert
//! between a stated rate/quotient and the integer the wire carries.

/// Bits in one kilobit, converting a stated kbit/s rate to bits per second.
pub(crate) const BITS_PER_KILOBIT: u32 = 1000;

/// Divisor that halves a denominator so `(num + den / ROUNDING_DIVISOR) / den`
/// rounds a quotient to nearest.
pub(crate) const ROUNDING_DIVISOR: u64 = 2;
