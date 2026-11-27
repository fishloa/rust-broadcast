//! Image item properties and derived image items — ISO/IEC 23008-12:2025
//! (HEIF, third edition) clause 6 as Amendment 1 applies, the AV1 Image File
//! Format (AVIF) v1.2.0, and the item properties ISO/IEC 14496-12:2026 §8.11
//! itself defines.
//!
//! | Type | Four-CC | Source |
//! |------|---------|--------|
//! | [`ImageSpatialExtentsProperty`] | `ispe` | 23008-12 6.5.3 |
//! | [`PixelAspectRatioBox`] | `pasp` | 23008-12 6.5.4 (same type as §12.1.4) |
//! | [`ColourInformationBox`] | `colr` | 23008-12 6.5.5 (same type) |
//! | [`PixelInformationProperty`] | `pixi` | 23008-12 6.5.6 |
//! | [`RelativeLocationProperty`] | `rloc` | 23008-12 6.5.7 |
//! | [`AuxiliaryTypeProperty`] | `auxC` | 23008-12 6.5.8, spelling per Cor.1 |
//! | [`CleanApertureBox`] | `clap` | 23008-12 6.5.9 (same type) |
//! | [`ImageRotation`] | `irot` | 23008-12 6.5.10 |
//! | [`LayerSelectorProperty`] | `lsel` | 23008-12 6.5.11 |
//! | [`ImageMirror`] | `imir` | 23008-12 6.5.12 |
//! | [`HEVCConfigurationBox`] | `hvcC` | 23008-12 B.2 (reused) |
//! | [`AVCConfigurationBox`] | `avcC` | 23008-12 E.2 (reused) |
//! | [`Av1ConfigurationBox`] | `av1C` | AVIF 2.2.1 (reused) |
//! | [`OperatingPointSelectorProperty`] | `a1op` | AVIF 2.3.2.1 |
//! | [`Av1LayeredImageIndexingProperty`] | `a1lx` | AVIF 2.3.2.3 |
//! | [`MasteringDisplayColourVolume`] | `mdcv` | ISO/IEC 14496-12:2026 §12.1.7 (reused) |
//! | [`ContentLightLevel`] | `clli` | ISO/IEC 14496-12:2026 §12.1.6 (reused) |
//! | [`BrandProperty`] | `brnd` | ISO/IEC 14496-12:2026 §8.11.15 |
//! | [`HandlerProperty`] | `hdlp` | ISO/IEC 14496-12:2026 §8.11.16 |
//! | [`GainMapMetadata`] | `tmap` body | Amd 1 6.6.2.4.2 |
//!
//! The gain map metadata layout comes from the open implementations that agree
//! on it because ISO 21496-1 is not held; the private note
//! `private/specs-verified/misc/gain-map/SOURCES.md` records the four sources,
//! where they agree and the one view adopted here.
//!
//! The derived image item bodies are [`IdentityDerivation`] (`iden`),
//! [`ImageOverlay`] (`iovl`) and [`ImageGrid`] (`grid`)
//! (ISO/IEC 23008-12:2025 6.6.2). The reader and writer are
//! [`crate::item_reader`] and [`crate::item_writer`].
//!
//! Sources: `private/specs-verified/mpeg/heif/iso-iec-23008-12-2025.md`
//! clause 6 and `private/specs-verified/mpeg/heif/iso-iec-23008-12-2025-amd1-2025.md`
//! (the third edition keeps every 2017 sub-clause this module types; the
//! 2017 text is history only), and
//! `private/specs/aom_av1_avif_image_file_format.html` (AVIF v1.2.0). Types
//! whose syntax another clause of the base format already defines are that
//! type here too, never a second parser.

use alloc::vec::Vec;

use broadcast_core::len::{fit_u8, need_in, need_out};
use broadcast_core::wire::{be_u16, be_u32, i32_at, u24_at, u32_at};
use broadcast_core::{Parse, Serialize};

use crate::av1::Av1ConfigurationBox;
use crate::avc_config::AVCConfigurationBox;
use crate::box_support::OpaqueBox;
use crate::box_types::{
    BOX_HEADER_MIN_SIZE, BOX_TYPE_SIZE, FULLBOX_EXTRA_SIZE, FULLBOX_FLAGS_MASK, box_len,
    write_box_header, write_full_box_header,
};
use crate::error::{Error, Result};
use crate::hdr_boxes::{
    CLLI_FOURCC, CLLI_PAYLOAD_LEN, ContentLightLevel, MDCV_FOURCC, MDCV_PAYLOAD_LEN,
    MasteringDisplayColourVolume,
};
use crate::hevc_config::HEVCConfigurationBox;
use crate::image_derived::{ColourFormatEnhancement, ToneMapImage};
use crate::image_props::{
    AccessibilityTextProperty, AutoExposureProperty, CameraExtrinsicMatrixProperty,
    CameraIntrinsicMatrixProperty, ConstrainedExtentsGridProperty, CreationTimeProperty,
    DepthOfFieldProperty, DisparityAdjustmentProperty, FadeTransitionEffectProperty,
    FlashExposureProperty, FocusProperty, ImageScaling, MaskConfigurationProperty,
    ModificationTimeProperty, NominalDiffuseWhiteProperty, PanoramaProperty,
    ProgressiveDerivedImageItemInformationProperty, ReferenceViewingEnvironmentProperty,
    RequiredReferenceTypesProperty, SingleStreamProperty, SplitTransitionEffectProperty,
    StereoAggressorsProperty, SuggestedTimeDisplayDurationProperty,
    SuggestedTransitionPeriodProperty, TargetOutputLayerSetProperty, UserDescriptionProperty,
    WhiteBalanceProperty, WipeTransitionEffectProperty, ZoomTransitionEffectProperty,
};
use crate::item::{BrandProperty, HandlerProperty};
use crate::visual_ext::{
    AMVE, AmbientViewingEnvironmentBox, CCLV, CleanApertureBox, ColourInformationBox,
    ContentColourVolumeBox, PixelAspectRatioBox,
};

/// Four-CC of the Image Spatial Extents property (23008-12 6.5.3).
pub const ISPE: [u8; BOX_TYPE_SIZE] = *b"ispe";
/// Four-CC of the pixel aspect ratio property (23008-12 6.5.4).
pub const PASP: [u8; BOX_TYPE_SIZE] = *b"pasp";
/// Four-CC of the colour information property (23008-12 6.5.5).
pub const COLR: [u8; BOX_TYPE_SIZE] = *b"colr";
/// Four-CC of the Pixel Information property (23008-12 6.5.6).
pub const PIXI: [u8; BOX_TYPE_SIZE] = *b"pixi";
/// Four-CC of the Relative Location property (23008-12 6.5.7).
pub const RLOC: [u8; BOX_TYPE_SIZE] = *b"rloc";
/// Four-CC of the Auxiliary Type property (23008-12 6.5.8, as corrected).
pub const AUXC: [u8; BOX_TYPE_SIZE] = *b"auxC";
/// Four-CC of the clean aperture property (23008-12 6.5.9).
pub const CLAP: [u8; BOX_TYPE_SIZE] = *b"clap";
/// Four-CC of the Image Rotation property (23008-12 6.5.10).
pub const IROT: [u8; BOX_TYPE_SIZE] = *b"irot";
/// Four-CC of the Layer Selector property (23008-12 6.5.11).
pub const LSEL: [u8; BOX_TYPE_SIZE] = *b"lsel";
/// Four-CC of the Image Mirror property (23008-12 6.5.12).
pub const IMIR: [u8; BOX_TYPE_SIZE] = *b"imir";
/// Four-CC of the HEVC configuration property (23008-12 B.2).
pub const HVCC: [u8; BOX_TYPE_SIZE] = *b"hvcC";
/// Four-CC of the AVC configuration property (23008-12 E.2).
pub const AVCC: [u8; BOX_TYPE_SIZE] = *b"avcC";
/// Four-CC of the AV1 configuration property (AVIF 2.2.1).
pub const AV1C: [u8; BOX_TYPE_SIZE] = *b"av1C";
/// Four-CC of the Operating Point Selector property (AVIF 2.3.2.1).
pub const A1OP: [u8; BOX_TYPE_SIZE] = *b"a1op";
/// Four-CC of the AV1 Layered Image Indexing property (AVIF 2.3.2.3).
pub const A1LX: [u8; BOX_TYPE_SIZE] = *b"a1lx";

/// Degrees one `irot` step rotates the image (23008-12 6.5.10.1).
const DEGREES_PER_ROTATION_STEP: u16 = 90;
/// Bits of `irot`'s `angle`.
const IROT_ANGLE_BITS: u32 = 2;
/// Bits of `imir`'s `axis`.
const IMIR_AXIS_BITS: u32 = 1;
/// Bits of `a1lx`'s `large_size`.
const A1LX_LARGE_BITS: u32 = 1;
/// Layers an `a1lx` property documents (all but the last).
const A1LX_LAYER_COUNT: usize = 3;
/// Width of one `a1lx` size when `large_size` is 0.
const A1LX_SMALL_SIZE_LEN: usize = core::mem::size_of::<u16>();
/// Width of one `a1lx` size when `large_size` is 1.
const A1LX_LARGE_SIZE_LEN: usize = core::mem::size_of::<u32>();
/// Channels in an overlay canvas fill value (R, G, B, A; 6.6.2.2.3).
const OVERLAY_FILL_CHANNELS: usize = 4;
/// Bytes of `version` + `flags` before an `iovl` fill value (6.6.2.2.2).
const OVERLAY_PREFIX_LEN: usize = 2;
/// Width of one overlay canvas fill channel (6.6.2.2.2).
const OVERLAY_CHANNEL_LEN: usize = 2;
/// Offsets (horizontal and vertical) one overlay input carries.
const OVERLAY_OFFSETS_PER_INPUT: usize = 2;
/// Dimensions (width and height) an overlay canvas states.
const OVERLAY_DIMENSION_COUNT: usize = 2;
/// Bytes of the `iovl` fixed prefix before its offset pairs (6.6.2.2.2).
const OVERLAY_FIXED_LEN: usize = OVERLAY_PREFIX_LEN + OVERLAY_FILL_CHANNELS * OVERLAY_CHANNEL_LEN;
/// Bytes of the `grid` fixed prefix (6.6.2.3.2).
const GRID_FIXED_LEN: usize = 4;
/// Offset of `rows_minus_one` in a grid body (6.6.2.3.2).
const GRID_ROWS_OFFSET: usize = 2;
/// Offset of `columns_minus_one` in a grid body (6.6.2.3.2).
const GRID_COLUMNS_OFFSET: usize = 3;
/// Dimensions (width and height) a grid body states (6.6.2.3.2).
const GRID_DIMENSION_COUNT: usize = 2;
/// Width of a derived-image dimension/offset field when `flags & 1` is 0.
const DERIVED_SMALL_FIELD_LEN: usize = 2;
/// Width of a derived-image dimension/offset field when `flags & 1` is 1.
const DERIVED_LARGE_FIELD_LEN: usize = 4;
/// Version every derived image body requires (6.6.2).
const DERIVED_VERSION: u8 = 0;
/// The `(flags & 1)` bit that widens the derived body's dimension fields.
const DERIVED_WIDE_FIELDS: u8 = 1;
/// Channel index of the overlay canvas alpha value.
const OVERLAY_ALPHA_INDEX: usize = 3;
/// Dimensions an `ispe` property carries.
const ISPE_DIM_COUNT: usize = 2;
/// Width of an `ispe` dimension.
const ISPE_DIM_LEN: usize = core::mem::size_of::<u32>();
/// Offsets an `rloc` property carries.
const RLOC_OFFSET_COUNT: usize = 2;
/// Width of an `rloc` offset.
const RLOC_OFFSET_LEN: usize = core::mem::size_of::<u32>();
/// Width of an `a1op` `op_index`.
const A1OP_LEN: usize = 1;
/// Width of an `lsel` `layer_id`.
const LSEL_LEN: usize = core::mem::size_of::<u16>();
/// Width of one rational component of the gain map metadata.
const GAIN_MAP_COMPONENT_LEN: usize = core::mem::size_of::<u32>();
/// Bytes of `minimum_version` and `writer_version`.
const GAIN_MAP_VERSION_LEN: usize = 2 * core::mem::size_of::<u16>();
/// Offset of `minimum_version`.
const GAIN_MAP_MINIMUM_OFFSET: usize = 0;
/// Offset of `writer_version`.
const GAIN_MAP_WRITER_OFFSET: usize = GAIN_MAP_MINIMUM_OFFSET + core::mem::size_of::<u16>();
/// Offset of the gain map flags byte, after both versions.
const GAIN_MAP_FLAGS_OFFSET: usize = GAIN_MAP_VERSION_LEN;
/// Offset of `base_hdr_headroom_numerator`.
const GAIN_MAP_BASE_HEADROOM_OFFSET: usize = GAIN_MAP_FLAGS_OFFSET + 1;
/// Offset of `alternate_hdr_headroom_numerator`.
const GAIN_MAP_ALTERNATE_HEADROOM_OFFSET: usize =
    GAIN_MAP_BASE_HEADROOM_OFFSET + 2 * GAIN_MAP_COMPONENT_LEN;
/// Bytes the gain map metadata spends before its per-channel values.
const GAIN_MAP_FIXED_LEN: usize = GAIN_MAP_ALTERNATE_HEADROOM_OFFSET + 2 * GAIN_MAP_COMPONENT_LEN;
/// Bytes one channel of gain map metadata spends (five rationals).
const GAIN_MAP_CHANNEL_LEN: usize = 5 * 2 * GAIN_MAP_COMPONENT_LEN;
/// The `minimum_version` this view implements.
const GAIN_MAP_MINIMUM_VERSION: u16 = 0;
/// The flag bit selecting three channels rather than one.
const GAIN_MAP_MULTICHANNEL_FLAG: u8 = 0x80;
/// The flag bit saying the gain map is applied in the base colour space.
const GAIN_MAP_BASE_COLOUR_SPACE_FLAG: u8 = 0x40;
/// The flag bits this view treats as reserved.
const GAIN_MAP_RESERVED_FLAGS_MASK: u8 = 0x3f;
/// Channels a multichannel gain map carries.
const GAIN_MAP_MULTICHANNEL_COUNT: usize = 3;
/// Channels a single-channel gain map carries.
const GAIN_MAP_SINGLE_CHANNEL_COUNT: usize = 1;

/// The `aux_type` URN of an alpha auxiliary image (AVIF 4.1).
pub const AUX_TYPE_ALPHA: &str = "urn:mpeg:mpegB:cicp:systems:auxiliary:alpha";
/// The `aux_type` URN of a depth auxiliary image (AVIF 4.1).
pub const AUX_TYPE_DEPTH: &str = "urn:mpeg:mpegB:cicp:systems:auxiliary:depth";
/// The `aux_type` URN of an HEVC alpha plane (ISO/IEC 23008-12:2025 B.2.3.3).
pub const AUX_TYPE_HEVC_ALPHA: &str = "urn:mpeg:hevc:2015:auxid:1";
/// The `aux_type` URN of an HEVC depth image (ISO/IEC 23008-12:2025 B.2.3.3).
pub const AUX_TYPE_HEVC_DEPTH: &str = "urn:mpeg:hevc:2015:auxid:2";

/// The `mif1` structural brand (23008-12 10.2.1).
pub const BRAND_MIF1: [u8; BOX_TYPE_SIZE] = *b"mif1";
/// The `msf1` image-sequence structural brand (23008-12 10.3.1).
pub const BRAND_MSF1: [u8; BOX_TYPE_SIZE] = *b"msf1";
/// The HEVC image brand (23008-12 C).
pub const BRAND_HEIC: [u8; BOX_TYPE_SIZE] = *b"heic";
/// The AVIF image brand (AVIF 6.2).
pub const BRAND_AVIF: [u8; BOX_TYPE_SIZE] = *b"avif";
/// The AVIF intra-only brand (AVIF 6.2).
pub const BRAND_AVIO: [u8; BOX_TYPE_SIZE] = *b"avio";
/// The AVIF image-sequence brand (AVIF 6.3).
pub const BRAND_AVIS: [u8; BOX_TYPE_SIZE] = *b"avis";
/// The AVIF Baseline profile brand (AVIF 8.2).
pub const BRAND_MA1B: [u8; BOX_TYPE_SIZE] = *b"MA1B";
/// The AVIF Advanced profile brand (AVIF 8.3).
pub const BRAND_MA1A: [u8; BOX_TYPE_SIZE] = *b"MA1A";

/// Whether `brand` names a still-image file this module models an item layer
/// for (the `mif1` family and both AVIF item brands).
#[must_use]
pub fn is_still_image_brand(brand: [u8; BOX_TYPE_SIZE]) -> bool {
    matches!(
        &brand,
        b"mif1" | b"msf1" | b"heic" | b"heix" | b"heim" | b"heis" | b"avif" | b"avis"
    )
}

pub(crate) fn body_of<'a>(
    bytes: &'a [u8],
    min_body: usize,
    what: &'static str,
) -> Result<&'a [u8]> {
    crate::box_types::box_body(bytes, min_body, what)
}

pub(crate) fn expect_type(
    bytes: &[u8],
    fourcc: &[u8; BOX_TYPE_SIZE],
    what: &'static str,
) -> Result<()> {
    need_in(bytes, BOX_HEADER_MIN_SIZE, what)?;
    if broadcast_core::wire::u32_at(bytes, crate::box_types::BOX_TYPE_OFFSET)
        != u32::from_be_bytes(*fourcc)
    {
        return Err(Error::InvalidInput(what));
    }
    Ok(())
}

pub(crate) fn full_flags(body: &[u8]) -> u32 {
    u24_at(body, crate::box_types::FULLBOX_FLAGS_OFFSET) & FULLBOX_FLAGS_MASK
}

/// Image Spatial Extents property (`ispe`) — ISO/IEC 23008-12:2025 6.5.3.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub struct ImageSpatialExtentsProperty {
    version: u8,
    flags: u32,
    image_width: u32,
    image_height: u32,
}

impl ImageSpatialExtentsProperty {
    /// A version-0 property of `width` x `height` pixels.
    #[must_use]
    pub const fn new(image_width: u32, image_height: u32) -> Self {
        Self {
            version: 0,
            flags: 0,
            image_width,
            image_height,
        }
    }

    /// The FullBox version.
    #[must_use]
    pub const fn version(&self) -> u8 {
        self.version
    }

    /// The FullBox flags.
    #[must_use]
    pub const fn flags(&self) -> u32 {
        self.flags
    }

    /// `image_width`.
    #[must_use]
    pub const fn image_width(&self) -> u32 {
        self.image_width
    }

    /// `image_height`.
    #[must_use]
    pub const fn image_height(&self) -> u32 {
        self.image_height
    }
}

impl<'a> Parse<'a> for ImageSpatialExtentsProperty {
    type Error = Error;

    fn parse(bytes: &'a [u8]) -> Result<Self> {
        expect_type(bytes, &ISPE, "expected ispe")?;
        let body = body_of(
            bytes,
            FULLBOX_EXTRA_SIZE + ISPE_DIM_LEN * ISPE_DIM_COUNT,
            "ispe",
        )?;
        Ok(Self {
            version: body[0],
            flags: full_flags(body),
            image_width: be_u32(body, FULLBOX_EXTRA_SIZE, "ispe image_width")?,
            image_height: be_u32(body, FULLBOX_EXTRA_SIZE + ISPE_DIM_LEN, "ispe image_height")?,
        })
    }
}

impl Serialize for ImageSpatialExtentsProperty {
    type Error = Error;

    fn serialized_len(&self) -> usize {
        box_len(BOX_HEADER_MIN_SIZE + FULLBOX_EXTRA_SIZE + ISPE_DIM_LEN * ISPE_DIM_COUNT)
    }

    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        let need = self.serialized_len();
        need_out(buf, need)?;
        let mut cursor = write_full_box_header(buf, &ISPE, need, self.version, self.flags, false)?;
        buf[cursor..cursor + ISPE_DIM_LEN].copy_from_slice(&self.image_width.to_be_bytes());
        cursor += ISPE_DIM_LEN;
        buf[cursor..cursor + ISPE_DIM_LEN].copy_from_slice(&self.image_height.to_be_bytes());
        cursor += ISPE_DIM_LEN;
        Ok(cursor)
    }
}

/// Pixel Information property (`pixi`) — ISO/IEC 23008-12:2025 6.5.6.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub struct PixelInformationProperty {
    version: u8,
    flags: u32,
    bits_per_channel: Vec<u8>,
}

impl PixelInformationProperty {
    /// A version-0 property carrying one bit depth per channel.
    #[must_use]
    pub fn new(bits_per_channel: Vec<u8>) -> Self {
        Self {
            version: 0,
            flags: 0,
            bits_per_channel,
        }
    }

    /// The FullBox version.
    #[must_use]
    pub const fn version(&self) -> u8 {
        self.version
    }

    /// The FullBox flags.
    #[must_use]
    pub const fn flags(&self) -> u32 {
        self.flags
    }

    /// `num_channels`.
    #[must_use]
    pub fn num_channels(&self) -> usize {
        self.bits_per_channel.len()
    }

    /// `bits_per_channel`, one per channel.
    #[must_use]
    pub fn bits_per_channel(&self) -> &[u8] {
        &self.bits_per_channel
    }
}

impl<'a> Parse<'a> for PixelInformationProperty {
    type Error = Error;

    fn parse(bytes: &'a [u8]) -> Result<Self> {
        expect_type(bytes, &PIXI, "expected pixi")?;
        let body = body_of(bytes, FULLBOX_EXTRA_SIZE + 1, "pixi")?;
        let count = usize::from(body[FULLBOX_EXTRA_SIZE]);
        let start = FULLBOX_EXTRA_SIZE + 1;
        crate::box_support::check_entry_count(body.len() - start, 1, count, "pixi num_channels")?;
        Ok(Self {
            version: body[0],
            flags: full_flags(body),
            bits_per_channel: body[start..start + count].to_vec(),
        })
    }
}

impl Serialize for PixelInformationProperty {
    type Error = Error;

    fn serialized_len(&self) -> usize {
        box_len(BOX_HEADER_MIN_SIZE + FULLBOX_EXTRA_SIZE + 1 + self.bits_per_channel.len())
    }

    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        let need = self.serialized_len();
        need_out(buf, need)?;
        let mut cursor = write_full_box_header(buf, &PIXI, need, self.version, self.flags, false)?;
        buf[cursor] = fit_u8(self.bits_per_channel.len(), "pixi num_channels")?;
        cursor += 1;
        buf[cursor..cursor + self.bits_per_channel.len()].copy_from_slice(&self.bits_per_channel);
        cursor += self.bits_per_channel.len();
        Ok(cursor)
    }
}

/// Relative Location property (`rloc`) — ISO/IEC 23008-12:2025 6.5.7.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub struct RelativeLocationProperty {
    version: u8,
    flags: u32,
    horizontal_offset: u32,
    vertical_offset: u32,
}

impl RelativeLocationProperty {
    /// A version-0 property at the given offsets.
    #[must_use]
    pub const fn new(horizontal_offset: u32, vertical_offset: u32) -> Self {
        Self {
            version: 0,
            flags: 0,
            horizontal_offset,
            vertical_offset,
        }
    }

    /// The FullBox version.
    #[must_use]
    pub const fn version(&self) -> u8 {
        self.version
    }

    /// The FullBox flags.
    #[must_use]
    pub const fn flags(&self) -> u32 {
        self.flags
    }

    /// `horizontal_offset`.
    #[must_use]
    pub const fn horizontal_offset(&self) -> u32 {
        self.horizontal_offset
    }

    /// `vertical_offset`.
    #[must_use]
    pub const fn vertical_offset(&self) -> u32 {
        self.vertical_offset
    }
}

impl<'a> Parse<'a> for RelativeLocationProperty {
    type Error = Error;

    fn parse(bytes: &'a [u8]) -> Result<Self> {
        expect_type(bytes, &RLOC, "expected rloc")?;
        let body = body_of(
            bytes,
            FULLBOX_EXTRA_SIZE + RLOC_OFFSET_LEN * RLOC_OFFSET_COUNT,
            "rloc",
        )?;
        Ok(Self {
            version: body[0],
            flags: full_flags(body),
            horizontal_offset: be_u32(body, FULLBOX_EXTRA_SIZE, "rloc horizontal_offset")?,
            vertical_offset: be_u32(
                body,
                FULLBOX_EXTRA_SIZE + RLOC_OFFSET_LEN,
                "rloc vertical_offset",
            )?,
        })
    }
}

impl Serialize for RelativeLocationProperty {
    type Error = Error;

    fn serialized_len(&self) -> usize {
        box_len(BOX_HEADER_MIN_SIZE + FULLBOX_EXTRA_SIZE + RLOC_OFFSET_LEN * RLOC_OFFSET_COUNT)
    }

    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        let need = self.serialized_len();
        need_out(buf, need)?;
        let mut cursor = write_full_box_header(buf, &RLOC, need, self.version, self.flags, false)?;
        buf[cursor..cursor + RLOC_OFFSET_LEN]
            .copy_from_slice(&self.horizontal_offset.to_be_bytes());
        cursor += RLOC_OFFSET_LEN;
        buf[cursor..cursor + RLOC_OFFSET_LEN].copy_from_slice(&self.vertical_offset.to_be_bytes());
        cursor += RLOC_OFFSET_LEN;
        Ok(cursor)
    }
}

/// Auxiliary Type property (`auxC`) — ISO/IEC 23008-12:2025 6.5.8.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub struct AuxiliaryTypeProperty {
    version: u8,
    flags: u32,
    aux_type: Vec<u8>,
    aux_subtype: Vec<u8>,
}

impl AuxiliaryTypeProperty {
    /// A property naming `aux_type` (a URN) with no subtype bytes.
    #[must_use]
    pub fn new(aux_type: &[u8]) -> Self {
        Self {
            version: 0,
            flags: 0,
            aux_type: aux_type.to_vec(),
            aux_subtype: Vec::new(),
        }
    }

    /// Keep `aux_subtype` bytes after the URN.
    #[must_use]
    pub fn with_aux_subtype(mut self, aux_subtype: Vec<u8>) -> Self {
        self.aux_subtype = aux_subtype;
        self
    }

    /// The FullBox version.
    #[must_use]
    pub const fn version(&self) -> u8 {
        self.version
    }

    /// The FullBox flags.
    #[must_use]
    pub const fn flags(&self) -> u32 {
        self.flags
    }

    /// The `aux_type` URN bytes (terminator excluded).
    #[must_use]
    pub fn aux_type(&self) -> &[u8] {
        &self.aux_type
    }

    /// The `aux_type` URN as text, when it is UTF-8.
    #[must_use]
    pub fn aux_type_str(&self) -> Option<&str> {
        core::str::from_utf8(&self.aux_type).ok()
    }

    /// The `aux_subtype` bytes.
    #[must_use]
    pub fn aux_subtype(&self) -> &[u8] {
        &self.aux_subtype
    }

    /// Whether this auxiliary image is an alpha plane.
    #[must_use]
    pub fn is_alpha(&self) -> bool {
        matches!(
            self.aux_type_str(),
            Some(AUX_TYPE_ALPHA | AUX_TYPE_HEVC_ALPHA)
        )
    }

    /// Whether this auxiliary image is a depth map.
    #[must_use]
    pub fn is_depth(&self) -> bool {
        matches!(
            self.aux_type_str(),
            Some(AUX_TYPE_DEPTH | AUX_TYPE_HEVC_DEPTH)
        )
    }
}

impl<'a> Parse<'a> for AuxiliaryTypeProperty {
    type Error = Error;

    fn parse(bytes: &'a [u8]) -> Result<Self> {
        expect_type(bytes, &AUXC, "expected auxC")?;
        let mut body = body_of(bytes, FULLBOX_EXTRA_SIZE + 1, "auxC")?;
        let version = body[0];
        let flags = full_flags(body);
        body = &body[FULLBOX_EXTRA_SIZE..];
        let mut cursor = 0;
        let aux_type = crate::box_support::take_c_string(body, &mut cursor, "auxC aux_type")?;
        Ok(Self {
            version,
            flags,
            aux_type,
            aux_subtype: body[cursor..].to_vec(),
        })
    }
}

impl Serialize for AuxiliaryTypeProperty {
    type Error = Error;

    fn serialized_len(&self) -> usize {
        box_len(
            BOX_HEADER_MIN_SIZE
                + FULLBOX_EXTRA_SIZE
                + self.aux_type.len()
                + 1
                + self.aux_subtype.len(),
        )
    }

    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        let need = self.serialized_len();
        need_out(buf, need)?;
        let mut cursor = write_full_box_header(buf, &AUXC, need, self.version, self.flags, false)?;
        cursor += crate::box_support::write_c_string(buf, cursor, &self.aux_type)?;
        buf[cursor..cursor + self.aux_subtype.len()].copy_from_slice(&self.aux_subtype);
        cursor += self.aux_subtype.len();
        Ok(cursor)
    }
}

/// Image Rotation property (`irot`) — ISO/IEC 23008-12:2025 6.5.10.
///
/// The reconstructed image is rotated anti-clockwise by `angle * 90` degrees.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub struct ImageRotation {
    angle: u8,
}

impl ImageRotation {
    /// A rotation of `angle * 90` degrees anti-clockwise (0..=3).
    #[must_use]
    pub const fn new(angle: u8) -> Self {
        Self { angle }
    }

    /// `angle` (0..=3).
    #[must_use]
    pub const fn angle(&self) -> u8 {
        self.angle
    }

    /// The angle in degrees.
    #[must_use]
    pub const fn degrees(&self) -> u16 {
        (self.angle as u16) * DEGREES_PER_ROTATION_STEP
    }
}

impl<'a> Parse<'a> for ImageRotation {
    type Error = Error;

    fn parse(bytes: &'a [u8]) -> Result<Self> {
        expect_type(bytes, &IROT, "expected irot")?;
        let body = body_of(bytes, 1, "irot")?;
        Ok(Self {
            angle: body[0] & ((1u8 << IROT_ANGLE_BITS) - 1),
        })
    }
}

impl Serialize for ImageRotation {
    type Error = Error;

    fn serialized_len(&self) -> usize {
        box_len(BOX_HEADER_MIN_SIZE + 1)
    }

    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        let need = self.serialized_len();
        need_out(buf, need)?;
        let mut cursor = write_box_header(buf, &IROT, need)?;
        let angle =
            broadcast_core::len::fit_bits(u64::from(self.angle), IROT_ANGLE_BITS, "irot angle")?;
        buf[cursor] = u8::try_from(angle).map_err(|_| Error::InvalidInput("irot angle"))?;
        cursor += 1;
        Ok(cursor)
    }
}

/// Layer Selector property (`lsel`) — ISO/IEC 23008-12:2025 6.5.11.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub struct LayerSelectorProperty {
    layer_id: u16,
}

impl LayerSelectorProperty {
    /// A selector for `layer_id`.
    #[must_use]
    pub const fn new(layer_id: u16) -> Self {
        Self { layer_id }
    }

    /// `layer_id`.
    #[must_use]
    pub const fn layer_id(&self) -> u16 {
        self.layer_id
    }
}

impl<'a> Parse<'a> for LayerSelectorProperty {
    type Error = Error;

    fn parse(bytes: &'a [u8]) -> Result<Self> {
        expect_type(bytes, &LSEL, "expected lsel")?;
        let body = body_of(bytes, LSEL_LEN, "lsel")?;
        Ok(Self {
            layer_id: be_u16(body, 0, "lsel layer_id")?,
        })
    }
}

impl Serialize for LayerSelectorProperty {
    type Error = Error;

    fn serialized_len(&self) -> usize {
        box_len(BOX_HEADER_MIN_SIZE + LSEL_LEN)
    }

    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        let need = self.serialized_len();
        need_out(buf, need)?;
        let mut cursor = write_box_header(buf, &LSEL, need)?;
        buf[cursor..cursor + LSEL_LEN].copy_from_slice(&self.layer_id.to_be_bytes());
        cursor += LSEL_LEN;
        Ok(cursor)
    }
}

/// The mirroring axis of an [`ImageMirror`] property (6.5.12.3).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub enum ImageMirrorAxis {
    /// Mirror about a vertical axis (`axis = 0`).
    #[default]
    Vertical,
    /// Mirror about a horizontal axis (`axis = 1`).
    Horizontal,
}

impl ImageMirrorAxis {
    /// The `axis` value.
    #[must_use]
    pub const fn value(self) -> u8 {
        match self {
            Self::Vertical => 0,
            Self::Horizontal => 1,
        }
    }

    /// The axis named by `value`.
    #[must_use]
    pub const fn from_value(value: u8) -> Self {
        if value & 1 == 0 {
            Self::Vertical
        } else {
            Self::Horizontal
        }
    }

    /// Label for the axis, per the #204 convention.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::Vertical => "vertical",
            Self::Horizontal => "horizontal",
        }
    }
}

broadcast_core::impl_spec_display!(ImageMirrorAxis);

/// Image Mirror property (`imir`) — ISO/IEC 23008-12:2025 6.5.12.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub struct ImageMirror {
    axis: ImageMirrorAxis,
}

impl ImageMirror {
    /// A mirror about `axis`.
    #[must_use]
    pub const fn new(axis: ImageMirrorAxis) -> Self {
        Self { axis }
    }

    /// The mirroring axis.
    #[must_use]
    pub const fn axis(&self) -> ImageMirrorAxis {
        self.axis
    }
}

impl<'a> Parse<'a> for ImageMirror {
    type Error = Error;

    fn parse(bytes: &'a [u8]) -> Result<Self> {
        expect_type(bytes, &IMIR, "expected imir")?;
        let body = body_of(bytes, 1, "imir")?;
        Ok(Self {
            axis: ImageMirrorAxis::from_value(body[0] & ((1u8 << IMIR_AXIS_BITS) - 1)),
        })
    }
}

impl Serialize for ImageMirror {
    type Error = Error;

    fn serialized_len(&self) -> usize {
        box_len(BOX_HEADER_MIN_SIZE + 1)
    }

    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        let need = self.serialized_len();
        need_out(buf, need)?;
        let mut cursor = write_box_header(buf, &IMIR, need)?;
        buf[cursor] = self.axis.value();
        cursor += 1;
        Ok(cursor)
    }
}

/// Operating Point Selector property (`a1op`) — AVIF v1.2.0 2.3.2.1.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub struct OperatingPointSelectorProperty {
    op_index: u8,
}

impl OperatingPointSelectorProperty {
    /// A selector for operating point `op_index`.
    #[must_use]
    pub const fn new(op_index: u8) -> Self {
        Self { op_index }
    }

    /// `op_index`.
    #[must_use]
    pub const fn op_index(&self) -> u8 {
        self.op_index
    }
}

impl<'a> Parse<'a> for OperatingPointSelectorProperty {
    type Error = Error;

    fn parse(bytes: &'a [u8]) -> Result<Self> {
        expect_type(bytes, &A1OP, "expected a1op")?;
        let body = body_of(bytes, A1OP_LEN, "a1op")?;
        Ok(Self { op_index: body[0] })
    }
}

impl Serialize for OperatingPointSelectorProperty {
    type Error = Error;

    fn serialized_len(&self) -> usize {
        box_len(BOX_HEADER_MIN_SIZE + A1OP_LEN)
    }

    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        let need = self.serialized_len();
        need_out(buf, need)?;
        let mut cursor = write_box_header(buf, &A1OP, need)?;
        buf[cursor] = self.op_index;
        cursor += 1;
        Ok(cursor)
    }
}

/// AV1 Layered Image Indexing property (`a1lx`) — AVIF v1.2.0 2.3.2.3.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub struct Av1LayeredImageIndexingProperty {
    large_size: bool,
    layer_sizes: [u32; A1LX_LAYER_COUNT],
}

impl Av1LayeredImageIndexingProperty {
    /// The size of each layer but the last, with 16-bit fields when every size
    /// fits, else 32-bit fields.
    #[must_use]
    pub fn new(layer_sizes: [u32; A1LX_LAYER_COUNT]) -> Self {
        let large_size = layer_sizes.iter().any(|size| *size > u32::from(u16::MAX));
        Self {
            large_size,
            layer_sizes,
        }
    }

    /// Whether the fields are 32 bits wide.
    #[must_use]
    pub const fn large_size(&self) -> bool {
        self.large_size
    }

    /// `layer_size[i]`, in increasing spatial-id order.
    #[must_use]
    pub const fn layer_sizes(&self) -> [u32; A1LX_LAYER_COUNT] {
        self.layer_sizes
    }

    fn field_len(&self) -> usize {
        if self.large_size {
            A1LX_LARGE_SIZE_LEN
        } else {
            A1LX_SMALL_SIZE_LEN
        }
    }
}

impl<'a> Parse<'a> for Av1LayeredImageIndexingProperty {
    type Error = Error;

    fn parse(bytes: &'a [u8]) -> Result<Self> {
        expect_type(bytes, &A1LX, "expected a1lx")?;
        let body = body_of(bytes, 1, "a1lx")?;
        let large_size = body[0] & ((1u8 << A1LX_LARGE_BITS) - 1) != 0;
        let width = if large_size {
            A1LX_LARGE_SIZE_LEN
        } else {
            A1LX_SMALL_SIZE_LEN
        };
        let start = 1;
        crate::box_support::check_entry_count(
            body.len() - start,
            width,
            A1LX_LAYER_COUNT,
            "a1lx layer_size",
        )?;
        let mut layer_sizes = [0u32; A1LX_LAYER_COUNT];
        for (index, slot) in layer_sizes.iter_mut().enumerate() {
            let at = start + index * width;
            *slot = if large_size {
                be_u32(body, at, "a1lx layer_size")?
            } else {
                u32::from(be_u16(body, at, "a1lx layer_size")?)
            };
        }
        Ok(Self {
            large_size,
            layer_sizes,
        })
    }
}

impl Serialize for Av1LayeredImageIndexingProperty {
    type Error = Error;

    fn serialized_len(&self) -> usize {
        box_len(BOX_HEADER_MIN_SIZE + 1 + A1LX_LAYER_COUNT * self.field_len())
    }

    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        let need = self.serialized_len();
        need_out(buf, need)?;
        let mut cursor = write_box_header(buf, &A1LX, need)?;
        buf[cursor] = u8::from(self.large_size);
        cursor += 1;
        for size in self.layer_sizes {
            if self.large_size {
                buf[cursor..cursor + A1LX_LARGE_SIZE_LEN].copy_from_slice(&size.to_be_bytes());
                cursor += A1LX_LARGE_SIZE_LEN;
            } else {
                let narrow = u16::try_from(size)
                    .map_err(|_| Error::InvalidInput("a1lx layer_size exceeds u16"))?;
                buf[cursor..cursor + A1LX_SMALL_SIZE_LEN].copy_from_slice(&narrow.to_be_bytes());
                cursor += A1LX_SMALL_SIZE_LEN;
            }
        }
        Ok(cursor)
    }
}

/// Identity derivation (`iden`) item body — ISO/IEC 23008-12:2025 6.6.2.1.
///
/// The body is empty: the output is the input with the item's transformative
/// properties applied.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub struct IdentityDerivation;

impl IdentityDerivation {
    /// The one, empty body.
    #[must_use]
    pub const fn new() -> Self {
        Self
    }

    /// Check the body is empty.
    pub fn parse(body: &[u8]) -> Result<Self> {
        if body.is_empty() {
            Ok(Self)
        } else {
            Err(Error::InvalidValue(broadcast_core::error::InvalidValue {
                field: "iden item body",
                value: u64::try_from(body.len())
                    .map_err(|_| Error::InvalidInput("iden item body"))?,
                reason: "an identity derived image item has no body",
            }))
        }
    }

    /// The body length (zero).
    #[must_use]
    pub const fn serialized_len(&self) -> usize {
        0
    }
}

/// Image Overlay derivation (`iovl`) item body — ISO/IEC 23008-12:2025 6.6.2.2.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub struct ImageOverlay {
    flags: u8,
    canvas_fill_value: [u16; OVERLAY_FILL_CHANNELS],
    output_width: u32,
    output_height: u32,
    offsets: Vec<(i32, i32)>,
}

impl ImageOverlay {
    /// An overlay of a canvas of `output_width` x `output_height` with the
    /// given per-input offsets, nothing laid out yet.
    #[must_use]
    pub fn new(output_width: u32, output_height: u32, offsets: Vec<(i32, i32)>) -> Self {
        Self {
            flags: 0,
            canvas_fill_value: [0; OVERLAY_FILL_CHANNELS],
            output_width,
            output_height,
            offsets,
        }
    }

    /// Set the RGBA canvas fill value.
    #[must_use]
    pub fn with_canvas_fill_value(
        mut self,
        canvas_fill_value: [u16; OVERLAY_FILL_CHANNELS],
    ) -> Self {
        self.canvas_fill_value = canvas_fill_value;
        self
    }

    /// The FullBox-style `flags` byte (bit 0 widens the dimension fields).
    #[must_use]
    pub const fn flags(&self) -> u8 {
        self.flags
    }

    /// The RGBA canvas fill value.
    #[must_use]
    pub const fn canvas_fill_value(&self) -> [u16; OVERLAY_FILL_CHANNELS] {
        self.canvas_fill_value
    }

    /// The canvas alpha value.
    #[must_use]
    pub const fn canvas_alpha(&self) -> u16 {
        self.canvas_fill_value[OVERLAY_ALPHA_INDEX]
    }

    /// `output_width`.
    #[must_use]
    pub const fn output_width(&self) -> u32 {
        self.output_width
    }

    /// `output_height`.
    #[must_use]
    pub const fn output_height(&self) -> u32 {
        self.output_height
    }

    /// The per-input offsets, in the `dimg` reference order.
    #[must_use]
    pub fn offsets(&self) -> &[(i32, i32)] {
        &self.offsets
    }

    fn field_len(&self) -> usize {
        if self.flags & DERIVED_WIDE_FIELDS != 0 {
            DERIVED_LARGE_FIELD_LEN
        } else {
            DERIVED_SMALL_FIELD_LEN
        }
    }

    /// Parse the body, using `reference_count` from the item's `dimg`
    /// reference (§6.6.2.2.2).
    pub fn parse_with_reference_count(body: &[u8], reference_count: usize) -> Result<Self> {
        need_in(body, OVERLAY_FIXED_LEN, "iovl item body")?;
        let version = body[0];
        if version != DERIVED_VERSION {
            return Err(Error::UnsupportedVersion(
                broadcast_core::error::UnsupportedVersion {
                    what: "iovl version",
                    found: version,
                },
            ));
        }
        let flags = body[1];
        let mut cursor = OVERLAY_PREFIX_LEN;
        let mut canvas_fill_value = [0u16; OVERLAY_FILL_CHANNELS];
        for slot in &mut canvas_fill_value {
            *slot = be_u16(body, cursor, "iovl canvas_fill_value")?;
            cursor += OVERLAY_CHANNEL_LEN;
        }
        let width = if flags & DERIVED_WIDE_FIELDS != 0 {
            DERIVED_LARGE_FIELD_LEN
        } else {
            DERIVED_SMALL_FIELD_LEN
        };
        let output_width = read_derived_unsigned(body, &mut cursor, width, "iovl output_width")?;
        let output_height = read_derived_unsigned(body, &mut cursor, width, "iovl output_height")?;
        crate::box_support::check_entry_count(
            body.len() - cursor,
            width * OVERLAY_OFFSETS_PER_INPUT,
            reference_count,
            "iovl reference_count",
        )?;
        let mut offsets = Vec::with_capacity(crate::box_support::bounded_entry_count(
            body.len() - cursor,
            width * OVERLAY_OFFSETS_PER_INPUT,
            reference_count,
        ));
        for _ in 0..reference_count {
            let horizontal =
                read_derived_signed(body, &mut cursor, width, "iovl horizontal_offset")?;
            let vertical = read_derived_signed(body, &mut cursor, width, "iovl vertical_offset")?;
            offsets.push((horizontal, vertical));
        }
        Ok(Self {
            flags,
            canvas_fill_value,
            output_width,
            output_height,
            offsets,
        })
    }

    /// The body length.
    #[must_use]
    pub fn serialized_len(&self) -> usize {
        OVERLAY_FIXED_LEN
            + self.field_len()
                * (OVERLAY_DIMENSION_COUNT + self.offsets.len() * OVERLAY_OFFSETS_PER_INPUT)
    }

    /// Write the body.
    pub fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        let need = self.serialized_len();
        need_out(buf, need)?;
        buf[0] = DERIVED_VERSION;
        buf[1] = self.flags;
        let mut cursor = OVERLAY_PREFIX_LEN;
        for value in self.canvas_fill_value {
            buf[cursor..cursor + OVERLAY_CHANNEL_LEN].copy_from_slice(&value.to_be_bytes());
            cursor += OVERLAY_CHANNEL_LEN;
        }
        let width = self.field_len();
        write_derived_unsigned(buf, &mut cursor, width, u64::from(self.output_width))?;
        write_derived_unsigned(buf, &mut cursor, width, u64::from(self.output_height))?;
        for (horizontal, vertical) in &self.offsets {
            write_derived_signed(buf, &mut cursor, width, *horizontal)?;
            write_derived_signed(buf, &mut cursor, width, *vertical)?;
        }
        Ok(cursor)
    }
}

/// Image Grid derivation (`grid`) item body — ISO/IEC 23008-12:2025 6.6.2.3.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub struct ImageGrid {
    flags: u8,
    rows_minus_one: u8,
    columns_minus_one: u8,
    output_width: u32,
    output_height: u32,
}

impl ImageGrid {
    /// A grid of `rows` x `columns` tiles trimmed to `output_width` x
    /// `output_height`.
    #[must_use]
    pub fn new(rows: u8, columns: u8, output_width: u32, output_height: u32) -> Self {
        Self {
            flags: 0,
            rows_minus_one: rows.saturating_sub(1),
            columns_minus_one: columns.saturating_sub(1),
            output_width,
            output_height,
        }
    }

    /// The `flags` byte (bit 0 widens the dimension fields).
    #[must_use]
    pub const fn flags(&self) -> u8 {
        self.flags
    }

    /// The number of tile rows.
    #[must_use]
    pub const fn rows(&self) -> u8 {
        self.rows_minus_one + 1
    }

    /// The number of tiles per row.
    #[must_use]
    pub const fn columns(&self) -> u8 {
        self.columns_minus_one + 1
    }

    /// The number of tiles (`rows * columns`).
    #[must_use]
    pub const fn tile_count(&self) -> u32 {
        (self.rows() as u32) * (self.columns() as u32)
    }

    /// `output_width`.
    #[must_use]
    pub const fn output_width(&self) -> u32 {
        self.output_width
    }

    /// `output_height`.
    #[must_use]
    pub const fn output_height(&self) -> u32 {
        self.output_height
    }

    fn field_len(&self) -> usize {
        if self.flags & DERIVED_WIDE_FIELDS != 0 {
            DERIVED_LARGE_FIELD_LEN
        } else {
            DERIVED_SMALL_FIELD_LEN
        }
    }

    /// Parse the body.
    pub fn parse(body: &[u8]) -> Result<Self> {
        need_in(body, GRID_FIXED_LEN, "grid item body")?;
        let version = body[0];
        if version != DERIVED_VERSION {
            return Err(Error::UnsupportedVersion(
                broadcast_core::error::UnsupportedVersion {
                    what: "grid version",
                    found: version,
                },
            ));
        }
        let flags = body[1];
        let rows_minus_one = body[GRID_ROWS_OFFSET];
        let columns_minus_one = body[GRID_COLUMNS_OFFSET];
        let width = if flags & DERIVED_WIDE_FIELDS != 0 {
            DERIVED_LARGE_FIELD_LEN
        } else {
            DERIVED_SMALL_FIELD_LEN
        };
        let mut cursor = GRID_FIXED_LEN;
        let output_width = read_derived_unsigned(body, &mut cursor, width, "grid output_width")?;
        let output_height = read_derived_unsigned(body, &mut cursor, width, "grid output_height")?;
        Ok(Self {
            flags,
            rows_minus_one,
            columns_minus_one,
            output_width,
            output_height,
        })
    }

    /// The body length.
    #[must_use]
    pub fn serialized_len(&self) -> usize {
        GRID_FIXED_LEN + self.field_len() * GRID_DIMENSION_COUNT
    }

    /// Write the body.
    pub fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        let need = self.serialized_len();
        need_out(buf, need)?;
        buf[0] = DERIVED_VERSION;
        buf[1] = self.flags;
        buf[GRID_ROWS_OFFSET] = self.rows_minus_one;
        buf[GRID_COLUMNS_OFFSET] = self.columns_minus_one;
        let width = self.field_len();
        let mut cursor = GRID_FIXED_LEN;
        write_derived_unsigned(buf, &mut cursor, width, u64::from(self.output_width))?;
        write_derived_unsigned(buf, &mut cursor, width, u64::from(self.output_height))?;
        Ok(cursor)
    }
}

fn read_derived_unsigned(
    bytes: &[u8],
    cursor: &mut usize,
    width: usize,
    what: &'static str,
) -> Result<u32> {
    need_in(bytes, *cursor + width, what)?;
    let value = match width {
        DERIVED_SMALL_FIELD_LEN => u32::from(be_u16(bytes, *cursor, what)?),
        _ => be_u32(bytes, *cursor, what)?,
    };
    *cursor += width;
    Ok(value)
}

fn read_derived_signed(
    bytes: &[u8],
    cursor: &mut usize,
    width: usize,
    what: &'static str,
) -> Result<i32> {
    need_in(bytes, *cursor + width, what)?;
    let value = match width {
        DERIVED_SMALL_FIELD_LEN => {
            let mut word = [0u8; DERIVED_SMALL_FIELD_LEN];
            word.copy_from_slice(
                bytes
                    .get(*cursor..*cursor + DERIVED_SMALL_FIELD_LEN)
                    .ok_or(Error::BufferTooShort(
                        broadcast_core::error::BufferTooShort {
                            need: *cursor + DERIVED_SMALL_FIELD_LEN,
                            have: bytes.len(),
                            what,
                        },
                    ))?,
            );
            i32::from(i16::from_be_bytes(word))
        }
        _ => {
            let mut word = [0u8; DERIVED_LARGE_FIELD_LEN];
            word.copy_from_slice(
                bytes
                    .get(*cursor..*cursor + DERIVED_LARGE_FIELD_LEN)
                    .ok_or(Error::BufferTooShort(
                        broadcast_core::error::BufferTooShort {
                            need: *cursor + DERIVED_LARGE_FIELD_LEN,
                            have: bytes.len(),
                            what,
                        },
                    ))?,
            );
            i32::from_be_bytes(word)
        }
    };
    *cursor += width;
    Ok(value)
}

fn write_derived_unsigned(
    buf: &mut [u8],
    cursor: &mut usize,
    width: usize,
    value: u64,
) -> Result<()> {
    if width == DERIVED_SMALL_FIELD_LEN {
        let narrow = u16::try_from(value)
            .map_err(|_| Error::InvalidInput("derived image field exceeds u16"))?;
        buf[*cursor..*cursor + DERIVED_SMALL_FIELD_LEN].copy_from_slice(&narrow.to_be_bytes());
    } else {
        let wide = u32::try_from(value)
            .map_err(|_| Error::InvalidInput("derived image field exceeds u32"))?;
        buf[*cursor..*cursor + DERIVED_LARGE_FIELD_LEN].copy_from_slice(&wide.to_be_bytes());
    }
    *cursor += width;
    Ok(())
}

fn write_derived_signed(
    buf: &mut [u8],
    cursor: &mut usize,
    width: usize,
    value: i32,
) -> Result<()> {
    if width == DERIVED_SMALL_FIELD_LEN {
        let narrow = i16::try_from(value)
            .map_err(|_| Error::InvalidInput("derived image offset exceeds i16"))?;
        buf[*cursor..*cursor + DERIVED_SMALL_FIELD_LEN].copy_from_slice(&narrow.to_be_bytes());
    } else {
        buf[*cursor..*cursor + DERIVED_LARGE_FIELD_LEN].copy_from_slice(&value.to_be_bytes());
    }
    *cursor += width;
    Ok(())
}

/// A rational number with a signed numerator, as the gain map metadata states
/// a per-channel minimum, maximum or offset.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct SignedRational {
    numerator: i32,
    denominator: u32,
}

impl SignedRational {
    /// A rational `numerator / denominator`.
    #[must_use]
    pub const fn new(numerator: i32, denominator: u32) -> Self {
        Self {
            numerator,
            denominator,
        }
    }

    /// `numerator`.
    #[must_use]
    pub const fn numerator(&self) -> i32 {
        self.numerator
    }

    /// `denominator`.
    #[must_use]
    pub const fn denominator(&self) -> u32 {
        self.denominator
    }

    /// Whether `denominator` is zero, which no value can be divided by.
    #[must_use]
    pub const fn is_zero_denominator(&self) -> bool {
        self.denominator == 0
    }
}

/// A rational number with an unsigned numerator, as the gain map metadata
/// states a gamma or an HDR headroom.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct UnsignedRational {
    numerator: u32,
    denominator: u32,
}

impl UnsignedRational {
    /// A rational `numerator / denominator`.
    #[must_use]
    pub const fn new(numerator: u32, denominator: u32) -> Self {
        Self {
            numerator,
            denominator,
        }
    }

    /// `numerator`.
    #[must_use]
    pub const fn numerator(&self) -> u32 {
        self.numerator
    }

    /// `denominator`.
    #[must_use]
    pub const fn denominator(&self) -> u32 {
        self.denominator
    }

    /// Whether `denominator` is zero, which no value can be divided by.
    #[must_use]
    pub const fn is_zero_denominator(&self) -> bool {
        self.denominator == 0
    }
}

/// The five per-channel values of the gain map metadata, in wire order.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub struct GainMapChannel {
    min: SignedRational,
    max: SignedRational,
    gamma: UnsignedRational,
    base_offset: SignedRational,
    alternate_offset: SignedRational,
}

impl GainMapChannel {
    /// A channel with its five values.
    #[must_use]
    pub const fn new(
        min: SignedRational,
        max: SignedRational,
        gamma: UnsignedRational,
        base_offset: SignedRational,
        alternate_offset: SignedRational,
    ) -> Self {
        Self {
            min,
            max,
            gamma,
            base_offset,
            alternate_offset,
        }
    }

    /// The minimum gain map value.
    #[must_use]
    pub const fn min(&self) -> SignedRational {
        self.min
    }

    /// The maximum gain map value.
    #[must_use]
    pub const fn max(&self) -> SignedRational {
        self.max
    }

    /// The encoding gamma of the gain map image.
    #[must_use]
    pub const fn gamma(&self) -> UnsignedRational {
        self.gamma
    }

    /// The offset applied to the base image samples.
    #[must_use]
    pub const fn base_offset(&self) -> SignedRational {
        self.base_offset
    }

    /// The offset applied to the alternate image samples.
    #[must_use]
    pub const fn alternate_offset(&self) -> SignedRational {
        self.alternate_offset
    }

    fn parse(body: &[u8]) -> Self {
        Self {
            min: signed_at(body, 0),
            max: signed_at(body, 2 * GAIN_MAP_COMPONENT_LEN),
            gamma: unsigned_at(body, 4 * GAIN_MAP_COMPONENT_LEN),
            base_offset: signed_at(body, 6 * GAIN_MAP_COMPONENT_LEN),
            alternate_offset: signed_at(body, 8 * GAIN_MAP_COMPONENT_LEN),
        }
    }

    fn serialize_into(&self, buf: &mut [u8]) {
        let mut cursor = 0;
        write_signed(buf, &mut cursor, self.min);
        write_signed(buf, &mut cursor, self.max);
        write_unsigned(buf, &mut cursor, self.gamma);
        write_signed(buf, &mut cursor, self.base_offset);
        write_signed(buf, &mut cursor, self.alternate_offset);
    }
}

/// One value of the gain map metadata that carries a rational.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub enum GainMapField {
    /// A base image HDR headroom.
    BaseHeadroom,
    /// An alternate image HDR headroom.
    AlternateHeadroom,
    /// A channel's minimum gain map value.
    Min,
    /// A channel's maximum gain map value.
    Max,
    /// A channel's gain map gamma.
    Gamma,
    /// A channel's base image offset.
    BaseOffset,
    /// A channel's alternate image offset.
    AlternateOffset,
}

impl GainMapField {
    /// The field's stable token.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::BaseHeadroom => "base-headroom",
            Self::AlternateHeadroom => "alternate-headroom",
            Self::Min => "min",
            Self::Max => "max",
            Self::Gamma => "gamma",
            Self::BaseOffset => "base-offset",
            Self::AlternateOffset => "alternate-offset",
        }
    }
}

broadcast_core::impl_spec_display!(GainMapField);

/// The `GainMapMetadata` structure carried by a tone-map (`tmap`) item.
///
/// The layout comes from the open implementations that agree on it because
/// ISO 21496-1 is not held; `private/specs-verified/misc/gain-map/SOURCES.md`
/// records the four sources, where they agree and the one view adopted. The
/// wrapper (one body version byte, then this structure) is ISO/IEC
/// 23008-12:2025 Amd 1 6.6.2.4.2.
///
/// This type only represents the view those sources agree on: `minimum_version`
/// 0, no reserved flag bit set, and exactly the bytes the channel count
/// implies. Any other body stays an opaque payload in [`ToneMapImage`] and
/// round-trips; asking for the typed view of such a body returns `None`.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub struct GainMapMetadata {
    minimum_version: u16,
    writer_version: u16,
    multichannel: bool,
    base_colour_space: bool,
    reserved_flags: u8,
    base_headroom: UnsignedRational,
    alternate_headroom: UnsignedRational,
    channels: Vec<GainMapChannel>,
}

impl GainMapMetadata {
    /// Metadata with the minimum and writer versions this view implements and
    /// no reserved flag bit set, from one or three channels.
    pub fn new(
        base_colour_space: bool,
        base_headroom: UnsignedRational,
        alternate_headroom: UnsignedRational,
        channels: Vec<GainMapChannel>,
    ) -> Result<Self> {
        if channels.len() != GAIN_MAP_SINGLE_CHANNEL_COUNT
            && channels.len() != GAIN_MAP_MULTICHANNEL_COUNT
        {
            return Err(Error::InvalidInput(
                "gain map metadata needs one or three channels",
            ));
        }
        Ok(Self {
            minimum_version: GAIN_MAP_MINIMUM_VERSION,
            writer_version: GAIN_MAP_MINIMUM_VERSION,
            multichannel: channels.len() == GAIN_MAP_MULTICHANNEL_COUNT,
            base_colour_space,
            reserved_flags: 0,
            base_headroom,
            alternate_headroom,
            channels,
        })
    }

    /// `minimum_version`.
    #[must_use]
    pub const fn minimum_version(&self) -> u16 {
        self.minimum_version
    }

    /// `writer_version`.
    #[must_use]
    pub const fn writer_version(&self) -> u16 {
        self.writer_version
    }

    /// Whether the gain map carries three channels rather than one.
    #[must_use]
    pub const fn is_multichannel(&self) -> bool {
        self.multichannel
    }

    /// Whether the gain map is applied in the base image's colour space.
    #[must_use]
    pub const fn uses_base_colour_space(&self) -> bool {
        self.base_colour_space
    }

    /// The reserved flag bits, carried as read.
    #[must_use]
    pub const fn reserved_flags(&self) -> u8 {
        self.reserved_flags
    }

    /// The base image HDR headroom.
    #[must_use]
    pub const fn base_headroom(&self) -> UnsignedRational {
        self.base_headroom
    }

    /// The alternate image HDR headroom.
    #[must_use]
    pub const fn alternate_headroom(&self) -> UnsignedRational {
        self.alternate_headroom
    }

    /// The per-channel values, one or three entries.
    #[must_use]
    pub fn channels(&self) -> &[GainMapChannel] {
        &self.channels
    }

    /// Every rational whose denominator is zero, in wire order, with the
    /// channel index (`None` for a headroom).
    #[must_use]
    pub fn zero_denominators(&self) -> Vec<(Option<usize>, GainMapField)> {
        let mut zero = Vec::new();
        if self.base_headroom.is_zero_denominator() {
            zero.push((None, GainMapField::BaseHeadroom));
        }
        if self.alternate_headroom.is_zero_denominator() {
            zero.push((None, GainMapField::AlternateHeadroom));
        }
        for (index, channel) in self.channels.iter().enumerate() {
            let fields = [
                (GainMapField::Min, channel.min.is_zero_denominator()),
                (GainMapField::Max, channel.max.is_zero_denominator()),
                (GainMapField::Gamma, channel.gamma.is_zero_denominator()),
                (
                    GainMapField::BaseOffset,
                    channel.base_offset.is_zero_denominator(),
                ),
                (
                    GainMapField::AlternateOffset,
                    channel.alternate_offset.is_zero_denominator(),
                ),
            ];
            for (field, is_zero) in fields {
                if is_zero {
                    zero.push((Some(index), field));
                }
            }
        }
        zero
    }
}

impl<'a> Parse<'a> for GainMapMetadata {
    type Error = Error;

    fn parse(body: &'a [u8]) -> Result<Self> {
        need_in(body, GAIN_MAP_FIXED_LEN, "gain map metadata")?;
        let minimum_version = be_u16(body, GAIN_MAP_MINIMUM_OFFSET, "gain map metadata")?;
        if minimum_version != GAIN_MAP_MINIMUM_VERSION {
            return Err(Error::UnsupportedFeature("gain map minimum version"));
        }
        let writer_version = be_u16(body, GAIN_MAP_WRITER_OFFSET, "gain map metadata")?;
        let flags = body[GAIN_MAP_FLAGS_OFFSET];
        let reserved_flags = flags & GAIN_MAP_RESERVED_FLAGS_MASK;
        if reserved_flags != 0 {
            return Err(Error::UnsupportedFeature("gain map reserved flags"));
        }
        let multichannel = flags & GAIN_MAP_MULTICHANNEL_FLAG != 0;
        let channel_count = if multichannel {
            GAIN_MAP_MULTICHANNEL_COUNT
        } else {
            GAIN_MAP_SINGLE_CHANNEL_COUNT
        };
        if body.len() != GAIN_MAP_FIXED_LEN + channel_count * GAIN_MAP_CHANNEL_LEN {
            return Err(Error::InvalidInput("gain map metadata length"));
        }
        let channels = body[GAIN_MAP_FIXED_LEN..]
            .chunks_exact(GAIN_MAP_CHANNEL_LEN)
            .map(GainMapChannel::parse)
            .collect();
        Ok(Self {
            minimum_version,
            writer_version,
            multichannel,
            base_colour_space: flags & GAIN_MAP_BASE_COLOUR_SPACE_FLAG != 0,
            reserved_flags,
            base_headroom: unsigned_at(body, GAIN_MAP_BASE_HEADROOM_OFFSET),
            alternate_headroom: unsigned_at(body, GAIN_MAP_ALTERNATE_HEADROOM_OFFSET),
            channels,
        })
    }
}

impl Serialize for GainMapMetadata {
    type Error = Error;

    fn serialized_len(&self) -> usize {
        GAIN_MAP_FIXED_LEN + self.channels.len() * GAIN_MAP_CHANNEL_LEN
    }

    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        let need = self.serialized_len();
        need_out(buf, need)?;
        buf[GAIN_MAP_MINIMUM_OFFSET..GAIN_MAP_MINIMUM_OFFSET + core::mem::size_of::<u16>()]
            .copy_from_slice(&self.minimum_version.to_be_bytes());
        buf[GAIN_MAP_WRITER_OFFSET..GAIN_MAP_WRITER_OFFSET + core::mem::size_of::<u16>()]
            .copy_from_slice(&self.writer_version.to_be_bytes());
        let mut flags = self.reserved_flags;
        if self.multichannel {
            flags |= GAIN_MAP_MULTICHANNEL_FLAG;
        }
        if self.base_colour_space {
            flags |= GAIN_MAP_BASE_COLOUR_SPACE_FLAG;
        }
        buf[GAIN_MAP_FLAGS_OFFSET] = flags;
        let mut cursor = GAIN_MAP_BASE_HEADROOM_OFFSET;
        write_unsigned(buf, &mut cursor, self.base_headroom);
        write_unsigned(buf, &mut cursor, self.alternate_headroom);
        for channel in &self.channels {
            channel.serialize_into(&mut buf[cursor..cursor + GAIN_MAP_CHANNEL_LEN]);
            cursor += GAIN_MAP_CHANNEL_LEN;
        }
        Ok(need)
    }
}

fn signed_at(body: &[u8], offset: usize) -> SignedRational {
    SignedRational::new(
        i32_at(body, offset),
        u32_at(body, offset + GAIN_MAP_COMPONENT_LEN),
    )
}

fn unsigned_at(body: &[u8], offset: usize) -> UnsignedRational {
    UnsignedRational::new(
        u32_at(body, offset),
        u32_at(body, offset + GAIN_MAP_COMPONENT_LEN),
    )
}

fn write_signed(buf: &mut [u8], cursor: &mut usize, value: SignedRational) {
    buf[*cursor..*cursor + GAIN_MAP_COMPONENT_LEN].copy_from_slice(&value.numerator.to_be_bytes());
    *cursor += GAIN_MAP_COMPONENT_LEN;
    buf[*cursor..*cursor + GAIN_MAP_COMPONENT_LEN]
        .copy_from_slice(&value.denominator.to_be_bytes());
    *cursor += GAIN_MAP_COMPONENT_LEN;
}

fn write_unsigned(buf: &mut [u8], cursor: &mut usize, value: UnsignedRational) {
    buf[*cursor..*cursor + GAIN_MAP_COMPONENT_LEN].copy_from_slice(&value.numerator.to_be_bytes());
    *cursor += GAIN_MAP_COMPONENT_LEN;
    buf[*cursor..*cursor + GAIN_MAP_COMPONENT_LEN]
        .copy_from_slice(&value.denominator.to_be_bytes());
    *cursor += GAIN_MAP_COMPONENT_LEN;
}

/// A property of an item — one box of the `ItemPropertyContainerBox`
/// (ISO/IEC 14496-12:2026 §8.11.14.1).
///
/// The typed variants reuse the crate's one type per structure; a property
/// this crate does not model stays [`ItemProperty::Opaque`] so it keeps its
/// index and round-trips.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub enum ItemProperty {
    /// `ispe`: image spatial extents.
    Ispe(ImageSpatialExtentsProperty),
    /// `pasp`: pixel aspect ratio.
    Pasp(PixelAspectRatioBox),
    /// `colr`: colour information.
    Colr(ColourInformationBox),
    /// `pixi`: pixel information.
    Pixi(PixelInformationProperty),
    /// `rloc`: relative location.
    Rloc(RelativeLocationProperty),
    /// `auxC`: auxiliary image type.
    AuxC(AuxiliaryTypeProperty),
    /// `clap`: clean aperture.
    Clap(CleanApertureBox),
    /// `irot`: image rotation.
    Irot(ImageRotation),
    /// `lsel`: layer selection.
    Lsel(LayerSelectorProperty),
    /// `imir`: image mirroring.
    Imir(ImageMirror),
    /// `hvcC`: HEVC decoder configuration.
    HvcC(HEVCConfigurationBox),
    /// `avcC`: AVC decoder configuration.
    AvcC(AVCConfigurationBox),
    /// `av1C`: AV1 decoder configuration.
    Av1C(Av1ConfigurationBox),
    /// `vvcC`: VVC decoder configuration (ISO/IEC 23008-12:2025).
    VvcC(crate::vvc_config::VvcConfigurationBox),
    /// `a1op`: AV1 operating point selector.
    A1op(OperatingPointSelectorProperty),
    /// `a1lx`: AV1 layered image indexing.
    A1lx(Av1LayeredImageIndexingProperty),
    /// `mdcv`: mastering display colour volume.
    Mdcv(MasteringDisplayColourVolume),
    /// `clli`: content light level.
    Clli(ContentLightLevel),
    /// `brnd`: a brand list.
    Brnd(BrandProperty),
    /// `hdlp`: a handler mapping.
    Hdlp(HandlerProperty),
    /// `scrb`: a scramble scheme (§13.4.8).
    Scrb(crate::scheme::ScrambleSchemeInfoBox),
    /// `amve`: the ambient viewing environment (§12.1.9).
    Amve(AmbientViewingEnvironmentBox),
    /// `cclv`: the content colour volume (§12.1.8).
    Cclv(ContentColourVolumeBox),
    /// `iscl`: image scaling (23008-12 6.5.13).
    Iscl(ImageScaling),
    /// `rref`: required reference types (6.5.17).
    Rref(RequiredReferenceTypesProperty),
    /// `crtt`: creation time (6.5.18).
    Crtt(CreationTimeProperty),
    /// `mdft`: modification time (6.5.19).
    Mdft(ModificationTimeProperty),
    /// `udes`: user description (6.5.20).
    Udes(UserDescriptionProperty),
    /// `altt`: accessibility text (6.5.21).
    Altt(AccessibilityTextProperty),
    /// `aebr`: auto exposure (6.5.22).
    Aebr(AutoExposureProperty),
    /// `wbbr`: white balance (6.5.23).
    Wbbr(WhiteBalanceProperty),
    /// `fobr`: focus (6.5.24).
    Fobr(FocusProperty),
    /// `afbr`: flash exposure (6.5.25).
    Afbr(FlashExposureProperty),
    /// `dobr`: depth of field (6.5.26).
    Dobr(DepthOfFieldProperty),
    /// `pano`: panorama (6.5.27).
    Pano(PanoramaProperty),
    /// `tols`: target output layer set (6.5.29).
    Tols(TargetOutputLayerSetProperty),
    /// `wipe`: wipe transition (6.5.30).
    Wipe(WipeTransitionEffectProperty),
    /// `zoom`: zoom transition (6.5.31).
    Zoom(ZoomTransitionEffectProperty),
    /// `fade`: fade transition (6.5.32).
    Fade(FadeTransitionEffectProperty),
    /// `splt`: split transition (6.5.33).
    Splt(SplitTransitionEffectProperty),
    /// `stpe`: suggested transition period (6.5.34).
    Stpe(SuggestedTransitionPeriodProperty),
    /// `ssld`: suggested time display duration (6.5.35).
    Ssld(SuggestedTimeDisplayDurationProperty),
    /// `prdi`: progressive derived image item information (6.5.37).
    Prdi(ProgressiveDerivedImageItemInformationProperty),
    /// `sstr`: single stream (6.5.38).
    Sstr(SingleStreamProperty),
    /// `cmex`: camera extrinsic matrix (6.5.39).
    Cmex(CameraExtrinsicMatrixProperty),
    /// `cmin`: camera intrinsic matrix (6.5.40).
    Cmin(CameraIntrinsicMatrixProperty),
    /// `cexg`: constrained extents grid (Amd 1 6.5.41).
    Cexg(ConstrainedExtentsGridProperty),
    /// `dadj`: disparity adjustment (Amd 1 6.5.42).
    Dadj(DisparityAdjustmentProperty),
    /// `stag`: stereo aggressors (Amd 1 6.5.43).
    Stag(StereoAggressorsProperty),
    /// `reve`: reference viewing environment (Amd 1 6.5.44).
    Reve(ReferenceViewingEnvironmentProperty),
    /// `ndwt`: nominal diffuse white (Amd 1 6.5.45).
    Ndwt(NominalDiffuseWhiteProperty),
    /// `mskC`: mask configuration (11.2.2.2).
    MskC(MaskConfigurationProperty),
    /// `subs`: sub-sample information (6.5.28, the `subs` box reused).
    Subs(crate::sample_groups::SubSampleInformationBox),
    /// `stvi`: stereoscopic video information (ISO/IEC 14496-12:2026 13.5.4).
    Stvi(crate::track_desc::StereoVideoBox),
    /// Any property this crate does not model, preserved box and all.
    Opaque(OpaqueBox),
}

impl ItemProperty {
    /// The property's four-CC.
    #[must_use]
    pub fn fourcc(&self) -> [u8; BOX_TYPE_SIZE] {
        match self {
            Self::Ispe(_) => ISPE,
            Self::Pasp(_) => PASP,
            Self::Colr(_) => COLR,
            Self::Pixi(_) => PIXI,
            Self::Rloc(_) => RLOC,
            Self::AuxC(_) => AUXC,
            Self::Clap(_) => CLAP,
            Self::Irot(_) => IROT,
            Self::Lsel(_) => LSEL,
            Self::Imir(_) => IMIR,
            Self::HvcC(_) => HVCC,
            Self::AvcC(_) => AVCC,
            Self::Av1C(_) => AV1C,
            Self::VvcC(_) => *b"vvcC",
            Self::A1op(_) => A1OP,
            Self::A1lx(_) => A1LX,
            Self::Mdcv(_) => MDCV_FOURCC,
            Self::Clli(_) => CLLI_FOURCC,
            Self::Brnd(_) => crate::item::BRND,
            Self::Hdlp(_) => crate::item::HDLP,
            Self::Scrb(_) => crate::scheme::SCRB,
            Self::Amve(_) => AMVE,
            Self::Cclv(_) => CCLV,
            Self::Iscl(_) => crate::image_props::ISCL,
            Self::Rref(_) => crate::image_props::RREF,
            Self::Crtt(_) => crate::image_props::CRTT,
            Self::Mdft(_) => crate::image_props::MDFT,
            Self::Udes(_) => crate::image_props::UDES,
            Self::Altt(_) => crate::image_props::ALTT,
            Self::Aebr(_) => crate::image_props::AEBR,
            Self::Wbbr(_) => crate::image_props::WBBR,
            Self::Fobr(_) => crate::image_props::FOBR,
            Self::Afbr(_) => crate::image_props::AFBR,
            Self::Dobr(_) => crate::image_props::DOBR,
            Self::Pano(_) => crate::image_props::PANO,
            Self::Tols(_) => crate::image_props::TOLS,
            Self::Wipe(_) => crate::image_props::WIPE,
            Self::Zoom(_) => crate::image_props::ZOOM,
            Self::Fade(_) => crate::image_props::FADE,
            Self::Splt(_) => crate::image_props::SPLT,
            Self::Stpe(_) => crate::image_props::STPE,
            Self::Ssld(_) => crate::image_props::SSLD,
            Self::Prdi(_) => crate::image_props::PRDI,
            Self::Sstr(_) => crate::image_props::SSTR,
            Self::Cmex(_) => crate::image_props::CMEX,
            Self::Cmin(_) => crate::image_props::CMIN,
            Self::Cexg(_) => crate::image_props::CEXG,
            Self::Dadj(_) => crate::image_props::DADJ,
            Self::Stag(_) => crate::image_props::STAG,
            Self::Reve(_) => crate::image_props::REVE,
            Self::Ndwt(_) => crate::image_props::NDWT,
            Self::MskC(_) => crate::image_props::MSKC,
            Self::Subs(_) => *b"subs",
            Self::Stvi(_) => crate::track_desc::STVI,
            Self::Opaque(opaque) => opaque.box_type,
        }
    }

    /// Whether this crate models the property's structure.
    #[must_use]
    pub fn is_known(&self) -> bool {
        !matches!(self, Self::Opaque(_))
    }

    /// The spatial-extents property, when this is one.
    #[must_use]
    pub fn as_spatial_extents(&self) -> Option<&ImageSpatialExtentsProperty> {
        match self {
            Self::Ispe(value) => Some(value),
            _ => None,
        }
    }

    /// The pixel-information property, when this is one.
    #[must_use]
    pub fn as_pixel_information(&self) -> Option<&PixelInformationProperty> {
        match self {
            Self::Pixi(value) => Some(value),
            _ => None,
        }
    }

    /// The rotation property, when this is one.
    #[must_use]
    pub fn as_rotation(&self) -> Option<&ImageRotation> {
        match self {
            Self::Irot(value) => Some(value),
            _ => None,
        }
    }

    /// The mirror property, when this is one.
    #[must_use]
    pub fn as_mirror(&self) -> Option<&ImageMirror> {
        match self {
            Self::Imir(value) => Some(value),
            _ => None,
        }
    }

    /// The auxiliary-type property, when this is one.
    #[must_use]
    pub fn as_aux_type(&self) -> Option<&AuxiliaryTypeProperty> {
        match self {
            Self::AuxC(value) => Some(value),
            _ => None,
        }
    }

    /// Parse one property box (whole box, header included).
    pub fn parse_box(bytes: &[u8]) -> Result<Self> {
        let item = crate::box_types::parse_box(bytes)?.0;
        let body = item.body;
        match &item.four_cc() {
            b"ispe" => Ok(Self::Ispe(ImageSpatialExtentsProperty::parse(item.bytes)?)),
            b"pasp" => Ok(Self::Pasp(PixelAspectRatioBox::parse(body)?)),
            b"colr" => Ok(Self::Colr(ColourInformationBox::parse(body)?)),
            b"pixi" => Ok(Self::Pixi(PixelInformationProperty::parse(item.bytes)?)),
            b"rloc" => Ok(Self::Rloc(RelativeLocationProperty::parse(item.bytes)?)),
            b"auxC" => Ok(Self::AuxC(AuxiliaryTypeProperty::parse(item.bytes)?)),
            b"clap" => Ok(Self::Clap(CleanApertureBox::parse(body)?)),
            b"irot" => Ok(Self::Irot(ImageRotation::parse(item.bytes)?)),
            b"lsel" => Ok(Self::Lsel(LayerSelectorProperty::parse(item.bytes)?)),
            b"imir" => Ok(Self::Imir(ImageMirror::parse(item.bytes)?)),
            b"hvcC" => Ok(Self::HvcC(HEVCConfigurationBox::parse_body(body)?)),
            b"avcC" => Ok(Self::AvcC(AVCConfigurationBox::parse_body(body)?)),
            b"av1C" => Ok(Self::Av1C(Av1ConfigurationBox::parse(body)?)),
            b"vvcC" => Ok(Self::VvcC(
                crate::vvc_config::VvcConfigurationBox::parse_body(body)?,
            )),
            b"a1op" => Ok(Self::A1op(OperatingPointSelectorProperty::parse(
                item.bytes,
            )?)),
            b"a1lx" => Ok(Self::A1lx(Av1LayeredImageIndexingProperty::parse(
                item.bytes,
            )?)),
            b"mdcv" => Ok(Self::Mdcv(MasteringDisplayColourVolume::parse(body)?)),
            b"clli" => Ok(Self::Clli(ContentLightLevel::parse(body)?)),
            b"brnd" => Ok(Self::Brnd(BrandProperty::parse(item.bytes)?)),
            b"hdlp" => Ok(Self::Hdlp(HandlerProperty::parse(item.bytes)?)),
            b"scrb" => Ok(Self::Scrb(crate::scheme::ScrambleSchemeInfoBox::parse(
                item.bytes,
            )?)),
            b"amve" => Ok(Self::Amve(AmbientViewingEnvironmentBox::parse(body)?)),
            b"cclv" => Ok(Self::Cclv(ContentColourVolumeBox::parse(body)?)),
            b"iscl" => Ok(Self::Iscl(crate::image_props::ImageScaling::parse(
                item.bytes,
            )?)),
            b"rref" => Ok(Self::Rref(
                crate::image_props::RequiredReferenceTypesProperty::parse(item.bytes)?,
            )),
            b"crtt" => Ok(Self::Crtt(crate::image_props::CreationTimeProperty::parse(
                item.bytes,
            )?)),
            b"mdft" => Ok(Self::Mdft(
                crate::image_props::ModificationTimeProperty::parse(item.bytes)?,
            )),
            b"udes" => Ok(Self::Udes(
                crate::image_props::UserDescriptionProperty::parse(item.bytes)?,
            )),
            b"altt" => Ok(Self::Altt(
                crate::image_props::AccessibilityTextProperty::parse(item.bytes)?,
            )),
            b"aebr" => Ok(Self::Aebr(crate::image_props::AutoExposureProperty::parse(
                item.bytes,
            )?)),
            b"wbbr" => Ok(Self::Wbbr(crate::image_props::WhiteBalanceProperty::parse(
                item.bytes,
            )?)),
            b"fobr" => Ok(Self::Fobr(crate::image_props::FocusProperty::parse(
                item.bytes,
            )?)),
            b"afbr" => Ok(Self::Afbr(
                crate::image_props::FlashExposureProperty::parse(item.bytes)?,
            )),
            b"dobr" => Ok(Self::Dobr(crate::image_props::DepthOfFieldProperty::parse(
                item.bytes,
            )?)),
            b"pano" => Ok(Self::Pano(crate::image_props::PanoramaProperty::parse(
                item.bytes,
            )?)),
            b"tols" => Ok(Self::Tols(
                crate::image_props::TargetOutputLayerSetProperty::parse(item.bytes)?,
            )),
            b"wipe" => Ok(Self::Wipe(
                crate::image_props::WipeTransitionEffectProperty::parse(item.bytes)?,
            )),
            b"zoom" => Ok(Self::Zoom(
                crate::image_props::ZoomTransitionEffectProperty::parse(item.bytes)?,
            )),
            b"fade" => Ok(Self::Fade(
                crate::image_props::FadeTransitionEffectProperty::parse(item.bytes)?,
            )),
            b"splt" => Ok(Self::Splt(
                crate::image_props::SplitTransitionEffectProperty::parse(item.bytes)?,
            )),
            b"stpe" => Ok(Self::Stpe(
                crate::image_props::SuggestedTransitionPeriodProperty::parse(item.bytes)?,
            )),
            b"ssld" => Ok(Self::Ssld(
                crate::image_props::SuggestedTimeDisplayDurationProperty::parse(item.bytes)?,
            )),
            b"prdi" => Ok(Self::Prdi(
                crate::image_props::ProgressiveDerivedImageItemInformationProperty::parse(
                    item.bytes,
                )?,
            )),
            b"sstr" => Ok(Self::Sstr(crate::image_props::SingleStreamProperty::parse(
                item.bytes,
            )?)),
            b"cmex" => Ok(Self::Cmex(
                crate::image_props::CameraExtrinsicMatrixProperty::parse(item.bytes)?,
            )),
            b"cmin" => Ok(Self::Cmin(
                crate::image_props::CameraIntrinsicMatrixProperty::parse(item.bytes)?,
            )),
            b"cexg" => Ok(Self::Cexg(
                crate::image_props::ConstrainedExtentsGridProperty::parse(item.bytes)?,
            )),
            b"dadj" => Ok(Self::Dadj(
                crate::image_props::DisparityAdjustmentProperty::parse(item.bytes)?,
            )),
            b"stag" => Ok(Self::Stag(
                crate::image_props::StereoAggressorsProperty::parse(item.bytes)?,
            )),
            b"reve" => Ok(Self::Reve(
                crate::image_props::ReferenceViewingEnvironmentProperty::parse(item.bytes)?,
            )),
            b"ndwt" => Ok(Self::Ndwt(
                crate::image_props::NominalDiffuseWhiteProperty::parse(item.bytes)?,
            )),
            b"mskC" => Ok(Self::MskC(
                crate::image_props::MaskConfigurationProperty::parse(item.bytes)?,
            )),
            b"subs" => Ok(Self::Subs(
                crate::sample_groups::SubSampleInformationBox::parse(item.bytes)?,
            )),
            b"stvi" => Ok(Self::Stvi(crate::track_desc::StereoVideoBox::parse(
                item.bytes,
            )?)),
            _ => {
                let mut opaque = OpaqueBox::new(item.four_cc(), item.payload().to_vec());
                opaque.to_end = item.to_end();
                opaque.largesize = item.has_largesize();
                Ok(Self::Opaque(opaque))
            }
        }
    }

    fn write_body_box(
        buf: &mut [u8],
        fourcc: &[u8; BOX_TYPE_SIZE],
        payload_len: usize,
        body: &impl Serialize<Error = Error>,
    ) -> Result<usize> {
        let need = box_len(BOX_HEADER_MIN_SIZE + payload_len);
        need_out(buf, need)?;
        let mut cursor = write_box_header(buf, fourcc, need)?;
        cursor += body.serialize_into(&mut buf[cursor..])?;
        Ok(cursor)
    }
}

impl Serialize for ItemProperty {
    type Error = Error;

    fn serialized_len(&self) -> usize {
        match self {
            Self::Pasp(value) => box_len(BOX_HEADER_MIN_SIZE + value.serialized_len()),
            Self::Colr(value) => box_len(BOX_HEADER_MIN_SIZE + value.serialized_len()),
            Self::Clap(value) => box_len(BOX_HEADER_MIN_SIZE + value.serialized_len()),
            Self::Av1C(value) => box_len(BOX_HEADER_MIN_SIZE + value.serialized_len()),
            Self::Mdcv(_) => box_len(BOX_HEADER_MIN_SIZE + MDCV_PAYLOAD_LEN),
            Self::Clli(_) => box_len(BOX_HEADER_MIN_SIZE + CLLI_PAYLOAD_LEN),
            Self::Ispe(value) => value.serialized_len(),
            Self::Pixi(value) => value.serialized_len(),
            Self::Rloc(value) => value.serialized_len(),
            Self::AuxC(value) => value.serialized_len(),
            Self::Irot(value) => value.serialized_len(),
            Self::Lsel(value) => value.serialized_len(),
            Self::Imir(value) => value.serialized_len(),
            Self::HvcC(value) => value.serialized_len(),
            Self::AvcC(value) => value.serialized_len(),
            Self::VvcC(value) => value.serialized_len(),
            Self::A1op(value) => value.serialized_len(),
            Self::A1lx(value) => value.serialized_len(),
            Self::Brnd(value) => value.serialized_len(),
            Self::Hdlp(value) => value.serialized_len(),
            Self::Scrb(value) => value.serialized_len(),
            Self::Amve(value) => box_len(BOX_HEADER_MIN_SIZE + value.serialized_len()),
            Self::Cclv(value) => box_len(BOX_HEADER_MIN_SIZE + value.serialized_len()),
            Self::Iscl(value) => value.serialized_len(),
            Self::Rref(value) => value.serialized_len(),
            Self::Crtt(value) => value.serialized_len(),
            Self::Mdft(value) => value.serialized_len(),
            Self::Udes(value) => value.serialized_len(),
            Self::Altt(value) => value.serialized_len(),
            Self::Aebr(value) => value.serialized_len(),
            Self::Wbbr(value) => value.serialized_len(),
            Self::Fobr(value) => value.serialized_len(),
            Self::Afbr(value) => value.serialized_len(),
            Self::Dobr(value) => value.serialized_len(),
            Self::Pano(value) => value.serialized_len(),
            Self::Tols(value) => value.serialized_len(),
            Self::Wipe(value) => value.serialized_len(),
            Self::Zoom(value) => value.serialized_len(),
            Self::Fade(value) => value.serialized_len(),
            Self::Splt(value) => value.serialized_len(),
            Self::Stpe(value) => value.serialized_len(),
            Self::Ssld(value) => value.serialized_len(),
            Self::Prdi(value) => value.serialized_len(),
            Self::Sstr(value) => value.serialized_len(),
            Self::Cmex(value) => value.serialized_len(),
            Self::Cmin(value) => value.serialized_len(),
            Self::Cexg(value) => value.serialized_len(),
            Self::Dadj(value) => value.serialized_len(),
            Self::Stag(value) => value.serialized_len(),
            Self::Reve(value) => value.serialized_len(),
            Self::Ndwt(value) => value.serialized_len(),
            Self::MskC(value) => value.serialized_len(),
            Self::Subs(value) => value.serialized_len(),
            Self::Stvi(value) => value.serialized_len(),
            Self::Opaque(value) => value.serialized_len(),
        }
    }

    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        match self {
            Self::Pasp(value) => Self::write_body_box(buf, &PASP, value.serialized_len(), value),
            Self::Colr(value) => Self::write_body_box(buf, &COLR, value.serialized_len(), value),
            Self::Clap(value) => Self::write_body_box(buf, &CLAP, value.serialized_len(), value),
            Self::Av1C(value) => Self::write_body_box(buf, &AV1C, value.serialized_len(), value),
            Self::Mdcv(value) => Self::write_body_box(buf, &MDCV_FOURCC, MDCV_PAYLOAD_LEN, value),
            Self::Clli(value) => Self::write_body_box(buf, &CLLI_FOURCC, CLLI_PAYLOAD_LEN, value),
            Self::Ispe(value) => value.serialize_into(buf),
            Self::Pixi(value) => value.serialize_into(buf),
            Self::Rloc(value) => value.serialize_into(buf),
            Self::AuxC(value) => value.serialize_into(buf),
            Self::Irot(value) => value.serialize_into(buf),
            Self::Lsel(value) => value.serialize_into(buf),
            Self::Imir(value) => value.serialize_into(buf),
            Self::HvcC(value) => value.serialize_into(buf),
            Self::AvcC(value) => value.serialize_into(buf),
            Self::VvcC(value) => value.serialize_into(buf),
            Self::A1op(value) => value.serialize_into(buf),
            Self::A1lx(value) => value.serialize_into(buf),
            Self::Brnd(value) => value.serialize_into(buf),
            Self::Hdlp(value) => value.serialize_into(buf),
            Self::Scrb(value) => value.serialize_into(buf),
            Self::Amve(value) => Self::write_body_box(buf, &AMVE, value.serialized_len(), value),
            Self::Cclv(value) => Self::write_body_box(buf, &CCLV, value.serialized_len(), value),
            Self::Iscl(value) => value.serialize_into(buf),
            Self::Rref(value) => value.serialize_into(buf),
            Self::Crtt(value) => value.serialize_into(buf),
            Self::Mdft(value) => value.serialize_into(buf),
            Self::Udes(value) => value.serialize_into(buf),
            Self::Altt(value) => value.serialize_into(buf),
            Self::Aebr(value) => value.serialize_into(buf),
            Self::Wbbr(value) => value.serialize_into(buf),
            Self::Fobr(value) => value.serialize_into(buf),
            Self::Afbr(value) => value.serialize_into(buf),
            Self::Dobr(value) => value.serialize_into(buf),
            Self::Pano(value) => value.serialize_into(buf),
            Self::Tols(value) => value.serialize_into(buf),
            Self::Wipe(value) => value.serialize_into(buf),
            Self::Zoom(value) => value.serialize_into(buf),
            Self::Fade(value) => value.serialize_into(buf),
            Self::Splt(value) => value.serialize_into(buf),
            Self::Stpe(value) => value.serialize_into(buf),
            Self::Ssld(value) => value.serialize_into(buf),
            Self::Prdi(value) => value.serialize_into(buf),
            Self::Sstr(value) => value.serialize_into(buf),
            Self::Cmex(value) => value.serialize_into(buf),
            Self::Cmin(value) => value.serialize_into(buf),
            Self::Cexg(value) => value.serialize_into(buf),
            Self::Dadj(value) => value.serialize_into(buf),
            Self::Stag(value) => value.serialize_into(buf),
            Self::Reve(value) => value.serialize_into(buf),
            Self::Ndwt(value) => value.serialize_into(buf),
            Self::MskC(value) => value.serialize_into(buf),
            Self::Subs(value) => value.serialize_into(buf),
            Self::Stvi(value) => value.serialize_into(buf),
            Self::Opaque(value) => value.serialize_into(buf),
        }
    }
}

/// A derived image item's typed body (ISO/IEC 23008-12:2025 6.6.2, Amd 1
/// 6.6.2.4-6.6.2.5).
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub enum DerivedImage {
    /// `iden`: identity derivation (empty body).
    Identity(IdentityDerivation),
    /// `iovl`: image overlay.
    Overlay(ImageOverlay),
    /// `grid`: image grid.
    Grid(ImageGrid),
    /// `tmap`: tone-map derivation (Amd 1 6.6.2.4).
    ToneMap(ToneMapImage),
    /// `cfen`: colour format enhancement (Amd 1 6.6.2.5).
    ColourFormatEnhancement(ColourFormatEnhancement),
}

impl DerivedImage {
    /// Parse the body of an item whose type is `item_type`. `reference_count`
    /// is the number of `dimg` references the item has, which an overlay body
    /// needs (§6.6.2.2.2); other derived types ignore it.
    pub fn parse(
        item_type: crate::item::ItemType,
        body: &[u8],
        reference_count: usize,
    ) -> Result<Self> {
        use crate::item::ItemType;
        match item_type {
            ItemType::Iden => Ok(Self::Identity(IdentityDerivation::parse(body)?)),
            ItemType::Iovl => Ok(Self::Overlay(ImageOverlay::parse_with_reference_count(
                body,
                reference_count,
            )?)),
            ItemType::Grid => Ok(Self::Grid(ImageGrid::parse(body)?)),
            ItemType::Tmap => {
                let tone_map = ToneMapImage::parse(body)?;
                if tone_map.version() != 0 {
                    return Err(Error::UnsupportedFeature("tmap version"));
                }
                Ok(Self::ToneMap(tone_map))
            }
            ItemType::Cfen => {
                let enhancement =
                    ColourFormatEnhancement::parse_with_reference_count(body, reference_count)?;
                if enhancement.version() != 0 {
                    return Err(Error::UnsupportedFeature("cfen version"));
                }
                Ok(Self::ColourFormatEnhancement(enhancement))
            }
            _ => Err(Error::UnsupportedFeature(
                "item type is not a derived image",
            )),
        }
    }

    /// The body length.
    #[must_use]
    pub fn serialized_len(&self) -> usize {
        match self {
            Self::Identity(value) => value.serialized_len(),
            Self::Overlay(value) => value.serialized_len(),
            Self::Grid(value) => value.serialized_len(),
            Self::ToneMap(value) => value.serialized_len(),
            Self::ColourFormatEnhancement(value) => value.serialized_len(),
        }
    }

    /// Write the body.
    pub fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        match self {
            Self::Identity(value) => {
                need_out(buf, value.serialized_len())?;
                Ok(value.serialized_len())
            }
            Self::Overlay(value) => value.serialize_into(buf),
            Self::Grid(value) => value.serialize_into(buf),
            Self::ToneMap(value) => value.serialize_into(buf),
            Self::ColourFormatEnhancement(value) => value.serialize_into(buf),
        }
    }
}

impl DerivedImage {
    /// The body bytes, for a writer that adds a derived item whole.
    pub fn to_bytes(&self) -> Result<Vec<u8>> {
        let mut out = alloc::vec![0u8; self.serialized_len()];
        let written = self.serialize_into(&mut out)?;
        out.truncate(written);
        Ok(out)
    }
}
