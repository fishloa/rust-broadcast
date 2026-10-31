//! HDR10+ dynamic metadata: SMPTE ST 2094-40 Application #4 carried in an HEVC
//! prefix SEI `user_data_registered_itu_t_t35` message (one per access unit).
//!
//! - Semantics, ranges and units: `private/specs-verified/smpte/hdr10plus/st2094-40-2020.md`
//!   (clauses 5-9; equations in `st2094-40-2020.corrections.md`) and
//!   `st2094-1-2016.md`.
//! - Wire syntax: CTA-861-H Annex S, which is paywalled and NOT vendored. The
//!   T.35 header values and every per-field bit width are RECONSTRUCTED from
//!   FFmpeg `av_dynamic_hdr_plus_from_t35` and `hdr10plus_tool`, cross-checked
//!   against the denominators of ST 2094-40 clauses 7.2 and 8.2-8.8 and a real
//!   stream; see `private/specs-verified/smpte/hdr10plus/hdr10plus-t35-carriage-and-wire-syntax.md`.
//!   Two points there stay unconfirmed: the unit of
//!   `targeted_system_display_maximum_luminance` (kept as the raw 27-bit
//!   integer) and the position of `color_saturation_mapping_flag` when more
//!   than one window is present (read per window, next to `tone_mapping_flag`,
//!   as FFmpeg and the A/341 element list do; `hdr10plus_tool` reads it once
//!   after the loop; the two coincide for the single-window streams seen).
//! - HEVC carriage (prefix SEI NAL type 39, payloadType 4): ETSI TS 101 154
//!   L.3.3.10.4.4 and ATSC A/341 6.3.2.2.2.
//!
//! Leniency: parsing enforces only what the bit syntax forces plus the
//! structural limits of ST 2094-40 (1..=3 windows, at most 25x25 peak
//! luminance tables, `ApplicationIdentifier` 4, `ApplicationVersion` 0 or 1).
//! The `ApplicationVersion` 1 value constraints (9 distribution entries, at
//! most 9 Bezier anchors, Table 1 `V1`/`V2`) are NOT enforced: the committed
//! real stream (`private/fixtures/hdr10plus/`) violates Table 1 and a parser
//! that rejected it would drop signalling real encoders emit. Serialization is
//! symmetric: every parsed value, including the alignment padding bits, is
//! re-emitted unchanged.

use crate::error::{Error, Result};
use alloc::vec::Vec;
use broadcast_common::bits::{BitError, BitReader, BitWriter};
use broadcast_common::{Parse, Serialize};

/// SEI `payloadType` of `user_data_registered_itu_t_t35` (H.265 D.2.6).
pub const SEI_PAYLOAD_USER_DATA_REGISTERED_ITU_T_T35: u32 = 4;
/// `itu_t_t35_country_code` of the United States (ITU-T T.35 Annex A).
pub const T35_COUNTRY_CODE_US: u8 = 0xB5;
/// `itu_t_t35_terminal_provider_code` of the HDR10+ payload.
pub const T35_PROVIDER_CODE: u16 = 0x003C;
/// `itu_t_t35_terminal_provider_oriented_code` of the HDR10+ payload.
pub const T35_PROVIDER_ORIENTED_CODE: u16 = 0x0001;
/// ST 2094-40 clause 5: `ApplicationIdentifier`.
pub const APPLICATION_IDENTIFIER: u8 = 4;
/// ST 2094-40 clause 5: the highest defined `ApplicationVersion`.
pub const APPLICATION_VERSION_MAX: u8 = 1;

/// Length of the T.35 header through `application_version`.
pub const T35_HEADER_LEN: usize = 7;
/// The seven header bytes of every HDR10+ T.35 payload with
/// `ApplicationVersion` 1 (`B5 00 3C 00 01 04 01`).
pub const T35_HEADER_V1: [u8; T35_HEADER_LEN] = [
    T35_COUNTRY_CODE_US,
    (T35_PROVIDER_CODE >> BITS_PER_BYTE) as u8,
    T35_PROVIDER_CODE as u8,
    (T35_PROVIDER_ORIENTED_CODE >> BITS_PER_BYTE) as u8,
    T35_PROVIDER_ORIENTED_CODE as u8,
    APPLICATION_IDENTIFIER,
    APPLICATION_VERSION_MAX,
];

const BITS_PER_BYTE: u32 = 8;
const HEVC_NAL_HEADER_LEN: usize = 2;
const HEVC_NAL_TYPE_SHIFT: u8 = 1;
const HEVC_PREFIX_SEI_LAYER_ZERO_TID_PLUS1: u8 = 1;
const SEI_RBSP_TRAILING_BYTE: u8 = 0x80;
const SEI_SIZE_ESCAPE_BYTE: u8 = 0xFF;
const EMULATION_PREVENTION_BYTE: u8 = 0x03;
const EMULATION_ZERO_RUN: usize = 2;
const NAL_LENGTH_PREFIX_LEN: usize = 4;

const NUM_WINDOWS_BITS: u32 = 2;
const MIN_WINDOWS: usize = 1;
const MAX_WINDOWS: usize = 3;
const WINDOW_CORNER_BITS: u32 = 16;
const ELLIPSE_CENTER_BITS: u32 = 16;
const ROTATION_ANGLE_BITS: u32 = 8;
const ELLIPSE_AXIS_BITS: u32 = 16;
const OVERLAP_PROCESS_OPTION_BITS: u32 = 1;
const TARGETED_MAX_LUMINANCE_BITS: u32 = 27;
const FLAG_BITS: u32 = 1;
const LUT_DIMENSION_BITS: u32 = 5;
const LUT_MAX_DIMENSION: u8 = 25;
const LUT_VALUE_BITS: u32 = 4;
const MAXSCL_BITS: u32 = 17;
const MAXSCL_COMPONENTS: usize = 3;
const AVERAGE_MAXRGB_BITS: u32 = 17;
const DISTRIBUTION_COUNT_BITS: u32 = 4;
const DISTRIBUTION_PERCENTAGE_BITS: u32 = 7;
const DISTRIBUTION_PERCENTILE_BITS: u32 = 17;
const FRACTION_BRIGHT_PIXELS_BITS: u32 = 10;
const KNEE_POINT_BITS: u32 = 12;
const BEZIER_ANCHOR_COUNT_BITS: u32 = 4;
const BEZIER_ANCHOR_BITS: u32 = 10;
const COLOR_SATURATION_WEIGHT_BITS: u32 = 6;

fn bit_err(what: &'static str, e: BitError) -> Error {
    match e {
        BitError::OutOfBounds {
            needed_bits,
            remaining_bits,
        } => Error::BufferTooShort {
            need: remaining_bits.div_ceil(8) + needed_bits.div_ceil(8),
            have: remaining_bits.div_ceil(8),
            what,
        },
        BitError::TooManyBits { requested } => Error::InvalidValue {
            field: what,
            value: u64::from(requested),
            reason: "field wider than 64 bits",
        },
        BitError::ValueTooWide { value, .. } => Error::InvalidValue {
            field: what,
            value,
            reason: "value does not fit the field width",
        },
        _ => Error::InvalidValue {
            field: what,
            value: 0,
            reason: "bit reader error",
        },
    }
}

fn read(r: &mut BitReader<'_>, bits: u32, what: &'static str) -> Result<u64> {
    r.read_bits(bits).map_err(|e| bit_err(what, e))
}

fn write(w: &mut BitWriter<'_>, value: u64, bits: u32, what: &'static str) -> Result<()> {
    let value = broadcast_common::len::fit_bits(value, bits, what)?;
    w.write_bits(value, bits).map_err(|e| bit_err(what, e))
}

/// `overlap_process_option` (ST 2094-40 clause 6.7): how the rendered pixels
/// of overlapping elliptical selectors are combined.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum OverlapProcessOption {
    /// 0: weighted average with window 0 (clause 6.7.2).
    WeightedAverage,
    /// 1: layering, lowest non-zero-weighted window wins (clause 6.7.3).
    Layering,
}

impl OverlapProcessOption {
    /// The 1-bit wire value.
    pub fn code(&self) -> u8 {
        match self {
            Self::WeightedAverage => 0,
            Self::Layering => 1,
        }
    }

    fn from_bit(bit: bool) -> Self {
        if bit {
            Self::Layering
        } else {
            Self::WeightedAverage
        }
    }

    /// Spec label.
    pub fn name(&self) -> &'static str {
        match self {
            Self::WeightedAverage => "weighted average",
            Self::Layering => "layering",
        }
    }
}
broadcast_common::impl_spec_display!(OverlapProcessOption);

/// The extra pixel selector of a window with `WindowNumber` > 0: the bounding
/// rectangle plus the concentric elliptical selector (ST 2094-40 clause 6;
/// window rectangle from ST 2094-1).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WindowRegion {
    /// `window_upper_left_corner_x`.
    pub upper_left_corner_x: u16,
    /// `window_upper_left_corner_y`.
    pub upper_left_corner_y: u16,
    /// `window_lower_right_corner_x`.
    pub lower_right_corner_x: u16,
    /// `window_lower_right_corner_y`.
    pub lower_right_corner_y: u16,
    /// `center_of_ellipse_x` (clause 6.2).
    pub center_of_ellipse_x: u16,
    /// `center_of_ellipse_y` (clause 6.2).
    pub center_of_ellipse_y: u16,
    /// `rotation_angle`, degrees clockwise, 0..=180 (clause 6.3).
    pub rotation_angle: u8,
    /// `semimajor_axis_internal_ellipse`, pixels (clause 6.4).
    pub semimajor_axis_internal_ellipse: u16,
    /// `semimajor_axis_external_ellipse`, pixels (clause 6.5).
    pub semimajor_axis_external_ellipse: u16,
    /// `semiminor_axis_external_ellipse`, pixels (clause 6.6).
    pub semiminor_axis_external_ellipse: u16,
    /// `overlap_process_option` (clause 6.7).
    pub overlap_process_option: OverlapProcessOption,
}

/// A two-input sampled actual-peak-luminance table (ST 2094-40 clauses 7.2 and
/// 8.2): `rows` x `columns` outputs, each a multiple of 1/15.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PeakLuminanceTable {
    /// `num_rows_*_actual_peak_luminance`, at most 25.
    pub rows: u8,
    /// `num_cols_*_actual_peak_luminance`, at most 25.
    pub columns: u8,
    /// `actual_peak_luminance[i][j]` in row-major order, in units of 1/15
    /// (0..=15); `rows * columns` entries.
    pub values: Vec<u8>,
}

/// One `DistributionMaxRGB` point (ST 2094-40 clause 8.5).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DistributionPoint {
    /// `distribution_maxrgb_percentages`, 0..=100.
    pub percentage: u8,
    /// `distribution_maxrgb_percentiles`, in units of 0.00001.
    pub percentile: u32,
}

/// `KneePoint` and `BezierCurveAnchors` (ST 2094-40 clauses 8.7.2-8.7.3).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BezierToneMapping {
    /// `knee_point_x`, in units of 1/4095.
    pub knee_point_x: u16,
    /// `knee_point_y`, in units of 1/4095.
    pub knee_point_y: u16,
    /// `bezier_curve_anchors`, each in units of 1/1023 (at most 15).
    pub anchors: Vec<u16>,
}

/// The `ColorVolumeTransform` items of one processing window (ST 2094-40
/// clauses 8.3-8.8).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ColorVolumeTransform {
    /// `maxscl[0..3]` (R, G, B), in units of 0.00001 (clause 8.3).
    pub max_scl: [u32; MAXSCL_COMPONENTS],
    /// `average_maxrgb`, in units of 0.00001 (clause 8.4).
    pub average_max_rgb: u32,
    /// `DistributionMaxRGB` (clause 8.5), at most 15 points.
    pub distribution_max_rgb: Vec<DistributionPoint>,
    /// `fraction_bright_pixels`, in units of 0.001 (clause 8.6).
    pub fraction_bright_pixels: u16,
    /// `tone_mapping_flag` and its payload (clause 8.7); `None` when the flag
    /// is clear.
    pub tone_mapping: Option<BezierToneMapping>,
    /// `color_saturation_weight`, in units of 1/8 (clause 8.8); `None` when
    /// `color_saturation_mapping_flag` is clear.
    pub color_saturation_weight: Option<u8>,
}

/// A processing window with `WindowNumber` > 0.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AdditionalWindow {
    /// Rectangle and elliptical selector.
    pub region: WindowRegion,
    /// The window's colour volume transform items.
    pub color_volume: ColorVolumeTransform,
}

/// HDR10+ dynamic metadata of one access unit: the T.35 payload of ST 2094-40
/// Application #4.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Hdr10PlusMetadata {
    /// `application_version` (ST 2094-40 clause 5: 0 or 1).
    pub application_version: u8,
    /// `targeted_system_display_maximum_luminance`, the raw 27-bit field.
    pub targeted_system_display_maximum_luminance: u32,
    /// `TargetedSystemDisplayActualPeakLuminance` (clause 7.2).
    pub targeted_system_display_actual_peak_luminance: Option<PeakLuminanceTable>,
    /// Window 0, which covers the whole image.
    pub primary_window: ColorVolumeTransform,
    /// Windows 1 and 2 (at most two).
    pub additional_windows: Vec<AdditionalWindow>,
    /// `MasteringDisplayActualPeakLuminance` (clause 8.2).
    pub mastering_display_actual_peak_luminance: Option<PeakLuminanceTable>,
    /// Value of the unused low bits of the last payload byte (zero in every
    /// stream seen), kept so a re-serialization is byte-identical.
    pub alignment_padding: u8,
}

fn read_lut(r: &mut BitReader<'_>, what: &'static str) -> Result<PeakLuminanceTable> {
    let rows = read(r, LUT_DIMENSION_BITS, what)? as u8;
    let columns = read(r, LUT_DIMENSION_BITS, what)? as u8;
    if rows > LUT_MAX_DIMENSION || columns > LUT_MAX_DIMENSION {
        return Err(Error::InvalidValue {
            field: what,
            value: u64::from(rows.max(columns)),
            reason: "more than 25 rows or columns (ST 2094-40 clause 7.2)",
        });
    }
    let count = usize::from(rows) * usize::from(columns);
    let mut values = Vec::with_capacity(count);
    for _ in 0..count {
        values.push(read(r, LUT_VALUE_BITS, what)? as u8);
    }
    Ok(PeakLuminanceTable {
        rows,
        columns,
        values,
    })
}

fn lut_bits(lut: &Option<PeakLuminanceTable>) -> usize {
    lut.as_ref().map_or(0, |l| {
        2 * LUT_DIMENSION_BITS as usize + l.values.len() * LUT_VALUE_BITS as usize
    })
}

fn write_lut(
    w: &mut BitWriter<'_>,
    lut: &Option<PeakLuminanceTable>,
    what: &'static str,
) -> Result<()> {
    write(w, u64::from(lut.is_some()), FLAG_BITS, what)?;
    let Some(l) = lut else { return Ok(()) };
    if l.rows > LUT_MAX_DIMENSION
        || l.columns > LUT_MAX_DIMENSION
        || l.values.len() != usize::from(l.rows) * usize::from(l.columns)
    {
        return Err(Error::InvalidValue {
            field: what,
            value: l.values.len() as u64,
            reason: "table size disagrees with rows x columns or exceeds 25x25",
        });
    }
    write(w, u64::from(l.rows), LUT_DIMENSION_BITS, what)?;
    write(w, u64::from(l.columns), LUT_DIMENSION_BITS, what)?;
    for v in &l.values {
        write(w, u64::from(*v), LUT_VALUE_BITS, what)?;
    }
    Ok(())
}

fn read_color_volume(r: &mut BitReader<'_>) -> Result<ColorVolumeTransform> {
    let mut max_scl = [0u32; MAXSCL_COMPONENTS];
    for c in &mut max_scl {
        *c = read(r, MAXSCL_BITS, "maxscl")? as u32;
    }
    let average_max_rgb = read(r, AVERAGE_MAXRGB_BITS, "average_maxrgb")? as u32;
    let count = read(
        r,
        DISTRIBUTION_COUNT_BITS,
        "num_distribution_maxrgb_percentiles",
    )? as usize;
    let mut distribution_max_rgb = Vec::with_capacity(count);
    for _ in 0..count {
        distribution_max_rgb.push(DistributionPoint {
            percentage: read(
                r,
                DISTRIBUTION_PERCENTAGE_BITS,
                "distribution_maxrgb_percentages",
            )? as u8,
            percentile: read(
                r,
                DISTRIBUTION_PERCENTILE_BITS,
                "distribution_maxrgb_percentiles",
            )? as u32,
        });
    }
    let fraction_bright_pixels =
        read(r, FRACTION_BRIGHT_PIXELS_BITS, "fraction_bright_pixels")? as u16;
    Ok(ColorVolumeTransform {
        max_scl,
        average_max_rgb,
        distribution_max_rgb,
        fraction_bright_pixels,
        tone_mapping: None,
        color_saturation_weight: None,
    })
}

fn color_volume_bits(c: &ColorVolumeTransform) -> usize {
    MAXSCL_COMPONENTS * MAXSCL_BITS as usize
        + AVERAGE_MAXRGB_BITS as usize
        + DISTRIBUTION_COUNT_BITS as usize
        + c.distribution_max_rgb.len()
            * (DISTRIBUTION_PERCENTAGE_BITS + DISTRIBUTION_PERCENTILE_BITS) as usize
        + FRACTION_BRIGHT_PIXELS_BITS as usize
}

fn write_color_volume(w: &mut BitWriter<'_>, c: &ColorVolumeTransform) -> Result<()> {
    for v in c.max_scl {
        write(w, u64::from(v), MAXSCL_BITS, "maxscl")?;
    }
    write(
        w,
        u64::from(c.average_max_rgb),
        AVERAGE_MAXRGB_BITS,
        "average_maxrgb",
    )?;
    write(
        w,
        c.distribution_max_rgb.len() as u64,
        DISTRIBUTION_COUNT_BITS,
        "num_distribution_maxrgb_percentiles",
    )?;
    for p in &c.distribution_max_rgb {
        write(
            w,
            u64::from(p.percentage),
            DISTRIBUTION_PERCENTAGE_BITS,
            "distribution_maxrgb_percentages",
        )?;
        write(
            w,
            u64::from(p.percentile),
            DISTRIBUTION_PERCENTILE_BITS,
            "distribution_maxrgb_percentiles",
        )?;
    }
    write(
        w,
        u64::from(c.fraction_bright_pixels),
        FRACTION_BRIGHT_PIXELS_BITS,
        "fraction_bright_pixels",
    )
}

fn read_transform_tail(r: &mut BitReader<'_>, c: &mut ColorVolumeTransform) -> Result<()> {
    if read(r, FLAG_BITS, "tone_mapping_flag")? != 0 {
        let knee_point_x = read(r, KNEE_POINT_BITS, "knee_point_x")? as u16;
        let knee_point_y = read(r, KNEE_POINT_BITS, "knee_point_y")? as u16;
        let count = read(r, BEZIER_ANCHOR_COUNT_BITS, "num_bezier_curve_anchors")? as usize;
        let mut anchors = Vec::with_capacity(count);
        for _ in 0..count {
            anchors.push(read(r, BEZIER_ANCHOR_BITS, "bezier_curve_anchors")? as u16);
        }
        c.tone_mapping = Some(BezierToneMapping {
            knee_point_x,
            knee_point_y,
            anchors,
        });
    }
    if read(r, FLAG_BITS, "color_saturation_mapping_flag")? != 0 {
        c.color_saturation_weight =
            Some(read(r, COLOR_SATURATION_WEIGHT_BITS, "color_saturation_weight")? as u8);
    }
    Ok(())
}

fn transform_tail_bits(c: &ColorVolumeTransform) -> usize {
    let tone = c.tone_mapping.as_ref().map_or(FLAG_BITS as usize, |t| {
        FLAG_BITS as usize
            + 2 * KNEE_POINT_BITS as usize
            + BEZIER_ANCHOR_COUNT_BITS as usize
            + t.anchors.len() * BEZIER_ANCHOR_BITS as usize
    });
    let saturation = FLAG_BITS as usize
        + c.color_saturation_weight
            .map_or(0, |_| COLOR_SATURATION_WEIGHT_BITS as usize);
    tone + saturation
}

fn write_transform_tail(w: &mut BitWriter<'_>, c: &ColorVolumeTransform) -> Result<()> {
    write(
        w,
        u64::from(c.tone_mapping.is_some()),
        FLAG_BITS,
        "tone_mapping_flag",
    )?;
    if let Some(t) = &c.tone_mapping {
        write(
            w,
            u64::from(t.knee_point_x),
            KNEE_POINT_BITS,
            "knee_point_x",
        )?;
        write(
            w,
            u64::from(t.knee_point_y),
            KNEE_POINT_BITS,
            "knee_point_y",
        )?;
        write(
            w,
            t.anchors.len() as u64,
            BEZIER_ANCHOR_COUNT_BITS,
            "num_bezier_curve_anchors",
        )?;
        for a in &t.anchors {
            write(w, u64::from(*a), BEZIER_ANCHOR_BITS, "bezier_curve_anchors")?;
        }
    }
    write(
        w,
        u64::from(c.color_saturation_weight.is_some()),
        FLAG_BITS,
        "color_saturation_mapping_flag",
    )?;
    if let Some(weight) = c.color_saturation_weight {
        write(
            w,
            u64::from(weight),
            COLOR_SATURATION_WEIGHT_BITS,
            "color_saturation_weight",
        )?;
    }
    Ok(())
}

fn read_region(r: &mut BitReader<'_>) -> Result<WindowRegion> {
    Ok(WindowRegion {
        upper_left_corner_x: read(r, WINDOW_CORNER_BITS, "window_upper_left_corner_x")? as u16,
        upper_left_corner_y: read(r, WINDOW_CORNER_BITS, "window_upper_left_corner_y")? as u16,
        lower_right_corner_x: read(r, WINDOW_CORNER_BITS, "window_lower_right_corner_x")? as u16,
        lower_right_corner_y: read(r, WINDOW_CORNER_BITS, "window_lower_right_corner_y")? as u16,
        center_of_ellipse_x: read(r, ELLIPSE_CENTER_BITS, "center_of_ellipse_x")? as u16,
        center_of_ellipse_y: read(r, ELLIPSE_CENTER_BITS, "center_of_ellipse_y")? as u16,
        rotation_angle: read(r, ROTATION_ANGLE_BITS, "rotation_angle")? as u8,
        semimajor_axis_internal_ellipse: read(
            r,
            ELLIPSE_AXIS_BITS,
            "semimajor_axis_internal_ellipse",
        )? as u16,
        semimajor_axis_external_ellipse: read(
            r,
            ELLIPSE_AXIS_BITS,
            "semimajor_axis_external_ellipse",
        )? as u16,
        semiminor_axis_external_ellipse: read(
            r,
            ELLIPSE_AXIS_BITS,
            "semiminor_axis_external_ellipse",
        )? as u16,
        overlap_process_option: OverlapProcessOption::from_bit(
            read(r, OVERLAP_PROCESS_OPTION_BITS, "overlap_process_option")? != 0,
        ),
    })
}

const REGION_BITS: usize = 4 * WINDOW_CORNER_BITS as usize
    + 2 * ELLIPSE_CENTER_BITS as usize
    + ROTATION_ANGLE_BITS as usize
    + 3 * ELLIPSE_AXIS_BITS as usize
    + OVERLAP_PROCESS_OPTION_BITS as usize;

fn write_region(w: &mut BitWriter<'_>, g: &WindowRegion) -> Result<()> {
    write(
        w,
        u64::from(g.upper_left_corner_x),
        WINDOW_CORNER_BITS,
        "window_upper_left_corner_x",
    )?;
    write(
        w,
        u64::from(g.upper_left_corner_y),
        WINDOW_CORNER_BITS,
        "window_upper_left_corner_y",
    )?;
    write(
        w,
        u64::from(g.lower_right_corner_x),
        WINDOW_CORNER_BITS,
        "window_lower_right_corner_x",
    )?;
    write(
        w,
        u64::from(g.lower_right_corner_y),
        WINDOW_CORNER_BITS,
        "window_lower_right_corner_y",
    )?;
    write(
        w,
        u64::from(g.center_of_ellipse_x),
        ELLIPSE_CENTER_BITS,
        "center_of_ellipse_x",
    )?;
    write(
        w,
        u64::from(g.center_of_ellipse_y),
        ELLIPSE_CENTER_BITS,
        "center_of_ellipse_y",
    )?;
    write(
        w,
        u64::from(g.rotation_angle),
        ROTATION_ANGLE_BITS,
        "rotation_angle",
    )?;
    write(
        w,
        u64::from(g.semimajor_axis_internal_ellipse),
        ELLIPSE_AXIS_BITS,
        "semimajor_axis_internal_ellipse",
    )?;
    write(
        w,
        u64::from(g.semimajor_axis_external_ellipse),
        ELLIPSE_AXIS_BITS,
        "semimajor_axis_external_ellipse",
    )?;
    write(
        w,
        u64::from(g.semiminor_axis_external_ellipse),
        ELLIPSE_AXIS_BITS,
        "semiminor_axis_external_ellipse",
    )?;
    write(
        w,
        u64::from(g.overlap_process_option.code()),
        OVERLAP_PROCESS_OPTION_BITS,
        "overlap_process_option",
    )
}

impl Hdr10PlusMetadata {
    fn payload_bits(&self) -> usize {
        let mut bits = NUM_WINDOWS_BITS as usize
            + self.additional_windows.len() * REGION_BITS
            + TARGETED_MAX_LUMINANCE_BITS as usize
            + FLAG_BITS as usize
            + lut_bits(&self.targeted_system_display_actual_peak_luminance)
            + FLAG_BITS as usize
            + lut_bits(&self.mastering_display_actual_peak_luminance);
        for c in self.windows() {
            bits += color_volume_bits(c) + transform_tail_bits(c);
        }
        bits
    }

    fn windows(&self) -> impl Iterator<Item = &ColorVolumeTransform> {
        core::iter::once(&self.primary_window)
            .chain(self.additional_windows.iter().map(|w| &w.color_volume))
    }

    /// Number of processing windows (1..=3).
    pub fn num_windows(&self) -> usize {
        1 + self.additional_windows.len()
    }

    fn padding_bits(&self) -> u32 {
        (self.payload_bits().next_multiple_of(8) - self.payload_bits()) as u32
    }

    /// Whether `payload` (a `user_data_registered_itu_t_t35` payload, starting at
    /// `itu_t_t35_country_code`) begins with the HDR10+ T.35 header.
    pub fn has_t35_header(payload: &[u8]) -> bool {
        payload.len() >= T35_HEADER_LEN
            && payload[..T35_HEADER_LEN - 1] == T35_HEADER_V1[..T35_HEADER_LEN - 1]
            && payload[T35_HEADER_LEN - 1] <= APPLICATION_VERSION_MAX
    }

    /// The `user_data_registered_itu_t_t35` payloads (unescaped) of an HEVC
    /// prefix SEI NAL unit (2-byte NAL header included) that carry the HDR10+
    /// T.35 header.
    pub fn hevc_sei_nal_payloads(nal: &[u8]) -> Vec<Vec<u8>> {
        if nal.len() <= HEVC_NAL_HEADER_LEN
            || crate::nal::nal_unit_type(crate::nal::NalCodec::Hevc, nal)
                != Some(crate::hdr::HEVC_PREFIX_SEI_NUT)
        {
            return Vec::new();
        }
        crate::nal::sei_messages(&nal[HEVC_NAL_HEADER_LEN..])
            .into_iter()
            .filter(|(t, p)| {
                *t == SEI_PAYLOAD_USER_DATA_REGISTERED_ITU_T_T35 && Self::has_t35_header(p)
            })
            .map(|(_, p)| p)
            .collect()
    }

    /// Parse the HDR10+ message of an HEVC prefix SEI NAL unit; `None` when the
    /// NAL unit carries no HDR10+ T.35 header.
    pub fn from_hevc_sei_nal(nal: &[u8]) -> Option<Result<Self>> {
        Self::hevc_sei_nal_payloads(nal)
            .into_iter()
            .next()
            .map(|p| Self::parse(&p))
    }

    /// Whether an HEVC prefix SEI NAL unit carries the HDR10+ T.35 header.
    pub fn hevc_sei_nal_carries_hdr10plus(nal: &[u8]) -> bool {
        !Self::hevc_sei_nal_payloads(nal).is_empty()
    }

    /// Whether an Annex B access unit holds an HDR10+ prefix SEI NAL unit.
    pub fn annexb_access_unit_carries_hdr10plus(access_unit: &[u8]) -> bool {
        crate::annexb::iter_annexb_nals(access_unit).any(Self::hevc_sei_nal_carries_hdr10plus)
    }

    /// Whether a 4-byte length-prefixed (ISO/IEC 14496-15) sample holds an
    /// HDR10+ prefix SEI NAL unit.
    pub fn sample_carries_hdr10plus(sample: &[u8]) -> bool {
        let mut at = 0usize;
        while at + NAL_LENGTH_PREFIX_LEN <= sample.len() {
            let mut len = [0u8; NAL_LENGTH_PREFIX_LEN];
            len.copy_from_slice(&sample[at..at + NAL_LENGTH_PREFIX_LEN]);
            let end = at + NAL_LENGTH_PREFIX_LEN + u32::from_be_bytes(len) as usize;
            let Some(nal) = sample.get(at + NAL_LENGTH_PREFIX_LEN..end) else {
                return false;
            };
            if Self::hevc_sei_nal_carries_hdr10plus(nal) {
                return true;
            }
            at = end;
        }
        false
    }

    /// The HEVC prefix SEI NAL unit (2-byte header, one `payloadType` 4
    /// message, `rbsp_trailing_bits`, emulation prevention applied) carrying
    /// this metadata.
    pub fn to_hevc_prefix_sei_nal(&self) -> Result<Vec<u8>> {
        let payload = self.try_to_bytes()?;
        let mut rbsp = Vec::with_capacity(payload.len() + 4);
        rbsp.push(SEI_PAYLOAD_USER_DATA_REGISTERED_ITU_T_T35 as u8);
        let mut remaining = payload.len();
        while remaining >= usize::from(SEI_SIZE_ESCAPE_BYTE) {
            rbsp.push(SEI_SIZE_ESCAPE_BYTE);
            remaining -= usize::from(SEI_SIZE_ESCAPE_BYTE);
        }
        rbsp.push(remaining as u8);
        rbsp.extend_from_slice(&payload);
        rbsp.push(SEI_RBSP_TRAILING_BYTE);
        let mut nal = Vec::with_capacity(rbsp.len() + HEVC_NAL_HEADER_LEN + 2);
        nal.push(crate::hdr::HEVC_PREFIX_SEI_NUT << HEVC_NAL_TYPE_SHIFT);
        nal.push(HEVC_PREFIX_SEI_LAYER_ZERO_TID_PLUS1);
        let mut zeros = 0usize;
        for b in rbsp {
            if zeros >= EMULATION_ZERO_RUN && b <= EMULATION_PREVENTION_BYTE {
                nal.push(EMULATION_PREVENTION_BYTE);
                zeros = 0;
            }
            zeros = if b == 0 { zeros + 1 } else { 0 };
            nal.push(b);
        }
        Ok(nal)
    }
}

impl<'a> Parse<'a> for Hdr10PlusMetadata {
    type Error = Error;

    fn parse(bytes: &'a [u8]) -> Result<Self> {
        if bytes.len() < T35_HEADER_LEN {
            return Err(Error::BufferTooShort {
                need: T35_HEADER_LEN,
                have: bytes.len(),
                what: "HDR10+ T.35 header",
            });
        }
        if bytes[..T35_HEADER_LEN - 2] != T35_HEADER_V1[..T35_HEADER_LEN - 2] {
            return Err(Error::InvalidValue {
                field: "itu_t_t35 header",
                value: u64::from(bytes[0]),
                reason: "not the HDR10+ country/provider/oriented code",
            });
        }
        if bytes[T35_HEADER_LEN - 2] != APPLICATION_IDENTIFIER {
            return Err(Error::InvalidValue {
                field: "application_identifier",
                value: u64::from(bytes[T35_HEADER_LEN - 2]),
                reason: "ST 2094-40 clause 5 requires 4",
            });
        }
        let application_version = bytes[T35_HEADER_LEN - 1];
        if application_version > APPLICATION_VERSION_MAX {
            return Err(Error::InvalidValue {
                field: "application_version",
                value: u64::from(application_version),
                reason: "ST 2094-40 clause 5 allows 0 or 1",
            });
        }

        let mut r = BitReader::new(&bytes[T35_HEADER_LEN..]);
        let num_windows = read(&mut r, NUM_WINDOWS_BITS, "num_windows")? as usize;
        if !(MIN_WINDOWS..=MAX_WINDOWS).contains(&num_windows) {
            return Err(Error::InvalidValue {
                field: "num_windows",
                value: num_windows as u64,
                reason: "ST 2094-40 clause 9.2 allows 1 to 3 windows",
            });
        }
        let mut regions = Vec::with_capacity(num_windows - 1);
        for _ in 1..num_windows {
            regions.push(read_region(&mut r)?);
        }
        let targeted_system_display_maximum_luminance = read(
            &mut r,
            TARGETED_MAX_LUMINANCE_BITS,
            "targeted_system_display_maximum_luminance",
        )? as u32;
        let targeted_system_display_actual_peak_luminance = if read(
            &mut r,
            FLAG_BITS,
            "targeted_system_display_actual_peak_luminance_flag",
        )? != 0
        {
            Some(read_lut(
                &mut r,
                "targeted_system_display_actual_peak_luminance",
            )?)
        } else {
            None
        };
        let mut transforms = Vec::with_capacity(num_windows);
        for _ in 0..num_windows {
            transforms.push(read_color_volume(&mut r)?);
        }
        let mastering_display_actual_peak_luminance = if read(
            &mut r,
            FLAG_BITS,
            "mastering_display_actual_peak_luminance_flag",
        )? != 0
        {
            Some(read_lut(&mut r, "mastering_display_actual_peak_luminance")?)
        } else {
            None
        };
        for t in &mut transforms {
            read_transform_tail(&mut r, t)?;
        }

        let pad = r.bits_remaining();
        if pad >= BITS_PER_BYTE as usize {
            return Err(Error::InvalidValue {
                field: "HDR10+ payload",
                value: (pad / BITS_PER_BYTE as usize) as u64,
                reason: "trailing bytes after the defined syntax",
            });
        }
        let alignment_padding = read(&mut r, pad as u32, "alignment padding")? as u8;

        let mut transforms = transforms.into_iter();
        let primary_window = transforms.next().expect("num_windows >= 1");
        let additional_windows = regions
            .into_iter()
            .zip(transforms)
            .map(|(region, color_volume)| AdditionalWindow {
                region,
                color_volume,
            })
            .collect();
        Ok(Self {
            application_version,
            targeted_system_display_maximum_luminance,
            targeted_system_display_actual_peak_luminance,
            primary_window,
            additional_windows,
            mastering_display_actual_peak_luminance,
            alignment_padding,
        })
    }
}

impl Serialize for Hdr10PlusMetadata {
    type Error = Error;

    fn serialized_len(&self) -> usize {
        T35_HEADER_LEN + self.payload_bits().div_ceil(BITS_PER_BYTE as usize)
    }

    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        let need = self.serialized_len();
        if buf.len() < need {
            return Err(Error::OutputBufferTooSmall {
                need,
                have: buf.len(),
            });
        }
        if self.application_version > APPLICATION_VERSION_MAX {
            return Err(Error::InvalidValue {
                field: "application_version",
                value: u64::from(self.application_version),
                reason: "ST 2094-40 clause 5 allows 0 or 1",
            });
        }
        if self.num_windows() > MAX_WINDOWS {
            return Err(Error::InvalidValue {
                field: "num_windows",
                value: self.num_windows() as u64,
                reason: "ST 2094-40 clause 9.2 allows 1 to 3 windows",
            });
        }
        buf[..T35_HEADER_LEN - 1].copy_from_slice(&T35_HEADER_V1[..T35_HEADER_LEN - 1]);
        buf[T35_HEADER_LEN - 1] = self.application_version;
        let body = &mut buf[T35_HEADER_LEN..need];
        body.fill(0);
        let mut w = BitWriter::new(body);
        write(
            &mut w,
            self.num_windows() as u64,
            NUM_WINDOWS_BITS,
            "num_windows",
        )?;
        for extra in &self.additional_windows {
            write_region(&mut w, &extra.region)?;
        }
        write(
            &mut w,
            u64::from(self.targeted_system_display_maximum_luminance),
            TARGETED_MAX_LUMINANCE_BITS,
            "targeted_system_display_maximum_luminance",
        )?;
        write_lut(
            &mut w,
            &self.targeted_system_display_actual_peak_luminance,
            "targeted_system_display_actual_peak_luminance",
        )?;
        for c in self.windows() {
            write_color_volume(&mut w, c)?;
        }
        write_lut(
            &mut w,
            &self.mastering_display_actual_peak_luminance,
            "mastering_display_actual_peak_luminance",
        )?;
        for c in self.windows() {
            write_transform_tail(&mut w, c)?;
        }
        let pad = self.padding_bits();
        if pad > 0 {
            write(
                &mut w,
                u64::from(self.alignment_padding),
                pad,
                "alignment padding",
            )?;
        } else if self.alignment_padding != 0 {
            return Err(Error::InvalidValue {
                field: "alignment padding",
                value: u64::from(self.alignment_padding),
                reason: "payload is byte aligned, there are no padding bits",
            });
        }
        Ok(need)
    }
}
