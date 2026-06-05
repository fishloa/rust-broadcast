//! DTS Descriptor — ETSI EN 300 468 Annex G, Table G.1 (tag 0x7B).
//!
//! Carried in the PMT ES_info loop to identify DTS Coherent Acoustics audio.
//! Per the SI PDF (etsi_en_300_468_v01.19.01, Annex G §G.2.1, Table G.1,
//! PDF pp. 184-186) the body packs 40 fixed bits then an additional_info run:
//!
//! ```text
//! sample_rate_code(4) + bit_rate_code(6) + nblks(7) + fsize(14)
//!   + surround_mode(6) + lfe_flag(1) + extended_surround_flag(2)
//!   + additional_info_byte(8*N)
//! ```
//!
//! Field codings: sample_rate_code Table G.2 (PDF p. 185), bit_rate_code
//! Table G.3 (PDF p. 185), surround_mode Table G.4 (PDF p. 186),
//! extended_surround_flag Table G.5 (PDF p. 186).

use crate::error::{Error, Result};
use crate::traits::Descriptor;
use dvb_common::{Parse, Serialize};

/// Descriptor tag for DTS_descriptor.
pub const TAG: u8 = 0x7B;
const HEADER_LEN: usize = 2;
/// Five bytes hold the 40 packed fixed bits.
const FIXED_LEN: usize = 5;

const SAMPLE_RATE_CODE_MAX: u8 = 0x0F; // 4 bits
const BIT_RATE_CODE_MAX: u8 = 0x3F; // 6 bits
const NBLKS_MAX: u8 = 0x7F; // 7 bits
const FSIZE_MAX: u16 = 0x3FFF; // 14 bits
const SURROUND_MODE_MAX: u8 = 0x3F; // 6 bits
const EXTENDED_SURROUND_MAX: u8 = 0x03; // 2 bits

/// DTS Descriptor.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct DtsDescriptor<'a> {
    /// 4-bit sample_rate_code (SFREQ, Table G.2).
    pub sample_rate_code: u8,
    /// 6-bit bit_rate_code (Table G.3).
    pub bit_rate_code: u8,
    /// 7-bit nblks (NBLKS; valid range 5..=127).
    pub nblks: u8,
    /// 14-bit fsize (FSIZE; valid range 95..=8192).
    pub fsize: u16,
    /// 6-bit surround_mode (AMODE, Table G.4).
    pub surround_mode: u8,
    /// 1-bit lfe_flag (LFE channel on/off).
    pub lfe_flag: bool,
    /// 2-bit extended_surround_flag (Table G.5).
    pub extended_surround_flag: u8,
    /// Trailing additional_info bytes (TS 101 154 §6.3).
    pub additional_info: &'a [u8],
}

impl<'a> Parse<'a> for DtsDescriptor<'a> {
    type Error = crate::error::Error;
    fn parse(bytes: &'a [u8]) -> Result<Self> {
        if bytes.len() < HEADER_LEN {
            return Err(Error::BufferTooShort {
                need: HEADER_LEN,
                have: bytes.len(),
                what: "DtsDescriptor header",
            });
        }
        if bytes[0] != TAG {
            return Err(Error::InvalidDescriptor {
                tag: bytes[0],
                reason: "unexpected tag for DTS_descriptor",
            });
        }
        let length = bytes[1] as usize;
        let end = HEADER_LEN + length;
        if bytes.len() < end {
            return Err(Error::BufferTooShort {
                need: end,
                have: bytes.len(),
                what: "DtsDescriptor body",
            });
        }
        if length < FIXED_LEN {
            return Err(Error::InvalidDescriptor {
                tag: TAG,
                reason: "DTS_descriptor body shorter than 5 bytes",
            });
        }
        let body = &bytes[HEADER_LEN..end];
        // Pack the 5 fixed bytes into a 40-bit big-endian value.
        let packed: u64 = (u64::from(body[0]) << 32)
            | (u64::from(body[1]) << 24)
            | (u64::from(body[2]) << 16)
            | (u64::from(body[3]) << 8)
            | u64::from(body[4]);
        let sample_rate_code = ((packed >> 36) & 0x0F) as u8;
        let bit_rate_code = ((packed >> 30) & 0x3F) as u8;
        let nblks = ((packed >> 23) & 0x7F) as u8;
        let fsize = ((packed >> 9) & 0x3FFF) as u16;
        let surround_mode = ((packed >> 3) & 0x3F) as u8;
        let lfe_flag = ((packed >> 2) & 0x01) != 0;
        let extended_surround_flag = (packed & 0x03) as u8;
        let additional_info = &body[FIXED_LEN..];
        Ok(Self {
            sample_rate_code,
            bit_rate_code,
            nblks,
            fsize,
            surround_mode,
            lfe_flag,
            extended_surround_flag,
            additional_info,
        })
    }
}

impl Serialize for DtsDescriptor<'_> {
    type Error = crate::error::Error;
    fn serialized_len(&self) -> usize {
        HEADER_LEN + FIXED_LEN + self.additional_info.len()
    }

    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        if self.sample_rate_code > SAMPLE_RATE_CODE_MAX {
            return Err(Error::InvalidDescriptor {
                tag: TAG,
                reason: "sample_rate_code exceeds 4 bits",
            });
        }
        if self.bit_rate_code > BIT_RATE_CODE_MAX {
            return Err(Error::InvalidDescriptor {
                tag: TAG,
                reason: "bit_rate_code exceeds 6 bits",
            });
        }
        if self.nblks > NBLKS_MAX {
            return Err(Error::InvalidDescriptor {
                tag: TAG,
                reason: "nblks exceeds 7 bits",
            });
        }
        if self.fsize > FSIZE_MAX {
            return Err(Error::InvalidDescriptor {
                tag: TAG,
                reason: "fsize exceeds 14 bits",
            });
        }
        if self.surround_mode > SURROUND_MODE_MAX {
            return Err(Error::InvalidDescriptor {
                tag: TAG,
                reason: "surround_mode exceeds 6 bits",
            });
        }
        if self.extended_surround_flag > EXTENDED_SURROUND_MAX {
            return Err(Error::InvalidDescriptor {
                tag: TAG,
                reason: "extended_surround_flag exceeds 2 bits",
            });
        }
        if FIXED_LEN + self.additional_info.len() > u8::MAX as usize {
            return Err(Error::InvalidDescriptor {
                tag: TAG,
                reason: "DTS_descriptor body exceeds 255 bytes",
            });
        }
        let len = self.serialized_len();
        if buf.len() < len {
            return Err(Error::OutputBufferTooSmall {
                need: len,
                have: buf.len(),
            });
        }
        buf[0] = TAG;
        buf[1] = (FIXED_LEN + self.additional_info.len()) as u8;
        let packed: u64 = ((u64::from(self.sample_rate_code) & 0x0F) << 36)
            | ((u64::from(self.bit_rate_code) & 0x3F) << 30)
            | ((u64::from(self.nblks) & 0x7F) << 23)
            | ((u64::from(self.fsize) & 0x3FFF) << 9)
            | ((u64::from(self.surround_mode) & 0x3F) << 3)
            | (u64::from(self.lfe_flag) << 2)
            | (u64::from(self.extended_surround_flag) & 0x03);
        buf[2] = (packed >> 32) as u8;
        buf[3] = (packed >> 24) as u8;
        buf[4] = (packed >> 16) as u8;
        buf[5] = (packed >> 8) as u8;
        buf[6] = packed as u8;
        buf[HEADER_LEN + FIXED_LEN..len].copy_from_slice(self.additional_info);
        Ok(len)
    }
}

impl<'a> Descriptor<'a> for DtsDescriptor<'a> {
    const TAG: u8 = TAG;
    fn descriptor_length(&self) -> u8 {
        (FIXED_LEN + self.additional_info.len()) as u8
    }
}

impl<'a> crate::traits::DescriptorDef<'a> for DtsDescriptor<'a> {
    const TAG: u8 = TAG;
    const NAME: &'static str = "DTS";
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_no_additional_info() {
        // sr=0b1101 (48kHz), brc=0b001010 (384k), nblks=8, fsize=1024,
        // surround=2 (stereo), lfe=1, ext=1.
        let d_in = DtsDescriptor {
            sample_rate_code: 0b1101,
            bit_rate_code: 0b001010,
            nblks: 8,
            fsize: 1024,
            surround_mode: 0b000010,
            lfe_flag: true,
            extended_surround_flag: 0b01,
            additional_info: &[],
        };
        let mut buf = vec![0u8; d_in.serialized_len()];
        d_in.serialize_into(&mut buf).unwrap();
        let d = DtsDescriptor::parse(&buf).unwrap();
        assert_eq!(d, d_in);
        assert_eq!(buf[1], 5);
    }

    #[test]
    fn parse_with_additional_info() {
        let d_in = DtsDescriptor {
            sample_rate_code: 0b1000,
            bit_rate_code: 0b011010,
            nblks: 127,
            fsize: 0x3FFF,
            surround_mode: 0b001001,
            lfe_flag: false,
            extended_surround_flag: 0b10,
            additional_info: &[0xAA, 0xBB, 0xCC],
        };
        let mut buf = vec![0u8; d_in.serialized_len()];
        d_in.serialize_into(&mut buf).unwrap();
        let d = DtsDescriptor::parse(&buf).unwrap();
        assert_eq!(d.additional_info, &[0xAA, 0xBB, 0xCC]);
        assert_eq!(d, d_in);
    }

    #[test]
    fn parse_rejects_wrong_tag() {
        let bytes = [0x7C, 5, 0, 0, 0, 0, 0];
        assert!(matches!(
            DtsDescriptor::parse(&bytes).unwrap_err(),
            Error::InvalidDescriptor { tag: 0x7C, .. }
        ));
    }

    #[test]
    fn parse_rejects_body_too_short() {
        let bytes = [TAG, 4, 0, 0, 0, 0];
        assert!(matches!(
            DtsDescriptor::parse(&bytes).unwrap_err(),
            Error::InvalidDescriptor { .. }
        ));
    }

    #[test]
    fn parse_rejects_length_overrunning_buffer() {
        let bytes = [TAG, 5, 0, 0, 0];
        assert!(matches!(
            DtsDescriptor::parse(&bytes).unwrap_err(),
            Error::BufferTooShort { .. }
        ));
    }

    #[test]
    fn serialize_round_trip_max_fields() {
        let d = DtsDescriptor {
            sample_rate_code: 0x0F,
            bit_rate_code: 0x3F,
            nblks: 0x7F,
            fsize: 0x3FFF,
            surround_mode: 0x3F,
            lfe_flag: true,
            extended_surround_flag: 0x03,
            additional_info: &[0x01],
        };
        let mut buf = vec![0u8; d.serialized_len()];
        d.serialize_into(&mut buf).unwrap();
        assert_eq!(DtsDescriptor::parse(&buf).unwrap(), d);
    }

    #[test]
    fn serialize_rejects_fsize_over_range() {
        let d = DtsDescriptor {
            sample_rate_code: 0,
            bit_rate_code: 0,
            nblks: 0,
            fsize: 0x4000,
            surround_mode: 0,
            lfe_flag: false,
            extended_surround_flag: 0,
            additional_info: &[],
        };
        let mut buf = vec![0u8; d.serialized_len()];
        assert!(matches!(
            d.serialize_into(&mut buf).unwrap_err(),
            Error::InvalidDescriptor { .. }
        ));
    }

    #[test]
    fn serialize_rejects_bit_rate_code_over_range() {
        let d = DtsDescriptor {
            sample_rate_code: 0,
            bit_rate_code: 0x40,
            nblks: 0,
            fsize: 0,
            surround_mode: 0,
            lfe_flag: false,
            extended_surround_flag: 0,
            additional_info: &[],
        };
        let mut buf = vec![0u8; d.serialized_len()];
        assert!(matches!(
            d.serialize_into(&mut buf).unwrap_err(),
            Error::InvalidDescriptor { .. }
        ));
    }

    #[cfg(feature = "serde")]
    #[test]
    fn serde_serializes_to_stable_json() {
        // Borrowed `&[u8]` cannot deserialize from a JSON number array, so we
        // assert the Serialize impl is wired and emits stable JSON.
        let d = DtsDescriptor {
            sample_rate_code: 0b1101,
            bit_rate_code: 0b001010,
            nblks: 16,
            fsize: 2048,
            surround_mode: 0b001000,
            lfe_flag: true,
            extended_surround_flag: 0b01,
            additional_info: &[0x99],
        };
        let j = serde_json::to_string(&d).unwrap();
        // Valid, re-parseable JSON (key order is map-defined, so we do not
        // assert byte-for-byte string stability).
        let _v: serde_json::Value = serde_json::from_str(&j).unwrap();
        assert!(j.contains("sample_rate_code"));
    }
}
