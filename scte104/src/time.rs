//! Time structures — ANSI/SCTE 104 2023 §12.4, §12.5.
//!
//! - [`Time`] (§12.4): 8-byte structure (seconds + microseconds since GPS epoch
//!   1980-01-06 00:00:00 UTC), used in `alive_request_data()` /
//!   `alive_response_data()`.
//! - [`Timestamp`] (§12.5): variable-length timestamp with `time_type`
//!   discriminator (none / UTC / SMPTE VITC / GPI), used in
//!   `multiple_operation_message()` header.
//! - [`SpliceScheduleTime`]: bare 4-byte u32 seconds (used in
//!   `schedule_definition_data()` / `schedule_component_mode_request_data()`).

use crate::error::{Error, Result};
use dvb_common::{Parse, Serialize};

/// `time()` — §12.4.
///
/// 8 bytes: 4-byte big-endian seconds since GPS epoch (1980-01-06 00:00:00 UTC)
/// + 4-byte big-endian microseconds (0..=999_999).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct Time {
    /// Seconds since 1980-01-06 00:00:00 UTC, including leap seconds.
    pub seconds: u32,
    /// Microsecond offset within the second.
    pub microseconds: u32,
}

/// Fixed wire length for `time()`.
pub const TIME_LEN: usize = 8;

impl Time {
    /// Construct a zero `Time` (signals unused / time sync not active).
    #[must_use]
    pub fn zero() -> Self {
        Self {
            seconds: 0,
            microseconds: 0,
        }
    }
}

impl<'a> Parse<'a> for Time {
    type Error = Error;
    fn parse(bytes: &'a [u8]) -> Result<Self> {
        if bytes.len() < TIME_LEN {
            return Err(Error::BufferTooShort {
                need: TIME_LEN,
                have: bytes.len(),
                what: "time()",
            });
        }
        Ok(Self {
            seconds: u32::from_be_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]),
            microseconds: u32::from_be_bytes([bytes[4], bytes[5], bytes[6], bytes[7]]),
        })
    }
}

impl Serialize for Time {
    type Error = Error;
    fn serialized_len(&self) -> usize {
        TIME_LEN
    }
    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        if buf.len() < TIME_LEN {
            return Err(Error::OutputBufferTooSmall {
                need: TIME_LEN,
                have: buf.len(),
            });
        }
        buf[0..4].copy_from_slice(&self.seconds.to_be_bytes());
        buf[4..8].copy_from_slice(&self.microseconds.to_be_bytes());
        Ok(TIME_LEN)
    }
}

/// Bare 4-byte seconds value used in `schedule_definition_data()` and
/// `schedule_component_mode_request_data()` — §9.7.2.1.
///
/// Seconds since 1980-01-06 00:00:00 UTC (same epoch as [`Time`], sans microseconds).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct SpliceScheduleTime {
    /// Seconds since 1980-01-06 00:00:00 UTC.
    pub seconds: u32,
}

/// Fixed wire length for `SpliceScheduleTime`.
pub const SCHEDULE_TIME_LEN: usize = 4;

impl<'a> Parse<'a> for SpliceScheduleTime {
    type Error = Error;
    fn parse(bytes: &'a [u8]) -> Result<Self> {
        if bytes.len() < SCHEDULE_TIME_LEN {
            return Err(Error::BufferTooShort {
                need: SCHEDULE_TIME_LEN,
                have: bytes.len(),
                what: "schedule time",
            });
        }
        Ok(Self {
            seconds: u32::from_be_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]),
        })
    }
}

impl Serialize for SpliceScheduleTime {
    type Error = Error;
    fn serialized_len(&self) -> usize {
        SCHEDULE_TIME_LEN
    }
    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        if buf.len() < SCHEDULE_TIME_LEN {
            return Err(Error::OutputBufferTooSmall {
                need: SCHEDULE_TIME_LEN,
                have: buf.len(),
            });
        }
        buf[0..4].copy_from_slice(&self.seconds.to_be_bytes());
        Ok(SCHEDULE_TIME_LEN)
    }
}

/// `time_type` values for [`Timestamp`] — §12.5.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub enum TimeType {
    /// No timestamp — immediate processing.
    None,
    /// UTC time (seconds since NTP epoch 1900-01-01 + microseconds).
    Utc,
    /// SMPTE VITC timecode (hours, minutes, seconds, frames).
    SmpteVitc,
    /// GPI trigger (GPI number + edge).
    Gpi,
    /// Reserved `time_type` value.
    Reserved(u8),
}

impl TimeType {
    /// Parse from the `time_type` byte in the wire format.
    #[must_use]
    pub fn from_byte(b: u8) -> Self {
        match b {
            0 => Self::None,
            1 => Self::Utc,
            2 => Self::SmpteVitc,
            3 => Self::Gpi,
            _ => Self::Reserved(b),
        }
    }

    /// Return the wire byte for this `time_type`.
    #[must_use]
    pub fn to_byte(self) -> u8 {
        match self {
            Self::None => 0,
            Self::Utc => 1,
            Self::SmpteVitc => 2,
            Self::Gpi => 3,
            Self::Reserved(b) => b,
        }
    }

    /// Label per the project's `name()` convention.
    #[must_use]
    pub fn name(&self) -> &'static str {
        match self {
            Self::None => "none",
            Self::Utc => "utc",
            Self::SmpteVitc => "smpte vitc",
            Self::Gpi => "gpi",
            Self::Reserved(_) => "reserved",
        }
    }
}

dvb_common::impl_spec_display!(TimeType, Reserved);

/// UTC payload of [`Timestamp`] when `time_type == 1` — §12.5.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct UtcTimestamp {
    /// Seconds since NTP epoch (1900-01-01 00:00:00 UTC).
    pub seconds: u32,
    /// Microsecond offset (0..=999_999).
    pub microseconds: u16,
}

/// SMPTE VITC payload of [`Timestamp`] when `time_type == 2` — §12.5.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct SmpteVitcTimestamp {
    /// Hours (0-23).
    pub hours: u8,
    /// Minutes (0-59).
    pub minutes: u8,
    /// Seconds (0-59).
    pub seconds: u8,
    /// Frames.
    pub frames: u8,
}

/// GPI payload of [`Timestamp`] when `time_type == 3` — §12.5.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct GpiTimestamp {
    /// GPI port number.
    pub gpi_number: u8,
    /// GPI edge: `0` = falling, `1` = rising.
    pub gpi_edge: u8,
}

/// `timestamp()` — §12.5.
///
/// Variable-length: 1 byte for type=none, 7 for UTC, 5 for VITC, 3 for GPI.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub enum Timestamp {
    /// No timestamp — immediate processing.
    #[default]
    None,
    /// UTC time (seconds since 1900-01-01 + microseconds).
    Utc(UtcTimestamp),
    /// SMPTE VITC timecode.
    SmpteVitc(SmpteVitcTimestamp),
    /// GPI trigger.
    Gpi(GpiTimestamp),
}

impl Timestamp {
    /// The `time_type` byte this variant encodes to.
    #[must_use]
    pub fn time_type(&self) -> u8 {
        match self {
            Self::None => 0,
            Self::Utc(_) => 1,
            Self::SmpteVitc(_) => 2,
            Self::Gpi(_) => 3,
        }
    }
}

impl<'a> Parse<'a> for Timestamp {
    type Error = Error;
    fn parse(bytes: &'a [u8]) -> Result<Self> {
        if bytes.is_empty() {
            return Err(Error::BufferTooShort {
                need: 1,
                have: 0,
                what: "timestamp time_type",
            });
        }
        match bytes[0] {
            0 => Ok(Self::None),
            1 => {
                if bytes.len() < 7 {
                    return Err(Error::BufferTooShort {
                        need: 7,
                        have: bytes.len(),
                        what: "timestamp UTC",
                    });
                }
                Ok(Self::Utc(UtcTimestamp {
                    seconds: u32::from_be_bytes([bytes[1], bytes[2], bytes[3], bytes[4]]),
                    microseconds: u16::from_be_bytes([bytes[5], bytes[6]]),
                }))
            }
            2 => {
                if bytes.len() < 5 {
                    return Err(Error::BufferTooShort {
                        need: 5,
                        have: bytes.len(),
                        what: "timestamp VITC",
                    });
                }
                Ok(Self::SmpteVitc(SmpteVitcTimestamp {
                    hours: bytes[1],
                    minutes: bytes[2],
                    seconds: bytes[3],
                    frames: bytes[4],
                }))
            }
            3 => {
                if bytes.len() < 3 {
                    return Err(Error::BufferTooShort {
                        need: 3,
                        have: bytes.len(),
                        what: "timestamp GPI",
                    });
                }
                Ok(Self::Gpi(GpiTimestamp {
                    gpi_number: bytes[1],
                    gpi_edge: bytes[2],
                }))
            }
            _ => Err(Error::InvalidValue {
                field: "timestamp time_type",
                reason: "unknown time_type value",
            }),
        }
    }
}

impl Serialize for Timestamp {
    type Error = Error;
    fn serialized_len(&self) -> usize {
        match self {
            Self::None => 1,
            Self::Utc(_) => 7,
            Self::SmpteVitc(_) => 5,
            Self::Gpi(_) => 3,
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
        match self {
            Self::None => {
                buf[0] = 0;
            }
            Self::Utc(u) => {
                buf[0] = 1;
                buf[1..5].copy_from_slice(&u.seconds.to_be_bytes());
                buf[5..7].copy_from_slice(&u.microseconds.to_be_bytes());
            }
            Self::SmpteVitc(v) => {
                buf[0] = 2;
                buf[1] = v.hours;
                buf[2] = v.minutes;
                buf[3] = v.seconds;
                buf[4] = v.frames;
            }
            Self::Gpi(g) => {
                buf[0] = 3;
                buf[1] = g.gpi_number;
                buf[2] = g.gpi_edge;
            }
        }
        Ok(need)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn time_round_trip() {
        let t = Time {
            seconds: 0x1234_5678,
            microseconds: 0x000F_4240, // 1_000_000
        };
        let bytes = t.to_bytes();
        assert_eq!(bytes.len(), TIME_LEN);
        let back = Time::parse(&bytes).unwrap();
        assert_eq!(t, back);
    }

    #[test]
    fn timestamp_none() {
        let ts = Timestamp::None;
        assert_eq!(ts.serialized_len(), 1);
        let bytes = ts.to_bytes();
        assert_eq!(bytes, [0]);
        let back = Timestamp::parse(&bytes).unwrap();
        assert!(matches!(back, Timestamp::None));
    }

    #[test]
    fn timestamp_utc_round_trip() {
        let ts = Timestamp::Utc(UtcTimestamp {
            seconds: 0x6000_0000,
            microseconds: 50_000,
        });
        let bytes = ts.to_bytes();
        assert_eq!(bytes.len(), 7);
        let back = Timestamp::parse(&bytes).unwrap();
        match back {
            Timestamp::Utc(u) => {
                assert_eq!(u.seconds, 0x6000_0000);
                assert_eq!(u.microseconds, 50_000);
            }
            _ => panic!("expected UTC"),
        }
        // mutate field changes bytes
        let mut ts2 = ts;
        match &mut ts2 {
            Timestamp::Utc(u) => u.seconds = 1,
            _ => unreachable!(),
        }
        assert_ne!(ts2.to_bytes(), bytes);
    }

    #[test]
    fn timestamp_vitc_round_trip() {
        let ts = Timestamp::SmpteVitc(SmpteVitcTimestamp {
            hours: 10,
            minutes: 30,
            seconds: 45,
            frames: 15,
        });
        let bytes = ts.to_bytes();
        assert_eq!(bytes.len(), 5);
        assert_eq!(bytes[0], 2);
        let back = Timestamp::parse(&bytes).unwrap();
        match back {
            Timestamp::SmpteVitc(v) => {
                assert_eq!(v.hours, 10);
                assert_eq!(v.minutes, 30);
                assert_eq!(v.seconds, 45);
                assert_eq!(v.frames, 15);
            }
            _ => panic!("expected VITC"),
        }
    }

    #[test]
    fn timestamp_gpi_round_trip() {
        let ts = Timestamp::Gpi(GpiTimestamp {
            gpi_number: 5,
            gpi_edge: 1,
        });
        let bytes = ts.to_bytes();
        assert_eq!(bytes.len(), 3);
        assert_eq!(bytes[0], 3);
        let back = Timestamp::parse(&bytes).unwrap();
        match back {
            Timestamp::Gpi(g) => {
                assert_eq!(g.gpi_number, 5);
                assert_eq!(g.gpi_edge, 1);
            }
            _ => panic!("expected GPI"),
        }
    }

    #[test]
    fn splice_schedule_time_round_trip() {
        let t = SpliceScheduleTime {
            seconds: 0x6000_0000,
        };
        let bytes = t.to_bytes();
        assert_eq!(bytes.len(), SCHEDULE_TIME_LEN);
        let back = SpliceScheduleTime::parse(&bytes).unwrap();
        assert_eq!(t, back);
    }

    #[test]
    fn time_type_name() {
        assert_eq!(TimeType::None.name(), "none");
        assert_eq!(TimeType::Utc.name(), "utc");
        assert_eq!(TimeType::SmpteVitc.name(), "smpte vitc");
        assert_eq!(TimeType::Gpi.name(), "gpi");
        assert_eq!(TimeType::Reserved(0xFF).name(), "reserved");
    }
}
