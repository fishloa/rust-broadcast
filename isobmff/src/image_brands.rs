//! Image file format brands — ISO/IEC 23008-12:2025 clause 10.
//!
//! [`ImageBrand`] names every brand clause 10 and the codec annexes define;
//! [`ImageBrands`] is the typed view of a file's `ftyp` brand set, its
//! structural brand class, its reader requirements ([`ImageBrands::reader_properties`])
//! and the structurally checkable requirements ([`ImageBrands::findings`]).
//! A finding is reported, never a refusal to read (10.2.1).
//!
//! Sources: `private/specs-verified/mpeg/heif/iso-iec-23008-12-2025.md` clause
//! 10 and `private/specs-verified/mpeg/heif/iso-iec-23008-12-2025-amd1-2025.md`
//! 10.2.6.

use alloc::vec::Vec;

use crate::box_types::BOX_TYPE_SIZE;

/// The `mif1` structural brand (10.2.2).
pub const BRAND_MIF1: [u8; BOX_TYPE_SIZE] = *b"mif1";
/// The `mif2` structural brand (10.2.3).
pub const BRAND_MIF2: [u8; BOX_TYPE_SIZE] = *b"mif2";
/// The `pred` brand (10.2.4).
pub const BRAND_PRED: [u8; BOX_TYPE_SIZE] = *b"pred";
/// The `1pic` brand (10.2.5).
pub const BRAND_1PIC: [u8; BOX_TYPE_SIZE] = *b"1pic";
/// The `tmap` brand (Amd 1 10.2.6).
pub const BRAND_TMAP: [u8; BOX_TYPE_SIZE] = *b"tmap";
/// The `msf1` image-sequence structural brand (10.3.1).
pub const BRAND_MSF1: [u8; BOX_TYPE_SIZE] = *b"msf1";
/// The `iso8` base-media brand `msf1` requires (10.3.1.1).
pub const BRAND_ISO8: [u8; BOX_TYPE_SIZE] = *b"iso8";
/// The `unif` brand (Amd 1 6.5.42, 6.8.11).
pub const BRAND_UNIF: [u8; BOX_TYPE_SIZE] = *b"unif";
/// The HEVC image brand (Annex B/C).
pub const BRAND_HEIC: [u8; BOX_TYPE_SIZE] = *b"heic";
/// The HEVC image brand with `hvc1` (Annex B).
pub const BRAND_HEIX: [u8; BOX_TYPE_SIZE] = *b"heix";
/// The HEVC multi-image brand (Annex B).
pub const BRAND_HEIM: [u8; BOX_TYPE_SIZE] = *b"heim";
/// The HEVC multi-image brand with `hvc1` (Annex B).
pub const BRAND_HEIS: [u8; BOX_TYPE_SIZE] = *b"heis";
/// The AVIF image brand (AVIF 6.2).
pub const BRAND_AVIF: [u8; BOX_TYPE_SIZE] = *b"avif";
/// The AVIF intra-only brand (AVIF 6.2).
pub const BRAND_AVIO: [u8; BOX_TYPE_SIZE] = *b"avio";
/// The AVIF image-sequence brand (AVIF 6.3).
pub const BRAND_AVIS: [u8; BOX_TYPE_SIZE] = *b"avis";

/// One image file format brand (clause 10 and the codec annexes).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub enum ImageBrand {
    /// `mif1`.
    Mif1,
    /// `mif2`.
    Mif2,
    /// `pred`.
    Pred,
    /// `1pic`.
    Pic1,
    /// `tmap`.
    Tmap,
    /// `msf1`.
    Msf1,
    /// `iso8`.
    Iso8,
    /// `unif`.
    Unif,
    /// `heic`.
    Heic,
    /// `heix`.
    Heix,
    /// `heim`.
    Heim,
    /// `heis`.
    Heis,
    /// `avif`.
    Avif,
    /// `avio`.
    Avio,
    /// `avis`.
    Avis,
    /// Any other brand.
    Other([u8; BOX_TYPE_SIZE]),
}

impl ImageBrand {
    /// The brand a four-CC names.
    #[must_use]
    pub const fn from_fourcc(fourcc: [u8; BOX_TYPE_SIZE]) -> Self {
        match &fourcc {
            b"mif1" => Self::Mif1,
            b"mif2" => Self::Mif2,
            b"pred" => Self::Pred,
            b"1pic" => Self::Pic1,
            b"tmap" => Self::Tmap,
            b"msf1" => Self::Msf1,
            b"iso8" => Self::Iso8,
            b"unif" => Self::Unif,
            b"heic" => Self::Heic,
            b"heix" => Self::Heix,
            b"heim" => Self::Heim,
            b"heis" => Self::Heis,
            b"avif" => Self::Avif,
            b"avio" => Self::Avio,
            b"avis" => Self::Avis,
            _ => Self::Other(fourcc),
        }
    }

    /// The brand's four-CC.
    #[must_use]
    pub const fn fourcc(self) -> [u8; BOX_TYPE_SIZE] {
        match self {
            Self::Mif1 => BRAND_MIF1,
            Self::Mif2 => BRAND_MIF2,
            Self::Pred => BRAND_PRED,
            Self::Pic1 => BRAND_1PIC,
            Self::Tmap => BRAND_TMAP,
            Self::Msf1 => BRAND_MSF1,
            Self::Iso8 => BRAND_ISO8,
            Self::Unif => BRAND_UNIF,
            Self::Heic => BRAND_HEIC,
            Self::Heix => BRAND_HEIX,
            Self::Heim => BRAND_HEIM,
            Self::Heis => BRAND_HEIS,
            Self::Avif => BRAND_AVIF,
            Self::Avio => BRAND_AVIO,
            Self::Avis => BRAND_AVIS,
            Self::Other(fourcc) => fourcc,
        }
    }

    /// Whether the brand declares a still-image item collection.
    #[must_use]
    pub const fn is_item_brand(self) -> bool {
        matches!(
            self,
            Self::Mif1
                | Self::Mif2
                | Self::Pred
                | Self::Pic1
                | Self::Tmap
                | Self::Heic
                | Self::Heix
                | Self::Heim
                | Self::Heis
                | Self::Avif
                | Self::Avio
        )
    }

    /// Whether the brand declares an image sequence.
    #[must_use]
    pub const fn is_sequence_brand(self) -> bool {
        matches!(self, Self::Msf1 | Self::Avis)
    }

    /// The spec token.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::Mif1 => "mif1",
            Self::Mif2 => "mif2",
            Self::Pred => "pred",
            Self::Pic1 => "1pic",
            Self::Tmap => "tmap",
            Self::Msf1 => "msf1",
            Self::Iso8 => "iso8",
            Self::Unif => "unif",
            Self::Heic => "heic",
            Self::Heix => "heix",
            Self::Heim => "heim",
            Self::Heis => "heis",
            Self::Avif => "avif",
            Self::Avio => "avio",
            Self::Avis => "avis",
            Self::Other(_) => "other",
        }
    }
}

broadcast_core::impl_spec_display!(ImageBrand);

/// The item properties a reader of a brand shall recognize (Tables 5-7).
///
/// `mif2` and `pred` add `rref`; `mif2` also adds `iscl`. Everything a
/// `mif1` reader recognizes is included for every brand that requires `mif1`.
#[must_use]
pub fn reader_properties(brand: ImageBrand) -> Vec<[u8; BOX_TYPE_SIZE]> {
    let mut properties: Vec<[u8; BOX_TYPE_SIZE]> = alloc::vec![
        *b"ispe", *b"pasp", *b"colr", *b"pixi", *b"rloc", *b"auxC", *b"clap", *b"irot", *b"imir",
    ];
    if matches!(brand, ImageBrand::Mif2 | ImageBrand::Pred) {
        properties.push(*b"rref");
    }
    if brand == ImageBrand::Mif2 {
        properties.push(*b"iscl");
    }
    properties
}

/// A structurally checkable brand requirement (clause 10).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub enum BrandFinding {
    /// `msf1` is declared without the `iso8` brand (10.3.1.1).
    Msf1WithoutIso8,
    /// The `tmap` brand is declared with no tone-map derived image item
    /// (Amd 1 10.2.6.2).
    TmapWithoutToneMapItem,
}

impl BrandFinding {
    /// The spec token.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::Msf1WithoutIso8 => "msf1-without-iso8",
            Self::TmapWithoutToneMapItem => "tmap-without-tone-map-item",
        }
    }
}

broadcast_core::impl_spec_display!(BrandFinding);

/// A file's typed brand set and the findings it implies.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub struct ImageBrands {
    major_brand: [u8; BOX_TYPE_SIZE],
    compatible_brands: Vec<[u8; BOX_TYPE_SIZE]>,
}

impl ImageBrands {
    /// The brand set of a `ftyp`.
    #[must_use]
    pub fn new(
        major_brand: [u8; BOX_TYPE_SIZE],
        compatible_brands: Vec<[u8; BOX_TYPE_SIZE]>,
    ) -> Self {
        Self {
            major_brand,
            compatible_brands,
        }
    }

    /// `major_brand`.
    #[must_use]
    pub const fn major_brand(&self) -> ImageBrand {
        ImageBrand::from_fourcc(self.major_brand)
    }

    /// The compatible brands.
    #[must_use]
    pub fn compatible_brands(&self) -> &[[u8; BOX_TYPE_SIZE]] {
        &self.compatible_brands
    }

    /// Whether `brand` is the major or a compatible brand.
    #[must_use]
    pub fn has(&self, brand: [u8; BOX_TYPE_SIZE]) -> bool {
        self.major_brand == brand || self.compatible_brands.contains(&brand)
    }

    /// The brands a still-image reader must recognize the properties of.
    #[must_use]
    pub fn reader_properties(&self) -> Vec<[u8; BOX_TYPE_SIZE]> {
        let mut properties = Vec::new();
        for fourcc in
            core::iter::once(self.major_brand).chain(self.compatible_brands.iter().copied())
        {
            for property in reader_properties(ImageBrand::from_fourcc(fourcc)) {
                if !properties.contains(&property) {
                    properties.push(property);
                }
            }
        }
        properties
    }

    /// The brand-set findings (a `tmap` brand needs the item model to check, so
    /// that one is reported by the item reader).
    #[must_use]
    pub fn findings(&self) -> Vec<BrandFinding> {
        let mut findings = Vec::new();
        if self.has(BRAND_MSF1) && !self.has(BRAND_ISO8) {
            findings.push(BrandFinding::Msf1WithoutIso8);
        }
        findings
    }
}

/// A structurally checkable requirement of the item layer (clauses 6.8, 11).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub enum ImageFinding {
    /// A `tmap` derived image item does not have exactly two `dimg` inputs
    /// (Amd 1 6.6.2.4.1).
    ToneMapWrongInputCount {
        /// The derived item.
        item: crate::item::ItemId,
        /// The `dimg` reference count.
        count: usize,
    },
    /// An entity group names an entity with no item and no group of that
    /// identifier (6.5.1).
    GroupNamesMissingEntity {
        /// The group.
        group: crate::item::GroupId,
        /// The unresolved entity identifier.
        entity: u32,
    },
    /// A `ster` group does not contain exactly two image items (6.8.5).
    StereoGroupWrongEntityCount {
        /// The group.
        group: crate::item::GroupId,
        /// The entity count.
        count: usize,
    },
    /// A `stem` group does not contain exactly three image items
    /// (Amd 1 6.8.11).
    StereoFallbackGroupWrongEntityCount {
        /// The group.
        group: crate::item::GroupId,
        /// The entity count.
        count: usize,
    },
    /// An `iaug` group does not contain exactly two entities (6.8.4).
    AudioImageGroupWrongEntityCount {
        /// The group.
        group: crate::item::GroupId,
        /// The entity count.
        count: usize,
    },
    /// A region item's `cdsc` reference names no item (11.3.2).
    RegionNamesMissingImage {
        /// The region item.
        item: crate::item::ItemId,
        /// The missing target.
        target: crate::item::ItemId,
    },
    /// A region item's referenced-mask regions and its `mask` references do
    /// not line up (11.3.2.1).
    RegionMaskReferenceCount {
        /// The region item.
        item: crate::item::ItemId,
        /// The regions whose geometry is a referenced mask.
        regions: usize,
        /// The `mask` item references.
        references: usize,
    },
    /// A box the reader types was in a form the crate cannot reproduce, so it
    /// was kept opaque, byte-exact, and reading continued.
    UnrepresentableBox {
        /// The box four-CC.
        box_type: [u8; crate::box_types::BOX_TYPE_SIZE],
        /// The box's absolute byte offset in the file.
        offset: u64,
        /// Why the typed parse did not represent it.
        reason: crate::error::OpaqueReason,
    },
    /// A box the item model needs carried bytes past its last declared field.
    /// They are kept verbatim and written back; nothing about the table is
    /// lost.
    TrailingBytes {
        /// The box four-CC.
        box_type: [u8; crate::box_types::BOX_TYPE_SIZE],
        /// The box's absolute byte offset in the file.
        offset: u64,
        /// How many bytes past its last field the box carried.
        carried: u64,
    },
    /// The file carried a second file-level `meta` box. The first is the item
    /// model's; this one is carried opaque in its own position and re-emitted
    /// byte-exact.
    ExtraMetaBox {
        /// The second box's absolute byte offset in the file.
        offset: u64,
    },
    /// An item's data could not be addressed: an extent left the file, or the
    /// data reference it named is not this file. The item is kept, without
    /// data, and everything else plays.
    ItemDataDropped {
        /// The item whose data was dropped.
        item: crate::item::ItemId,
        /// The extent's first byte, as the file stated it.
        offset: u64,
        /// The extent's declared length.
        length: u64,
    },
    /// A box declared more bytes than the input holds; it was read to the end
    /// of the input and the cut is reported.
    TruncatedBox {
        /// The box four-CC.
        box_type: [u8; crate::box_types::BOX_TYPE_SIZE],
        /// The box's absolute byte offset in the file.
        offset: u64,
    },
    /// A gain map metadata rational has a zero denominator, which no value can
    /// be divided by.
    GainMapZeroDenominator {
        /// The `tmap` item.
        item: crate::item::ItemId,
        /// The channel index, absent for an HDR headroom.
        channel: Option<usize>,
        /// The field.
        field: crate::image::GainMapField,
    },
}

impl ImageFinding {
    /// The finding's stable token.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::ToneMapWrongInputCount { .. } => "tmap-wrong-input-count",
            Self::GroupNamesMissingEntity { .. } => "group-names-missing-entity",
            Self::StereoGroupWrongEntityCount { .. } => "ster-wrong-entity-count",
            Self::StereoFallbackGroupWrongEntityCount { .. } => "stem-wrong-entity-count",
            Self::AudioImageGroupWrongEntityCount { .. } => "iaug-wrong-entity-count",
            Self::RegionNamesMissingImage { .. } => "region-names-missing-image",
            Self::RegionMaskReferenceCount { .. } => "region-mask-reference-count",
            Self::ItemDataDropped { .. } => "item-data-dropped",
            Self::TruncatedBox { .. } => "truncated-box",
            Self::GainMapZeroDenominator { .. } => "tmap-zero-denominator",
            Self::UnrepresentableBox { .. } => "unrepresentable-box",
            Self::TrailingBytes { .. } => "trailing-bytes",
            Self::ExtraMetaBox { .. } => "extra-meta-box",
        }
    }
}

broadcast_core::impl_spec_display!(ImageFinding);
