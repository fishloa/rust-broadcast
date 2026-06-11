//! Time Offset Table — ETSI EN 300 468 §5.2.6.
//!
//! Carried on PID 0x0014 with table_id 0x73. Structure:
//!   5-byte UTC time + descriptor loop + 4-byte CRC32.
//!
//! The TOT is the spec's framing exception: `section_syntax_indicator` SHALL
//! be `0b0` (§5.2.6: "This 1-bit field shall be set to 0b0") yet the section
//! still ends with a CRC_32. Do not route TOT bytes through the generic
//! [`crate::section::Section`] short-form path — it would fold the CRC into
//! the payload. Parse with [`TotSection::parse`] directly.

use crate::descriptors::DescriptorLoop;
use crate::error::{Error, Result};
use dvb_common::{Parse, Serialize};

/// table_id for Time Offset Table.
pub const TABLE_ID: u8 = 0x73;
/// Well-known PID on which TOT is carried (same as TDT).
pub const PID: u16 = 0x0014;

const HEADER_LEN: usize = 3;
const UTC_TIME_LEN: usize = 5;
const DESC_LOOP_LEN_FIELD: usize = 2;
const CRC_LEN: usize = 4;
const MIN_SECTION_LEN: usize = HEADER_LEN + UTC_TIME_LEN + DESC_LOOP_LEN_FIELD + CRC_LEN;

/// Time Offset Table.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[cfg_attr(feature = "yoke", derive(yoke::Yokeable))]
pub struct TotSection<'a> {
    /// Raw 5-byte UTC time (16-bit MJD + 24-bit BCD HHMMSS).
    /// Private — use [`utc_time_decoded`](Self::utc_time_decoded) for
    /// decoded access.
    pub(crate) utc_time_raw: [u8; 5],
    /// Raw descriptor bytes (typically local_time_offset_descriptor tag 0x58).
    /// Descriptor loop. Serializes as the typed descriptor sequence;
    /// `.raw()` yields the wire bytes.
    pub descriptors: DescriptorLoop<'a>,
}

impl<'a> TotSection<'a> {
    /// Decode the UTC time to a plain date-time struct (no `chrono` feature
    /// required).
    ///
    /// Returns `None` if the date/time fields are out of range. MJD→calendar
    /// conversion per ETSI EN 300 468 Annex C.
    #[must_use]
    pub fn utc_time_decoded(&self) -> Option<dvb_common::time::MjdBcdDateTime> {
        dvb_common::time::decode_mjd_bcd(self.utc_time_raw)
    }

    /// Set the UTC time, encoding it from a [`dvb_common::time::MjdBcdDateTime`].
    ///
    /// # Errors
    /// [`ValueOutOfRange`](crate::Error::ValueOutOfRange) if the date is
    /// outside the representable 16-bit MJD range.
    pub fn set_utc_time_decoded(&mut self, dt: dvb_common::time::MjdBcdDateTime) -> Result<()> {
        self.utc_time_raw = dvb_common::time::encode_mjd_bcd(dt).ok_or(Error::ValueOutOfRange {
            field: "TotSection::utc_time",
            reason: "date not representable in 16-bit MJD",
        })?;
        Ok(())
    }

    /// Raw 5-byte UTC time field (for round-trip / serialization).
    #[must_use]
    pub fn utc_time_raw(&self) -> [u8; 5] {
        self.utc_time_raw
    }

    /// Construct a `TotSection` from raw wire fields.
    #[must_use]
    pub fn new(utc_time_raw: [u8; 5], descriptors: DescriptorLoop<'a>) -> Self {
        Self {
            utc_time_raw,
            descriptors,
        }
    }
}

#[cfg(feature = "chrono")]
impl TotSection<'_> {
    /// Decode `utc_time_raw` (16-bit MJD + 24-bit BCD UTC) to a UTC datetime.
    ///
    /// Returns `None` if the date/time fields are out of range. MJD→calendar
    /// conversion per ETSI EN 300 468 Annex C.
    #[must_use]
    pub fn utc_time(&self) -> Option<chrono::DateTime<chrono::Utc>> {
        dvb_common::time::decode_mjd_bcd_utc(self.utc_time_raw)
    }

    /// Set the UTC time, encoding it into the 40-bit `utc_time` field.
    ///
    /// # Errors
    /// [`ValueOutOfRange`](crate::Error::ValueOutOfRange) if the date is
    /// outside the representable 16-bit MJD range.
    pub fn set_utc_time(&mut self, utc_time: chrono::DateTime<chrono::Utc>) -> Result<()> {
        self.utc_time_raw =
            dvb_common::time::encode_mjd_bcd_utc(utc_time).ok_or(Error::ValueOutOfRange {
                field: "TotSection::utc_time",
                reason: "date not representable in 16-bit MJD",
            })?;
        Ok(())
    }
}

impl<'a> Parse<'a> for TotSection<'a> {
    type Error = crate::error::Error;
    fn parse(bytes: &'a [u8]) -> Result<Self> {
        let min_len = HEADER_LEN + UTC_TIME_LEN + DESC_LOOP_LEN_FIELD + CRC_LEN;
        if bytes.len() < min_len {
            return Err(Error::BufferTooShort {
                need: min_len,
                have: bytes.len(),
                what: "TotSection",
            });
        }
        if bytes[0] != TABLE_ID {
            return Err(Error::UnexpectedTableId {
                table_id: bytes[0],
                what: "TotSection",
                expected: &[TABLE_ID],
            });
        }
        let section_length = ((bytes[1] & 0x0F) as u16) << 8 | bytes[2] as u16;
        let total = super::check_section_length(
            bytes.len(),
            HEADER_LEN,
            section_length as usize,
            MIN_SECTION_LEN,
        )?;
        let utc_time_raw = [bytes[3], bytes[4], bytes[5], bytes[6], bytes[7]];
        let dl_pos = HEADER_LEN + UTC_TIME_LEN;
        let dl = (((bytes[dl_pos] & 0x0F) as usize) << 8) | bytes[dl_pos + 1] as usize;
        let d_start = dl_pos + DESC_LOOP_LEN_FIELD;
        let d_end = d_start + dl;
        if d_end > total - CRC_LEN {
            return Err(Error::SectionLengthOverflow {
                declared: dl,
                available: (total - CRC_LEN).saturating_sub(d_start),
            });
        }
        Ok(TotSection {
            utc_time_raw,
            descriptors: DescriptorLoop::new(&bytes[d_start..d_end]),
        })
    }
}

impl Serialize for TotSection<'_> {
    type Error = crate::error::Error;
    fn serialized_len(&self) -> usize {
        HEADER_LEN + UTC_TIME_LEN + DESC_LOOP_LEN_FIELD + self.descriptors.len() + CRC_LEN
    }
    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        let len = self.serialized_len();
        if buf.len() < len {
            return Err(Error::OutputBufferTooSmall {
                need: len,
                have: buf.len(),
            });
        }
        let section_length = (len - HEADER_LEN) as u16;
        if section_length > 0x0FFF {
            return Err(Error::SectionLengthOverflow {
                declared: section_length as usize,
                available: 0x0FFF,
            });
        }
        buf[0] = TABLE_ID;
        // §5.2.6: section_syntax_indicator SHALL be 0 for the TOT (despite the
        // trailing CRC_32). 0x70 = SSI(0) | reserved_future_use(1) | reserved(11).
        buf[1] = super::SECTION_B1_FLAGS_SHORT | ((section_length >> 8) as u8 & 0x0F);
        buf[2] = (section_length & 0xFF) as u8;
        buf[3..8].copy_from_slice(&self.utc_time_raw);
        let dl = self.descriptors.len() as u16;
        buf[8] = 0xF0 | ((dl >> 8) as u8 & 0x0F);
        buf[9] = (dl & 0xFF) as u8;
        let d_end = 10 + self.descriptors.len();
        buf[10..d_end].copy_from_slice(self.descriptors.raw());
        let crc_pos = len - CRC_LEN;
        let crc = dvb_common::crc32_mpeg2::compute(&buf[..crc_pos]);
        buf[crc_pos..len].copy_from_slice(&crc.to_be_bytes());
        Ok(len)
    }
}
impl<'a> crate::traits::TableDef<'a> for TotSection<'a> {
    const TABLE_ID_RANGES: &'static [(u8, u8)] = &[(TABLE_ID, TABLE_ID)];
    const NAME: &'static str = "TIME_OFFSET";
}

#[cfg(test)]
mod tests {
    use super::*;

    fn build_tot(desc: &[u8]) -> Vec<u8> {
        let section_length = (UTC_TIME_LEN + DESC_LOOP_LEN_FIELD + desc.len() + CRC_LEN) as u16;
        let mut v = Vec::new();
        v.push(TABLE_ID);
        // SSI=0 per §5.2.6 (the TOT exception: SSI=0 but CRC present).
        v.push(0x70 | ((section_length >> 8) as u8 & 0x0F));
        v.push((section_length & 0xFF) as u8);
        v.extend_from_slice(&[0xE4, 0x09, 0x12, 0x34, 0x56]);
        let dl = desc.len() as u16;
        v.push(0xF0 | ((dl >> 8) as u8 & 0x0F));
        v.push((dl & 0xFF) as u8);
        v.extend_from_slice(desc);
        let crc = dvb_common::crc32_mpeg2::compute(&v);
        v.extend_from_slice(&crc.to_be_bytes());
        v
    }

    #[test]
    fn parse_with_no_descriptors() {
        let bytes = build_tot(&[]);
        let tot = TotSection::parse(&bytes).unwrap();
        assert_eq!(tot.utc_time_raw(), [0xE4, 0x09, 0x12, 0x34, 0x56]);
        assert_eq!(tot.descriptors.raw(), &[] as &[u8]);
    }

    #[test]
    fn parse_with_local_time_offset_descriptor() {
        let lto = [
            0x58u8, 13, b'G', b'B', b'R', 0x02, 0x00, 0x00, 0xE4, 0x09, 0x12, 0x34, 0x56, 0x01,
            0x00,
        ];
        let bytes = build_tot(&lto);
        let tot = TotSection::parse(&bytes).unwrap();
        assert_eq!(tot.descriptors.raw(), &lto[..]);
    }

    #[test]
    fn parse_rejects_wrong_tag() {
        let mut bytes = build_tot(&[]);
        bytes[0] = 0x70;
        assert!(matches!(
            TotSection::parse(&bytes).unwrap_err(),
            Error::UnexpectedTableId { table_id: 0x70, .. }
        ));
    }

    #[test]
    fn serialize_round_trip() {
        let lto = [0x58u8, 0];
        let bytes = build_tot(&lto);
        let tot = TotSection::parse(&bytes).unwrap();
        let mut buf = vec![0u8; tot.serialized_len()];
        tot.serialize_into(&mut buf).unwrap();
        assert_eq!(buf, bytes, "TOT byte-identity against hand-built input");
        let re = TotSection::parse(&buf).unwrap();
        assert_eq!(tot, re);
    }

    #[test]
    fn parse_rejects_zero_section_length() {
        let mut buf = vec![0u8; 64];
        buf[0] = TABLE_ID;
        buf[1] = 0xF0;
        buf[2] = 0x00;
        for b in &mut buf[3..] {
            *b = 0xFF;
        }
        assert!(matches!(
            TotSection::parse(&buf).unwrap_err(),
            Error::SectionLengthOverflow { .. }
        ));
    }
}
