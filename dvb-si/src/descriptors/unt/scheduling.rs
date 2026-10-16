//! scheduling_descriptor — ETSI TS 102 006 §9.5.2.9 Table 31 (UNT tag 0x01),
//! time units per Table 32.
//!
//! Defines when an SSU is available: a campaign start/end (40-bit MJD+BCD
//! UTC, EN 300 468 clause 5.2), a final-availability and a periodicity flag,
//! and the period / duration / estimated cycle time with their units. It can
//! appear more than once in a loop; schedules in the operational loop replace
//! those of the common loop. UNT common and operational loops (Table 19).

use crate::descriptors::mjd_time::{MJD_BCD_LEN, MjdBcdTime};
use crate::descriptors::{descriptor_body, write_descriptor_header};
use crate::error::{Error, Result};
use broadcast_common::{Parse, Serialize};

/// Descriptor tag for scheduling_descriptor (UNT namespace).
pub const TAG: u8 = 0x01;
const HEADER_LEN: usize = 2;
const FIXED_LEN: usize = 2 * MJD_BCD_LEN + 4;
const FINAL_AVAILABILITY: u8 = 0x80;
const PERIODICITY_FLAG: u8 = 0x40;
const UNIT_MASK: u8 = 0x03;
const PERIOD_UNIT_SHIFT: u8 = 4;
const DURATION_UNIT_SHIFT: u8 = 2;

/// Time unit multiplier — ETSI TS 102 006 §9.5.2.9 Table 32 (2 bits).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub enum TimeUnit {
    /// `00` — second.
    Second,
    /// `01` — minute.
    Minute,
    /// `10` — hour.
    Hour,
    /// `11` — day.
    Day,
}

impl TimeUnit {
    /// Decode from the 2-bit value (higher bits ignored). All four values
    /// are defined, so no reserved catch-all exists.
    #[must_use]
    pub fn from_u8(v: u8) -> Self {
        match v & UNIT_MASK {
            0 => Self::Second,
            1 => Self::Minute,
            2 => Self::Hour,
            _ => Self::Day,
        }
    }

    /// Encode to the 2-bit value.
    #[must_use]
    pub const fn to_u8(self) -> u8 {
        match self {
            Self::Second => 0,
            Self::Minute => 1,
            Self::Hour => 2,
            Self::Day => 3,
        }
    }

    /// Spec label (Table 32).
    #[must_use]
    pub fn name(self) -> &'static str {
        match self {
            Self::Second => "second",
            Self::Minute => "minute",
            Self::Hour => "hour",
            Self::Day => "day",
        }
    }
}
broadcast_common::impl_spec_display!(TimeUnit);

/// scheduling_descriptor.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[cfg_attr(feature = "yoke", derive(yoke::Yokeable))]
pub struct SchedulingDescriptor<'a> {
    /// `start_date_time` of the SSU campaign (40-bit MJD+BCD UTC).
    pub start_date_time: MjdBcdTime,
    /// `end_date_time` of the SSU campaign (40-bit MJD+BCD UTC).
    pub end_date_time: MjdBcdTime,
    /// `final_availability` — 1: after this schedule the update will be
    /// unavailable for the foreseeable future.
    pub final_availability: bool,
    /// `periodicity_flag` — 1: available periodically between start and end.
    pub periodicity_flag: bool,
    /// `period_unit` (Table 32).
    pub period_unit: TimeUnit,
    /// `duration_unit` (Table 32).
    pub duration_unit: TimeUnit,
    /// `estimated_cycle_time_unit` (Table 32).
    pub estimated_cycle_time_unit: TimeUnit,
    /// `period` — repetition period, in `period_unit`s.
    pub period: u8,
    /// `duration` — availability at the start of each period, in
    /// `duration_unit`s.
    pub duration: u8,
    /// `estimated_cycle_time` — in `estimated_cycle_time_unit`s; 0 = no
    /// meaning.
    pub estimated_cycle_time: u8,
    /// `private_data_byte`s — opaque operator data.
    pub private_data: &'a [u8],
}

impl<'a> Parse<'a> for SchedulingDescriptor<'a> {
    type Error = Error;
    fn parse(bytes: &'a [u8]) -> Result<Self> {
        let body = descriptor_body(
            bytes,
            TAG,
            "SchedulingDescriptor",
            "unexpected tag for scheduling_descriptor",
        )?;
        if body.len() < FIXED_LEN {
            return Err(Error::InvalidDescriptor {
                tag: TAG,
                reason: "scheduling_descriptor body shorter than 14 bytes",
            });
        }
        let flags = body[2 * MJD_BCD_LEN];
        let units = body[2 * MJD_BCD_LEN + 1];
        Ok(Self {
            start_date_time: MjdBcdTime::read(body, 0, "scheduling_descriptor start_date_time")?,
            end_date_time: MjdBcdTime::read(body, MJD_BCD_LEN, "scheduling_descriptor end_date_time")?,
            final_availability: flags & FINAL_AVAILABILITY != 0,
            periodicity_flag: flags & PERIODICITY_FLAG != 0,
            // byte 10: final_availability(1) periodicity_flag(1) period_unit(2)
            // duration_unit(2) estimated_cycle_time_unit(2)
            period_unit: TimeUnit::from_u8(flags >> PERIOD_UNIT_SHIFT),
            duration_unit: TimeUnit::from_u8(flags >> DURATION_UNIT_SHIFT),
            estimated_cycle_time_unit: TimeUnit::from_u8(flags),
            period: units,
            duration: body[2 * MJD_BCD_LEN + 2],
            estimated_cycle_time: body[2 * MJD_BCD_LEN + 3],
            private_data: &body[FIXED_LEN..],
        })
    }
}

impl Serialize for SchedulingDescriptor<'_> {
    type Error = Error;
    fn serialized_len(&self) -> usize {
        HEADER_LEN + FIXED_LEN + self.private_data.len()
    }
    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        let len = self.serialized_len();
        if buf.len() < len {
            return Err(Error::OutputBufferTooSmall {
                need: len,
                have: buf.len(),
            });
        }
        write_descriptor_header(buf, TAG, len - HEADER_LEN)?;
        let b = &mut buf[HEADER_LEN..];
        self.start_date_time.write(b, 0);
        self.end_date_time.write(b, MJD_BCD_LEN);
        let mut flags = (self.period_unit.to_u8() << PERIOD_UNIT_SHIFT)
            | (self.duration_unit.to_u8() << DURATION_UNIT_SHIFT)
            | self.estimated_cycle_time_unit.to_u8();
        if self.final_availability {
            flags |= FINAL_AVAILABILITY;
        }
        if self.periodicity_flag {
            flags |= PERIODICITY_FLAG;
        }
        b[2 * MJD_BCD_LEN] = flags;
        b[2 * MJD_BCD_LEN + 1] = self.period;
        b[2 * MJD_BCD_LEN + 2] = self.duration;
        b[2 * MJD_BCD_LEN + 3] = self.estimated_cycle_time;
        b[FIXED_LEN..FIXED_LEN + self.private_data.len()].copy_from_slice(self.private_data);
        Ok(len)
    }
}

impl<'a> crate::traits::DescriptorDef<'a> for SchedulingDescriptor<'a> {
    const TAG: u8 = TAG;
    const NAME: &'static str = "SCHEDULING";
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_fields_and_round_trip() {
        // start MJD 0xC079 12:45:00, end 0xC07A 01:02:03; flags: final=1
        // periodic=1 period=hour(10) duration=minute(01) cycle=day(11).
        let bytes = [
            TAG, 15, 0xC0, 0x79, 0x12, 0x45, 0x00, 0xC0, 0x7A, 0x01, 0x02, 0x03, 0b1110_0111, 6, 30,
            4, 0xEE,
        ];
        let d = SchedulingDescriptor::parse(&bytes).unwrap();
        assert_eq!(d.start_date_time.parts(), (0xC079, Some(12), Some(45), Some(0)));
        assert_eq!(d.end_date_time.parts(), (0xC07A, Some(1), Some(2), Some(3)));
        assert!(d.final_availability && d.periodicity_flag);
        assert_eq!(d.period_unit, TimeUnit::Hour);
        assert_eq!(d.duration_unit, TimeUnit::Minute);
        assert_eq!(d.estimated_cycle_time_unit, TimeUnit::Day);
        assert_eq!((d.period, d.duration, d.estimated_cycle_time), (6, 30, 4));
        assert_eq!(d.private_data, &[0xEE]);
        let mut out = vec![0u8; d.serialized_len()];
        d.serialize_into(&mut out).unwrap();
        assert_eq!(out, bytes);
    }

    #[test]
    fn every_flag_byte_round_trips() {
        for f in 0..=u8::MAX {
            let bytes = [TAG, 14, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, f, 1, 2, 3];
            let d = SchedulingDescriptor::parse(&bytes).unwrap();
            let mut out = vec![0u8; d.serialized_len()];
            d.serialize_into(&mut out).unwrap();
            assert_eq!(out, bytes, "flags {f:#04x}");
        }
    }

    #[test]
    fn rejects_short_body_and_wrong_tag() {
        assert!(SchedulingDescriptor::parse(&[TAG, 13, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0]).is_err());
        assert!(SchedulingDescriptor::parse(&[0x02, 0]).is_err());
        assert_eq!(TimeUnit::Day.to_string(), "day");
    }
}
