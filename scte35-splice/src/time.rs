//! Time structures and 90 kHz helpers — ANSI/SCTE 35 2023r1 §9.8.1 (Table 14)
//! and §9.8.2 (Table 15).
//!
//! `splice_time()` and `break_duration()` carry 33-bit / 40-bit counts of
//! ticks of the program's 90 kHz clock. The decoded accessors here convert
//! those tick counts to/from [`core::time::Duration`] (the 4.1.0 decoded-field
//! pattern), so callers never re-derive the 90 kHz scaling by hand.

use crate::error::{Error, Result};
use dvb_common::{Parse, Serialize};

/// One second of the program clock in 90 kHz ticks.
pub const TICKS_PER_SECOND: u64 = 90_000;

/// Number of distinct values of a 33-bit field; the `pts_adjustment` /
/// `pts_time` wrap modulus (§9.6.1: carry is ignored on overflow).
pub const PTS_MODULUS: u64 = 1 << 33;

/// Largest value a 33-bit field can hold (`2^33 - 1`).
pub const PTS_MAX: u64 = PTS_MODULUS - 1;

/// Largest value a 40-bit field can hold (`2^40 - 1`); the `break_duration`
/// `duration` range.
pub const DURATION_40_MAX: u64 = (1 << 40) - 1;

/// Convert a 90 kHz tick count to a [`Duration`](core::time::Duration).
///
/// Nanosecond-exact: `ticks * 1e9 / 90_000`. Computed in `u128` so the full
/// 40-bit `break_duration` range cannot overflow.
#[must_use]
pub fn ticks_to_duration(ticks: u64) -> core::time::Duration {
    let nanos = (ticks as u128) * 1_000_000_000 / (TICKS_PER_SECOND as u128);
    // nanos fits u64 for the whole 40-bit range (2^40/90000 s ≈ 12_725_000 s).
    core::time::Duration::from_nanos(nanos as u64)
}

/// Convert a [`Duration`](core::time::Duration) to a 90 kHz tick count,
/// truncating toward zero. Returns `None` if the result would exceed `max`
/// (the field's wire capacity).
#[must_use]
pub fn duration_to_ticks(d: core::time::Duration, max: u64) -> Option<u64> {
    let nanos = d.as_nanos();
    let ticks = nanos * (TICKS_PER_SECOND as u128) / 1_000_000_000;
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
    (pts_time.wrapping_add(pts_adjustment)) % PTS_MODULUS
}

/// `splice_time()` — §9.8.1, Table 14.
///
/// When `pts_time` is present it is a 33-bit count of 90 kHz ticks; absent
/// (`time_specified_flag == 0`) it signals an immediate command.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct SpliceTime {
    /// 33-bit `pts_time` in 90 kHz ticks, or `None` when
    /// `time_specified_flag == 0`.
    pub pts_time: Option<u64>,
}

impl SpliceTime {
    /// Bytes this structure serializes to: 5 when a `pts_time` is present
    /// (1 flag/reserved byte + the 33-bit field spread over 5 bytes), else 1.
    pub const LEN_WITH_TIME: usize = 5;
    /// Length when `time_specified_flag == 0`.
    pub const LEN_NO_TIME: usize = 1;

    /// A `splice_time()` carrying an explicit 33-bit `pts_time` (ticks).
    #[must_use]
    pub fn with_pts(pts_time: u64) -> Self {
        Self {
            pts_time: Some(pts_time & PTS_MAX),
        }
    }

    /// The `pts_time` decoded to a [`Duration`](core::time::Duration), if present.
    #[must_use]
    pub fn pts_time_duration(&self) -> Option<core::time::Duration> {
        self.pts_time.map(ticks_to_duration)
    }

    /// Set `pts_time` from a [`Duration`](core::time::Duration) (truncating to
    /// 90 kHz ticks). Errors if the duration exceeds the 33-bit range.
    pub fn set_pts_time_duration(&mut self, d: core::time::Duration) -> Result<()> {
        let ticks = duration_to_ticks(d, PTS_MAX).ok_or(Error::InvalidValue {
            field: "splice_time.pts_time",
            reason: "duration exceeds 33-bit 90 kHz range",
        })?;
        self.pts_time = Some(ticks);
        Ok(())
    }
}

impl<'a> Parse<'a> for SpliceTime {
    type Error = Error;
    fn parse(bytes: &'a [u8]) -> Result<Self> {
        if bytes.is_empty() {
            return Err(Error::BufferTooShort {
                need: 1,
                have: 0,
                what: "splice_time",
            });
        }
        let time_specified = bytes[0] & 0x80 != 0;
        if time_specified {
            if bytes.len() < Self::LEN_WITH_TIME {
                return Err(Error::BufferTooShort {
                    need: Self::LEN_WITH_TIME,
                    have: bytes.len(),
                    what: "splice_time pts_time",
                });
            }
            // 1 bit flag, 6 reserved, then 33 bits of pts_time.
            let pts = ((u64::from(bytes[0] & 0x01)) << 32)
                | (u64::from(bytes[1]) << 24)
                | (u64::from(bytes[2]) << 16)
                | (u64::from(bytes[3]) << 8)
                | u64::from(bytes[4]);
            Ok(Self {
                pts_time: Some(pts),
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
        if buf.len() < need {
            return Err(Error::OutputBufferTooSmall {
                need,
                have: buf.len(),
            });
        }
        match self.pts_time {
            Some(pts) => {
                let pts = pts & PTS_MAX;
                // time_specified_flag=1, 6 reserved bits = 1, top pts bit.
                buf[0] = 0x80 | 0x7E | ((pts >> 32) as u8 & 0x01);
                buf[1] = (pts >> 24) as u8;
                buf[2] = (pts >> 16) as u8;
                buf[3] = (pts >> 8) as u8;
                buf[4] = pts as u8;
            }
            None => {
                // time_specified_flag=0, 7 reserved bits = 1.
                buf[0] = 0x7F;
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

    /// The break `duration` decoded to a [`Duration`](core::time::Duration).
    #[must_use]
    pub fn duration_value(&self) -> core::time::Duration {
        ticks_to_duration(self.duration)
    }

    /// Set `duration` from a [`Duration`](core::time::Duration) (truncating to
    /// 90 kHz ticks). Errors if it exceeds the 33-bit range.
    pub fn set_duration_value(&mut self, d: core::time::Duration) -> Result<()> {
        self.duration = duration_to_ticks(d, PTS_MAX).ok_or(Error::InvalidValue {
            field: "break_duration.duration",
            reason: "duration exceeds 33-bit 90 kHz range",
        })?;
        Ok(())
    }
}

impl<'a> Parse<'a> for BreakDuration {
    type Error = Error;
    fn parse(bytes: &'a [u8]) -> Result<Self> {
        if bytes.len() < Self::LEN {
            return Err(Error::BufferTooShort {
                need: Self::LEN,
                have: bytes.len(),
                what: "break_duration",
            });
        }
        let auto_return = bytes[0] & 0x80 != 0;
        let duration = ((u64::from(bytes[0] & 0x01)) << 32)
            | (u64::from(bytes[1]) << 24)
            | (u64::from(bytes[2]) << 16)
            | (u64::from(bytes[3]) << 8)
            | u64::from(bytes[4]);
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
        if buf.len() < Self::LEN {
            return Err(Error::OutputBufferTooSmall {
                need: Self::LEN,
                have: buf.len(),
            });
        }
        let d = self.duration & PTS_MAX;
        // auto_return, 6 reserved bits = 1, top duration bit.
        buf[0] = (u8::from(self.auto_return) << 7) | 0x7E | ((d >> 32) as u8 & 0x01);
        buf[1] = (d >> 24) as u8;
        buf[2] = (d >> 16) as u8;
        buf[3] = (d >> 8) as u8;
        buf[4] = d as u8;
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
            SpliceTime::with_pts(0x1_2345_6789 & PTS_MAX),
            SpliceTime::default(),
        ] {
            let bytes = st.to_bytes();
            let back = SpliceTime::parse(&bytes).unwrap();
            assert_eq!(st, back);
            assert_eq!(back.to_bytes(), bytes);
        }
    }

    #[test]
    fn splice_time_max_pts_round_trips() {
        let st = SpliceTime::with_pts(PTS_MAX);
        let bytes = st.to_bytes();
        assert_eq!(SpliceTime::parse(&bytes).unwrap().pts_time, Some(PTS_MAX));
    }

    #[test]
    fn break_duration_round_trip() {
        let bd = BreakDuration {
            auto_return: true,
            duration: PTS_MAX,
        };
        let bytes = bd.to_bytes();
        let back = BreakDuration::parse(&bytes).unwrap();
        assert_eq!(bd, back);
        assert_eq!(back.to_bytes(), bytes);
    }

    #[test]
    fn break_duration_decoded_accessor() {
        let mut bd = BreakDuration::default();
        bd.set_duration_value(core::time::Duration::from_secs(60))
            .unwrap();
        assert_eq!(bd.duration, 60 * 90_000);
        assert_eq!(bd.duration_value(), core::time::Duration::from_secs(60));
    }
}
