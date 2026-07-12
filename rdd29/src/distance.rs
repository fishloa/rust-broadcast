//! Relative distance coding — RDD 29:2019 §3.2.
//!
//! These are read-only derived views over the raw wire codes (`ObjectPosX`/
//! `ObjectPosY`/`ObjectPosZ`/`ObjectSpread`) — the raw `u16` code is always
//! the round-tripped source of truth; these helpers never feed back into
//! parsing/serialization.
//!
//! No `libm`/`std` transcendental functions are used (only shifts, and core
//! `f64` arithmetic), so these work identically under `no_std`.

/// Decode a `DistanceXY`-coded 16-bit value (§3.2) to its `[0,1]`-range
/// linear position (`ObjectPosX`/`ObjectPosY`, always 16 bits wide):
///
/// ```text
/// DistanceXY = Dn/2^(n-1) - (2^(n-1)-1)/2^(n-1)
/// ```
#[must_use]
pub fn distance_xy(d: u16) -> f64 {
    const N: u32 = 16;
    let dn = f64::from(d);
    let half = f64::from(1u32 << (N - 1)); // 2^(n-1)
    dn / half - (half - 1.0) / half
}

/// Decode a `DistanceZ`-coded `n`-bit value (§3.2) to its `[0,1]`-range
/// linear position/spread (`ObjectPosZ`, `n=16`; `ObjectSpread`, `n=8` or
/// `n=12` depending on [`crate::ObjectSpreadMode`]):
///
/// ```text
/// DistanceZ = Dn/(2^n - 1)
/// ```
#[must_use]
pub fn distance_z(d: u16, n: u32) -> f64 {
    let dn = f64::from(d);
    let max = f64::from((1u32 << n) - 1); // 2^n - 1
    dn / max
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn distance_xy_boundary_values() {
        // Formula's own stated valid range: 2^(n-1)-1 <= Dn <= 2^n-1.
        assert!((distance_xy(0x7FFF) - 0.0).abs() < 1e-9); // Dn = 2^15-1
        assert!((distance_xy(0xFFFF) - 1.0).abs() < 1e-9); // Dn = 2^16-1
    }

    #[test]
    fn distance_z_boundary_values() {
        assert!((distance_z(0, 16) - 0.0).abs() < 1e-9);
        assert!((distance_z(0xFFFF, 16) - 1.0).abs() < 1e-9);
        assert!((distance_z(0, 8) - 0.0).abs() < 1e-9);
        assert!((distance_z(0xFF, 8) - 1.0).abs() < 1e-9);
    }
}
