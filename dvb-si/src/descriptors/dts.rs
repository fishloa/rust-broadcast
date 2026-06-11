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

use super::descriptor_body;
use crate::error::{Error, Result};
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
            0x09 => Some(88_020),
            0x0A => Some(176_400),
            0x0B => Some(12_000),
            0x0C => Some(24_000),
            0x0D => Some(48_000),
            0x0E => Some(96_000),
            0x0F => Some(192_000),
            _ => None,
        }
    }

    /// Returns a human-readable sample rate label, or `None` for
    /// invalid/unknown codes.
    #[must_use]
    pub fn sample_rate_name(&self) -> Option<&'static str> {
        match self.sample_rate_code {
            0x00 => Some("invalid"),
            0x01 => Some("8 kHz"),
            0x02 => Some("16 kHz"),
            0x03 => Some("32 kHz"),
            0x04 => Some("64 kHz"),
            0x05 => Some("128 kHz"),
            0x06 => Some("11.025 kHz"),
            0x07 => Some("22.05 kHz"),
            0x08 => Some("44.1 kHz"),
            // Spec typo: 88,02 kHz (comma instead of period) at code 0x09.
            0x09 => Some("88.02 kHz"),
            0x0A => Some("176.4 kHz"),
            0x0B => Some("12 kHz"),
            0x0C => Some("24 kHz"),
            0x0D => Some("48 kHz"),
            0x0E => Some("96 kHz"),
            0x0F => Some("192 kHz"),
            _ => None,
        }
    }

    /// Decodes `bit_rate_code` to the bit rate in kbit/s, per
    /// ETSI EN 300 468 Annex G Table G.3.  The MSB (bit 5) of the
    /// 6-bit code is reserved ("x") and is masked off; bits `[4:0]`
    /// carry the actual code. Returns `None` for unknown codes.
    #[must_use]
    pub fn bit_rate_kbits(&self) -> Option<u32> {
        match self.bit_rate_code & 0x1F {
            0x05 => Some(128),
            0x06 => Some(192),
            0x07 => Some(224),
            0x08 => Some(256),
            0x09 => Some(320),
            0x0A => Some(384),
            0x0B => Some(448),
            0x0C => Some(512),
            0x0D => Some(576),
            0x0E => Some(640),
            0x0F => Some(768),
            0x10 => Some(960),
            0x11 => Some(1_024),
            0x12 => Some(1_152),
            0x13 => Some(1_280),
            0x14 => Some(1_344),
            0x15 => Some(1_408),
            0x16 => Some(1_411),
            0x17 => Some(1_472),
            0x18 => Some(1_536),
            0x19 => Some(1_920),
            0x1A => Some(2_048),
            0x1B => Some(3_072),
            0x1C => Some(3_840),
            0x1D => None, // "open"
            0x1E => None, // "variable"
            0x1F => None, // "lossless"
            _ => None,
        }
    }

    /// Returns a human-readable bit rate label, or `None` for unknown codes.
    #[must_use]
    pub fn bit_rate_name(&self) -> Option<&'static str> {
        match self.bit_rate_code & 0x1F {
            0x05 => Some("128 kbit/s"),
            0x06 => Some("192 kbit/s"),
            0x07 => Some("224 kbit/s"),
            0x08 => Some("256 kbit/s"),
            0x09 => Some("320 kbit/s"),
            0x0A => Some("384 kbit/s"),
            0x0B => Some("448 kbit/s"),
            0x0C => Some("512 kbit/s"),
            0x0D => Some("576 kbit/s"),
            0x0E => Some("640 kbit/s"),
            0x0F => Some("768 kbit/s"),
            0x10 => Some("960 kbit/s"),
            0x11 => Some("1024 kbit/s"),
            0x12 => Some("1152 kbit/s"),
            0x13 => Some("1280 kbit/s"),
            0x14 => Some("1344 kbit/s"),
            0x15 => Some("1408 kbit/s"),
            0x16 => Some("1411.2 kbit/s"),
            0x17 => Some("1472 kbit/s"),
            0x18 => Some("1536 kbit/s"),
            0x19 => Some("1920 kbit/s"),
            0x1A => Some("2048 kbit/s"),
            0x1B => Some("3072 kbit/s"),
            0x1C => Some("3840 kbit/s"),
            0x1D => Some("open"),
            0x1E => Some("variable"),
            0x1F => Some("lossless"),
            _ => None,
        }
    }

    /// Returns a human-readable surround mode label, per
    /// ETSI EN 300 468 Annex G Table G.4.  Returns `None` for user-defined
    /// or unknown codes.
    #[must_use]
    pub fn surround_mode_name(&self) -> Option<&'static str> {
        match self.surround_mode {
            0x00 => Some("1 / mono"),
            0x02 => Some("2 / L+R (stereo)"),
            0x03 => Some("2 / (L+R)+(L-R) (sum-difference)"),
            0x04 => Some("2 / LT+RT (left and right total)"),
            0x05 => Some("3 / C+L+R"),
            0x06 => Some("3 / L+R+S"),
            0x07 => Some("4 / C+L+R+S"),
            0x08 => Some("4 / L+R+SL+SR"),
            0x09 => Some("5 / C+L+R+SL+SR"),
            0x0A..=0x0F => None, // user defined
            0x10..=0x3F => None, // user defined
            _ => None,
        }
    }

    /// Returns a human-readable extended surround flag label, per
    /// ETSI EN 300 468 Annex G Table G.5.
    #[must_use]
    pub fn extended_surround_name(&self) -> &'static str {
        match self.extended_surround_flag {
            0x00 => "no extended surround",
            0x01 => "matrixed extended surround",
            0x02 => "discrete extended surround",
            0x03 => "undefined",
            _ => "unknown",
        }
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
        assert_eq!(d.sample_rate_name(), Some("48 kHz"));
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
        assert_eq!(d.sample_rate_name(), Some("invalid"));
    }

    #[test]
    fn decode_bit_rate_384() {
        // Table G.3: bit_rate_code 0bx01010 → & 0x1F = 0x0A → 384 kbit/s
        let d = DtsDescriptor {
            sample_rate_code: 0,
            bit_rate_code: 0x0A,
            nblks: 0,
            fsize: 0,
            surround_mode: 0,
            lfe_flag: false,
            extended_surround_flag: 0,
            additional_info: &[],
        };
        assert_eq!(d.bit_rate_kbits(), Some(384));
        assert_eq!(d.bit_rate_name(), Some("384 kbit/s"));
    }

    #[test]
    fn decode_bit_rate_128() {
        // Table G.3: bit_rate_code 0bx00101 → & 0x1F = 0x05 → 128 kbit/s
        let d = DtsDescriptor {
            sample_rate_code: 0,
            bit_rate_code: 0x05,
            nblks: 0,
            fsize: 0,
            surround_mode: 0,
            lfe_flag: false,
            extended_surround_flag: 0,
            additional_info: &[],
        };
        assert_eq!(d.bit_rate_kbits(), Some(128));
    }

    #[test]
    fn decode_bit_rate_lossless() {
        // Table G.3: bit_rate_code 0bx11111 → & 0x1F = 0x1F → lossless
        let d = DtsDescriptor {
            sample_rate_code: 0,
            bit_rate_code: 0x3F,
            nblks: 0,
            fsize: 0,
            surround_mode: 0,
            lfe_flag: false,
            extended_surround_flag: 0,
            additional_info: &[],
        };
        assert_eq!(d.bit_rate_kbits(), None);
        assert_eq!(d.bit_rate_name(), Some("lossless"));
    }

    #[test]
    fn decode_bit_rate_reserved_msb_ignored() {
        // The MSB of bit_rate_code is reserved ("x"). Setting it should not
        // change the decoded value. bit_rate_code 0x2A has bit5=1 but
        // bits[4:0]=0x0A → still 384 kbit/s.
        let d_low = DtsDescriptor {
            sample_rate_code: 0,
            bit_rate_code: 0x0A,
            nblks: 0,
            fsize: 0,
            surround_mode: 0,
            lfe_flag: false,
            extended_surround_flag: 0,
            additional_info: &[],
        };
        let d_high = DtsDescriptor {
            sample_rate_code: 0,
            bit_rate_code: 0x2A,
            nblks: 0,
            fsize: 0,
            surround_mode: 0,
            lfe_flag: false,
            extended_surround_flag: 0,
            additional_info: &[],
        };
        assert_eq!(d_low.bit_rate_kbits(), d_high.bit_rate_kbits());
        assert_eq!(d_low.bit_rate_name(), d_high.bit_rate_name());
    }

    #[test]
    fn decode_surround_mode() {
        let d = DtsDescriptor {
            sample_rate_code: 0,
            bit_rate_code: 0,
            nblks: 0,
            fsize: 0,
            surround_mode: 0x09,
            lfe_flag: false,
            extended_surround_flag: 0,
            additional_info: &[],
        };
        assert_eq!(d.surround_mode_name(), Some("5 / C+L+R+SL+SR"));
    }

    #[test]
    fn decode_surround_mode_user_defined() {
        let d = DtsDescriptor {
            sample_rate_code: 0,
            bit_rate_code: 0,
            nblks: 0,
            fsize: 0,
            surround_mode: 0x20,
            lfe_flag: false,
            extended_surround_flag: 0,
            additional_info: &[],
        };
        assert_eq!(d.surround_mode_name(), None);
    }

    #[test]
    fn decode_extended_surround() {
        let d = DtsDescriptor {
            sample_rate_code: 0,
            bit_rate_code: 0,
            nblks: 0,
            fsize: 0,
            surround_mode: 0,
            lfe_flag: false,
            extended_surround_flag: 0x02,
            additional_info: &[],
        };
        assert_eq!(d.extended_surround_name(), "discrete extended surround");
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
