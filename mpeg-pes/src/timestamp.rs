//! PTS / DTS — 33-bit presentation / decoding timestamps at 90 kHz
//! (ISO/IEC 13818-1 §2.4.3.7). Encoded across 5 bytes with a 4-bit prefix and
//! three interleaved `marker_bit`s.

use crate::error::{Error, Result};

/// 33-bit value mask.
const TS_MASK: u64 = (1 << 33) - 1;

/// Presentation Time Stamp (33-bit, 90 kHz units).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct Pts(pub u64);

/// Decoding Time Stamp (33-bit, 90 kHz units).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct Dts(pub u64);

impl Pts {
    /// Value in 90 kHz units (0..=2³³−1).
    #[must_use]
    pub const fn ticks(self) -> u64 {
        self.0
    }
    /// Value in seconds.
    #[must_use]
    pub fn seconds(self) -> f64 {
        self.0 as f64 / 90_000.0
    }
    /// Encode as a standalone 5-byte PTS field (prefix `0010`, for `PTS_DTS_flags = 10`).
    #[must_use]
    pub fn to_field_bytes(self) -> [u8; 5] {
        write(self.0, 0b0010)
    }
    /// Decode from a 5-byte PTS-only field (prefix `0010`).
    ///
    /// Exact inverse of [`to_field_bytes`](Self::to_field_bytes).
    /// Returns an error if the prefix or marker bits are wrong.
    pub fn from_field_bytes(b: &[u8; 5]) -> crate::Result<Self> {
        Ok(Pts(read(b, 0b0010, "PTS")?))
    }
    /// Decode from a 5-byte PTS field in a PTS+DTS pair (prefix `0011`).
    pub fn from_field_bytes_with_dts(b: &[u8; 5]) -> crate::Result<Self> {
        Ok(Pts(read(b, 0b0011, "PTS(with DTS)")?))
    }
}

impl Dts {
    /// Value in 90 kHz units (0..=2³³−1).
    #[must_use]
    pub const fn ticks(self) -> u64 {
        self.0
    }
    /// Value in seconds.
    #[must_use]
    pub fn seconds(self) -> f64 {
        self.0 as f64 / 90_000.0
    }
    /// Encode as a 5-byte DTS field (prefix `0001`, the DTS half of a PTS+DTS pair).
    #[must_use]
    pub fn to_field_bytes(self) -> [u8; 5] {
        write(self.0, 0b0001)
    }
    /// Decode from a 5-byte DTS field (prefix `0001`).
    ///
    /// Exact inverse of [`to_field_bytes`](Self::to_field_bytes).
    pub fn from_field_bytes(b: &[u8; 5]) -> crate::Result<Self> {
        Ok(Dts(read(b, 0b0001, "DTS")?))
    }
}

/// Decode a 5-byte PTS/DTS field. `prefix` is the expected leading 4-bit value
/// (`0b0010` PTS-only, `0b0011` PTS in a PTS+DTS pair, `0b0001` DTS). The three
/// `marker_bit`s must be `1`.
pub(crate) fn read(b: &[u8], prefix: u8, what: &'static str) -> Result<u64> {
    if b.len() < 5 {
        return Err(Error::BufferTooShort {
            need: 5,
            have: b.len(),
            what,
        });
    }
    if (b[0] >> 4) != prefix {
        return Err(Error::BadTimestampPrefix(what));
    }
    if b[0] & 0x01 == 0 || b[2] & 0x01 == 0 || b[4] & 0x01 == 0 {
        return Err(Error::BadTimestampMarker(what));
    }
    let hi = u64::from((b[0] >> 1) & 0x07); // [32:30]
    let mid = (u64::from(b[1]) << 7) | u64::from(b[2] >> 1); // [29:15]
    let lo = (u64::from(b[3]) << 7) | u64::from(b[4] >> 1); // [14:0]
    Ok((hi << 30) | (mid << 15) | lo)
}

/// Encode a 33-bit value into a 5-byte PTS/DTS field with the given 4-bit prefix.
pub(crate) fn write(ts: u64, prefix: u8) -> [u8; 5] {
    let ts = ts & TS_MASK;
    [
        (prefix << 4) | ((((ts >> 30) & 0x07) as u8) << 1) | 0x01,
        ((ts >> 22) & 0xFF) as u8,
        ((((ts >> 15) & 0x7F) as u8) << 1) | 0x01,
        ((ts >> 7) & 0xFF) as u8,
        (((ts & 0x7F) as u8) << 1) | 0x01,
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pts_round_trip_boundary_values() {
        for ts in [0u64, 1, 90_000, 0x1_2345_6789, TS_MASK] {
            let enc = write(ts, 0b0010);
            assert_eq!(read(&enc, 0b0010, "pts").unwrap(), ts, "ts={ts:#x}");
        }
    }

    #[test]
    fn rejects_bad_prefix() {
        let enc = write(0, 0b0011);
        assert!(matches!(
            read(&enc, 0b0010, "pts"),
            Err(Error::BadTimestampPrefix(_))
        ));
    }

    #[test]
    fn rejects_bad_marker() {
        let mut enc = write(0, 0b0010);
        enc[2] &= 0xFE; // clear a marker bit
        assert!(matches!(
            read(&enc, 0b0010, "pts"),
            Err(Error::BadTimestampMarker(_))
        ));
    }

    #[test]
    fn seconds() {
        assert!((Pts(90_000).seconds() - 1.0).abs() < 1e-9);
    }

    // ── from_field_bytes round-trips ─────────────────────────────────────────

    /// `to_field_bytes` → `from_field_bytes` → same value.
    #[test]
    fn pts_from_field_bytes_round_trip() {
        for val in [0u64, 1, 90_000, 0x1_FFFF_FFFF, TS_MASK] {
            let pts = Pts(val);
            let bytes = pts.to_field_bytes();
            let decoded = Pts::from_field_bytes(&bytes).unwrap();
            assert_eq!(decoded, pts, "val={val:#x}");
        }
    }

    /// `to_field_bytes` → `from_field_bytes_with_dts` → same value.
    #[test]
    fn pts_from_field_bytes_with_dts_round_trip() {
        let pts = Pts(0x1234_5678);
        // In a PTS+DTS pair, PTS uses prefix 0b0011.
        let bytes = crate::timestamp::write(pts.0, 0b0011);
        let decoded = Pts::from_field_bytes_with_dts(&bytes).unwrap();
        assert_eq!(decoded, pts);
    }

    /// `Dts::to_field_bytes` → `Dts::from_field_bytes` → same value.
    #[test]
    fn dts_from_field_bytes_round_trip() {
        for val in [0u64, 1, 90_000, TS_MASK] {
            let dts = Dts(val);
            let bytes = dts.to_field_bytes();
            let decoded = Dts::from_field_bytes(&bytes).unwrap();
            assert_eq!(decoded, dts, "val={val:#x}");
        }
    }
}
