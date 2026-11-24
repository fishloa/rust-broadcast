//! Time structures and 90 kHz helpers — ANSI/SCTE 35 2023r1 §9.8.1 (Table 14)
//! and §9.8.2 (Table 15).
//!
//! `splice_time()` and `break_duration()` carry 33-bit / 40-bit counts of
//! ticks of the program's 90 kHz clock. The decoded accessors here convert
//! those tick counts to/from [`core::time::Duration`] (the 4.1.0 decoded-field
//! pattern), so callers never re-derive the 90 kHz scaling by hand.

use crate::error::{Error, Result};
use broadcast_common::clock33::Clock33;
use broadcast_common::len::{need_in, need_out};
use broadcast_common::{Parse, Serialize};

/// One second of the program clock in 90 kHz ticks.
pub const TICKS_PER_SECOND: u64 = 90_000;

/// Number of distinct values of a 33-bit field; the `pts_adjustment` /
/// `pts_time` wrap modulus (§9.6.1: carry is ignored on overflow).
pub const PTS_MODULUS: u64 = broadcast_common::clock33::WRAP_33BIT;

/// Largest value a 33-bit field can hold (`2^33 - 1`).
pub const PTS_MAX: u64 = PTS_MODULUS - 1;

/// Largest value a 40-bit field can hold (`2^40 - 1`); the `break_duration`
/// `duration` range.
pub const DURATION_40_MAX: u64 = (1 << 40) - 1;

/// Nanoseconds per second.
const NANOS_PER_SECOND: u128 = 1_000_000_000;
const PTS_FLAG_MASK: u8 = 0x80;
/// The 6 reserved bits set to 1 after the pts_time flag.
const PTS_RESERVED_BITS: u8 = 0x7E;
/// The 7 reserved bits set to 1 when no pts_time follows.
const NO_TIME_RESERVED_BITS: u8 = 0x7F;
const PTS_TOP_MASK: u8 = 0x01;
const PTS_TOP_SHIFT: u32 = 32;
const PTS_HIGH_SHIFT: u32 = 24;
const PTS_MID_SHIFT: u32 = 16;
const PTS_LOW_SHIFT: u32 = 8;
const AUTO_RETURN_MASK: u8 = 0x80;
const AUTO_RETURN_SHIFT: u32 = 7;
// Byte offsets of the 5-byte pts_time/duration placeholders.
const PTS_TOP_OFFSET: usize = 0;
const PTS_HIGH_OFFSET: usize = 1;
const PTS_MID_OFFSET: usize = 2;
const PTS_LOW_OFFSET: usize = 3;
const PTS_BOTTOM_OFFSET: usize = 4;

/// Convert a 90 kHz tick count to a [`Duration`](core::time::Duration).
///
/// Nanosecond-exact: `ticks * 1e9 / 90_000`. Computed in `u128` so the full
/// 40-bit `break_duration` range cannot overflow.
#[must_use]
pub fn ticks_to_duration(ticks: u64) -> core::time::Duration {
    let nanos = (ticks as u128) * NANOS_PER_SECOND / (TICKS_PER_SECOND as u128);
    // nanos fits u64 for the whole 40-bit range (2^40/90000 s ≈ 12_725_000 s);
    // an out-of-field u64 tick count saturates instead of silently wrapping.
    core::time::Duration::from_nanos(u64::try_from(nanos).unwrap_or(u64::MAX))
}

/// Convert a [`Duration`](core::time::Duration) to a 90 kHz tick count,
/// truncating toward zero. Returns `None` if the result would exceed `max`
/// (the field's wire capacity).
#[must_use]
pub fn duration_to_ticks(d: core::time::Duration, max: u64) -> Option<u64> {
    let nanos = d.as_nanos();
    let ticks = nanos * (TICKS_PER_SECOND as u128) / NANOS_PER_SECOND;
    if ticks > max as u128 {
        None
    } else {
        Some(ticks as u64)
    }
}

/// Add two 33-bit PTS values modulo `2^33`, the carry-ignored wrap the spec
/// defines for `pts_adjustment` applied to a `pts_time` (§9.6.1, §9.8.1).
#[must_use]
pub fn pts_add_wrapping(pts_time: u64, pts_adjustment: u64) -> u64 {
    broadcast_common::clock33::add(pts_time, pts_adjustment)
}

/// `splice_time()` — §9.8.1, Table 14.
///
/// When `pts_time` is present it is a 33-bit count of 90 kHz ticks; absent
/// (`time_specified_flag == 0`) it signals an immediate command.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub struct SpliceTime {
    /// 33-bit `pts_time` in 90 kHz ticks, or `None` when
    /// `time_specified_flag == 0`.
    pub pts_time: Option<Clock33>,
}

impl SpliceTime {
    /// Bytes this structure serializes to: 5 when a `pts_time` is present
    /// (1 flag/reserved byte + the 33-bit field spread over 5 bytes), else 1.
    pub const LEN_WITH_TIME: usize = 5;
    /// Length when `time_specified_flag == 0`.
    pub const LEN_NO_TIME: usize = 1;

    /// A `splice_time()` carrying an explicit [`Clock33`] `pts_time`.
    ///
    /// The type already guarantees `pts_time < 2^33`, so every value this
    /// accepts serializes: there is no second, masking path beside the
    /// serializer's range check.
    #[must_use]
    pub const fn at(pts_time: Clock33) -> Self {
        Self {
            pts_time: Some(pts_time),
        }
    }

    /// The `time_specified_flag == 0` form: no time, the splicer acts at the
    /// nearest opportunity.
    #[must_use]
    pub const fn immediate() -> Self {
        Self { pts_time: None }
    }
}

impl<'a> Parse<'a> for SpliceTime {
    type Error = Error;
    fn parse(bytes: &'a [u8]) -> Result<Self> {
        need_in(bytes, 1, "splice_time")?;
        let time_specified = bytes[PTS_TOP_OFFSET] & PTS_FLAG_MASK != 0;
        if time_specified {
            need_in(bytes, Self::LEN_WITH_TIME, "splice_time pts_time")?;
            // 1 bit flag, 6 reserved, then 33 bits of pts_time.
            let pts = ((u64::from(bytes[PTS_TOP_OFFSET] & PTS_TOP_MASK)) << PTS_TOP_SHIFT)
                | (u64::from(bytes[PTS_HIGH_OFFSET]) << PTS_HIGH_SHIFT)
                | (u64::from(bytes[PTS_MID_OFFSET]) << PTS_MID_SHIFT)
                | (u64::from(bytes[PTS_LOW_OFFSET]) << PTS_LOW_SHIFT)
                | u64::from(bytes[PTS_BOTTOM_OFFSET]);
            let pts_time = Clock33::try_new(pts).map_err(|e| {
                Error::InvalidValue(broadcast_common::error::InvalidValue {
                    field: "splice_time.pts_time",
                    value: e.value,
                    reason: "pts_time is a 33-bit field; the flag bit set means the top data bit cannot be set",
                })
            })?;
            Ok(Self {
                pts_time: Some(pts_time),
            })
        } else {
            Ok(Self { pts_time: None })
        }
    }
}

impl Serialize for SpliceTime {
    type Error = Error;
    fn serialized_len(&self) -> usize {
        match self.pts_time {
            Some(_) => Self::LEN_WITH_TIME,
            None => Self::LEN_NO_TIME,
        }
    }

    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        let need = self.serialized_len();
        need_out(buf, need)?;
        match self.pts_time {
            Some(clock) => {
                let pts = clock.ticks();
                // time_specified_flag=1, 6 reserved bits = 1, top pts bit.
                buf[PTS_TOP_OFFSET] = PTS_FLAG_MASK
                    | PTS_RESERVED_BITS
                    | ((pts >> PTS_TOP_SHIFT) as u8 & PTS_TOP_MASK);
                buf[PTS_HIGH_OFFSET] = (pts >> PTS_HIGH_SHIFT) as u8;
                buf[PTS_MID_OFFSET] = (pts >> PTS_MID_SHIFT) as u8;
                buf[PTS_LOW_OFFSET] = (pts >> PTS_LOW_SHIFT) as u8;
                buf[PTS_BOTTOM_OFFSET] = pts as u8;
            }
            None => {
                // time_specified_flag=0, 7 reserved bits = 1.
                buf[PTS_TOP_OFFSET] = NO_TIME_RESERVED_BITS;
            }
        }
        Ok(need)
    }
}

/// `break_duration()` — §9.8.2, Table 15.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct BreakDuration {
    /// `auto_return`: when `true`, the splicer uses `duration` to know when to
    /// return to the network feed (§9.9.2.2).
    pub auto_return: bool,
    /// 33-bit elapsed time of the break in 90 kHz ticks.
    pub duration: u64,
}

impl BreakDuration {
    /// Fixed wire length: 1 byte (auto_return + 6 reserved + top duration bit)
    /// plus 4 bytes for the remaining 32 duration bits.
    pub const LEN: usize = 5;
}

impl<'a> Parse<'a> for BreakDuration {
    type Error = Error;
    fn parse(bytes: &'a [u8]) -> Result<Self> {
        need_in(bytes, Self::LEN, "break_duration")?;
        let auto_return = bytes[PTS_TOP_OFFSET] & AUTO_RETURN_MASK != 0;
        let duration = ((u64::from(bytes[PTS_TOP_OFFSET] & PTS_TOP_MASK)) << PTS_TOP_SHIFT)
            | (u64::from(bytes[PTS_HIGH_OFFSET]) << PTS_HIGH_SHIFT)
            | (u64::from(bytes[PTS_MID_OFFSET]) << PTS_MID_SHIFT)
            | (u64::from(bytes[PTS_LOW_OFFSET]) << PTS_LOW_SHIFT)
            | u64::from(bytes[PTS_BOTTOM_OFFSET]);
        Ok(Self {
            auto_return,
            duration,
        })
    }
}

impl Serialize for BreakDuration {
    type Error = Error;
    fn serialized_len(&self) -> usize {
        Self::LEN
    }

    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        need_out(buf, Self::LEN)?;
        if self.duration > PTS_MAX {
            return Err(Error::FieldOverflow(broadcast_common::len::FieldOverflow {
                field: "break_duration.duration",
                value: self.duration,
                max: PTS_MAX,
            }));
        }
        let d = self.duration;
        // auto_return, 6 reserved bits = 1, top duration bit.
        buf[PTS_TOP_OFFSET] = (u8::from(self.auto_return) << AUTO_RETURN_SHIFT)
            | PTS_RESERVED_BITS
            | ((d >> PTS_TOP_SHIFT) as u8 & PTS_TOP_MASK);
        buf[PTS_HIGH_OFFSET] = (d >> PTS_HIGH_SHIFT) as u8;
        buf[PTS_MID_OFFSET] = (d >> PTS_MID_SHIFT) as u8;
        buf[PTS_LOW_OFFSET] = (d >> PTS_LOW_SHIFT) as u8;
        buf[PTS_BOTTOM_OFFSET] = d as u8;
        Ok(Self::LEN)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ticks_duration_round_trip_exact() {
        // 90_000 ticks = 1 s.
        assert_eq!(
            ticks_to_duration(90_000),
            core::time::Duration::from_secs(1)
        );
        assert_eq!(
            duration_to_ticks(core::time::Duration::from_secs(1), PTS_MAX),
            Some(90_000)
        );
    }

    /// #1102 O1: an out-of-field tick count saturates, it must not wrap to a
    /// small bogus duration.
    #[test]
    fn ticks_to_duration_saturates() {
        assert_eq!(
            ticks_to_duration(u64::MAX),
            core::time::Duration::from_nanos(u64::MAX)
        );
        assert!(ticks_to_duration(u64::MAX) > ticks_to_duration(DURATION_40_MAX));
    }

    #[test]
    fn duration_to_ticks_rejects_over_range() {
        // A duration well past the 33-bit tick capacity must fail to encode.
        // PTS_MAX ticks ≈ 95443.7 s, so 95444 s is over the field's range.
        let over = core::time::Duration::from_secs(95_444);
        assert_eq!(duration_to_ticks(over, PTS_MAX), None);
        // A whole-second duration just inside the range still encodes exactly.
        assert_eq!(
            duration_to_ticks(core::time::Duration::from_secs(95_443), PTS_MAX),
            Some(95_443 * TICKS_PER_SECOND)
        );
    }

    #[test]
    fn pts_add_wraps_at_2pow33() {
        // PTS_MAX + 1 wraps to 0 (carry ignored).
        assert_eq!(pts_add_wrapping(PTS_MAX, 1), 0);
        // Wrap by a large adjustment.
        assert_eq!(pts_add_wrapping(PTS_MAX, PTS_MAX), PTS_MAX - 1);
        // No wrap.
        assert_eq!(pts_add_wrapping(10, 20), 30);
    }

    #[test]
    fn splice_time_round_trip_with_and_without_pts() {
        for st in [
            SpliceTime::at(Clock33::try_new(0x1_2345_6789 & PTS_MAX).expect("in range")),
            SpliceTime::default(),
        ] {
            let bytes = st.try_to_bytes().unwrap();
            let back = SpliceTime::parse(&bytes).unwrap();
            assert_eq!(st, back);
            assert_eq!(back.try_to_bytes().unwrap(), bytes);
        }
    }

    #[test]
    fn splice_time_max_pts_round_trips() {
        let st = SpliceTime::at(Clock33::try_new(PTS_MAX).expect("in range"));
        let bytes = st.try_to_bytes().unwrap();
        assert_eq!(
            SpliceTime::parse(&bytes).unwrap().pts_time,
            Some(Clock33::try_new(PTS_MAX).expect("in range"))
        );
    }

    /// Audit 19b, finding 17: the public construction path accepts only a
    /// validated clock reading, so a value cannot be masked into a different
    /// cue. The old code had two policies for one field — `SpliceTime::with_pts`
    /// masked its argument with `& PTS_MAX` while `serialize_into` rejected
    /// anything above it — so asking for 2^33 produced PTS 0 in a section
    /// that serialized perfectly: the scheduled-cue-silently-changed shape
    /// the audit reported.
    #[test]
    fn splice_time_constructs_only_from_a_validated_clock() {
        assert!(
            Clock33::try_new(1u64 << 33).is_err(),
            "2^33 is off the 33-bit clock, so no SpliceTime can carry it"
        );
        let worst_case = SpliceTime::at(Clock33::try_new(PTS_MAX).expect("in range"));
        let bytes = worst_case.try_to_bytes().unwrap();
        assert_eq!(SpliceTime::parse(&bytes).unwrap(), worst_case);
    }

    #[test]
    fn break_duration_over_33_bits_is_rejected_not_masked() {
        let bd = BreakDuration {
            auto_return: false,
            duration: 1u64 << 33,
        };
        assert_eq!(
            bd.try_to_bytes().err(),
            Some(crate::error::Error::FieldOverflow(
                broadcast_common::len::FieldOverflow {
                    field: "break_duration.duration",
                    value: 1u64 << 33,
                    max: PTS_MAX
                }
            ))
        );
    }

    #[test]
    fn break_duration_round_trip() {
        let bd = BreakDuration {
            auto_return: true,
            duration: PTS_MAX,
        };
        let bytes = bd.try_to_bytes().unwrap();
        let back = BreakDuration::parse(&bytes).unwrap();
        assert_eq!(bd, back);
        assert_eq!(back.try_to_bytes().unwrap(), bytes);
    }
}
