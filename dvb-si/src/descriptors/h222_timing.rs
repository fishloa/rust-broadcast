//! HEVC / VVC / EVC timing and HRD descriptors — Rec. ITU-T H.222.0 (08/2023) §2.6.97, §2.6.131, §2.6.135.
//!
//! `HEVC_timing_and_HRD_descriptor` (Table 2-115, `extension_descriptor_tag`
//! 0x03), `VVC_timing_and_HRD_descriptor` (Table 2-139, 0x15) and
//! `EVC_timing_and_HRD_descriptor` (Table 2-141, 0x16) all carry the same
//! `picture_and_timing_info` block; syntax in
//! `docs/descriptors/iso_13818_1/3f-extension-descriptor-bodies-0x02-0x07.md`
//! and `…-0x0f-0x19.md`.
//!
//! ```text
//! HEVC:  hrd_management_valid_flag(1) target_schedule_idx_not_present_flag(1)
//!        target_schedule_idx(5) picture_and_timing_info_present_flag(1) [info]
//! VVC/EVC: hrd_management_valid_flag(1) reserved(6)
//!        picture_and_timing_info_present_flag(1) [info]
//! info:  90kHz_flag(1) reserved(7) [N(32) K(32) if 90kHz_flag==0] num_units_in_tick(32)
//! ```

use crate::error::Result;
use crate::h222_bits::{BitReader, BitWriter};
use alloc::vec::Vec;

/// Default of every `reserved` field of these descriptors (H.222.0 clause 2.1:
/// "reserved" bits are `1`).
const RESERVED_7_DEFAULT: u8 = 0x7F;
const RESERVED_6_DEFAULT: u8 = 0x3F;
const RESERVED_IDX_DEFAULT: u8 = 0x1F;
const TARGET_SCHEDULE_IDX_BITS: usize = 5;

/// `N` / `K` of the time-base mapping `time_scale = N × system_clock_frequency / K`,
/// present only when `90kHz_flag == '0'` (N = 1, K = 300 otherwise).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub struct TimeScaleNk {
    /// `N` (32 bits).
    pub n: u32,
    /// `K` (32 bits).
    pub k: u32,
}

impl TimeScaleNk {
    /// Build from `N` and `K`.
    #[must_use]
    pub const fn new(n: u32, k: u32) -> Self {
        Self { n, k }
    }
}

/// The `picture_and_timing_info` block shared by the three descriptors.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub struct PictureAndTimingInfo {
    /// `N`/`K`; `None` means `90kHz_flag == '1'` (N = 1, K = 300).
    pub nk: Option<TimeScaleNk>,
    /// `num_units_in_tick` (32 bits; coded as the codec's VUI field).
    pub num_units_in_tick: u32,
    /// `reserved(7)` after `90kHz_flag`, right-aligned; re-emitted verbatim,
    /// default `0x7F`.
    pub reserved: u8,
}

impl PictureAndTimingInfo {
    /// Build with the spec `reserved` fill.
    #[must_use]
    pub const fn new(nk: Option<TimeScaleNk>, num_units_in_tick: u32) -> Self {
        Self {
            nk,
            num_units_in_tick,
            reserved: RESERVED_7_DEFAULT,
        }
    }

    fn read(r: &mut BitReader<'_>) -> Result<Self> {
        let is_90khz = r.flag()?;
        let reserved = r.u8(7)?;
        let nk = if is_90khz {
            None
        } else {
            Some(TimeScaleNk {
                n: r.bits(32)?,
                k: r.bits(32)?,
            })
        };
        Ok(Self {
            nk,
            num_units_in_tick: r.bits(32)?,
            reserved,
        })
    }

    fn write(&self, w: &mut BitWriter) -> Result<()> {
        w.flag(self.nk.is_none(), "90kHz_flag")?;
        w.put(u64::from(self.reserved), 7, "reserved")?;
        if let Some(nk) = &self.nk {
            w.put(u64::from(nk.n), 32, "N")?;
            w.put(u64::from(nk.k), 32, "K")?;
        }
        w.put(u64::from(self.num_units_in_tick), 32, "num_units_in_tick")
    }
}

/// `HEVC_timing_and_HRD_descriptor()` — Table 2-115 (`extension_descriptor_tag` 0x03).
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub struct HevcTimingAndHrd {
    /// `hrd_management_valid_flag`.
    pub hrd_management_valid_flag: bool,
    /// `target_schedule_idx` (5 bits); `None` means
    /// `target_schedule_idx_not_present_flag == '1'`.
    pub target_schedule_idx: Option<u8>,
    /// The 5 "unspecified" bits that stand in for `target_schedule_idx` when it
    /// is not present (right-aligned, default `0x1F`); ignored otherwise.
    pub unspecified_schedule_bits: u8,
    /// `picture_and_timing_info`, present iff `picture_and_timing_info_present_flag`.
    pub picture_timing: Option<PictureAndTimingInfo>,
    /// Bytes after the syntax, preserved verbatim.
    pub trailing: Vec<u8>,
}

impl HevcTimingAndHrd {
    /// Build with the spec fill and no trailing bytes.
    #[must_use]
    pub fn new(
        hrd_management_valid_flag: bool,
        target_schedule_idx: Option<u8>,
        picture_timing: Option<PictureAndTimingInfo>,
    ) -> Self {
        Self {
            hrd_management_valid_flag,
            target_schedule_idx,
            unspecified_schedule_bits: RESERVED_IDX_DEFAULT,
            picture_timing,
            trailing: Vec::new(),
        }
    }

    pub(crate) fn parse_body(body: &[u8]) -> Result<Self> {
        let mut r = BitReader::new(body, "HevcTimingAndHrd");
        let hrd_management_valid_flag = r.flag()?;
        let not_present = r.flag()?;
        let idx = r.u8(TARGET_SCHEDULE_IDX_BITS)?;
        let info_present = r.flag()?;
        let picture_timing = if info_present {
            Some(PictureAndTimingInfo::read(&mut r)?)
        } else {
            None
        };
        Ok(Self {
            hrd_management_valid_flag,
            target_schedule_idx: (!not_present).then_some(idx),
            unspecified_schedule_bits: if not_present {
                idx
            } else {
                RESERVED_IDX_DEFAULT
            },
            picture_timing,
            trailing: r.rest().to_vec(),
        })
    }

    pub(crate) fn write_body(&self) -> Result<Vec<u8>> {
        let mut w = BitWriter::new();
        w.flag(self.hrd_management_valid_flag, "hrd_management_valid_flag")?;
        w.flag(
            self.target_schedule_idx.is_none(),
            "target_schedule_idx_not_present_flag",
        )?;
        let idx = self
            .target_schedule_idx
            .unwrap_or(self.unspecified_schedule_bits);
        w.put(
            u64::from(idx),
            TARGET_SCHEDULE_IDX_BITS,
            "target_schedule_idx",
        )?;
        w.flag(
            self.picture_timing.is_some(),
            "picture_and_timing_info_present_flag",
        )?;
        if let Some(p) = &self.picture_timing {
            p.write(&mut w)?;
        }
        w.bytes(&self.trailing);
        Ok(w.into_vec())
    }
}

/// `hrd_management_valid_flag(1) reserved(6) picture_and_timing_info_present_flag(1)
/// [picture_and_timing_info]` — identical in Tables 2-139 and 2-141.
fn read_flag_block(r: &mut BitReader<'_>) -> Result<(bool, u8, Option<PictureAndTimingInfo>)> {
    let hrd_management_valid_flag = r.flag()?;
    let reserved = r.u8(6)?;
    let picture_timing = if r.flag()? {
        Some(PictureAndTimingInfo::read(r)?)
    } else {
        None
    };
    Ok((hrd_management_valid_flag, reserved, picture_timing))
}

fn write_flag_block(
    w: &mut BitWriter,
    hrd_management_valid_flag: bool,
    reserved: u8,
    picture_timing: Option<&PictureAndTimingInfo>,
) -> Result<()> {
    w.flag(hrd_management_valid_flag, "hrd_management_valid_flag")?;
    w.put(u64::from(reserved), 6, "reserved")?;
    w.flag(
        picture_timing.is_some(),
        "picture_and_timing_info_present_flag",
    )?;
    if let Some(p) = picture_timing {
        p.write(w)?;
    }
    Ok(())
}

/// `VVC_timing_and_HRD_descriptor()` — Table 2-139 (`extension_descriptor_tag` 0x15).
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub struct VvcTimingAndHrd {
    /// `hrd_management_valid_flag`.
    pub hrd_management_valid_flag: bool,
    /// `reserved(6)` after the flag, right-aligned; re-emitted verbatim,
    /// default `0x3F`.
    pub reserved: u8,
    /// `picture_and_timing_info`, present iff `picture_and_timing_info_present_flag`.
    pub picture_timing: Option<PictureAndTimingInfo>,
    /// Bytes after the syntax, preserved verbatim.
    pub trailing: Vec<u8>,
}

impl VvcTimingAndHrd {
    /// Build with the spec `reserved` fill and no trailing bytes.
    #[must_use]
    pub fn new(
        hrd_management_valid_flag: bool,
        picture_timing: Option<PictureAndTimingInfo>,
    ) -> Self {
        Self {
            hrd_management_valid_flag,
            reserved: RESERVED_6_DEFAULT,
            picture_timing,
            trailing: Vec::new(),
        }
    }

    pub(crate) fn parse_body(body: &[u8]) -> Result<Self> {
        let mut r = BitReader::new(body, "VvcTimingAndHrd");
        let (hrd_management_valid_flag, reserved, picture_timing) = read_flag_block(&mut r)?;
        Ok(Self {
            hrd_management_valid_flag,
            reserved,
            picture_timing,
            trailing: r.rest().to_vec(),
        })
    }

    pub(crate) fn write_body(&self) -> Result<Vec<u8>> {
        let mut w = BitWriter::new();
        write_flag_block(
            &mut w,
            self.hrd_management_valid_flag,
            self.reserved,
            self.picture_timing.as_ref(),
        )?;
        w.bytes(&self.trailing);
        Ok(w.into_vec())
    }
}

/// `EVC_timing_and_HRD_descriptor()` — Table 2-141 (`extension_descriptor_tag` 0x16).
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub struct EvcTimingAndHrd {
    /// `hrd_management_valid_flag`.
    pub hrd_management_valid_flag: bool,
    /// `reserved(6)` after the flag, right-aligned; re-emitted verbatim,
    /// default `0x3F`.
    pub reserved: u8,
    /// `picture_and_timing_info`, present iff `picture_and_timing_info_present_flag`.
    pub picture_timing: Option<PictureAndTimingInfo>,
    /// Bytes after the syntax, preserved verbatim.
    pub trailing: Vec<u8>,
}

impl EvcTimingAndHrd {
    /// Build with the spec `reserved` fill and no trailing bytes.
    #[must_use]
    pub fn new(
        hrd_management_valid_flag: bool,
        picture_timing: Option<PictureAndTimingInfo>,
    ) -> Self {
        Self {
            hrd_management_valid_flag,
            reserved: RESERVED_6_DEFAULT,
            picture_timing,
            trailing: Vec::new(),
        }
    }

    pub(crate) fn parse_body(body: &[u8]) -> Result<Self> {
        let mut r = BitReader::new(body, "EvcTimingAndHrd");
        let (hrd_management_valid_flag, reserved, picture_timing) = read_flag_block(&mut r)?;
        Ok(Self {
            hrd_management_valid_flag,
            reserved,
            picture_timing,
            trailing: r.rest().to_vec(),
        })
    }

    pub(crate) fn write_body(&self) -> Result<Vec<u8>> {
        let mut w = BitWriter::new();
        write_flag_block(
            &mut w,
            self.hrd_management_valid_flag,
            self.reserved,
            self.picture_timing.as_ref(),
        )?;
        w.bytes(&self.trailing);
        Ok(w.into_vec())
    }
}

#[cfg(test)]
#[allow(clippy::unusual_byte_groupings)] // bit-field-wise literals mirror the syntax tables
mod tests {
    use super::*;

    #[test]
    fn hevc_90khz_and_nk_round_trip() {
        // hrd=1, not_present=1, idx(unspecified)=0x1F, info_present=1 -> 0xFF; 90kHz=1,reserved=7F -> 0xFF; ticks
        let b = [0xFF, 0xFF, 0x00, 0x00, 0x03, 0xE9];
        let d = HevcTimingAndHrd::parse_body(&b).unwrap();
        assert!(d.hrd_management_valid_flag);
        assert_eq!(d.target_schedule_idx, None);
        let p = d.picture_timing.as_ref().unwrap();
        assert_eq!(p.nk, None);
        assert_eq!(p.num_units_in_tick, 1001);
        assert_eq!(d.write_body().unwrap(), b);
        // 90kHz=0 -> N and K on the wire, schedule idx present (= 3)
        let b2 = [
            0b1_0_00011_1,
            0x7F,
            0,
            0,
            0,
            1,
            0,
            0,
            1,
            0x2C,
            0,
            0,
            0,
            0x64,
        ];
        let d2 = HevcTimingAndHrd::parse_body(&b2).unwrap();
        assert_eq!(d2.target_schedule_idx, Some(3));
        let nk = d2.picture_timing.as_ref().unwrap().nk.unwrap();
        assert_eq!((nk.n, nk.k), (1, 300));
        assert_eq!(d2.write_body().unwrap(), b2);
    }

    #[test]
    fn hevc_truncated_and_overflow() {
        assert!(HevcTimingAndHrd::parse_body(&[0xFF, 0x7F]).is_err());
        let mut d = HevcTimingAndHrd::new(true, Some(0x20), None);
        assert!(d.write_body().is_err());
        d.target_schedule_idx = Some(0x1F);
        assert!(d.write_body().is_ok());
    }

    #[test]
    fn vvc_and_evc_round_trip_with_reserved_and_trailing() {
        let b = [0b0_111111_0, 0xAA, 0xBB];
        let v = VvcTimingAndHrd::parse_body(&b).unwrap();
        assert!(v.picture_timing.is_none());
        assert_eq!(v.trailing, [0xAA, 0xBB]);
        assert_eq!(v.write_body().unwrap(), b);
        // non-default reserved bits survive
        let b = [0b1_000000_1, 0x80, 0, 0, 0, 7];
        let e = EvcTimingAndHrd::parse_body(&b).unwrap();
        assert_eq!(e.reserved, 0);
        assert_eq!(e.picture_timing.as_ref().unwrap().reserved, 0);
        assert_eq!(e.write_body().unwrap(), b);
    }
}
