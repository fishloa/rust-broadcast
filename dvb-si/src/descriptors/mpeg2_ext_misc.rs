//! MPEG-2 `Extension_descriptor` bodies — Rec. ITU-T H.222.0 (08/2023) |
//! ISO/IEC 13818-1:2023, `extension_descriptor_tag`:
//!
//! * 0x02 `ODUpdate_descriptor` — §2.6.92 (Table 2-109 selects
//!   `ObjectDescriptorUpdate()` of ISO/IEC 14496-1 §7.2.5.5.2, not vendored: opaque).
//! * 0x04 `af_extensions_descriptor()` — §2.6.99, Table 2-116 (empty syntax).
//! * 0x07 `Green_extension_descriptor()` — §2.6.104, Table 2-120.
//! * 0x0F `Quality_extension_descriptor()` — §2.6.119, Table 2-128.
//! * 0x11 `timed_metadata_extension_descriptor()` — named in Tables 2-109/2-110;
//!   its structure is clause 8.2.3 of ISO/IEC 23001-13 (not vendored: opaque).
//! * 0x14 `JXS_video_descriptor()` — §2.6.127, Table 2-132.
//!
//! (0x10 `Virtual_segmentation_descriptor` is deliberately not typed: the
//! `MDL` byte count is ambiguous in Table 2-129 / §2.6.121, see the docs.)
//!
//! Syntax transcriptions: `docs/descriptors/iso_13818_1/3f-extension-descriptor-bodies-*.md`.
//! Every `reserved` field is a typed field re-emitted verbatim (default: all
//! ones; `zero_bits` of the JPEG XS descriptor defaults to zero as the spec says).

use crate::error::Result;
use crate::h222_bits::{BitReader, BitWriter};
use alloc::vec::Vec;

const RES6: u8 = 0x3F;
/// Default of `reserved(7)` (all ones).
const RES7: u8 = 0x7F;
/// `buffer_model_type`: "only 2 is allowed" (Table 2-132).
const JXS_BUFFER_MODEL_TYPE: u8 = 2;

/// `ObjectDescriptorUpdate()` carried by `ODUpdate_descriptor` (ext 0x02).
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub struct OdUpdate {
    /// The `ObjectDescriptorUpdate()` command bytes, ISO/IEC 14496-1 §7.2.5.5.2
    /// (opaque: that standard is not vendored).
    pub object_descriptor_update: Vec<u8>,
}

impl OdUpdate {
    /// Build from the `ObjectDescriptorUpdate()` command bytes.
    #[must_use]
    pub fn new(object_descriptor_update: Vec<u8>) -> Self {
        Self {
            object_descriptor_update,
        }
    }
}

/// `timed_metadata_extension_descriptor()` (ext 0x11).
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub struct TimedMetadataExtension {
    /// The descriptor body, ISO/IEC 23001-13 clause 8.2.3 (opaque: not vendored).
    pub data: Vec<u8>,
}

impl TimedMetadataExtension {
    /// Build from the opaque descriptor body.
    #[must_use]
    pub fn new(data: Vec<u8>) -> Self {
        Self { data }
    }
}

/// `af_extensions_descriptor()` (ext 0x04): the syntax is empty (Table 2-116).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub struct AfExtensions {
    /// Bytes after the (empty) syntax, preserved verbatim.
    pub trailing: Vec<u8>,
}

/// `Green_extension_descriptor()` (ext 0x07) — Table 2-120.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub struct GreenExtension {
    /// `constant_backlight_voltage_time_interval[i]` (16 bits each; the count
    /// `num_constant_backlight_voltage_time_intervals` is 2 bits, so at most 3).
    pub constant_backlight_voltage_time_intervals: Vec<u16>,
    /// `reserved(6)` after the first count, default `0x3F`.
    pub reserved_a: u8,
    /// `max_variation[j]` (16 bits each; at most 3).
    pub max_variations: Vec<u16>,
    /// `reserved(6)` after the second count, default `0x3F`.
    pub reserved_b: u8,
    /// Bytes after the syntax, preserved verbatim.
    pub trailing: Vec<u8>,
}

impl GreenExtension {
    /// Build with the spec `reserved` fill and no trailing bytes.
    #[must_use]
    pub fn new(
        constant_backlight_voltage_time_intervals: Vec<u16>,
        max_variations: Vec<u16>,
    ) -> Self {
        Self {
            constant_backlight_voltage_time_intervals,
            reserved_a: RES6,
            max_variations,
            reserved_b: RES6,
            trailing: Vec::new(),
        }
    }

    pub(crate) fn parse_body(body: &[u8]) -> Result<Self> {
        let mut r = BitReader::new(body, "GreenExtension");
        let n1 = r.bits(2)?;
        let reserved_a = r.u8(6)?;
        let mut intervals = Vec::new();
        for _ in 0..n1 {
            intervals.push(r.u16(16)?);
        }
        let n2 = r.bits(2)?;
        let reserved_b = r.u8(6)?;
        let mut max_variations = Vec::new();
        for _ in 0..n2 {
            max_variations.push(r.u16(16)?);
        }
        Ok(Self {
            constant_backlight_voltage_time_intervals: intervals,
            reserved_a,
            max_variations,
            reserved_b,
            trailing: r.rest().to_vec(),
        })
    }

    pub(crate) fn write_body(&self) -> Result<Vec<u8>> {
        let mut w = BitWriter::new();
        w.put(
            self.constant_backlight_voltage_time_intervals.len() as u64,
            2,
            "num_constant_backlight_voltage_time_intervals",
        )?;
        w.put(u64::from(self.reserved_a), 6, "reserved")?;
        for v in &self.constant_backlight_voltage_time_intervals {
            w.put(
                u64::from(*v),
                16,
                "constant_backlight_voltage_time_interval",
            )?;
        }
        w.put(self.max_variations.len() as u64, 2, "num_max_variations")?;
        w.put(u64::from(self.reserved_b), 6, "reserved")?;
        for v in &self.max_variations {
            w.put(u64::from(*v), 16, "max_variation")?;
        }
        w.bytes(&self.trailing);
        Ok(w.into_vec())
    }
}

/// `Quality_extension_descriptor()` (ext 0x0F) — Table 2-128.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub struct QualityExtension {
    /// `field_size_bytes`: constant byte size of a metric value in each Quality
    /// Access Unit.
    pub field_size_bytes: u8,
    /// `metric_code[i]` (32 bits each, ISO/IEC 23001-10 §4.2 / §4.3; the
    /// registry is not vendored so the codes stay numeric).
    pub metric_codes: Vec<u32>,
    /// Bytes after the syntax, preserved verbatim.
    pub trailing: Vec<u8>,
}

impl QualityExtension {
    /// Build with no trailing bytes.
    #[must_use]
    pub fn new(field_size_bytes: u8, metric_codes: Vec<u32>) -> Self {
        Self {
            field_size_bytes,
            metric_codes,
            trailing: Vec::new(),
        }
    }

    pub(crate) fn parse_body(body: &[u8]) -> Result<Self> {
        let mut r = BitReader::new(body, "QualityExtension");
        let field_size_bytes = r.u8(8)?;
        let n = r.bits(8)?;
        let mut metric_codes = Vec::new();
        for _ in 0..n {
            metric_codes.push(r.bits(32)?);
        }
        Ok(Self {
            field_size_bytes,
            metric_codes,
            trailing: r.rest().to_vec(),
        })
    }

    pub(crate) fn write_body(&self) -> Result<Vec<u8>> {
        let mut w = BitWriter::new();
        w.put(u64::from(self.field_size_bytes), 8, "field_size_bytes")?;
        w.put(self.metric_codes.len() as u64, 8, "metric_count")?;
        for c in &self.metric_codes {
            w.put(u64::from(*c), 32, "metric_code")?;
        }
        w.bytes(&self.trailing);
        Ok(w.into_vec())
    }
}

/// Mastering-display block of the JPEG XS descriptor (`mdm_flag == '1'`):
/// SMPTE ST 2086 chromaticities / luminance and CTA-861 content light levels.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub struct JxsMasteringDisplay {
    /// `X_c0, Y_c0, X_c1, Y_c1, X_c2, Y_c2` (increments of 0.00002).
    pub primaries: [u16; 6],
    /// `X_wp`.
    pub white_point_x: u16,
    /// `Y_wp`.
    pub white_point_y: u16,
    /// `L_max` (units of 0.0001 cd/m²).
    pub l_max: u32,
    /// `L_min` (units of 0.0001 cd/m²).
    pub l_min: u32,
    /// `MaxCLL` (cd/m²).
    pub max_cll: u16,
    /// `MaxFALL` (cd/m²).
    pub max_fall: u16,
}

impl JxsMasteringDisplay {
    /// Build from every field of the block (all are spec values, no reserved bits).
    #[must_use]
    pub fn new(
        primaries: [u16; 6],
        white_point_x: u16,
        white_point_y: u16,
        l_max: u32,
        l_min: u32,
        max_cll: u16,
        max_fall: u16,
    ) -> Self {
        Self {
            primaries,
            white_point_x,
            white_point_y,
            l_max,
            l_min,
            max_cll,
            max_fall,
        }
    }
}

/// `JXS_video_descriptor()` (ext 0x14) — Table 2-132.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub struct JxsVideo {
    /// `descriptor_version` (shall be 0).
    pub descriptor_version: u8,
    /// `horizontal_size` (`Wf`, ISO/IEC 21122-1).
    pub horizontal_size: u16,
    /// `vertical_size` (`Hf`).
    pub vertical_size: u16,
    /// `brat` — maximum bit rate in Mbit/s (ISO/IEC 21122-3).
    pub brat: u32,
    /// `frat` — frame rate and interlace flag (ISO/IEC 21122-3 coding).
    pub frat: u32,
    /// `schar` — sample characteristics (ISO/IEC 21122-3 coding).
    pub schar: u16,
    /// `Ppih` — profile (ISO/IEC 21122-2).
    pub ppih: u16,
    /// `Plev` — level and sublevel (ISO/IEC 21122-2).
    pub plev: u16,
    /// `max_buffer_size` (Mbytes).
    pub max_buffer_size: u32,
    /// `buffer_model_type` (only 2 is allowed).
    pub buffer_model_type: u8,
    /// `colour_primaries` (ISO/IEC 23091-2).
    pub colour_primaries: u8,
    /// `transfer_characteristics` (ISO/IEC 23091-2).
    pub transfer_characteristics: u8,
    /// `matrix_coefficients` (ISO/IEC 23091-2).
    pub matrix_coefficients: u8,
    /// `video_full_range_flag`.
    pub video_full_range_flag: bool,
    /// `reserved(7)` after `video_full_range_flag`, default `0x7F`.
    pub reserved: u8,
    /// `still_mode`.
    pub still_mode: bool,
    /// `zero_bits(6)`: "shall be all set to '0'"; re-emitted verbatim, default 0.
    pub zero_bits: u8,
    /// The mastering-display block, present iff `mdm_flag == '1'`.
    pub mastering_display: Option<JxsMasteringDisplay>,
    /// `private_data_byte`s.
    pub private_data: Vec<u8>,
}

impl JxsVideo {
    /// Build with the spec fills: `descriptor_version` 0, `buffer_model_type` 2
    /// (the only allowed value), `video_full_range_flag` and `still_mode` clear,
    /// `reserved(7)` all ones, `zero_bits` zero, no mastering display, no private
    /// data. Set the remaining public fields directly to override.
    #[must_use]
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        horizontal_size: u16,
        vertical_size: u16,
        brat: u32,
        frat: u32,
        schar: u16,
        ppih: u16,
        plev: u16,
        max_buffer_size: u32,
        colour_primaries: u8,
        transfer_characteristics: u8,
        matrix_coefficients: u8,
    ) -> Self {
        Self {
            descriptor_version: 0,
            horizontal_size,
            vertical_size,
            brat,
            frat,
            schar,
            ppih,
            plev,
            max_buffer_size,
            buffer_model_type: JXS_BUFFER_MODEL_TYPE,
            colour_primaries,
            transfer_characteristics,
            matrix_coefficients,
            video_full_range_flag: false,
            reserved: RES7,
            still_mode: false,
            zero_bits: 0,
            mastering_display: None,
            private_data: Vec::new(),
        }
    }

    pub(crate) fn parse_body(body: &[u8]) -> Result<Self> {
        let mut r = BitReader::new(body, "JxsVideo");
        let descriptor_version = r.u8(8)?;
        let horizontal_size = r.u16(16)?;
        let vertical_size = r.u16(16)?;
        let brat = r.bits(32)?;
        let frat = r.bits(32)?;
        let schar = r.u16(16)?;
        let ppih = r.u16(16)?;
        let plev = r.u16(16)?;
        let max_buffer_size = r.bits(32)?;
        let buffer_model_type = r.u8(8)?;
        let colour_primaries = r.u8(8)?;
        let transfer_characteristics = r.u8(8)?;
        let matrix_coefficients = r.u8(8)?;
        let video_full_range_flag = r.flag()?;
        let reserved = r.u8(7)?;
        let still_mode = r.flag()?;
        let mdm_flag = r.flag()?;
        let zero_bits = r.u8(6)?;
        let mastering_display = if mdm_flag {
            let mut primaries = [0u16; 6];
            for p in &mut primaries {
                *p = r.u16(16)?;
            }
            Some(JxsMasteringDisplay {
                primaries,
                white_point_x: r.u16(16)?,
                white_point_y: r.u16(16)?,
                l_max: r.bits(32)?,
                l_min: r.bits(32)?,
                max_cll: r.u16(16)?,
                max_fall: r.u16(16)?,
            })
        } else {
            None
        };
        Ok(Self {
            descriptor_version,
            horizontal_size,
            vertical_size,
            brat,
            frat,
            schar,
            ppih,
            plev,
            max_buffer_size,
            buffer_model_type,
            colour_primaries,
            transfer_characteristics,
            matrix_coefficients,
            video_full_range_flag,
            reserved,
            still_mode,
            zero_bits,
            mastering_display,
            private_data: r.rest().to_vec(),
        })
    }

    pub(crate) fn write_body(&self) -> Result<Vec<u8>> {
        let mut w = BitWriter::new();
        w.put(u64::from(self.descriptor_version), 8, "descriptor_version")?;
        w.put(u64::from(self.horizontal_size), 16, "horizontal_size")?;
        w.put(u64::from(self.vertical_size), 16, "vertical_size")?;
        w.put(u64::from(self.brat), 32, "brat")?;
        w.put(u64::from(self.frat), 32, "frat")?;
        w.put(u64::from(self.schar), 16, "schar")?;
        w.put(u64::from(self.ppih), 16, "Ppih")?;
        w.put(u64::from(self.plev), 16, "Plev")?;
        w.put(u64::from(self.max_buffer_size), 32, "max_buffer_size")?;
        w.put(u64::from(self.buffer_model_type), 8, "buffer_model_type")?;
        w.put(u64::from(self.colour_primaries), 8, "colour_primaries")?;
        w.put(
            u64::from(self.transfer_characteristics),
            8,
            "transfer_characteristics",
        )?;
        w.put(
            u64::from(self.matrix_coefficients),
            8,
            "matrix_coefficients",
        )?;
        w.flag(self.video_full_range_flag, "video_full_range_flag")?;
        w.put(u64::from(self.reserved), 7, "reserved")?;
        w.flag(self.still_mode, "still_mode")?;
        w.flag(self.mastering_display.is_some(), "mdm_flag")?;
        w.put(u64::from(self.zero_bits), 6, "zero_bits")?;
        if let Some(m) = &self.mastering_display {
            for p in &m.primaries {
                w.put(u64::from(*p), 16, "X_c/Y_c")?;
            }
            w.put(u64::from(m.white_point_x), 16, "X_wp")?;
            w.put(u64::from(m.white_point_y), 16, "Y_wp")?;
            w.put(u64::from(m.l_max), 32, "L_max")?;
            w.put(u64::from(m.l_min), 32, "L_min")?;
            w.put(u64::from(m.max_cll), 16, "MaxCLL")?;
            w.put(u64::from(m.max_fall), 16, "MaxFALL")?;
        }
        w.bytes(&self.private_data);
        Ok(w.into_vec())
    }
}

#[cfg(test)]
#[allow(clippy::unusual_byte_groupings)] // bit-field-wise literals mirror the syntax tables
mod tests {
    use super::*;

    #[test]
    fn green_spec_vector_and_limits() {
        // Table 2-120: 2 intervals (0x0010,0x0020), 1 max_variation (0x0030), reserved all ones.
        let b = [0b10_111111, 0x00, 0x10, 0x00, 0x20, 0b01_111111, 0x00, 0x30];
        let d = GreenExtension::parse_body(&b).unwrap();
        assert_eq!(d.constant_backlight_voltage_time_intervals, [0x10, 0x20]);
        assert_eq!(d.max_variations, [0x30]);
        assert_eq!(d.write_body().unwrap(), b);
        // 4 entries do not fit the 2-bit count
        let big = GreenExtension::new(vec![0; 4], vec![]);
        assert!(big.write_body().is_err());
        assert!(GreenExtension::parse_body(&b[..4]).is_err());
    }

    #[test]
    fn quality_spec_vector() {
        // field_size_bytes=2, 2 metrics: 'PSNR' and 'SSIM' as 32-bit codes.
        let b = [2, 2, b'P', b'S', b'N', b'R', b'S', b'S', b'I', b'M', 0xEE];
        let d = QualityExtension::parse_body(&b).unwrap();
        assert_eq!(d.metric_codes, [0x5053_4E52, 0x5353_494D]);
        assert_eq!(d.trailing, [0xEE]);
        assert_eq!(d.write_body().unwrap(), b);
        assert!(QualityExtension::new(1, vec![0; 256]).write_body().is_err());
    }

    #[test]
    fn jxs_round_trip_with_and_without_mdm() {
        let mut b = vec![0u8]; // descriptor_version
        b.extend_from_slice(&[0x07, 0x80, 0x04, 0x38]); // 1920x1080
        b.extend_from_slice(&[0, 0, 0, 100]); // brat
        b.extend_from_slice(&[0, 0, 0x00, 0x19]); // frat
        b.extend_from_slice(&[0x01, 0x02, 0x03, 0x04, 0x05, 0x06]); // schar Ppih Plev
        b.extend_from_slice(&[0, 0, 0, 8]); // max_buffer_size
        b.extend_from_slice(&[2, 1, 1, 1]); // buffer model, cicp
        b.push(0b1_1111111); // full range + reserved
        b.push(0b0_0_000000); // still_mode, mdm_flag=0, zero_bits
        b.extend_from_slice(&[0xAB, 0xCD]); // private data
        let d = JxsVideo::parse_body(&b).unwrap();
        assert_eq!(d.horizontal_size, 1920);
        assert_eq!(d.ppih, 0x0304);
        assert!(d.mastering_display.is_none());
        assert_eq!(d.private_data, [0xAB, 0xCD]);
        assert_eq!(d.write_body().unwrap(), b);
        // mdm_flag = 1 adds the 24-byte block
        let mut m = b[..b.len() - 2].to_vec();
        let n = m.len();
        m[n - 1] = 0b0_1_000000;
        for i in 0..8u8 {
            m.push(0);
            m.push(i);
        }
        m.extend_from_slice(&[0, 0, 0x27, 0x10, 0, 0, 0, 1]);
        m.extend_from_slice(&[0x03, 0xE8, 0x01, 0x90]);
        let d = JxsVideo::parse_body(&m).unwrap();
        let md = d.mastering_display.as_ref().unwrap();
        assert_eq!(md.l_max, 10000);
        assert_eq!(md.max_cll, 1000);
        assert_eq!(d.write_body().unwrap(), m);
        assert!(JxsVideo::parse_body(&m[..m.len() - 1]).is_err());
    }
}
