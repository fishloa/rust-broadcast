# Changelog

All notable changes to `isobmff` will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added

- The reader takes a caller policy for what a report of skipped content means
  (`ReaderConfig::with_damage_policy`, `ItemReaderConfig::with_damage_policy`,
  `broadcast_core::damage::DamagePolicy`): the default publishes every report
  and continues, `Disposition::Fail` stops the read with the same `Damage`
  inside the new `Error::Damage`. `ReaderConfig::with_resync_scan_limit` bounds
  how far the walk may scan for the next top-level box header after one declares
  an impossible size (`None`, the default, scans to the end of the input).
- `Reader::set_total_len`: a range-reading caller learns the input's length
  from its first response, and a truncated input can only be reported as
  truncated once the reader knows how long it is.
- `CursorState::tolerant` (crate-internal) reads a track's sample tables as far
  as they go, reporting each table it had to do without and cutting the sample
  count to the shortest extent every table agrees on.

### Breaking

- Best effort is now the default for **both** readers: `Mode::Permissive` is
  the default of `Reader`/`ReaderConfig` and `Mode::Strict` is the explicit
  opt-in a validator asks for. A caller that relied on strict-by-default must
  call `with_mode(Mode::Strict)` (`media-doctor` does).
- The readers publish the workspace's one report instead of their own
  vocabulary: `Reader::step` yields `Step::Damage(Damage)` where it used to
  yield `Step::Warning(Warning)`, and `DriverStep::Warning`/`Advance::Warning`
  are now `Damage`; `ItemReader::step` gains `ItemStep::Damage`.
  `reader::Warning`, `image_brands::ImageFinding` and
  `image_brands::BrandFinding` stay as the typed detail and convert with
  `to_damage()`/`From`, which is where the four-character code, byte range and
  recoverability of each report are decided.
- `ItemReaderConfig` is no longer `Copy` (it carries a `DamagePolicy`);
  `Clone`/`PartialEq`/`Eq` remain.
- `Reader::step` ends with `Error::InvalidInput` when a file carrying media
  states no track at all, or carries a `moov`/`moof` nowhere. A still image and
  a file of type declarations alone still finish with `Step::Done`.

- `multiview`: the multiview view-attribute boxes with a defined syntax —
  `ProfileLevelAttribute` (`prfl`/`levl`), `ViewArrayOrder` (`ilvi`/`elvi`),
  `RectangularViewArray` (`rtvi`), `MultiviewSceneInfoBox` (`vwdi`) and
  `MultiviewInformationBox` (`mvci`). `geom`, `plvi` and `spvi` are pinned as
  not applicable: the attribute table states their syntax as Unspecified.

- `nalu_sample_entry::NaluVideoSampleEntry`: the NAL-unit video sample entries
  (`mvc1`…`mvc4`, `svc1`, `svc2`, `svcM`, `mvd1`…`mvd4`, `a3d1`…`a3d4`,
  `hvc2`/`hev2`, `hvt1`, `lhv1`, `lht1`) as one type over the standard visual
  fixed fields, dispatched from `stsd`.
- `image::JpegConfigurationProperty` (`jpgC`), typed both as an item property
  and as a sample-entry box, and `image_sequence::MjpegSampleEntry` (`mjpg`)
  with its `jpeg_configuration()` accessor. `image_sequence::
  AuxiliaryTypeInfoBox` (`auxi`) and the `evir` item reference join the typed
  tables.
- `Av1SampleEntry::codec_type`: the AV1 entry records its four-CC so the Dolby
  Vision `dav1` code can use it (`dva1`/`dvav` join the AVC entry's dispatch).

### Breaking

- `Av1SampleEntry` gains a public `codec_type` field; a struct literal must
  name it (`av01`).

- `scheme::RestrictedVideoSampleEntry` (`resv`) and
  `RestrictedAudioSampleEntry` (`resa`), and `subtitle_entries::
  TextSubtitleSampleEntry` (`sbtt`, §12.6.3): the restricted sample entries
  keep the base class and expose `restricted_scheme()`; `sbtt` carries its two
  null-terminated MIME strings.
- `transform::TransformSampleEntry` and `IncompleteAudioSampleEntry`: the
  protected (`encf`/`encm`/`encp`/`encs`/`enc3`) and incomplete
  (`icps`/`icpp`/`icpm`/`icp3`/`icpv`/`icpa`) entries, each keeping the base
  class's fixed bytes and the appended `sinf`/`cinf` verbatim, with typed
  accessors for the transform box. `TextMetaDataSampleEntry` accepts `enct`
  and `icpt`, and `XmlSubtitleSampleEntry` records its four-CC so it can carry
  `encu`.
- `hint::RtpoOffsetBox` (`rtpo`), `hint::RolloverCounterBox` (`sroc`) and
  `hint::ObjectDescriptorBox` (`tOD `), the last reusing the `tPAT`/`tPMT`
  section-box definition and joining the MPEG-2 TS entry's typed children.

### Breaking

- `subtitle_entries::XmlSubtitleSampleEntry` gains a public `codec_type`
  field: the entry's four-CC is now carried instead of being rewritten as
  `stpp`, so a protected `encu` entry round-trips. A struct literal must name
  it (or start from `new`).

- `subtitle_entries::XmlSubtitleSampleEntry` gains a public `trailing` field:
  the bytes after its last complete child box — a header cut short by the
  entry's own size, or a region too small to frame one — are carried instead of
  being dropped on a rewrite. A struct literal must name it (or start from
  `new`).

- `threegpp`: the 3GPP TS 26.244 code points — `ThreeGppAudioEntry`
  (`samr`/`sawb`/`sawp`/`sevs`, with `AmrDecoderSpecificBox` and
  `AmrwbPlusDecoderSpecificBox`), `H263SampleEntry` and
  `H263DecoderSpecificBox` (`s263`/`d263`), `CvoSampleEntry` (`3gvo`),
  `ThreeGppSimpleEntry` (`3glo`/`3gor`), the fragment adjustment boxes
  `tfad`/`tfma`, `ServiceProtectionDescriptionBox` (`spdb`),
  `ApplicationDataUnitInfoBox` (`3gau`), and the user-data boxes
  `clsf`/`coll`/`kywd`/`rtng`/`urat`/`thmb`/`orie` (the last with the 1-bit
  `Pan_indication` packed as the table states). All are reachable from
  `sample_entry_variant` and `metadata::UdtaChild`.

- `quicktime::QuickTimeVideoEntry`: every registered QuickTime video sample
  description (`apco`/`apcs`/`apcn`/`apch`/`ap4h`/`ap4x`, `jpeg`, `SVQ1`,
  `h263`, `png`, …) as one type over the standard 78-byte video description,
  dispatched from `stsd`, with child atoms kept verbatim; the QuickTime sound
  entries `QDM2`/`QDMC`/`Qclp`/`agsm`/`vdva` join `SoundEntry`'s dispatch.
- `subtitle_entries::C708` and `ClosedCaptionSampleEntry::codec_type`/
  `with_codec_type`: the CEA-708 closed-caption video sample description, the
  same structure as `c608` (QuickTime File Format, "Closed captioning sample
  description").
- `visual_ext::ColourType::Full`: the QuickTime `full` colour type, whose body
  is the `nclx` parameter block with the full-range flag set.
- `quicktime::PreloadBox` (QuickTime track loading settings, `load`), surfaced
  as `reader::TrackInfo::preload`.
- `compressed::CompressedBox`: the four compressed top-level boxes `!mov`,
  `!mof`, `!six` and `!ssx` (ISO/IEC 14496-12:2026 §8.16), one definition over
  the replacement-type map, with the payload and header form kept verbatim.
- `segments::TrackTypeBox` (`ttyp`, §8.3.5): the track brand list with
  `FileTypeBox` payload syntax.

- `metadata::Itunsmpb` and `Metadata::itunsmpb`: the iTunes `iTunSMPB`
  freeform item's priming, padding and original sample count, parsed from its
  hex text (a de-facto format, cited in the module). The typed view borrows
  nothing; the item itself still round-trips byte for byte.
- `sample_groups::GROUPING_TYPE_PROL` and `SgpdEntry::Prol`: the audio
  pre-roll sample group description (§10.1), with `SgpdEntry::pre_roll_samples`
  reporting a negative `roll` or positive `prol` distance as the decoder
  pre-roll it states.
- `reader::TrackInfo::pre_roll`: the track's decoder pre-roll requirement in
  samples, from its `roll`/`prol` sample group (§10.1); and
  `reader::TrackInfo::itunsmpb`: the file's iTunes gapless facts, a per-track
  item taking precedence over a movie-level one that applies only when the
  file has one audio track.

### Breaking

- **Every reserved or fixed field the box audit reached is now carried.** The
  movie, track and media headers (`MovieHeaderBox` `reserved`/`pre_defined`,
  `TrackHeaderBox` `reserved`/`reserved_2`/`reserved_3`, `MediaHeaderBox`
  `quality`, `SoundMediaHeaderBox` `reserved`), `SegmentIndexBox`
  `reserved`, `TrackFragmentRandomAccessBox` `reserved_widths`,
  `MovieFragmentHeaderBox` `version`/`flags`, `TrackFragmentHeaderBox`
  `version`, `MediaDataBox` `largesize`/`to_end`, `Av1ConfigurationBox`
  `reserved`, `ImageRotation`/`ImageMirror` reserved bits, and the item entry's
  final-string terminator hold what the file said instead of being rewritten.
  A struct literal must state the new fields (`Default::default()` is the
  spec-fill). Version bytes the headers used to normalise to 0/1 are carried
  too.

- **One opaque-child carrier.** `isobmff::sample_entries::OpaqueBox` is gone:
  the sample entries' `extra_boxes` are `box_support::OpaqueBox` like every
  other container's, so a child written with `size == 0` or `size == 1` +
  `largesize` keeps that form instead of being normalised to the compact one.

- **`subs` version 2 and `ssrt` carry every bit of their wire state.**
  `SubsampleProperties` gains a `reserved` field for the two reserved bits of a
  version-2 `subs` property byte (three in an `ssrt` entry),
  `SubSampleInformationBoxV2` gains `reserved` for the six after
  `is_short_ref`, and `SubSampleReferenceTableBox` gains `version`, which was
  hard-coded to 0 on write. A struct literal must state them (parsed values
  carry them already).

- **A structural box carries the bytes its declared table does not account
  for.** `DataReferenceBox`, `SampleDescriptionBox`, `SampleSizeBox`,
  `SampleAuxInfoSizesBox`, `SegmentIndexBox`, `TrackFragmentRandomAccessBox`,
  `ItemPropertyAssociationBox`, `TrackFragmentHeaderBox`,
  `TrackFragmentRunBox` and `ChannelLayoutBox` gain a private `trailing`
  field: bytes after the box's last declared field are kept verbatim and
  written back, so the round trip is exact and the reader still gets the
  table. A declared count or size that runs past the box is still a typed
  error, as is a FullBox version the crate does not model. Build one through
  its `new` (and `with_trailing`), and read the carried bytes through
  `trailing()`; a struct literal can no longer name every field.

- **Sample entries keep every parsed byte on a rewrite.** The audio sample
  entries (`Mp4aSampleEntry`, `Ac3SampleEntry`, `Ec3SampleEntry`,
  `Ac4SampleEntry`, `OpusSampleEntry`, `FlacSampleEntry`, `MhaSampleEntry`,
  `DtsSampleEntry`) and the `stpp`/`wvtt` subtitle entries gain a
  `reserved: [u8; 6]` field carrying the `SampleEntry` `reserved[6]` block
  instead of rewriting it as zeros. `SoundEntry`, the audio entries and
  `TimecodeEntry` gain a `trailing: Vec<u8>` field: a child whose declared size
  runs past the entry is kept verbatim there rather than being shortened and
  re-emitted with the clamped length. A sample entry written with `size == 0`
  stays opaque, and a stub shorter than its declared size is a typed error.
  Construct a new entry with the type's `new`/builder (or set `reserved` to
  zeros and `trailing` to empty), not a struct literal. `UdtaText` gains a
  `plain_nul_terminated` field: a plain `udta` text atom (one without the `©`
  prefix) is a NUL-terminated string, and a source that omitted the NUL no
  longer gains one on a rewrite.

- **The HEIF 2025 boxes carry their whole wire state.** `CodingConstraintsBox`
  carries the 26 `reserved` bits after `max_ref_per_pic`, each
  `RegionAnnotationAssociation` the five bits before its
  `annotation_container_type`, each `ColourFormatInput` the seven bits before
  its `is_packed_flag`, `StereoAggressor` the `sub_type_present` bit (a
  signalled-but-empty URI no longer loses its bit and terminator), and
  `RegionSampleEntry` its `reserved` block, sibling children and any child
  bytes a walk cannot represent, so an `rgan` entry no longer rewrites its
  reserved block or re-frames a truncated tail. `RegionSampleEntry::new` is a
  plain `fn`. An `md5i` stating a non-zero `version` no longer parses, and a
  clause-6.5 property whose FullBox `version`/`flags` the clause fixes at zero,
  or whose body runs past its last field, is a typed error instead of being
  rewritten without them. `ColourFormatEnhancement::serialized_len` reports the
  byte its `serialize_into` writes.

### Added

- **Both readers degrade a box they cannot reproduce instead of refusing the
  file.** `Reader` keeps a non-structural box (`edts`/`elst`, `trgr`, `elng`,
  a media header, `gmhd`, a `udta`/`meta` atom) opaque in its own position,
  byte-exact, and reports it as `Warning::UnrepresentableBox`; bytes a
  structural box carries past its declared fields are reported as
  `Warning::TrailingBytes`. The `Reader` docs table states the split. The item
  reader does the same for item properties, entity groups, item references and
  the derived/region/integrity item bodies, reporting
  `ImageFinding::UnrepresentableBox`, and reports carried bytes as
  `ImageFinding::TrailingBytes`. A second file-level `meta` box is carried and
  reported as `ImageFinding::ExtraMetaBox`. `ItemPropertyContainerBox` gains
  `parse_body_degrading`, `ItemPropertiesBox`, `ItemReferenceBox` and
  `GroupsListBox` gain `parse_degrading`, and `item::DegradedChild` names what
  was kept. Only structure a reader cannot walk (the sample tables, the item
  location and information tables, the property association) remains an
  error.

### Fixed

- **The item reader carries the bytes it cannot frame instead of dropping
  them.** A top-level box whose size (or `largesize`) cannot hold its own
  header, a header the input cuts, and a `meta` child the input cuts are kept
  verbatim in their source position with an `ImageFinding::TrailingBytes`
  naming the box, offset and length, so `StillImage::reemit_into` reproduces
  the input byte for byte; `Mode::Strict` still refuses each with a typed
  error (`Error::BoxSizeUnderflow` for the size). A `largesize` of 0 used to
  be kept as a zero-length payload and drop the bytes it claimed; the
  `isobmff_item_reader` target's round-trip assertion found it.
- **A `sidx`, `ssix`, `mfra` or `ftyp` that cannot be parsed no longer fails
  a best-effort read.** None is needed to play the samples:
  the box is reported as `UnrepresentableBox` at its own byte range and the
  walk continues; a seek that needed the `mfra` is `Error::NoIndex`.

- **A second file-level `meta` box is carried, not refused.** The item reader
  uses the first; a later one keeps its file position and re-emits byte-exact,
  reported as `ImageFinding::ExtraMetaBox`. It used to be a typed error,
  because only one body was kept. The whole-file round-trip assertion the
  `isobmff_item_reader` fuzz target makes found the original rewrite; the
  345-byte input stays as a regression test.

- **A fixed-layout box refuses bytes its declared fields do not account
  for.** `btrt`, `tims`, `tsro` and `snro` wrote back only the fields their
  clause lists, so bytes past them were dropped on the way out. Each is now a
  typed error, and the reader that meets one keeps it opaque. The box audit's
  new trailing-byte pass appends four bytes past every occurrence's last
  declared field and requires a carry or a refusal; the boxes that still do
  neither are pinned in `tests/it/box_audit.rs`.

- **A box whose declared table leaves bytes over keeps them.** `dref`, `stsd`,
  `stsz`, `saiz`, `sidx`, `tfra`, `ipma`, `tfhd`, `trun` and `chnl` used to
  drop those bytes on the way out (and a sample-info table a non-zero default
  rules out was silently ignored); they are now carried verbatim. `chnl` still
  refuses a FullBox version above 1 and a tail shorter than a byte. `cprt`
  keeps its language field unmasked. The new `tests/it/box_audit.rs` walks
  every committed media fixture, flips each byte of each typed box's fixed
  part, and requires parse -> serialize identity or a typed error; it found
  and pinned these.

- **A `meta` child keeps the header form it was written with.** A child using
  `size == 0` or `size == 1` + `largesize` was re-emitted with the compact
  length, changing bytes the reader had accepted;
  `isobmff_item_reader` found it on a 93-byte input, kept as a regression
  test.

- **A visual sample entry's opaque child keeps its header form.** A child of
  an `avc1`/`hvc1`/`vvc1` entry written with `size == 0` or `size == 1` +
  `largesize` was re-emitted with the compact length.

- **`subs` version 2 and `ssrt` keep their reserved bits, and `ssrt` its
  FullBox version.** The reserved bits were dropped and `ssrt` was always
  written as version 0, so a rewrite changed bytes it had parsed.

- **`sgpd` writes the `default_length` the file stated.** The serializer
  re-derived it from the entries: a version-1 box whose `default_length` was 0
  with uniform entries came back in the shared-length form, and a non-zero
  value with no entries came back as 0. `default_length` is now carried, 0
  still means a per-entry `description_length` prefix, and a shared value that
  disagrees with an entry's width is a typed error. A box you build must state
  `default_length` itself instead of leaving it 0 for the serializer to fill
  in.

- **A `udta`/`meta`/`ilst` child written with `size == 0` or `size == 1` +
  `largesize` keeps that header form.** A typed child serialises the compact
  header, so such a child now stays opaque and is re-emitted verbatim.

- **A sample-entry child box is never shortened and re-emitted.** One
  `box_support::capture_children` walk serves the sample entries and the
  QuickTime `tapt`/`gmhd`/`wave` atoms: a child that runs past its region is a
  typed error, and the codec-config search no longer returns a box clamped to
  the buffer end (which a rewrite emitted at a length the file never stated).
  `ColourFormatEnhancement` also refuses a body with bytes past its last input.

### Added

- **The MP4 registration tables are measured, and their held codes typed
  (issue #4167).** `coverage::mp4ra` pins every code in all 30 MP4RA tables as
  typed, held-but-untyped, not-held or not-applicable, and
  `tests/it/mp4ra.rs` proves the partition against the CSVs and the README
  counts. The registered identifier codes gain typed enums in `registered`
  (`Brand`, `HandlerType`, `SampleGroupType`, `TrackReferenceType`,
  `TrackGroupType`, `TrackSelectionType`, `KeyNamespace`,
  `RestrictedSchemeType`), each mapping a four-CC to a named variant and back,
  with accessors on `HandlerBox`, `TrackReferenceTypeBox`, `TrackGroupTypeBox`,
  `TrackSelectionBox`, `KeyEntry`, `SampleToGroupBox`,
  `SampleGroupDescriptionBox`, `SchemeTypeBox`, `FileTypeBox` and
  `SegmentTypeBox`. New typed boxes: `sthd` (`SubtitleMediaHeaderBox`, now
  reported by `MediaInformationBox::media_header` as
  `MediaInformationHeader::Subtitle`), `wide` (`WideAtom`), `tran`
  (`TranscodingInfoBox`/`AvcTranscodingInfo`) and the `c608` sample entry
  (`ClosedCaptionSampleEntry`, dispatched as
  `SampleEntryVariant::ClosedCaption`). `ItemType` gains `Auvd`, `Evc1`,
  `Hvt1`, `Lhv1`, `Jpeg`, `Sato` and `Vvc1`; `ItemProperty` gains `Stvi` and
  `VvcC`; `EntityGroupType` gains `Opeg`. 604 of the 1566 registered codes are
  typed; the README lists the remaining held and not-held counts per table and
  the specifications not held, in unlock order.

- **Every ISO/IEC 14496-12:2026 box type is typed.** The hint-track family
  joins the crate: `hnti`/`rtp `/`sdp `, the `hinf` counters, the sample-entry
  children `tims`/`tsro`/`snro`/`tssy`/`rssr`/`srpp`/`rtpx`/`rcsr`, the MPEG-2
  TS entry metadata `tPAT`/`tPMT`/`tsti`/`istm`, the RTP and MPEG-2 TS hint
  sample entries, and the RTP sample format. The FLUTE file-delivery family
  follows: `fiin`/`paen`/`fpar`/`fecr`/`fire`/`segr`/`gitn`, the FD hint sample
  entry, `fdsa`/`fdpa`/`extr`/`feci`, the FD sample format, and the `fdel` item
  information extension. `btrt`, `stsh` and `rsot` complete the set;
  `SampleEntryInfo` gains `hint` and `bit_rate`, `TrackInfo` gains
  `hint_information` and `hint_statistics`, and `Metadata` gains
  `fd_item_information`. The coverage test now reports 167 of 167 registry box
  types.

- **Gain map metadata typed (Amd 1 6.6.2.4.2).** `image` gains
  `GainMapMetadata` with its `GainMapChannel`, `SignedRational` and
  `UnsignedRational` values and the `GainMapField` names, read and written
  byte-exactly. The layout is the one libavif, CrabbyAvif, Skia and libultrahdr
  agree on; a body with a `minimum_version` above 0, a reserved flag bit set or
  trailing bytes stays the opaque payload it was and round-trips.
  `ToneMapImage::with_metadata` builds a `tmap` body from the typed value, and
  a zero denominator is reported as `ImageFinding::GainMapZeroDenominator`
  rather than refusing the read. Three real fixtures written by
  `avifgainmaputil` 1.4.2 (multichannel, single-channel and swapped base)
  cover it.

- **Image-sequence and region sample group entries typed (Amd 1).** `sgpd`
  now dispatches `eqiv`, the five bracketing entries (`aebr`, `wbbr`, `fobr`,
  `afbr`, `dobr`), `pano`, and the `refs`, `regm` and `udes` entries to typed
  `SgpdEntry` variants that read and write byte-exactly; the `rgan`
  region-track sample entry joins the movie reader's typed sample-entry
  dispatch as `SampleEntryVariant::Rgan`.

- **Still images brought to the current HEIF edition (issue #4167).** The item
  layer now reads and writes ISO/IEC 23008-12:2025 (third edition) with
  Amendment 1 applied, not only the 2017 subset. New modules `image_props`
  (every clause 6.5 property added since 2017: `iscl`, `rref`, `crtt`, `mdft`,
  `udes`, `altt`, the capture-information set `aebr`/`wbbr`/`fobr`/`afbr`/
  `dobr`/`pano`, `tols`, `wipe`/`zoom`/`fade`/`splt`, `stpe`, `ssld`, `prdi`,
  `sstr`, `cmex`, `cmin`, and Amendment 1's `cexg`, `dadj`, `stag`, `reve`,
  `ndwt` and the `mskC` mask configuration), `image_derived` (`tmap` tone-map
  derivation with its ISO 21496-1 metadata typed, and `cfen` colour format
  enhancement), `image_groups` (typed `pymd` tile data and `rgpa` areas),
  `image_brands` (clause 10 typed brands, reader-property union and
  structurally checkable findings), `image_sequence` (`ccst`, `auxi`, the
  `refs` sample group), `region` (`RegionGeometry` for every geometry at both
  field widths, `RegionItem`, region-track configuration/sample, the `regm`
  and `udes` sample groups) and `integrity` (`md5i` and the `mint` item body).
  `ItemProperty` gains all of them plus the reused `subs`; `DerivedImage` gains
  `ToneMap`/`ColourFormatEnhancement`; `ItemType` gains `Tmap`, `Cfen`, `Mski`
  and `Rgan`; `ReferenceType` gains `pred`, `prem`, `dpnd`, `mask`, `eroi`,
  `drgn`, `rpds` and `anot`; `EntityGroupType` gains the clause 6.8 set and
  `ImageGroupBody` types the `pymd`/`rgpa` bodies. `StillImage` reports typed
  brand and structure findings and exposes a tone map's base/gain-map pair and
  metadata, a stereo pair, a group's entities, an image's region items and an
  item's capture information; `Item` carries typed region and integrity bodies;
  `ItemWriter` can add derived, region and integrity items whole;
  `TrackInfo::is_image_sequence`, `coding_constraints` and
  `auxiliary_type_info` surface clause 7. The per-clause coverage table, the
  2017-vs-2025 re-check and the OPEN list live in the private repo at
  `private/specs-verified/mpeg/heif/isobmff-coverage-2025.md`.

- Four new still-image fixtures (`heif/props.heic`, `heif/tiled.heic`,
  `heif/premult.heic`, `heif/groups.mp4`) written by `heif-enc`/`MP4Box` and a
  twelfth example, `heif_2025`.

### Breaking

- **`StillImage::tone_map_inputs` returns `ToneMapInputs`**: the base and gain
  map items plus the typed gain map metadata when the body carries the modelled
  layout. Callers that destructured the old pair use `ToneMapInputs::base` and
  `ToneMapInputs::gain_map`.

### Fixed

- An `md5i` integrity box whose body ends right after an `sgpd` `input_4cc`
  indexed past the body and panicked; the missing grouping type is now the
  typed buffer error it should be. Found by the `isobmff_item_boxes` fuzz
  target, with its artifact as the regression case.

- The still-image reader resolved an item stored in `idat` (`construction_method`
  1) to the wrong absolute offset whenever `idat` was not the `meta` box's first
  child. The offset now accumulates the bytes consumed by preceding children,
  so a real tiled HEIC's `grid` body is read from its own `idat` range.

### Added

- **Tier 3 and tier 4 box types: metadata tracks, file-level extras, schemes
  and the newer audio and colour signalling (issue #4167).** New modules
  `meta_track` (ISO/IEC 14496-12:2026 §12.3.3, §12.5.3, §12.8.4, §8.10.2,
  §8.11.2: `uri `/`uriI`/`txtC`/`mime`, `xml `/`bxml`, `cprt`, the metadata key
  table's `keyd`/`loca`/`setu`, and the `metx`/`mett`/`stxt`/`urim`/`mebx`
  sample entries), `scheme` (§13.4.8, §13.5.3, §13.5.5, §13.5.6.2, §13.6.3:
  `rinf`, `csch`, `spki`, `scrb`, `cinf`) and `audio_elements` (§12.2.8–14:
  `ardi` with its `AudioRenderingIndication` label enum, `aelm`, `aedb`,
  `aepp`, `aepr`, `aesb`, `aesd`). `segments` gains `imda`, `tyco`, `etyp` and
  `otyp`, `init_segment` gains `nmhd`, `hmhd`, `vvhd` and `mehd`, `track_desc`
  gains `strk`/`stri`/`strd`, `visual_ext` gains `amve`/`cclv`, `index_boxes`
  gains `assp` (typed inside `trep`), and `scrb`, `amve` and `cclv` are typed
  inside an `ipco`. The reader reports a metadata entry's URI and init data
  (`SampleEntryInfo::timed_metadata`), a track's media header
  (`TrackInfo::media_header`), its sub-tracks (`TrackInfo::subtracks`) and its
  restricted/scrambled/incomplete scheme boxes, and the file's extended types,
  original file type, identified media data and `mehd` duration. A sample
  addressed through an `imdt` data reference is resolved against the `imda`
  payload it names. Update: this takes the crate's coverage from 88 to 122 of
  the 167 ISO/IEC 14496-12:2026 registry box types.

- **The item layer: HEIF and AVIF still images read and written (issue #4167
  tier 2).** New modules `item` (ISO/IEC 14496-12:2026 §8.11/§8.15: `iloc`,
  `iinf`/`infe`, `pitm`, `iref` with its typed reference boxes,
  `iprp`/`ipco`/`ipma`, `idat`, `ipro`, `grpl`/`EntityToGroupBox`, `brnd`,
  `hdlp`), `image` (ISO/IEC 23008-12:2017 clause 6 with its 2020 corrigendum:
  `ispe`, `pixi`, `rloc`, `auxC`, `irot`, `lsel`, `imir`, the `grid`/`iovl`/
  `iden` derived bodies, the HEVC/AVC configuration properties, and AVIF
  v1.2.0's `a1op`/`a1lx` and brands), `item_reader` (a sans-IO reader yielding
  brands, the primary item, every item with its properties, references and
  resolved data ranges, entity groups, image size/rotation/mirroring,
  thumbnails, auxiliary images, grid tiles/canvas, overlay layers, Exif/XMP
  ranges, and a byte-exact re-emit plan) and `item_writer` (builds a file from
  typed parts, choosing the `iloc` widths and offsets). A property whose
  syntax another clause already defines reuses that type; coded item data is
  opaque and addressed by byte ranges. New typed registry boxes take the
  crate's coverage to 88 of 167 ISO/IEC 14496-12:2026 types. New types
  `ItemId`, `PropertyIndex`, `GroupId`, `ItemType`, `ReferenceType`,
  `EntityGroupType`, `ItemStep`, `StillImage`, `ReemitOp` and `ItemWriter`.
  `Limits` gains caller-set item/property/reference count caps that default to
  unlimited.


- **Tier 1 box types and a tested coverage table (issue #4167).** New typed,
  private-field boxes with builders, accessors and symmetric parse/serialize:
  audio signalling `chnl`/`dmix`/`ludt` (`tlou`, `alou`)/`labl`; track
  description `elng`/`kind`/`trgr` (`msrc`, `ster`)/`tsel`/`stvi`; indexing
  `ssix`/`leva`/`pdin`/`trep`; sample tables `csgp`/`stdp`/`padb`/`stsg`.
  `elng`, `trgr`, `leva` and `trep` are typed in their containers, `kind`,
  `labl`, `ludt` and `tsel` in a track's `udta`, `stvi` in `schi`, and
  `csgp`/`stdp`/`padb` in `stbl`. `TrackInfo` now exposes the extended
  language, track groups, kinds, labels, loudness and track selection;
  `SampleEntryInfo` exposes an audio entry's `chnl`/`dmix`; `FragmentInfo`
  exposes the `ssix` and `leva` beside the `sidx`. `TrackBox::sample_group_mappings`
  reads both `sbgp` and `csgp` through the one `SampleGroupMapping` lookup.
  `coverage::TYPED_BOXES` is the crate's typed registry set (76 of 167
  ISO/IEC 14496-12:2026 box types), each row proving itself against a minimal
  box, and `tests/it/coverage.rs` pins every remaining type under its issue
  tier so the README count cannot drift.

- `sample_table::CursorState::new_with_aux` and `TrackCursor::new_with_aux`
  surface a progressive track's `stbl`-level `senc` on every `SampleRef::aux`,
  one entry per declared sample, so a protected progressive file reads exactly
  like a fragmented one. `cenc::SampleEncryptionBox::parse_box` parses a whole
  `senc` box, `cenc::clear_sample_entry` reconstructs the cleartext
  `frma`-format sample entry from an `encv`/`enca` entry, and
  `init_segment::sample_entry_variant` is public so a demuxer can reparse that
  cleartext entry.

### Breaking

- `nalu_types::HevcNalArray` gained a `reserved` field: real files (libheif)
  set the bit the `hvcC` array header declares reserved, and dropping it made
  a HEIF item property fail the byte-exact round trip. `HevcNalArray::new`
  keeps writing 0; a parsed array keeps the bit it read.

### Changed

- The crate depends on `broadcast-core` only. The big-endian field readers it
  used from `broadcast-common` now live in `broadcast_core::wire`, so
  `broadcast-common` is a dev-dependency (tests and two examples) rather than
  a normal one, and an outside project adopting `isobmff` takes the frozen
  contract and nothing else of the workspace.

### Fixed

- **A truncated `aepp` coordinate triple is an error, not a panic (#4179).**
  The audio element positioning box read its three azimuth values with an
  unchecked reader, so a body that set the azimuth-present flag without
  carrying the values panicked out of the parser (the `isobmff_tier34_boxes`
  fuzz target found it in ten seconds). The parser now asks for the whole
  triple first and returns the typed short-buffer error;
  `tests/it/fuzz_regressions.rs` feeds the artifact's own bytes back through
  the same entry point.

- **A rewritten fragmented init keeps its empty `stsz`'s constant
  `sample_size` (#4148).** A source `stsz` with `sample_count == 0` and a
  non-zero constant `sample_size` (value 1 in a real MPEH-G init segment) was
  rebuilt with `sample_size == 0`, so the init segment did not round-trip. The
  fragmented writer now carries the source's constant size into the empty
  `stsz` it rebuilds.

- **The reader keeps every `moof` queued ahead of its `mdat` (r05-W26).** A
  fragment written the CMF2 way (one `moof` per track, then one `mdat` holding
  both tracks' data, §8.8.4) used to overwrite the previous pending `moof`, so
  the first fragment's samples were silently dropped. The reader now holds the
  queued `moof`s, counts their bytes against `Limits::max_fragment_bytes`
  (exceeding it is the existing `LimitExceeded`), and resolves and emits each
  one in file order once the following `mdat`(s) arrive, on both a seekable and
  a forward-only source. `tests/it/reader_fragmented.rs` rewrites a real
  two-track fragment into that shape and reads every sample back through
  `FileReader` and `ForwardReader`.

### Breaking

- The wire-format contract types now come from `broadcast-core`.

- `registry::BoxDef` and `registry::RegisteredBox` require `Send + Sync`, so a `Reader` holding registered values can cross threads (a tokio task) and one registry can serve many readers.
- `writer::FragmentOptions` gains `layout` and `position`, `writer::FragmentTrack` gains `trex`, and `writer::Fragment` gains `moof_offset`.
- `writer::FragmentedWriter::trailer` appends the regenerated `mfra` when the index layout asks for one.
### Added

- `writer::ProgressiveWriter::plan` exposes the `OwnedPlan` a progressive
  writer was built over, so a muxer feeding it can name a size/index mismatch
  with the plan's own values instead of the writer's box-level error.
- `frag_layout`: each movie fragment's write-back layout. `reader::FragmentInfo`
  now carries the `FragmentLayout` the reader recorded for it (which `tfhd`
  optional fields are present and their values, `default-base-is-moof`, the
  `trun` field choices, versions and split points, the `tfdt` version, the
  `traf`/`moof` child order including opaque boxes, and the top-level boxes
  between fragments), and `Reader::fragment_index` the forms of the `sidx`
  before the fragments and the `mfra` after them.
- `writer::FragmentedWriter::set_fragment_layout` writes a fragment exactly as
  a recorded layout states it, recomputing `tfhd.base_data_offset` and every
  `trun.data_offset` from where the fragment is placed and refusing a
  `tfhd`/`trun` default the samples contradict with
  `Error::LayoutMismatch`. `set_position` states where the emitted bytes land.
  Without a layout the output is unchanged.
- `set_compact_layout` writes the compact form
  `FragmentLayout::compact` derives: `tfhd` sample defaults where a track's
  samples agree and `first_sample_flags` where only the first differs.
- `with_sidx`/`with_mfra` emit a segment index and a trailing random-access
  index regenerated from the offsets the writer placed its own fragments at
  (`segment_index`, `trailer`), in the form a source recorded. Neither index is
  ever copied: carried offsets would be stale.
- `SampleEntryInfo::error` carries the sample-entry parser's own message when a
  known four-CC's `stsd` body fails to parse; `typed` is recovered by
  re-parsing the raw entry, so a demuxer can report the real cause (issue #952).

- `registry`: `BoxRegistry` registers a private box or sample entry from outside
  the crate. `BoxDef` names its four-CC (or `uuid` extended type) and an
  optional parent scope; `register` refuses a key this crate already types, or
  one already registered, unless `register_override` is used. A matching `stsd`
  entry comes back typed in `SampleEntryInfo::registered`,
  `BoxRegistry::parse_children` types the children of an opaque box, and
  `SampleEntry::from_registered` gives the writer the entry bytes it carries.
  `ReaderConfig::with_registry` attaches one; an unregistered box stays an
  `OpaqueBox` and writes back unchanged.
- A whole-file byte-identical round trip: a plan read from a file now carries
  the parsed `trak` shells, the `moov`'s extra children (`udta`/`meta`,
  `iods`, `mvex`), each `stbl` child a plan does not rebuild (`sgpd`/`sbgp`/
  `sdtp`) in its wire position, and the top-level boxes outside
  `ftyp`/`moov`/`mdat` (`free`, `wide`, `uuid`, `skip`) with their positions.
  `MovieShell`, `TopLevelBox` and `BoxPosition` are the plan-side types;
  `TrackSetup::shell` carries the per-track half, `FragmentedWriter::
  set_movie_shell`/`trailer` the fragmented one. A `PlanBuilder` plan carries
  none and writes exactly the previous layout, so every golden is unchanged.
- `HandlerBox::pre_defined` and `HandlerBox::reserved` are parsed and written
  back: a QuickTime file keeps its component type (`mhlr`/`dhlr`) and
  manufacturer code there, so a `.mov` round-trips byte for byte.
- QuickTime atoms are written as well as read: `SoundEntry`, `WaveAtom`,
  `GmhdAtom`/`GminAtom`/`TcmiAtom`, `TimecodeEntry` and
  `TrackApertureModeDimensionsBox`/`ApertureDimensions` implement `Serialize`
  as the exact inverse of their parse, including the `tmcd` wrapper, child
  order, the sample-entry reserved bytes and the `tcmi` reserved field.
  `MediaInformationBox` carries a typed `gmhd`, `TrackSetup` gains `gmhd` and
  `references` (`tref`), and `FileType::quicktime`/`without_ftyp` select a
  QuickTime-brand file or no `ftyp` at all.
- `VisualSampleEntryFields` keeps the SampleEntry `reserved[6]`, the
  QuickTime `revision_level`/`vendor`/quality block and any trailing pad, and
  `AVCSampleEntry`/`HEVCSampleEntry`/`VVCSampleEntry` keep the position of
  their config box among the extra boxes, so a typed `.mov` track serialises
  back to the file's bytes.

### Breaking
- `Error::QuickTimeOnly` and `SampleEntryVariant::is_quicktime_only` are
  removed: every QuickTime sample entry this crate parses now serialises.
- `FileType` gains `emit_ftyp`; `TrackSetup` gains `gmhd` and `references`;
  `VisualSampleEntryFields` gains `reserved`, `pre_defined_and_reserved` and
  `trailing`; the AVC, HEVC and VVC sample entries gain `config_position`.
### Fixed
- `TrackId::next` saturates at `u32::MAX` instead of panicking on a crafted
  `track_ID` (fuzz finding on `isobmff_whole_file`).
- A `SampleEntry::Raw` whose bytes use the `size == 0` ("to end of container",
  §4.2) or `largesize` forms is written back in that form instead of being given
  an explicit 32-bit length (fuzz finding).
- A progressive rewrite keeps the `tkhd`/`mdhd` version the source stated
  instead of normalizing it to 0 (fuzz finding).
- A fragment run that uses a non-default `stsd` entry states it in
  `tfhd.sample_description_index`, so a reader sees the same
  `description_index` it started with (fuzz finding). `SampleMeta` gains
  `description_index` (default 1, which omits the field and keeps every
  existing golden byte-identical), and `media_fragment_run` takes it as the
  optional per-`traf` value; a run whose samples disagree is an error.
### Changed
- A sample plan's `stsd` entries are carried as their exact box bytes
  (`SampleEntry::Raw`) rather than re-serialized from the typed parse, so a
  `colr` ahead of `avcC`, or a QuickTime `vendor` field, survives a rewrite.
  The typed parse is unchanged on `TrackInfo`.
- The reader walks the boxes after the sample data for a seekable source, so
  a trailing `free`/`skip`/`uuid` box is carried; a forward-only source still
  finishes as soon as its samples are exhausted.
- A truncated final `mdat` is reported once, not once per fault site.
### Fixed

- **A `uuid` item reference and a `uuid` entity group are preserved with
  their extended type instead of failing.** Both keep their 16-byte usertype
  and round-trip byte-identically.

### Breaking

- `MediaInformationBox` gains `nmhd`, `hmhd` and `vvhd` fields, and
  `SchemeInformationBox` gains `spki`. A downstream struct literal of either
  must list the new field (`MediaInformationBox::new` and the parsed tree set
  it). `SampleEntryVariant` gains `Metx`, `Text`, `Urim` and `Mebx` (the enum
  is `#[non_exhaustive]`, so only a match needs a catch-all).
- `TrackInfo` gains `media_header` and `subtracks` (`#[non_exhaustive]`).

## [Unreleased]
### Added
- `quicktime`, QuickTime atoms a camera or editing `.mov` carries:
- `Reader::end_of_unit(unit_end)` for sequential input: it declares that the
  bytes supplied so far end one self-contained unit (a media segment). A
  top-level box whose declared end runs past the unit is dropped; in permissive
  mode the cut fragment's samples wholly inside the unit stay available and a
  `Warning::TruncatedFragment` follows them, while strict mode returns the
  truncation error. The `moov`, its track state, the `trex` defaults and the
  running decode time are kept, and the top-level walk resumes at `unit_end`,
  so a damaged segment cannot affect the next one.
- `quicktime`, read-only QuickTime atoms a camera or editing `.mov` carries:
  sound description versions 1 and 2 (including the 64-bit version 2 sample
  rate and the four version 1 fields), the `wave` decompression container and
  its `esds`, the base media information header `gmhd`/`gmin`/`tcmi`, the
  `tmcd` timecode sample entry with `timecode_from_sample`, track aperture
  dimensions (`tapt`), chapter-title samples, and a `qt  ` or absent `ftyp`.
  `Mp4aSampleEntry::wave`/`esds` reach an `esds` inside `wave` through the same
  accessor as a top-level one, `TrackInfo` carries `tapt`, `MediaInformationBox`
  exposes `gmhd`, and `Reader::brand` reports the `ftyp` box. (They are read
  and written; see the first `[Unreleased]` entry for the serialisers.)
- `Mode::Permissive` and `Step::Warning(Warning)`: the reader tolerates the
  faults streaming-first design §4.7 lists (a clamped `moov` child, an
  unparseable known sample entry, an `mp4a` `wave` with no `esds`, a zero
  `mvhd` timescale, a truncated final `mdat` or fragment, a malformed middle
  `moof`) and reports each as a typed warning. Strict mode keeps rejecting
  them, and both modes still fail every `Limits` violation.
- `DriverStep::Warning` from every driver, `Advance::Warning` in the shared
  step classifier, and `broadcast-client`'s HTTP-range driver passes it too.
- `reader::TrackInfo::from_trak(&TrackBox)`: the reader's per-track view built
  from an already-parsed `trak`, for a caller that holds the whole `moov`.
  `Reader::movie_timescale()` and `Reader::is_fragmented()` expose the movie
  facts a demuxer needs without re-parsing it, and `Warning::position()`
  returns the absolute offset a tolerated fault starts at.

## [Unreleased]
### Added

- `driver` (feature `std`): `FileReader<R: Read + Seek>` and
  `ForwardReader<R: Read>` answer the sans-IO `Reader`'s structure demands
  themselves and yield `DriverStep::{Tracks, Fragment, Sample, Done}`.
  `read_sample` fetches one sample's bytes under `Limits::max_sample_bytes`;
  the forward-only form retains one header's tail so a `largesize`/`uuid`
  re-read needs no backward seek, and reports a sample behind its position as
  `DriverError::SampleBehindPosition`. `Error::MoovAfterMdatForwardOnly` is
  unchanged and reaches `DriverError::Format`.
- `examples/list_samples`: print every track of an MP4 and its sample count
  through `FileReader`.
- `metadata`, typed MP4 metadata in both directions: `udta` user-data text
  (QuickTime international text and the 3GPP null-terminated form), `©xyz` as
  an `Iso6709` with `latitude()`/`longitude()`/`altitude()`, the 3GPP `loci`
  box, Nero `chpl` chapter lists, and `meta` with `hdlr`, `keys`, `ilst`
  (both the iTunes four-character items and Apple's `mdta` keyed items),
  `data`/`mean`/`name` and `----` freeform items. Every unknown atom, data
  type, `free` pad and item order round-trips byte for byte as an
  `OpaqueBox`. `Metadata`/`MetadataItem` is the logical view and
  `Metadata::to_udta`/`to_meta` write it back; `MovieMetadata` carries the
  movie's and each track's items.
- `MovieBox::metadata`/`TrackBox::metadata` parse the preserved `udta`/`meta`
  children under the caller's `Limits` without changing the typed tree, so
  every existing init-segment golden stays byte-identical.
- `Step::Metadata(&MovieMetadata)` from `reader::Reader`, emitted once after
  `Step::Tracks` when the movie or a track carries `udta` or `meta`.
- `DriverStep::Metadata(&MovieMetadata)` from every driver: `FileReader` and
  `ForwardReader` (and, above this crate, `isobmff-tokio`'s async driver and
  `broadcast-client`'s range driver) store the owned value and return a borrow,
  so a driver user sees the tags instead of the step being swallowed.
  `read_tags` and `write_tags` now drive the file through `FileReader`.
- `metadata::ItemKey`'s named keys: `TITLE`, `ARTIST`, `ALBUM`, `DATE`,
  `GENRE`, `COMMENT`, `ENCODER`, `COMPOSER`, `TRACK_NUMBER`, `DISC_NUMBER`,
  `COVER_ART`, and the Apple keyed names `APPLE_MAKE`, `APPLE_MODEL`,
  `APPLE_SOFTWARE`, `APPLE_CREATION_DATE`, `APPLE_LOCATION_ISO6709` (the keyed
  ones are `&'static str`, since a keyed `ItemKey` owns its `String`).
  `ItemKey` prints as text: `Display`/`Debug` write `©nam` for the atom and the
  reverse-DNS string for a keyed item, so a tag is never spelled as byte values.
- `Metadata`'s readers: `get`, `title`, `artist`, `album`, `date`, `genre`,
  `comment`, `encoder`, `track`, `disc`, `cover_art` (a `CoverArt` carrying the
  `data` type), `location` (a `©xyz` item or the keyed
  `com.apple.quicktime.location.ISO6709` text), and `chapters`. The setters
  `set_text`, `set_track`, `set_disc` and `remove` replace or append one item
  and leave every other item and the item order untouched.
- `read_tags` and `write_tags` examples.

- `writer::Writer::progressive`, the sans-IO progressive (single-file) writer:
  it fixes the whole layout from an `OwnedPlan` before a sample byte arrives
  (interleaved chunks, `stco` or `co64`, the `mdat` header form and every
  sample's absolute offset), then takes each sample with `push` and emits
  `Write { offset, bytes }` values. A `Sink::Seekable` caller places them in
  any order and the writer holds no sample bytes; a `Sink::AppendOnly` caller
  gets file order enforced, with out-of-turn samples held in a buffer capped by
  its own `reorder_bytes` (`Error::ReorderBufferFull`). A pushed length that
  disagrees with the plan is `Error::PlanMismatch`, a missing one at `finish`
  is `Error::PlanIncomplete`, and `Interleave::AsPlanned` without a source
  chunk table is `Error::NoPlan`.
- `plan::OwnedPlan` and `plan::PlanBuilder`: a progressive file's sample tables
  in their compact wire form (no per-sample expansion), with the `stsd`
  entries, `elst` entries, durations and source chunk layout, plus a builder
  that run-length codes each table as samples are pushed.
  `SamplePlan::to_owned_plan` turns a reader's index into one.
- `sample_table::build_stsc_with_descriptions`, the `stsc` builder that keeps
  one `stsd` entry per chunk (a plan read from a file writes back the runs it
  was read with); `build_stsc` is the same builder with the first entry
  everywhere.
- `writer::TrackSetup::language` (`mdhd.language`), so a track read from a file
  keeps its language when it is written back; a handler with no media header of
  its own (`meta`, `text`, `tmcd`, …) now gets `nmhd` unless the caller named
  another.
- The `reader` walks fragmented files: after a `moov` with `mvex` it reads one
  `moof` at a time under `Limits::max_fragment_bytes`, applies the `trex`
  defaults, learns the following `mdat` payload ranges from their headers
  (never their bytes), and emits `Step::Fragment` followed by one
  `Step::Sample` per sample with its `tfdt`-based decode time, effective sync
  flag, `tfhd`/`trex` description index and, for a protected track, the
  `senc` IV and subsample map in `SampleRef::aux`. A fragment's samples are
  addressed by `frag_offsets` from the `moof` and those ranges, so no byte
  inside an `mdat` payload is requested.
- `Reader::seek_to` on a fragmented file: the last sync sample at or before the
  time is found through a `sidx` seen before the fragments or, failing that,
  the trailing `mfra` reached through its `mfro` (which needs `total_len` and a
  seekable source); a file with neither is `Error::NoIndex`. The next material
  step after a seek is the `Step::Fragment` containing the time.
- Typed `tfra` (`TrackFragmentRandomAccessBox`), `mfra`
  (`MovieFragmentRandomAccessBox`) and `mfro`
  (`MovieFragmentRandomAccessOffsetBox`) in `timing`, both `Parse` and
  `Serialize`, with the entry count bounded by the box body.
- `cenc::sinf_of_sample_entry`, the one place that knows where a protected
  `encv`/`enca` sample entry's children (and so its `sinf`) begin.
- `movie_fragment::apply_trex_defaults`, the `trex` fallback for a fragment's
  sample defaults, moved down from `transmux`.

- `writer`, the sans-IO fragmented MP4 writer: `Writer::fragmented` returns a
  `FragmentedWriter` whose `init()` assembles `ftyp`+`moov` from `TrackSetup`
  values (a typed `SampleEntry` or a raw sample-entry box, so a track this
  crate has no codec for still writes), and whose `push`/`flush` emit one
  `[styp] [emsg]* moof mdat` fragment capped by `Limits::max_fragment_bytes`
  (`Error::FragmentTooLarge`). `writer::fragmented_segment` is the same
  builder for a caller that already holds its samples, and
  `writer::fragmented::protect_media_segment` (with `FragmentProtection`) is
  the CENC media-segment pass moved down from `transmux` — the
  `senc`/`saiz`/`saio` boxes are emitted by the writer when a sample carries
  `SampleAux`.
- `reader::Reader`, the sans-IO progressive-file reader: it asks the caller
  for structure byte ranges with `Step::Need`, surfaces every track
  (`Step::Tracks`, including handlers and sample entries transmux cannot map,
  with each `stsd` entry's raw bytes), then yields one `SampleRef` at a time in
  decode-time or file order, and `seek_to` lands every track on its sync sample
  at or before a time. It owns the supplied `moov` and nothing else, so a
  `moov` after a multi-gigabyte `mdat` costs headers only; a forward-only
  source gets a typed `Error::MoovAfterMdatForwardOnly` instead.
- `plan::SamplePlan`, the read plan `Reader::plan()` returns from the parsed
  index, with a per-track `cursor(&Limits)` and no per-sample storage.
- `sample_table::CursorState`, the walk's owning state (plain integers) that
  `TrackCursor` is now expressed over, so a reader that owns its `moov` bytes
  can keep a position per track without a self-referential borrow.
- `Error::{MoovAfterMdatForwardOnly, SupplyMismatch, NoIndex}`, and
  `Reader::bytes_requested()` under `scan-counters`.

- `Limits`, the hostile-input caps for the Phase 2 reader: walk depth, `moov`
  size, samples per track, one sample, one fragment and one metadata item,
  each with a named default and a `with_*` builder.

- Typed `stz2` (`CompactSampleSizeBox`, 4/8/16-bit entries) and `sdtp`
  (`SampleDependencyTypeBox`, four 2-bit fields per sample) as `StblChild`
  variants, both symmetric `Parse`/`Serialize`.

- `sample_table::TrackCursor`, a lazy per-track cursor over
  `stts`/`ctts`/`stsc`/`stsz`/`stz2`/`stco`/`co64`/`stss` yielding one
  `SampleRef` at a time with `len()`, `peek_decode_time()` and
  `seek_sync()`. It validates the tables against `Limits` up front and
  allocates nothing proportional to a declared sample count.

- `sample_table::{build_stts, build_ctts, build_stss, build_stsc}`, the
  run-length sample-table encoders moved down from `transmux`.

- Four-CC constants for the boxes the crate already modelled but had not
  named: `segments::{FREE, SKIP}`, `init_segment::{MOOV, MVHD, TRAK, TKHD,
  MDIA, MINF, STBL, STSD}`, `movie_fragment::{TRAF, TFHD, TFDT, TRUN}`,
  `cenc::{TENC, SENC, SAIZ, SAIO, SINF}`, `sample_groups::PRFT`, and
  `box_types::UUID` — so a consumer names a box type through `isobmff` instead
  of a raw literal (enforced by the `one_box_home` guard).

- `BoxItem` (one box: header, offset, whole bytes and body), `BoxIter::clamped`
  (the tolerant mode of the one walker), `BoxItem::children`, and the
  one-level `box_types::find_box` lookup. Strict iteration is `BoxIter::new`;
  the strict `parse_box` remains the single-box entry point.

- The ISO/IEC 14496-12:2026 box layer, extracted from `transmux`: the box /
  FullBox headers (64-bit `largesize`, `uuid` usertype), the `BoxIter`
  walker, `ftyp`/`styp`/`mdat`, the sample-table and timing boxes, the
  visual-extension boxes, the sample-group boxes, the movie-fragment boxes,
  the CENC boxes (ISO/IEC 23001-7 §12), and the §8.8.7/§8.8.8 fragment
  sample-offset resolver.
- `Error`, `Result` and the box-level `TrackId` (the 14496-12 `track_ID`,
  which prints as a bare integer and serialises transparently).
- `ser_timescale` (feature `serde`): serialise a `Timescale` as the plain
  ticks-per-second integer the wire box carries.
- The codec configuration boxes moved from `transmux`: `avcC`/`hvcC`/`vvcC`
  (`avc_config`/`hevc_config`/`vvc_config`), the AVC/HEVC NAL-unit newtypes
  (`nalu_types`), `av1C`/`vpcC` (`av1`/`vp9`), `dOps`, `dfLa`, `dac4`,
  `dac3`/`dec3`, `ddts`, `mhaC`, the Dolby Vision record (`dovi`), the
  `mdcv`/`clli` bodies (`hdr_boxes`), the `esds` descriptor chain
  (`mp4esds`), and the AVS2/AVS3 configuration records
  (`avs2`/`avs3`/`avs3_audio`), with the real-fixture tests that exercise
  them.
- The sample entries and the `moov` tree moved from `transmux`:
  `init_segment` (`mvhd`, `tkhd`, `mdhd`, `hdlr`, `vmhd`/`smhd`, `dref`,
  the sample-table boxes, `stsd`, the audio/visual/subtitle sample entries,
  `SampleEntryVariant`, `StblChild`, `TrackBox`, `MovieBox`, `mvex`/`trex`,
  `edts`, the AVC parameter set stream and the box-level
  `protect_sample_entry`), `sample_entries`, `subtitle_entries`,
  `avc_parameter_set`, `track_reference`, and the `av01`/`vp09`/`avs3`/`avst`
  sample entries in `av1`/`vp9`/`avs3`, with their in-file and real-fixture
  tests.
- `sample_entries::SAMPLE_ENTRY_RESERVED_LEN` and
  `init_segment::MediaInformationBox::set_leading_opaque_header` are public:
  a caller above this crate builds an audio sample entry and installs an
  opaque media header.
- `emsg` (ISO/IEC 23009-1 §5.10.3.3, DASH-IF IOP Part 10): `EmsgBox`,
  `PresentationTime`, `EmsgVersion` and the box constants, moved from the
  absorbed `mp4-emsg` crate, plus its real-fixture tests and examples. The
  emsg parse/serialize errors are variants of this crate's `Error`.

### Changed

- The fragmented walk emits a fragment's `Fragment` step and samples as soon as
  every run of its `traf`s resolves inside the `mdat` payloads it has located,
  instead of first reading the box header after the media. A seekable source
  sees the same `Need`s in a different order; a forward-only source now has the
  `mdat` payload still ahead of it, so `ForwardReader::read_sample` works on a
  fragmented file read from a pipe or socket. A run that addresses data in a
  later `mdat` still resolves once that `mdat` is located.
- `ForwardReader` forces `SampleOrder::File`, documented on the type: a
  `DecodeTime` order across one fragment's tracks is not file order, and a
  forward-only source can only read samples in file order.
- `frag_offsets` addresses a fragment without a slice of the file:
  `sample_ranges_in` / `sample_ranges_in_run` take the parsed `moof`, its
  absolute offset, the `mdat` payload ranges and the run's sample-byte
  allowance; the old whole-file `sample_ranges` is gone. `mdat_ranges` stays
  as the convenience that walks a supplied file.

- `ftyp`/`styp` accept the 64-bit `largesize` and `size == 0` (to-end) box
  forms (ISO/IEC 14496-12:2026 §4.2); the serializer still emits the compact
  form.

### Breaking

- `Metadata::item` is renamed `Metadata::get`.
- `SampleRef` carries CENC per-sample data in `aux` (`Option<SampleAux>`), so
  it is no longer `Copy` (it stays `Clone`); `SampleRef::aux` is `None` for a
  progressive sample and for a constant-IV track.
- `frag_offsets::FragmentSampleRange::{start, end}` are `u64` as on the wire,
  `frag_offsets::MdatRange` holds `u64` offsets and is built with
  `MdatRange::new` / read with `start`/`end`, and `sample_bytes_budget` takes
  and returns `u64`.
- `Error::SampleDataBudgetExceeded::limit` is `u64`, matching the offsets and
  byte counts it bounds.

- The box walkers are consolidated into one iterator. `BoxRef`, `ClampedBox`,
  `box_iter`, `clamped_boxes`, `find_top_box`, `find_child_box`, `child_boxes`,
  `walk_children`, `ChildBox`, `box_support::box_body`, `is_four_cc`,
  `child_extent`, `child_extent_in`, `encode_box_header` and
  `write_compact_box_header` are removed. Use `BoxItem`/`BoxIter` (with
  `BoxIter::new` or `BoxIter::clamped`), `BoxItem::children`, `find_box`,
  the single `box_types::box_body`, `BoxType::is`, `write_box_header` and
  `write_full_box_header`. Wire bytes and parsed results are unchanged.

### Breaking

- New crate. Nothing was released before this entry, so there is no previous
  path to break.
