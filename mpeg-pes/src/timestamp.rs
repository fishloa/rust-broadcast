//! PTS / DTS — 33-bit presentation / decoding timestamps at 90 kHz
//! (ISO/IEC 13818-1 §2.4.3.7). Encoded across 5 bytes with a 4-bit prefix and
//! three interleaved `marker_bit`s.

use crate::error::Result;
use broadcast_common::clock33::{Clock33, ClockOutOfRange};

/// The 4-bit prefix of a standalone DTS field (ISO/IEC 13818-1 Table 2-21).
pub(crate) const DTS_PREFIX: u8 = 0b0001;
/// The 4-bit prefix of a standalone PTS field.
pub(crate) const PTS_PREFIX: u8 = 0b0010;
/// The 4-bit prefix of the PTS field that is followed by a DTS.
pub(crate) const PTS_WITH_DTS_PREFIX: u8 = 0b0011;
/// Byte width of a PTS/DTS field.
pub(crate) const FIELD_LEN: usize = 5;
/// Bit shift of the 4-bit prefix within the field's first byte.
const PREFIX_SHIFT: u32 = 4;
/// `marker_bit` at bit 0 of field bytes 0, 2 and 4.
const MARKER_MASK: u8 = 0x01;
/// Byte offsets of the `marker_bit`s within the PTS/DTS field
/// (ISO/IEC 13818-1 Table 2-21).
const MARKER_BYTE_0: usize = 0;
const MARKER_BYTE_2: usize = MARKER_BYTE_0 + 2;
const MARKER_BYTE_4: usize = MARKER_BYTE_2 + 2;

/// Presentation Time Stamp (33-bit, 90 kHz units), carried as the validated
/// [`Clock33`] so an out-of-range value cannot be constructed.
///
/// Construct with [`Pts::try_new`] (checked) or [`Pts::from`] a [`Clock33`];
/// read the raw 90 kHz count with [`Pts::ticks`]. Parsing is infallible on the
/// value itself: the 5-byte wire field is exactly 33 bits, so a parsed value is
/// always in range.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct Pts(Clock33);

/// Decoding Time Stamp (33-bit, 90 kHz units), carried as the validated
/// [`Clock33`] (see [`Pts`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct Dts(Clock33);

impl Pts {
    /// Construct from a raw 90 kHz tick count, rejecting any value at or above
    /// 2^33 rather than silently wrapping it to a different timestamp.
    ///
    /// # Errors
    /// [`ClockOutOfRange`] (`field: "33-bit ticks"`) when `ticks >= 2^33`.
    pub fn try_new(ticks: u64) -> core::result::Result<Self, ClockOutOfRange> {
        Clock33::try_new(ticks).map(Pts)
    }

    /// The validated 33-bit clock value.
    #[must_use]
    pub const fn clock33(self) -> Clock33 {
        self.0
    }

    /// Value in 90 kHz units (0..=2³³−1).
    #[must_use]
    pub const fn ticks(self) -> u64 {
        self.0.ticks()
    }

    /// Value in seconds.
    #[must_use]
    pub fn seconds(self) -> f64 {
        self.0.ticks() as f64 / 90_000.0
    }

    /// Encode as a standalone 5-byte PTS field (prefix `0010`, for `PTS_DTS_flags = 10`).
    #[must_use]
    pub fn to_field_bytes(self) -> [u8; 5] {
        write(self.0.ticks(), PTS_PREFIX)
    }

    /// Decode from a 5-byte PTS-only field (prefix `0010`).
    ///
    /// Exact inverse of [`to_field_bytes`](Self::to_field_bytes).
    /// Returns an error if the prefix or marker bits are wrong.
    pub fn from_field_bytes(b: &[u8; 5]) -> Result<Self> {
        Ok(Pts(Clock33::try_new(read(b, PTS_PREFIX, "PTS")?)?))
    }

    /// Decode from a 5-byte PTS field in a PTS+DTS pair (prefix `0011`).
    pub fn from_field_bytes_with_dts(b: &[u8; 5]) -> Result<Self> {
        Ok(Pts(Clock33::try_new(read(
            b,
            PTS_WITH_DTS_PREFIX,
            "PTS(with DTS)",
        )?)?))
    }
}

impl From<Clock33> for Pts {
    fn from(clock: Clock33) -> Self {
        Pts(clock)
    }
}

impl Dts {
    /// Construct from a raw 90 kHz tick count, rejecting a value at or above
    /// 2^33 (see [`Pts::try_new`]).
    ///
    /// # Errors
    /// [`ClockOutOfRange`] when `ticks >= 2^33`.
    pub fn try_new(ticks: u64) -> core::result::Result<Self, ClockOutOfRange> {
        Clock33::try_new(ticks).map(Dts)
    }

    /// The validated 33-bit clock value.
    #[must_use]
    pub const fn clock33(self) -> Clock33 {
        self.0
    }

    /// Value in 90 kHz units (0..=2³³−1).
    #[must_use]
    pub const fn ticks(self) -> u64 {
        self.0.ticks()
    }

    /// Value in seconds.
    #[must_use]
    pub fn seconds(self) -> f64 {
        self.0.ticks() as f64 / 90_000.0
    }

    /// Encode as a 5-byte DTS field (prefix `0001`, the DTS half of a
    /// PTS+DTS pair).
    #[must_use]
    pub fn to_field_bytes(self) -> [u8; 5] {
        write(self.0.ticks(), DTS_PREFIX)
    }

    /// Decode from a 5-byte DTS field (prefix `0001`).
    ///
    /// Exact inverse of [`to_field_bytes`](Self::to_field_bytes).
    pub fn from_field_bytes(b: &[u8; 5]) -> Result<Self> {
        Ok(Dts(Clock33::try_new(read(b, DTS_PREFIX, "DTS")?)?))
    }
}

impl From<Clock33> for Dts {
    fn from(clock: Clock33) -> Self {
        Dts(clock)
    }
}

/// Decode a 5-byte PTS/DTS field. `prefix` is the expected leading 4-bit value
/// (`0b0010` PTS-only, `0b0011` PTS in a PTS+DTS pair, `0b0001` DTS). The three
/// `marker_bit`s must be `1`. The returned value is always in `[0, 2^33)`, so it
/// fits a [`Clock33`] without another check.
pub(crate) fn read(b: &[u8], prefix: u8, what: &'static str) -> Result<u64> {
    if b.len() < FIELD_LEN {
        return Err(crate::error::Error::BufferTooShort(
            broadcast_common::error::BufferTooShort {
                need: FIELD_LEN,
                have: b.len(),
                what,
            },
        ));
    }
    if (b[0] >> PREFIX_SHIFT) != prefix {
        return Err(crate::error::Error::BadTimestampPrefix(what));
    }
    if b[MARKER_BYTE_0] & MARKER_MASK == 0
        || b[MARKER_BYTE_2] & MARKER_MASK == 0
        || b[MARKER_BYTE_4] & MARKER_MASK == 0
    {
        return Err(crate::error::Error::BadTimestampMarker(what));
    }
    // `b.len() < FIELD_LEN` was rejected above, so the field bytes are present.
    let mut field = [0_u8; FIELD_LEN];
    field.copy_from_slice(&b[..FIELD_LEN]);
    Ok(broadcast_common::clock33::unpack_marked(&field).timestamp)
}

/// Encode a 33-bit value into a 5-byte PTS/DTS field with the given 4-bit prefix.
pub(crate) fn write(ts: u64, prefix: u8) -> [u8; FIELD_LEN] {
    broadcast_common::clock33::pack_marked(ts, prefix)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pts_round_trip_boundary_values() {
        for ts in [
            0u64,
            1,
            90_000,
            0x1_2345_6789,
            broadcast_common::clock33::WRAP_33BIT - 1,
        ] {
            let enc = write(ts, 0b0010);
            assert_eq!(read(&enc, 0b0010, "pts").unwrap(), ts, "ts={ts:#x}");
        }
    }

    #[test]
    fn try_new_rejects_out_of_range() {
        assert!(Pts::try_new(broadcast_common::clock33::WRAP_33BIT).is_err());
        assert!(Pts::try_new(broadcast_common::clock33::WRAP_33BIT - 1).is_ok());
    }

    #[test]
    fn rejects_bad_prefix() {
        let enc = write(0, 0b0011);
        assert!(matches!(
            read(&enc, 0b0010, "pts"),
            Err(crate::error::Error::BadTimestampPrefix(_))
        ));
    }

    #[test]
    fn rejects_bad_marker() {
        let mut enc = write(0, 0b0010);
        enc[2] &= 0xFE; // clear a marker bit
        assert!(matches!(
            read(&enc, 0b0010, "pts"),
            Err(crate::error::Error::BadTimestampMarker(_))
        ));
    }

    #[test]
    fn seconds() {
        assert!((Pts::try_new(90_000).unwrap().seconds() - 1.0).abs() < 1e-9);
    }

    // ── from_field_bytes round-trips ─────────────────────────────────────────

    /// `to_field_bytes` → `from_field_bytes` → same value.
    #[test]
    fn pts_from_field_bytes_round_trip() {
        for val in [
            0u64,
            1,
            90_000,
            0x1_FFFF_FFFF,
            broadcast_common::clock33::WRAP_33BIT - 1,
        ] {
            let pts = Pts::try_new(val).unwrap();
            let bytes = pts.to_field_bytes();
            let decoded = Pts::from_field_bytes(&bytes).unwrap();
            assert_eq!(decoded, pts, "val={val:#x}");
        }
    }

    /// `to_field_bytes` → `from_field_bytes_with_dts` → same value.
    #[test]
    fn pts_from_field_bytes_with_dts_round_trip() {
        let pts = Pts::try_new(0x1234_5678).unwrap();
        // In a PTS+DTS pair, PTS uses prefix 0b0011.
        let bytes = crate::timestamp::write(pts.ticks(), 0b0011);
        let decoded = Pts::from_field_bytes_with_dts(&bytes).unwrap();
        assert_eq!(decoded, pts);
    }

    /// `Dts::to_field_bytes` → `Dts::from_field_bytes` → same value.
    #[test]
    fn dts_from_field_bytes_round_trip() {
        for val in [0u64, 1, 90_000, broadcast_common::clock33::WRAP_33BIT - 1] {
            let dts = Dts::try_new(val).unwrap();
            let bytes = dts.to_field_bytes();
            let decoded = Dts::from_field_bytes(&bytes).unwrap();
            assert_eq!(decoded, dts, "val={val:#x}");
        }
    }

    /// A four-byte slice is rejected before any marker bit is read.
    #[test]
    fn a_short_field_reports_the_five_bytes_it_needs() {
        let short = [0x21u8, 0x00, 0x01, 0x01];
        match read(&short, 0b0010, "PTS") {
            Err(crate::error::Error::BufferTooShort(e)) => {
                assert_eq!(e.need, 5);
                assert_eq!(e.have, 4);
            }
            other => panic!("expected BufferTooShort, got {other:?}"),
        }
    }
}
