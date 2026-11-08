//! DTS Descriptor — ETSI EN 300 468 V1.20.1 Annex G §G.2.1, Table G.1 (tag 0x7B).
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

use super::descriptor_body;
use crate::error::{Error, Result};
use broadcast_common::len::need_out;
use broadcast_common::{Parse, Serialize};

/// Descriptor tag for DTS_descriptor.
pub const TAG: u8 = 0x7B;
const HEADER_LEN: usize = 2;
/// Five bytes hold the 40 packed fixed bits.
const FIXED_LEN: usize = 5;

const SAMPLE_RATE_CODE_MAX: u8 = 0x0F; // 4 bits
const BIT_RATE_CODE_MAX: u8 = 0x3F; // 6 bits
/// Table G.3 `bit_rate_code` bits `[4:0]`; the MSB is reserved ("x").
const BIT_RATE_CODE_MASK: u8 = 0x1F;
/// Table G.3 lists 1 411,2 kbit/s, so the table below is in units of 100 bit/s.
const BIT_RATE_UNIT_BPS: u32 = 100;
const NBLKS_MAX: u8 = 0x7F; // 7 bits
const FSIZE_MAX: u16 = 0x3FFF; // 14 bits
const SURROUND_MODE_MAX: u8 = 0x3F; // 6 bits
const EXTENDED_SURROUND_MAX: u8 = 0x03; // 2 bits

/// DTS Descriptor.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[cfg_attr(feature = "yoke", derive(yoke::Yokeable))]
pub struct DtsDescriptor<'a> {
    /// 4-bit sample_rate_code (SFREQ, Table G.2).
    pub sample_rate_code: u8,
    /// 6-bit bit_rate_code (Table G.3). The MSB (bit 5) is reserved ("x")
    /// and should be ignored when decoding; `[4:0]` carry the code.
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

impl DtsDescriptor<'_> {
    /// Decodes `sample_rate_code` to the sample rate in Hz, per
    /// ETSI EN 300 468 Annex G Table G.2.  Returns `None` for code 0
    /// (invalid) and for unknown codes.
    #[must_use]
    pub fn sample_rate_hz(&self) -> Option<u32> {
        match self.sample_rate_code {
            0x01 => Some(8_000),
            0x02 => Some(16_000),
            0x03 => Some(32_000),
            0x04 => Some(64_000),
            0x05 => Some(128_000),
            0x06 => Some(11_025),
            0x07 => Some(22_050),
            0x08 => Some(44_100),
            // EN 300 468 Table G.2 prints "88,02 kHz" for 0b1001 — a digit
            // transposition of 88,2 kHz: the same spec's DTS-HD table (Table
            // G.7, dts-hd-descriptor.md) lists 88,2 kHz, and 0b0110/0b0111/
            // 0b1000/0b1010 are 11,025/22,05/44,1/176,4 kHz, so 0b1001 is the
            // missing 2x44,1 step (r03-W11). Cross-checked against the DTS
            // core spec ETSI TS 102 114 V1.6.1 (#1197 item e): its Table 5-5
            // (SFREQ, the field `sample_rate_code` is "equivalent to") lists
            // 0b1001 only as "Invalid" because >48 kHz is carried by the
            // extension streams, and its Table 8-5 (sFreqIndex 7) gives
            // 88 200 Hz as the DTS 88,2 kHz rate; no DTS table has an
            // 88 020 Hz rate. The code points form doubling chains
            // (8/16/32/64/128, 11,025/22,05/44,1/88,2/176,4, 12/24/48/96/192)
            // in which 0b1001 can only be 88,2 kHz.
            0x09 => Some(88_200),
            0x0A => Some(176_400),
            0x0B => Some(12_000),
            0x0C => Some(24_000),
            0x0D => Some(48_000),
            0x0E => Some(96_000),
            0x0F => Some(192_000),
            _ => None,
        }
    }

    /// Decodes `bit_rate_code` to the bit rate in bit/s, per ETSI EN 300 468
    /// Annex G Table G.3. The MSB of the 6-bit code is reserved ("x") and is
    /// ignored; bits `[4:0]` select the rate. Returns `None` for codes below
    /// `0x05` and for the non-numeric `open` / `variable` / `lossless` codes.
    #[must_use]
    pub fn bit_rate_bps(&self) -> Option<u32> {
        let hundreds_of_bps: u32 = match self.bit_rate_code & BIT_RATE_CODE_MASK {
            0x05 => 1_280,
            0x06 => 1_920,
            0x07 => 2_240,
            0x08 => 2_560,
            0x09 => 3_200,
            0x0A => 3_840,
            0x0B => 4_480,
            0x0C => 5_120,
            0x0D => 5_760,
            0x0E => 6_400,
            0x0F => 7_680,
            0x10 => 9_600,
            0x11 => 10_240,
            0x12 => 11_520,
            0x13 => 12_800,
            0x14 => 13_440,
            0x15 => 14_080,
            0x16 => 14_112,
            0x17 => 14_720,
            0x18 => 15_360,
            0x19 => 19_200,
            0x1A => 20_480,
            0x1B => 30_720,
            0x1C => 38_400,
            _ => return None,
        };
        Some(hundreds_of_bps * BIT_RATE_UNIT_BPS)
    }
}

impl<'a> Parse<'a> for DtsDescriptor<'a> {
    type Error = crate::error::Error;
    fn parse(bytes: &'a [u8]) -> Result<Self> {
        let body = descriptor_body(
            bytes,
            TAG,
            "DtsDescriptor",
            "unexpected tag for DTS_descriptor",
        )?;
        if body.len() < FIXED_LEN {
            return Err(Error::InvalidDescriptor {
                tag: TAG,
                reason: "DTS_descriptor body shorter than 5 bytes",
            });
        }
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
        need_out(buf, len)?;
        crate::descriptors::write_descriptor_header(
            buf,
            TAG,
            FIXED_LEN + self.additional_info.len(),
        )?;
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
impl<'a> crate::traits::DescriptorDef<'a> for DtsDescriptor<'a> {
    const TAG: u8 = TAG;
    const NAME: &'static str = "DTS";
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::vec;

    #[test]
    fn bit_rate_bps_follows_table_g3_and_ignores_the_reserved_msb() {
        let rate = |code: u8| {
            DtsDescriptor {
                sample_rate_code: 0,
                bit_rate_code: code,
                nblks: 5,
                fsize: 95,
                surround_mode: 0,
                lfe_flag: false,
                extended_surround_flag: 0,
                additional_info: &[],
            }
            .bit_rate_bps()
        };
        assert_eq!(rate(0x00), None);
        assert_eq!(rate(0x04), None);
        assert_eq!(rate(0x05), Some(128_000));
        assert_eq!(rate(0x0A), Some(384_000));
        assert_eq!(rate(0x10), Some(960_000));
        assert_eq!(rate(0x11), Some(1_024_000));
        assert_eq!(rate(0x16), Some(1_411_200));
        assert_eq!(rate(0x1C), Some(3_840_000));
        for open_variable_lossless in [0x1D, 0x1E, 0x1F] {
            assert_eq!(rate(open_variable_lossless), None);
        }
        for code in 0x05..=0x1C {
            assert_eq!(rate(code), rate(code | 0x20), "code {code:#04x}");
        }
    }

    /// r03-W11: code 0b1001 is 88,2 kHz (2 x 44,1), not the 88,02 kHz the
    /// spec table prints; the 44,1 kHz family doubles at each step.
    #[test]
    fn sample_rate_0x09_is_88_200_hz_in_the_44k1_family() {
        let rate = |code: u8| {
            DtsDescriptor {
                sample_rate_code: code,
                bit_rate_code: 0,
                nblks: 5,
                fsize: 95,
                surround_mode: 0,
                lfe_flag: false,
                extended_surround_flag: 0,
                additional_info: &[],
            }
            .sample_rate_hz()
        };
        assert_eq!(rate(0x08), Some(44_100));
        assert_eq!(rate(0x09), Some(88_200));
        assert_eq!(rate(0x0A), Some(176_400));
        assert_eq!(rate(0x09), rate(0x08).map(|r| r * 2));
        assert_eq!(rate(0x0A), rate(0x09).map(|r| r * 2));
    }

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
    fn decode_sample_rate() {
        let d = DtsDescriptor {
            sample_rate_code: 0x0D,
            bit_rate_code: 0,
            nblks: 0,
            fsize: 0,
            surround_mode: 0,
            lfe_flag: false,
            extended_surround_flag: 0,
            additional_info: &[],
        };
        assert_eq!(d.sample_rate_hz(), Some(48_000));
    }

    #[test]
    fn decode_sample_rate_invalid() {
        let d = DtsDescriptor {
            sample_rate_code: 0x00,
            bit_rate_code: 0,
            nblks: 0,
            fsize: 0,
            surround_mode: 0,
            lfe_flag: false,
            extended_surround_flag: 0,
            additional_info: &[],
        };
        assert_eq!(d.sample_rate_hz(), None);
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
            Error::BufferTooShort(..)
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
}
