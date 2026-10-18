//! Display geometry of a video track: what a player needs to show each
//! picture at its correct shape.
//!
//! Phase 1a of `docs/display-geometry/DESIGN.md`: the **sample aspect ratio**
//! (SAR) signalled in the SPS VUI plus the coded picture size, derived once
//! per track into [`TrackSpec::display`](super::TrackSpec::display). Cropping,
//! colour description, a change event and per-sample tags are later phases
//! (every type here is `#[non_exhaustive]`, so they land additively).
//!
//! # Spec citations
//!
//! - **H.264**: ITU-T H.264 (V15, 08/2024) §E.1.1 (`vui_parameters()` syntax)
//!   and §E.2.1 Table E-1 (meaning of `aspect_ratio_idc`).
//! - **H.265**: ITU-T H.265 (V11, 01/2026) §E.2.1 (syntax) and §E.3.1
//!   Table E.1 (same values as Table E-1).
//!
//! Both tables were re-verified against the rendered PDF pages (see
//! `docs/display-geometry/SPEC-NOTES.md`), not only the markdown text, whose
//! Table E-1 rows are merged.
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

/// Sequence-level display geometry of a video track: everything derived from
/// the active SPS (in a conforming stream it changes only at an IDR / IRAP,
/// H.264 §7.4.1.2.1 / H.265 §7.4.3.2). Lives on
/// [`TrackSpec::display`](super::TrackSpec::display).
///
/// Phase 1a carries the coded size and the VUI aspect ratio; cropping, colour
/// description and the default display window are additive follow-ups.
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
}

impl DisplayGeometry {
    /// Build from explicit values (for muxers and tests; demuxed tracks get
    /// theirs from the SPS).
    pub fn new(coded_width: u32, coded_height: u32, aspect_ratio: Option<AspectRatioIdc>) -> Self {
        Self {
            coded_width,
            coded_height,
            aspect_ratio,
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
