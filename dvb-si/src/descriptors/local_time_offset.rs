//! Local Time Offset Descriptor — ETSI EN 300 468 §6.2.20 (tag 0x58).
//!
//! Carried inside the TOT (Time Offset Table) on PID 0x0014. Signals per-
//! country offsets from UTC plus any upcoming DST transition.

use super::descriptor_body;
use crate::error::{Error, Result};
use crate::text::LangCode;
use alloc::vec::Vec;
use broadcast_common::len::need_out;
use broadcast_common::reserved::ReservedOnes;
use broadcast_common::{Parse, Serialize};

/// Descriptor tag for local_time_offset_descriptor.
pub const TAG: u8 = 0x58;
const HEADER_LEN: usize = 2;
const ENTRY_LEN: usize = 13;
const POLARITY_MASK: u8 = 0x01;
const REGION_ID_MASK: u8 = 0xFC;
/// `country_region_id` is 6 bits.
const REGION_ID_BITS: u32 = 6;
const RESERVED_BIT_MASK: u8 = 0x02;
const RESERVED_BIT_SHIFT: u8 = 1;
/// Width of the entry's `reserved_future_use` (1 bit).
const RESERVED_BITS: u32 = 1;
/// Width of the `time_of_change` field (MJD + BCD HHMMSS), bytes.
const TIME_OF_CHANGE_LEN: usize = 5;
/// Width of the MJD part of `time_of_change`, bytes.
const TIME_OF_CHANGE_MJD_LEN: usize = core::mem::size_of::<u16>();
/// Offset of the hour byte within `time_of_change`.
const TIME_OF_CHANGE_HOURS_OFFSET: usize = TIME_OF_CHANGE_MJD_LEN;
/// Offset of the minute byte within `time_of_change`.
const TIME_OF_CHANGE_MINUTES_OFFSET: usize = TIME_OF_CHANGE_HOURS_OFFSET + 1;
/// Offset of the second byte within `time_of_change`.
const TIME_OF_CHANGE_SECONDS_OFFSET: usize = TIME_OF_CHANGE_MINUTES_OFFSET + 1;
/// Width of `ISO_639_language_code` in an entry.
const COUNTRY_CODE_LEN: usize = 3;
/// Offset of the flags byte in an entry.
const FLAGS_OFFSET: usize = COUNTRY_CODE_LEN;
/// Offset of `local_time_offset` in an entry.
const LOCAL_TIME_OFFSET_BCD_OFFSET: usize = FLAGS_OFFSET + 1;
/// Offset of `time_of_change` in an entry.
const TIME_OF_CHANGE_OFFSET: usize = LOCAL_TIME_OFFSET_BCD_OFFSET + core::mem::size_of::<u16>();
/// Offset of `next_time_offset` in an entry.
const NEXT_TIME_OFFSET_BCD_OFFSET: usize = TIME_OF_CHANGE_OFFSET + TIME_OF_CHANGE_LEN;
/// Shift of `country_region_id` in the flags byte.
const REGION_ID_SHIFT: u8 = 2;
/// Shift between the two bytes of a BCD `HHMM` offset.
#[cfg(feature = "jiff")]
const BCD_HI_BYTE_SHIFT: u8 = 8;
/// Mask of the low byte of a BCD `HHMM` offset.
#[cfg(feature = "jiff")]
const BCD_LO_BYTE_MASK: u16 = 0xFF;
/// Minutes in one hour.
#[cfg(feature = "jiff")]
const MINUTES_PER_HOUR: u32 = 60;
/// Largest `HH` value a BCD offset can carry.
#[cfg(feature = "jiff")]
const MAX_OFFSET_HOURS: u64 = 99;
/// Spec default of the entry's 1-bit `reserved_future_use` (`reserved` before
/// V1.17.1): `1` (EN 300 468 V1.3.1-V1.20.1 Table 29; audit
/// `private/specs-verified/dvb/dvb-si/reserved-bits-edition-audit.md` §6.1, class B: renamed only).
pub const RESERVED_FUTURE_USE_DEFAULT: ReservedOnes<RESERVED_BITS> =
    ReservedOnes::<RESERVED_BITS>::FILL;

/// One per-country offset entry.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub struct LocalTimeOffsetEntry {
    /// `reserved_future_use`(1) between `country_region_id` and the polarity
    /// bit (1 bit, right-aligned). Captured on parse and re-emitted verbatim;
    /// [`LocalTimeOffsetEntry::new`] defaults to [`RESERVED_FUTURE_USE_DEFAULT`].
    pub reserved_future_use: ReservedOnes<RESERVED_BITS>,
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
    /// [`Self::time_of_change`] (feature `jiff`) for a `jiff::Timestamp`.
    time_of_change_raw: [u8; TIME_OF_CHANGE_LEN],
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
        time_of_change_raw: [u8; TIME_OF_CHANGE_LEN],
        next_time_offset_bcd: u16,
    ) -> Self {
        Self {
            reserved_future_use: RESERVED_FUTURE_USE_DEFAULT,
            country_code,
            country_region_id,
            local_time_offset_negative,
            local_time_offset_bcd,
            time_of_change_raw,
            next_time_offset_bcd,
        }
    }

    /// Decode `time_of_change_raw` into its MJD + BCD HHMMSS components
    /// without requiring the `jiff` feature.
    ///
    /// Returns `(mjd, hours, minutes, seconds)`. Each BCD field is `None` if
    /// its nibbles are non-decimal.
    #[must_use]
    pub fn time_of_change_parts(&self) -> (u16, Option<u8>, Option<u8>, Option<u8>) {
        let mjd = u16::from_be_bytes([
            self.time_of_change_raw[0],
            self.time_of_change_raw[TIME_OF_CHANGE_HOURS_OFFSET - 1],
        ]);
        let h = broadcast_common::bcd::from_bcd_byte(
            self.time_of_change_raw[TIME_OF_CHANGE_HOURS_OFFSET],
        );
        let m = broadcast_common::bcd::from_bcd_byte(
            self.time_of_change_raw[TIME_OF_CHANGE_MINUTES_OFFSET],
        );
        let s = broadcast_common::bcd::from_bcd_byte(
            self.time_of_change_raw[TIME_OF_CHANGE_SECONDS_OFFSET],
        );
        (mjd, h, m, s)
    }

    /// The raw 5-byte MJD+BCD time-of-change field (for serialization).
    #[must_use]
    pub fn time_of_change_raw(&self) -> [u8; TIME_OF_CHANGE_LEN] {
        self.time_of_change_raw
    }

    /// Set the `time_of_change_raw` field directly.
    pub fn set_time_of_change_raw(&mut self, raw: [u8; TIME_OF_CHANGE_LEN]) {
        self.time_of_change_raw = raw;
    }
}

/// Decode a BCD `HHMM` offset to a signed [`jiff::SignedDuration`] (negative when
/// `negative`). `None` if a BCD nibble is out of range.
#[cfg(feature = "jiff")]
fn decode_hhmm(bcd: u16, negative: bool) -> Option<jiff::SignedDuration> {
    let h = broadcast_common::bcd::from_bcd_byte((bcd >> BCD_HI_BYTE_SHIFT) as u8)?;
    let m = broadcast_common::bcd::from_bcd_byte((bcd & BCD_LO_BYTE_MASK) as u8)?;
    let mins = i64::from(h) * i64::from(MINUTES_PER_HOUR) + i64::from(m);
    Some(jiff::SignedDuration::from_mins(if negative {
        -mins
    } else {
        mins
    }))
}

/// Encode a signed offset to `(negative, BCD HHMM)`. `None` if the magnitude is
/// 100 hours or longer.
#[cfg(feature = "jiff")]
fn encode_hhmm(offset: jiff::SignedDuration) -> Option<(bool, u16)> {
    let negative = offset.is_negative();
    let total_min = offset.as_mins().unsigned_abs();
    let h = total_min / u64::from(MINUTES_PER_HOUR);
    let m = total_min % u64::from(MINUTES_PER_HOUR);
    if h > MAX_OFFSET_HOURS {
        return None;
    }
    let hb = broadcast_common::bcd::to_bcd_byte(h as u8)?;
    let mb = broadcast_common::bcd::to_bcd_byte(m as u8)?;
    Some((
        negative,
        (u16::from(hb) << BCD_HI_BYTE_SHIFT) | u16::from(mb),
    ))
}

#[cfg(feature = "jiff")]
impl LocalTimeOffsetEntry {
    /// Decode `local_time_offset` (BCD `HHMM`, signed by
    /// `local_time_offset_negative`) to a [`jiff::SignedDuration`]. `None` if the
    /// BCD nibbles are out of range.
    #[must_use]
    pub fn local_time_offset(&self) -> Option<jiff::SignedDuration> {
        decode_hhmm(self.local_time_offset_bcd, self.local_time_offset_negative)
    }

    /// Decode `next_time_offset` (BCD `HHMM`) to a [`jiff::SignedDuration`]. It
    /// shares the single `local_time_offset_negative` polarity bit (EN 300 468
    /// §6.2.20). `None` if the BCD nibbles are out of range.
    #[must_use]
    pub fn next_time_offset(&self) -> Option<jiff::SignedDuration> {
        decode_hhmm(self.next_time_offset_bcd, self.local_time_offset_negative)
    }

    /// Decode `time_of_change_raw` (16-bit MJD + 24-bit BCD UTC) to a UTC
    /// datetime. `None` if the date/time fields are out of range.
    #[must_use]
    pub fn time_of_change(&self) -> Option<jiff::Timestamp> {
        broadcast_common::time::decode_mjd_bcd_utc(self.time_of_change_raw)
    }

    /// Set the `time_of_change`, encoding it into the 40-bit raw field.
    ///
    /// # Errors
    /// [`ValueOutOfRange`](crate::Error::ValueOutOfRange) if the date is
    /// outside the representable 16-bit MJD range.
    pub fn set_time_of_change(&mut self, dt: jiff::Timestamp) -> Result<()> {
        self.time_of_change_raw =
            broadcast_common::time::encode_mjd_bcd_utc(dt).ok_or(Error::ValueOutOfRange {
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
    pub fn set_offsets(
        &mut self,
        local: jiff::SignedDuration,
        next: jiff::SignedDuration,
    ) -> Result<()> {
        let oor = |reason| Error::ValueOutOfRange {
            field: "LocalTimeOffsetEntry offsets",
            reason,
        };
        let local_neg = local.is_negative();
        let next_neg = next.is_negative();
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
        for chunk in body.chunks_exact(ENTRY_LEN) {
            let mut country_code_bytes = [0u8; COUNTRY_CODE_LEN];
            country_code_bytes.copy_from_slice(&chunk[..COUNTRY_CODE_LEN]);
            let country_code = LangCode(country_code_bytes);
            let flags = chunk[FLAGS_OFFSET];
            let country_region_id = (flags & REGION_ID_MASK) >> REGION_ID_SHIFT;
            let local_time_offset_negative = flags & POLARITY_MASK != 0;
            let local_time_offset_bcd =
                crate::wire::be_u16(chunk, LOCAL_TIME_OFFSET_BCD_OFFSET, "local_time_offset")?;
            let mut time_of_change_raw = [0u8; TIME_OF_CHANGE_LEN];
            time_of_change_raw
                .copy_from_slice(&chunk[TIME_OF_CHANGE_OFFSET..NEXT_TIME_OFFSET_BCD_OFFSET]);
            let next_time_offset_bcd =
                crate::wire::be_u16(chunk, NEXT_TIME_OFFSET_BCD_OFFSET, "local_time_offset")?;
            entries.push(LocalTimeOffsetEntry {
                reserved_future_use: ReservedOnes::from_bits(u64::from(
                    (flags & RESERVED_BIT_MASK) >> RESERVED_BIT_SHIFT,
                )),
                country_code,
                country_region_id,
                local_time_offset_negative,
                local_time_offset_bcd,
                time_of_change_raw,
                next_time_offset_bcd,
            });
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
        need_out(buf, len)?;
        crate::descriptors::write_descriptor_header(buf, TAG, len - HEADER_LEN)?;
        let mut offset = HEADER_LEN;
        for entry in &self.entries {
            buf[offset..offset + COUNTRY_CODE_LEN].copy_from_slice(&entry.country_code.0);
            let region_id = broadcast_common::len::fit_bits(
                u64::from(entry.country_region_id),
                REGION_ID_BITS,
                "country_region_id",
            )? as u8;
            let flags = (broadcast_common::len::fit_mask(
                region_id,
                REGION_ID_MASK >> REGION_ID_SHIFT,
                "region_id",
            )? << REGION_ID_SHIFT)
                | ((entry.reserved_future_use.get() as u8) << RESERVED_BIT_SHIFT)
                | if entry.local_time_offset_negative {
                    POLARITY_MASK
                } else {
                    0
                };
            buf[offset + FLAGS_OFFSET] = flags;
            buf[offset + LOCAL_TIME_OFFSET_BCD_OFFSET..offset + TIME_OF_CHANGE_OFFSET]
                .copy_from_slice(&entry.local_time_offset_bcd.to_be_bytes());
            buf[offset + TIME_OF_CHANGE_OFFSET..offset + NEXT_TIME_OFFSET_BCD_OFFSET]
                .copy_from_slice(&entry.time_of_change_raw);
            buf[offset + NEXT_TIME_OFFSET_BCD_OFFSET..offset + ENTRY_LEN]
                .copy_from_slice(&entry.next_time_offset_bcd.to_be_bytes());
            offset += ENTRY_LEN;
        }
        Ok(len)
    }
}
impl crate::traits::DescriptorDef<'_> for LocalTimeOffsetDescriptor {
    const TAG: u8 = TAG;
    const NAME: &'static str = "LOCAL_TIME_OFFSET";
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::vec;

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
            reserved_future_use: RESERVED_FUTURE_USE_DEFAULT,
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

    /// Both reserved_future_use values re-serialize verbatim (the spec default
    /// is 1; a sender writing 0 is preserved). Hand-built entries carry the
    /// default.
    #[test]
    fn reserved_preserved() {
        for r in 0u8..=1 {
            let flags = (r << 1) | 0x01;
            let bytes = [
                TAG, 13, 0x46, 0x52, 0x41, flags, 0x01, 0x00, 0xAB, 0xCD, 0xEF, 0x12, 0x34, 0x02,
                0x00,
            ];
            let d = LocalTimeOffsetDescriptor::parse(&bytes).unwrap();
            assert_eq!(d.entries[0].reserved_future_use.get(), u64::from(r));
            let mut buf = vec![0u8; d.serialized_len()];
            d.serialize_into(&mut buf).unwrap();
            assert_eq!(buf, bytes);
        }
    }

    #[test]
    fn default_is_spec_value() {
        let e = LocalTimeOffsetEntry::new(LangCode(*b"FRA"), 0, false, 0x0100, [0; 5], 0x0200);
        let d = LocalTimeOffsetDescriptor { entries: vec![e] };
        let mut buf = vec![0u8; d.serialized_len()];
        d.serialize_into(&mut buf).unwrap();
        assert_eq!(buf[5], 0x02);
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
            reserved_future_use: RESERVED_FUTURE_USE_DEFAULT,
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
            reserved_future_use: RESERVED_FUTURE_USE_DEFAULT,
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

    #[test]
    fn serialize_rejects_over_6bit_region_id() {
        // Pre-fix `(id << 2) & 0xFC` silently dropped bit 6/7 (r03-W3).
        let d = LocalTimeOffsetDescriptor {
            entries: alloc::vec![LocalTimeOffsetEntry {
                reserved_future_use: RESERVED_FUTURE_USE_DEFAULT,
                country_code: LangCode([0x46, 0x52, 0x41]),
                country_region_id: 0x40,
                local_time_offset_negative: false,
                local_time_offset_bcd: 0x0100,
                time_of_change_raw: [0xAB, 0xCD, 0xEF, 0x12, 0x34],
                next_time_offset_bcd: 0x0200,
            }],
        };
        let mut buf = vec![0u8; d.serialized_len()];
        assert!(matches!(
            d.serialize_into(&mut buf).unwrap_err(),
            Error::FieldOverflow(_)
        ));
    }

    #[cfg(feature = "jiff")]
    #[test]
    fn set_offsets_keeps_negative_polarity_beside_zero() {
        let mut e = LocalTimeOffsetEntry::new(LangCode(*b"FRA"), 0, false, 0, [0; 5], 0);
        e.set_offsets(
            jiff::SignedDuration::from_mins(-60),
            jiff::SignedDuration::from_mins(0),
        )
        .unwrap();
        assert!(e.local_time_offset_negative);
        assert_eq!(e.local_time_offset_bcd, 0x0100);
        assert_eq!(
            e.local_time_offset(),
            Some(jiff::SignedDuration::from_mins(-60))
        );

        let mut e = LocalTimeOffsetEntry::new(LangCode(*b"FRA"), 0, false, 0, [0; 5], 0);
        e.set_offsets(
            jiff::SignedDuration::from_mins(0),
            jiff::SignedDuration::from_mins(-60),
        )
        .unwrap();
        assert!(e.local_time_offset_negative);
        assert_eq!(e.next_time_offset_bcd, 0x0100);
    }

    #[cfg(feature = "jiff")]
    #[test]
    fn set_offsets_accepts_99_59_and_rejects_100_hours() {
        let max = jiff::SignedDuration::from_mins(99 * 60 + 59);
        let mut e = LocalTimeOffsetEntry::new(LangCode(*b"FRA"), 0, false, 0, [0; 5], 0);
        e.set_offsets(max, max).unwrap();
        assert_eq!(e.local_time_offset_bcd, 0x9959);
        assert_eq!(e.next_time_offset_bcd, 0x9959);
        assert_eq!(e.local_time_offset(), Some(max));
        assert!(!e.local_time_offset_negative);

        let too_large = jiff::SignedDuration::from_mins(100 * 60);
        assert!(matches!(
            e.set_offsets(too_large, too_large),
            Err(Error::ValueOutOfRange { .. })
        ));
    }
}
