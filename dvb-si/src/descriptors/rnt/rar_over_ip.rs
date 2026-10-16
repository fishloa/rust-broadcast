//! RAR over IP descriptor — ETSI TS 102 323 V1.4.1 §5.3.6 Table 13 (RNT tag
//! 0x41).
//!
//! Encapsulates a RAR for a CRID authority whose data can be found via an IP
//! network. Permitted in the CRID authority descriptor loop of an RNT section
//! (Table 4); tag 0x41 here overrides the SI meaning of that tag
//! (`service_list_descriptor`) inside an RNT. The 1-bit `reserved` "shall be
//! set to '1'" (clause 5.3.6): parse is tolerant, the parsed bit is preserved
//! on serialize, a hand-built descriptor uses `1`.

use crate::descriptors::mjd_time::{MJD_BCD_LEN, MjdBcdTime};
use crate::descriptors::{descriptor_body, write_descriptor_header};
use crate::error::{Error, Result};
use crate::text::DvbText;
use broadcast_common::{Parse, Serialize};

/// Descriptor tag for RAR_over_IP_descriptor (RNT namespace).
pub const TAG: u8 = 0x41;
const HEADER_LEN: usize = 2;
/// first/last valid date, weighting+flags byte, url_length.
const FIXED_LEN: usize = 2 * MJD_BCD_LEN + 2;
const WEIGHTING_SHIFT: u8 = 2;
const WEIGHTING_MASK: u8 = 0x3F;
const COMPLETE_FLAG: u8 = 0x02;
const RESERVED_MASK: u8 = 0x01;
/// Default of the 1-bit `reserved`: `1` (TS 102 323 §5.3.6 "Reserved bits shall
/// be set to '1'").
pub const RESERVED_DEFAULT: u8 = 1;

/// RAR over IP descriptor.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[cfg_attr(feature = "yoke", derive(yoke::Yokeable))]
pub struct RarOverIpDescriptor<'a> {
    /// `first_valid_date` — first date the reference can be used (40-bit
    /// MJD+BCD).
    pub first_valid_date: MjdBcdTime,
    /// `last_valid_date` — first date the reference can no longer be used.
    pub last_valid_date: MjdBcdTime,
    /// `weighting` (6 bits) — order hint; the largest is tried first.
    pub weighting: u8,
    /// `complete_flag` — the referenced CRI data is complete.
    pub complete_flag: bool,
    /// `reserved` (1 bit, right-aligned). Captured on parse and re-emitted
    /// verbatim; [`Self::new`] defaults to [`RESERVED_DEFAULT`].
    pub reserved: u8,
    /// `url_char`s — where CRIDs of this authority can be resolved.
    pub url: DvbText<'a>,
    /// Bytes after the URL inside `descriptor_length`; the spec defines none
    /// (kept for byte-identical re-serialization).
    pub trailing: &'a [u8],
}

impl<'a> RarOverIpDescriptor<'a> {
    /// Build a descriptor with the spec-default reserved bit.
    #[must_use]
    pub fn new(
        first_valid_date: MjdBcdTime,
        last_valid_date: MjdBcdTime,
        weighting: u8,
        complete_flag: bool,
        url: DvbText<'a>,
    ) -> Self {
        Self {
            first_valid_date,
            last_valid_date,
            weighting,
            complete_flag,
            reserved: RESERVED_DEFAULT,
            url,
            trailing: &[],
        }
    }
}

impl<'a> Parse<'a> for RarOverIpDescriptor<'a> {
    type Error = Error;
    fn parse(bytes: &'a [u8]) -> Result<Self> {
        let body = descriptor_body(
            bytes,
            TAG,
            "RarOverIpDescriptor",
            "unexpected tag for RAR_over_IP_descriptor",
        )?;
        if body.len() < FIXED_LEN {
            return Err(Error::InvalidDescriptor {
                tag: TAG,
                reason: "RAR_over_IP_descriptor body shorter than 12 bytes",
            });
        }
        let flags = body[2 * MJD_BCD_LEN];
        let url_len = usize::from(body[2 * MJD_BCD_LEN + 1]);
        let url_end = FIXED_LEN + url_len;
        if url_end > body.len() {
            return Err(Error::InvalidDescriptor {
                tag: TAG,
                reason: "url_length runs past descriptor end",
            });
        }
        Ok(Self {
            first_valid_date: MjdBcdTime::read(body, 0, "first_valid_date")?,
            last_valid_date: MjdBcdTime::read(body, MJD_BCD_LEN, "last_valid_date")?,
            weighting: flags >> WEIGHTING_SHIFT,
            complete_flag: flags & COMPLETE_FLAG != 0,
            reserved: flags & RESERVED_MASK,
            url: DvbText::new(&body[FIXED_LEN..url_end]),
            trailing: &body[url_end..],
        })
    }
}

impl Serialize for RarOverIpDescriptor<'_> {
    type Error = Error;
    fn serialized_len(&self) -> usize {
        HEADER_LEN + FIXED_LEN + self.url.len() + self.trailing.len()
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
        let reserved = broadcast_common::len::fit_mask(self.reserved, RESERVED_MASK, "reserved")?;
        let url_len = broadcast_common::len::fit_u8(self.url.len(), "url_length")?;
        write_descriptor_header(buf, TAG, len - HEADER_LEN)?;
        let b = &mut buf[HEADER_LEN..];
        self.first_valid_date.write(b, 0);
        self.last_valid_date.write(b, MJD_BCD_LEN);
        let mut flags = (weighting << WEIGHTING_SHIFT) | reserved;
        if self.complete_flag {
            flags |= COMPLETE_FLAG;
        }
        b[2 * MJD_BCD_LEN] = flags;
        b[2 * MJD_BCD_LEN + 1] = url_len;
        let url_end = FIXED_LEN + self.url.len();
        b[FIXED_LEN..url_end].copy_from_slice(self.url.raw());
        b[url_end..url_end + self.trailing.len()].copy_from_slice(self.trailing);
        Ok(len)
    }
}

impl<'a> crate::traits::DescriptorDef<'a> for RarOverIpDescriptor<'a> {
    const TAG: u8 = TAG;
    const NAME: &'static str = "RAR_OVER_IP";
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample(flags: u8) -> Vec<u8> {
        let mut v = vec![TAG, 0];
        v.extend_from_slice(&[0xC0, 0x79, 0x12, 0x00, 0x00]);
        v.extend_from_slice(&[0xC0, 0x80, 0x23, 0x59, 0x59]);
        v.push(flags);
        let url = b"http://crid.example/rar";
        v.push(url.len() as u8);
        v.extend_from_slice(url);
        v[1] = (v.len() - 2) as u8;
        v
    }

    #[test]
    fn parse_and_round_trip() {
        // weighting 9, complete, reserved 1.
        let bytes = sample((9 << 2) | 0x02 | 0x01);
        let d = RarOverIpDescriptor::parse(&bytes).unwrap();
        assert_eq!(d.weighting, 9);
        assert!(d.complete_flag);
        assert_eq!(d.reserved, 1);
        assert_eq!(d.url.raw(), b"http://crid.example/rar");
        let mut out = vec![0u8; d.serialized_len()];
        d.serialize_into(&mut out).unwrap();
        assert_eq!(out, bytes);
    }

    #[test]
    fn off_spec_reserved_zero_is_preserved() {
        let bytes = sample(9 << 2);
        let d = RarOverIpDescriptor::parse(&bytes).unwrap();
        assert_eq!(d.reserved, 0);
        let mut out = vec![0u8; d.serialized_len()];
        d.serialize_into(&mut out).unwrap();
        assert_eq!(out, bytes);
        let built =
            RarOverIpDescriptor::new(d.first_valid_date, d.last_valid_date, 1, false, DvbText::new(b"u"));
        assert_eq!(built.reserved, RESERVED_DEFAULT);
    }

    #[test]
    fn trailing_and_errors() {
        let mut bytes = sample(0x01);
        bytes.push(0xEE);
        bytes[1] += 1;
        let d = RarOverIpDescriptor::parse(&bytes).unwrap();
        assert_eq!(d.trailing, &[0xEE]);
        let mut out = vec![0u8; d.serialized_len()];
        d.serialize_into(&mut out).unwrap();
        assert_eq!(out, bytes);
        let mut over = sample(0x01);
        over[2 + 11] = 200;
        assert!(RarOverIpDescriptor::parse(&over).is_err());
        assert!(RarOverIpDescriptor::parse(&[TAG, 2, 0, 0]).is_err());
        assert!(RarOverIpDescriptor::parse(&[0x40, 0]).is_err());
    }
}
