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
//! real stream (`private/fixtures/video/hdr10plus/`) violates Table 1 and a parser
//! that rejected it would drop signalling real encoders emit. Serialization is
//! symmetric: every parsed value, including the alignment padding bits, is
//! re-emitted unchanged.

use crate::annexb::iter_annexb_nals;
use crate::bitreader::{bit_error, write_bits_checked};
use crate::error::{Error, Result};
use crate::findings::{Findings, ParseFinding};
use crate::nal::{
    HEVC_NAL_HEADER_LEN, HEVC_PREFIX_SEI_NUT, NalCodec, hevc_prefix_sei_nal, nal_unit_type,
    sei_messages,
};
use alloc::vec::Vec;
use broadcast_core::bits::{BitReader, BitWriter};
use broadcast_core::error::InvalidValue;
use broadcast_core::len::{need_in, need_out};
use broadcast_core::{Parse, Serialize};

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
    (T35_PROVIDER_CODE >> u8::BITS) as u8,
    T35_PROVIDER_CODE as u8,
    (T35_PROVIDER_ORIENTED_CODE >> u8::BITS) as u8,
    T35_PROVIDER_ORIENTED_CODE as u8,
    APPLICATION_IDENTIFIER,
    APPLICATION_VERSION_MAX,
];

/// Bytes the T.35 header spends on `application_identifier` and
/// `application_version`, after the country/provider/oriented prefix
/// (SMPTE ST 2094-40 §5).
const T35_APPLICATION_SUFFIX_LEN: usize = 2;
/// Length of the country/provider/oriented prefix of the T.35 header.
const T35_PREFIX_LEN: usize = T35_HEADER_LEN - T35_APPLICATION_SUFFIX_LEN;
/// Offset of `application_identifier` in the T.35 header.
const APPLICATION_IDENTIFIER_OFFSET: usize = T35_PREFIX_LEN;
/// Offset of `application_version` in the T.35 header.
const APPLICATION_VERSION_OFFSET: usize = T35_APPLICATION_SUFFIX_LEN + T35_PREFIX_LEN - 1;
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
/// A peak-luminance table carries one extent per dimension (rows, columns).
const LUT_DIMENSION_COUNT: usize = 2;
const LUT_MAX_DIMENSION: u8 = 25;
const LUT_VALUE_BITS: u32 = 4;
const MAXSCL_BITS: u32 = 17;
const MAXSCL_COMPONENTS: usize = 3;
const AVERAGE_MAXRGB_BITS: u32 = 17;
const DISTRIBUTION_COUNT_BITS: u32 = 4;
const DISTRIBUTION_PERCENTAGE_BITS: u32 = 7;
const DISTRIBUTION_PERCENTILE_BITS: u32 = 17;
const FRACTION_BRIGHT_PIXELS_BITS: u32 = 10;
/// Knee points bounding the bezier curve (SMPTE ST 2094-40 §9.5).
const KNEE_POINT_COUNT: usize = 2;
const KNEE_POINT_BITS: u32 = 12;
const BEZIER_ANCHOR_COUNT_BITS: u32 = 4;
const BEZIER_ANCHOR_BITS: u32 = 10;
const COLOR_SATURATION_WEIGHT_BITS: u32 = 6;

fn read(r: &mut BitReader<'_>, bits: u32, what: &'static str) -> Result<u64> {
    r.read_bits(bits).map_err(|e| bit_error(what, e))
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
broadcast_core::impl_spec_display!(OverlapProcessOption);

/// The extra pixel selector of a window with `WindowNumber` > 0: the bounding
/// rectangle plus the concentric elliptical selector (ST 2094-40 clause 6;
/// window rectangle from ST 2094-1).
///
/// `#[non_exhaustive]`: build it with [`WindowRegion::builder`].
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct WindowRegion {
    upper_left_corner_x: u16,
    upper_left_corner_y: u16,
    lower_right_corner_x: u16,
    lower_right_corner_y: u16,
    center_of_ellipse_x: u16,
    center_of_ellipse_y: u16,
    rotation_angle: u8,
    semimajor_axis_internal_ellipse: u16,
    semimajor_axis_external_ellipse: u16,
    semiminor_axis_external_ellipse: u16,
    overlap_process_option: OverlapProcessOption,
}

impl WindowRegion {
    /// A builder with every field zero and `overlap_process_option` the
    /// weighted average (clause 6.7.2).
    #[must_use]
    pub fn builder() -> WindowRegionBuilder {
        WindowRegionBuilder::default()
    }

    /// `window_upper_left_corner_x`.
    #[must_use]
    pub fn upper_left_corner_x(&self) -> u16 {
        self.upper_left_corner_x
    }

    /// `window_upper_left_corner_y`.
    #[must_use]
    pub fn upper_left_corner_y(&self) -> u16 {
        self.upper_left_corner_y
    }

    /// `window_lower_right_corner_x`.
    #[must_use]
    pub fn lower_right_corner_x(&self) -> u16 {
        self.lower_right_corner_x
    }

    /// `window_lower_right_corner_y`.
    #[must_use]
    pub fn lower_right_corner_y(&self) -> u16 {
        self.lower_right_corner_y
    }

    /// `center_of_ellipse_x` (clause 6.2).
    #[must_use]
    pub fn center_of_ellipse_x(&self) -> u16 {
        self.center_of_ellipse_x
    }

    /// `center_of_ellipse_y` (clause 6.2).
    #[must_use]
    pub fn center_of_ellipse_y(&self) -> u16 {
        self.center_of_ellipse_y
    }

    /// `rotation_angle`, degrees clockwise, 0..=180 (clause 6.3).
    #[must_use]
    pub fn rotation_angle(&self) -> u8 {
        self.rotation_angle
    }

    /// `semimajor_axis_internal_ellipse`, pixels (clause 6.4).
    #[must_use]
    pub fn semimajor_axis_internal_ellipse(&self) -> u16 {
        self.semimajor_axis_internal_ellipse
    }

    /// `semimajor_axis_external_ellipse`, pixels (clause 6.5).
    #[must_use]
    pub fn semimajor_axis_external_ellipse(&self) -> u16 {
        self.semimajor_axis_external_ellipse
    }

    /// `semiminor_axis_external_ellipse`, pixels (clause 6.6).
    #[must_use]
    pub fn semiminor_axis_external_ellipse(&self) -> u16 {
        self.semiminor_axis_external_ellipse
    }

    /// `overlap_process_option` (clause 6.7).
    #[must_use]
    pub fn overlap_process_option(&self) -> OverlapProcessOption {
        self.overlap_process_option
    }

    /// Replace `overlap_process_option` (clause 6.7).
    #[must_use]
    pub fn with_overlap_process_option(mut self, option: OverlapProcessOption) -> Self {
        self.overlap_process_option = option;
        self
    }
}

/// Builder for [`WindowRegion`] (ST 2094-40 clause 6).
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct WindowRegionBuilder {
    upper_left_corner_x: u16,
    upper_left_corner_y: u16,
    lower_right_corner_x: u16,
    lower_right_corner_y: u16,
    center_of_ellipse_x: u16,
    center_of_ellipse_y: u16,
    rotation_angle: u8,
    semimajor_axis_internal_ellipse: u16,
    semimajor_axis_external_ellipse: u16,
    semiminor_axis_external_ellipse: u16,
    overlap_process_option: OverlapProcessOption,
}

impl WindowRegionBuilder {
    /// `window_upper_left_corner_x`.
    #[must_use]
    pub fn upper_left_corner_x(mut self, value: u16) -> Self {
        self.upper_left_corner_x = value;
        self
    }

    /// `window_upper_left_corner_y`.
    #[must_use]
    pub fn upper_left_corner_y(mut self, value: u16) -> Self {
        self.upper_left_corner_y = value;
        self
    }

    /// `window_lower_right_corner_x`.
    #[must_use]
    pub fn lower_right_corner_x(mut self, value: u16) -> Self {
        self.lower_right_corner_x = value;
        self
    }

    /// `window_lower_right_corner_y`.
    #[must_use]
    pub fn lower_right_corner_y(mut self, value: u16) -> Self {
        self.lower_right_corner_y = value;
        self
    }

    /// `center_of_ellipse_x`.
    #[must_use]
    pub fn center_of_ellipse_x(mut self, value: u16) -> Self {
        self.center_of_ellipse_x = value;
        self
    }

    /// `center_of_ellipse_y`.
    #[must_use]
    pub fn center_of_ellipse_y(mut self, value: u16) -> Self {
        self.center_of_ellipse_y = value;
        self
    }

    /// `rotation_angle`.
    #[must_use]
    pub fn rotation_angle(mut self, value: u8) -> Self {
        self.rotation_angle = value;
        self
    }

    /// `semimajor_axis_internal_ellipse`.
    #[must_use]
    pub fn semimajor_axis_internal_ellipse(mut self, value: u16) -> Self {
        self.semimajor_axis_internal_ellipse = value;
        self
    }

    /// `semimajor_axis_external_ellipse`.
    #[must_use]
    pub fn semimajor_axis_external_ellipse(mut self, value: u16) -> Self {
        self.semimajor_axis_external_ellipse = value;
        self
    }

    /// `semiminor_axis_external_ellipse`.
    #[must_use]
    pub fn semiminor_axis_external_ellipse(mut self, value: u16) -> Self {
        self.semiminor_axis_external_ellipse = value;
        self
    }

    /// `overlap_process_option`.
    #[must_use]
    pub fn overlap_process_option(mut self, value: OverlapProcessOption) -> Self {
        self.overlap_process_option = value;
        self
    }

    /// The window selector these setters describe.
    #[must_use]
    pub fn build(self) -> WindowRegion {
        WindowRegion {
            upper_left_corner_x: self.upper_left_corner_x,
            upper_left_corner_y: self.upper_left_corner_y,
            lower_right_corner_x: self.lower_right_corner_x,
            lower_right_corner_y: self.lower_right_corner_y,
            center_of_ellipse_x: self.center_of_ellipse_x,
            center_of_ellipse_y: self.center_of_ellipse_y,
            rotation_angle: self.rotation_angle,
            semimajor_axis_internal_ellipse: self.semimajor_axis_internal_ellipse,
            semimajor_axis_external_ellipse: self.semimajor_axis_external_ellipse,
            semiminor_axis_external_ellipse: self.semiminor_axis_external_ellipse,
            overlap_process_option: self.overlap_process_option,
        }
    }
}

impl Default for WindowRegionBuilder {
    fn default() -> Self {
        Self {
            upper_left_corner_x: 0,
            upper_left_corner_y: 0,
            lower_right_corner_x: 0,
            lower_right_corner_y: 0,
            center_of_ellipse_x: 0,
            center_of_ellipse_y: 0,
            rotation_angle: 0,
            semimajor_axis_internal_ellipse: 0,
            semimajor_axis_external_ellipse: 0,
            semiminor_axis_external_ellipse: 0,
            overlap_process_option: OverlapProcessOption::WeightedAverage,
        }
    }
}

/// A two-input sampled actual-peak-luminance table (ST 2094-40 clauses 7.2 and
/// 8.2): `rows` x `columns` outputs, each a multiple of 1/15.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct PeakLuminanceTable {
    rows: u8,
    columns: u8,
    values: Vec<u8>,
}

impl PeakLuminanceTable {
    /// A `rows` x `columns` table holding `values` in row-major order, each in
    /// units of 1/15.
    #[must_use]
    pub fn new(rows: u8, columns: u8, values: Vec<u8>) -> Self {
        Self {
            rows,
            columns,
            values,
        }
    }

    /// `num_rows_*_actual_peak_luminance`, at most 25.
    #[must_use]
    pub fn rows(&self) -> u8 {
        self.rows
    }

    /// `num_cols_*_actual_peak_luminance`, at most 25.
    #[must_use]
    pub fn columns(&self) -> u8 {
        self.columns
    }

    /// `actual_peak_luminance[i][j]` in row-major order, in units of 1/15
    /// (0..=15); `rows * columns` entries.
    #[must_use]
    pub fn values(&self) -> &[u8] {
        &self.values
    }
}

/// One `DistributionMaxRGB` point (ST 2094-40 clause 8.5).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub struct DistributionPoint {
    percentage: u8,
    percentile: u32,
}

impl DistributionPoint {
    /// One `DistributionMaxRGB` point.
    #[must_use]
    pub fn new(percentage: u8, percentile: u32) -> Self {
        Self {
            percentage,
            percentile,
        }
    }

    /// `distribution_maxrgb_percentages`, 0..=100.
    #[must_use]
    pub fn percentage(&self) -> u8 {
        self.percentage
    }

    /// `distribution_maxrgb_percentiles`, in units of 0.00001.
    #[must_use]
    pub fn percentile(&self) -> u32 {
        self.percentile
    }
}

/// `KneePoint` and `BezierCurveAnchors` (ST 2094-40 clauses 8.7.2-8.7.3).
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct BezierToneMapping {
    knee_point_x: u16,
    knee_point_y: u16,
    anchors: Vec<u16>,
}

impl BezierToneMapping {
    /// The knee point and the curve anchors of a tone-mapping payload.
    #[must_use]
    pub fn new(knee_point_x: u16, knee_point_y: u16, anchors: Vec<u16>) -> Self {
        Self {
            knee_point_x,
            knee_point_y,
            anchors,
        }
    }

    /// `knee_point_x`, in units of 1/4095.
    #[must_use]
    pub fn knee_point_x(&self) -> u16 {
        self.knee_point_x
    }

    /// `knee_point_y`, in units of 1/4095.
    #[must_use]
    pub fn knee_point_y(&self) -> u16 {
        self.knee_point_y
    }

    /// `bezier_curve_anchors`, each in units of 1/1023 (at most 15).
    #[must_use]
    pub fn anchors(&self) -> &[u16] {
        &self.anchors
    }
}

/// The `ColorVolumeTransform` items of one processing window (ST 2094-40
/// clauses 8.3-8.8).
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct ColorVolumeTransform {
    max_scl: [u32; MAXSCL_COMPONENTS],
    average_max_rgb: u32,
    distribution_max_rgb: Vec<DistributionPoint>,
    fraction_bright_pixels: u16,
    tone_mapping: Option<BezierToneMapping>,
    color_saturation_weight: Option<u8>,
}

impl ColorVolumeTransform {
    /// A transform with no tone mapping and no colour saturation weight.
    #[must_use]
    pub fn new(
        max_scl: [u32; MAXSCL_COMPONENTS],
        average_max_rgb: u32,
        distribution_max_rgb: Vec<DistributionPoint>,
        fraction_bright_pixels: u16,
    ) -> Self {
        Self {
            max_scl,
            average_max_rgb,
            distribution_max_rgb,
            fraction_bright_pixels,
            tone_mapping: None,
            color_saturation_weight: None,
        }
    }

    /// `maxscl[0..3]` (R, G, B), in units of 0.00001 (clause 8.3).
    #[must_use]
    pub fn max_scl(&self) -> [u32; MAXSCL_COMPONENTS] {
        self.max_scl
    }

    /// `average_maxrgb`, in units of 0.00001 (clause 8.4).
    #[must_use]
    pub fn average_max_rgb(&self) -> u32 {
        self.average_max_rgb
    }

    /// `DistributionMaxRGB` (clause 8.5), at most 15 points.
    #[must_use]
    pub fn distribution_max_rgb(&self) -> &[DistributionPoint] {
        &self.distribution_max_rgb
    }

    /// `fraction_bright_pixels`, in units of 0.001 (clause 8.6).
    #[must_use]
    pub fn fraction_bright_pixels(&self) -> u16 {
        self.fraction_bright_pixels
    }

    /// `tone_mapping_flag` and its payload (clause 8.7); `None` when the flag
    /// is clear.
    #[must_use]
    pub fn tone_mapping(&self) -> Option<&BezierToneMapping> {
        self.tone_mapping.as_ref()
    }

    /// `color_saturation_weight`, in units of 1/8 (clause 8.8); `None` when
    /// `color_saturation_mapping_flag` is clear.
    #[must_use]
    pub fn color_saturation_weight(&self) -> Option<u8> {
        self.color_saturation_weight
    }

    /// Set the tone-mapping payload (clause 8.7).
    #[must_use]
    pub fn with_tone_mapping(mut self, tone_mapping: BezierToneMapping) -> Self {
        self.tone_mapping = Some(tone_mapping);
        self
    }

    /// Set `color_saturation_weight` (clause 8.8).
    #[must_use]
    pub fn with_color_saturation_weight(mut self, weight: u8) -> Self {
        self.color_saturation_weight = Some(weight);
        self
    }

    /// Replace the `maxscl` triple (clause 8.3).
    #[must_use]
    pub fn with_max_scl(mut self, max_scl: [u32; MAXSCL_COMPONENTS]) -> Self {
        self.max_scl = max_scl;
        self
    }

    /// Replace the `DistributionMaxRGB` points (clause 8.5).
    #[must_use]
    pub fn with_distribution_max_rgb(mut self, points: Vec<DistributionPoint>) -> Self {
        self.distribution_max_rgb = points;
        self
    }
}

/// A processing window with `WindowNumber` > 0.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct AdditionalWindow {
    region: WindowRegion,
    color_volume: ColorVolumeTransform,
}

impl AdditionalWindow {
    /// A window selector and its colour volume transform items.
    #[must_use]
    pub fn new(region: WindowRegion, color_volume: ColorVolumeTransform) -> Self {
        Self {
            region,
            color_volume,
        }
    }

    /// Rectangle and elliptical selector.
    #[must_use]
    pub fn region(&self) -> &WindowRegion {
        &self.region
    }

    /// The window's colour volume transform items.
    #[must_use]
    pub fn color_volume(&self) -> &ColorVolumeTransform {
        &self.color_volume
    }
}

/// HDR10+ dynamic metadata of one access unit: the T.35 payload of ST 2094-40
/// Application #4.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct Hdr10PlusMetadata {
    application_version: u8,
    targeted_system_display_maximum_luminance: u32,
    targeted_system_display_actual_peak_luminance: Option<PeakLuminanceTable>,
    primary_window: ColorVolumeTransform,
    additional_windows: Vec<AdditionalWindow>,
    mastering_display_actual_peak_luminance: Option<PeakLuminanceTable>,
    alignment_padding: u8,
}

impl Hdr10PlusMetadata {
    /// A single-window message with no peak-luminance tables and no padding.
    #[must_use]
    pub fn new(
        application_version: u8,
        targeted_system_display_maximum_luminance: u32,
        primary_window: ColorVolumeTransform,
    ) -> Self {
        Self {
            application_version,
            targeted_system_display_maximum_luminance,
            targeted_system_display_actual_peak_luminance: None,
            primary_window,
            additional_windows: Vec::new(),
            mastering_display_actual_peak_luminance: None,
            alignment_padding: 0,
        }
    }

    /// `application_version` (ST 2094-40 clause 5: 0 or 1).
    #[must_use]
    pub fn application_version(&self) -> u8 {
        self.application_version
    }

    /// `targeted_system_display_maximum_luminance`, the raw 27-bit field.
    #[must_use]
    pub fn targeted_system_display_maximum_luminance(&self) -> u32 {
        self.targeted_system_display_maximum_luminance
    }

    /// `TargetedSystemDisplayActualPeakLuminance` (clause 7.2).
    #[must_use]
    pub fn targeted_system_display_actual_peak_luminance(&self) -> Option<&PeakLuminanceTable> {
        self.targeted_system_display_actual_peak_luminance.as_ref()
    }

    /// Window 0, which covers the whole image.
    #[must_use]
    pub fn primary_window(&self) -> &ColorVolumeTransform {
        &self.primary_window
    }

    /// Windows 1 and 2 (at most two).
    #[must_use]
    pub fn additional_windows(&self) -> &[AdditionalWindow] {
        &self.additional_windows
    }

    /// `MasteringDisplayActualPeakLuminance` (clause 8.2).
    #[must_use]
    pub fn mastering_display_actual_peak_luminance(&self) -> Option<&PeakLuminanceTable> {
        self.mastering_display_actual_peak_luminance.as_ref()
    }

    /// Value of the unused low bits of the last payload byte (zero in every
    /// stream seen), kept so a re-serialization is byte-identical.
    #[must_use]
    pub fn alignment_padding(&self) -> u8 {
        self.alignment_padding
    }

    /// Set `TargetedSystemDisplayActualPeakLuminance` (clause 7.2).
    #[must_use]
    pub fn with_targeted_system_display_actual_peak_luminance(
        mut self,
        table: PeakLuminanceTable,
    ) -> Self {
        self.targeted_system_display_actual_peak_luminance = Some(table);
        self
    }

    /// Set `MasteringDisplayActualPeakLuminance` (clause 8.2).
    #[must_use]
    pub fn with_mastering_display_actual_peak_luminance(
        mut self,
        table: PeakLuminanceTable,
    ) -> Self {
        self.mastering_display_actual_peak_luminance = Some(table);
        self
    }

    /// Replace the extra processing windows (at most two).
    #[must_use]
    pub fn with_additional_windows(mut self, windows: Vec<AdditionalWindow>) -> Self {
        self.additional_windows = windows;
        self
    }

    /// Replace window 0.
    #[must_use]
    pub fn with_primary_window(mut self, primary_window: ColorVolumeTransform) -> Self {
        self.primary_window = primary_window;
        self
    }

    /// Set the value of the payload's trailing padding bits.
    #[must_use]
    pub fn with_alignment_padding(mut self, padding: u8) -> Self {
        self.alignment_padding = padding;
        self
    }
}

fn read_lut(r: &mut BitReader<'_>, what: &'static str) -> Result<PeakLuminanceTable> {
    let rows = read(r, LUT_DIMENSION_BITS, what)? as u8;
    let columns = read(r, LUT_DIMENSION_BITS, what)? as u8;
    if rows > LUT_MAX_DIMENSION || columns > LUT_MAX_DIMENSION {
        return Err(Error::InvalidValue(InvalidValue {
            field: what,
            value: u64::from(rows.max(columns)),
            reason: "more than 25 rows or columns (ST 2094-40 clause 7.2)",
        }));
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
        LUT_DIMENSION_COUNT * LUT_DIMENSION_BITS as usize + l.values.len() * LUT_VALUE_BITS as usize
    })
}

fn write_lut(
    w: &mut BitWriter<'_>,
    lut: &Option<PeakLuminanceTable>,
    what: &'static str,
) -> Result<()> {
    write_bits_checked(w, u64::from(lut.is_some()), FLAG_BITS, what)?;
    let Some(l) = lut else { return Ok(()) };
    if l.rows > LUT_MAX_DIMENSION
        || l.columns > LUT_MAX_DIMENSION
        || l.values.len() != usize::from(l.rows) * usize::from(l.columns)
    {
        return Err(Error::InvalidValue(InvalidValue {
            field: what,
            value: l.values.len() as u64,
            reason: "table size disagrees with rows x columns or exceeds 25x25",
        }));
    }
    write_bits_checked(w, u64::from(l.rows), LUT_DIMENSION_BITS, what)?;
    write_bits_checked(w, u64::from(l.columns), LUT_DIMENSION_BITS, what)?;
    for v in &l.values {
        write_bits_checked(w, u64::from(*v), LUT_VALUE_BITS, what)?;
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
        write_bits_checked(w, u64::from(v), MAXSCL_BITS, "maxscl")?;
    }
    write_bits_checked(
        w,
        u64::from(c.average_max_rgb),
        AVERAGE_MAXRGB_BITS,
        "average_maxrgb",
    )?;
    write_bits_checked(
        w,
        c.distribution_max_rgb.len() as u64,
        DISTRIBUTION_COUNT_BITS,
        "num_distribution_maxrgb_percentiles",
    )?;
    for p in &c.distribution_max_rgb {
        write_bits_checked(
            w,
            u64::from(p.percentage),
            DISTRIBUTION_PERCENTAGE_BITS,
            "distribution_maxrgb_percentages",
        )?;
        write_bits_checked(
            w,
            u64::from(p.percentile),
            DISTRIBUTION_PERCENTILE_BITS,
            "distribution_maxrgb_percentiles",
        )?;
    }
    write_bits_checked(
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
            + KNEE_POINT_COUNT * KNEE_POINT_BITS as usize
            + BEZIER_ANCHOR_COUNT_BITS as usize
            + t.anchors.len() * BEZIER_ANCHOR_BITS as usize
    });
    let saturation = FLAG_BITS as usize
        + c.color_saturation_weight
            .map_or(0, |_| COLOR_SATURATION_WEIGHT_BITS as usize);
    tone + saturation
}

fn write_transform_tail(w: &mut BitWriter<'_>, c: &ColorVolumeTransform) -> Result<()> {
    write_bits_checked(
        w,
        u64::from(c.tone_mapping.is_some()),
        FLAG_BITS,
        "tone_mapping_flag",
    )?;
    if let Some(t) = &c.tone_mapping {
        write_bits_checked(
            w,
            u64::from(t.knee_point_x),
            KNEE_POINT_BITS,
            "knee_point_x",
        )?;
        write_bits_checked(
            w,
            u64::from(t.knee_point_y),
            KNEE_POINT_BITS,
            "knee_point_y",
        )?;
        write_bits_checked(
            w,
            t.anchors.len() as u64,
            BEZIER_ANCHOR_COUNT_BITS,
            "num_bezier_curve_anchors",
        )?;
        for a in &t.anchors {
            write_bits_checked(w, u64::from(*a), BEZIER_ANCHOR_BITS, "bezier_curve_anchors")?;
        }
    }
    write_bits_checked(
        w,
        u64::from(c.color_saturation_weight.is_some()),
        FLAG_BITS,
        "color_saturation_mapping_flag",
    )?;
    if let Some(weight) = c.color_saturation_weight {
        write_bits_checked(
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
    write_bits_checked(
        w,
        u64::from(g.upper_left_corner_x),
        WINDOW_CORNER_BITS,
        "window_upper_left_corner_x",
    )?;
    write_bits_checked(
        w,
        u64::from(g.upper_left_corner_y),
        WINDOW_CORNER_BITS,
        "window_upper_left_corner_y",
    )?;
    write_bits_checked(
        w,
        u64::from(g.lower_right_corner_x),
        WINDOW_CORNER_BITS,
        "window_lower_right_corner_x",
    )?;
    write_bits_checked(
        w,
        u64::from(g.lower_right_corner_y),
        WINDOW_CORNER_BITS,
        "window_lower_right_corner_y",
    )?;
    write_bits_checked(
        w,
        u64::from(g.center_of_ellipse_x),
        ELLIPSE_CENTER_BITS,
        "center_of_ellipse_x",
    )?;
    write_bits_checked(
        w,
        u64::from(g.center_of_ellipse_y),
        ELLIPSE_CENTER_BITS,
        "center_of_ellipse_y",
    )?;
    write_bits_checked(
        w,
        u64::from(g.rotation_angle),
        ROTATION_ANGLE_BITS,
        "rotation_angle",
    )?;
    write_bits_checked(
        w,
        u64::from(g.semimajor_axis_internal_ellipse),
        ELLIPSE_AXIS_BITS,
        "semimajor_axis_internal_ellipse",
    )?;
    write_bits_checked(
        w,
        u64::from(g.semimajor_axis_external_ellipse),
        ELLIPSE_AXIS_BITS,
        "semimajor_axis_external_ellipse",
    )?;
    write_bits_checked(
        w,
        u64::from(g.semiminor_axis_external_ellipse),
        ELLIPSE_AXIS_BITS,
        "semiminor_axis_external_ellipse",
    )?;
    write_bits_checked(
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
        (self.payload_bits().next_multiple_of(u8::BITS as usize) - self.payload_bits()) as u32
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
    pub fn hevc_sei_nal_payloads(nal: &[u8], findings: &mut Findings) -> Vec<Vec<u8>> {
        if nal.len() <= HEVC_NAL_HEADER_LEN
            || nal_unit_type(NalCodec::Hevc, nal) != Some(HEVC_PREFIX_SEI_NUT)
        {
            return Vec::new();
        }
        sei_messages(&nal[HEVC_NAL_HEADER_LEN..], findings)
            .into_iter()
            .filter(|(t, p)| {
                *t == SEI_PAYLOAD_USER_DATA_REGISTERED_ITU_T_T35 && Self::has_t35_header(p)
            })
            .map(|(_, p)| p)
            .collect()
    }

    /// Parse the HDR10+ message of an HEVC prefix SEI NAL unit; `None` when the
    /// NAL unit carries no HDR10+ T.35 header. A message that does not parse is
    /// recorded in `findings` instead of being returned as an error.
    pub fn from_hevc_sei_nal(nal: &[u8], findings: &mut Findings) -> Option<Self> {
        let payload = Self::hevc_sei_nal_payloads(nal, findings)
            .into_iter()
            .next()?;
        match Self::parse(&payload) {
            Ok(metadata) => Some(metadata),
            Err(_) => {
                findings.push(ParseFinding::hdr_metadata(HEVC_NAL_HEADER_LEN));
                None
            }
        }
    }

    /// Whether an HEVC prefix SEI NAL unit carries the HDR10+ T.35 header.
    pub fn hevc_sei_nal_carries_hdr10plus(nal: &[u8]) -> bool {
        !Self::hevc_sei_nal_payloads(nal, &mut Findings::new()).is_empty()
    }

    /// Whether an Annex B access unit holds an HDR10+ prefix SEI NAL unit.
    pub fn annexb_access_unit_carries_hdr10plus(access_unit: &[u8]) -> bool {
        iter_annexb_nals(access_unit).any(Self::hevc_sei_nal_carries_hdr10plus)
    }

    /// Whether a 4-byte length-prefixed (ISO/IEC 14496-15:2017) sample holds an
    /// HDR10+ prefix SEI NAL unit.
    pub fn sample_carries_hdr10plus(sample: &[u8]) -> bool {
        let mut at = 0usize;
        while at + NAL_LENGTH_PREFIX_LEN <= sample.len() {
            let mut len = [0u8; NAL_LENGTH_PREFIX_LEN];
            len.copy_from_slice(&sample[at..at + NAL_LENGTH_PREFIX_LEN]);
            let Some(end) = usize::try_from(u32::from_be_bytes(len))
                .ok()
                .and_then(|n| n.checked_add(at + NAL_LENGTH_PREFIX_LEN))
            else {
                return false;
            };
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
        Ok(hevc_prefix_sei_nal(&[(
            SEI_PAYLOAD_USER_DATA_REGISTERED_ITU_T_T35,
            &payload,
        )]))
    }
}

impl<'a> Parse<'a> for Hdr10PlusMetadata {
    type Error = Error;

    fn parse(bytes: &'a [u8]) -> Result<Self> {
        need_in(bytes, T35_HEADER_LEN, "HDR10+ T.35 header")?;
        if bytes[..T35_PREFIX_LEN] != T35_HEADER_V1[..T35_PREFIX_LEN] {
            return Err(Error::InvalidValue(InvalidValue {
                field: "itu_t_t35 header",
                value: u64::from(bytes[0]),
                reason: "not the HDR10+ country/provider/oriented code",
            }));
        }
        if bytes[APPLICATION_IDENTIFIER_OFFSET] != APPLICATION_IDENTIFIER {
            return Err(Error::InvalidValue(InvalidValue {
                field: "application_identifier",
                value: u64::from(bytes[APPLICATION_IDENTIFIER_OFFSET]),
                reason: "ST 2094-40 clause 5 requires 4",
            }));
        }
        let application_version = bytes[APPLICATION_VERSION_OFFSET];
        if application_version > APPLICATION_VERSION_MAX {
            return Err(Error::InvalidValue(InvalidValue {
                field: "application_version",
                value: u64::from(application_version),
                reason: "ST 2094-40 clause 5 allows 0 or 1",
            }));
        }

        let mut r = BitReader::new(&bytes[T35_HEADER_LEN..]);
        let num_windows = read(&mut r, NUM_WINDOWS_BITS, "num_windows")? as usize;
        if !(MIN_WINDOWS..=MAX_WINDOWS).contains(&num_windows) {
            return Err(Error::InvalidValue(InvalidValue {
                field: "num_windows",
                value: num_windows as u64,
                reason: "ST 2094-40 clause 9.2 allows 1 to 3 windows",
            }));
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
        if pad >= u8::BITS as usize {
            return Err(Error::InvalidValue(InvalidValue {
                field: "HDR10+ payload",
                value: (pad / u8::BITS as usize) as u64,
                reason: "trailing bytes after the defined syntax",
            }));
        }
        let alignment_padding = read(&mut r, pad as u32, "alignment padding")? as u8;

        let mut transforms = transforms.into_iter();
        let Some(primary_window) = transforms.next() else {
            return Err(Error::InvalidValue(InvalidValue {
                field: "num_windows",
                value: 0,
                reason: "at least one window is required",
            }));
        };
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
        T35_HEADER_LEN + self.payload_bits().div_ceil(u8::BITS as usize)
    }

    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        let need = self.serialized_len();
        need_out(buf, need)?;
        if self.application_version > APPLICATION_VERSION_MAX {
            return Err(Error::InvalidValue(InvalidValue {
                field: "application_version",
                value: u64::from(self.application_version),
                reason: "ST 2094-40 clause 5 allows 0 or 1",
            }));
        }
        if self.num_windows() > MAX_WINDOWS {
            return Err(Error::InvalidValue(InvalidValue {
                field: "num_windows",
                value: self.num_windows() as u64,
                reason: "ST 2094-40 clause 9.2 allows 1 to 3 windows",
            }));
        }
        buf[..T35_HEADER_LEN - 1].copy_from_slice(&T35_HEADER_V1[..T35_HEADER_LEN - 1]);
        buf[T35_HEADER_LEN - 1] = self.application_version;
        let body = &mut buf[T35_HEADER_LEN..need];
        body.fill(0);
        let mut w = BitWriter::new(body);
        write_bits_checked(
            &mut w,
            self.num_windows() as u64,
            NUM_WINDOWS_BITS,
            "num_windows",
        )?;
        for extra in &self.additional_windows {
            write_region(&mut w, &extra.region)?;
        }
        write_bits_checked(
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
            write_bits_checked(
                &mut w,
                u64::from(self.alignment_padding),
                pad,
                "alignment padding",
            )?;
        } else if self.alignment_padding != 0 {
            return Err(Error::InvalidValue(InvalidValue {
                field: "alignment padding",
                value: u64::from(self.alignment_padding),
                reason: "payload is byte aligned, there are no padding bits",
            }));
        }
        Ok(need)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// H.265 Table 7-1: `SUFFIX_SEI_NUT`.
    const HEVC_SUFFIX_SEI_NUT: u8 = 40;

    #[test]
    fn has_t35_header_rejects_each_wrong_byte() {
        assert!(
            Hdr10PlusMetadata::has_t35_header(&T35_HEADER_V1),
            "the bare 7-byte header is a valid T.35 header"
        );
        let mut with_body = T35_HEADER_V1.to_vec();
        with_body.extend_from_slice(&[0x00, 0x00]);
        assert!(Hdr10PlusMetadata::has_t35_header(&with_body));

        assert!(!Hdr10PlusMetadata::has_t35_header(&T35_HEADER_V1[..6]));

        let mut wrong_country = T35_HEADER_V1;
        wrong_country[0] = 0xB4;
        assert!(!Hdr10PlusMetadata::has_t35_header(&wrong_country));

        let mut wrong_provider = T35_HEADER_V1;
        wrong_provider[2] = 0x3D;
        assert!(!Hdr10PlusMetadata::has_t35_header(&wrong_provider));

        let mut future_version = T35_HEADER_V1;
        future_version[T35_HEADER_LEN - 1] = APPLICATION_VERSION_MAX + 1;
        assert!(!Hdr10PlusMetadata::has_t35_header(&future_version));
    }

    #[test]
    fn hevc_sei_nal_payloads_keeps_only_prefixed_t35_messages() {
        let prefix =
            hevc_prefix_sei_nal(&[(SEI_PAYLOAD_USER_DATA_REGISTERED_ITU_T_T35, &T35_HEADER_V1)]);
        let payloads = Hdr10PlusMetadata::hevc_sei_nal_payloads(&prefix, &mut Findings::new());
        assert_eq!(payloads.len(), 1);
        assert_eq!(payloads[0], T35_HEADER_V1);

        let mut suffix = prefix.clone();
        suffix[0] = HEVC_SUFFIX_SEI_NUT << 1;
        assert!(
            Hdr10PlusMetadata::hevc_sei_nal_payloads(&suffix, &mut Findings::new()).is_empty(),
            "a suffix SEI NAL is not a prefix SEI"
        );

        let other_type = hevc_prefix_sei_nal(&[(6, &[0u8; 4])]);
        assert!(
            Hdr10PlusMetadata::hevc_sei_nal_payloads(&other_type, &mut Findings::new()).is_empty(),
            "a non-T.35 payloadType is not HDR10+"
        );

        let t35_without_header =
            hevc_prefix_sei_nal(&[(SEI_PAYLOAD_USER_DATA_REGISTERED_ITU_T_T35, &[0u8; 4])]);
        assert!(
            Hdr10PlusMetadata::hevc_sei_nal_payloads(&t35_without_header, &mut Findings::new())
                .is_empty(),
            "a T.35 message without the HDR10+ header is not HDR10+"
        );

        assert!(
            Hdr10PlusMetadata::hevc_sei_nal_payloads(
                &[HEVC_PREFIX_SEI_NUT << 1, 1],
                &mut Findings::new(),
            )
            .is_empty(),
            "a bare 2-byte NAL has no SEI body"
        );
    }
}
