//! Local Time Offset Descriptor — ETSI EN 300 468 §6.2.20 (tag 0x58).
//!
//! Carried inside the TOT (Time Offset Table) on PID 0x0014. Signals per-
//! country offsets from UTC plus any upcoming DST transition.

use super::descriptor_body;
use crate::error::{Error, Result};
use crate::text::LangCode;
use dvb_common::{Parse, Serialize};

/// Descriptor tag for local_time_offset_descriptor.
pub const TAG: u8 = 0x58;
const HEADER_LEN: usize = 2;
const ENTRY_LEN: usize = 13;
const POLARITY_MASK: u8 = 0x01;
const REGION_ID_MASK: u8 = 0xFC;
const RESERVED_BIT_MASK: u8 = 0x02;

/// One per-country offset entry.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct LocalTimeOffsetEntry {
    /// ISO 3166 alpha country code.
    pub country_code: LangCode,
    /// 6-bit country_region_id for sub-national regions.
    pub country_region_id: u8,
    /// Polarity: false = offset is positive (local = UTC + offset),
    /// true = offset is negative (local = UTC − offset).
    pub local_time_offset_negative: bool,
    /// 16-bit BCD HHMM local time offset.
    pub local_time_offset_bcd: u16,
    /// 40-bit MJD+BCD UTC raw bytes of the DST/offset transition moment.
    /// Private: use [`Self::time_of_change_parts`] for decoded access, or
    /// [`Self::time_of_change`] (chrono feature) for a `DateTime`.
    time_of_change_raw: [u8; 5],
    /// 16-bit BCD HHMM next offset (applied after `time_of_change`).
    pub next_time_offset_bcd: u16,
}

impl LocalTimeOffsetEntry {
    /// Create a new entry with all fields specified.
    #[must_use]
    pub fn new(
        country_code: LangCode,
        country_region_id: u8,
        local_time_offset_negative: bool,
        local_time_offset_bcd: u16,
        time_of_change_raw: [u8; 5],
        next_time_offset_bcd: u16,
    ) -> Self {
        Self {
            country_code,
            country_region_id,
            local_time_offset_negative,
            local_time_offset_bcd,
            time_of_change_raw,
            next_time_offset_bcd,
        }
    }

    /// Decode `time_of_change_raw` into its MJD + BCD HHMMSS components
    /// without requiring the `chrono` feature.
    ///
    /// Returns `(mjd, hours, minutes, seconds)`. Each BCD field is `None` if
    /// its nibbles are non-decimal.
    #[must_use]
    pub fn time_of_change_parts(&self) -> (u16, Option<u8>, Option<u8>, Option<u8>) {
        let mjd = u16::from_be_bytes([self.time_of_change_raw[0], self.time_of_change_raw[1]]);
        let h = dvb_common::bcd::from_bcd_byte(self.time_of_change_raw[2]);
        let m = dvb_common::bcd::from_bcd_byte(self.time_of_change_raw[3]);
        let s = dvb_common::bcd::from_bcd_byte(self.time_of_change_raw[4]);
        (mjd, h, m, s)
    }

    /// The raw 5-byte MJD+BCD time-of-change field (for serialization).
    #[must_use]
    pub fn time_of_change_raw(&self) -> [u8; 5] {
        self.time_of_change_raw
    }

    /// Set the `time_of_change_raw` field directly.
    pub fn set_time_of_change_raw(&mut self, raw: [u8; 5]) {
        self.time_of_change_raw = raw;
    }
}

/// Decode a BCD `HHMM` offset to a signed [`chrono::Duration`] (negative when
/// `negative`). `None` if a BCD nibble is out of range.
#[cfg(feature = "chrono")]
fn decode_hhmm(bcd: u16, negative: bool) -> Option<chrono::Duration> {
    let h = dvb_common::bcd::from_bcd_byte((bcd >> 8) as u8)?;
    let m = dvb_common::bcd::from_bcd_byte((bcd & 0xFF) as u8)?;
    let mins = i64::from(h) * 60 + i64::from(m);
    Some(chrono::Duration::minutes(if negative {
        -mins
    } else {
        mins
    }))
}

/// Encode a signed offset to `(negative, BCD HHMM)`. `None` if the magnitude is
/// 100 hours or longer.
#[cfg(feature = "chrono")]
fn encode_hhmm(offset: chrono::Duration) -> Option<(bool, u16)> {
    let negative = offset < chrono::Duration::zero();
    let total_min = offset.num_minutes().unsigned_abs();
    let h = total_min / 60;
    let m = total_min % 60;
    if h > 99 {
        return None;
    }
    let hb = dvb_common::bcd::to_bcd_byte(h as u8)?;
    let mb = dvb_common::bcd::to_bcd_byte(m as u8)?;
    Some((negative, (u16::from(hb) << 8) | u16::from(mb)))
}

#[cfg(feature = "chrono")]
impl LocalTimeOffsetEntry {
    /// Decode `local_time_offset` (BCD `HHMM`, signed by
    /// `local_time_offset_negative`) to a [`chrono::Duration`]. `None` if the
    /// BCD nibbles are out of range.
    #[must_use]
    pub fn local_time_offset(&self) -> Option<chrono::Duration> {
        decode_hhmm(self.local_time_offset_bcd, self.local_time_offset_negative)
    }

    /// Decode `next_time_offset` (BCD `HHMM`) to a [`chrono::Duration`]. It
    /// shares the single `local_time_offset_negative` polarity bit (EN 300 468
    /// §6.2.20). `None` if the BCD nibbles are out of range.
    #[must_use]
    pub fn next_time_offset(&self) -> Option<chrono::Duration> {
        decode_hhmm(self.next_time_offset_bcd, self.local_time_offset_negative)
    }

    /// Decode `time_of_change_raw` (16-bit MJD + 24-bit BCD UTC) to a UTC
    /// datetime. `None` if the date/time fields are out of range.
    #[must_use]
    pub fn time_of_change(&self) -> Option<chrono::DateTime<chrono::Utc>> {
        dvb_common::time::decode_mjd_bcd_utc(self.time_of_change_raw)
    }

    /// Set the `time_of_change`, encoding it into the 40-bit raw field.
    ///
    /// # Errors
    /// [`ValueOutOfRange`](crate::Error::ValueOutOfRange) if the date is
    /// outside the representable 16-bit MJD range.
    pub fn set_time_of_change(&mut self, dt: chrono::DateTime<chrono::Utc>) -> Result<()> {
        self.time_of_change_raw =
            dvb_common::time::encode_mjd_bcd_utc(dt).ok_or(Error::ValueOutOfRange {
                field: "LocalTimeOffsetEntry::time_of_change",
                reason: "date not representable in 16-bit MJD",
            })?;
        Ok(())
    }

    /// Set both offsets and the shared polarity bit from signed durations.
    ///
    /// The wire format carries one polarity bit for both offsets, so `local`
    /// and `next` must share a sign (zero matches either).
    ///
    /// # Errors
    /// [`ValueOutOfRange`](crate::Error::ValueOutOfRange) if the two
    /// offsets disagree in sign or a magnitude is 100 hours or longer.
    pub fn set_offsets(&mut self, local: chrono::Duration, next: chrono::Duration) -> Result<()> {
        let oor = |reason| Error::ValueOutOfRange {
            field: "LocalTimeOffsetEntry offsets",
            reason,
        };
        let local_neg = local < chrono::Duration::zero();
        let next_neg = next < chrono::Duration::zero();
        if local_neg != next_neg && !local.is_zero() && !next.is_zero() {
            return Err(oor("local and next offsets must share a sign"));
        }
        let (lneg, lbcd) = encode_hhmm(local).ok_or(oor("local offset magnitude too large"))?;
        let (nneg, nbcd) = encode_hhmm(next).ok_or(oor("next offset magnitude too large"))?;
        self.local_time_offset_negative = lneg || nneg;
        self.local_time_offset_bcd = lbcd;
        self.next_time_offset_bcd = nbcd;
        Ok(())
    }
}

/// Local Time Offset Descriptor.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct LocalTimeOffsetDescriptor {
    /// Entries in wire order.
    pub entries: Vec<LocalTimeOffsetEntry>,
}

impl<'a> Parse<'a> for LocalTimeOffsetDescriptor {
    type Error = crate::error::Error;
    fn parse(bytes: &'a [u8]) -> Result<Self> {
        let body = descriptor_body(
            bytes,
            TAG,
            "LocalTimeOffsetDescriptor",
            "unexpected tag for local_time_offset_descriptor",
        )?;
        if body.len() % ENTRY_LEN != 0 {
            return Err(Error::InvalidDescriptor {
                tag: TAG,
                reason: "descriptor_length must be a multiple of 13",
            });
        }
        let mut entries = Vec::with_capacity(body.len() / ENTRY_LEN);
        let mut offset = 0;
        while offset < body.len() {
            let country_code = LangCode([body[offset], body[offset + 1], body[offset + 2]]);
            let flags = body[offset + 3];
            let country_region_id = (flags & REGION_ID_MASK) >> 2;
            let local_time_offset_negative = flags & POLARITY_MASK != 0;
            let local_time_offset_bcd = u16::from_be_bytes([body[offset + 4], body[offset + 5]]);
            let mut time_of_change_raw = [0u8; 5];
            time_of_change_raw.copy_from_slice(&body[offset + 6..offset + 11]);
            let next_time_offset_bcd = u16::from_be_bytes([body[offset + 11], body[offset + 12]]);
            entries.push(LocalTimeOffsetEntry {
                country_code,
                country_region_id,
                local_time_offset_negative,
                local_time_offset_bcd,
                time_of_change_raw,
                next_time_offset_bcd,
            });
            offset += ENTRY_LEN;
        }
        Ok(Self { entries })
    }
}

impl Serialize for LocalTimeOffsetDescriptor {
    type Error = crate::error::Error;
    fn serialized_len(&self) -> usize {
        HEADER_LEN + ENTRY_LEN * self.entries.len()
    }

    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        let len = self.serialized_len();
        if buf.len() < len {
            return Err(Error::OutputBufferTooSmall {
                need: len,
                have: buf.len(),
            });
        }
        buf[0] = TAG;
        buf[1] = (len - HEADER_LEN) as u8;
        let mut offset = HEADER_LEN;
        for entry in &self.entries {
            buf[offset..offset + 3].copy_from_slice(&entry.country_code.0);
            let flags = ((entry.country_region_id << 2) & REGION_ID_MASK)
                | RESERVED_BIT_MASK
                | if entry.local_time_offset_negative {
                    POLARITY_MASK
                } else {
                    0
                };
            buf[offset + 3] = flags;
            buf[offset + 4..offset + 6].copy_from_slice(&entry.local_time_offset_bcd.to_be_bytes());
            buf[offset + 6..offset + 11].copy_from_slice(&entry.time_of_change_raw);
            buf[offset + 11..offset + 13]
                .copy_from_slice(&entry.next_time_offset_bcd.to_be_bytes());
            offset += ENTRY_LEN;
        }
        Ok(len)
    }
}
impl<'a> crate::traits::DescriptorDef<'a> for LocalTimeOffsetDescriptor {
    const TAG: u8 = TAG;
    const NAME: &'static str = "LOCAL_TIME_OFFSET";
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_single_entry() {
        let bytes = [
            TAG, 13, 0x46, 0x52, 0x41, 0x02, 0x01, 0x00, 0xAB, 0xCD, 0xEF, 0x12, 0x34, 0x02, 0x00,
        ];
        let d = LocalTimeOffsetDescriptor::parse(&bytes).unwrap();
        assert_eq!(d.entries.len(), 1);
        assert_eq!(d.entries[0].country_code, LangCode([0x46, 0x52, 0x41]));
        assert_eq!(d.entries[0].country_region_id, 0);
        assert!(!d.entries[0].local_time_offset_negative);
        assert_eq!(d.entries[0].local_time_offset_bcd, 0x0100);
        assert_eq!(
            d.entries[0].time_of_change_raw(),
            [0xAB, 0xCD, 0xEF, 0x12, 0x34]
        );
        assert_eq!(d.entries[0].next_time_offset_bcd, 0x0200);
    }

    #[test]
    fn time_of_change_parts_decoded() {
        let bytes = [
            TAG, 13, 0x46, 0x52, 0x41, 0x02, 0x01, 0x00, 0xAB, 0xCD, 0xEF, 0x12, 0x34, 0x02, 0x00,
        ];
        let d = LocalTimeOffsetDescriptor::parse(&bytes).unwrap();
        let (mjd, h, m, s) = d.entries[0].time_of_change_parts();
        assert_eq!(mjd, 0xABCD);
        assert_eq!(h, None);
        assert_eq!(m, Some(12));
        assert_eq!(s, Some(34));
    }

    #[test]
    fn time_of_change_parts_valid_bcd() {
        let entry = LocalTimeOffsetEntry {
            country_code: LangCode([0x47, 0x42, 0x52]),
            country_region_id: 0,
            local_time_offset_negative: false,
            local_time_offset_bcd: 0x0000,
            time_of_change_raw: [0xC0, 0x3E, 0x12, 0x30, 0x00],
            next_time_offset_bcd: 0x0100,
        };
        let (mjd, h, m, s) = entry.time_of_change_parts();
        assert_eq!(mjd, 0xC03E);
        assert_eq!(h, Some(12));
        assert_eq!(m, Some(30));
        assert_eq!(s, Some(0));
    }

    #[test]
    fn parse_multiple_entries_preserves_order() {
        let bytes = [
            TAG, 26, 0x46, 0x52, 0x41, 0x02, 0x01, 0x00, 0xAB, 0xCD, 0xEF, 0x12, 0x34, 0x02, 0x00,
            0x47, 0x42, 0x52, 0x06, 0x00, 0x00, 0x11, 0x22, 0x33, 0x44, 0x55, 0x01, 0x00,
        ];
        let d = LocalTimeOffsetDescriptor::parse(&bytes).unwrap();
        assert_eq!(d.entries.len(), 2);
        assert_eq!(d.entries[0].country_code, LangCode([0x46, 0x52, 0x41]));
        assert_eq!(d.entries[1].country_code, LangCode([0x47, 0x42, 0x52]));
    }

    #[test]
    fn parse_extracts_polarity_negative() {
        let bytes = [
            TAG, 13, 0x46, 0x52, 0x41, 0x03, 0x01, 0x00, 0xAB, 0xCD, 0xEF, 0x12, 0x34, 0x02, 0x00,
        ];
        let d = LocalTimeOffsetDescriptor::parse(&bytes).unwrap();
        assert!(d.entries[0].local_time_offset_negative);
    }

    #[test]
    fn parse_extracts_country_region_id() {
        let bytes = [
            TAG, 13, 0x46, 0x52, 0x41, 0x1A, 0x01, 0x00, 0xAB, 0xCD, 0xEF, 0x12, 0x34, 0x02, 0x00,
        ];
        let d = LocalTimeOffsetDescriptor::parse(&bytes).unwrap();
        assert_eq!(d.entries[0].country_region_id, 6);
    }

    #[test]
    fn parse_rejects_wrong_tag() {
        let err = LocalTimeOffsetDescriptor::parse(&[
            0x59, 13, 0x46, 0x52, 0x41, 0x02, 0x01, 0x00, 0xAB, 0xCD, 0xEF, 0x12, 0x34, 0x02, 0x00,
        ])
        .unwrap_err();
        assert!(matches!(err, Error::InvalidDescriptor { tag: 0x59, .. }));
    }

    #[test]
    fn parse_rejects_length_not_multiple_of_13() {
        let bytes = [
            TAG, 14, 0x46, 0x52, 0x41, 0x02, 0x01, 0x00, 0xAB, 0xCD, 0xEF, 0x12, 0x34, 0x02, 0x00,
            0xFF,
        ];
        let err = LocalTimeOffsetDescriptor::parse(&bytes).unwrap_err();
        assert!(matches!(err, Error::InvalidDescriptor { tag: TAG, .. }));
    }

    #[test]
    fn parse_ignores_reserved_bit_not_set() {
        let bytes = [
            TAG, 13, 0x46, 0x52, 0x41, 0x00, 0x01, 0x00, 0xAB, 0xCD, 0xEF, 0x12, 0x34, 0x02, 0x00,
        ];
        let d = LocalTimeOffsetDescriptor::parse(&bytes).unwrap();
        assert_eq!(d.entries.len(), 1);
        assert!(!d.entries[0].local_time_offset_negative);
    }

    #[test]
    fn serialize_round_trip() {
        let e = LocalTimeOffsetEntry {
            country_code: LangCode([0x46, 0x52, 0x41]),
            country_region_id: 0,
            local_time_offset_negative: false,
            local_time_offset_bcd: 0x0100,
            time_of_change_raw: [0xAB, 0xCD, 0xEF, 0x12, 0x34],
            next_time_offset_bcd: 0x0200,
        };
        let d = LocalTimeOffsetDescriptor { entries: vec![e] };
        let mut buf = vec![0u8; d.serialized_len()];
        d.serialize_into(&mut buf).unwrap();
        let re = LocalTimeOffsetDescriptor::parse(&buf).unwrap();
        assert_eq!(d, re);
    }

    #[test]
    fn set_time_of_change_raw_updates_field() {
        let mut e = LocalTimeOffsetEntry {
            country_code: LangCode([0x46, 0x52, 0x41]),
            country_region_id: 0,
            local_time_offset_negative: false,
            local_time_offset_bcd: 0x0100,
            time_of_change_raw: [0; 5],
            next_time_offset_bcd: 0x0200,
        };
        e.set_time_of_change_raw([0xAB, 0xCD, 0xEF, 0x12, 0x34]);
        assert_eq!(e.time_of_change_raw(), [0xAB, 0xCD, 0xEF, 0x12, 0x34]);
    }

    #[test]
    fn empty_descriptor_valid() {
        let bytes = [TAG, 0];
        let d = LocalTimeOffsetDescriptor::parse(&bytes).unwrap();
        assert!(d.entries.is_empty());
    }
}
