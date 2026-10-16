//! RAR over DVB stream descriptor — ETSI TS 102 323 V1.4.1 §5.3.5 Table 12
//! (RNT tag 0x40).
//!
//! Encapsulates a RAR (resolution authority record) for a CRID authority whose
//! data is found on a DVB stream. Permitted in the CRID authority descriptor
//! loop of an RNT section (Table 4); tag 0x40 here "overrides the definition
//! of that descriptor tag value defined in EN 300 468 [1], when this
//! descriptor tag is used inside an RNT" (`network_name_descriptor` in SI).

use crate::descriptors::mjd_time::{MJD_BCD_LEN, MjdBcdTime};
use crate::descriptors::{descriptor_body, write_descriptor_header};
use crate::error::{Error, Result};
use broadcast_common::{Parse, Serialize};

/// Descriptor tag for RAR_over_DVB_stream_descriptor (RNT namespace).
pub const TAG: u8 = 0x40;
const HEADER_LEN: usize = 2;
/// first/last valid date, weighting+flags byte, tsid, onid, service_id,
/// component_tag.
const FIXED_LEN: usize = 2 * MJD_BCD_LEN + 1 + 2 + 2 + 2 + 1;
/// download_start_time(40) + download_period_duration(8) + download_cycle_time(8).
const SCHEDULE_LEN: usize = MJD_BCD_LEN + 2;
const WEIGHTING_SHIFT: u8 = 2;
const WEIGHTING_MASK: u8 = 0x3F;
const COMPLETE_FLAG: u8 = 0x02;
const SCHEDULED_FLAG: u8 = 0x01;

/// The optional download schedule present when `scheduled_flag == 1`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct RarDownloadSchedule {
    /// `download_start_time` — when the CRI service starts to be available
    /// (40-bit MJD+BCD UTC).
    pub download_start_time: MjdBcdTime,
    /// `download_period_duration` — how long it is available, as a count of
    /// 6-minute periods.
    pub download_period_duration: u8,
    /// `download_cycle_time` — minutes for one complete repetition of the CRI
    /// data.
    pub download_cycle_time: u8,
}

/// RAR over DVB stream descriptor.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[cfg_attr(feature = "yoke", derive(yoke::Yokeable))]
pub struct RarOverDvbStreamDescriptor<'a> {
    /// `first_valid_date` — first date the reference can be used (40-bit
    /// MJD+BCD).
    pub first_valid_date: MjdBcdTime,
    /// `last_valid_date` — first date the reference can no longer be used.
    pub last_valid_date: MjdBcdTime,
    /// `weighting` (6 bits) — order hint for several records of one
    /// authority; the largest is tried first.
    pub weighting: u8,
    /// `complete_flag` — the referenced CRI data is complete.
    pub complete_flag: bool,
    /// `transport_stream_id` of the DVB service carrying the CRI; 0x0000 means
    /// ignore (identify by `original_network_id` + `service_id`).
    pub transport_stream_id: u16,
    /// `original_network_id` of the DVB service carrying the CRI.
    pub original_network_id: u16,
    /// `service_id` of the DVB service carrying the CRI.
    pub service_id: u16,
    /// `component_tag` of the elementary stream carrying the CRI.
    pub component_tag: u8,
    /// The download schedule; `scheduled_flag` is 1 iff this is `Some`.
    pub schedule: Option<RarDownloadSchedule>,
    /// Bytes after the last defined field inside `descriptor_length`; the
    /// spec defines none (kept for byte-identical re-serialization).
    pub trailing: &'a [u8],
}

impl<'a> Parse<'a> for RarOverDvbStreamDescriptor<'a> {
    type Error = Error;
    fn parse(bytes: &'a [u8]) -> Result<Self> {
        let body = descriptor_body(
            bytes,
            TAG,
            "RarOverDvbStreamDescriptor",
            "unexpected tag for RAR_over_DVB_stream_descriptor",
        )?;
        if body.len() < FIXED_LEN {
            return Err(Error::InvalidDescriptor {
                tag: TAG,
                reason: "RAR_over_DVB_stream_descriptor body shorter than 18 bytes",
            });
        }
        let flags = body[2 * MJD_BCD_LEN];
        let be16 = |o: usize| u16::from_be_bytes([body[o], body[o + 1]]);
        let o = 2 * MJD_BCD_LEN + 1;
        let (schedule, trailing_start) = if flags & SCHEDULED_FLAG != 0 {
            if body.len() < FIXED_LEN + SCHEDULE_LEN {
                return Err(Error::InvalidDescriptor {
                    tag: TAG,
                    reason: "scheduled_flag set but the download schedule is truncated",
                });
            }
            (
                Some(RarDownloadSchedule {
                    download_start_time: MjdBcdTime::read(body, FIXED_LEN, "download_start_time")?,
                    download_period_duration: body[FIXED_LEN + MJD_BCD_LEN],
                    download_cycle_time: body[FIXED_LEN + MJD_BCD_LEN + 1],
                }),
                FIXED_LEN + SCHEDULE_LEN,
            )
        } else {
            (None, FIXED_LEN)
        };
        Ok(Self {
            first_valid_date: MjdBcdTime::read(body, 0, "first_valid_date")?,
            last_valid_date: MjdBcdTime::read(body, MJD_BCD_LEN, "last_valid_date")?,
            weighting: flags >> WEIGHTING_SHIFT,
            complete_flag: flags & COMPLETE_FLAG != 0,
            transport_stream_id: be16(o),
            original_network_id: be16(o + 2),
            service_id: be16(o + 4),
            component_tag: body[o + 6],
            schedule,
            trailing: &body[trailing_start..],
        })
    }
}

impl Serialize for RarOverDvbStreamDescriptor<'_> {
    type Error = Error;
    fn serialized_len(&self) -> usize {
        HEADER_LEN + FIXED_LEN + self.schedule.map_or(0, |_| SCHEDULE_LEN) + self.trailing.len()
    }
    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        let len = self.serialized_len();
        if buf.len() < len {
            return Err(Error::OutputBufferTooSmall {
                need: len,
                have: buf.len(),
            });
        }
        let weighting =
            broadcast_common::len::fit_mask(self.weighting, WEIGHTING_MASK, "weighting")?;
        write_descriptor_header(buf, TAG, len - HEADER_LEN)?;
        let b = &mut buf[HEADER_LEN..];
        self.first_valid_date.write(b, 0);
        self.last_valid_date.write(b, MJD_BCD_LEN);
        let mut flags = weighting << WEIGHTING_SHIFT;
        if self.complete_flag {
            flags |= COMPLETE_FLAG;
        }
        if self.schedule.is_some() {
            flags |= SCHEDULED_FLAG;
        }
        let o = 2 * MJD_BCD_LEN;
        b[o] = flags;
        b[o + 1..o + 3].copy_from_slice(&self.transport_stream_id.to_be_bytes());
        b[o + 3..o + 5].copy_from_slice(&self.original_network_id.to_be_bytes());
        b[o + 5..o + 7].copy_from_slice(&self.service_id.to_be_bytes());
        b[o + 7] = self.component_tag;
        let mut end = FIXED_LEN;
        if let Some(s) = self.schedule {
            s.download_start_time.write(b, end);
            b[end + MJD_BCD_LEN] = s.download_period_duration;
            b[end + MJD_BCD_LEN + 1] = s.download_cycle_time;
            end += SCHEDULE_LEN;
        }
        b[end..end + self.trailing.len()].copy_from_slice(self.trailing);
        Ok(len)
    }
}

impl<'a> crate::traits::DescriptorDef<'a> for RarOverDvbStreamDescriptor<'a> {
    const TAG: u8 = TAG;
    const NAME: &'static str = "RAR_OVER_DVB_STREAM";
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample(scheduled: bool) -> Vec<u8> {
        let mut v = vec![TAG, 0];
        v.extend_from_slice(&[0xC0, 0x79, 0x12, 0x00, 0x00]); // first_valid_date
        v.extend_from_slice(&[0xC0, 0x80, 0x23, 0x59, 0x59]); // last_valid_date
        // weighting 5, complete 1, scheduled flag
        v.push((5 << 2) | 0x02 | u8::from(scheduled));
        v.extend_from_slice(&[0x12, 0x34, 0x56, 0x78, 0x9A, 0xBC, 0x21]);
        if scheduled {
            v.extend_from_slice(&[0xC0, 0x79, 0x03, 0x30, 0x00, 10, 15]);
        }
        v[1] = (v.len() - 2) as u8;
        v
    }

    #[test]
    fn parse_and_round_trip_unscheduled() {
        let bytes = sample(false);
        let d = RarOverDvbStreamDescriptor::parse(&bytes).unwrap();
        assert_eq!(d.weighting, 5);
        assert!(d.complete_flag && d.schedule.is_none());
        assert_eq!(
            (
                d.transport_stream_id,
                d.original_network_id,
                d.service_id,
                d.component_tag
            ),
            (0x1234, 0x5678, 0x9ABC, 0x21)
        );
        assert_eq!(
            d.first_valid_date.parts(),
            (0xC079, Some(12), Some(0), Some(0))
        );
        let mut out = vec![0u8; d.serialized_len()];
        d.serialize_into(&mut out).unwrap();
        assert_eq!(out, bytes);
    }

    #[test]
    fn parse_and_round_trip_scheduled() {
        let bytes = sample(true);
        let d = RarOverDvbStreamDescriptor::parse(&bytes).unwrap();
        let s = d.schedule.unwrap();
        assert_eq!(
            s.download_start_time.parts(),
            (0xC079, Some(3), Some(30), Some(0))
        );
        assert_eq!(
            (s.download_period_duration, s.download_cycle_time),
            (10, 15)
        );
        let mut out = vec![0u8; d.serialized_len()];
        d.serialize_into(&mut out).unwrap();
        assert_eq!(out, bytes);
    }

    #[test]
    fn trailing_round_trip_and_errors() {
        let mut bytes = sample(false);
        bytes.push(0xEE);
        bytes[1] += 1;
        let d = RarOverDvbStreamDescriptor::parse(&bytes).unwrap();
        assert_eq!(d.trailing, &[0xEE]);
        let mut out = vec![0u8; d.serialized_len()];
        d.serialize_into(&mut out).unwrap();
        assert_eq!(out, bytes);
        // scheduled_flag set but schedule missing
        let mut trunc = sample(true);
        trunc.truncate(trunc.len() - 3);
        trunc[1] -= 3;
        assert!(RarOverDvbStreamDescriptor::parse(&trunc).is_err());
        assert!(RarOverDvbStreamDescriptor::parse(&[TAG, 3, 0, 0, 0]).is_err());
        assert!(RarOverDvbStreamDescriptor::parse(&[0x41, 0]).is_err());
    }

    #[test]
    fn serialize_rejects_over_range_weighting() {
        let bytes = sample(false);
        let mut d = RarOverDvbStreamDescriptor::parse(&bytes).unwrap();
        d.weighting = 64;
        let mut out = vec![0u8; d.serialized_len()];
        assert!(d.serialize_into(&mut out).is_err());
    }
}
