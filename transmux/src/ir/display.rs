//! Display geometry of a video track: what a player needs to show each
//! picture at its correct shape.
//!
//! Phases 1a and 1b of `docs/display-geometry/DESIGN.md`: the **sample aspect
//! ratio** (SAR) signalled in the SPS VUI, the coded picture size, and the
//! **video signal type / colour description** (video format, full range and
//! the colour primaries / transfer characteristics / matrix coefficients),
//! derived once per track into [`TrackSpec::display`](super::TrackSpec::display).
//! Cropping, a change event and per-sample tags are later phases (every type
//! here is `#[non_exhaustive]`, so they land additively).
//!
//! # Spec citations
//!
//! - **H.264**: ITU-T H.264 (V15, 08/2024) §E.1.1 (`vui_parameters()` syntax),
//!   §E.2.1 Table E-1 (meaning of `aspect_ratio_idc`), Table E-2
//!   (`video_format`), Table E-3 (`colour_primaries`), Table E-4
//!   (`transfer_characteristics`) and Table E-5 (`matrix_coefficients`).
//! - **H.265**: ITU-T H.265 (V11, 01/2026) §E.2.1 (syntax) and §E.3.1
//!   Tables E.1 (same values as Table E-1), E.2, E.3, E.4 and E.5.
//!
//! Every table was re-verified row by row against the rendered PDF pages and
//! is transcribed in `docs/codec/vui-colour-tables.md` (see also
//! `docs/display-geometry/SPEC-NOTES.md`), not read from the markdown text,
//! whose Table E-1 rows are merged and whose Tables E-3..E-5 lost their
//! column structure.
//!
//! # Semantics kept exactly
//!
//! - `aspect_ratio_idc` absent (no VUI, or `aspect_ratio_info_present_flag`
//!   0) is inferred 0 (unspecified): [`DisplayGeometry::aspect_ratio`] is
//!   `None`.
//! - "When `aspect_ratio_idc` is equal to 0 or `sar_width` is equal to 0 or
//!   `sar_height` is equal to 0, the sample aspect ratio shall be considered
//!   unspecified": [`AspectRatioIdc::sample_aspect_ratio`] is `None`, never a
//!   `0:0` sentinel.
//! - H.265 §E.3.1: values 17..=254 are reserved and "decoders shall interpret"
//!   them "as equivalent to the value 0". H.264 marks the same range reserved
//!   without that sentence; this crate applies the H.265 rule to both.
//! - Two broadcasters code a 720x576 16:9 picture as `16:11` (idc 4, Table
//!   E-1 NOTE 1: horizontal overscan) and as `64:45` (Extended_SAR). Both are
//!   legitimate and are kept exactly as signalled, never normalised to each
//!   other.

use crate::error::Result;
use crate::ir::CodecConfig;
use core::num::NonZeroU16;

/// `aspect_ratio_idc` value that selects `sar_width` / `sar_height`
/// (H.264 Table E-1 `Extended_SAR`; H.265 Table E.1 `EXTENDED_SAR`).
pub const EXTENDED_SAR_IDC: u8 = 255;

/// First reserved `aspect_ratio_idc` (H.264 Table E-1 / H.265 Table E.1:
/// "17..254 Reserved").
const FIRST_RESERVED_IDC: u8 = 17;

/// H.264 Table E-1 / H.265 Table E.1 `aspect_ratio_idc`, the wire form kept
/// losslessly ([`idc`](Self::idc) returns the wire byte).
///
/// The SAR rows are named for their ratio (`Sar16x11` = `16:11`).
#[non_exhaustive]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AspectRatioIdc {
    /// 0: unspecified.
    Unspecified,
    /// 1: `1:1` ("square").
    Square,
    /// 2: `12:11`.
    Sar12x11,
    /// 3: `10:11`.
    Sar10x11,
    /// 4: `16:11`.
    Sar16x11,
    /// 5: `40:33`.
    Sar40x33,
    /// 6: `24:11`.
    Sar24x11,
    /// 7: `20:11`.
    Sar20x11,
    /// 8: `32:11`.
    Sar32x11,
    /// 9: `80:33`.
    Sar80x33,
    /// 10: `18:11`.
    Sar18x11,
    /// 11: `15:11`.
    Sar15x11,
    /// 12: `64:33`.
    Sar64x33,
    /// 13: `160:99`.
    Sar160x99,
    /// 14: `4:3`.
    Sar4x3,
    /// 15: `3:2`.
    Sar3x2,
    /// 16: `2:1`.
    Sar2x1,
    /// 17..=254: reserved (wire value kept).
    Reserved(u8),
    /// 255: `Extended_SAR`, with the explicit `sar_width` / `sar_height`
    /// exactly as signalled (either may be 0, which means unspecified).
    ExtendedSar {
        /// `sar_width`, `u(16)`.
        sar_width: u16,
        /// `sar_height`, `u(16)`.
        sar_height: u16,
    },
}

/// (idc, SAR width, SAR height) for the Table E-1 rows 1..=16, in idc order.
/// The single copy of the table: [`AspectRatioIdc::from_wire`],
/// [`AspectRatioIdc::idc`] and [`AspectRatioIdc::sample_aspect_ratio`] all
/// read it, so they cannot disagree.
const TABLE_E1: [(AspectRatioIdc, u16, u16); 16] = [
    (AspectRatioIdc::Square, 1, 1),
    (AspectRatioIdc::Sar12x11, 12, 11),
    (AspectRatioIdc::Sar10x11, 10, 11),
    (AspectRatioIdc::Sar16x11, 16, 11),
    (AspectRatioIdc::Sar40x33, 40, 33),
    (AspectRatioIdc::Sar24x11, 24, 11),
    (AspectRatioIdc::Sar20x11, 20, 11),
    (AspectRatioIdc::Sar32x11, 32, 11),
    (AspectRatioIdc::Sar80x33, 80, 33),
    (AspectRatioIdc::Sar18x11, 18, 11),
    (AspectRatioIdc::Sar15x11, 15, 11),
    (AspectRatioIdc::Sar64x33, 64, 33),
    (AspectRatioIdc::Sar160x99, 160, 99),
    (AspectRatioIdc::Sar4x3, 4, 3),
    (AspectRatioIdc::Sar3x2, 3, 2),
    (AspectRatioIdc::Sar2x1, 2, 1),
];

impl AspectRatioIdc {
    /// Build from the wire `aspect_ratio_idc` and, when it is
    /// [`EXTENDED_SAR_IDC`], the `(sar_width, sar_height)` that follow it.
    /// `extended` is ignored for every other idc.
    ///
    /// An Extended_SAR idc with no `extended` pair is stored as `0:0`
    /// (unspecified), not invented.
    pub fn from_wire(idc: u8, extended: Option<(u16, u16)>) -> Self {
        match idc {
            0 => Self::Unspecified,
            EXTENDED_SAR_IDC => {
                let (sar_width, sar_height) = extended.unwrap_or((0, 0));
                Self::ExtendedSar {
                    sar_width,
                    sar_height,
                }
            }
            FIRST_RESERVED_IDC..=254 => Self::Reserved(idc),
            // 1..=16: `TABLE_E1[idc - 1]`, indexed by construction (the arms
            // above removed 0, 17..=254 and 255).
            _ => TABLE_E1[usize::from(idc) - 1].0,
        }
    }

    /// The wire `aspect_ratio_idc` value (the inverse of
    /// [`from_wire`](Self::from_wire)).
    pub fn idc(&self) -> u8 {
        match self {
            Self::Unspecified => 0,
            Self::Reserved(idc) => *idc,
            Self::ExtendedSar { .. } => EXTENDED_SAR_IDC,
            table_row => TABLE_E1
                .iter()
                .zip(1u8..)
                .find_map(|((row, _, _), idc)| (row == table_row).then_some(idc))
                // Unreachable: every variant not matched above is a table row.
                .unwrap_or(0),
        }
    }

    /// The specified sample aspect ratio, or `None` when it is unspecified:
    /// idc 0, the reserved range 17..=254 (H.265 §E.3.1 "interpret as 0",
    /// applied to both codecs), or Extended_SAR with a zero component.
    pub fn sample_aspect_ratio(&self) -> Option<SampleAspectRatio> {
        match self {
            Self::Unspecified | Self::Reserved(_) => None,
            Self::ExtendedSar {
                sar_width,
                sar_height,
            } => SampleAspectRatio::new(*sar_width, *sar_height),
            table_row => TABLE_E1
                .iter()
                .find(|(row, _, _)| row == table_row)
                .and_then(|(_, w, h)| SampleAspectRatio::new(*w, *h)),
        }
    }

    /// Label: the spec token (`"16:11"`, `"Extended_SAR"`), `"unspecified"` for
    /// idc 0, `"reserved"` for 17..=254.
    pub fn name(&self) -> &'static str {
        match self {
            Self::Unspecified => "unspecified",
            Self::Square => "1:1",
            Self::Sar12x11 => "12:11",
            Self::Sar10x11 => "10:11",
            Self::Sar16x11 => "16:11",
            Self::Sar40x33 => "40:33",
            Self::Sar24x11 => "24:11",
            Self::Sar20x11 => "20:11",
            Self::Sar32x11 => "32:11",
            Self::Sar80x33 => "80:33",
            Self::Sar18x11 => "18:11",
            Self::Sar15x11 => "15:11",
            Self::Sar64x33 => "64:33",
            Self::Sar160x99 => "160:99",
            Self::Sar4x3 => "4:3",
            Self::Sar3x2 => "3:2",
            Self::Sar2x1 => "2:1",
            Self::Reserved(_) => "reserved",
            Self::ExtendedSar { .. } => "Extended_SAR",
        }
    }
}

broadcast_common::impl_spec_display!(AspectRatioIdc, Reserved);

/// A *specified* sample (pixel) aspect ratio: both terms non-zero. Unspecified
/// is `Option::None` at every use site, never a `0:0` sentinel.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SampleAspectRatio {
    width: NonZeroU16,
    height: NonZeroU16,
}

impl SampleAspectRatio {
    /// `None` when either term is 0 (the spec's "unspecified").
    pub fn new(width: u16, height: u16) -> Option<Self> {
        Some(Self {
            width: NonZeroU16::new(width)?,
            height: NonZeroU16::new(height)?,
        })
    }

    /// Horizontal term (`sar_width`).
    pub fn width(&self) -> u16 {
        self.width.get()
    }

    /// Vertical term (`sar_height`).
    pub fn height(&self) -> u16 {
        self.height.get()
    }

    /// `(width, height)`.
    pub fn as_pair(&self) -> (u16, u16) {
        (self.width(), self.height())
    }

    /// Reduced to lowest terms. The spec requires `sar_width` / `sar_height`
    /// to be relatively prime already; a non-reduced wire pair is kept as
    /// signalled in [`AspectRatioIdc::ExtendedSar`] and only reduced here.
    pub fn reduced(&self) -> Self {
        let (mut a, mut b) = (self.width(), self.height());
        while b != 0 {
            (a, b) = (b, a % b);
        }
        // `a` is the gcd of two non-zero values, so non-zero.
        let g = a.max(1);
        Self {
            width: NonZeroU16::new(self.width() / g).unwrap_or(self.width),
            height: NonZeroU16::new(self.height() / g).unwrap_or(self.height),
        }
    }
}

impl From<SampleAspectRatio> for (u16, u16) {
    fn from(sar: SampleAspectRatio) -> Self {
        sar.as_pair()
    }
}

/// The spec's inferred `video_format` when `video_signal_type_present_flag`
/// is 0 (H.264 §E.2.1: "shall be inferred to be equal to 5").
pub const INFERRED_VIDEO_FORMAT: u8 = 5;

/// The spec's inferred `video_full_range_flag` when absent (H.264 §E.2.1: "= 0").
pub const INFERRED_FULL_RANGE: bool = false;

/// The spec's inferred `colour_primaries` / `transfer_characteristics` /
/// `matrix_coefficients` when absent (H.264 §E.2.1: "= 2", unspecified).
pub const INFERRED_COLOUR_CODE: u8 = 2;

/// H.264 Table E-2 / H.265 Table E.2 `video_format`, the wire form kept
/// losslessly ([`from_wire`](Self::from_wire) / [`code`](Self::code)).
///
/// The rows 0..=5 are identical in both specs. H.264 defines 6 and 7 as
/// `Reserved` with no interpretation; H.265 additionally says decoders "shall
/// interpret the values 6 and 7 ... as equivalent to the value 5". This crate
/// names neither (H.264's label is only "Reserved"), so both are
/// [`Reserved`](Self::Reserved). Unspecified is row 5.
#[non_exhaustive]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum VideoFormat {
    /// 0: component.
    Component,
    /// 1: PAL.
    Pal,
    /// 2: NTSC.
    Ntsc,
    /// 3: SECAM.
    Secam,
    /// 4: MAC.
    Mac,
    /// 5: unspecified video format.
    Unspecified,
    /// 6 or 7: reserved (wire value kept).
    Reserved(u8),
}

impl VideoFormat {
    /// Decode a 3-bit `video_format` value. `None` for a value the `u(3)`
    /// field cannot carry (so [`code`](Self::code) round-trips every value the
    /// wire can produce). 6 and 7 are [`Reserved`](Self::Reserved).
    pub fn from_wire(code: u8) -> Option<Self> {
        match code {
            0 => Some(Self::Component),
            1 => Some(Self::Pal),
            2 => Some(Self::Ntsc),
            3 => Some(Self::Secam),
            4 => Some(Self::Mac),
            5 => Some(Self::Unspecified),
            6 | 7 => Some(Self::Reserved(code)),
            _ => None,
        }
    }

    /// The wire `video_format` value.
    pub fn code(&self) -> u8 {
        match self {
            Self::Component => 0,
            Self::Pal => 1,
            Self::Ntsc => 2,
            Self::Secam => 3,
            Self::Mac => 4,
            Self::Unspecified => 5,
            Self::Reserved(code) => *code,
        }
    }

    /// Label: the spec's Table E-2 meaning, `"reserved"` for 6/7.
    pub fn name(&self) -> &'static str {
        match self {
            Self::Component => "Component",
            Self::Pal => "PAL",
            Self::Ntsc => "NTSC",
            Self::Secam => "SECAM",
            Self::Mac => "MAC",
            Self::Unspecified => "Unspecified video format",
            Self::Reserved(_) => "reserved",
        }
    }

    /// `video_format` when the VUI omits the field (inferred 5).
    pub fn inferred() -> Self {
        Self::from_wire(INFERRED_VIDEO_FORMAT).unwrap_or(Self::Unspecified)
    }
}

broadcast_common::impl_spec_display!(VideoFormat, Reserved);

/// H.264 Table E-3 / H.265 Table E.3 `colour_primaries` (identical rows in
/// both specs), the wire form kept losslessly.
#[non_exhaustive]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ColourPrimaries {
    /// 0: reserved for future use by ITU-T | ISO/IEC.
    Reserved(u8),
    /// 1: Rec. ITU-R BT.709-6 (also BT.1361-0 conventional gamut, sRGB/sYCC).
    Bt709,
    /// 2: unspecified (image characteristics unknown or determined by the
    /// application).
    Unspecified,
    /// 4: Rec. ITU-R BT.470-7 System M (historical; NTSC 1953).
    Bt470SystemM,
    /// 5: Rec. ITU-R BT.470-7 System B, G (historical; BT.601-7 625).
    Bt470SystemBg,
    /// 6: Rec. ITU-R BT.601-7 525 (functionally the same as row 7).
    Bt601_525,
    /// 7: SMPTE 240M (1999, historical) (functionally the same as row 6).
    Smpte240M,
    /// 8: generic film (colour filters using Illuminant C).
    GenericFilm,
    /// 9: Rec. ITU-R BT.2020-2.
    Bt2020,
    /// 10: SMPTE ST 428-1 (2006) (CIE 1931 XYZ as in ISO 11664-1).
    Xyz,
    /// 11: SMPTE RP 431-2 (2011) (SMPTE ST 2113 (2018) "P3DCI").
    SmpteRp431_2,
    /// 12: SMPTE EG 432-1 (2010) (SMPTE ST 2113 (2018) "P3D65").
    SmpteEg432_1,
    /// 22: EBU Tech. 3213-E (1975).
    EbuTech3213E,
}

impl ColourPrimaries {
    /// Decode a `colour_primaries` value; unnamed rows are reserved.
    fn from_wire(code: u8) -> Self {
        match code {
            1 => Self::Bt709,
            2 => Self::Unspecified,
            4 => Self::Bt470SystemM,
            5 => Self::Bt470SystemBg,
            6 => Self::Bt601_525,
            7 => Self::Smpte240M,
            8 => Self::GenericFilm,
            9 => Self::Bt2020,
            10 => Self::Xyz,
            11 => Self::SmpteRp431_2,
            12 => Self::SmpteEg432_1,
            22 => Self::EbuTech3213E,
            other => Self::Reserved(other),
        }
    }

    /// The wire `colour_primaries` value (inverse of
    /// [`from_wire`](Self::from_wire)).
    pub fn code(&self) -> u8 {
        match self {
            Self::Reserved(code) => *code,
            Self::Bt709 => 1,
            Self::Unspecified => 2,
            Self::Bt470SystemM => 4,
            Self::Bt470SystemBg => 5,
            Self::Bt601_525 => 6,
            Self::Smpte240M => 7,
            Self::GenericFilm => 8,
            Self::Bt2020 => 9,
            Self::Xyz => 10,
            Self::SmpteRp431_2 => 11,
            Self::SmpteEg432_1 => 12,
            Self::EbuTech3213E => 22,
        }
    }

    /// Label: the Table E-3 primaries text (without the informative remark),
    /// `"reserved"` for reserved rows.
    pub fn name(&self) -> &'static str {
        match self {
            Self::Reserved(_) => "reserved",
            Self::Bt709 => "Rec. ITU-R BT.709-6",
            Self::Unspecified => "Unspecified",
            Self::Bt470SystemM => "Rec. ITU-R BT.470-7 System M (historical)",
            Self::Bt470SystemBg => "Rec. ITU-R BT.470-7 System B, G (historical)",
            Self::Bt601_525 => "Rec. ITU-R BT.601-7 525",
            Self::Smpte240M => "SMPTE 240M (1999, historical)",
            Self::GenericFilm => "Generic film (colour filters using Illuminant C)",
            Self::Bt2020 => "Rec. ITU-R BT.2020-2",
            Self::Xyz => "SMPTE ST 428-1 (2006)",
            Self::SmpteRp431_2 => "SMPTE RP 431-2 (2011)",
            Self::SmpteEg432_1 => "SMPTE EG 432-1 (2010)",
            Self::EbuTech3213E => "EBU Tech. 3213-E (1975)",
        }
    }

    /// `colour_primaries` when the VUI omits the field (inferred 2).
    pub fn inferred() -> Self {
        Self::Unspecified
    }
}

broadcast_common::impl_spec_display!(ColourPrimaries, Reserved);

/// H.264 Table E-4 / H.265 Table E.4 `transfer_characteristics` (identical
/// rows in both specs), the wire form kept losslessly.
#[non_exhaustive]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TransferCharacteristics {
    /// 0: reserved for future use by ITU-T | ISO/IEC.
    Reserved(u8),
    /// 1: Rec. ITU-R BT.709-6 (also BT.1361-0 conventional gamut).
    Bt709,
    /// 2: unspecified.
    Unspecified,
    /// 4: assumed display gamma 2.2 (BT.470-7 System M, historical).
    Gamma22,
    /// 5: assumed display gamma 2.8 (BT.470-7 System B, G, historical).
    Gamma28,
    /// 6: Rec. ITU-R BT.601-7 (525 or 625).
    Bt601,
    /// 7: SMPTE 240M (1999, historical).
    Smpte240M,
    /// 8: linear transfer characteristics.
    Linear,
    /// 9: logarithmic transfer characteristic (100:1 range).
    Log100,
    /// 10: logarithmic transfer characteristic (100 * Sqrt(10) : 1 range).
    Log316,
    /// 11: IEC 61966-2-4.
    Iec61966_2_4,
    /// 12: Rec. ITU-R BT.1361-0 extended colour gamut system (historical).
    Bt1361Extended,
    /// 13: IEC 61966-2-1 sRGB / sYCC.
    Iec61966_2_1,
    /// 14: Rec. ITU-R BT.2020-2 (10 bit system).
    Bt2020_10Bit,
    /// 15: Rec. ITU-R BT.2020-2 (12 bit system).
    Bt2020_12Bit,
    /// 16: SMPTE ST 2084 (2014) / Rec. ITU-R BT.2100-2 PQ system.
    SmpteSt2084,
    /// 17: SMPTE ST 428-1 (2006).
    SmpteSt428_1,
    /// 18: ARIB STD-B67 / Rec. ITU-R BT.2100-2 HLG system.
    AribStdB67,
}

impl TransferCharacteristics {
    /// Decode a `transfer_characteristics` value; unnamed rows are reserved.
    fn from_wire(code: u8) -> Self {
        match code {
            1 => Self::Bt709,
            2 => Self::Unspecified,
            4 => Self::Gamma22,
            5 => Self::Gamma28,
            6 => Self::Bt601,
            7 => Self::Smpte240M,
            8 => Self::Linear,
            9 => Self::Log100,
            10 => Self::Log316,
            11 => Self::Iec61966_2_4,
            12 => Self::Bt1361Extended,
            13 => Self::Iec61966_2_1,
            14 => Self::Bt2020_10Bit,
            15 => Self::Bt2020_12Bit,
            16 => Self::SmpteSt2084,
            17 => Self::SmpteSt428_1,
            18 => Self::AribStdB67,
            other => Self::Reserved(other),
        }
    }

    /// The wire `transfer_characteristics` value.
    pub fn code(&self) -> u8 {
        match self {
            Self::Reserved(code) => *code,
            Self::Bt709 => 1,
            Self::Unspecified => 2,
            Self::Gamma22 => 4,
            Self::Gamma28 => 5,
            Self::Bt601 => 6,
            Self::Smpte240M => 7,
            Self::Linear => 8,
            Self::Log100 => 9,
            Self::Log316 => 10,
            Self::Iec61966_2_4 => 11,
            Self::Bt1361Extended => 12,
            Self::Iec61966_2_1 => 13,
            Self::Bt2020_10Bit => 14,
            Self::Bt2020_12Bit => 15,
            Self::SmpteSt2084 => 16,
            Self::SmpteSt428_1 => 17,
            Self::AribStdB67 => 18,
        }
    }

    /// Label: the Table E-4 transfer characteristic / informative remark
    /// (without the transfer-function equation), `"reserved"` for reserved
    /// rows. HLG (18) is an H.265-defined row that H.264 also defines.
    pub fn name(&self) -> &'static str {
        match self {
            Self::Reserved(_) => "reserved",
            Self::Bt709 => "Rec. ITU-R BT.709-6",
            Self::Unspecified => "Unspecified",
            Self::Gamma22 => "Assumed display gamma 2.2",
            Self::Gamma28 => "Assumed display gamma 2.8",
            Self::Bt601 => "Rec. ITU-R BT.601-7 525 or 625",
            Self::Smpte240M => "SMPTE 240M (1999, historical)",
            Self::Linear => "Linear transfer characteristics",
            Self::Log100 => "Logarithmic transfer characteristic (100:1 range)",
            Self::Log316 => "Logarithmic transfer characteristic (100 * Sqrt(10) : 1 range)",
            Self::Iec61966_2_4 => "IEC 61966-2-4",
            Self::Bt1361Extended => {
                "Rec. ITU-R BT.1361-0 extended colour gamut system (historical)"
            }
            Self::Iec61966_2_1 => {
                "IEC 61966-2-1 sRGB (with matrix_coefficients equal to 0) \
                 / IEC 61966-2-1 sYCC (with matrix_coefficients equal to 5)"
            }
            Self::Bt2020_10Bit => "Rec. ITU-R BT.2020-2 (10 bit system)",
            Self::Bt2020_12Bit => "Rec. ITU-R BT.2020-2 (12 bit system)",
            Self::SmpteSt2084 => "SMPTE ST 2084 (2014) / Rec. ITU-R BT.2100-2 PQ system",
            Self::SmpteSt428_1 => "SMPTE ST 428-1 (2006)",
            Self::AribStdB67 => "ARIB STD-B67 / Rec. ITU-R BT.2100-2 HLG system",
        }
    }

    /// `transfer_characteristics` when the VUI omits the field (inferred 2).
    pub fn inferred() -> Self {
        Self::Unspecified
    }
}

broadcast_common::impl_spec_display!(TransferCharacteristics, Reserved);

/// H.264 Table E-5 / H.265 Table E.5 `matrix_coefficients` (identical rows in
/// both specs), the wire form kept losslessly.
///
/// The broadcaster-facing rows are [`Bt709`](Self::Bt709),
/// [`Bt470SystemB`](Self::Bt470SystemB) (BT.601 625) and
/// [`Bt470SystemM`](Self::Bt470SystemM) (BT.601 525), plus
/// [`Bt2020NonConstant`](Self::Bt2020NonConstant) for UHD.
#[non_exhaustive]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MatrixCoefficients {
    /// 0: GBR (H.264) / Identity (H.265): the identity matrix (RGB / YZX).
    Gbr,
    /// 1: KR = 0.2126; KB = 0.0722 (Rec. ITU-R BT.709-6).
    Bt709,
    /// 2: unspecified.
    Unspecified,
    /// 3: reserved for future use by ITU-T | ISO/IEC.
    Reserved(u8),
    /// 4: KR = 0.30; KB = 0.11 (FCC Title 47 CFR 73.682).
    Fcc,
    /// 5: KR = 0.299; KB = 0.114 (BT.470-7 System B, G / BT.601-7 625 / sYCC).
    Bt470SystemB,
    /// 6: KR = 0.299; KB = 0.114 (BT.601-7 525 / SMPTE 170M).
    Bt470SystemM,
    /// 7: KR = 0.212; KB = 0.087 (SMPTE 240M, 1999, historical).
    Smpte240M,
    /// 8: YCgCo or YCgCo-R.
    YCgCo,
    /// 9: KR = 0.2627; KB = 0.0593 (BT.2020-2 non-constant luminance).
    Bt2020NonConstant,
    /// 10: KR = 0.2627; KB = 0.0593 (BT.2020-2 constant luminance).
    Bt2020Constant,
    /// 11: Y'D'ZD'X (SMPTE ST 2085 (2015)).
    SmpteSt2085,
    /// 12: chromaticity-derived non-constant luminance system.
    ChromaticityDerivedNonConstant,
    /// 13: chromaticity-derived constant luminance system.
    ChromaticityDerivedConstant,
    /// 14: ICtCp (BT.2100-2).
    ICtCp,
    /// 15: IPT-C2.
    IptC2,
    /// 16: YCgCo-Re.
    YCgCoRe,
    /// 17: YCgCo-Ro.
    YCgCoRo,
}

impl MatrixCoefficients {
    /// Decode a `matrix_coefficients` value; unnamed rows are reserved.
    fn from_wire(code: u8) -> Self {
        match code {
            0 => Self::Gbr,
            1 => Self::Bt709,
            2 => Self::Unspecified,
            4 => Self::Fcc,
            5 => Self::Bt470SystemB,
            6 => Self::Bt470SystemM,
            7 => Self::Smpte240M,
            8 => Self::YCgCo,
            9 => Self::Bt2020NonConstant,
            10 => Self::Bt2020Constant,
            11 => Self::SmpteSt2085,
            12 => Self::ChromaticityDerivedNonConstant,
            13 => Self::ChromaticityDerivedConstant,
            14 => Self::ICtCp,
            15 => Self::IptC2,
            16 => Self::YCgCoRe,
            17 => Self::YCgCoRo,
            other => Self::Reserved(other),
        }
    }

    /// The wire `matrix_coefficients` value.
    pub fn code(&self) -> u8 {
        match self {
            Self::Gbr => 0,
            Self::Bt709 => 1,
            Self::Unspecified => 2,
            Self::Reserved(code) => *code,
            Self::Fcc => 4,
            Self::Bt470SystemB => 5,
            Self::Bt470SystemM => 6,
            Self::Smpte240M => 7,
            Self::YCgCo => 8,
            Self::Bt2020NonConstant => 9,
            Self::Bt2020Constant => 10,
            Self::SmpteSt2085 => 11,
            Self::ChromaticityDerivedNonConstant => 12,
            Self::ChromaticityDerivedConstant => 13,
            Self::ICtCp => 14,
            Self::IptC2 => 15,
            Self::YCgCoRe => 16,
            Self::YCgCoRo => 17,
        }
    }

    /// Label: the Table E-5 matrix / informative remark (without the
    /// equations), `"reserved"` for reserved rows. Row 0 uses H.264's `GBR`.
    pub fn name(&self) -> &'static str {
        match self {
            Self::Gbr => "GBR",
            Self::Bt709 => "Rec. ITU-R BT.709-6",
            Self::Unspecified => "Unspecified",
            Self::Reserved(_) => "reserved",
            Self::Fcc => "FCC Title 47 CFR 73.682 (a) (20)",
            Self::Bt470SystemB => {
                "Rec. ITU-R BT.470-7 System B, G (historical) / Rec. ITU-R BT.601-7 625"
            }
            Self::Bt470SystemM => "Rec. ITU-R BT.601-7 525 / SMPTE 170M (2004)",
            Self::Smpte240M => "SMPTE 240M (1999, historical)",
            Self::YCgCo => "YCgCo or YCgCo-R",
            Self::Bt2020NonConstant => {
                "Rec. ITU-R BT.2020-2 non-constant luminance system / Rec. ITU-R BT.2100-2 Y'CbCr"
            }
            Self::Bt2020Constant => "Rec. ITU-R BT.2020-2 constant luminance system",
            Self::SmpteSt2085 => "Y'D'ZD'X (SMPTE ST 2085 (2015))",
            Self::ChromaticityDerivedNonConstant => {
                "Chromaticity-derived non-constant luminance system"
            }
            Self::ChromaticityDerivedConstant => "Chromaticity-derived constant luminance system",
            Self::ICtCp => "ICtCp (Rec. ITU-R BT.2100-2)",
            Self::IptC2 => "IPT-C2 (IPT-PQ-C2)",
            Self::YCgCoRe => "YCgCo-Re",
            Self::YCgCoRo => "YCgCo-Ro",
        }
    }

    /// `matrix_coefficients` when the VUI omits the field (inferred 2).
    pub fn inferred() -> Self {
        Self::Unspecified
    }
}

broadcast_common::impl_spec_display!(MatrixCoefficients, Reserved);

/// `colour_description_present_flag == 1`: H.264 §E.2.1 Tables E-3..E-5 /
/// H.265 §E.3.1 Tables E.3..E.5.
#[non_exhaustive]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ColourDescription {
    /// `colour_primaries`.
    pub primaries: ColourPrimaries,
    /// `transfer_characteristics`.
    pub transfer: TransferCharacteristics,
    /// `matrix_coefficients` (H.265: `matrix_coeffs`).
    pub matrix: MatrixCoefficients,
}

impl ColourDescription {
    /// Build from the three wire values (`u(8)` each), keeping every code
    /// point losslessly.
    pub fn from_wire(primaries: u8, transfer: u8, matrix: u8) -> Self {
        Self {
            primaries: ColourPrimaries::from_wire(primaries),
            transfer: TransferCharacteristics::from_wire(transfer),
            matrix: MatrixCoefficients::from_wire(matrix),
        }
    }
}

/// `video_signal_type_present_flag == 1`: H.264 §E.2.1 / H.265 §E.3.1.
///
/// Only what was signalled is recorded, except that `video_format` has no
/// `None` state on the wire (the field is always the full `u(3)` when the
/// block is present). The spec's inferred values when the whole block is
/// absent live on [`DisplayGeometry::video_format`](DisplayGeometry::video_format)
/// and friends, never in here.
#[non_exhaustive]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct VideoSignalType {
    /// `video_format`.
    pub video_format: VideoFormat,
    /// `video_full_range_flag` (1 = full range).
    pub full_range: bool,
    /// `None` when `colour_description_present_flag` is 0 (the spec then
    /// infers primaries/transfer/matrix 2, available via
    /// [`DisplayGeometry::colour_description`](DisplayGeometry::colour_description)).
    pub colour: Option<ColourDescription>,
}

impl VideoSignalType {
    /// Build from the field values as read.
    pub fn new(
        video_format: VideoFormat,
        full_range: bool,
        colour: Option<ColourDescription>,
    ) -> Self {
        Self {
            video_format,
            full_range,
            colour,
        }
    }
}

/// Sequence-level display geometry of a video track: everything derived from
/// the active SPS (in a conforming stream it changes only at an IDR / IRAP,
/// H.264 §7.4.1.2.1 / H.265 §7.4.3.2). Lives on
/// [`TrackSpec::display`](super::TrackSpec::display).
///
/// Phases 1a/1b carry the coded size, the VUI aspect ratio, the overscan
/// flag and the video signal type (format, full range, colour description);
/// cropping and the H.265 default display window are additive follow-ups.
#[non_exhaustive]
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DisplayGeometry {
    /// Decoded picture width before cropping, in luma samples (H.264
    /// `PicWidthInSamplesL`; H.265 `pic_width_in_luma_samples`). `u32`, wider
    /// than the `u16` [`CodecConfig`] carries; unlike `CodecConfig`'s width it
    /// is **not** cropped.
    pub coded_width: u32,
    /// Decoded picture height before cropping, in luma samples (H.264
    /// `16 * FrameHeightInMbs`; H.265 `pic_height_in_luma_samples`).
    pub coded_height: u32,
    /// The VUI `aspect_ratio_idc` as signalled. `None` when the SPS has no VUI,
    /// `aspect_ratio_info_present_flag` is 0, or the VUI was truncated before
    /// the field was complete (all inferred idc 0, "unspecified"). `Some` does
    /// **not** imply a specified ratio: see
    /// [`sample_aspect_ratio`](Self::sample_aspect_ratio).
    pub aspect_ratio: Option<AspectRatioIdc>,
    /// `overscan_appropriate_flag` as signalled. `None` when
    /// `overscan_info_present_flag` is 0 or the VUI was truncated before it.
    pub overscan_appropriate: Option<bool>,
    /// `video_signal_type()` as signalled. `None` when
    /// `video_signal_type_present_flag` is 0 (or there is no VUI). The spec's
    /// inferred defaults (format 5, limited range, colour 2/2/2) are then
    /// available through [`video_format`](Self::video_format),
    /// [`full_range`](Self::full_range) and
    /// [`colour_description`](Self::colour_description); the struct itself
    /// records only what was signalled.
    pub video_signal: Option<VideoSignalType>,
}

impl DisplayGeometry {
    /// Build from explicit values (for muxers and tests; demuxed tracks get
    /// theirs from the SPS).
    pub fn new(coded_width: u32, coded_height: u32, aspect_ratio: Option<AspectRatioIdc>) -> Self {
        Self {
            coded_width,
            coded_height,
            aspect_ratio,
            overscan_appropriate: None,
            video_signal: None,
        }
    }

    /// Geometry of an H.264 SPS NAL unit (1-byte NAL header included; the same
    /// input as [`decode_avc_sps`](crate::decode_avc_sps), and the same VUI
    /// walk).
    pub fn from_avc_sps(sps_nal: &[u8]) -> Result<Self> {
        crate::sps::decode_avc_display_facts(sps_nal).map(Self::from_facts)
    }

    /// Geometry of an H.265 SPS NAL unit (2-byte NAL header included; the same
    /// input as [`decode_hevc_sps`](crate::decode_hevc_sps)).
    pub fn from_hevc_sps(sps_nal: &[u8]) -> Result<Self> {
        crate::sps::decode_hevc_display_facts(sps_nal).map(Self::from_facts)
    }

    /// Geometry derived from the first SPS of an `avcC` / `hvcC` record.
    /// `None` for other codecs, for a record with no SPS (a placeholder
    /// config), or when that SPS does not decode; never a fabricated value.
    pub fn from_codec_config(config: &CodecConfig) -> Option<Self> {
        match config {
            CodecConfig::Avc { config, .. } => {
                let sps = config.config.sps.first()?;
                Self::from_avc_sps(&sps.0).ok()
            }
            CodecConfig::Hevc { config, .. } => {
                let sps = config
                    .config
                    .arrays
                    .iter()
                    .filter(|a| a.nal_unit_type == crate::nalu_types::HEVC_SPS_NUT)
                    .flat_map(|a| a.nalus.iter())
                    .next()?;
                Self::from_hevc_sps(&sps.0).ok()
            }
            _ => None,
        }
    }

    fn from_facts(f: crate::sps::DisplayFacts) -> Self {
        Self {
            coded_width: f.coded_width,
            coded_height: f.coded_height,
            aspect_ratio: f.aspect_ratio,
            overscan_appropriate: f.overscan_appropriate,
            video_signal: f.video_signal,
        }
    }

    /// The specified sample aspect ratio as `(width, height)`, or `None` when
    /// it is unspecified (no VUI / flag clear, idc 0, reserved idc, or an
    /// Extended_SAR with a zero component). Exactly as signalled, not reduced:
    /// see [`SampleAspectRatio::reduced`].
    pub fn sample_aspect_ratio(&self) -> Option<(u16, u16)> {
        self.typed_sample_aspect_ratio().map(Into::into)
    }

    /// [`sample_aspect_ratio`](Self::sample_aspect_ratio) as the typed
    /// [`SampleAspectRatio`].
    pub fn typed_sample_aspect_ratio(&self) -> Option<SampleAspectRatio> {
        self.aspect_ratio?.sample_aspect_ratio()
    }

    /// `video_full_range_flag`: `true` only when the VUI signalled full range;
    /// the spec's inferred value when the video signal type is absent is 0
    /// (limited range), so this returns `false` then.
    pub fn full_range(&self) -> bool {
        self.video_signal.is_some_and(|vst| vst.full_range)
    }

    /// `video_format`, the signalled value or the inferred 5 when the video
    /// signal type is absent (H.264 §E.2.1).
    pub fn video_format(&self) -> VideoFormat {
        self.video_signal
            .map_or_else(VideoFormat::inferred, |vst| vst.video_format)
    }

    /// The colour description, the signalled triple or, when
    /// `colour_description_present_flag` is 0 (or the whole video signal type
    /// is absent), the spec's inferred primaries / transfer / matrix 2/2/2.
    /// A player wanting to know whether the stream really said BT.709 reads
    /// [`video_signal`](Self::video_signal) instead.
    pub fn colour_description(&self) -> ColourDescription {
        self.video_signal
            .and_then(|vst| vst.colour)
            .unwrap_or(ColourDescription {
                primaries: ColourPrimaries::inferred(),
                transfer: TransferCharacteristics::inferred(),
                matrix: MatrixCoefficients::inferred(),
            })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::string::ToString;

    #[test]
    fn idc_round_trips_for_every_wire_value() {
        for idc in 0..=u8::MAX {
            let v = AspectRatioIdc::from_wire(idc, Some((3, 4)));
            assert_eq!(v.idc(), idc, "idc {idc}");
        }
    }

    #[test]
    fn variants_land_on_their_table_e1_rows() {
        assert_eq!(
            AspectRatioIdc::from_wire(0, None),
            AspectRatioIdc::Unspecified
        );
        assert_eq!(AspectRatioIdc::from_wire(1, None), AspectRatioIdc::Square);
        assert_eq!(AspectRatioIdc::from_wire(4, None), AspectRatioIdc::Sar16x11);
        assert_eq!(AspectRatioIdc::from_wire(14, None), AspectRatioIdc::Sar4x3);
        assert_eq!(AspectRatioIdc::from_wire(16, None), AspectRatioIdc::Sar2x1);
        assert_eq!(
            AspectRatioIdc::from_wire(17, None),
            AspectRatioIdc::Reserved(17)
        );
        assert_eq!(
            AspectRatioIdc::from_wire(255, Some((64, 45))),
            AspectRatioIdc::ExtendedSar {
                sar_width: 64,
                sar_height: 45
            }
        );
        // `extended` is ignored for a non-255 idc, and a missing pair for 255
        // is unspecified rather than invented.
        assert_eq!(
            AspectRatioIdc::from_wire(4, Some((9, 9))),
            AspectRatioIdc::Sar16x11
        );
        assert_eq!(
            AspectRatioIdc::from_wire(255, None).sample_aspect_ratio(),
            None
        );
    }

    #[test]
    fn labels_follow_the_spec_tokens() {
        assert_eq!(AspectRatioIdc::Square.to_string(), "1:1");
        assert_eq!(AspectRatioIdc::Sar160x99.to_string(), "160:99");
        assert_eq!(AspectRatioIdc::Unspecified.to_string(), "unspecified");
        assert_eq!(AspectRatioIdc::Reserved(0x11).to_string(), "reserved(0x11)");
        let ext = AspectRatioIdc::from_wire(255, Some((64, 45)));
        assert_eq!(ext.to_string(), "Extended_SAR");
    }

    #[test]
    fn sample_aspect_ratio_rejects_a_zero_term() {
        assert!(SampleAspectRatio::new(0, 1).is_none());
        assert!(SampleAspectRatio::new(1, 0).is_none());
        let r = SampleAspectRatio::new(64, 45).unwrap();
        assert_eq!((r.width(), r.height()), (64, 45));
        assert_eq!(r.reduced(), r);
        let r = SampleAspectRatio::new(10, 4).unwrap();
        assert_eq!(r.reduced().as_pair(), (5, 2));
    }

    #[test]
    fn codecs_without_an_sps_have_no_geometry() {
        let data = CodecConfig::Data {
            stream_type: 0x06,
            descriptors: alloc::vec::Vec::new(),
            carriage: crate::ir::DataCarriage::Pes,
        };
        assert_eq!(DisplayGeometry::from_codec_config(&data), None);
    }
}
