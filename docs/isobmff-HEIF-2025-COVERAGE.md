# `isobmff` still images against ISO/IEC 23008-12:2025

The third edition (2025-07) and Amendment 1 (2025-10) are transcribed in the
private repo at
`private/specs-verified/mpeg/heif/iso-iec-23008-12-2025.md` and
`private/specs-verified/mpeg/heif/iso-iec-23008-12-2025-amd1-2025.md`. This
file records what the crate types, what it carries opaque, and what is OPEN.

Status vocabulary:

- **typed + fixture**: a typed type, asserted against a real file written by
  another tool and re-emitted byte-exact.
- **typed + clause**: a typed type, exercised by clause-assembled bytes (no
  installed tool writes it).
- **opaque by design**: the structure is defined only in a document that is not
  held; its bytes are carried verbatim in a typed wrapper.
- **OPEN**: not implemented; the reason is stated.
- **n/a**: no structure to type.

## G1 — every sub-clause

### 6.5 Image properties

| Clause | Type | Status |
|---|---|---|
| 6.5.1 General | property order preserved; descriptive-after-transform rule | OPEN: order kept, the ignore rule is not enforced |
| 6.5.2 Decoder configuration | existing `hvcC`/`avcC`/`av1C` | typed + fixture |
| 6.5.3 Image spatial extents | `ImageSpatialExtentsProperty` | typed + fixture |
| 6.5.4 Pixel aspect ratio | `PixelAspectRatioBox` | typed + fixture |
| 6.5.5 Colour information | `ColourInformationBox` | typed + fixture |
| 6.5.6 Pixel information | `PixelInformationProperty` | typed + fixture |
| 6.5.7 Relative location | `RelativeLocationProperty` | typed + clause |
| 6.5.8 Auxiliary type | `AuxiliaryTypeProperty` | typed + fixture |
| 6.5.9 Clean aperture | `CleanApertureBox` | typed + fixture |
| 6.5.10 Image rotation | `ImageRotation` | typed + fixture |
| 6.5.11 Layer selection | `LayerSelectorProperty` | typed + clause |
| 6.5.12 Image mirroring | `ImageMirror` | typed + fixture |
| 6.5.13 Image scaling | `ImageScaling` | typed + clause |
| 6.5.14 Content light level | `ContentLightLevel` | typed + fixture |
| 6.5.15 Mastering display colour volume | `MasteringDisplayColourVolume` | typed + clause |
| 6.5.16 Content colour volume | `ContentColourVolumeBox` | typed + clause |
| 6.5.17 Required reference types | `RequiredReferenceTypesProperty` | typed + clause |
| 6.5.18 Creation time | `CreationTimeProperty` | typed + clause |
| 6.5.19 Modification time | `ModificationTimeProperty` | typed + clause |
| 6.5.20 User description | `UserDescriptionProperty` | typed + fixture |
| 6.5.21 Accessibility text | `AccessibilityTextProperty` | typed + clause |
| 6.5.22 Auto exposure | `AutoExposureProperty` | typed + clause |
| 6.5.23 White balance | `WhiteBalanceProperty` | typed + clause |
| 6.5.24 Focus | `FocusProperty` | typed + clause |
| 6.5.25 Flash exposure | `FlashExposureProperty` | typed + clause |
| 6.5.26 Depth of field | `DepthOfFieldProperty` | typed + clause |
| 6.5.27 Panorama | `PanoramaProperty` | typed + clause |
| 6.5.28 Sub-sample information | `SubSampleInformationBox` reused | typed + clause |
| 6.5.29 Target output layer set | `TargetOutputLayerSetProperty` | typed + clause |
| 6.5.30 Wipe transition | `WipeTransitionEffectProperty` | typed + clause |
| 6.5.31 Zoom transition | `ZoomTransitionEffectProperty` | typed + clause |
| 6.5.32 Fade transition | `FadeTransitionEffectProperty` | typed + clause |
| 6.5.33 Split transition | `SplitTransitionEffectProperty` | typed + clause |
| 6.5.34 Suggested transition period | `SuggestedTransitionPeriodProperty` | typed + clause |
| 6.5.35 Suggested time display duration | `SuggestedTimeDisplayDurationProperty` | typed + clause |
| 6.5.36 Ambient viewing environment | `AmbientViewingEnvironmentBox` | typed + clause |
| 6.5.37 Progressive derived image information | `ProgressiveDerivedImageItemInformationProperty` | typed + clause |
| 6.5.38 Single stream | `SingleStreamProperty` | typed + clause |
| 6.5.39 Camera extrinsic matrix | `CameraExtrinsicMatrixProperty` | typed v0 + clause; v1 orientation opaque by design (`ISO/IEC 23090-7` not held) |
| 6.5.40 Camera intrinsic matrix | `CameraIntrinsicMatrixProperty` | typed + clause |
| 6.5.41 Constrained extents grid (Amd 1) | `ConstrainedExtentsGridProperty` | typed + clause; the per-extent decoder configuration record is coding-format specific and opaque |
| 6.5.42 Disparity adjustment (Amd 1) | `DisparityAdjustmentProperty` | typed + clause |
| 6.5.43 Stereo aggressors (Amd 1) | `StereoAggressorsProperty` | typed + clause |
| 6.5.44 Reference viewing environment (Amd 1) | `ReferenceViewingEnvironmentProperty` | typed + clause |
| 6.5.45 Nominal diffuse white (Amd 1) | `NominalDiffuseWhiteProperty` | typed + clause |

### 6.6 Derived images

| Clause | Type | Status |
|---|---|---|
| 6.6.1 General | `DerivedImage`, `dimg` references | typed + fixture |
| 6.6.2.1 Identity | `IdentityDerivation` | typed + clause |
| 6.6.2.2 Overlay | `ImageOverlay` | typed + clause |
| 6.6.2.3 Grid | `ImageGrid` | typed + fixture |
| 6.6.2.4 Tone-map (Amd 1) | `ToneMapImage` | typed + clause; the ISO 21496-1 gain-map metadata is opaque by design |
| 6.6.2.5 Colour format enhancement (Amd 1) | `ColourFormatEnhancement` | typed + clause |

### 6.8 Entity and sample groups

| Clause | Type | Status |
|---|---|---|
| 6.8.1 Relating an untimed item to a timed sequence | `Eqiv` group, `thmb`/`auxl`/`base` references | group typed; the `eqiv` *sample group entry* (`time_offset`, `timescale_multiplier`) is OPEN |
| 6.8.2 Burst images | `Brst` group | typed + clause |
| 6.8.3 `tsyn` group | `Tsyn` group | typed + clause |
| 6.8.4 `iaug` group | `Iaug` group | typed + clause |
| 6.8.5 `ster` group | `Ster` group | typed + fixture |
| 6.8.6 Bracketed sets | `Aebr`/`Wbbr`/`Fobr`/`Afbr`/`Dobr` groups | groups typed; the five bracketing *sample group entries* are OPEN |
| 6.8.7 User-defined image collections | `Albc`/`Favc` groups | typed + clause |
| 6.8.8 Panorama | `Pano` group + `PanoramaProperty` | group typed + clause; the `pano` sample group entry is OPEN |
| 6.8.9 Slideshow | `Slid` group + transition properties | typed + fixture |
| 6.8.10 Progressive rendering | `Prgr` group + `prdi` | typed + fixture |
| 6.8.11 `stem` (Amd 1) | `Stem` group | typed + clause |
| 6.8.12 Image pyramid (Amd 1) | `Pymd` group, `ImagePyramidEntityGroup` body | typed + fixture (no-body form) and clause (tile sizes) |
| 6.8.13 Region partition (Amd 1) | `Rgpa` group, `RegionPartitionGroup` body | typed + clause |

### 7 Image sequences

| Clause | Type | Status |
|---|---|---|
| 7.1 General | `TrackInfo::is_image_sequence` (`pict` handler) | typed + fixture (movie reader) |
| 7.2.1 Track Header matrix constraints | — | OPEN: prose over an existing box, not checked |
| 7.2.2 Handler type | `TrackInfo::is_image_sequence` | typed |
| 7.2.3 Coding Constraints box | `CodingConstraintsBox`, `TrackInfo::coding_constraints` | typed + clause |
| 7.3 Presentation / hidden samples | — | OPEN: prose rule over existing boxes |
| 7.4.1 Direct reference samples list | `DirectReferenceSamplesList` | typed + clause; not yet dispatched from `sgpd` (OPEN) |
| 7.5.2 Thumbnail image sequence track | existing `thmb` track reference | facts reachable |
| 7.5.3 Auxiliary image sequence track | `AuxiliaryTypeInfoBox`, `TrackInfo::auxiliary_type_info` | typed + clause |

### 8 Metadata and integrity

| Clause | Type | Status |
|---|---|---|
| 8.1 General | — | n/a |
| 8.2.1 Metadata for image items | `cdsc` references in the item model | facts reachable |
| 8.2.2 Deductive information | `Uri` item type + `Dpnd` reference | typed; `uuid` properties stay opaque |
| 8.2.2.2 Cascaded deductive information | `Dpnd` reference | typed + clause |
| 8.3 Metadata for image sequence tracks | existing `meta_track`/`cdsc` track references | facts reachable |
| 8.4 Integrity checks | `Md5IntegrityBox`, `IntegrityItem` | typed + clause |

### 10 Brands

| Clause | Type | Status |
|---|---|---|
| 10.1 General | `ImageBrand` | typed |
| 10.2.1 Brand requirements | `ImageBrands::findings` | partial: only structurally checkable findings |
| 10.2.2 `mif1` | `ImageBrands::reader_properties` | typed; mandatory-box checks OPEN |
| 10.2.3 `mif2` | reader properties, `Prem` reference | typed |
| 10.2.4 `pred` | `Pred` reference, reader properties | typed |
| 10.2.5 `1pic` | brand | typed; codec constraint not checkable structurally |
| 10.2.6 `tmap` (Amd 1) | brand + `TmapWithoutToneMapItem` finding | typed + clause |
| 10.3.1 `msf1` | brand + `Msf1WithoutIso8` finding | typed; the `pict`-track requirement is not cross-checked by the item reader (OPEN) |

### 11 Regions

| Clause | Type | Status |
|---|---|---|
| 11.1 Overview | — | n/a |
| 11.2.1 Region geometry | `RegionGeometry` (all geometry types, both widths) | typed + clause |
| 11.2.2 Mask item | `Mski` item type, `MaskConfigurationProperty` | typed + clause; pixel data opaque |
| 11.3.1 General | `cdsc`/`eroi`/`mask` references | typed + clause |
| 11.3.2 Region item | `RegionItem` | typed + clause |
| 11.3.3 Derived region item | `Drgn` reference; `iden` body | typed + clause |
| 11.3.4 Merge region item (Amd 1) | `base` references | typed + clause (no new syntax) |
| 11.4.1 General | track references | facts reachable |
| 11.4.2 Region track | `RegionTrackConfigBox`, `RegionSample` | typed + clause; the `rgan` sample entry is not in the movie reader's typed sample-entry dispatch (OPEN) |
| 11.4.3.1 `regm` sample group | `RegionAnnotationMap` | typed + clause; not dispatched from `sgpd` (OPEN) |
| 11.4.3.2 `udes` sample group | `UserDescriptionSampleGroupEntry` | typed + clause; not dispatched from `sgpd` (OPEN) |

## G2 — 2017 vs 2025 re-check

| 2017 item | 2025 clause | Difference | Change |
|---|---|---|---|
| `ispe` | 6.5.3 | Semantics extend to text items; syntax same | No syntax change; module citation updated to 2025 |
| `pasp` | 6.5.4 | Same syntax and semantics | No difference |
| `colr` | 6.5.5 | Adds the two-`colr` pairing rules (semantics only) | No syntax change; the model keeps every association in order |
| `pixi` | 6.5.6 | Same | No difference |
| `rloc` | 6.5.7 | Same | No difference |
| `auxC` | 6.5.8 | Same (the 2020 corrigendum's spelling is in the 2025 text) | No difference |
| `clap` | 6.5.9 | Same | No difference |
| `irot` | 6.5.10 | Same | No difference |
| `lsel` | 6.5.11 | Same | No difference |
| `imir` | 6.5.12 | Same | No difference |
| `iden` | 6.6.2.1 | Reference wording now names `SingleItemTypeReferenceBoxLarge` too | No syntax change; `iref` is already one model |
| `iovl` | 6.6.2.2 | Same large-reference wording | No syntax change |
| `grid` | 6.6.2.3 | Same large-reference wording | No syntax change |
| `ster` | 6.8.5 | Semantics unchanged; Amd 1 adds the `stem` note | No change to `ster` |
| `eqiv` | 6.8.1 | Semantics unchanged | No change to the entity group |
| `mint` | 8.4 (was 9.8) | Clause moved | Citations updated to 8.4 |
| brands | 10.2 | `mif1`/`mif2`/`pred`/`1pic` read requirements unchanged | Typed view added |

## OPEN

1. 6.5.1: the reader preserves property order but does not drop descriptive
   properties that follow the first transformative one.
2. 6.5.39 version 1: the `ViewpointGlobalCoordinateSysRotationStruct` is
   defined in ISO/IEC 23090-7, which is not held; it and the fields after it are
   carried opaque.
3. 6.6.2.4: the ISO 21496-1 `GainMapMetadata` is not held; the bytes are
   carried opaque.
4. 6.8.1.2, 6.8.6, 6.8.8.2: the `eqiv`, `aebr`/`wbbr`/`fobr`/`afbr`/`dobr` and
   `pano` sample group entries are not typed.
5. 7.2.1 and 7.3: the track-header matrix constraints and the hidden-sample
   rule are prose over existing boxes and are not checked.
6. 7.4.1, 11.4.3.1 and 11.4.3.2: the typed sample group entries are not yet
   dispatched from `SampleGroupDescriptionBox`; their bytes stay opaque there.
7. 11.4.2: the `rgan` region-track sample entry is not yet in the movie
   reader's typed sample-entry dispatch.
8. 10.2.2, 10.3.1: mandatory-box and `pict`-track requirements are not
   cross-checked by the item reader.
