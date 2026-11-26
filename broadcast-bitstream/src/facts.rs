//! Coded picture facts: the values a codec specification defines, kept as the
//! wire form.
//!
//! These are the parts of a video stream's description that a codec
//! specification states, not a presentation or container concept: the sample
//! aspect ratio and cropping rectangle a sequence parameter set signals, and
//! the colour description, video format, active-format description and bar
//! data a bitstream carries. A presentation type built from them (a display
//! geometry, a track's active format) lives in the consumer.
//!
//! Every type is `#[non_exhaustive]` with private fields and accessors, so a
//! later syntax element is an additive change, and every spec/field enum
//! carries `name()` plus `Display`.
//!
//! # Spec citations
//!
//! - **H.264**: ITU-T H.264 (V15, 08/2024) §E.1.1 (`vui_parameters()` syntax),
//!   §E.2.1 Table E-1 (meaning of `aspect_ratio_idc`), Table E-2
//!   (`video_format`), Table E-3 (`colour_primaries`), Table E-4
//!   (`transfer_characteristics`) and Table E-5 (`matrix_coefficients`).
//! - **H.265**: ITU-T H.265 (V11, 01/2026) §E.2.1 (syntax) and §E.3.1
//!   Tables E.1 (same values as Table E-1), E.2, E.3, E.4 and E.5.
//! - **AFD / bar data**: ETSI TS 101 154 Table B.5 / SMPTE ST 2016-1 Table 1
//!   (active format description) and ETSI TS 101 154 Table B.7 (bar data).
//!
//! Every table was re-verified row by row against the rendered PDF pages and
//! is transcribed in `private/specs-verified/mpeg/transmux/codec/vui-colour-tables.md`
//! and `private/specs-verified/mpeg/display-geometry/public-notes/SPEC-NOTES.md`.
//!
//! # Semantics kept exactly
//!
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

use core::num::NonZeroU16;

/// `aspect_ratio_idc` value that selects `sar_width` / `sar_height`
/// (H.264 Table E-1 `Extended_SAR`; H.265 Table E.1 `EXTENDED_SAR`).
pub const EXTENDED_SAR_IDC: u8 = 255;

/// First reserved `aspect_ratio_idc` (H.264 Table E-1 / H.265 Table E.1:
/// "17..254 Reserved").
const FIRST_RESERVED_IDC: u8 = 17;

/// Last reserved `aspect_ratio_idc` (H.264 Table E-1 / H.265 Table E.1:
/// "17..254 Reserved").
const LAST_RESERVED_IDC: u8 = 254;

/// `active_format` is carried in the low four bits of the AFD byte
/// (ETSI TS 101 154 Table B.5).
const AFD_CODE_MASK: u8 = 0x0F;

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
            FIRST_RESERVED_IDC..=LAST_RESERVED_IDC => Self::Reserved(idc),
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

broadcast_core::impl_spec_display!(AspectRatioIdc, Reserved);

/// A *specified* sample (pixel) aspect ratio: both terms non-zero. Unspecified
/// is `Option::None` at every use site, never a `0:0` sentinel.
#[non_exhaustive]
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

/// A rectangle in luma samples of the decoded picture, top-left origin.
/// `#[non_exhaustive]`: future fields may be added additively.
#[non_exhaustive]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PictureRect {
    left: u32,
    top: u32,
    width: u32,
    height: u32,
}

impl PictureRect {
    /// Build a rectangle from its four edges.
    pub fn new(left: u32, top: u32, width: u32, height: u32) -> Self {
        Self {
            left,
            top,
            width,
            height,
        }
    }

    /// Left offset in luma samples.
    pub fn left(&self) -> u32 {
        self.left
    }

    /// Top offset in luma samples.
    pub fn top(&self) -> u32 {
        self.top
    }

    /// Width in luma samples.
    pub fn width(&self) -> u32 {
        self.width
    }

    /// Height in luma samples.
    pub fn height(&self) -> u32 {
        self.height
    }
}

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

    /// `video_format` when the VUI omits the field: row 5 (unspecified).
    pub fn inferred() -> Self {
        Self::Unspecified
    }
}

broadcast_core::impl_spec_display!(VideoFormat, Reserved);

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
    pub fn from_wire(code: u8) -> Self {
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

broadcast_core::impl_spec_display!(ColourPrimaries, Reserved);

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
    pub fn from_wire(code: u8) -> Self {
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
            Self::Iec61966_2_1 => concat!(
                "IEC 61966-2-1 sRGB (with matrix_coefficients equal to 0)",
                " / IEC 61966-2-1 sYCC (with matrix_coefficients equal to 5)"
            ),
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

broadcast_core::impl_spec_display!(TransferCharacteristics, Reserved);

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
    pub fn from_wire(code: u8) -> Self {
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

broadcast_core::impl_spec_display!(MatrixCoefficients, Reserved);

/// `colour_description_present_flag == 1`: H.264 §E.2.1 Tables E-3..E-5 /
/// H.265 §E.3.1 Tables E.3..E.5.
#[non_exhaustive]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ColourDescription {
    primaries: ColourPrimaries,
    transfer: TransferCharacteristics,
    matrix: MatrixCoefficients,
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

    /// `colour_primaries`.
    pub fn primaries(&self) -> ColourPrimaries {
        self.primaries
    }

    /// `transfer_characteristics`.
    pub fn transfer(&self) -> TransferCharacteristics {
        self.transfer
    }

    /// `matrix_coefficients` (H.265: `matrix_coeffs`).
    pub fn matrix(&self) -> MatrixCoefficients {
        self.matrix
    }
}

/// `video_signal_type_present_flag == 1`: H.264 §E.2.1 / H.265 §E.3.1.
///
/// Only what was signalled is recorded, except that `video_format` has no
/// `None` state on the wire (the field is always the full `u(3)` when the
/// block is present). The spec's inferred values when the whole block is
/// absent are the consumer's, never in here.
#[non_exhaustive]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct VideoSignalType {
    video_format: VideoFormat,
    full_range: bool,
    colour: Option<ColourDescription>,
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

    /// `video_format`.
    pub fn video_format(&self) -> VideoFormat {
        self.video_format
    }

    /// `video_full_range_flag` (1 = full range).
    pub fn full_range(&self) -> bool {
        self.full_range
    }

    /// `None` when `colour_description_present_flag` is 0.
    pub fn colour(&self) -> Option<ColourDescription> {
        self.colour
    }
}

/// Everything that determines how one access unit is shown
/// (`docs/display-geometry/DESIGN.md` §4, Phase 2).
///
/// ETSI TS 101 154 Table B.5 / SMPTE ST 2016-1 Table 1 (SN §5), wire form
/// kept losslessly.
#[non_exhaustive]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ActiveFormatDescription {
    /// 0000: AFD unknown.
    Unknown,
    /// 0010: box 16:9 (top).
    Box16x9Top,
    /// 0011: box 14:9 (top).
    Box14x9Top,
    /// 0100: box > 16:9 (centre).
    BoxWiderThan16x9Centre,
    /// 1000: active format is the same as the coded frame.
    SameAsCodedFrame,
    /// 1001: 4:3 (centre).
    Centre4x3,
    /// 1010: 16:9 (centre).
    Centre16x9,
    /// 1011: 14:9 (centre).
    Centre14x9,
    /// 1101: 4:3 (with shoot and protect 14:9 centre).
    Centre4x3ShootProtect14x9,
    /// 1110: 16:9 (with shoot and protect 14:9 centre).
    Centre16x9ShootProtect14x9,
    /// 1111: 16:9 (with shoot and protect 4:3 centre).
    Centre16x9ShootProtect4x3,
    /// 0001, 0101..0111, 1100: reserved (wire value kept).
    Reserved(u8),
}

impl ActiveFormatDescription {
    /// The 4-bit wire code (lossless).
    pub fn code(&self) -> u8 {
        match self {
            Self::Unknown => 0b0000,
            Self::Box16x9Top => 0b0010,
            Self::Box14x9Top => 0b0011,
            Self::BoxWiderThan16x9Centre => 0b0100,
            Self::SameAsCodedFrame => 0b1000,
            Self::Centre4x3 => 0b1001,
            Self::Centre16x9 => 0b1010,
            Self::Centre14x9 => 0b1011,
            Self::Centre4x3ShootProtect14x9 => 0b1101,
            Self::Centre16x9ShootProtect14x9 => 0b1110,
            Self::Centre16x9ShootProtect4x3 => 0b1111,
            Self::Reserved(v) => *v,
        }
    }

    /// Build from the 4-bit wire code (lossless, every value maps).
    pub fn from_code(code: u8) -> Self {
        match code & AFD_CODE_MASK {
            0b0000 => Self::Unknown,
            0b0010 => Self::Box16x9Top,
            0b0011 => Self::Box14x9Top,
            0b0100 => Self::BoxWiderThan16x9Centre,
            0b1000 => Self::SameAsCodedFrame,
            0b1001 => Self::Centre4x3,
            0b1010 => Self::Centre16x9,
            0b1011 => Self::Centre14x9,
            0b1101 => Self::Centre4x3ShootProtect14x9,
            0b1110 => Self::Centre16x9ShootProtect14x9,
            0b1111 => Self::Centre16x9ShootProtect4x3,
            other => Self::Reserved(other),
        }
    }

    /// Label: the Table B.5 meaning, `"reserved"` for a reserved code.
    pub fn name(&self) -> &'static str {
        match self {
            Self::Unknown => "AFD unknown",
            Self::Box16x9Top => "box 16:9 (top)",
            Self::Box14x9Top => "box 14:9 (top)",
            Self::BoxWiderThan16x9Centre => "box > 16:9 (centre)",
            Self::SameAsCodedFrame => "same as coded frame",
            Self::Centre4x3 => "4:3 (centre)",
            Self::Centre16x9 => "16:9 (centre)",
            Self::Centre14x9 => "14:9 (centre)",
            Self::Centre4x3ShootProtect14x9 => "4:3 (with shoot and protect 14:9 centre)",
            Self::Centre16x9ShootProtect14x9 => "16:9 (with shoot and protect 14:9 centre)",
            Self::Centre16x9ShootProtect4x3 => "16:9 (with shoot and protect 4:3 centre)",
            Self::Reserved(_) => "reserved",
        }
    }
}
broadcast_core::impl_spec_display!(ActiveFormatDescription, Reserved);

/// TS 101 154 Table B.7. Only the signalled pair is `Some`; the 14-bit values
/// are kept as wire values.
#[non_exhaustive]
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BarData {
    top_bar_end_line: Option<u16>,
    bottom_bar_start_line: Option<u16>,
    left_bar_end_pixel: Option<u16>,
    right_bar_start_pixel: Option<u16>,
}

impl BarData {
    /// Build from the signalled values; each side is `None` when its flag is 0.
    pub fn new(
        top_bar_end_line: Option<u16>,
        bottom_bar_start_line: Option<u16>,
        left_bar_end_pixel: Option<u16>,
        right_bar_start_pixel: Option<u16>,
    ) -> Self {
        Self {
            top_bar_end_line,
            bottom_bar_start_line,
            left_bar_end_pixel,
            right_bar_start_pixel,
        }
    }

    /// `top_bar_end_line` (`u(14)`) when `top_bar_flag` is 1.
    pub fn top_bar_end_line(&self) -> Option<u16> {
        self.top_bar_end_line
    }

    /// `bottom_bar_start_line` (`u(14)`) when `bottom_bar_flag` is 1.
    pub fn bottom_bar_start_line(&self) -> Option<u16> {
        self.bottom_bar_start_line
    }

    /// `left_bar_end_pixel` (`u(14)`) when `left_bar_flag` is 1.
    pub fn left_bar_end_pixel(&self) -> Option<u16> {
        self.left_bar_end_pixel
    }

    /// `right_bar_start_pixel` (`u(14)`) when `right_bar_flag` is 1.
    pub fn right_bar_start_pixel(&self) -> Option<u16> {
        self.right_bar_start_pixel
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
    fn picture_rect_exposes_its_edges() {
        let r = PictureRect::new(1, 2, 3, 4);
        assert_eq!((r.left(), r.top(), r.width(), r.height()), (1, 2, 3, 4));
    }

    /// H.264 Table E-3 (H.265 E.3): defined rows -> primaries text.
    const SPEC_TABLE_E3: [(u8, &str); 12] = [
        (1, "Rec. ITU-R BT.709-6"),
        (2, "Unspecified"),
        (4, "Rec. ITU-R BT.470-7 System M (historical)"),
        (5, "Rec. ITU-R BT.470-7 System B, G (historical)"),
        (6, "Rec. ITU-R BT.601-7 525"),
        (7, "SMPTE 240M (1999, historical)"),
        (8, "Generic film (colour filters using Illuminant C)"),
        (9, "Rec. ITU-R BT.2020-2"),
        (10, "SMPTE ST 428-1 (2006)"),
        (11, "SMPTE RP 431-2 (2011)"),
        (12, "SMPTE EG 432-1 (2010)"),
        (22, "EBU Tech. 3213-E (1975)"),
    ];

    /// H.264 Table E-4 / H.265 E.4: every defined row.
    const SPEC_TABLE_E4_ROWS: [u8; 17] =
        [1, 2, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 17, 18];

    /// H.264 Table E-5 / H.265 E.5: every defined row 0..=17.
    const SPEC_TABLE_E5_ROWS: [u8; 17] =
        [0, 1, 2, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 17];

    /// A colour description from three wire code points.
    fn colour(primaries: u8, transfer: u8, matrix: u8) -> ColourDescription {
        ColourDescription::from_wire(primaries, transfer, matrix)
    }

    #[test]
    fn colour_primaries_match_table_e3_rows() {
        for (code, label) in SPEC_TABLE_E3 {
            let p = <ColourPrimaries>::from_wire(code);
            assert_eq!(p.code(), code);
            assert_eq!(p.name(), label, "colour_primaries {code}");
        }
    }

    #[test]
    fn transfer_matches_table_e4_defined_rows() {
        for code in SPEC_TABLE_E4_ROWS {
            let t = TransferCharacteristics::from_wire(code);
            assert_eq!(t.code(), code);
            assert_ne!(t.name(), "reserved", "transfer {code} must be defined");
        }
        // The two HDR transfers a UHD player must not miss.
        assert_eq!(
            TransferCharacteristics::from_wire(16),
            TransferCharacteristics::SmpteSt2084
        );
        assert_eq!(
            TransferCharacteristics::from_wire(18),
            TransferCharacteristics::AribStdB67
        );
    }

    #[test]
    fn matrix_coefficients_match_table_e5_defined_rows() {
        for code in SPEC_TABLE_E5_ROWS {
            let m = MatrixCoefficients::from_wire(code);
            assert_eq!(m.code(), code);
            assert_ne!(m.name(), "reserved", "matrix {code} must be defined");
        }
        // The classic Y'CbCr matrices a player must tell apart.
        assert_eq!(
            MatrixCoefficients::from_wire(5),
            MatrixCoefficients::Bt470SystemB
        );
        assert_eq!(
            MatrixCoefficients::from_wire(6),
            MatrixCoefficients::Bt470SystemM
        );
        // BT.709 vs BT.601 must render differently.
        assert_ne!(
            MatrixCoefficients::from_wire(1).name(),
            MatrixCoefficients::from_wire(6).name()
        );
    }

    /// H.264 Table E-2 0..=7; 6 and 7 are reserved in both specs.
    #[test]
    fn video_format_matches_table_e2() {
        const MEANINGS: [&str; 8] = [
            "Component",
            "PAL",
            "NTSC",
            "SECAM",
            "MAC",
            "Unspecified video format",
            "reserved",
            "reserved",
        ];
        for (code, label) in (0u8..).zip(MEANINGS) {
            let f = VideoFormat::from_wire(code).expect("u(3) value");
            assert_eq!(f.code(), code);
            assert_eq!(f.name(), label);
        }
        // The field is u(3): nothing wider can come off the wire.
        assert_eq!(VideoFormat::from_wire(8), None);
        assert_eq!(VideoFormat::from_wire(255), None);
    }

    #[test]
    fn every_colour_code_round_trips_and_has_a_label() {
        for code in 0..=u8::MAX {
            let p = ColourPrimaries::from_wire(code);
            let t = TransferCharacteristics::from_wire(code);
            let m = MatrixCoefficients::from_wire(code);
            assert_eq!((p.code(), t.code(), m.code()), (code, code, code));
            for label in [p.name(), t.name(), m.name()] {
                assert!(!label.is_empty());
            }
            let d = colour(code, code, code);
            assert_eq!(d.primaries().code(), code);
            assert_eq!(d.transfer().code(), code);
            assert_eq!(d.matrix().code(), code);
        }
        // Reserved rows keep their code and say so.
        assert_eq!(ColourPrimaries::from_wire(0), ColourPrimaries::Reserved(0));
        assert_eq!(
            TransferCharacteristics::from_wire(19),
            TransferCharacteristics::Reserved(19)
        );
        assert_eq!(
            MatrixCoefficients::from_wire(3),
            MatrixCoefficients::Reserved(3)
        );
        assert_eq!(ColourPrimaries::Reserved(255).code(), 255u8);
    }

    #[test]
    fn video_signal_type_exposes_what_was_signalled() {
        let none = VideoSignalType::new(VideoFormat::Component, true, None);
        assert_eq!(none.video_format(), VideoFormat::Component);
        assert!(none.full_range());
        assert_eq!(none.colour(), None);
        let described = VideoSignalType::new(
            VideoFormat::Pal,
            false,
            Some(ColourDescription::from_wire(9, 16, 9)),
        );
        let d = described.colour().expect("colour description present");
        assert_eq!(d.primaries(), ColourPrimaries::Bt2020);
        assert_eq!(d.transfer(), TransferCharacteristics::SmpteSt2084);
        assert_eq!(d.matrix(), MatrixCoefficients::Bt2020NonConstant);
    }

    /// ETSI TS 101 154 Table B.5: every 4-bit code maps, reserved ones kept.
    #[test]
    fn afd_codes_round_trip_and_reserved_ones_are_kept() {
        for code in 0u8..=0x0F {
            let afd = ActiveFormatDescription::from_code(code);
            assert_eq!(afd.code(), code, "AFD {code}");
            assert!(!afd.name().is_empty());
        }
        assert_eq!(
            ActiveFormatDescription::from_code(0b1010),
            ActiveFormatDescription::Centre16x9
        );
        assert_eq!(
            ActiveFormatDescription::from_code(0b0001),
            ActiveFormatDescription::Reserved(0b0001)
        );
        // A byte whose low four bits are the code: the high bits are ignored.
        assert_eq!(
            ActiveFormatDescription::from_code(0xF1),
            ActiveFormatDescription::Reserved(0b0001)
        );
    }

    #[test]
    fn bar_data_exposes_only_the_signalled_side() {
        let bars = BarData::new(Some(1), None, Some(3), None);
        assert_eq!(bars.top_bar_end_line(), Some(1));
        assert_eq!(bars.bottom_bar_start_line(), None);
        assert_eq!(bars.left_bar_end_pixel(), Some(3));
        assert_eq!(bars.right_bar_start_pixel(), None);
    }
}
