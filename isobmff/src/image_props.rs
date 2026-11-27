//! The item properties ISO/IEC 23008-12:2025 clause 6.5 adds beyond the 2017
//! set, and the mask configuration property clause 11.2.2 defines.
//!
//! | Type | Four-CC | Clause |
//! |------|---------|--------|
//! | [`ImageScaling`] | `iscl` | 6.5.13 |
//! | [`RequiredReferenceTypesProperty`] | `rref` | 6.5.17 |
//! | [`CreationTimeProperty`] | `crtt` | 6.5.18 |
//! | [`ModificationTimeProperty`] | `mdft` | 6.5.19 |
//! | [`UserDescriptionProperty`] | `udes` | 6.5.20 |
//! | [`AccessibilityTextProperty`] | `altt` | 6.5.21 |
//! | [`AutoExposureProperty`] | `aebr` | 6.5.22 |
//! | [`WhiteBalanceProperty`] | `wbbr` | 6.5.23 |
//! | [`FocusProperty`] | `fobr` | 6.5.24 |
//! | [`FlashExposureProperty`] | `afbr` | 6.5.25 |
//! | [`DepthOfFieldProperty`] | `dobr` | 6.5.26 |
//! | [`PanoramaProperty`] | `pano` | 6.5.27 |
//! | [`TargetOutputLayerSetProperty`] | `tols` | 6.5.29 |
//! | [`WipeTransitionEffectProperty`] | `wipe` | 6.5.30 |
//! | [`ZoomTransitionEffectProperty`] | `zoom` | 6.5.31 |
//! | [`FadeTransitionEffectProperty`] | `fade` | 6.5.32 |
//! | [`SplitTransitionEffectProperty`] | `splt` | 6.5.33 |
//! | [`SuggestedTransitionPeriodProperty`] | `stpe` | 6.5.34 |
//! | [`SuggestedTimeDisplayDurationProperty`] | `ssld` | 6.5.35 |
//! | [`ProgressiveDerivedImageItemInformationProperty`] | `prdi` | 6.5.37 |
//! | [`SingleStreamProperty`] | `sstr` | 6.5.38 |
//! | [`CameraExtrinsicMatrixProperty`] | `cmex` | 6.5.39 |
//! | [`CameraIntrinsicMatrixProperty`] | `cmin` | 6.5.40 |
//! | [`ConstrainedExtentsGridProperty`] | `cexg` | Amd 1 6.5.41 |
//! | [`DisparityAdjustmentProperty`] | `dadj` | Amd 1 6.5.42 |
//! | [`StereoAggressorsProperty`] | `stag` | Amd 1 6.5.43 |
//! | [`ReferenceViewingEnvironmentProperty`] | `reve` | Amd 1 6.5.44 |
//! | [`NominalDiffuseWhiteProperty`] | `ndwt` | Amd 1 6.5.45 |
//! | [`MaskConfigurationProperty`] | `mskC` | 11.2.2.2 |
//!
//! Properties whose syntax ISO/IEC 14496-12:2026 already defines (`clli`, `mdcv`,
//! `cclv`, `amve`) are reused from [`crate::hdr_boxes`] and
//! [`crate::visual_ext`], never re-declared. The camera extrinsic orientation
//! for version 1 is the `ViewpointGlobalCoordinateSysRotationStruct` of
//! ISO/IEC 23090-7, which is not held, so it and everything after it are kept
//! opaque ([`CameraExtrinsicMatrixProperty::opaque_tail`]); the
//! `ExtentDecoderConfigurationRecord` of the constrained extents grid is
//! coding-format specific and likewise opaque.
//!
//! Sources: `private/specs-verified/mpeg/heif/iso-iec-23008-12-2025.md` clause
//! 6.5 and `private/specs-verified/mpeg/heif/iso-iec-23008-12-2025-amd1-2025.md`
//! clauses 6.5.41-6.5.45.

use alloc::vec::Vec;

use broadcast_core::len::{fit_mask, fit_u8, need_in, need_out};
use broadcast_core::wire::be_u64;
use broadcast_core::wire::{be_u16, be_u32, i16_at, i32_at};
use broadcast_core::{Parse, Serialize};

use crate::box_types::{
    BOX_HEADER_MIN_SIZE, BOX_TYPE_SIZE, FULLBOX_EXTRA_SIZE, box_len, write_box_header,
    write_full_box_header,
};
use crate::error::{Error, Result};
use crate::image::{body_of, expect_type, full_flags};

/// Width of a `u8` field.
const U8_LEN: usize = core::mem::size_of::<u8>();
/// Width of a `u16` field.
const U16_LEN: usize = core::mem::size_of::<u16>();
/// Width of a `u32` field.
const U32_LEN: usize = core::mem::size_of::<u32>();
/// Width of a `u64` field.
const U64_LEN: usize = core::mem::size_of::<u64>();
/// Width of a two-field `u8` pair.
const U8_PAIR_LEN: usize = 2 * U8_LEN;
/// Width of a two-field `u16` pair.
const U16_PAIR_LEN: usize = 2 * U16_LEN;
/// Width of a three-field `u16` group.
const U16_TRIPLE_LEN: usize = 3 * U16_LEN;
/// Width of a two-field `u32` pair.
const U32_PAIR_LEN: usize = 2 * U32_LEN;
/// Offset of `iscl`'s height numerator after its width pair.
const ISCL_HEIGHT_OFFSET: usize = U16_PAIR_LEN;
/// Offset of `iscl`'s height denominator.
const ISCL_HEIGHT_DENOMINATOR_OFFSET: usize = U16_TRIPLE_LEN;
/// Components of a camera quaternion.
const QUATERNION_COMPONENTS: usize = 3;
/// Fields of the simplified camera intrinsic matrix.
const CMIN_SIMPLIFIED_FIELDS: usize = 3;
/// Fields of the full camera intrinsic matrix.
const CMIN_FULL_FIELDS: usize = 5;
/// Fields of the `cexg` grid dimensions.
const CEXG_GRID_FIELDS: usize = 2;
/// Fields of one stereo aggressor before its optional URI.
const STAG_ENTRY_FIELDS: usize = 2;
/// Strings a `udes` property carries.
const UDES_STRING_COUNT: usize = 4;
/// Strings an `altt` property carries.
const ALTT_STRING_COUNT: usize = 2;

/// One `iscl` ratio pair (numerator and denominator).
const SCALING_FIELD_COUNT: usize = 4;
/// `iscl` body length: four `u16` ratios.
const ISCL_LEN: usize = SCALING_FIELD_COUNT * U16_LEN;
/// `rref` reference-type width (6.5.17.2).
const RREF_TYPE_LEN: usize = U32_LEN;

/// `prdi` flag: candidates follow the `dimg` declaration order (6.5.37.1).
pub const PRDI_ITEM_REFERENCE_ORDER: u32 = 0x000001;
/// `prdi` flag: each progressive step consumes one candidate (6.5.37.1).
pub const PRDI_SINGLE_CANDIDATE_PER_STEP: u32 = 0x000002;
/// `prdi` flag: an alternative to an input is a candidate (6.5.37.1).
pub const PRDI_ALTERNATIVE_IS_CANDIDATE: u32 = 0x000004;

/// `cmex` flag: `pos_x` present (6.5.39.1).
pub const CMEX_POS_X: u32 = 0x000001;
/// `cmex` flag: `pos_y` present (6.5.39.1).
pub const CMEX_POS_Y: u32 = 0x000002;
/// `cmex` flag: `pos_z` present (6.5.39.1).
pub const CMEX_POS_Z: u32 = 0x000004;
/// `cmex` flag: orientation present (6.5.39.1).
pub const CMEX_ORIENTATION: u32 = 0x000008;
/// `cmex` flag: orientation elements are 32-bit (6.5.39.1).
pub const CMEX_ROT_LARGE_FIELD: u32 = 0x000010;
/// `cmex` flag: the world coordinate system `id` is present (6.5.39.1).
pub const CMEX_ID: u32 = 0x000020;

/// `cmin` flag: full intrinsics (6.5.40.2).
pub const CMIN_FULL_INTRINSICS: u32 = 0x000001;
/// `cmin` flags mask of the denominator shift operand (6.5.40.1).
pub const CMIN_DENOMINATOR_MASK: u32 = 0x001F00;
/// Shift of the denominator operand inside the `cmin` flags (6.5.40.1).
pub const CMIN_DENOMINATOR_SHIFT: u32 = 8;
/// `cmin` flags mask of the skew denominator shift operand (6.5.40.1).
pub const CMIN_SKEW_DENOMINATOR_MASK: u32 = 0x1F0000;
/// Shift of the skew denominator operand inside the `cmin` flags (6.5.40.1).
pub const CMIN_SKEW_DENOMINATOR_SHIFT: u32 = 16;

/// `cexg` flag: 32-bit tile dimensions (Amd 1 6.5.41.1).
pub const CEXG_FIELD_LENGTH: u32 = 0x000001;
/// `cexg` flag: per-extent decoder configuration present (Amd 1 6.5.41.1).
pub const CEXG_TILE_CONFIG_INFO_PRESENT: u32 = 0x000002;

/// `zoom` direction bit (6.5.31.2).
const ZOOM_DIRECTION_MASK: u8 = 0x80;
/// `zoom` shape mask (6.5.31.2).
const ZOOM_SHAPE_MASK: u8 = 0x7f;
/// `stag` sub-type-present bit (Amd 1 6.5.43.2).
const STAG_SUB_TYPE_MASK: u8 = 0x80;
/// `stag` severity mask (Amd 1 6.5.43.2).
const STAG_SEVERITY_MASK: u8 = 0x7f;

/// `pano` body length without the optional grid fields (6.5.27.2).
const PANO_BASE_LEN: usize = U8_LEN;
/// `pano` body length with the optional grid fields (6.5.27.2).
const PANO_GRID_LEN: usize = PANO_BASE_LEN + 2 * U8_LEN;
/// `pano` direction of the raster-scan grid panorama (6.5.27.3).
pub const PANO_DIRECTION_GRID: u8 = 4;
/// `pano` direction of the continuous-order grid panorama (6.5.27.3).
pub const PANO_DIRECTION_GRID_CONTINUOUS: u8 = 5;

/// Parse one full-box property body, returning the body slice (FullBox word
/// included).
fn full_body<'a>(
    bytes: &'a [u8],
    fourcc: &[u8; BOX_TYPE_SIZE],
    min_body: usize,
    what: &'static str,
) -> Result<&'a [u8]> {
    expect_type(bytes, fourcc, what)?;
    body_of(bytes, min_body, what)
}

/// The body of a property whose clause fixes its whole FullBox word at
/// `version = 0, flags = 0` (6.5): a non-zero word is a typed error rather
/// than a byte a rewrite would silently change.
fn zero_full_body<'a>(
    bytes: &'a [u8],
    fourcc: &[u8; BOX_TYPE_SIZE],
    min_body: usize,
    what: &'static str,
) -> Result<&'a [u8]> {
    let body = full_body(bytes, fourcc, min_body, what)?;
    if body.get(..FULLBOX_EXTRA_SIZE) != Some(&[0u8; FULLBOX_EXTRA_SIZE][..]) {
        return Err(Error::InvalidInput(what));
    }
    Ok(body)
}

/// [`zero_full_body`] for a property whose body is exactly `body_len` bytes
/// (FullBox word included): a tail past the last field is a typed error.
fn exact_full_body<'a>(
    bytes: &'a [u8],
    fourcc: &[u8; BOX_TYPE_SIZE],
    body_len: usize,
    what: &'static str,
) -> Result<&'a [u8]> {
    let body = zero_full_body(bytes, fourcc, body_len, what)?;
    if body.len() != body_len {
        return Err(Error::InvalidInput(what));
    }
    Ok(body)
}

/// Reject a `version` the clause fixes at 0 where the `flags` word is free
/// (6.5.37, 6.5.40, Amd 1 6.6.2.5).
fn expect_version_zero(body: &[u8], what: &'static str) -> Result<()> {
    if body.first() != Some(&0) {
        return Err(Error::InvalidInput(what));
    }
    Ok(())
}

/// Split exactly `count` null-terminated byte strings out of `body`.
pub(crate) fn split_cstrings(
    body: &[u8],
    count: usize,
    what: &'static str,
) -> Result<Vec<Vec<u8>>> {
    let mut out = Vec::with_capacity(count);
    let mut cursor = 0usize;
    for _ in 0..count {
        let rest = body
            .get(cursor..)
            .ok_or(Error::InvalidInput("string property body"))?;
        let end = rest
            .iter()
            .position(|byte| *byte == 0)
            .ok_or(Error::InvalidInput(what))?;
        out.push(rest[..end].to_vec());
        cursor = cursor
            .checked_add(end)
            .and_then(|value| value.checked_add(1))
            .ok_or(Error::InvalidInput(what))?;
    }
    if cursor != body.len() {
        return Err(Error::InvalidInput(what));
    }
    Ok(out)
}

/// The wire length of a set of null-terminated strings.
pub(crate) fn cstrings_len(strings: &[Vec<u8>]) -> usize {
    strings.iter().map(|value| value.len() + 1).sum()
}

/// Write null-terminated strings at `buf`.
pub(crate) fn write_cstrings(buf: &mut [u8], strings: &[Vec<u8>]) -> Result<usize> {
    let mut cursor = 0usize;
    for value in strings {
        let end = cursor
            .checked_add(value.len() + 1)
            .ok_or(Error::InvalidInput("string property length"))?;
        need_out(buf, end)?;
        buf[cursor..cursor + value.len()].copy_from_slice(value);
        buf[cursor + value.len()] = 0;
        cursor = end;
    }
    Ok(cursor)
}

/// Write a full-box property whose body is `body_len` bytes, returning the
/// cursor after the whole box. The caller writes the body at the cursor.
fn begin_fullbox(
    buf: &mut [u8],
    fourcc: &[u8; BOX_TYPE_SIZE],
    version: u8,
    flags: u32,
    body_len: usize,
) -> Result<usize> {
    let need = box_len(BOX_HEADER_MIN_SIZE + FULLBOX_EXTRA_SIZE + body_len);
    need_out(buf, need)?;
    write_full_box_header(buf, fourcc, need, version, flags, false)
}

/// Four-CC of the Image Scaling property (6.5.13).
pub const ISCL: [u8; BOX_TYPE_SIZE] = *b"iscl";
/// Four-CC of the Required Reference Types property (6.5.17).
pub const RREF: [u8; BOX_TYPE_SIZE] = *b"rref";
/// Four-CC of the Creation Time property (6.5.18).
pub const CRTT: [u8; BOX_TYPE_SIZE] = *b"crtt";
/// Four-CC of the Modification Time property (6.5.19).
pub const MDFT: [u8; BOX_TYPE_SIZE] = *b"mdft";
/// Four-CC of the User Description property (6.5.20).
pub const UDES: [u8; BOX_TYPE_SIZE] = *b"udes";
/// Four-CC of the Accessibility Text property (6.5.21).
pub const ALTT: [u8; BOX_TYPE_SIZE] = *b"altt";

/// Image Scaling property (`iscl`) — ISO/IEC 23008-12:2025 6.5.13.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub struct ImageScaling {
    target_width_numerator: u16,
    target_width_denominator: u16,
    target_height_numerator: u16,
    target_height_denominator: u16,
}

impl ImageScaling {
    /// A scaling to the ratio `numerator`/`denominator` (6.5.13.3).
    #[must_use]
    pub const fn new(
        target_width_numerator: u16,
        target_width_denominator: u16,
        target_height_numerator: u16,
        target_height_denominator: u16,
    ) -> Self {
        Self {
            target_width_numerator,
            target_width_denominator,
            target_height_numerator,
            target_height_denominator,
        }
    }

    /// `target_width_numerator`.
    #[must_use]
    pub const fn target_width_numerator(&self) -> u16 {
        self.target_width_numerator
    }

    /// `target_width_denominator`.
    #[must_use]
    pub const fn target_width_denominator(&self) -> u16 {
        self.target_width_denominator
    }

    /// `target_height_numerator`.
    #[must_use]
    pub const fn target_height_numerator(&self) -> u16 {
        self.target_height_numerator
    }

    /// `target_height_denominator`.
    #[must_use]
    pub const fn target_height_denominator(&self) -> u16 {
        self.target_height_denominator
    }
}

impl<'a> Parse<'a> for ImageScaling {
    type Error = Error;

    fn parse(bytes: &'a [u8]) -> Result<Self> {
        let body = exact_full_body(bytes, &ISCL, FULLBOX_EXTRA_SIZE + ISCL_LEN, "iscl")?;
        let at = FULLBOX_EXTRA_SIZE;
        Ok(Self {
            target_width_numerator: be_u16(body, at, "iscl width numerator")?,
            target_width_denominator: be_u16(body, at + U16_LEN, "iscl width denominator")?,
            target_height_numerator: be_u16(
                body,
                at + ISCL_HEIGHT_OFFSET,
                "iscl height numerator",
            )?,
            target_height_denominator: be_u16(
                body,
                at + ISCL_HEIGHT_DENOMINATOR_OFFSET,
                "iscl height denominator",
            )?,
        })
    }
}

impl Serialize for ImageScaling {
    type Error = Error;

    fn serialized_len(&self) -> usize {
        box_len(BOX_HEADER_MIN_SIZE + FULLBOX_EXTRA_SIZE + ISCL_LEN)
    }

    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        let mut cursor = begin_fullbox(buf, &ISCL, 0, 0, ISCL_LEN)?;
        for value in [
            self.target_width_numerator,
            self.target_width_denominator,
            self.target_height_numerator,
            self.target_height_denominator,
        ] {
            buf[cursor..cursor + U16_LEN].copy_from_slice(&value.to_be_bytes());
            cursor += U16_LEN;
        }
        Ok(cursor)
    }
}

/// Required Reference Types property (`rref`) — ISO/IEC 23008-12:2025 6.5.17.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub struct RequiredReferenceTypesProperty {
    reference_types: Vec<[u8; BOX_TYPE_SIZE]>,
}

impl RequiredReferenceTypesProperty {
    /// A property listing the required reference types.
    #[must_use]
    pub fn new(reference_types: Vec<[u8; BOX_TYPE_SIZE]>) -> Self {
        Self { reference_types }
    }

    /// The required reference types, in order.
    #[must_use]
    pub fn reference_types(&self) -> &[[u8; BOX_TYPE_SIZE]] {
        &self.reference_types
    }
}

impl<'a> Parse<'a> for RequiredReferenceTypesProperty {
    type Error = Error;

    fn parse(bytes: &'a [u8]) -> Result<Self> {
        let body = zero_full_body(bytes, &RREF, FULLBOX_EXTRA_SIZE + U8_LEN, "rref")?;
        let count = usize::from(body[FULLBOX_EXTRA_SIZE]);
        let start = FULLBOX_EXTRA_SIZE + U8_LEN;
        crate::box_support::check_entry_count(body.len() - start, RREF_TYPE_LEN, count, "rref")?;
        let mut reference_types = Vec::with_capacity(count);
        for index in 0..count {
            let at = start + index * RREF_TYPE_LEN;
            let fourcc = broadcast_core::wire::fourcc_at(body, at);
            reference_types.push(fourcc);
        }
        if start + count * RREF_TYPE_LEN != body.len() {
            return Err(Error::InvalidInput("rref trailing bytes"));
        }
        Ok(Self { reference_types })
    }
}

impl Serialize for RequiredReferenceTypesProperty {
    type Error = Error;

    fn serialized_len(&self) -> usize {
        box_len(
            BOX_HEADER_MIN_SIZE
                + FULLBOX_EXTRA_SIZE
                + U8_LEN
                + self.reference_types.len() * RREF_TYPE_LEN,
        )
    }

    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        let body_len = U8_LEN + self.reference_types.len() * RREF_TYPE_LEN;
        let mut cursor = begin_fullbox(buf, &RREF, 0, 0, body_len)?;
        buf[cursor] = fit_u8(self.reference_types.len(), "rref count")?;
        cursor += U8_LEN;
        for fourcc in &self.reference_types {
            buf[cursor..cursor + RREF_TYPE_LEN].copy_from_slice(fourcc);
            cursor += RREF_TYPE_LEN;
        }
        Ok(cursor)
    }
}

/// Creation Time property (`crtt`) — ISO/IEC 23008-12:2025 6.5.18.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub struct CreationTimeProperty {
    creation_time: u64,
}

impl CreationTimeProperty {
    /// A property documenting `creation_time` in microseconds since 1904-01-01.
    #[must_use]
    pub const fn new(creation_time: u64) -> Self {
        Self { creation_time }
    }

    /// `creation_time`.
    #[must_use]
    pub const fn creation_time(&self) -> u64 {
        self.creation_time
    }
}

impl<'a> Parse<'a> for CreationTimeProperty {
    type Error = Error;

    fn parse(bytes: &'a [u8]) -> Result<Self> {
        let body = exact_full_body(bytes, &CRTT, FULLBOX_EXTRA_SIZE + U64_LEN, "crtt")?;
        Ok(Self {
            creation_time: be_u64(body, FULLBOX_EXTRA_SIZE, "crtt creation_time")?,
        })
    }
}

impl Serialize for CreationTimeProperty {
    type Error = Error;

    fn serialized_len(&self) -> usize {
        box_len(BOX_HEADER_MIN_SIZE + FULLBOX_EXTRA_SIZE + U64_LEN)
    }

    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        let mut cursor = begin_fullbox(buf, &CRTT, 0, 0, U64_LEN)?;
        buf[cursor..cursor + U64_LEN].copy_from_slice(&self.creation_time.to_be_bytes());
        cursor += U64_LEN;
        Ok(cursor)
    }
}

/// Modification Time property (`mdft`) — ISO/IEC 23008-12:2025 6.5.19.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub struct ModificationTimeProperty {
    modification_time: u64,
}

impl ModificationTimeProperty {
    /// A property documenting `modification_time` in microseconds since
    /// 1904-01-01.
    #[must_use]
    pub const fn new(modification_time: u64) -> Self {
        Self { modification_time }
    }

    /// `modification_time`.
    #[must_use]
    pub const fn modification_time(&self) -> u64 {
        self.modification_time
    }
}

impl<'a> Parse<'a> for ModificationTimeProperty {
    type Error = Error;

    fn parse(bytes: &'a [u8]) -> Result<Self> {
        let body = exact_full_body(bytes, &MDFT, FULLBOX_EXTRA_SIZE + U64_LEN, "mdft")?;
        Ok(Self {
            modification_time: be_u64(body, FULLBOX_EXTRA_SIZE, "mdft modification_time")?,
        })
    }
}

impl Serialize for ModificationTimeProperty {
    type Error = Error;

    fn serialized_len(&self) -> usize {
        box_len(BOX_HEADER_MIN_SIZE + FULLBOX_EXTRA_SIZE + U64_LEN)
    }

    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        let mut cursor = begin_fullbox(buf, &MDFT, 0, 0, U64_LEN)?;
        buf[cursor..cursor + U64_LEN].copy_from_slice(&self.modification_time.to_be_bytes());
        cursor += U64_LEN;
        Ok(cursor)
    }
}

/// User Description property (`udes`) — ISO/IEC 23008-12:2025 6.5.20.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub struct UserDescriptionProperty {
    lang: Vec<u8>,
    name: Vec<u8>,
    description: Vec<u8>,
    tags: Vec<u8>,
}

impl UserDescriptionProperty {
    /// A description in `lang` with `name`, `description` and `tags` (each may
    /// be empty, as §6.5.20.3 defines).
    #[must_use]
    pub fn new(lang: &[u8], name: &[u8], description: &[u8], tags: &[u8]) -> Self {
        Self {
            lang: lang.to_vec(),
            name: name.to_vec(),
            description: description.to_vec(),
            tags: tags.to_vec(),
        }
    }

    /// `lang`, an RFC 5646 language tag (empty when unknown).
    #[must_use]
    pub fn lang(&self) -> &[u8] {
        &self.lang
    }

    /// `name`.
    #[must_use]
    pub fn name_bytes(&self) -> &[u8] {
        &self.name
    }

    /// `description`.
    #[must_use]
    pub fn description(&self) -> &[u8] {
        &self.description
    }

    /// `tags`, comma separated.
    #[must_use]
    pub fn tags(&self) -> &[u8] {
        &self.tags
    }

    fn strings(&self) -> [&Vec<u8>; UDES_STRING_COUNT] {
        [&self.lang, &self.name, &self.description, &self.tags]
    }
}

impl<'a> Parse<'a> for UserDescriptionProperty {
    type Error = Error;

    fn parse(bytes: &'a [u8]) -> Result<Self> {
        let body = zero_full_body(bytes, &UDES, FULLBOX_EXTRA_SIZE, "udes")?;
        let mut strings = split_cstrings(
            &body[FULLBOX_EXTRA_SIZE..],
            UDES_STRING_COUNT,
            "udes strings",
        )?
        .into_iter();
        let lang = strings.next().unwrap_or_default();
        let name = strings.next().unwrap_or_default();
        let description = strings.next().unwrap_or_default();
        let tags = strings.next().unwrap_or_default();
        Ok(Self {
            lang,
            name,
            description,
            tags,
        })
    }
}

impl Serialize for UserDescriptionProperty {
    type Error = Error;

    fn serialized_len(&self) -> usize {
        box_len(BOX_HEADER_MIN_SIZE + FULLBOX_EXTRA_SIZE + cstrings_len(&self.as_vecs()))
    }

    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        let strings = self.as_vecs();
        let body_len = cstrings_len(&strings);
        let mut cursor = begin_fullbox(buf, &UDES, 0, 0, body_len)?;
        cursor += write_cstrings(&mut buf[cursor..], &strings)?;
        Ok(cursor)
    }
}

impl UserDescriptionProperty {
    fn as_vecs(&self) -> Vec<Vec<u8>> {
        self.strings()
            .iter()
            .map(|value| (*value).clone())
            .collect()
    }
}

/// Accessibility Text property (`altt`) — ISO/IEC 23008-12:2025 6.5.21.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub struct AccessibilityTextProperty {
    alt_text: Vec<u8>,
    alt_lang: Vec<u8>,
}

impl AccessibilityTextProperty {
    /// Alternate text `alt_text` in `alt_lang`.
    #[must_use]
    pub fn new(alt_text: &[u8], alt_lang: &[u8]) -> Self {
        Self {
            alt_text: alt_text.to_vec(),
            alt_lang: alt_lang.to_vec(),
        }
    }

    /// `alt_text`.
    #[must_use]
    pub fn alt_text(&self) -> &[u8] {
        &self.alt_text
    }

    /// `alt_lang`, an RFC 5646 language tag (empty when unknown).
    #[must_use]
    pub fn alt_lang(&self) -> &[u8] {
        &self.alt_lang
    }
}

impl<'a> Parse<'a> for AccessibilityTextProperty {
    type Error = Error;

    fn parse(bytes: &'a [u8]) -> Result<Self> {
        let body = zero_full_body(bytes, &ALTT, FULLBOX_EXTRA_SIZE, "altt")?;
        let mut strings = split_cstrings(
            &body[FULLBOX_EXTRA_SIZE..],
            ALTT_STRING_COUNT,
            "altt strings",
        )?
        .into_iter();
        Ok(Self {
            alt_text: strings.next().unwrap_or_default(),
            alt_lang: strings.next().unwrap_or_default(),
        })
    }
}

impl Serialize for AccessibilityTextProperty {
    type Error = Error;

    fn serialized_len(&self) -> usize {
        box_len(
            BOX_HEADER_MIN_SIZE
                + FULLBOX_EXTRA_SIZE
                + self.alt_text.len()
                + 1
                + self.alt_lang.len()
                + 1,
        )
    }

    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        let strings = alloc::vec![self.alt_text.clone(), self.alt_lang.clone()];
        let body_len = cstrings_len(&strings);
        let mut cursor = begin_fullbox(buf, &ALTT, 0, 0, body_len)?;
        cursor += write_cstrings(&mut buf[cursor..], &strings)?;
        Ok(cursor)
    }
}

/// Auto Exposure property (`aebr`) — ISO/IEC 23008-12:2025 6.5.22.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub struct AutoExposureProperty {
    exposure_step: i8,
    exposure_numerator: i8,
}

impl AutoExposureProperty {
    /// An exposure variation of `exposure_numerator`/`exposure_step` stops.
    #[must_use]
    pub const fn new(exposure_step: i8, exposure_numerator: i8) -> Self {
        Self {
            exposure_step,
            exposure_numerator,
        }
    }

    /// `exposure_step`.
    #[must_use]
    pub const fn exposure_step(&self) -> i8 {
        self.exposure_step
    }

    /// `exposure_numerator`.
    #[must_use]
    pub const fn exposure_numerator(&self) -> i8 {
        self.exposure_numerator
    }
}

impl<'a> Parse<'a> for AutoExposureProperty {
    type Error = Error;

    fn parse(bytes: &'a [u8]) -> Result<Self> {
        let body = exact_full_body(bytes, &AEBR, FULLBOX_EXTRA_SIZE + U8_PAIR_LEN, "aebr")?;
        Ok(Self {
            exposure_step: body[FULLBOX_EXTRA_SIZE] as i8,
            exposure_numerator: body[FULLBOX_EXTRA_SIZE + U8_LEN] as i8,
        })
    }
}

impl Serialize for AutoExposureProperty {
    type Error = Error;

    fn serialized_len(&self) -> usize {
        box_len(BOX_HEADER_MIN_SIZE + FULLBOX_EXTRA_SIZE + U8_PAIR_LEN)
    }

    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        let mut cursor = begin_fullbox(buf, &AEBR, 0, 0, U8_PAIR_LEN)?;
        buf[cursor] = self.exposure_step as u8;
        buf[cursor + U8_LEN] = self.exposure_numerator as u8;
        cursor += U8_PAIR_LEN;
        Ok(cursor)
    }
}

/// Four-CC of the Auto Exposure property (6.5.22).
pub const AEBR: [u8; BOX_TYPE_SIZE] = *b"aebr";
/// Four-CC of the White Balance property (6.5.23).
pub const WBBR: [u8; BOX_TYPE_SIZE] = *b"wbbr";
/// Four-CC of the Focus property (6.5.24).
pub const FOBR: [u8; BOX_TYPE_SIZE] = *b"fobr";
/// Four-CC of the Flash Exposure property (6.5.25).
pub const AFBR: [u8; BOX_TYPE_SIZE] = *b"afbr";
/// Four-CC of the Depth of Field property (6.5.26).
pub const DOBR: [u8; BOX_TYPE_SIZE] = *b"dobr";
/// Four-CC of the Panorama property (6.5.27).
pub const PANO: [u8; BOX_TYPE_SIZE] = *b"pano";
/// Four-CC of the Target Output Layer Set property (6.5.29).
pub const TOLS: [u8; BOX_TYPE_SIZE] = *b"tols";

/// White Balance property (`wbbr`) — ISO/IEC 23008-12:2025 6.5.23.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub struct WhiteBalanceProperty {
    blue_amber: u16,
    green_magenta: i8,
}

impl WhiteBalanceProperty {
    /// A white balance of `blue_amber` kelvin and `green_magenta` 1/100 Duv.
    #[must_use]
    pub const fn new(blue_amber: u16, green_magenta: i8) -> Self {
        Self {
            blue_amber,
            green_magenta,
        }
    }

    /// `blue_amber`.
    #[must_use]
    pub const fn blue_amber(&self) -> u16 {
        self.blue_amber
    }

    /// `green_magenta`.
    #[must_use]
    pub const fn green_magenta(&self) -> i8 {
        self.green_magenta
    }
}

impl<'a> Parse<'a> for WhiteBalanceProperty {
    type Error = Error;

    fn parse(bytes: &'a [u8]) -> Result<Self> {
        let body = exact_full_body(bytes, &WBBR, FULLBOX_EXTRA_SIZE + U16_LEN + U8_LEN, "wbbr")?;
        Ok(Self {
            blue_amber: be_u16(body, FULLBOX_EXTRA_SIZE, "wbbr blue_amber")?,
            green_magenta: body[FULLBOX_EXTRA_SIZE + U16_LEN] as i8,
        })
    }
}

impl Serialize for WhiteBalanceProperty {
    type Error = Error;

    fn serialized_len(&self) -> usize {
        box_len(BOX_HEADER_MIN_SIZE + FULLBOX_EXTRA_SIZE + U16_LEN + U8_LEN)
    }

    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        let body_len = U16_LEN + U8_LEN;
        let mut cursor = begin_fullbox(buf, &WBBR, 0, 0, body_len)?;
        buf[cursor..cursor + U16_LEN].copy_from_slice(&self.blue_amber.to_be_bytes());
        cursor += U16_LEN;
        buf[cursor] = self.green_magenta as u8;
        cursor += U8_LEN;
        Ok(cursor)
    }
}

/// Focus property (`fobr`) — ISO/IEC 23008-12:2025 6.5.24.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub struct FocusProperty {
    focus_distance_numerator: u16,
    focus_distance_denominator: u16,
}

impl FocusProperty {
    /// A focus distance of `numerator`/`denominator` metres (a zero
    /// denominator is focus at infinity, §6.5.24.3).
    #[must_use]
    pub const fn new(focus_distance_numerator: u16, focus_distance_denominator: u16) -> Self {
        Self {
            focus_distance_numerator,
            focus_distance_denominator,
        }
    }

    /// `focus_distance_numerator`.
    #[must_use]
    pub const fn focus_distance_numerator(&self) -> u16 {
        self.focus_distance_numerator
    }

    /// `focus_distance_denominator`.
    #[must_use]
    pub const fn focus_distance_denominator(&self) -> u16 {
        self.focus_distance_denominator
    }
}

impl<'a> Parse<'a> for FocusProperty {
    type Error = Error;

    fn parse(bytes: &'a [u8]) -> Result<Self> {
        let body = exact_full_body(bytes, &FOBR, FULLBOX_EXTRA_SIZE + U16_PAIR_LEN, "fobr")?;
        Ok(Self {
            focus_distance_numerator: be_u16(body, FULLBOX_EXTRA_SIZE, "fobr numerator")?,
            focus_distance_denominator: be_u16(
                body,
                FULLBOX_EXTRA_SIZE + U16_LEN,
                "fobr denominator",
            )?,
        })
    }
}

impl Serialize for FocusProperty {
    type Error = Error;

    fn serialized_len(&self) -> usize {
        box_len(BOX_HEADER_MIN_SIZE + FULLBOX_EXTRA_SIZE + U16_PAIR_LEN)
    }

    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        let mut cursor = begin_fullbox(buf, &FOBR, 0, 0, U16_PAIR_LEN)?;
        buf[cursor..cursor + U16_LEN].copy_from_slice(&self.focus_distance_numerator.to_be_bytes());
        cursor += U16_LEN;
        buf[cursor..cursor + U16_LEN]
            .copy_from_slice(&self.focus_distance_denominator.to_be_bytes());
        cursor += U16_LEN;
        Ok(cursor)
    }
}

/// Flash Exposure property (`afbr`) — ISO/IEC 23008-12:2025 6.5.25.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub struct FlashExposureProperty {
    flash_exposure_numerator: i8,
    flash_exposure_denominator: i8,
}

impl FlashExposureProperty {
    /// A flash exposure of `numerator`/`denominator` f-stops.
    #[must_use]
    pub const fn new(flash_exposure_numerator: i8, flash_exposure_denominator: i8) -> Self {
        Self {
            flash_exposure_numerator,
            flash_exposure_denominator,
        }
    }

    /// `flash_exposure_numerator`.
    #[must_use]
    pub const fn flash_exposure_numerator(&self) -> i8 {
        self.flash_exposure_numerator
    }

    /// `flash_exposure_denominator`.
    #[must_use]
    pub const fn flash_exposure_denominator(&self) -> i8 {
        self.flash_exposure_denominator
    }
}

impl<'a> Parse<'a> for FlashExposureProperty {
    type Error = Error;

    fn parse(bytes: &'a [u8]) -> Result<Self> {
        let body = exact_full_body(bytes, &AFBR, FULLBOX_EXTRA_SIZE + U8_PAIR_LEN, "afbr")?;
        Ok(Self {
            flash_exposure_numerator: body[FULLBOX_EXTRA_SIZE] as i8,
            flash_exposure_denominator: body[FULLBOX_EXTRA_SIZE + U8_LEN] as i8,
        })
    }
}

impl Serialize for FlashExposureProperty {
    type Error = Error;

    fn serialized_len(&self) -> usize {
        box_len(BOX_HEADER_MIN_SIZE + FULLBOX_EXTRA_SIZE + U8_PAIR_LEN)
    }

    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        let mut cursor = begin_fullbox(buf, &AFBR, 0, 0, U8_PAIR_LEN)?;
        buf[cursor] = self.flash_exposure_numerator as u8;
        buf[cursor + U8_LEN] = self.flash_exposure_denominator as u8;
        cursor += U8_PAIR_LEN;
        Ok(cursor)
    }
}

/// Depth of Field property (`dobr`) — ISO/IEC 23008-12:2025 6.5.26.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub struct DepthOfFieldProperty {
    f_stop_numerator: i8,
    f_stop_denominator: i8,
}

impl DepthOfFieldProperty {
    /// A depth-of-field variation of `numerator`/`denominator` stops.
    #[must_use]
    pub const fn new(f_stop_numerator: i8, f_stop_denominator: i8) -> Self {
        Self {
            f_stop_numerator,
            f_stop_denominator,
        }
    }

    /// `f_stop_numerator`.
    #[must_use]
    pub const fn f_stop_numerator(&self) -> i8 {
        self.f_stop_numerator
    }

    /// `f_stop_denominator`.
    #[must_use]
    pub const fn f_stop_denominator(&self) -> i8 {
        self.f_stop_denominator
    }
}

impl<'a> Parse<'a> for DepthOfFieldProperty {
    type Error = Error;

    fn parse(bytes: &'a [u8]) -> Result<Self> {
        let body = exact_full_body(bytes, &DOBR, FULLBOX_EXTRA_SIZE + U8_PAIR_LEN, "dobr")?;
        Ok(Self {
            f_stop_numerator: body[FULLBOX_EXTRA_SIZE] as i8,
            f_stop_denominator: body[FULLBOX_EXTRA_SIZE + U8_LEN] as i8,
        })
    }
}

impl Serialize for DepthOfFieldProperty {
    type Error = Error;

    fn serialized_len(&self) -> usize {
        box_len(BOX_HEADER_MIN_SIZE + FULLBOX_EXTRA_SIZE + U8_PAIR_LEN)
    }

    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        let mut cursor = begin_fullbox(buf, &DOBR, 0, 0, U8_PAIR_LEN)?;
        buf[cursor] = self.f_stop_numerator as u8;
        buf[cursor + U8_LEN] = self.f_stop_denominator as u8;
        cursor += U8_PAIR_LEN;
        Ok(cursor)
    }
}

/// The scanning order of a panorama (6.5.27.3).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub enum PanoramaDirection {
    /// `0`: left-to-right horizontal.
    #[default]
    LeftToRight,
    /// `1`: right-to-left horizontal.
    RightToLeft,
    /// `2`: bottom-to-top vertical.
    BottomToTop,
    /// `3`: top-to-bottom vertical.
    TopToBottom,
    /// `4`: grid in raster-scan order.
    GridRaster,
    /// `5`: grid in continuous (boustrophedon) order.
    GridContinuous,
    /// A reserved value.
    Reserved(u8),
}

impl PanoramaDirection {
    /// The wire value.
    #[must_use]
    pub const fn value(self) -> u8 {
        match self {
            Self::LeftToRight => 0,
            Self::RightToLeft => 1,
            Self::BottomToTop => 2,
            Self::TopToBottom => 3,
            Self::GridRaster => PANO_DIRECTION_GRID,
            Self::GridContinuous => PANO_DIRECTION_GRID_CONTINUOUS,
            Self::Reserved(value) => value,
        }
    }

    /// The direction a wire value names.
    #[must_use]
    pub const fn from_value(value: u8) -> Self {
        match value {
            0 => Self::LeftToRight,
            1 => Self::RightToLeft,
            2 => Self::BottomToTop,
            3 => Self::TopToBottom,
            PANO_DIRECTION_GRID => Self::GridRaster,
            PANO_DIRECTION_GRID_CONTINUOUS => Self::GridContinuous,
            other => Self::Reserved(other),
        }
    }

    /// Whether the direction carries the optional grid fields (6.5.27.2).
    #[must_use]
    pub const fn is_grid(self) -> bool {
        matches!(self, Self::GridRaster | Self::GridContinuous)
    }

    /// The spec token for the direction.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::LeftToRight => "left-to-right",
            Self::RightToLeft => "right-to-left",
            Self::BottomToTop => "bottom-to-top",
            Self::TopToBottom => "top-to-bottom",
            Self::GridRaster => "grid-raster",
            Self::GridContinuous => "grid-continuous",
            Self::Reserved(_) => "reserved",
        }
    }
}

broadcast_core::impl_spec_display!(PanoramaDirection, Reserved);

/// Panorama property (`pano`) — ISO/IEC 23008-12:2025 6.5.27.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub struct PanoramaProperty {
    direction: PanoramaDirection,
    grid: Option<(u8, u8)>,
}

impl PanoramaProperty {
    /// A property for `direction`.
    #[must_use]
    pub const fn new(direction: PanoramaDirection) -> Self {
        Self {
            direction,
            grid: None,
        }
    }

    /// Add the grid dimensions a grid direction requires (6.5.27.2).
    #[must_use]
    pub const fn with_grid(mut self, rows_minus_one: u8, columns_minus_one: u8) -> Self {
        self.grid = Some((rows_minus_one, columns_minus_one));
        self
    }

    /// `panorama_direction`.
    #[must_use]
    pub const fn direction(&self) -> PanoramaDirection {
        self.direction
    }

    /// `rows_minus_one`, when the direction is a grid.
    #[must_use]
    pub const fn rows_minus_one(&self) -> Option<u8> {
        match self.grid {
            Some((rows, _)) => Some(rows),
            None => None,
        }
    }

    /// `columns_minus_one`, when the direction is a grid.
    #[must_use]
    pub const fn columns_minus_one(&self) -> Option<u8> {
        match self.grid {
            Some((_, columns)) => Some(columns),
            None => None,
        }
    }
}

impl<'a> Parse<'a> for PanoramaProperty {
    type Error = Error;

    fn parse(bytes: &'a [u8]) -> Result<Self> {
        let body = zero_full_body(bytes, &PANO, FULLBOX_EXTRA_SIZE + PANO_BASE_LEN, "pano")?;
        let direction = PanoramaDirection::from_value(body[FULLBOX_EXTRA_SIZE]);
        let body_len = FULLBOX_EXTRA_SIZE
            + if direction.is_grid() {
                PANO_GRID_LEN
            } else {
                PANO_BASE_LEN
            };
        if body.len() != body_len {
            return Err(Error::InvalidInput("pano trailing bytes"));
        }
        let grid = if direction.is_grid() {
            let at = FULLBOX_EXTRA_SIZE + PANO_BASE_LEN;
            need_in(body, at + U8_PAIR_LEN, "pano grid")?;
            Some((body[at], body[at + U8_LEN]))
        } else {
            None
        };
        Ok(Self { direction, grid })
    }
}

impl Serialize for PanoramaProperty {
    type Error = Error;

    fn serialized_len(&self) -> usize {
        let body = if self.grid.is_some() {
            PANO_GRID_LEN
        } else {
            PANO_BASE_LEN
        };
        box_len(BOX_HEADER_MIN_SIZE + FULLBOX_EXTRA_SIZE + body)
    }

    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        let body = if self.grid.is_some() {
            PANO_GRID_LEN
        } else {
            PANO_BASE_LEN
        };
        let mut cursor = begin_fullbox(buf, &PANO, 0, 0, body)?;
        buf[cursor] = self.direction.value();
        cursor += U8_LEN;
        if let Some((rows, columns)) = self.grid {
            buf[cursor] = rows;
            buf[cursor + U8_LEN] = columns;
            cursor += U8_PAIR_LEN;
        }
        Ok(cursor)
    }
}

/// Target Output Layer Set property (`tols`) — ISO/IEC 23008-12:2025 6.5.29.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub struct TargetOutputLayerSetProperty {
    target_ols_idx: u16,
}

impl TargetOutputLayerSetProperty {
    /// A property naming output layer set `target_ols_idx`.
    #[must_use]
    pub const fn new(target_ols_idx: u16) -> Self {
        Self { target_ols_idx }
    }

    /// `target_ols_idx`.
    #[must_use]
    pub const fn target_ols_idx(&self) -> u16 {
        self.target_ols_idx
    }
}

impl<'a> Parse<'a> for TargetOutputLayerSetProperty {
    type Error = Error;

    fn parse(bytes: &'a [u8]) -> Result<Self> {
        let body = exact_full_body(bytes, &TOLS, FULLBOX_EXTRA_SIZE + U16_LEN, "tols")?;
        Ok(Self {
            target_ols_idx: be_u16(body, FULLBOX_EXTRA_SIZE, "tols target_ols_idx")?,
        })
    }
}

impl Serialize for TargetOutputLayerSetProperty {
    type Error = Error;

    fn serialized_len(&self) -> usize {
        box_len(BOX_HEADER_MIN_SIZE + FULLBOX_EXTRA_SIZE + U16_LEN)
    }

    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        let mut cursor = begin_fullbox(buf, &TOLS, 0, 0, U16_LEN)?;
        buf[cursor..cursor + U16_LEN].copy_from_slice(&self.target_ols_idx.to_be_bytes());
        cursor += U16_LEN;
        Ok(cursor)
    }
}

/// Four-CC of the Wipe Transition Effect property (6.5.30).
pub const WIPE: [u8; BOX_TYPE_SIZE] = *b"wipe";
/// Four-CC of the Zoom Transition Effect property (6.5.31).
pub const ZOOM: [u8; BOX_TYPE_SIZE] = *b"zoom";
/// Four-CC of the Fade Transition Effect property (6.5.32).
pub const FADE: [u8; BOX_TYPE_SIZE] = *b"fade";
/// Four-CC of the Split Transition Effect property (6.5.33).
pub const SPLT: [u8; BOX_TYPE_SIZE] = *b"splt";
/// Four-CC of the Suggested Transition Period property (6.5.34).
pub const STPE: [u8; BOX_TYPE_SIZE] = *b"stpe";
/// Four-CC of the Suggested Time Display Duration property (6.5.35).
pub const SSLD: [u8; BOX_TYPE_SIZE] = *b"ssld";

/// The wipe transition direction (6.5.30.3).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub enum WipeDirection {
    /// `0`: from the left.
    #[default]
    FromLeft,
    /// `1`: from the right.
    FromRight,
    /// `2`: from the top.
    FromTop,
    /// `3`: from the bottom.
    FromBottom,
    /// `4`: from the left top.
    FromLeftTop,
    /// `5`: from the right top.
    FromRightTop,
    /// `6`: from the left bottom.
    FromLeftBottom,
    /// `7`: from the right bottom.
    FromRightBottom,
    /// A reserved value.
    Reserved(u8),
}

impl WipeDirection {
    /// The wire value.
    #[must_use]
    pub const fn value(self) -> u8 {
        match self {
            Self::FromLeft => 0,
            Self::FromRight => 1,
            Self::FromTop => 2,
            Self::FromBottom => 3,
            Self::FromLeftTop => 4,
            Self::FromRightTop => 5,
            Self::FromLeftBottom => 6,
            Self::FromRightBottom => 7,
            Self::Reserved(value) => value,
        }
    }

    /// The direction a wire value names.
    #[must_use]
    pub const fn from_value(value: u8) -> Self {
        match value {
            0 => Self::FromLeft,
            1 => Self::FromRight,
            2 => Self::FromTop,
            3 => Self::FromBottom,
            4 => Self::FromLeftTop,
            5 => Self::FromRightTop,
            6 => Self::FromLeftBottom,
            7 => Self::FromRightBottom,
            other => Self::Reserved(other),
        }
    }

    /// The spec token.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::FromLeft => "from-left",
            Self::FromRight => "from-right",
            Self::FromTop => "from-top",
            Self::FromBottom => "from-bottom",
            Self::FromLeftTop => "from-left-top",
            Self::FromRightTop => "from-right-top",
            Self::FromLeftBottom => "from-left-bottom",
            Self::FromRightBottom => "from-right-bottom",
            Self::Reserved(_) => "reserved",
        }
    }
}

broadcast_core::impl_spec_display!(WipeDirection, Reserved);

/// Wipe Transition Effect property (`wipe`) — ISO/IEC 23008-12:2025 6.5.30.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub struct WipeTransitionEffectProperty {
    direction: WipeDirection,
}

impl WipeTransitionEffectProperty {
    /// A wipe in `direction`.
    #[must_use]
    pub const fn new(direction: WipeDirection) -> Self {
        Self { direction }
    }

    /// `transition_direction`.
    #[must_use]
    pub const fn direction(&self) -> WipeDirection {
        self.direction
    }
}

impl<'a> Parse<'a> for WipeTransitionEffectProperty {
    type Error = Error;

    fn parse(bytes: &'a [u8]) -> Result<Self> {
        let body = exact_full_body(bytes, &WIPE, FULLBOX_EXTRA_SIZE + U8_LEN, "wipe")?;
        Ok(Self {
            direction: WipeDirection::from_value(body[FULLBOX_EXTRA_SIZE]),
        })
    }
}

impl Serialize for WipeTransitionEffectProperty {
    type Error = Error;

    fn serialized_len(&self) -> usize {
        box_len(BOX_HEADER_MIN_SIZE + FULLBOX_EXTRA_SIZE + U8_LEN)
    }

    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        let mut cursor = begin_fullbox(buf, &WIPE, 0, 0, U8_LEN)?;
        buf[cursor] = self.direction.value();
        cursor += U8_LEN;
        Ok(cursor)
    }
}

/// The zoom transition direction (6.5.31.3).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub enum ZoomDirection {
    /// `0`: zoom in.
    #[default]
    In,
    /// `1`: zoom out.
    Out,
}

impl ZoomDirection {
    /// The wire value.
    #[must_use]
    pub const fn value(self) -> u8 {
        match self {
            Self::In => 0,
            Self::Out => 1,
        }
    }

    /// The direction a wire bit names.
    #[must_use]
    pub const fn from_value(value: u8) -> Self {
        if value == 0 { Self::In } else { Self::Out }
    }

    /// The spec token.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::In => "in",
            Self::Out => "out",
        }
    }
}

broadcast_core::impl_spec_display!(ZoomDirection);

/// The zoom transition shape (6.5.31.3).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub enum ZoomShape {
    /// `0`: rectangular.
    #[default]
    Rectangular,
    /// `1`: circle.
    Circle,
    /// `2`: star.
    Star,
    /// A reserved value.
    Reserved(u8),
}

impl ZoomShape {
    /// The wire value.
    #[must_use]
    pub const fn value(self) -> u8 {
        match self {
            Self::Rectangular => 0,
            Self::Circle => 1,
            Self::Star => 2,
            Self::Reserved(value) => value,
        }
    }

    /// The shape a wire value names.
    #[must_use]
    pub const fn from_value(value: u8) -> Self {
        match value {
            0 => Self::Rectangular,
            1 => Self::Circle,
            2 => Self::Star,
            other => Self::Reserved(other),
        }
    }

    /// The spec token.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::Rectangular => "rectangular",
            Self::Circle => "circle",
            Self::Star => "star",
            Self::Reserved(_) => "reserved",
        }
    }
}

broadcast_core::impl_spec_display!(ZoomShape, Reserved);

/// Zoom Transition Effect property (`zoom`) — ISO/IEC 23008-12:2025 6.5.31.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub struct ZoomTransitionEffectProperty {
    direction: ZoomDirection,
    shape: ZoomShape,
}

impl ZoomTransitionEffectProperty {
    /// A zoom in `direction` using `shape`.
    #[must_use]
    pub const fn new(direction: ZoomDirection, shape: ZoomShape) -> Self {
        Self { direction, shape }
    }

    /// `transition_direction`.
    #[must_use]
    pub const fn direction(&self) -> ZoomDirection {
        self.direction
    }

    /// `transition_shape`.
    #[must_use]
    pub const fn shape(&self) -> ZoomShape {
        self.shape
    }
}

impl<'a> Parse<'a> for ZoomTransitionEffectProperty {
    type Error = Error;

    fn parse(bytes: &'a [u8]) -> Result<Self> {
        let body = exact_full_body(bytes, &ZOOM, FULLBOX_EXTRA_SIZE + U8_LEN, "zoom")?;
        let packed = body[FULLBOX_EXTRA_SIZE];
        Ok(Self {
            direction: ZoomDirection::from_value(packed & ZOOM_DIRECTION_MASK),
            shape: ZoomShape::from_value(packed & ZOOM_SHAPE_MASK),
        })
    }
}

impl Serialize for ZoomTransitionEffectProperty {
    type Error = Error;

    fn serialized_len(&self) -> usize {
        box_len(BOX_HEADER_MIN_SIZE + FULLBOX_EXTRA_SIZE + U8_LEN)
    }

    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        let mut cursor = begin_fullbox(buf, &ZOOM, 0, 0, U8_LEN)?;
        let direction = fit_mask(u32::from(self.direction.value()), 1, "zoom direction")?;
        let shape = fit_mask(
            u32::from(self.shape.value()),
            u32::from(ZOOM_SHAPE_MASK),
            "zoom shape",
        )?;
        let packed = fit_mask(
            (direction * u32::from(ZOOM_DIRECTION_MASK)) | shape,
            u32::from(u8::MAX),
            "zoom",
        )?;
        buf[cursor] = packed as u8;
        cursor += U8_LEN;
        Ok(cursor)
    }
}

/// The fade transition direction (6.5.32.3).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub enum FadeDirection {
    /// `0`: through white.
    #[default]
    ThroughWhite,
    /// `1`: through black.
    ThroughBlack,
    /// `2`: dissolve.
    Dissolve,
    /// A reserved value.
    Reserved(u8),
}

impl FadeDirection {
    /// The wire value.
    #[must_use]
    pub const fn value(self) -> u8 {
        match self {
            Self::ThroughWhite => 0,
            Self::ThroughBlack => 1,
            Self::Dissolve => 2,
            Self::Reserved(value) => value,
        }
    }

    /// The direction a wire value names.
    #[must_use]
    pub const fn from_value(value: u8) -> Self {
        match value {
            0 => Self::ThroughWhite,
            1 => Self::ThroughBlack,
            2 => Self::Dissolve,
            other => Self::Reserved(other),
        }
    }

    /// The spec token.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::ThroughWhite => "through-white",
            Self::ThroughBlack => "through-black",
            Self::Dissolve => "dissolve",
            Self::Reserved(_) => "reserved",
        }
    }
}

broadcast_core::impl_spec_display!(FadeDirection, Reserved);

/// Fade Transition Effect property (`fade`) — ISO/IEC 23008-12:2025 6.5.32.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub struct FadeTransitionEffectProperty {
    direction: FadeDirection,
}

impl FadeTransitionEffectProperty {
    /// A fade in `direction`.
    #[must_use]
    pub const fn new(direction: FadeDirection) -> Self {
        Self { direction }
    }

    /// `transition_direction`.
    #[must_use]
    pub const fn direction(&self) -> FadeDirection {
        self.direction
    }
}

impl<'a> Parse<'a> for FadeTransitionEffectProperty {
    type Error = Error;

    fn parse(bytes: &'a [u8]) -> Result<Self> {
        let body = exact_full_body(bytes, &FADE, FULLBOX_EXTRA_SIZE + U8_LEN, "fade")?;
        Ok(Self {
            direction: FadeDirection::from_value(body[FULLBOX_EXTRA_SIZE]),
        })
    }
}

impl Serialize for FadeTransitionEffectProperty {
    type Error = Error;

    fn serialized_len(&self) -> usize {
        box_len(BOX_HEADER_MIN_SIZE + FULLBOX_EXTRA_SIZE + U8_LEN)
    }

    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        let mut cursor = begin_fullbox(buf, &FADE, 0, 0, U8_LEN)?;
        buf[cursor] = self.direction.value();
        cursor += U8_LEN;
        Ok(cursor)
    }
}

/// The split transition direction (6.5.33.3).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub enum SplitDirection {
    /// `0`: vertical in.
    #[default]
    VerticalIn,
    /// `1`: vertical out.
    VerticalOut,
    /// `2`: horizontal in.
    HorizontalIn,
    /// `3`: horizontal out.
    HorizontalOut,
    /// A reserved value.
    Reserved(u8),
}

impl SplitDirection {
    /// The wire value.
    #[must_use]
    pub const fn value(self) -> u8 {
        match self {
            Self::VerticalIn => 0,
            Self::VerticalOut => 1,
            Self::HorizontalIn => 2,
            Self::HorizontalOut => 3,
            Self::Reserved(value) => value,
        }
    }

    /// The direction a wire value names.
    #[must_use]
    pub const fn from_value(value: u8) -> Self {
        match value {
            0 => Self::VerticalIn,
            1 => Self::VerticalOut,
            2 => Self::HorizontalIn,
            3 => Self::HorizontalOut,
            other => Self::Reserved(other),
        }
    }

    /// The spec token.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::VerticalIn => "vertical-in",
            Self::VerticalOut => "vertical-out",
            Self::HorizontalIn => "horizontal-in",
            Self::HorizontalOut => "horizontal-out",
            Self::Reserved(_) => "reserved",
        }
    }
}

broadcast_core::impl_spec_display!(SplitDirection, Reserved);

/// Split Transition Effect property (`splt`) — ISO/IEC 23008-12:2025 6.5.33.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub struct SplitTransitionEffectProperty {
    direction: SplitDirection,
}

impl SplitTransitionEffectProperty {
    /// A split transition in `direction`.
    #[must_use]
    pub const fn new(direction: SplitDirection) -> Self {
        Self { direction }
    }

    /// `transition_direction`.
    #[must_use]
    pub const fn direction(&self) -> SplitDirection {
        self.direction
    }
}

impl<'a> Parse<'a> for SplitTransitionEffectProperty {
    type Error = Error;

    fn parse(bytes: &'a [u8]) -> Result<Self> {
        let body = exact_full_body(bytes, &SPLT, FULLBOX_EXTRA_SIZE + U8_LEN, "splt")?;
        Ok(Self {
            direction: SplitDirection::from_value(body[FULLBOX_EXTRA_SIZE]),
        })
    }
}

impl Serialize for SplitTransitionEffectProperty {
    type Error = Error;

    fn serialized_len(&self) -> usize {
        box_len(BOX_HEADER_MIN_SIZE + FULLBOX_EXTRA_SIZE + U8_LEN)
    }

    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        let mut cursor = begin_fullbox(buf, &SPLT, 0, 0, U8_LEN)?;
        buf[cursor] = self.direction.value();
        cursor += U8_LEN;
        Ok(cursor)
    }
}

/// Suggested Transition Period property (`stpe`) — ISO/IEC 23008-12:2025
/// 6.5.34.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub struct SuggestedTransitionPeriodProperty {
    transition_period: u8,
}

impl SuggestedTransitionPeriodProperty {
    /// A suggested period of `transition_period` 1/16 seconds.
    #[must_use]
    pub const fn new(transition_period: u8) -> Self {
        Self { transition_period }
    }

    /// `transition_period`.
    #[must_use]
    pub const fn transition_period(&self) -> u8 {
        self.transition_period
    }
}

impl<'a> Parse<'a> for SuggestedTransitionPeriodProperty {
    type Error = Error;

    fn parse(bytes: &'a [u8]) -> Result<Self> {
        let body = exact_full_body(bytes, &STPE, FULLBOX_EXTRA_SIZE + U8_LEN, "stpe")?;
        Ok(Self {
            transition_period: body[FULLBOX_EXTRA_SIZE],
        })
    }
}

impl Serialize for SuggestedTransitionPeriodProperty {
    type Error = Error;

    fn serialized_len(&self) -> usize {
        box_len(BOX_HEADER_MIN_SIZE + FULLBOX_EXTRA_SIZE + U8_LEN)
    }

    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        let mut cursor = begin_fullbox(buf, &STPE, 0, 0, U8_LEN)?;
        buf[cursor] = self.transition_period;
        cursor += U8_LEN;
        Ok(cursor)
    }
}

/// Suggested Time Display Duration property (`ssld`) — ISO/IEC 23008-12:2025
/// 6.5.35.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub struct SuggestedTimeDisplayDurationProperty {
    duration: u16,
}

impl SuggestedTimeDisplayDurationProperty {
    /// A suggested display duration of `duration` 1/16 seconds.
    #[must_use]
    pub const fn new(duration: u16) -> Self {
        Self { duration }
    }

    /// `duration`.
    #[must_use]
    pub const fn duration(&self) -> u16 {
        self.duration
    }
}

impl<'a> Parse<'a> for SuggestedTimeDisplayDurationProperty {
    type Error = Error;

    fn parse(bytes: &'a [u8]) -> Result<Self> {
        let body = exact_full_body(bytes, &SSLD, FULLBOX_EXTRA_SIZE + U16_LEN, "ssld")?;
        Ok(Self {
            duration: be_u16(body, FULLBOX_EXTRA_SIZE, "ssld duration")?,
        })
    }
}

impl Serialize for SuggestedTimeDisplayDurationProperty {
    type Error = Error;

    fn serialized_len(&self) -> usize {
        box_len(BOX_HEADER_MIN_SIZE + FULLBOX_EXTRA_SIZE + U16_LEN)
    }

    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        let mut cursor = begin_fullbox(buf, &SSLD, 0, 0, U16_LEN)?;
        buf[cursor..cursor + U16_LEN].copy_from_slice(&self.duration.to_be_bytes());
        cursor += U16_LEN;
        Ok(cursor)
    }
}

/// Four-CC of the Progressive Derived Image Item Information property
/// (6.5.37).
pub const PRDI: [u8; BOX_TYPE_SIZE] = *b"prdi";
/// Four-CC of the Single Stream property (6.5.38).
pub const SSTR: [u8; BOX_TYPE_SIZE] = *b"sstr";
/// Four-CC of the Camera Extrinsic Matrix property (6.5.39).
pub const CMEX: [u8; BOX_TYPE_SIZE] = *b"cmex";
/// Four-CC of the Camera Intrinsic Matrix property (6.5.40).
pub const CMIN: [u8; BOX_TYPE_SIZE] = *b"cmin";

/// Progressive Derived Image Item Information property (`prdi`) —
/// ISO/IEC 23008-12:2025 6.5.37.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub struct ProgressiveDerivedImageItemInformationProperty {
    flags: u32,
    step_count: Option<u16>,
    item_counts: Vec<u16>,
}

impl ProgressiveDerivedImageItemInformationProperty {
    /// A property with `step_count` (inferred from `item_counts` when `None`)
    /// and one item count per step.
    #[must_use]
    pub fn new(step_count: Option<u16>, item_counts: Vec<u16>) -> Self {
        Self {
            flags: 0,
            step_count,
            item_counts,
        }
    }

    /// Set the `flags` word (6.5.37.1).
    #[must_use]
    pub const fn with_flags(mut self, flags: u32) -> Self {
        self.flags = flags;
        self
    }

    /// The FullBox flags.
    #[must_use]
    pub const fn flags(&self) -> u32 {
        self.flags
    }

    /// `step_count`, when signalled.
    #[must_use]
    pub const fn step_count(&self) -> Option<u16> {
        self.step_count
    }

    /// `item_count[i]`.
    #[must_use]
    pub fn item_counts(&self) -> &[u16] {
        &self.item_counts
    }

    /// Whether candidates follow the `dimg` declaration order.
    #[must_use]
    pub const fn item_reference_order(&self) -> bool {
        self.flags & PRDI_ITEM_REFERENCE_ORDER != 0
    }

    /// Whether each step consumes exactly one candidate.
    #[must_use]
    pub const fn one_item_per_step(&self) -> bool {
        self.flags & PRDI_SINGLE_CANDIDATE_PER_STEP != 0
    }

    /// Whether an alternative to an input is a candidate.
    #[must_use]
    pub const fn alternative_is_candidate(&self) -> bool {
        self.flags & PRDI_ALTERNATIVE_IS_CANDIDATE != 0
    }
}

impl<'a> Parse<'a> for ProgressiveDerivedImageItemInformationProperty {
    type Error = Error;

    fn parse(bytes: &'a [u8]) -> Result<Self> {
        let body = full_body(bytes, &PRDI, FULLBOX_EXTRA_SIZE, "prdi")?;
        expect_version_zero(body, "prdi version")?;
        let flags = full_flags(body);
        let one_item = flags & PRDI_SINGLE_CANDIDATE_PER_STEP != 0;
        let alternatives = flags & PRDI_ALTERNATIVE_IS_CANDIDATE != 0;
        let mut cursor = FULLBOX_EXTRA_SIZE;
        let step_count = if !one_item || alternatives {
            let value = be_u16(body, cursor, "prdi step_count")?;
            cursor += U16_LEN;
            Some(value)
        } else {
            None
        };
        let mut item_counts = Vec::new();
        if !one_item {
            let count = usize::from(step_count.unwrap_or(0));
            crate::box_support::check_entry_count(body.len() - cursor, U16_LEN, count, "prdi")?;
            item_counts.reserve(count);
            for _ in 0..count {
                item_counts.push(be_u16(body, cursor, "prdi item_count")?);
                cursor += U16_LEN;
            }
        }
        if cursor != body.len() {
            return Err(Error::InvalidInput("prdi trailing bytes"));
        }
        Ok(Self {
            flags,
            step_count,
            item_counts,
        })
    }
}

impl Serialize for ProgressiveDerivedImageItemInformationProperty {
    type Error = Error;

    fn serialized_len(&self) -> usize {
        let mut body = 0usize;
        if self.step_count.is_some() {
            body += U16_LEN;
        }
        body += self.item_counts.len() * U16_LEN;
        box_len(BOX_HEADER_MIN_SIZE + FULLBOX_EXTRA_SIZE + body)
    }

    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        let mut body = 0usize;
        if self.step_count.is_some() {
            body += U16_LEN;
        }
        body += self.item_counts.len() * U16_LEN;
        let mut cursor = begin_fullbox(buf, &PRDI, 0, self.flags, body)?;
        if let Some(step_count) = self.step_count {
            buf[cursor..cursor + U16_LEN].copy_from_slice(&step_count.to_be_bytes());
            cursor += U16_LEN;
        }
        for item_count in &self.item_counts {
            buf[cursor..cursor + U16_LEN].copy_from_slice(&item_count.to_be_bytes());
            cursor += U16_LEN;
        }
        Ok(cursor)
    }
}

/// Single Stream property (`sstr`) — ISO/IEC 23008-12:2025 6.5.38.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub struct SingleStreamProperty;

impl SingleStreamProperty {
    /// The property (its body is empty).
    #[must_use]
    pub const fn new() -> Self {
        Self
    }
}

impl<'a> Parse<'a> for SingleStreamProperty {
    type Error = Error;

    fn parse(bytes: &'a [u8]) -> Result<Self> {
        let body = full_body(bytes, &SSTR, 0, "sstr")?;
        if !body.is_empty() {
            return Err(Error::InvalidInput("sstr body"));
        }
        Ok(Self)
    }
}

impl Serialize for SingleStreamProperty {
    type Error = Error;

    fn serialized_len(&self) -> usize {
        box_len(BOX_HEADER_MIN_SIZE)
    }

    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        let need = self.serialized_len();
        need_out(buf, need)?;
        write_box_header(buf, &SSTR, need)
    }
}

/// The camera orientation of a [`CameraExtrinsicMatrixProperty`] (6.5.39).
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub enum CameraOrientation {
    /// A version-0 quaternion, one signed fixed-point component per axis.
    Quaternion {
        /// `quat_x`.
        x: i32,
        /// `quat_y`.
        y: i32,
        /// `quat_z`.
        z: i32,
    },
    /// A version-1 `ViewpointGlobalCoordinateSysRotationStruct` (ISO/IEC
    /// 23090-7, not held) and everything after it, kept opaque.
    Opaque(Vec<u8>),
}

/// Camera Extrinsic Matrix property (`cmex`) — ISO/IEC 23008-12:2025 6.5.39.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub struct CameraExtrinsicMatrixProperty {
    version: u8,
    flags: u32,
    pos_x: Option<i32>,
    pos_y: Option<i32>,
    pos_z: Option<i32>,
    orientation: Option<CameraOrientation>,
    id: Option<u32>,
    opaque_tail: Vec<u8>,
}

impl CameraExtrinsicMatrixProperty {
    /// A version-0 property with the given position and quaternion.
    #[must_use]
    pub const fn new(
        pos_x: Option<i32>,
        pos_y: Option<i32>,
        pos_z: Option<i32>,
        orientation: Option<CameraOrientation>,
        id: Option<u32>,
    ) -> Self {
        let mut flags = 0u32;
        if pos_x.is_some() {
            flags |= CMEX_POS_X;
        }
        if pos_y.is_some() {
            flags |= CMEX_POS_Y;
        }
        if pos_z.is_some() {
            flags |= CMEX_POS_Z;
        }
        if orientation.is_some() {
            flags |= CMEX_ORIENTATION;
        }
        if id.is_some() {
            flags |= CMEX_ID;
        }
        Self {
            version: 0,
            flags,
            pos_x,
            pos_y,
            pos_z,
            orientation,
            id,
            opaque_tail: Vec::new(),
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

    /// `pos_x` in micrometres, when present.
    #[must_use]
    pub const fn pos_x(&self) -> Option<i32> {
        self.pos_x
    }

    /// `pos_y` in micrometres, when present.
    #[must_use]
    pub const fn pos_y(&self) -> Option<i32> {
        self.pos_y
    }

    /// `pos_z` in micrometres, when present.
    #[must_use]
    pub const fn pos_z(&self) -> Option<i32> {
        self.pos_z
    }

    /// The orientation, when present.
    #[must_use]
    pub const fn orientation(&self) -> Option<&CameraOrientation> {
        self.orientation.as_ref()
    }

    /// The world coordinate system `id`, when present.
    #[must_use]
    pub const fn id(&self) -> Option<u32> {
        self.id
    }

    /// The bytes of a version this crate does not interpret (the version-1
    /// rotation struct and the fields after it).
    #[must_use]
    pub fn opaque_tail(&self) -> &[u8] {
        &self.opaque_tail
    }
}

impl<'a> Parse<'a> for CameraExtrinsicMatrixProperty {
    type Error = Error;

    fn parse(bytes: &'a [u8]) -> Result<Self> {
        let body = full_body(bytes, &CMEX, FULLBOX_EXTRA_SIZE, "cmex")?;
        let version = body[0];
        let flags = full_flags(body);
        let mut cursor = FULLBOX_EXTRA_SIZE;
        let mut read_pos = |flag: u32, what: &'static str| -> Result<Option<i32>> {
            if flags & flag == 0 {
                return Ok(None);
            }
            need_in(body, cursor + U32_LEN, what)?;
            let value = i32_at(body, cursor);
            cursor += U32_LEN;
            Ok(Some(value))
        };
        let pos_x = read_pos(CMEX_POS_X, "cmex pos_x")?;
        let pos_y = read_pos(CMEX_POS_Y, "cmex pos_y")?;
        let pos_z = read_pos(CMEX_POS_Z, "cmex pos_z")?;
        if version != 0 {
            let tail = body
                .get(cursor..)
                .ok_or(Error::InvalidInput("cmex opaque tail"))?
                .to_vec();
            return Ok(Self {
                version,
                flags,
                pos_x,
                pos_y,
                pos_z,
                orientation: None,
                id: None,
                opaque_tail: tail,
            });
        }
        let large = flags & CMEX_ROT_LARGE_FIELD != 0;
        let orientation = if flags & CMEX_ORIENTATION != 0 {
            let width = if large { U32_LEN } else { U16_LEN };
            let x = read_signed(body, cursor, width, "cmex quat_x")?;
            cursor += width;
            let y = read_signed(body, cursor, width, "cmex quat_y")?;
            cursor += width;
            let z = read_signed(body, cursor, width, "cmex quat_z")?;
            cursor += width;
            Some(CameraOrientation::Quaternion { x, y, z })
        } else {
            None
        };
        let id = if flags & CMEX_ID != 0 {
            let value = be_u32(body, cursor, "cmex id")?;
            cursor += U32_LEN;
            Some(value)
        } else {
            None
        };
        if cursor != body.len() {
            return Err(Error::InvalidInput("cmex trailing bytes"));
        }
        Ok(Self {
            version,
            flags,
            pos_x,
            pos_y,
            pos_z,
            orientation,
            id,
            opaque_tail: Vec::new(),
        })
    }
}

impl Serialize for CameraExtrinsicMatrixProperty {
    type Error = Error;

    fn serialized_len(&self) -> usize {
        let mut body = 0usize;
        for present in [self.pos_x, self.pos_y, self.pos_z] {
            if present.is_some() {
                body += U32_LEN;
            }
        }
        if self.version == 0 {
            if let Some(CameraOrientation::Quaternion { .. }) = self.orientation {
                let width = if self.flags & CMEX_ROT_LARGE_FIELD != 0 {
                    U32_LEN
                } else {
                    U16_LEN
                };
                body += QUATERNION_COMPONENTS * width;
            }
            if self.id.is_some() {
                body += U32_LEN;
            }
        } else {
            body += self.opaque_tail.len();
        }
        box_len(BOX_HEADER_MIN_SIZE + FULLBOX_EXTRA_SIZE + body)
    }

    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        let mut body = 0usize;
        for present in [self.pos_x, self.pos_y, self.pos_z] {
            if present.is_some() {
                body += U32_LEN;
            }
        }
        if self.version == 0 {
            if let Some(CameraOrientation::Quaternion { .. }) = self.orientation {
                let width = if self.flags & CMEX_ROT_LARGE_FIELD != 0 {
                    U32_LEN
                } else {
                    U16_LEN
                };
                body += QUATERNION_COMPONENTS * width;
            }
            if self.id.is_some() {
                body += U32_LEN;
            }
        } else {
            body += self.opaque_tail.len();
        }
        let mut cursor = begin_fullbox(buf, &CMEX, self.version, self.flags, body)?;
        for value in [self.pos_x, self.pos_y, self.pos_z].into_iter().flatten() {
            buf[cursor..cursor + U32_LEN].copy_from_slice(&value.to_be_bytes());
            cursor += U32_LEN;
        }
        if self.version == 0 {
            if let Some(CameraOrientation::Quaternion { x, y, z }) = self.orientation {
                let width = if self.flags & CMEX_ROT_LARGE_FIELD != 0 {
                    U32_LEN
                } else {
                    U16_LEN
                };
                for value in [x, y, z] {
                    write_signed(buf, cursor, value, width)?;
                    cursor += width;
                }
            }
            if let Some(id) = self.id {
                buf[cursor..cursor + U32_LEN].copy_from_slice(&id.to_be_bytes());
                cursor += U32_LEN;
            }
        } else {
            need_out(buf, cursor + self.opaque_tail.len())?;
            let end = cursor + self.opaque_tail.len();
            buf[cursor..end].copy_from_slice(&self.opaque_tail);
            cursor = end;
        }
        Ok(cursor)
    }
}

/// Read a 16- or 32-bit signed field.
fn read_signed(body: &[u8], offset: usize, width: usize, what: &'static str) -> Result<i32> {
    need_in(body, offset + width, what)?;
    if width == U32_LEN {
        Ok(i32_at(body, offset))
    } else {
        Ok(i32::from(i16_at(body, offset)))
    }
}

/// Write a 16- or 32-bit signed field.
fn write_signed(buf: &mut [u8], offset: usize, value: i32, width: usize) -> Result<()> {
    need_out(buf, offset + width)?;
    if width == U32_LEN {
        buf[offset..offset + U32_LEN].copy_from_slice(&value.to_be_bytes());
    } else {
        let narrowed = i16::try_from(value).map_err(|_| Error::InvalidInput("cmex quaternion"))?;
        buf[offset..offset + U16_LEN].copy_from_slice(&narrowed.to_be_bytes());
    }
    Ok(())
}

/// Camera Intrinsic Matrix property (`cmin`) — ISO/IEC 23008-12:2025 6.5.40.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub struct CameraIntrinsicMatrixProperty {
    flags: u32,
    focal_length_x: i32,
    principal_point_x: i32,
    principal_point_y: i32,
    focal_length_y: Option<i32>,
    skew_factor: Option<i32>,
}

impl CameraIntrinsicMatrixProperty {
    /// Simplified intrinsics (square pixels, no skew).
    #[must_use]
    pub const fn simplified(
        focal_length_x: i32,
        principal_point_x: i32,
        principal_point_y: i32,
    ) -> Self {
        Self {
            flags: 0,
            focal_length_x,
            principal_point_x,
            principal_point_y,
            focal_length_y: None,
            skew_factor: None,
        }
    }

    /// Full intrinsics with an explicit vertical focal length and skew.
    #[must_use]
    pub const fn full(
        focal_length_x: i32,
        principal_point_x: i32,
        principal_point_y: i32,
        focal_length_y: i32,
        skew_factor: i32,
    ) -> Self {
        Self {
            flags: CMIN_FULL_INTRINSICS,
            focal_length_x,
            principal_point_x,
            principal_point_y,
            focal_length_y: Some(focal_length_y),
            skew_factor: Some(skew_factor),
        }
    }

    /// Set the denominator shift operand (6.5.40.1).
    #[must_use]
    pub const fn with_denominator_shift(mut self, shift: u32) -> Self {
        self.flags = (self.flags & !CMIN_DENOMINATOR_MASK) | (shift << CMIN_DENOMINATOR_SHIFT);
        self
    }

    /// Set the skew denominator shift operand (6.5.40.1).
    #[must_use]
    pub const fn with_skew_denominator_shift(mut self, shift: u32) -> Self {
        self.flags =
            (self.flags & !CMIN_SKEW_DENOMINATOR_MASK) | (shift << CMIN_SKEW_DENOMINATOR_SHIFT);
        self
    }

    /// The FullBox flags.
    #[must_use]
    pub const fn flags(&self) -> u32 {
        self.flags
    }

    /// `focal_length_x`.
    #[must_use]
    pub const fn focal_length_x(&self) -> i32 {
        self.focal_length_x
    }

    /// `principal_point_x`.
    #[must_use]
    pub const fn principal_point_x(&self) -> i32 {
        self.principal_point_x
    }

    /// `principal_point_y`.
    #[must_use]
    pub const fn principal_point_y(&self) -> i32 {
        self.principal_point_y
    }

    /// `focal_length_y`, when the full intrinsics are signalled.
    #[must_use]
    pub const fn focal_length_y(&self) -> Option<i32> {
        self.focal_length_y
    }

    /// `skew_factor`, when the full intrinsics are signalled.
    #[must_use]
    pub const fn skew_factor(&self) -> Option<i32> {
        self.skew_factor
    }

    /// The `denominator` shift operand.
    #[must_use]
    pub const fn denominator_shift(&self) -> u32 {
        (self.flags & CMIN_DENOMINATOR_MASK) >> CMIN_DENOMINATOR_SHIFT
    }

    /// The `skewDenominator` shift operand.
    #[must_use]
    pub const fn skew_denominator_shift(&self) -> u32 {
        (self.flags & CMIN_SKEW_DENOMINATOR_MASK) >> CMIN_SKEW_DENOMINATOR_SHIFT
    }
}

impl<'a> Parse<'a> for CameraIntrinsicMatrixProperty {
    type Error = Error;

    fn parse(bytes: &'a [u8]) -> Result<Self> {
        let body = full_body(
            bytes,
            &CMIN,
            FULLBOX_EXTRA_SIZE + CMIN_SIMPLIFIED_FIELDS * U32_LEN,
            "cmin",
        )?;
        expect_version_zero(body, "cmin version")?;
        let flags = full_flags(body);
        let mut cursor = FULLBOX_EXTRA_SIZE;
        let focal_length_x = read_signed(body, cursor, U32_LEN, "cmin focal_length_x")?;
        cursor += U32_LEN;
        let principal_point_x = read_signed(body, cursor, U32_LEN, "cmin principal_point_x")?;
        cursor += U32_LEN;
        let principal_point_y = read_signed(body, cursor, U32_LEN, "cmin principal_point_y")?;
        cursor += U32_LEN;
        let (focal_length_y, skew_factor) = if flags & CMIN_FULL_INTRINSICS != 0 {
            let fy = read_signed(body, cursor, U32_LEN, "cmin focal_length_y")?;
            cursor += U32_LEN;
            let skew = read_signed(body, cursor, U32_LEN, "cmin skew_factor")?;
            cursor += U32_LEN;
            (Some(fy), Some(skew))
        } else {
            (None, None)
        };
        if cursor != body.len() {
            return Err(Error::InvalidInput("cmin trailing bytes"));
        }
        Ok(Self {
            flags,
            focal_length_x,
            principal_point_x,
            principal_point_y,
            focal_length_y,
            skew_factor,
        })
    }
}

impl Serialize for CameraIntrinsicMatrixProperty {
    type Error = Error;

    fn serialized_len(&self) -> usize {
        let fields = if self.flags & CMIN_FULL_INTRINSICS != 0 {
            FULLBOX_EXTRA_SIZE + CMIN_FULL_FIELDS * U32_LEN
        } else {
            FULLBOX_EXTRA_SIZE + CMIN_SIMPLIFIED_FIELDS * U32_LEN
        };
        box_len(BOX_HEADER_MIN_SIZE + fields)
    }

    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        let fields = if self.flags & CMIN_FULL_INTRINSICS != 0 {
            CMIN_FULL_FIELDS * U32_LEN
        } else {
            CMIN_SIMPLIFIED_FIELDS * U32_LEN
        };
        let mut cursor = begin_fullbox(buf, &CMIN, 0, self.flags, fields)?;
        for value in [
            self.focal_length_x,
            self.principal_point_x,
            self.principal_point_y,
        ] {
            write_signed(buf, cursor, value, U32_LEN)?;
            cursor += U32_LEN;
        }
        if let (true, Some(fy), Some(skew)) = (
            self.flags & CMIN_FULL_INTRINSICS != 0,
            self.focal_length_y,
            self.skew_factor,
        ) {
            write_signed(buf, cursor, fy, U32_LEN)?;
            cursor += U32_LEN;
            write_signed(buf, cursor, skew, U32_LEN)?;
            cursor += U32_LEN;
        }
        Ok(cursor)
    }
}

/// Four-CC of the Constrained Extents Grid property (Amd 1 6.5.41).
pub const CEXG: [u8; BOX_TYPE_SIZE] = *b"cexg";
/// Four-CC of the Disparity Adjustment property (Amd 1 6.5.42).
pub const DADJ: [u8; BOX_TYPE_SIZE] = *b"dadj";
/// Four-CC of the Stereo Aggressors property (Amd 1 6.5.43).
pub const STAG: [u8; BOX_TYPE_SIZE] = *b"stag";
/// Four-CC of the Reference Viewing Environment property (Amd 1 6.5.44).
pub const REVE: [u8; BOX_TYPE_SIZE] = *b"reve";
/// Four-CC of the Nominal Diffuse White property (Amd 1 6.5.45).
pub const NDWT: [u8; BOX_TYPE_SIZE] = *b"ndwt";
/// Four-CC of the Mask Configuration property (11.2.2.2).
pub const MSKC: [u8; BOX_TYPE_SIZE] = *b"mskC";

/// Constrained Extents Grid property (`cexg`) — ISO/IEC 23008-12:2025
/// Amd 1 6.5.41.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub struct ConstrainedExtentsGridProperty {
    flags: u32,
    rows_minus_one: u16,
    columns_minus_one: u16,
    image_tile_width: u32,
    image_tile_height: u32,
    extent_config: Vec<u8>,
}

impl ConstrainedExtentsGridProperty {
    /// A grid of `rows` by `columns` tiles of `image_tile_width` x
    /// `image_tile_height` pixels.
    #[must_use]
    pub const fn new(
        rows_minus_one: u16,
        columns_minus_one: u16,
        image_tile_width: u32,
        image_tile_height: u32,
    ) -> Self {
        Self {
            flags: 0,
            rows_minus_one,
            columns_minus_one,
            image_tile_width,
            image_tile_height,
            extent_config: Vec::new(),
        }
    }

    /// Signal 32-bit tile dimensions (`field_length_flag`).
    #[must_use]
    pub const fn with_large_fields(mut self) -> Self {
        self.flags |= CEXG_FIELD_LENGTH;
        self
    }

    /// Carry the per-extent decoder configuration record verbatim.
    #[must_use]
    pub fn with_extent_config(mut self, extent_config: Vec<u8>) -> Self {
        self.flags |= CEXG_TILE_CONFIG_INFO_PRESENT;
        self.extent_config = extent_config;
        self
    }

    /// The FullBox flags.
    #[must_use]
    pub const fn flags(&self) -> u32 {
        self.flags
    }

    /// `rows_minus_one`.
    #[must_use]
    pub const fn rows_minus_one(&self) -> u16 {
        self.rows_minus_one
    }

    /// `columns_minus_one`.
    #[must_use]
    pub const fn columns_minus_one(&self) -> u16 {
        self.columns_minus_one
    }

    /// `image_tile_width`.
    #[must_use]
    pub const fn image_tile_width(&self) -> u32 {
        self.image_tile_width
    }

    /// `image_tile_height`.
    #[must_use]
    pub const fn image_tile_height(&self) -> u32 {
        self.image_tile_height
    }

    /// The `ExtentDecoderConfigurationRecord` bytes, coding-format specific and
    /// not interpreted by this crate.
    #[must_use]
    pub fn extent_config(&self) -> &[u8] {
        &self.extent_config
    }

    fn field_len(&self) -> usize {
        if self.flags & CEXG_FIELD_LENGTH != 0 {
            U32_LEN
        } else {
            U16_LEN
        }
    }
}

impl<'a> Parse<'a> for ConstrainedExtentsGridProperty {
    type Error = Error;

    fn parse(bytes: &'a [u8]) -> Result<Self> {
        let body = full_body(
            bytes,
            &CEXG,
            FULLBOX_EXTRA_SIZE + CEXG_GRID_FIELDS * U16_LEN,
            "cexg",
        )?;
        expect_version_zero(body, "cexg version")?;
        let flags = full_flags(body);
        let mut cursor = FULLBOX_EXTRA_SIZE;
        let rows_minus_one = be_u16(body, cursor, "cexg rows_minus_one")?;
        cursor += U16_LEN;
        let columns_minus_one = be_u16(body, cursor, "cexg columns_minus_one")?;
        cursor += U16_LEN;
        let width = if flags & CEXG_FIELD_LENGTH != 0 {
            U32_LEN
        } else {
            U16_LEN
        };
        let image_tile_width = read_unsigned(body, cursor, width, "cexg image_tile_width")?;
        cursor += width;
        let image_tile_height = read_unsigned(body, cursor, width, "cexg image_tile_height")?;
        cursor += width;
        let extent_config = if flags & CEXG_TILE_CONFIG_INFO_PRESENT != 0 {
            body.get(cursor..)
                .ok_or(Error::InvalidInput("cexg extent config"))?
                .to_vec()
        } else {
            if cursor != body.len() {
                return Err(Error::InvalidInput("cexg trailing bytes"));
            }
            Vec::new()
        };
        Ok(Self {
            flags,
            rows_minus_one,
            columns_minus_one,
            image_tile_width,
            image_tile_height,
            extent_config,
        })
    }
}

impl Serialize for ConstrainedExtentsGridProperty {
    type Error = Error;

    fn serialized_len(&self) -> usize {
        box_len(
            BOX_HEADER_MIN_SIZE
                + FULLBOX_EXTRA_SIZE
                + CEXG_GRID_FIELDS * U16_LEN
                + CEXG_GRID_FIELDS * self.field_len()
                + self.extent_config.len(),
        )
    }

    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        let width = self.field_len();
        let body_len =
            CEXG_GRID_FIELDS * U16_LEN + CEXG_GRID_FIELDS * width + self.extent_config.len();
        let mut cursor = begin_fullbox(buf, &CEXG, 0, self.flags, body_len)?;
        buf[cursor..cursor + U16_LEN].copy_from_slice(&self.rows_minus_one.to_be_bytes());
        cursor += U16_LEN;
        buf[cursor..cursor + U16_LEN].copy_from_slice(&self.columns_minus_one.to_be_bytes());
        cursor += U16_LEN;
        for value in [self.image_tile_width, self.image_tile_height] {
            write_unsigned(buf, cursor, value, width)?;
            cursor += width;
        }
        need_out(buf, cursor + self.extent_config.len())?;
        let end = cursor + self.extent_config.len();
        buf[cursor..end].copy_from_slice(&self.extent_config);
        cursor = end;
        Ok(cursor)
    }
}

/// Read a 16- or 32-bit unsigned field.
fn read_unsigned(body: &[u8], offset: usize, width: usize, what: &'static str) -> Result<u32> {
    if width == U32_LEN {
        Ok(be_u32(body, offset, what)?)
    } else {
        Ok(u32::from(be_u16(body, offset, what)?))
    }
}

/// Write a 16- or 32-bit unsigned field.
fn write_unsigned(buf: &mut [u8], offset: usize, value: u32, width: usize) -> Result<()> {
    if width == U32_LEN {
        need_out(buf, offset + U32_LEN)?;
        buf[offset..offset + U32_LEN].copy_from_slice(&value.to_be_bytes());
    } else {
        let narrowed = u16::try_from(value).map_err(|_| Error::InvalidInput("cexg tile size"))?;
        need_out(buf, offset + U16_LEN)?;
        buf[offset..offset + U16_LEN].copy_from_slice(&narrowed.to_be_bytes());
    }
    Ok(())
}

/// Disparity Adjustment property (`dadj`) — ISO/IEC 23008-12:2025 Amd 1
/// 6.5.42.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub struct DisparityAdjustmentProperty {
    disparity_adjustment: i32,
}

impl DisparityAdjustmentProperty {
    /// A disparity adjustment in 1/10 000 image widths.
    #[must_use]
    pub const fn new(disparity_adjustment: i32) -> Self {
        Self {
            disparity_adjustment,
        }
    }

    /// `disparity_adjustment`.
    #[must_use]
    pub const fn disparity_adjustment(&self) -> i32 {
        self.disparity_adjustment
    }
}

impl<'a> Parse<'a> for DisparityAdjustmentProperty {
    type Error = Error;

    fn parse(bytes: &'a [u8]) -> Result<Self> {
        let body = exact_full_body(bytes, &DADJ, FULLBOX_EXTRA_SIZE + U32_LEN, "dadj")?;
        Ok(Self {
            disparity_adjustment: read_signed(body, FULLBOX_EXTRA_SIZE, U32_LEN, "dadj")?,
        })
    }
}

impl Serialize for DisparityAdjustmentProperty {
    type Error = Error;

    fn serialized_len(&self) -> usize {
        box_len(BOX_HEADER_MIN_SIZE + FULLBOX_EXTRA_SIZE + U32_LEN)
    }

    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        let mut cursor = begin_fullbox(buf, &DADJ, 0, 0, U32_LEN)?;
        write_signed(buf, cursor, self.disparity_adjustment, U32_LEN)?;
        cursor += U32_LEN;
        Ok(cursor)
    }
}

/// A stereo aggressor type (Amd 1 6.5.43.3 Table 1).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub enum StereoAggressorType {
    /// `0`: unspecified.
    #[default]
    Unspecified,
    /// `1`: lens occlusion.
    LensOcclusion,
    /// `2`: image condition mismatch.
    ImageConditionMismatch,
    /// `3`: stereo window violation.
    StereoWindowViolation,
    /// `4`: object too close.
    ObjectTooClose,
    /// `5`: stereo calibration error.
    StereoCalibrationError,
    /// `6`: temporal alignment mismatch.
    TemporalAlignmentMismatch,
    /// `7`: poor image quality in one or both views.
    PoorImageQuality,
    /// A reserved value.
    Reserved(u8),
}

impl StereoAggressorType {
    /// The wire value.
    #[must_use]
    pub const fn value(self) -> u8 {
        match self {
            Self::Unspecified => 0,
            Self::LensOcclusion => 1,
            Self::ImageConditionMismatch => 2,
            Self::StereoWindowViolation => 3,
            Self::ObjectTooClose => 4,
            Self::StereoCalibrationError => 5,
            Self::TemporalAlignmentMismatch => 6,
            Self::PoorImageQuality => 7,
            Self::Reserved(value) => value,
        }
    }

    /// The type a wire value names.
    #[must_use]
    pub const fn from_value(value: u8) -> Self {
        match value {
            0 => Self::Unspecified,
            1 => Self::LensOcclusion,
            2 => Self::ImageConditionMismatch,
            3 => Self::StereoWindowViolation,
            4 => Self::ObjectTooClose,
            5 => Self::StereoCalibrationError,
            6 => Self::TemporalAlignmentMismatch,
            7 => Self::PoorImageQuality,
            other => Self::Reserved(other),
        }
    }

    /// The spec token.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::Unspecified => "unspecified",
            Self::LensOcclusion => "lens-occlusion",
            Self::ImageConditionMismatch => "image-condition-mismatch",
            Self::StereoWindowViolation => "stereo-window-violation",
            Self::ObjectTooClose => "object-too-close",
            Self::StereoCalibrationError => "stereo-calibration-error",
            Self::TemporalAlignmentMismatch => "temporal-alignment-mismatch",
            Self::PoorImageQuality => "poor-image-quality",
            Self::Reserved(_) => "reserved",
        }
    }
}

broadcast_core::impl_spec_display!(StereoAggressorType, Reserved);

/// One aggressor of a [`StereoAggressorsProperty`] (Amd 1 6.5.43.2).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub struct StereoAggressor {
    aggressor_type: StereoAggressorType,
    aggressor_severity: u8,
    sub_type_present: bool,
    sub_type_uri: Vec<u8>,
}

impl StereoAggressor {
    /// An aggressor of `aggressor_type` with severity `aggressor_severity`.
    #[must_use]
    pub fn new(aggressor_type: StereoAggressorType, aggressor_severity: u8) -> Self {
        Self {
            aggressor_type,
            aggressor_severity: aggressor_severity & STAG_SEVERITY_MASK,
            sub_type_present: false,
            sub_type_uri: Vec::new(),
        }
    }

    /// Add the absolute-metric sub-type URI.
    #[must_use]
    pub fn with_sub_type_uri(mut self, uri: &[u8]) -> Self {
        self.sub_type_present = true;
        self.sub_type_uri = uri.to_vec();
        self
    }

    /// `aggressor_type`.
    #[must_use]
    pub const fn aggressor_type(&self) -> StereoAggressorType {
        self.aggressor_type
    }

    /// `aggressor_severity`.
    #[must_use]
    pub const fn aggressor_severity(&self) -> u8 {
        self.aggressor_severity
    }

    /// Whether `sub_type_uri` is signalled, which the wire states separately
    /// from its length.
    #[must_use]
    pub const fn sub_type_present(&self) -> bool {
        self.sub_type_present
    }

    /// `sub_type_uri` (empty when absent, or when signalled and empty).
    #[must_use]
    pub fn sub_type_uri(&self) -> &[u8] {
        &self.sub_type_uri
    }
}

/// Stereo Aggressors property (`stag`) — ISO/IEC 23008-12:2025 Amd 1 6.5.43.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub struct StereoAggressorsProperty {
    aggressors: Vec<StereoAggressor>,
}

impl StereoAggressorsProperty {
    /// A property carrying `aggressors` (at least one).
    #[must_use]
    pub fn new(aggressors: Vec<StereoAggressor>) -> Self {
        Self { aggressors }
    }

    /// The aggressors.
    #[must_use]
    pub fn aggressors(&self) -> &[StereoAggressor] {
        &self.aggressors
    }
}

impl<'a> Parse<'a> for StereoAggressorsProperty {
    type Error = Error;

    fn parse(bytes: &'a [u8]) -> Result<Self> {
        let body = zero_full_body(bytes, &STAG, FULLBOX_EXTRA_SIZE + U8_LEN, "stag")?;
        let count = usize::from(body[FULLBOX_EXTRA_SIZE]) + 1;
        crate::box_support::check_entry_count(
            body.len() - (FULLBOX_EXTRA_SIZE + U8_LEN),
            STAG_ENTRY_FIELDS * U8_LEN,
            count,
            "stag aggressors",
        )?;
        let mut cursor = FULLBOX_EXTRA_SIZE + U8_LEN;
        let mut aggressors = Vec::with_capacity(count);
        for _ in 0..count {
            need_in(body, cursor + STAG_ENTRY_FIELDS * U8_LEN, "stag aggressor")?;
            let aggressor_type = StereoAggressorType::from_value(body[cursor]);
            cursor += U8_LEN;
            let packed = body[cursor];
            cursor += U8_LEN;
            let sub_type_present = packed & STAG_SUB_TYPE_MASK != 0;
            let sub_type_uri = if sub_type_present {
                let rest = body
                    .get(cursor..)
                    .ok_or(Error::InvalidInput("stag sub_type_uri"))?;
                let end = rest
                    .iter()
                    .position(|byte| *byte == 0)
                    .ok_or(Error::InvalidInput("stag sub_type_uri"))?;
                let uri = rest[..end].to_vec();
                cursor += end + 1;
                uri
            } else {
                Vec::new()
            };
            aggressors.push(StereoAggressor {
                aggressor_type,
                aggressor_severity: packed & STAG_SEVERITY_MASK,
                sub_type_present,
                sub_type_uri,
            });
        }
        if cursor != body.len() {
            return Err(Error::InvalidInput("stag trailing bytes"));
        }
        Ok(Self { aggressors })
    }
}

impl Serialize for StereoAggressorsProperty {
    type Error = Error;

    fn serialized_len(&self) -> usize {
        let entries: usize = self
            .aggressors
            .iter()
            .map(|entry| {
                STAG_ENTRY_FIELDS * U8_LEN
                    + if entry.sub_type_present {
                        entry.sub_type_uri.len() + 1
                    } else {
                        0
                    }
            })
            .sum();
        box_len(BOX_HEADER_MIN_SIZE + FULLBOX_EXTRA_SIZE + U8_LEN + entries)
    }

    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        let entries: usize = self
            .aggressors
            .iter()
            .map(|entry| {
                STAG_ENTRY_FIELDS * U8_LEN
                    + if entry.sub_type_present {
                        entry.sub_type_uri.len() + 1
                    } else {
                        0
                    }
            })
            .sum();
        let body_len = U8_LEN + entries;
        let mut cursor = begin_fullbox(buf, &STAG, 0, 0, body_len)?;
        let count = self.aggressors.len();
        if count == 0 {
            return Err(Error::InvalidInput("stag needs one aggressor"));
        }
        buf[cursor] = fit_u8(count - 1, "stag aggressor_count_minus_one")?;
        cursor += U8_LEN;
        for entry in &self.aggressors {
            buf[cursor] = entry.aggressor_type.value();
            cursor += U8_LEN;
            let present = if entry.sub_type_present {
                STAG_SUB_TYPE_MASK
            } else {
                0
            };
            let severity = fit_mask(
                u32::from(entry.aggressor_severity),
                u32::from(STAG_SEVERITY_MASK),
                "stag severity",
            )?;
            buf[cursor] = present | severity as u8;
            cursor += U8_LEN;
            if entry.sub_type_present {
                let end = cursor + entry.sub_type_uri.len() + 1;
                need_out(buf, end)?;
                buf[cursor..cursor + entry.sub_type_uri.len()].copy_from_slice(&entry.sub_type_uri);
                buf[cursor + entry.sub_type_uri.len()] = 0;
                cursor = end;
            }
        }
        Ok(cursor)
    }
}

/// The body length of a reference viewing environment property: two luminance
/// values and four chromaticity coordinates (Amd 1 6.5.44.2).
const REVE_LEN: usize = 2 * U32_LEN + 4 * U16_LEN;
/// `ndwt` body length: one luminance value (Amd 1 6.5.45.2).
const NDWT_LEN: usize = U32_LEN;

/// Reference Viewing Environment property (`reve`) — ISO/IEC 23008-12:2025
/// Amd 1 6.5.44.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub struct ReferenceViewingEnvironmentProperty {
    surround_luminance: u32,
    surround_light_x: u16,
    surround_light_y: u16,
    periphery_luminance: u32,
    periphery_light_x: u16,
    periphery_light_y: u16,
}

impl ReferenceViewingEnvironmentProperty {
    /// A reference viewing environment. Luminances are in 0.0001 cd/m^2 and
    /// chromaticities in 0.0001 steps.
    #[must_use]
    pub const fn new(
        surround_luminance: u32,
        surround_light_x: u16,
        surround_light_y: u16,
        periphery_luminance: u32,
        periphery_light_x: u16,
        periphery_light_y: u16,
    ) -> Self {
        Self {
            surround_luminance,
            surround_light_x,
            surround_light_y,
            periphery_luminance,
            periphery_light_x,
            periphery_light_y,
        }
    }

    /// `surround_luminance`.
    #[must_use]
    pub const fn surround_luminance(&self) -> u32 {
        self.surround_luminance
    }

    /// `surround_light_x`.
    #[must_use]
    pub const fn surround_light_x(&self) -> u16 {
        self.surround_light_x
    }

    /// `surround_light_y`.
    #[must_use]
    pub const fn surround_light_y(&self) -> u16 {
        self.surround_light_y
    }

    /// `periphery_luminance`.
    #[must_use]
    pub const fn periphery_luminance(&self) -> u32 {
        self.periphery_luminance
    }

    /// `periphery_light_x`.
    #[must_use]
    pub const fn periphery_light_x(&self) -> u16 {
        self.periphery_light_x
    }

    /// `periphery_light_y`.
    #[must_use]
    pub const fn periphery_light_y(&self) -> u16 {
        self.periphery_light_y
    }
}

impl<'a> Parse<'a> for ReferenceViewingEnvironmentProperty {
    type Error = Error;

    fn parse(bytes: &'a [u8]) -> Result<Self> {
        let body = exact_full_body(bytes, &REVE, FULLBOX_EXTRA_SIZE + REVE_LEN, "reve")?;
        let at = FULLBOX_EXTRA_SIZE;
        Ok(Self {
            surround_luminance: be_u32(body, at, "reve surround_luminance")?,
            surround_light_x: be_u16(body, at + U32_LEN, "reve surround_light_x")?,
            surround_light_y: be_u16(body, at + U32_LEN + U16_LEN, "reve surround_light_y")?,
            periphery_luminance: be_u32(
                body,
                at + U32_LEN + U16_PAIR_LEN,
                "reve periphery_luminance",
            )?,
            periphery_light_x: be_u16(
                body,
                at + U32_PAIR_LEN + U16_PAIR_LEN,
                "reve periphery_light_x",
            )?,
            periphery_light_y: be_u16(
                body,
                at + U32_PAIR_LEN + U16_TRIPLE_LEN,
                "reve periphery_light_y",
            )?,
        })
    }
}

impl Serialize for ReferenceViewingEnvironmentProperty {
    type Error = Error;

    fn serialized_len(&self) -> usize {
        box_len(BOX_HEADER_MIN_SIZE + FULLBOX_EXTRA_SIZE + REVE_LEN)
    }

    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        let mut cursor = begin_fullbox(buf, &REVE, 0, 0, REVE_LEN)?;
        buf[cursor..cursor + U32_LEN].copy_from_slice(&self.surround_luminance.to_be_bytes());
        cursor += U32_LEN;
        buf[cursor..cursor + U16_LEN].copy_from_slice(&self.surround_light_x.to_be_bytes());
        cursor += U16_LEN;
        buf[cursor..cursor + U16_LEN].copy_from_slice(&self.surround_light_y.to_be_bytes());
        cursor += U16_LEN;
        buf[cursor..cursor + U32_LEN].copy_from_slice(&self.periphery_luminance.to_be_bytes());
        cursor += U32_LEN;
        buf[cursor..cursor + U16_LEN].copy_from_slice(&self.periphery_light_x.to_be_bytes());
        cursor += U16_LEN;
        buf[cursor..cursor + U16_LEN].copy_from_slice(&self.periphery_light_y.to_be_bytes());
        cursor += U16_LEN;
        Ok(cursor)
    }
}

/// Nominal Diffuse White property (`ndwt`) — ISO/IEC 23008-12:2025 Amd 1
/// 6.5.45.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub struct NominalDiffuseWhiteProperty {
    diffuse_white_luminance: u32,
}

impl NominalDiffuseWhiteProperty {
    /// A nominal diffuse white of `diffuse_white_luminance` in 0.0001 cd/m^2
    /// (0 selects the ISO/TS 22028-5 default, Amd 1 6.5.45.3).
    #[must_use]
    pub const fn new(diffuse_white_luminance: u32) -> Self {
        Self {
            diffuse_white_luminance,
        }
    }

    /// `diffuse_white_luminance`.
    #[must_use]
    pub const fn diffuse_white_luminance(&self) -> u32 {
        self.diffuse_white_luminance
    }
}

impl<'a> Parse<'a> for NominalDiffuseWhiteProperty {
    type Error = Error;

    fn parse(bytes: &'a [u8]) -> Result<Self> {
        let body = exact_full_body(bytes, &NDWT, FULLBOX_EXTRA_SIZE + NDWT_LEN, "ndwt")?;
        Ok(Self {
            diffuse_white_luminance: be_u32(body, FULLBOX_EXTRA_SIZE, "ndwt")?,
        })
    }
}

impl Serialize for NominalDiffuseWhiteProperty {
    type Error = Error;

    fn serialized_len(&self) -> usize {
        box_len(BOX_HEADER_MIN_SIZE + FULLBOX_EXTRA_SIZE + NDWT_LEN)
    }

    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        let mut cursor = begin_fullbox(buf, &NDWT, 0, 0, NDWT_LEN)?;
        buf[cursor..cursor + NDWT_LEN].copy_from_slice(&self.diffuse_white_luminance.to_be_bytes());
        cursor += NDWT_LEN;
        Ok(cursor)
    }
}

/// Mask Configuration property (`mskC`) — ISO/IEC 23008-12:2025 11.2.2.2.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub struct MaskConfigurationProperty {
    bits_per_pixel: u8,
}

impl MaskConfigurationProperty {
    /// A mask of `bits_per_pixel` bits per pixel (1, 2, 4, 8, 16 or 24).
    #[must_use]
    pub const fn new(bits_per_pixel: u8) -> Self {
        Self { bits_per_pixel }
    }

    /// `bits_per_pixel`.
    #[must_use]
    pub const fn bits_per_pixel(&self) -> u8 {
        self.bits_per_pixel
    }
}

impl<'a> Parse<'a> for MaskConfigurationProperty {
    type Error = Error;

    fn parse(bytes: &'a [u8]) -> Result<Self> {
        let body = exact_full_body(bytes, &MSKC, FULLBOX_EXTRA_SIZE + U8_LEN, "mskC")?;
        Ok(Self {
            bits_per_pixel: body[FULLBOX_EXTRA_SIZE],
        })
    }
}

impl Serialize for MaskConfigurationProperty {
    type Error = Error;

    fn serialized_len(&self) -> usize {
        box_len(BOX_HEADER_MIN_SIZE + FULLBOX_EXTRA_SIZE + U8_LEN)
    }

    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        let mut cursor = begin_fullbox(buf, &MSKC, 0, 0, U8_LEN)?;
        buf[cursor] = self.bits_per_pixel;
        cursor += U8_LEN;
        Ok(cursor)
    }
}
