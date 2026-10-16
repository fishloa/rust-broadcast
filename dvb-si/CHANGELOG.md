# Changelog

## [Unreleased]

### Added (structures the vendored specs define that were wrongly documented as undefined)
- AIT descriptors `provider_export` 0x12 (`ProviderExportDescriptor`) and `provider_usage` 0x13 (`ProviderUsageDescriptor`), ETSI TS 102 727 §10.15 Tables 93/94 (PDF p.181). The tables print the inner loops as `For(j=0; i<N; i++)` (a `j`/`i` typo); the unambiguous length-prefixed reading is implemented and the typo is quoted in the transcription. Spec vector only (no real capture carries them).
- AIT dispatch now routes tag 0x5F to the SI `PrivateDataSpecifierDescriptor` (TS 102 809 §5.3.4.7 / Table 38 note 2: "the DVB SI private data specifier descriptor is defined for use in the Application Information Table").
- SI/MPEG descriptor 0x15 `deferred_association_tags` (`DeferredAssociationTagsDescriptor`, with the Table 65 `DeferredServiceLocation`), ETSI EN 301 192 §11.3.3 Tables 64/65 (PDF p.73); `DescriptorTag::DeferredAssociationTags`. Spec vector only. The COVERAGE row for 0x15-0x1A is narrowed to 0x16-0x1A, whose layouts appear in no vendored spec.
- MPEG-2 extension descriptors 0x10 `VirtualSegmentation` (Table 2-129, PDF p.167) and 0x13 `HevcSubregion` (Table 2-131, PDF pp.169-170), implemented per the determinate text of §2.6.121 (MDL = `maximum_duration_length_minus_1` + 1, i.e. `maximum_duration` is `MDL*8+5` bits; MDL = 0 without timescale) and §2.6.126 (the `SubstreamOffset` loop runs `SubstreamCountMinus1` times; `N1` is implied by the descriptor size). TSDuck 3.44 encodes both one unit differently; its committed fixture is untouched and `tests/h222_descriptors.rs` asserts that those bytes are rejected and what the divergence is. No longer `Mpeg2ExtensionBody::Other`.
- `quality_access_unit::QualityAccessUnit`: the `Quality_Access_Unit()` body of H.222.0 §2.20.1 Table 2-164 (PDF pp.250-251), with the `'0010'` marker-coded 33-bit `media_DTS` through `clock33`. Spec vector only. table_id 0x0A has no section-header syntax in the vendored text, so no section wrapper is implemented.

### Documentation
- ETSI TS 102 606-2 is stated as not vendored (was "not transcribed"); the EACEM/D-Book logical channel row now says EACEM TR-030 is not vendored, that the 10-bit-LCN layout rests on real captures only, and that the vendored NorDig spec defines LCD v1 (0x83) and v2 (0x87) in §12.2.9.
- AIT 0x08 `DVB-HTML_application_descriptor` stays deferred; the note now quotes both the Table 86 `i<N1` loop bound and the unit-less prose, and says a determinate reading needs the owner's call.

### Added (Rec. ITU-T H.222.0 (08/2023) descriptors and sections)
- Descriptors, auto-dispatched by `AnyDescriptor`: `MPEG-4_text` 0x2D (typed-opaque `textConfig()`), `MPEG-4_audio_extension` 0x2E (`audioSpecificConfig()` opaque), `MVC_operation_point` 0x33, `MPEG2_stereoscopic_video_format` 0x34, `Stereoscopic_program_info` 0x35, `Stereoscopic_video_info` 0x36, `Transport_profile` 0x37, `VVC_video` 0x39, `EVC_video` 0x3A (printed 16-bit `descriptor_length` kept 8-bit, spec erratum documented).
- MPEG-2 `Extension_descriptor` (0x3F) bodies typed in `Mpeg2ExtensionBody`: 0x02 `ODUpdate` and 0x11 `timed_metadata_extension` (typed-opaque), 0x03 HEVC timing and HRD, 0x04 `af_extensions`, 0x05 HEVC operation point, 0x06 HEVC hierarchy extension, 0x07 green extension, 0x0F quality extension, 0x12 HEVC tile substream, 0x14 JPEG XS video, 0x15 VVC timing and HRD, 0x16 EVC timing and HRD, 0x17 LCEVC video, 0x18 LCEVC linkage, 0x19 media service kind. 0x10 (virtual segmentation) and 0x13 (HEVC subregion) are typed too, see the spec-defined-gaps entry below.
- Sections, auto-dispatched by `AnyTableSection`: `Iso14496Section` (table_id 0x04/0x05/0x08, payload opaque), `MetadataSection` (0x06, `metadata_byte`s opaque), `GreenAccessUnitSection` (0x09, `Green_AU()` opaque). table_ids 0x07, 0x0B, 0x38, 0x39 have no section syntax in the primary text and remain `Unknown`; for 0x0A the access-unit body is typed (`quality_access_unit`), only its section wrapper is undefined.
- Every `reserved` field is a typed field re-emitted verbatim (default all ones; `zero_bits`/`reserved_zero_*` default zero). New syntax transcriptions in `docs/descriptors/iso_13818_1/` and `docs/tables/iso_13818_1/`, each checked against the rendered PDF page; `tests/h222_descriptors.rs` cross-checks against a TSDuck 3.44 `tstabcomp`-encoded PMT.

### Changed (BREAKING)
- `HevcOperationPoint::profile_tier_levels` is now `Vec<ProfileTierLevelInfo>` (was `Vec<[u8; 12]>`) and the `PROFILE_TIER_LEVEL_LEN` constant is removed. `ProfileTierLevelInfo` types the 96-bit `profile_tier_level_info[i]` (H.222.0 §2.6.101 / H.265 §7.3.3 general part: profile space/tier/idc, 32 compatibility flags, four source/constraint flags, `copied_44bits`, `general_level_idc`); the H.265 sub-layer part lies beyond the 96 bits and is not typed. Round-trips byte-identical.
- `CompatibilityDescriptor` gains a public `empty_count_present: bool` field (#1298). `parse` now records which empty form it read (`compatibilityDescriptorLength = 0`, or `= 2` with `descriptorCount = 0`, ISO/IEC 13818-6 / TS 102 006 Table 15) and `serialize` reproduces it, so a UNT/DSI/DII with the length=2/count=0 form is byte-identical (it used to lose 2 bytes). Struct-literal constructors must add the field; `false` keeps the previous 2-byte `00 00` output.
- `Mpeg2ExtensionBody` gains 15 variants (it is `#[non_exhaustive]`); a body with one of those `extension_descriptor_tag`s that used to surface as `Other` is now typed, and a hand-built `Other` carrying one of them is rejected on serialize.
- `AnyDescriptor` dispatch now types tags 0x2D, 0x2E, 0x33-0x37, 0x39, 0x3A (previously `Unknown`), and `DescriptorTag` gains the matching variants (`Mpeg4Text`, `Mpeg4AudioExtension`, `MvcOperationPoint`, `Mpeg2StereoscopicVideoFormat`, `StereoscopicProgramInfo`, `StereoscopicVideoInfo`, `TransportProfile`, `VvcVideo`, `EvcVideo`). `TableId` gains `Iso14496SceneDescription` 0x04, `Iso14496ObjectDescriptor` 0x05, `Metadata` 0x06, `Iso14496` 0x08 and `GreenAccessUnit` 0x09 (both enums are `#[non_exhaustive]`; an exhaustive-by-wildcard-free `match` elsewhere must add arms). A drift test (`tests/spec_drift.rs`) now walks `declare_descriptors!` / `declare_tables!` against both enums so they cannot diverge again.
- **BREAKING (behavioural):** table_ids 0x04, 0x05, 0x06, 0x08 and 0x09 now dispatch to typed parsers (`Iso14496Section`, `MetadataSection`, `GreenAccessUnitSection`) in `AnyTableSection::parse`, so a malformed section with one of those ids returns `Err` where it used to be `Ok(Unknown)`.
- **BREAKING (behavioural):** a malformed typed `Extension_descriptor` (0x3F) body for `extension_descriptor_tag` 0x03, 0x05-0x07, 0x0F, 0x12 or 0x14-0x19 now fails the whole 0x3F descriptor with `Err` instead of yielding `Mpeg2ExtensionBody::Other`.
- **BREAKING:** `MetadataSection` and the public structs introduced with the H.222.0 batch (`mpeg2_ext_*.rs`, `h222_timing.rs`, `vvc_video.rs`, `evc_video.rs`, `mvc_operation_point.rs`, `stereoscopic.rs`, `transport_profile.rs`, `mpeg4_text.rs`, `mpeg4_audio_extension.rs`) are now `#[non_exhaustive]`, matching `Iso14496Section` / `GreenAccessUnitSection`, so a future spec-field addition is not a break. Outside the crate they can no longer be built with a struct literal or `..Default` update: use the `new()` constructor (added for `OdUpdate`, `TimedMetadataExtension`, `JxsMasteringDisplay`, `JxsVideo`, `LcevcVideo`, `MediaIdentifier`, `HevcHierarchyExtension`, `HevcTileSubstream`, `MvcLevel`, `TimeScaleNk`, `VvcVideoDescriptor`, `EvcVideoDescriptor`; the rest already had one, `AfExtensions` has `Default`) and assign the remaining public fields. Reading fields is unchanged.

### Added (#1294 - AIT/MHP and DVB data-carousel descriptors)
- Nine more **AIT descriptors** in `descriptors::ait` (`AnyAitDescriptor` / `parse_ait_loop`): `application_recording` 0x06 (TS 102 809 Table 21), `DVB-HTML_application_location` 0x09 and `DVB-HTML_application_boundary` 0x0A (TS 102 727 Tables 87/89), `application_icons` 0x0B (TS 102 809 Table 25, with the typed `IconFlags`/`IconFormat` of Table 7), `pre-fetch` 0x0C and `DII_location` 0x0D (TS 102 727 Tables 82/83), `application_storage` 0x10 (TS 102 809 Table 37), `IP_signalling` 0x11 (TS 102 727 Table 81) and `graphics_constraints` 0x14 (TS 102 809 Table 27). Reserved bits are tolerated on parse and preserved on serialize; hand-built defaults are the spec value.
- New **data-carousel descriptor namespace** `descriptors::carousel` (`AnyCarouselDescriptor`, `parse_carousel_loop`, `CarouselDescriptorLoop`) for EN 301 192 Table 48: `type` 0x01, `name` 0x02, `info` 0x03, `module_link` 0x04, `CRC32` 0x05, `location` 0x06, `est_download_time` 0x07, `group_link` 0x08, `compressed_module` 0x09, and from TS 102 006 `SSU_module_type` 0x0A and `subgroup_association` 0x0B. `DiiModule::descriptors()` / `GroupInfo::descriptors()` walk a plain data-carousel `moduleInfo` / `GroupInfoByte` loop in this tag space (previously a raw byte slice).
- Spec transcriptions verified against the rendered PDF pages: TS 102 727 Tables 81-83, 86, 87, 89; TS 102 809 Table 7; TS 102 006 Table 10; clause labels of the EN 301 192 / TS 102 809 / TS 102 006 descriptor tables corrected.

### Changed (BREAKING, #1294)
- `ModuleInfo::user_info` is now a `CarouselDescriptorLoop` (it was a `DescriptorLoop`, which walked the BIOP `userInfo` in the EN 300 468 tag space and so mis-decoded e.g. tag 0x01 as `video_stream` in JSON output). `ModuleInfo::descriptors()` still yields raw `(tag, body)` pairs.
- `CompressedModuleDescriptor` moved to `descriptors::carousel` (re-exported from `carousel::biop::message`) and is now a lifetime-free, fully symmetric `Parse`/`Serialize` type: the `zlib_data` field is gone (the zlib stream is the module payload, never part of the 5-byte descriptor of Table 59) and a body that is not exactly 5 bytes is an error. `ModuleInfo::compressed_module_descriptor()` returns `Option<Result<CompressedModuleDescriptor>>` (no lifetime).
- Not implemented (reason): `DVB-HTML_application_descriptor` 0x08 (TS 102 727 Table 86): the syntax loops `i<N1` over 16-bit `application_id` while the prose calls `appid_set_length` "the length of the list", so count versus byte length is unresolved by the vendored text.

### Documentation
- **Generated coverage reference** `docs/COVERAGE.md`: every table_id, descriptor tag (MPEG-2, DVB, private/regional with their PDS scope and `DescriptorRegistry` opt-in), DVB `descriptor_tag_extension`, MPEG-2 `extension_descriptor_tag`, AIT/INT/UNT/DSM-CC/RNT-scoped descriptor, carousel layer, Annex A charset and time rule — each with its standard, the clause verified against the vendored edition, the module's own citation, its Rust type and status — plus the codes the vendored standards allocate that are **not** implemented (88) and a citation cross-check (35 module docs cite a different clause than the verified edition, issue #1287). Generated by the new `tools/gen-dvbsi-coverage.py` from the `declare_tables!` / `declare_descriptors!` / `declare_extension_bodies!` / `declare_ait_descriptors!` lists and module citations, joined with the hand-transcribed allocation lists in `docs/coverage/allocations.tsv`.
- **Drift guard** `tests/docs_coverage_drift.rs` runs the generator in `--check` mode: CI fails when a table/descriptor is added, removed or re-cited without regenerating the docs.
- **README rewritten** to the new per-crate documentation standard: generated "At a glance" counts and standards table (edition, scope, status per standard), what it does / does not do, a "Which API do I use?" guide, feature flags, design notes, related crates. Every Rust snippet in the README now compiles and runs as a doctest (`#![cfg_attr(doctest, doc = include_str!("../README.md"))]`), against the real TNT capture; the stale claims about `mux`/`resync` living in this crate (they are in `mpeg-ts`) are gone.
- **Crate-root docs** gain the generated "At a glance" block, a module map, a standards summary, a "Which API do I use?" list, a parse-and-round-trip example on real capture bytes and a PDS-scoped private-descriptor example; `tables`, `descriptors`, `carousel` and `text` module docs gain overview tables. `docs/README.md` is now a generated index of the spec transcriptions.
- **Spec-clause citations corrected (#1287).** 35 module `//!` docs (PAT/CAT/PMT/TSDT, RST/DIT, registration, ISO 639, and 27 EN 300 468 / TS 102 809 / TS 102 323 descriptors) cited a clause number that did not match the edition the crate implements (e.g. PAT `§2.4.4.3` -> `§2.4.4.4` of H.222.0 08/2023, network_name `§6.2.28` -> `§6.2.27` of EN 300 468 V1.20.1); the seven Annex D/G/H audio descriptors now cite their exact sub-clause (the AC-4 descriptor cited `§D.5`, which is E-AC-3; it is `§D.7`). Citations now name the edition, and the matching `_§…_` headers of the affected `docs/` transcriptions were fixed. `tools/gen-dvbsi-coverage.py` now **fails** (so does the `docs_coverage_drift` test) when a module cites a clause that is neither the verified clause in `allocations.tsv` nor a sub-clause of it. Every implemented row's clause was re-verified by rendering the PDF page and reading the heading + syntax (new `verified_page` column in `allocations.tsv`; the generator fails for an implemented row without one), not trusted from the converted-text TOC.
- Found while generating the reference: INT/UNT/RNT descriptor loops were decoded in the SI/MPEG tag namespace (#1285, fixed below).

### Added (#1285 - INT/UNT/RNT descriptor namespaces)
- **INT, UNT and RNT descriptor loops now decode in their own tag spaces** (they were walked through the SI/MPEG table, so INT tag 0x06 read as `data_stream_alignment`, UNT 0x02 as `video_stream`, RNT 0x40-0x42 as `network_name` / `service_list` / `stuffing`). New dispatch enums + loop walkers generated by `declare_scoped_namespace!` (ascending tag lists, drift-tested): `descriptors::int::{AnyIntDescriptor, parse_int_loop, IntDescriptorLoop}` (ETSI EN 301 192 V1.7.1 Table 19: tags `0x00..=0x3F` owned by the INT, `Unknown` when unimplemented, never SI), `descriptors::unt::{AnyUntDescriptor, parse_unt_loop, UntDescriptorLoop}` (ETSI TS 102 006 Table 19, same rule), `descriptors::rnt::{AnyRntDescriptor, parse_rnt_loop, RntDescriptorLoop}` (ETSI TS 102 323 V1.4.1 Table 4: only `0x40..=0x42` override EN 300 468 inside the RNT). Tags the spec hands back to EN 300 468 (`0x40..=0xFF` in INT/UNT: telephone 0x57, private_data_specifier 0x5F, user private; every other RNT tag incl. the MPEG-range `metadata_pointer` 0x25) fall through to the SI dispatch as the `Si(AnyDescriptor)` variant. Each loop type has `iter()`, `raw()`, `Deref<[u8]>` and a serde impl that surfaces parse errors as `{"parseError": ...}`.
- **26 typed descriptors** (every INT/UNT/RNT descriptor the standards allocate), each with spec-cited module, `TAG`, borrowed struct, checked `Parse`/`Serialize`, `DescriptorDef`, serde, in-module round-trip tests, and a `trailing` field that keeps any bytes the spec does not define so a non-conformant sender re-serializes byte-identically:
  - shared INT/UNT target descriptors (`descriptors::target`): `TargetSmartcardDescriptor` 0x06, `TargetMacAddressDescriptor` 0x07, `TargetSerialNumberDescriptor` 0x08, `TargetIpAddressDescriptor` 0x09, `TargetIpv6AddressDescriptor` 0x0A;
  - INT (`descriptors::int`): `IpMacPlatformNameDescriptor` 0x0C, `IpMacPlatformProviderNameDescriptor` 0x0D, `TargetMacAddressRangeDescriptor` 0x0E, `TargetIpSlashDescriptor` 0x0F, `TargetIpSourceSlashDescriptor` 0x10, `TargetIpv6SlashDescriptor` 0x11, `TargetIpv6SourceSlashDescriptor` 0x12, `IpMacStreamLocationDescriptor` 0x13, `IspAccessModeDescriptor` 0x14 (+ `IspAccessMode`), `IpMacGenericStreamLocationDescriptor` 0x15 (+ `ModulationSystemType`, `GenericStreamSelector`);
  - UNT (`descriptors::unt`): `SchedulingDescriptor` 0x01 (+ `TimeUnit`), `UpdateDescriptor` 0x02 (+ `UpdateFlag`, `UpdateMethod`), `SsuLocationDescriptor` 0x03, `MessageDescriptor` 0x04, `SsuEventNameDescriptor` 0x05, `SsuSubgroupAssociationDescriptor` 0x0B, `EnhancedMessageDescriptor` 0x0C (reserved bits preserved, default `0b111`), `SsuUriDescriptor` 0x0D;
  - RNT (`descriptors::rnt`): `RarOverDvbStreamDescriptor` 0x40, `RarOverIpDescriptor` 0x41 (reserved bit preserved, default 1), `RntScanDescriptor` 0x42.
- `descriptors::MjdBcdTime` (40-bit MJD+BCD UTC, EN 300 468 Annex C) and `descriptors::MacAddress`; `tests/int_unt_rnt_descriptors.rs` walks and byte-exact round-trips the TSDuck-encoded reference sections (`tsduck-test` reference/test-022 INT with all 15 INT descriptors, test-060 UNT, test-115 RNT, input/test-016 capture INT); enum drift tomls for `ModulationSystemType`, `IspAccessMode`, `UpdateFlag`, `UpdateMethod`, `TimeUnit`; fuzz target `si_scoped_descriptor_loop`.
- Transcription audit against the rendered PDF pages: `docs/descriptors/en_301_192/19-int-descriptors.md` (Table 19 had lost its Platform/Target/Operational column alignment) fixed; clause references of Tables 20-31 (EN 301 192) and 20-38 (TS 102 006) / 14 (TS 102 323) corrected; Tables 34/35 (modulation_system_type, PHY_stream_id) added under `docs/enums/en_301_192/`.

### Changed (BREAKING, #1285)
- **BREAKING:** `IntSection::platform_descriptors`, `UntSection::common_descriptors`, `RntSection::{common_descriptors, ResolutionProvider::descriptors, CridAuthority::descriptors}` and the target/operational loops are now `IntDescriptorLoop` / `UntDescriptorLoop` / `RntDescriptorLoop` instead of `DescriptorLoop` (same raw-bytes API; `.iter()` now yields the namespace's enum). `TargetOperationalLoop<'a>` became `TargetOperationalLoop<'a, L>` generic over the loop type; use the aliases `tables::int::IntLoopEntry<'a>` and the new `tables::unt::UntLoopEntry<'a>`. Constructors take the namespaced loop (`IntSection::new(.., IntDescriptorLoop::new(&[]), ..)`).
- Citations: INT/UNT/RNT table module docs now name the edition as `V1.7.1` / `V1.4.1` / `V1.4.1`. NOTE: the vendored `etsi_ts_102_006_v01.07.01_dvb_ssu.pdf` is TS 102 006 **V1.4.1 (2015-06)** on its title page (the file name is wrong; there is no V1.7.1), so the UNT descriptor modules cite V1.4.1.

### Changed (BREAKING: epoch — requires broadcast-common 10)
- Builds against new caret epochs of workspace siblings (`broadcast-common` 9 -> 10, `mpeg-ts` 0.5 -> 0.6). A caret-epoch change is major-class for this crate under the epoch-purity rule (`docs/RELEASE-AUDIT.md` §2), whatever its own API diff. `Serialize::to_bytes` was removed (use `try_to_bytes`); consumers must resolve a single `broadcast-common` 10.

### Changed (BREAKING, review of the tables/core batch)
- **`section_syntax_indicator` is preserved on every table.** It was neither checked nor kept: a PAT with SSI = 0 parsed and re-serialized with SSI = 1 (and a TDT/RST/DIT with SSI = 1 re-serialized with 0). It is now `SectionBits::section_syntax_indicator` (spec `1` for long-form tables, `0` for TDT/TOT/RST/DIT via the new `SectionBits::short_form()`; `StSection` already preserved it) and a `section_syntax_indicator` field on the tables that keep their own header bits (`CitSection`, `ContainerSection`, `MpeFecSection`, `MpeIfecSection`, `RctSection`, `SatSection`). **BREAKING:** `StSection::section_syntax_indicator` moved to `StSection::bits.section_syntax_indicator`; `SectionBits` gained a field.
- **BREAKING (behavioural):** a NIT/BAT whose `transport_stream_loop_length` is shorter than the bytes left before the CRC is rejected (it was accepted and the trailing bytes silently dropped on re-serialize: a real tnt-5w NIT cut by 64 came back 578 bytes instead of 642). AIT already rejected the same shape. A SAT body with bytes after its last whole entry is likewise rejected instead of losing them.
- **BREAKING (behavioural):** table headers no longer mask over-range values on serialize (the #1129 rule, previously enforced only for descriptors): `version_number` over 31 and a `reserved(2)`/`reserved(3)`/`reserved(4)` or `reserved_*` field wider than its width now return `Error::FieldOverflow` instead of being truncated (`PatSection::new(1, 32, ..)` used to write version 0; `PatEntry::reserved = 0x0F` used to become 7). Covers the shared helpers (`b1_of`, `b5_of`, `write_pid_r`, `write_desc_loop_len_r`) and the per-table reserved masks in MPE, MPE-FEC, protection message, RCT, RNT and SIT.

### Fixed (BREAKING, adversarial review of #1268)
- **BREAKING (behavioural):** the reserved-bit `& MASK` class of #1197 is now actually closed. Five serializers still masked an over-range hand-set reserved value silently (`NordigLogicalChannelV1` `reserved`, `extension::Cp` `reserved_future_use`, `extension::SupplementaryAudio` `reserved_future_use`, `S2XSatelliteDeliverySystem` / `S2Xv2SatelliteDeliverySystem` `reserved_zero_future_use_1`); the guard `tests/serialize_mask_guard.rs` missed them because its matcher only knew hex and ALL_CAPS masks (`& 1`, `& 0b111`) and `self.`-prefixed operands (a `reserved_*` parameter passed to a helper). The guard now matches decimal/binary masks and bare `reserved*` operands, and the further sites it found are converted too: RCT locator `reserved`, SIT service `reserved_future_use`, `application_signalling` `reserved_future_use_1`, `s2_satellite_delivery_system` `reserved_zero_future_use` / `reserved_future_use`, and the shared section helpers `b1_of` / `b5_of` / `write_pid_r` (`reserved(2)` / `reserved(3)` and, in `b5_of`, `version_number`, which was masked "defensively"). All return `Error::FieldOverflow` instead of writing a different value. The same pattern in `mpeg-ts`, `mpeg-pes`, `mpeg-ps`, `transmux` (`colr` nclx) and `scte35-splice` (DVB-TA DAS) is fixed in those crates.
- **BREAKING:** `ShModulation` gains `interleaver_type_when_absent` (EN 300 468 Table 119 always carries the `interleaver_type` bit; with `interleaver_presence == 0` it was dropped, so `3F` re-serialized as `1F`). Captured on parse and re-emitted; `ShModulation::new` defaults to `false`.
- **BREAKING:** `linkage::TargetId::UserDefined` gains `original_network_id_flag` / `service_id_flag` (the two flag bits are on the wire with `target_id_type == 3` although they select no field; `8E` re-serialized as `8C`) and is now `#[non_exhaustive]`: build it with the new `TargetId::user_defined(id)`.
- **BREAKING:** `TelephoneDescriptor`, `ServiceDescriptor`, `ShortEventDescriptor` and `AudioPreselection` gain a `trailing` field. Their syntax (Tables 100/86/87/110) has no tail, but parse accepted bytes after the last field and `serialize` dropped them (not byte-identical). Parse stays tolerant (a real sender can append bytes) and the bytes are kept in `trailing` and re-emitted, the same typed-tail treatment as `video_stream` / `s2_satellite_delivery_system`; `TelephoneDescriptor::new` leaves it empty.
- **BREAKING:** serializing `Mpeg2ExtensionBody::Other` with an `extension_descriptor_tag` in `0x08..=0x0E` is now `Error::InvalidDescriptor` (it was written as the typed MPEG-H body and parsed back as a different variant); use the typed variant.
- **BREAKING (behavioural):** tag 0x3F used to parse as `AnyDescriptor::Unknown`; it is now auto-dispatched to `AnyDescriptor::Mpeg2Extension`, so a malformed MPEG-H body (e.g. a truncated `0x08` body) that used to give `Ok(Unknown)` now returns `Err`.
- **BREAKING (enum):** `Avs3FrameRateCode` gains `Fps119_88` for `0b1110` (T/AI 109.2-2021 `frame_rate_code` table: 120000/1001; listed as a frame rate in T/AI 109.6 Table 10); it was `Reserved(14)`. `Fps200` / `Fps240` / `Fps300` for `0b1011..=0b1101` (T/AI 109.2 codec rates, not listed in the TS descriptor table T/AI 109.6 Table 10); only `0b1111` stays `Reserved`.
- `DescriptorRegistry::parse_loop_in_scope` + `RegistryIter::avs_scope`: T/AI 109.7 clause 8.1.3 and GY/T 420 clauses 7.2.2 / 7.3.3.1 / 7.4.3.1 say only that "the PMT shall contain" the 'AVSV'/'AVSA' `registration_descriptor` (heading "Program and program element descriptors") and place the AVS descriptors in the ES_info loop (7.3.1 d, 8.1.3.4); the registry's scope is per loop, so a `program_info` registration never reached them. Walk `program_info`, take `avs_scope()`, and walk each ES_info loop with `parse_loop_in_scope`. `ExtRegistryIter::avs_scope` + `DescriptorLoop::iter_with_extensions_in_scope` do the same for `iter_with_extensions`.

### Added (adversarial review of #1268)
- Public constructors for the `#[non_exhaustive]` MPEG-H 3D audio sub-structs (`SwitchGroup`, `GroupPreset`, `PresetCondition`, `PresetConditionDetail`, `TextLabelEntry::{group, switch_group_or_preset}`, `TextLabelLanguage`, `MpegH3dAudioTextLabel`, `AuxiliaryLocation`, `MultiStreamGroup::{in_main_stream, in_auxiliary_stream}`, `MultiStreamMain`, `MpegH3dAudioMultiStream`, `DrcScope::{group, preset}`, `DrcInstructions`, `DrcTargetLoudness`, `LoudnessInfoEntry`, `DownmixDefinition`, `DrcLoudnessInfo`, `MpegH3dAudioDrcLoudness`, `MpegH3dAudioConfig`, `MpegH3dAudioCommand`): they could not be built outside the crate. Reserved runs default to all ones; `tests/mpegh_constructors.rs` builds each from outside the crate and round-trips it.

### Added
- MPEG-2 `Extension_descriptor` (tag 0x3F, H.222.0 clause 2.6.90) with the seven MPEG-H 3D audio descriptors (`extension_descriptor_tag` 0x08-0x0E, clauses 2.6.106-2.6.118): typed `MpegH3dAudio`, `MpegH3dAudioScene`, `MpegH3dAudioTextLabel`, `MpegH3dAudioMultiStream`, `MpegH3dAudioDrcLoudness`, opaque `MpegH3dAudioConfig` / `MpegH3dAudioCommand`, verbatim `Other` for the remaining extension tags, all reserved bits preserved. The descriptor is auto-dispatched by `parse_loop` (`AnyDescriptor::Mpeg2Extension`) and `DescriptorTag::Mpeg2Extension` (0x3F) is new. Verified byte-for-byte against the PMTs of the Fraunhofer-IIS MPEG-H transport streams (fetch-on-demand `tools/fetch-test-streams.sh mpegh-*`) and TSDuck-compiled vectors.

### Changed (BREAKING, #1229 - GY/T 422-2025 multi-audio)
- **BREAKING:** `Preselection::audio_rendering_indication` is now the typed `AudioRenderingIndication` (EN 300 468 Table 111 / GY/T 422-2025 Table 2: no preference, stereo, two-dimensional, three-dimensional, headphones, reserved(n)) instead of a raw `u8`.

### Added (#1229)
- GY/T 422-2025 Table 1 (the Chinese multi-audio `audio_preselection_descriptor`, tag 0x7F ext 0x19) is a wire-compatible subset of the existing descriptor (the four DVB flags and `text_label_present` are reserved zero there); `Preselection::is_gy_t_422_profile()` and a spec-vector round-trip test (Table 1, verified on the rendered page) document and check it. Transcription: `docs/descriptors/gy_t_422/audio-preselection-descriptor.md`.
### Added (#1231 - AVS2 and older AVS3 descriptors)
- `descriptors::avs2_video::Avs2VideoDescriptor` (tag 0x40, GY/T 420-2025 Table 3, 5-byte body) scoped by an 'AVSV' registration_descriptor or private_data_specifier (`DescriptorRegistry::with_avs2_video()`; unscoped 0x40 stays `network_name`).
- `descriptors::avs2_audio::Avs2AudioDescriptor` (tag 0xD3, T/AI 109.7-2024 Table 10: text/language/version blocks) scoped by 'AVSA' (`with_avs2_audio()`). `anc_data_block()` is defined in GB/T 33475.3 Annex A.2 (not vendored), so the bytes after the version block are kept undivided in `trailing`.
- `descriptors::tuwa_avs3_video::TuwaAvs3VideoDescriptor` (tag 62, T/UWA 012.2-2023 Table 1, with the typed library-stream references `TuwaLibraryRef::{Peid, StreamId}`), PDS-agnostic opt-in `with_tuwa_avs3_video()`.
- `AnyDescriptor::{Avs2Video, Avs2Audio, TuwaAvs3Video}` variants (opt-in, not auto-dispatched). Transcriptions in `docs/descriptors/gy_t_420/`, `t_ai_109_7/`, `t_uwa_012_2/`.

### Added (#1224 - AVS3 audio)
- `descriptors::avs3_audio::Avs3AudioDescriptor`: the AVS3_audio_descriptor (tag 0xD2) from T/AI 109.7-2024 clause 8.1.3.4 Table 4 (spec located free at aitisa.org.cn): `audio_codec_id`-selected bodies (0, 1 with the optional 24-bit `sampling_frequency`, 2 with the Table 5 `content_type` variants and `total_bitrate`), `resolution`, opaque `addition_info`; reserved bits captured on parse, all-ones defaults when hand-built. Private descriptor: not auto-dispatched; enable with `DescriptorRegistry::with_avs3_audio()`, which scopes it to `PDS_AVSA` ('AVSA', 0x41565341) as ETSI TS 101 154 V2.10.1 section 4.1.8.35 requires. `AnyDescriptor::Avs3Audio` variant added (`@no_dispatch`). The Chinese documents scope it with a `registration_descriptor` 'AVSA' instead of a private_data_specifier; the registry accepts both (a registration whose format_identifier is 'AVSA'/'AVSV' opens the same scope; any other registration leaves the context untouched). Transcription: `docs/descriptors/t_ai_109_7/avs3-audio-descriptor.md`.
- `StreamType::Avs2Video` (0xD2, GY/T 420-2025 7.2.1), `Avs2Audio` (0xD3, T/AI 109.7 8.2), `Avs3Video` (0xD4) and `Avs3Audio` (0xD5) (TS 101 154 V2.10.1 section 4.1.8; T/AI 109.6 / 109.7 clause 8.1.2). Note 0x33 is VVC, not AVS3.
- `descriptors::avs3_video::Avs3VideoDescriptor` (tag 0xD1, T/AI 109.6-2022 clause 9.3.2 Table 9 / GY/T 420-2025 clause 7.3.3.2 Table 6) with typed `Avs3FrameRateCode`; 7- and 8-byte bodies (T/AI 109.6 Table 9 omits `colour_primaries`, its own semantics and GY/T 420 include it - both re-emitted faithfully); `DescriptorRegistry::with_avs3_video()` for 'AVSV' (PDS or registration). `AnyDescriptor::Avs3Video` variant. Transcription: `docs/descriptors/t_ai_109_6/avs3-video-descriptor.md`.
- `PDS_AVSV` (0x41565356, 'AVSV'): the specifier that precedes the AVS3_video_descriptor and the `registration_descriptor` format_identifier for AVS3 HDR; tests for the 'AVSV'/'AVSA' registration format identifiers.
### Added (#1227 - China closed caption, GY/T 270-2013)
- `descriptors::china_caption_service::ChinaCaptionServiceDescriptor` (tag 0x86, clause 6.4 Table 8, `char_set` Table 9): per-service `language`/`caption_service_number`/`wide_aspect_ratio`/typed `ChinaCaptionCharSet`, trailing `caption_service_pid`, all reserved runs preserved. The tag is context-dependent (ATSC uses it differently), so it is not auto-dispatched: `DescriptorRegistry::with_china_caption_service()`. `AnyDescriptor::ChinaCaptionService` variant. Transcription: `docs/descriptors/gy_t_270/caption-service-descriptor.md`.
- `StreamType::ChinaClosedCaption` (0x80, clause 6.2: `cc_data()` in a dedicated caption PES). The `cc_data()` link layer (Table 10) is bit-identical to `cc-data`'s existing parser.

### Changed (BREAKING, #1222 - EN 300 468 v1.20.1)
- Brought up to ETSI EN 300 468 V1.20.1 (2026-09); clause-by-clause delta in `docs/en300468-v1.20.1-delta.md`. v1.20.1 adds no tags, table_ids or wire layouts.
- **BREAKING (behavioural):** the 2038 MJD wrap rule (clauses 5.2.4/5.2.5/5.2.6, 6.2.20, 6.4.9) now applies to every time field (EIT `start_time`, TDT/TOT `UTC_time`, local_time_offset `time_of_change`, network_change_notify `start_time_of_change`): when the msb of the 16-bit MJD field is 0 the MJD is `0x10000` + the field, so `0x03D1123456` is 2040-12-25 12:34:56. Dates before 1948-08-05 are no longer representable (implemented in `broadcast_common::time`).
- **BREAKING:** SAT cell-fragment names follow v1.20.1 spelling: `CellCenter` -> `CellCentre`, `CellFragment::center` -> `centre`, `center_latitude`/`center_longitude` -> `centre_latitude`/`centre_longitude` (serde keys follow).
- SAT `satellite_position_v3_info`: `usable_start/stop_time_flag` (when `metadata_flag == 0`) and `interpolation_type`/`interpolation_degree` (when `interpolation_flag == 0`) are documented as "not used" (was "reserved"); values are preserved verbatim, now pinned by a test.

### Added (#1222)
- `PDS_AVSA` (`0x41565341`, 'AVSA'): the private_data_specifier that precedes the AVS-P3 audio descriptor (Annex O).
- Tests carrying the v1.20.1 Table 26 component types (AVS3-P3 audio `0x9/0x1/0x20-0x22`, T/UWA 005 DMI `0xB/0xF/0x0A`) byte-exact through `ComponentDescriptor`.

### Changed (BREAKING, #1197)
- **BREAKING (behavioural):** every remaining single-field `(value & MASK)` in a serializer is a checked `broadcast_common::len::fit_mask(..)?` that returns `Error::FieldOverflow` instead of silently dropping the high bits of an over-range value (the #1129 class; ~220 sites across the descriptors and tables, including bare `(field << n)` shifts such as `satellite_table_id`, `reference_type`, `c2_system_tuning_frequency_type`). Hand-built values that were wider than their field (reserved runs, `Reserved(v)` enum payloads such as `Bandwidth::Reserved(0xFE)` / `FecInner::Reserved(0x3E)`, 5-bit `stream_content_ext`, ...) used to be masked and now fail; values produced by `parse` always fit. Guarded by `tests/serialize_mask_guard.rs` (lexical, reasoned allow-list) and `tests/serialize_mask_overflow.rs` (behavioural).
- **BREAKING:** the SAT (`tables::sat`) body structs gain typed `reserved_zero_future_use*` fields for every reserved run of EN 300 468 Tables 11c/11d/11e/11g/11h (captured on parse, re-emitted verbatim, hand-built default `0`): `PositionV2Satellite`, `PositionSystem::Orbital`, `CellCenter`, `CellFragment`, `NewDeliverySystem`, `ObsolescentDeliverySystem`, `TimeAssociationBody`, `BeamhoppingPlan`, `BeamhoppingMode::{Mode0, Mode1, Mode2}`, `PositionV3Body`, `PositionV3Satellite`, `PositionV3Metadata`, `UsableTime`, `EphemerisData`, `CovarianceData`. The SAT bit writer no longer truncates: an over-range value (a count, a 9-bit `*_ext` above 0x1FF, a signed latitude out of range) is `Error::FieldOverflow`, and `first_occurence` without a `center` is `ValueOutOfRange`.
- **BREAKING:** `StSection` gains `section_syntax_indicator` (EN 300 468 Table 11: the stuffing section's indicator "may take either the value 0b1 or 0b0"); captured on parse, re-emitted verbatim, `new()` keeps `false`.

### Fixed (#1197)
- DTS `sample_rate_code` 0b1001 (88,2 kHz): the decision is now documented with the TS 102 114 / EN 300 468 V1.20.1 evidence (`docs/reserved-bits-edition-audit.md` section 12); no behaviour change.
- Local-archive evidence for the SSU selector (real zero reserved runs preserved) and `logical_channel` v1 (`reserved` = 1 in every real capture): `tests/local_archive_reserved_defaults.rs`.

### Fixed

- `--no-default-features --features chrono` now builds for bare-metal (`thumbv7em-none-eabi`): `epg` was missing `alloc::vec` and carried a serde-only `format`/`ServiceData` unconditionally (#1181).
### Changed (BREAKING)
- **BREAKING:** the ten `PID = 0x0000` placeholder constants that aliased the PAT PID are removed from the `ait`, `container`, `downloadable_font_info`, `int`, `mpe`, `mpe_fec`, `mpe_ifec`, `protection_message`, `rct` and `unt` table modules (a caller filtering on one picked up PAT traffic); these tables have no fixed PID (audit r02-W15 / r02-O12, #1077). The hidden `owned::doc` doctest helper module is removed too (r02-O7).
- **BREAKING (behavioural):** serializers reject a value wider than its wire field instead of masking it into a different valid-looking value (audit r03-W3 class, #1077): `ibp_descriptor.max_gop_length` (14 bits), NorDig LCN V1 (14) / V2 (10), `smoothing_buffer` (22), `metadata_std` (22), `multiplex_buffer_utilization` (15), `mvc_extension` (10 / 3), `target_background_grid` and `video_window` (14 / 14 / 4), INT `platform_id` (24), UNT `OUI` (24), RCT `dvb_service_triplet_id` (10), `content_labeling` time base (33), S2Xv2 scrambling indices (20). Parse stays tolerant; legal values round-trip byte-identically.
### Fixed
- DTS descriptor `sample_rate_hz()` for SFREQ `0b1001` is 88200 Hz (the table prints the digit transposition "88,02 kHz"; EN 300 468's own DTS-HD table lists 88,2 kHz) (r03-W11).
- BIOP directory / stream / stream-event serializers use the checked `fit_*` helpers like the file message (r02-W13).
### Performance
- Section collectors copy a section into an `Arc` only when it is retained: a duplicate carousel repeat allocates nothing (r02-O1, `tests/collect_alloc.rs`).
- Annex A text decode: no intermediate `Vec<u16>` for UCS-2 and no second `String` when no control code is present (r02-O4: 8 -> 1 and 15 -> 2 allocations for 200-character strings, `tests/text_alloc.rs`).
- SAT body sizing trial-encodes on the stack: a `serialized_len` + `serialize_into` pair no longer heap-allocates 16 KiB (r02-O2, `tests/sat_alloc.rs`).
- One `tables::write_crc` helper and one `CRC_LEN` replace ~25 copies of the CRC trailer code (r02-W10); residual bare section-header masks are named (r02-W16).
### Changed (BREAKING)

- **PID fields are the typed `mpeg_ts::pid::Pid`** (typed-ids wave 1b): `PatEntry::pid`, `PmtStream::elementary_pid`, `PmtSection::pcr_pid`, `CatCaEntry::ca_pid`, `CaDescriptor::ca_pid`, `Cp::cp_pid`, `PatSection::nit_pid()` (now `Option<Pid>`), and every per-table `PID` const (`pat::PID`, `eit::PID`, ...). The matching constructors (`PatEntry::new`, `PmtStream::new`, `PmtSection::new`, `CaDescriptor::new`, `Cp::new`) take `Pid`, so an over-13-bit value is unrepresentable (use `Pid::try_new` at a user boundary); the old serialize-time `FieldOverflow` for `ca_pid`/`cp_pid` is gone with it. Parse masks the 13 wire bits into the `Pid` (the 3 reserved bits stay in their typed reserved fields); serde and `dvb-tools` output are byte-identical (`Pid` serializes transparently as the number). `CatCaEntry` no longer derives `Default` (`Pid` has none). `XaitPid::xait_pid` stays a raw `u16`: TS 102 727 Table 95 defines one 16-bit field with no reserved split and the crate round-trips it verbatim. Migration: drop `Pid::new(entry.pid)` / `.value()` adaptors at call sites; pass `Pid::new(..)`/`Pid::try_new(..)` where a literal was used. Gate: `real_capture_byte_identity` now compares every PAT/PMT/CA PID against an independent 13-bit wire extraction over all real captures; `tests/typed_pid.rs` pins the boundary.
### Changed (breaking)

- **Tables preserve every `reserved` / `reserved_future_use` bit of the section header and body in typed fields** (reserved-bits audit §3.6): parse is tolerant and `serialize` re-emits what was parsed, so a real sender's section re-serializes byte-identically (CRC is recomputed); values built with `new()` / `Default` carry the spec fill (all ones, except the PSI `'0'` bit, `reserved_zero_future_use` = 0).
  - New `tables::SectionBits { private_indicator, reserved_b1, reserved_b5 }` (`#[non_exhaustive]`, `SectionBits::psi()` / `::dvb()`). The bit after `section_syntax_indicator` is H.222.0's `private_indicator` ("user definable" in all nine editions) and EN 300 468's `reserved_future_use`: PSI sections (PAT/PMT/CAT/TSDT) default it to `0`, EN 300 468 tables to `1`. New `bits` field on `PatSection`, `PmtSection`, `CatSection`, `TsdtSection`, `SdtSection`, `NitSection`, `BatSection`, `EitSection`, `TdtSection`, `TotSection`, `RstSection`, `StSection`, `DitSection`, `SitSection`, `AitSection`, `IntSection`, `UntSection`, `RntSection`, `ProtectionMessageSection`, `DownloadableFontInfoSection`.
  - The private-section tables that already carried `private_indicator` (`DsmccSection`, `ContainerSection`, `CitSection`, `RctSection` (`table_id_extension_flag`), `SatSection`, `MpeDatagramSection`, `MpeFecSection`, `MpeIfecSection`) gain `reserved_b1` / `reserved_b5` instead.
  - Body reserved fields are typed on the entry they belong to: `PatEntry::reserved` (3), `PmtStream::{reserved_pid, reserved_es_info_length}`, `PmtSection::{reserved_pcr_pid, reserved_program_info_length}`, `CatSection::reserved_extension` (the 16 reserved bits at bytes 3-4), `SdtService::reserved_future_use` (6), `SdtSection::reserved_future_use` (8), the `reserved_future_use(4)` before every descriptor-loop length in NIT/BAT/SIT/AIT/INT/UNT/RNT/RCT/TOT/`TargetOperationalLoop`/`ResolutionProvider`, `RstEntry::reserved_future_use` (5), `DitSection::reserved_future_use` (7), `SitService::reserved_future_use` (1), `CridAuthority::reserved` (2), RCT `LinkInfo`/`DvbBinaryLocator` reserved bits, MPE-FEC byte 4/5 reserved bits, `SatSection::reserved_zero_future_use` (8), DFIS `StyleWeight`/`FileUri` `reserved_zero_future_use`, protection-message loop/count nibbles.
  - **Migration:** the touched table structs are now `#[non_exhaustive]` (struct-literal construction no longer compiles outside the crate): use the new constructors `PatSection::new`, `PatEntry::new`, `PmtStream::new`, `CatSection::new`, `TsdtSection::new`, `SdtSection::new`, `SdtService::new`, `NitSection::new`, `NitTransportStream::new`, `BatSection::new`, `BatTransportStream::new`, `EitSection::new`, `RstSection::new`, `RstEntry::new`, `DitSection::new`, `SitSection::new`, `SitService::new`, `AitSection::new`, `AitApplication::new`, `IntSection::new`, `UntSection::new`, `UntPlatform::new`, `TargetOperationalLoop::new`, `RntSection::new`, `ResolutionProvider::new`, `CridAuthority::new`, `RctSection::new`, `LinkInfo::new`, `DvbBinaryLocator::new`, `CitSection::new`, `ContainerSection::new`, `DsmccSection::new`, `MpeDatagramSection::new`, `MpeFecSection::new`, `MpeIfecSection::new`, `ProtectionMessageSection::new`, `ProtectionMessageBody::certificate_collection`, `DownloadableFontInfoSection::new`, `FontInfo::{style_weight, file_uri}`, `SatSection::new` (`TdtSection::new`/`TotSection::new` already existed); the reserved fields stay `pub` for off-spec senders. `CatSection` / `PatSection` `Default` keep their meaning (`PatSection::default()` is now the PSI-filled value). `ProtectionMessageBody::AuthenticationMessage` / `CertificateCollection` and `FontInfo::{StyleWeight, FileUri}` gained fields, so struct-literal construction and exhaustive patterns need updating (add `..`).
  - serde output is unchanged (every reserved field is `serde(skip)`).
  - Real captures: every table section of the local `.test-streams` archive now re-serializes byte-identically except `section_syntax_indicator` of the random-bits stress capture `test-062` (a structural bit, not a reserved bit); TDT `test-120`, NIT `test-016`, PAT `test-012`/`test-065`, SDT `test-170`, EIT `test-109`/`test-170`/`test-171`, AIT `test-170` and DSM-CC/ST `test-062` left the `TOLERATED` list.
  - Not yet preserved: bit-level `reserved_zero_future_use` runs inside the `SAT` bodies (Position/Cell-fragment/Time-association/Beamhopping, `BitWriter`-emitted zeros).
### Changed (BREAKING)

- **Descriptors now preserve reserved bits (owner decision; supersedes r03-W7/r03-W8 "drop on parse, emit the spec value").** Parse stays tolerant; every `reserved` / `reserved_future_use` / `reserved_zero_future_use` region of every descriptor under `descriptors/` (incl. `extension/` and `ait/`) is captured in a typed, spec-named, right-aligned field (`reserved`, `reserved_future_use`, `reserved_flags`, `reserved_zero_future_use_6`, ...) and re-emitted verbatim, so real-sender descriptors re-serialize byte-identically; descriptors built with `new(..)`/`Default` carry the governing edition's value (audit: `docs/reserved-bits-edition-audit.md`). Every real descriptor in the local capture archive now re-serializes byte-identically (before: AC-3 47/62, application_signalling 15/52, supplementary_audio 37/47, satellite 10/678, CA 2/715, TVA_id 1/1 differed; after: 0).
  - **AC-3 `reserved_flags` default is now `0000`** (every EN 300 468 edition V1.4.1-V1.20.1 and TS 102 366 say "should always be set to 0"; dvb-si wrongly emitted `1111`). `application_signalling` entries gain `reserved_future_use_1` (MHP-era senders write 0 before the 15-bit `application_type`) and `reserved_future_use_3`. `satellite_delivery_system` now re-emits `roll_off` on DVB-S too (sender `01` round-trips; a constructed DVB-S descriptor uses `Alpha035` = `00`).
  - All changed structs are `#[non_exhaustive]` with `new(..)` and/or `Default`: **struct-literal construction outside dvb-si no longer compiles** (use `new`/`Default` + field assignment). New `*_DEFAULT` consts replace the old private `RESERVED_*` masks (extension-module consts are module-prefixed to avoid glob collisions).
  - Structural changes: `MosaicLogicalCell.elementary_cell_ids: Vec<u8>` -> `elementary_cells: Vec<ElementaryCell>`; `TtmlSubtitling.font_ids` -> `Option<Vec<TtmlFont>>`; `RelatedContentDescriptor` is no longer a unit struct (`trailing`); `C2DeliverySystem` is no longer `Copy`; `ShModulationMode::Tdm` / `ShInterleaver::Type1` are `#[non_exhaustive]` (use `ShModulationMode::tdm` / `ShInterleaver::type1`).
  - Tail bytes that parse used to drop silently are now kept in typed `trailing` fields and re-emitted: `video_stream`, `protection_message`, `related_content`, `c2_delivery_system`, `s2_satellite_delivery_system`.
  - New per-descriptor tests (parse non-default reserved -> byte-identical re-serialize; default-constructed -> spec value), mutation-verified; `real_capture_byte_identity` `TOLERATED` no longer lists any descriptor.


### Changed (BREAKING)

- **BREAKING (#1159):** `TsHeader::pid` (re-exported via `mpeg-ts`) is now `Pid`; `collect::*::pid` / `epg::EpgStore::feed_with_pid` take `Option<Pid>` instead of `Option<u16>`.
### Fixed

- **`S2Xv2_satellite_delivery_system` `satellite_id` over 24 bits is rejected (#1129).** It was written as three bytes of `value >> 16 / >> 8 / as u8`, silently dropping the top byte; it now returns `Error::FieldOverflow`. `CompatibilityDescriptor` length/count narrowing now goes through `broadcast_common::len::fit_*`, and `tests/serializer_length_truncation.rs` scans all of `src/` (it covered only `descriptors/`, `tables/` and `carousel/`).
- `hevc_video`: byte 12's `reserved(2)` (bits `[3:2]`) and the temporal sub-block's two `reserved(5)` fields are now all ones per H.222.0 Table 2-113. Real captures (9 HEVC_video_descriptors in the local TSDuck archive carry `0x9F` + `1F 1F`) contradicted the earlier r03-W7 reading that cleared bit 3 and the sub-block reserved bits, so they did not re-serialize byte-identically. Parse unaffected.

### Added

- `tests/table_reserved_bits.rs`: single-bit-flip property over every table type (real captured sections plus built sections): every flipped bit must survive `parse -> serialize`; spec-fill defaults; real-sender sections from the local archive.
- `tests/real_capture_byte_identity.rs`: re-serializes every table section and every descriptor in every PMT/SDT/NIT/BAT/EIT/CAT/TSDT/TOT loop of every capture under the local `.test-streams/` archive (skips when absent) and asserts byte-identity, with an explicit `TOLERATED` allowlist of reserved-bit deviations by real senders keyed by (capture, descriptor/table). Backed by a `#[doc(hidden)]` `AnyDescriptor::reserialize()`.

## [11.1.0] - 2026-10-13
### Fixed

- Presence flags stored next to an `Option` no longer drive the serializer, so a mismatch cannot write a misframed descriptor (r03-W5, #1138): `metadata` (`dsmcc_flag`), `content_labeling` (`content_reference_id_record_flag`, and an indicator/Option mismatch now returns `ValueOutOfRange`), `j2k_video` (`extended_capability_flag`, the stripe/block/mdm flags, and a missing `color_specification` now returns `ValueOutOfRange` instead of leaving a stale byte), `supplementary_audio` (`language_code_present`). The flag fields are kept (informational, set by parse); serialize derives the wire bit from the `Option`.
- **Reserved-bit emission (r03-W7) — serialized bytes change for non-conformant reserved bits; parse unaffected; lockstep minor.** `reserved` bits are now emitted as 1 per ISO/IEC 13818-1 / H.222.0 (and TS 102 006 Table 4) in `video_stream`, `system_clock`, `maximum_bitrate`, `smoothing_buffer`, `STD`, `AVC_timing_and_HRD`, `SVC_extension`, `MVC_extension` (confirmed by the TSDuck-compiled oracle PMT; `video_stream` also by the real `shared/real-pmt.bin`), plus `multiplex_buffer_utilization`, `metadata_pointer`, `metadata` and the SSU `data_broadcast_id` selector (spec convention only, no fixture carries them). Parsers still accept any reserved value and re-serialize normalizes it; `tests/reserved_bits_real_streams.rs` asserts the committed real/oracle descriptors re-serialize byte-exactly. `j2k_video` stays 0 (its table says "all bits to be set to '0'"). The remaining EN 300 468 / TS 102 809 descriptors and tables were swept and already follow their tables (`reserved_future_use`=1, `reserved_zero_future_use`=0).
- `video_depth_range`: a `production_disparity_hint` with `range_length > 3` or a multi-region-SEI entry with a body is preserved verbatim as `DepthRangeBody::Other` instead of dropping its tail; `protection_message` ignores (does not reject) bytes after the component-tag loop (r03-W9).

## [11.0.1] - 2026-10-11
### Fixed

- Production parsers no longer use `first_chunk::<N>().unwrap()` / `try_into().unwrap()` for big-endian field reads; they go through a checked `wire::be_u16`/`be_u32` helper that returns `Error::BufferTooShort` instead of being able to panic. Behaviour is byte-identical for valid input; public API unchanged.

## [11.0.0] - 2026-10-05
### Changed (breaking)

- **#1141 (r03-O1, r03-O3)**: the four `multilingual_*` descriptors and the AIT `application_name_descriptor` now share one `lang_text` reader/writer, and `teletext_descriptor` / `VBI_teletext_descriptor` share one entry loop (and one over-range length check). Observable changes: an over-range name/provider/text in `multilingual_network_name`, `multilingual_service_name` and `application_name` now serializes to `Error::FieldOverflow` (as `multilingual_bouquet_name`/`multilingual_component` already did) instead of `Error::InvalidDescriptor`; `teletext_descriptor` and `VBI_teletext_descriptor` now share one serializer, so an over-range body (over 255 bytes) is `Error::FieldOverflow` from the checked `descriptor_length` write for both (the VBI copy used to return `InvalidDescriptor` from a hand-written check); `VbiTeletextEntry` is now a type alias of `TeletextEntry` (same fields; `TeletextEntry` gains `Copy`). Wire output and parse results are unchanged.
- Table and carousel/BIOP serializers now return an error, instead of
  silently truncating, when a length or count value does not fit its wire
  field (#1129). `dvb_si::Error` gained a new `FieldOverflow` variant
  (`#[error(transparent)]` over `broadcast_common::len::FieldOverflow`).
- `broadcast-common` requirement raised to `9.4` (same epoch) for the new
  `broadcast_common::len` helpers.
- `tables::int::IntLoopEntry` is now a type alias for the new shared
  `tables::TargetOperationalLoop` (same fields), and
  `tables::unt::UntPlatform::target_operational_pairs` changed from
  `Vec<(DescriptorLoop, DescriptorLoop)>` to
  `Vec<TargetOperationalLoop>` — INT and UNT define the identical
  target/operational descriptor-loop pair and no longer use two
  representations (r02-W22).
- `collect::CompleteNit`/`CompleteBat`: `network_descriptors`/
  `bouquet_descriptors` only reflected section 0's loop, though EN 300 468
  §5.2.1/§5.2.2 give every section its own loop. `ParsedDescriptorLoop` now
  merges every contributing section's loop (`raw()` returns `&[DescriptorLoop]`,
  one per section — a breaking API change).
- `carousel::biop::message::ServiceGatewayInfo`: had no `Serialize` impl
  (broke the crate-wide Parse/Serialize symmetry), and `to_bytes` panicked
  on over-255 `service_context` entries and silently truncated an
  over-65,535-byte `user_info`. Implemented `Serialize`; `to_bytes` now
  returns `Result<Vec<u8>>` (breaking).
- `tables::pmt`/`tables::dsmcc`: removed the `PID = 0x0000` placeholder
  constant — it equals the real PAT PID, so a caller filtering on it would
  silently pick up PAT traffic (breaking).
- `tables::rct`: the descriptor loop in `LinkInfo`/`RctSection` accepted
  trailing bytes between the loop and `link_info_length`/the CRC, silently
  dropping them on re-serialize. Both now rejected. `DvbBinaryLocator` also
  carried `identifier_type`/`inline_service` as separate fields alongside
  `identifier`/`service`, a second source of truth a caller could set
  inconsistently with the enum variant being serialized; both are now
  derived (`identifier_type()`/`inline_service()` methods), and
  `windows`/`identifier`/`scheduled_time_reliability` consistency is
  validated at serialize (breaking: the two fields were removed from the
  struct).
- `tables::ait`: `ApplicationType::UserDefined` (documented for
  `0x8000..=0xFFFF`) could never be produced by `from_u16`, since
  `application_type` is a 15-bit wire field (bit 15 is the separate
  `test_application_flag`) — and the serializer silently dropped bit 15 of
  a directly-constructed out-of-range value instead of rejecting it.
  Removed the unreachable variant (breaking) and added an explicit 15-bit
  range check on serialize.
- `descriptors::private_data_indicator`: corrected the module citation
  (§2.6.22, which is actually `multiplex_buffer_utilization_descriptor` →
  §2.6.28) and removed a re-exported DVB PDS-registry name lookup that
  doesn't apply to this ISO/IEC 13818-1 field (breaking).
- Serializers no longer fabricate wire values for flag-gated fields:
  `unwrap_or(0)`/defaulted conditionals in metadata_pointer (transport_stream_id
  locator flag now derived from the Option), avc_timing_and_HRD (N/K),
  video_stream (profile_and_level_indication/chroma_format/
  frame_rate_extension_flag), s2/s2x/s2xv2 satellite delivery (scrambling
  sequence index, ISI, timeslice, SFFI, beamhopping time plan id), ac4 (config
  fields) and uri_linkage (min_polling_interval for ungated types) now return
  `ValueOutOfRange` instead of silently inventing bytes; `ExtensionDescriptor`
  serialization rejects a `tag_extension` that disagrees with its typed body;
  `ExtendedEventLinkageEntry` no longer stores `target_id_type`/
  `original_network_id_flag`/`service_id_flag` — those wire bits are derived
  from its `TargetId` (breaking) (r03-W5, #1077).
- `carousel::biop::message::ServiceGatewayInfo::download_taps` was a raw
  `&[u8]` (count byte + tap bytes) instead of a typed `Vec<Tap<'_>>`; a
  caller had to hand-parse each `Tap` itself even though the crate already
  has a `Tap` parser used by every other BIOP consumer (r02-W12, breaking).
- `carousel::biop::fs::DirectoryObjectData::entries` was an untyped
  `Vec<(Vec<u8>, u16, Vec<u8>)>`; replaced with a named `DirectoryEntry {
  name, module_id, object_key }` (r02-W12, breaking).
- `carousel::biop::message::ModuleInfo::user_info` was a raw `&[u8]`
  reimplementing its own private tag/length descriptor-loop walker;
  replaced with `descriptors::DescriptorLoop` (the crate's existing typed
  wrapper) and `descriptors()` now delegates to
  `DescriptorLoop::raw_tags()` instead of a duplicate walker (r02-W12,
  breaking).
- `carousel::biop::message::CompressedModuleDescriptor` exposed only a raw
  `body: &[u8]`, so the `compression_method`/`original_size` fixed fields
  (EN 301 192 §10.2.11 Table 59) were left for a caller to hand-slice off
  the front of the zlib stream. Split into typed `compression_method`,
  `original_size` and `zlib_data` fields; `ModuleInfo::compressed_module_descriptor`
  now returns `Option<Result<CompressedModuleDescriptor<'_>>>` so a body
  shorter than the fixed fields is a surfaced error, not a panic (r02-W12,
  breaking).

- The MPE-FEC/MPE-IFEC `real_time_parameters` encode no longer masks
  silently: `delta_t` over 12 bits or `address`/`prev_burst_size` over 18 bits
  now fail `serialize_into` with `Error::FieldOverflow` instead of being
  masked (release audit).

### Added
- `carousel::biop::message::MAX_DECOMPRESSED_MODULE_SIZE` (64 MiB) and
  `decompress_zlib_bounded(data, max_len)`, a general form of
  `decompress_zlib` for callers that have a tighter, descriptor-declared size
  to enforce.

### Fixed
- `descriptors::extension::dts_hd`: a DTS-HD asset with `component_type_flag`
  or `language_code_flag` set but no value now fails to serialize
  (`Error::ValueOutOfRange`) instead of emitting an invented `0` /
  `"   "` (release audit).
- Remaining unchecked `.len() as u8/u16` length narrowings in the
  `tables/` and `carousel/` serializers (`downloadable_font_info`,
  `protection_message`, `carousel::{messages, biop::message, biop::ior}`) now
  go through `broadcast_common::len::fit_*`; the regression scan in
  `tests/serializer_length_truncation.rs` covers `src/tables` and
  `src/carousel` as well as `src/descriptors` (release audit).
- 17 table serializers (pat, cat, tsdt, pmt, nit, bat, sdt, ait, int, sit,
  dsmcc, st, rst, downloadable_font_info, protection_message, eit, plus the
  narrowed inner loops in cit/rct) wrote a 12-bit `section_length` (or a
  nested loop length) with no range check, so an oversized body wrapped mod
  4096 and was written with a valid CRC over bytes the header didn't cover
  (#1129, #1001).
- 8 more serializers (cit, container, mpe, mpe_fec, mpe_ifec, rnt, tot, unt)
  guarded `section_length` by casting to `u16` *before* comparing against
  the 12-bit maximum, so a body of 65 536+ bytes wrapped the guard itself
  and bypassed it; the compare now happens in `usize` before any narrowing
  (#1129).
- Nested 8-bit/6-bit length and count fields in RCT (`uri_length`,
  `number_items`, `promotional_text_length`, `link_info_length`,
  `number_of_links`) and CIT (`unique_string_length`) silently truncated
  with no guard at all (#1129, #1003).
- `carousel::biop`'s `FileMessage`/`StreamMessage`/`StreamEventMessage`
  serializers cast `messageSize`/`messageBody_length` to `u32` with no
  check, inconsistent with `DirectoryMessage`'s guarded equivalent; several
  BIOP IOR profile length fields (`NameComponent`, `ServiceLocation`,
  `TaggedProfile`, `IOR`) had the same gap (#1129).
- Added a shared crate-internal `write_section_length` helper (not public API) that every
  table serializer now routes its outer `section_length` write through, so
  the guard cannot be forgotten again for a future table.
- ~80 descriptor serializers (starting from the `network_name_descriptor`
  template) wrote the outer 8-bit `descriptor_length` header byte with no
  range check, so a body over 255 bytes wrapped mod 256 and misframed the
  rest of the descriptor loop (#1129). Added a shared
  crate-internal `write_descriptor_header` helper (not public API; tag + checked 8-bit
  length) that every descriptor serializer now routes its header write
  through.
- Nested per-field lengths and counts inside descriptor bodies had the same
  gap and are now range-checked before narrowing, including several
  bit-packed sub-byte fields that previously masked an over-range value
  instead of rejecting it: `audio_preselection`'s `num_preselections` (5
  bits), `num_aux_component_tags` (3 bits) and `future_extension_length` (5
  bits); `target_region_name`'s `region_name_length` (6 bits);
  `vvc_subpictures`'s `number_of_vvc_subpictures` (6 bits);
  `protection_message`'s `component_count` (4 bits); `telephone`'s five
  packed char-field lengths (2/3/2/3/4 bits); and the
  `num_channel_bonds_minus_one` (1 bit) field shared by
  `s2x_satellite_delivery_system` and `s2xv2_satellite_delivery_system`.
  Plain 8-bit/16-bit fields across `short_event`, `extended_event`,
  `service`, `metadata`, `metadata_pointer`, `content_labeling`,
  `content_identifier`, `data_broadcast`, `data_broadcast_id`, `mosaic`,
  `carousel_identifier`, `nordig`, `cell_list`/`cell_frequency_link`,
  `t2_delivery_system`, `cpcm_delivery_signalling`, `image_icon`,
  `ttml_subtitling`, `ac4`, `uri_linkage`, `video_depth_range`, the
  `multilingual_*_name`/`component` descriptors, and the AIT descriptors
  (`simple_application_boundary`, `dvb_j_application`,
  `dvb_j_application_location`, `application_name`) had the same gap.
- Added a source-scan regression test
  (`dvb-si/tests/serializer_length_truncation.rs`) that fails CI if the
  literal unchecked pattern (`<expr>.len() as u8/u16/u32`) reappears
  anywhere under `src/descriptors/` outside a `#[cfg(test)]` module.
- `content_labeling_descriptor`'s 33-bit `content_time_base_value` /
  `metadata_time_base_value` were read/written from the wrong bit position
  (Table 2-83 is `reserved(7) | value(33)`; a conformant `FE 00 00 00 01`
  decoded as `8522825728` instead of `1`) (#1004).
  Verified against a `tstabcomp`-compiled PMT fixture
  (`fixtures/dvb-si/tsduck-w4-spec-misread-pmt.{xml,bin}`).
- `J2K_video_descriptor`'s extended-capability body had `still_mode`/
  `interlaced_video` positioned after the colour parameters and stripe/
  block/mdm sub-blocks instead of directly after the stripe/block/mdm flags
  byte (Table 2-101), misreading every field that follows for any real
  descriptor with `extended_capability_flag = 1` (#1005). Same TSDuck
  fixture as above.
- `Metadata_STD_descriptor`'s 22-bit `metadata_input_leak_rate` /
  `metadata_buffer_size` / `metadata_output_leak_rate` fields folded their 2
  leading reserved bits into the value (a conformant `C0 00 01` decoded as
  `12,582,913` instead of `1`) (#1006). Same TSDuck fixture as above.
- `FlexMuxTiming_descriptor`'s body length was 10 instead of the spec's 8
  (Table 2-82: `FCR_ES_ID`(16) + `FCRResolution`(32) + `FCRLength`(8) +
  `FmxRateLength`(8) = 64 bits), rejecting every conformant descriptor and,
  on serialize, writing `descriptor_length = 10` while only filling 8 body
  bytes, leaking 2 stale caller bytes into the wire output (#1053). Same
  TSDuck fixture as above.
- `collect::SectionSetCollector` and `collect::EitCollector` counted
  **completed** entries against their partial-key cap forever, so once the
  map filled with completed sets a new distinct key silently got `Ok(None)`
  — indistinguishable from "still collecting" — with no signal that
  capacity, not incompleteness, was the reason (#1002). A new key at
  capacity now evicts the least-recently-touched existing key instead
  (`evicted_for_capacity()` / `sections_evicted_for_capacity()` /
  `schedules_evicted_for_capacity()` expose the eviction counts).
  `epg::EpgStore`'s default/`with_max_services` now derive the underlying
  collector's logical-key cap from `max_services` (`* 17`, one
  present/following key plus up to 16 schedule table_ids per service) so
  the two caps no longer silently contradict each other (the collector's
  independent 256-key default used to exhaust long before `max_services`
  (1024) did).
- `EitCollector`: a schedule range reset (a non-conformant "flapping
  `last_table_id`" stream) dropped every other already-completed sub-table
  from the schedule, which could then never complete again since those
  sub-tables don't repeat under a new version. The reset now re-feeds
  retained completed sets for the new range.
- `text::decode_dvb_string`: ISO 8859-9 (Turkish) and 8859-11 (Thai) were
  decoded via windows-1254/windows-874, which remap 0x80-0x9F to printable
  characters where the true 8859-9/-11 tables (and DVB Annex A's own
  0x86/0x87/0x8A control codes) leave that range as C1 controls. An
  emphasis marker or CR/LF in Turkish/Thai text came out as a stray glyph
  instead of being recognised.
- `epg::extract_extended`/`EpgEvent`: a bilingual event's
  `extended_event_descriptor` fragments (interleaved in wire order across
  languages) were sorted by `descriptor_number` with no grouping by
  language, mixing two languages' text into one string. Fragments are now
  grouped by `ISO_639_language_code` first; `EpgEvent` gained
  `extended_by_language: Vec<ExtendedEventLanguage>` (`extended_text`/
  `extended_items` are now that list's first entry, not a language-blind
  concatenation of all of them).
- `epg::EpgStore`: `max_services`/`max_events_per_service` silently walled
  off every new key/event once first hit, so a long-running store stopped
  updating. Both caps now evict (least-recently-touched service; the
  earliest-starting event) to admit a new key, exposed via
  `services_evicted_for_capacity()`/`events_evicted_for_capacity()`.
  `feed_sdt` also ignored `max_services` entirely; it now respects it.
- `carousel::ModuleReassembler`: a completed module's slot was removed on
  completion, so the next DII repeat (same version) recreated it and
  reassembled + re-emitted the same module every carousel cycle. A
  `(download_id, module_id) -> completed_version` record now suppresses
  that. Added `retain`/`clear` to prune slots and records for a module
  withdrawn from the carousel.
- `carousel::biop::ior`: `ObjectLocation`/`ConnBinder`/`ServiceLocation`
  components, and `BiopProfileBody`/`LiteOptionsProfileBody` bodies, silently
  dropped trailing slack inside a declared length instead of rejecting it,
  and `Ior::parse` didn't check that `taggedProfiles` consumed the whole
  input. `Binding`/`ServiceGatewayInfo` used `ior.serialized_len()` as a
  stand-in for "bytes consumed", which was only correct because of that
  missing validation. Added a crate-internal `Ior::parse_at` (returns the real consumed
  count) and used it at both call sites; slack is now rejected everywhere.
- `demux::SiDemux`: a changed PAT added new PMT PIDs to the watch set but
  never stopped watching one a programme no longer used, so a stale PMT
  could still be emitted after its PID was reassigned. `follow_pat` now
  diffs the old/new PMT PID sets (never touching a well-known SI PID or a
  caller's explicit `.pid(...)`).
- `dsmcc.rs`: `private_indicator` (independent of
  `section_syntax_indicator`) was forced to 0 whenever SSI was true,
  instead of round-tripping the parsed value.
- `tables::sat`: a beamhopping `plan_length` shorter than the fixed +
  mode-specific fields already read moved the cursor backwards, so the next
  loop iteration re-read part of the current plan's body as a fabricated
  second plan. Now rejected.
- `tables::protection_message`: a `Hash.hash` whose length disagreed with
  `section_hash_length` was written verbatim, misframing every later hash
  entry; `parse_certificate_collection` silently dropped bytes after the
  last certificate. Both now rejected.
- `tables::downloadable_font_info`: `FontInfo::LengthDelimited {
  font_info_type: 0x00..=0x02, .. }` serialized those fixed-layout type
  codes verbatim, so the result re-parsed as StyleWeight/FileUri/FontSize
  instead. Now rejected.
- `descriptors::ait::AitDescriptorLoop`'s serde impl silently dropped parse
  errors from the sequence instead of surfacing them as
  `{"parseError": ...}`, unlike `DescriptorLoop`'s own serde impl.
- `descriptors::hierarchy::HierarchyType::from_u8` (public) panicked
  (`unreachable!()`) for any value over 15; it now masks to the low 4 bits
  instead.
- `descriptors::ca::CaDescriptor`: `ca_pid` (a 13-bit field) was not
  range-checked on serialize, so an out-of-range value (e.g. `0x2101`)
  silently wrapped into a different, wrong PID (`0x0101`) instead of being
  rejected.
- `descriptors::data_broadcast_id::IdSelector::serialize_into_at` and
  `descriptors::extension::dts_hd::SubstreamInfo::serialize_into` panicked
  (out-of-bounds slice index) on a buffer too small for `pos +
  serialized_len()`; both now return `Error::OutputBufferTooSmall`.
- Descriptors whose syntax has no length-delimited tail accepted trailing
  bytes and silently dropped them on re-serialize, so a non-conformant
  stream did not round-trip byte-identically: `application_usage`,
  `simple_application_boundary` (bytes past the declared extension count),
  `AVC_timing_and_HRD` (bytes after its flags byte), `AVC_video`,
  `data_broadcast` and `extended_event` (bytes after `text_length`),
  `xait_location`, `MPEG-2_AAC_audio`/`MPEG-4_audio`/`MPEG-4_video`, and the
  `xait_pid`/`service_relocated` extension bodies now reject the extra bytes
  instead (r03-W9, #1077). `DTS-HD`'s `substream_length` is now read as the
  bound on its substream, so an asset can no longer walk out of it; the
  serializer also wrote `substream_length` one byte short, so a
  re-serialized substream rejected its own assets.
- `HEVC_video_descriptor` treated its mandatory byte 12 (Table 2-113) as
  optional: a 12-byte body was accepted with the flags defaulted and then
  re-serialized as 13 bytes, and a 13-byte body with no temporal sub-block
  while `temporal_layer_subset_flag` was set was accepted too. Both are now
  rejected, and the body length must agree with the flag (r03-W12, #1077).
- `transport_protocol_descriptor`'s HTTP selector decoder silently truncated
  on malformed input and still reported `Http` with the URLs read so far,
  unlike its object-carousel decoder which falls back to `Unknown`; a
  truncated selector is now also `Unknown`, so "complete" and "truncated"
  are distinguishable (r03-W14, #1077).
- `FmxBufferSize_descriptor` split its body with an unsourced
  `descriptor_length % 4` heuristic, so a body too short to hold the
  mandatory `DefaultFlexMuxBufferDescriptor()` was reported as "default
  absent" and a partial trailing entry was accepted as entries. The split is
  now structural against a recorded transcription of ISO/IEC 14496-1 §11.2
  (`docs/descriptors/iso_14496_1/11_2-flexmux-buffer-descriptors.md` — a
  secondary source, since that clause is not vendored), and both invalid
  shapes are rejected (r03-W15, #1077).
- Section-header bit masks (`reserved(2)='11'`, `reserved(4)='1111'`,
  `reserved(3)='111'`, the 12-bit `section_length` high nibble, the
  `reserved_future_use(5)` fill before RST's `running_status(3)`, and the
  `private_indicator` byte-1 composition) were bare hex literals repeated in
  almost every table parser and serializer. They are now named constants and
  helpers in `tables/mod.rs` (`private_section_b1`, `RESERVED_FUTURE_USE_5`,
  `LOW_NIBBLE_MASK`, …), so a mask change is one edit rather than a
  crate-wide grep; a source-scan drift guard fails CI if a new mask literal
  appears outside that module (r02-W16, #1077).
- The crate-root RFU policy now states the flip side explicitly: reserved
  bits are dropped on parse (only spec-meaningful bits are stored), so
  re-serializing a stream with non-standard reserved-bit values normalizes
  them and is not byte-identical by design (r03-W8, #1077).
- ISO/IEC 13818-1 reserved bits are now emitted as '1's, matching the
  existing `ca`/`ac3` policy instead of contradicting it: hevc_video byte 12
  `reserved_future_use`, hierarchy layer/embedded/channel index padding, and
  the trailing reserved runs of audio_stream and avc_video byte 3; parsers
  already ignore those bits, so previously parsed streams still round-trip
  (r03-W7, #1077).
- Serializers that reserved bytes from a flag/depth field but wrote them from
  an `Option` no longer return `Ok(len)` with stale buffer bytes when the two
  disagree: hevc_video `temporal_sub` presence must match
  `temporal_layer_subset_flag`, CPCM USI activation flags must have their
  values, T2 cells must carry a frequency word, and target_region_name now
  sizes each region from the codes actually present (r03-W6, #1077).
- Silent numeric truncation in serializers is now an error: over-wide
  `MB_buffer_size`/`TB_leak_rate` (24-bit), `maximum_bitrate` (22-bit),
  `logical_channel_number` (10-bit), `country_region_id` (6-bit),
  hevc_video `profile_space`/`profile_idc`/`temporal_id`, network_change_notify
  `start_time_of_change` (40-bit) / `change_duration` (24-bit) /
  `receiver_category` (3-bit) / `loop_length`, service_prominence
  `sogi_priority` (12-bit) and its loop lengths, video_depth_range
  disparity hints outside the 12-bit two's-complement range, cp `CP_PID`
  (13-bit), and dts_hd substreams with more than 8 assets now return
  `FieldOverflow`/`ValueOutOfRange` instead of writing a corrupted byte
  (r03-W3, #1077).
- Sub-byte `Reserved(v)` enum payloads and raw multi-bit fields are now
  masked at the shift when composing a byte, so a user-constructed wide
  payload (e.g. `Bandwidth::Reserved(0xFE)`) can no longer bleed into the
  neighbouring bit field; covers terrestrial/cable delivery, video_stream,
  component, content, hierarchy, content_identifier, linkage, hevc_video,
  and the SH/T2/S2x/DTS-UHD/DTS-HD/image-icon/CPCM extension bodies
  (r03-W2, #1077).
- MPE-FEC and MPE-IFEC each carried their own copy of the identical 32-bit
  `real_time_parameters` bit packing; both `RealTimeParameters` structs now
  delegate to one shared bit codec, so the two can never drift (r02-W22).
- `carousel::biop::message::decompress_zlib` now caps decompressed output at
  `MAX_DECOMPRESSED_MODULE_SIZE` instead of reading a
  `compressed_module_descriptor` zlib stream to completion unconditionally —
  a small compressed stream of highly repetitive bytes could previously force
  an allocation orders of magnitude larger than the input.
- BIOP/IOR wire-length arithmetic (`carousel/biop/message.rs`,
  `carousel/biop/ior.rs`) now adds 32-bit wire-declared lengths to a cursor
  via checked addition instead of plain `+`. On a 32-bit target an oversized
  length (e.g. `messageBody_length = 0xFFFFFFFF`) could wrap `usize` rather
  than exceed it, defeating the bounds check that followed; 64-bit targets
  were not affected. Oversized lengths now return `Error::SectionLengthOverflow`.

### Security
Includes fixes for privately reported security issues; no public advisory is published.

## [10.0.1] - 2026-08-30

### Fixed
- **#971**: EIT schedule collector never completed on sparse broadcasts.
  `segment_last_section_number` was parsed but not used in the completion
  check — the collector demanded all `last_section_number + 1` sections
  even when a sparse 3-hour segment declared only its first section present
  (EN 300 468 §5.2.4). Completion now tracks each 8-section segment's own
  declared range independently.

## [10.0.0] - 2026-08-11

### Fixed
- `PmtSection`/`SdtSection`/`AitSection` parsing now rejects trailing slack
  after the last entry in the elementary-stream loop (`PmtSection`,
  ISO/IEC 13818-1 §2.4.4.8), the service loop (`SdtSection`, ETSI EN 300 468
  §5.2.3), and the application loop (`AitSection`, ETSI TS 102 809 §5.3.4),
  matching the post-loop check `EitSection` already had. Previously, a
  section whose final entry was truncated below its declared header length
  parsed "successfully" with that entry silently absent, and re-serialising
  such a section dropped the trailing bytes (parse -> serialize was not
  byte-identical). `AitSection` additionally now rejects an
  `application_loop_length` that undershoots the bytes actually available
  before `CRC_32` — the same silent-drop shape one level up, since nothing
  else follows the application loop in the section. **Behaviour change:**
  input that previously parsed now returns `Error::BufferTooShort` in these
  cases. No real broadcast capture in this crate's fixture suite
  (`dvb-si/tests/fixtures/`, `fixtures/dvb-si/`) is affected — verified by
  running the full fixture test suite after the change.

### Changed
- `tables::sat`'s private `BitReader::read_u` now delegates its bit
  extraction to `broadcast_common::bits::BitReader` (a duplication-audit
  finding: this crate already depends on `broadcast-common` and its
  `dvb-t2mi`/`rdd29`/`st291` siblings already reuse the shared reader). Same
  bounds-checking and error shape as before; no behaviour change. The
  companion `BitWriter` stays hand-rolled, with a comment explaining why:
  the shared writer validates value width and errors instead of truncating,
  which several call sites here rely on truncating silently — swapping it
  in would be a behaviour change this pass must not make.
- MSRV raised to **1.95.0** (issue #949). This removes the workspace's MSRV
  split: `webrtc-runtime`'s optional `media` feature needed rustc 1.88 (via
  `rcgen`), which had grown a dedicated CI job, six `--exclude` lanes and a
  guard script to contain. Adopting let-chains and `is_multiple_of` where the
  1.95 lints require them; no functional or API change.

## [9.2.0] - 2026-08-05
Lockstep minor alongside `dvb-conformance` 9.2.0; no functional changes of
this crate's own.

## [9.1.1] - 2026-07-30

### Fixed
- Floor `mpeg-ts` to `0.3.1`. The `^0.3` bucket also contains 0.3.0, which is
  built against `broadcast-common` 8, so a consumer could resolve two
  `broadcast-common` majors into one graph and hit trait-resolution errors
  pointing at this crate's internals (#858).

## [9.1.0] - 2026-07-30
Lockstep release alongside `dvb-conformance` 9.1.0; no functional changes.

## [9.0.0] - 2026-07-27
### Changed (Breaking)
- Lockstep major bump alongside `broadcast-common` 9.0.0, whose `Encrypt::encrypt`
  now takes `&mut self` instead of `&self` (needed so a stateful implementor can
  own a running per-key IV counter — see `broadcast-common`'s own CHANGELOG for
  the full rationale and the migration note for external `impl Encrypt`s).
  `dvb-si` itself does not implement `Encrypt`/`Decrypt` and has no functional
  or public API change of its own in this release.

### Changed
- Internal, test-only: replaced byte-char array literals with byte strings
  (`LangCode([b'G', b'B', b'R'])` -> `LangCode(*b"GBR")`, and one in
  `ttml_subtitling`'s tests) to satisfy `clippy::byte_char_slices` on the
  pinned canary toolchain. No public API or behaviour change.

## [8.5.0] - 2026-07-21
### Changed
- Widen the internal `mpeg-ts` dependency to `0.3` (issue #663; private
  dependency, used only behind the `ts` feature — no public API change to
  `dvb-si`).

## [8.4.0] - 2026-07-03
### Changed
- Rust **edition 2024**; MSRV raised to **1.86**; format-argument modernisation. No functional or API change.

## [8.3.0] - 2026-07-03

### Added
- `StreamType::is_video()`, `is_audio()`, `is_subtitle()`, `is_data()` predicate
  methods on the PMT stream_type enum (ISO/IEC 13818-1 Table 2-34 classification).
  `PesPrivateData` (0x06) conservatively classifies as `is_data`; descriptor-loop
  inspection is required to resolve the true content type (ETSI TS 101 154).
  Closes #522.

## [8.2.1] — 2026-07-02

### Changed
- Lockstep release tracking `broadcast-common` 8.2.1 (mux-trait documentation). No API/behaviour change to this crate.

## [8.2.0] — 2026-07-02

### Changed
- Lockstep release tracking `broadcast-common` 8.2.0 (new `mux` container-mux
  traits). No API or behavioural change to this crate.

## [8.1.0] — 2026-06-29

### Changed
- Lockstep release. Internal dependency `dvb-common` renamed to `broadcast-common`; no API change.

## [8.0.0] — 2026-06-27

### Removed (BREAKING)
- The MPEG-2 TS framing layer has moved to the new `mpeg-ts` crate.
  Replace `dvb_si::ts::*`, `dvb_si::section::*`, `dvb_si::resync::*`,
  `dvb_si::mux::*`, and `dvb_si::pid::*` with the equivalent items from
  `mpeg_ts`. All PSI tables (PAT/PMT/CAT/TSDT, NIT/SDT/EIT/TDT/TOT/UNT/…)
  and all descriptors remain in `dvb-si`. The `SiDemux` now depends on
  `mpeg-ts` internally via the `ts` feature.

## 7.9.0 — 2026-06-22

### Added
- **Typed CPCM USI decode** — `CpcmDeliverySignalling::usi()` returns
  `Option<Result<CpcmUsi>>` for `cpcm_version == 1`, decoding the 3-byte
  flag prefix and all conditional fields (view window, view period, simultaneous
  view count, remote access date, CPS vector) per ETSI TS 102 825-4 §5.4
  Table 8.  New public types: `CpcmUsi`, `CopyControl` (Table 9),
  `MoveCopyPropagation` (Table 10), `ViewPropagation` (Table 11),
  `CpsVectorEntry`.  Symmetric `Parse`/`Serialize` with byte-exact round-trip.
  The existing `selector_bytes` raw field and TSDuck interop test are
  unchanged.

## 7.8.0 — 2026-06-21

### Added
- **association_tag_descriptor (tag 0x14)** — ISO/IEC 13818-6 §11.4.1 / ETSI
  TR 101 202 §4.7.7.2 Table 4.18. Carried in the PMT `ES_info` loop to bind a
  16-bit `association_tag` to an elementary stream (the DSM-CC cousin of the
  8-bit `stream_identifier_descriptor`). Typed `transaction_id()` / `timeout()`
  accessors decode the `use == 0x0000` (DSI-with-IOR) selector. Verified against
  a real ETSI Hot Bird MHP capture. Previously fell through to
  `AnyDescriptor::Unknown`.

## 7.7.1 — 2026-06-21

### Changed
- Value-enum accessors (`to_u8`/`to_u16`/`to_u32`) are now `const fn`, so they can be used in `const` initializers downstream. Non-breaking.

## 7.7.0 — 2026-06-20

### Added
- **DTS-HD extension descriptor (tag_extension 0x0E)** — ETSI EN 300 468 Annex G.3
  Tables G.6–G.10; typed `substream_info()` blocks with `SamplingFrequency`,
  `AssetInfo` (optional `component_type` and `ISO_639_language_code`).
- **DTS-Neural extension descriptor (tag_extension 0x0F)** — ETSI EN 300 468
  Annex L.1 Table L.1; `config_id` + `additional_info`.
- **DTS-UHD extension descriptor (tag_extension 0x21)** — ETSI EN 300 468
  Annex G.5 Tables G.15–G.17; `DecoderProfileCode`, `FrameDurationCode`,
  `MaxPayloadCode`, `StreamIndex` + `codec_selector`. New public enums
  `FrameDurationCode` (Table G.16) and `MaxPayloadCode` (Table G.17).
  Previously preserved as `Raw`; now typed. TSDuck interop fixture
  (`tsduck-dts-descriptors-pmt.bin`) cross-validates all three (#263).

## 7.6.0 — 2026-06-20

### Added
- **cpcm_delivery_signalling extension descriptor (tag_extension 0x01)** — ETSI
  TS 102 825-9 §4.1.5 Table 2; `cpcm_version` + version-dependent CPCM USI
  `selector_bytes` (v1 selector content per TS 102 825-4). Previously `Raw`; now
  typed (`ExtensionBody::CpcmDeliverySignalling`). TSDuck interop fixture
  (`tsduck-cpcm-delivery-signalling-pmt.bin`) cross-validates it (#261).

## 7.5.0 — 2026-06-19

### Added
- **protection_message extension descriptor (tag_extension 0x18)** — ETSI TS 102 809
  §9.3.3 Table 40; the protected `component_tag` list. Previously preserved as
  `Raw`; now typed (`ExtensionBody::ProtectionMessage`) + `ExtensionTag::ProtectionMessage`.
  TSDuck interop fixture (`tsduck-protection-message-pmt.bin`) cross-validates it (#258).
- `examples/`: `build_and_parse_pat` (build → serialize → parse a PAT) and
  `list_services` (demux a capture, list services + table sections).

## 7.4.0 — 2026-06-18

### Added
- **NorDig private logical-channel descriptors (#239)**, from NorDig Unified
  Requirements v3.1.1 §12.2.9: `NordigLogicalChannelV1` (tag 0x83, **14-bit** LCN —
  distinct from the EACEM/D-Book 10-bit form on the same tag) and
  `NordigLogicalChannelV2` (tag 0x87, channel-list grouped with Annex-A names +
  per-list country_code and 10-bit per-service LCN). Register both via
  `DescriptorRegistry::with_nordig_lcn()` under `private_data_specifier` 0x00000029.
- **DVB extension-descriptor mop-up (#238)**: `CP` (ext-tag 0x02), `CP_identifier`
  (0x03) and `CI_ancillary_data` (0x14), from ETSI EN 300 468 §6.4 — now typed
  `ExtensionBody` variants. (`cpcm_delivery_signalling` 0x01 and DTS-HD/DTS-Neural
  0x0E/0x0F are deferred — their syntax is defined outside EN 300 468 and not yet
  vendored.)
- **MPEG-2 codec descriptors (#237)**, from Rec. ITU-T H.222.0 (06/2021) §2.6:
  `MPEG-4_video` (0x1B), `MPEG-4_audio` (0x1C), `AVC_video` (0x28),
  `AVC_timing_and_HRD` (0x2A), `MPEG-2_AAC_audio` (0x2B) and `HEVC_video` (0x38) —
  now typed `AnyDescriptor` variants (previously unknown). Adds the
  `AacAdditionalInfo` (Table 2-95) and `HdrWcgIdc` (Table 2-114) enums. Validated
  by an interop test that decodes a PMT compiled by TSDuck (`tstabcomp`) — the
  reference DVB toolkit — confirming byte-exact agreement (incl. the HEVC
  `copied_44bits` field and the `HDR_WCG_idc` bit position).
- **MPEG-2 systems descriptors — fixed/opaque set (#237)**: `IOD` (0x1D),
  `SL` (0x1E), `FMC` (0x1F), `External_ES_ID` (0x20), `Muxcode` (0x21),
  `FmxBufferSize` (0x22), `MultiplexBuffer` (0x23), `metadata_STD` (0x27),
  `FlexMuxTiming` (0x2C), `auxiliary_video_stream` (0x2F), `SVC_extension` (0x30)
  and `MVC_extension` (0x31). Opaque MPEG-4-systems sub-structures
  (`InitialObjectDescriptor`, `MuxCodeTableEntry`, FlexMux buffer descriptors,
  `si_rbsp`) are preserved as borrowed byte slices.
- **MPEG-2 systems descriptors — conditional set (#237)**: `content_labeling`
  (0x24), `metadata_pointer` (0x25), `metadata` (0x26) and `J2K_video` (0x32),
  with their nested optional blocks modelled as `Option` sub-structs and the
  `ContentTimeBaseIndicator`, `MetadataFormat`, `MpegCarriageFlags` and
  `DecoderConfigFlags` enums. Each conditional branch has its own round-trip test.
- **MPEG-2 systems descriptors 0x02–0x12 (#236)**, transcribed from Rec. ITU-T
  H.222.0 (06/2021) | ISO/IEC 13818-1:2021 §2.6 (`dvb-si/docs/descriptors/iso_13818_1/`):
  `video_stream` (0x02), `audio_stream` (0x03), `hierarchy` (0x04),
  `target_background_grid` (0x07), `video_window` (0x08), `system_clock` (0x0B),
  `multiplex_buffer_utilization` (0x0C), `copyright` (0x0D), `maximum_bitrate` (0x0E),
  `smoothing_buffer` (0x10), `STD` (0x11) and `IBP` (0x12) — each now decoded as a
  typed `AnyDescriptor` variant (previously fell through to the unknown arm).
  Adds the `FrameRateCode` (Table 2-47) and `HierarchyType` (Table 2-50) enums
  with the standard `name()` + `Display` label pair.

## 7.3.0 — 2026-06-17

### Added
- **Deferred DVB-MHP descriptors (#227)**, transcribed from ETSI TS 102 727 V1.1.1
  (`dvb-si/docs/ts_102_727_mhp.md`):
  - **AIT application-descriptor namespace**: `dvb_j_application` (tag 0x03,
    Table 84 — length-prefixed parameter strings) and `dvb_j_application_location`
    (tag 0x04, Table 85 — base directory / classpath extension / initial class),
    now decoded as the typed `AnyAitDescriptor::DvbJApplication` /
    `DvbJApplicationLocation` variants (previously `Unknown`).
  - **Extension descriptor** `xait_pid` (`descriptor_tag_extension` 0x0C,
    Table 95 — 16-bit `xait_PID`), now `ExtensionBody::XaitPid` /
    `ExtensionTag::XaitPid` (previously `Raw`).
  - Verified against a real Hot Bird MHP mux (`hotbird-mhp` capture).

## 7.2.0 — 2026-06-16

### Fixed
- Defensive hardening of cursor byte-readers: open-ended `slice[pos..]` reads
  feeding `split_first_chunk`/`split_last_chunk` are now `slice.get(pos..)`, so a
  wire-derived offset past the buffer end returns `BufferTooShort` instead of
  panicking (#217). No behaviour change for well-formed input.

### Added
- **Context-aware descriptor decoding (#211).**
  - **AIT application-descriptor namespace** (ETSI TS 102 809 §5.3): `AnyAitDescriptor`
    + `parse_ait_loop` + `AitDescriptorLoop` decode the AIT loops, where low tag
    values mean different things than the EN 300 468 SI namespace. Typed modules:
    `application` (0x00), `application_name` (0x01), `transport_protocol` (0x02,
    with typed OC/HTTP `selector()`), `external_application_authorisation` (0x05),
    `simple_application_location` (0x15), `application_usage` (0x16),
    `simple_application_boundary` (0x17). New `AitApplication::ait_descriptors()`
    and `AitSection::common_ait_descriptors()` accessors walk the loops in this
    namespace. (`dvb_j` 0x03/0x04 deferred — MHP.)
  - **PDS-scoped logical_channel**: `DescriptorRegistry::with_logical_channel_for_pds(pds)`
    dispatches the private `logical_channel` (0x83, LCN) to the typed
    `AnyDescriptor::LogicalChannel` only when the active `private_data_specifier`
    (0x5F) matches; new `PDS_EACEM` / `PDS_NORDIG` constants.

## 7.1.0 — 2026-06-15

### Changed
- Internal: byte-readers hardened with bounds-checked `split_first_chunk` /
  `split_last_chunk` / `first_chunk` (and fixed-size descriptor-entry loops with
  `chunks_exact`), folding manual length checks into the read. DSM-CC/BIOP
  windowed reads continue to bound-check against the declared message/object
  window. No API or behaviour change (#207).
- Internal: exclusive-range match patterns in coded-field decoders (Rust 1.81
  idiom cleanup) (#209).

## 7.0.0 — 2026-06-14

**BREAKING.**

### Added
- **BIOP stream + carousel typing**: `StreamMessage`/`StreamEventMessage`
  (Tables 4.11/4.13), typed `serviceContextList` (`ServiceContext`), and the
  `carousel_identifier_descriptor` (tag 0x13, Tables 4.17/4.17a).
- **no_std + alloc** (#63): `#![no_std]` without the default `std` feature
  (HashMap→BTreeMap, Arc→alloc).

### Changed (breaking)
- **`BiopMessage::Stream`/`StreamEvent`** now carry typed messages, not `&[u8]`.
- **`serviceContext`** fields are `Vec<ServiceContext>`, not `&[u8]`.
- **Coded fields are now typed enums** (#210): `binding_type`→`BindingType`,
  `application_type`→`ApplicationType`, `update_type`→`UpdateType`,
  `icon_transport_mode`→`IconTransportMode`, `identifier_type`→`IdentifierType`,
  `change_type`→`ChangeType`, `range_type`→`RangeType`, `time_plan_mode`→`TimePlanMode`.
- **Every public enum is `#[non_exhaustive]`** (#208).
- MSRV **1.81**.

## 6.7.0 — 2026-06-14

### Added
- `name()` + `Display` on every public spec/field enum (70 enums) via
  `impl_spec_display!`; 15 delivery-system / PMT / frequency-list field enums
  (`Bandwidth`, `Constellation`, `Hierarchy`, `CodeRate`, `GuardInterval`,
  `TransmissionMode`, `Modulation`, `FecOuter`, `Polarization`, `ModulationType`,
  `ModulationSystem`, `RollOff`, `ServiceType`, `StreamType`, `CodingType`)
  gained both. `label_coverage` drift-guard test (#204).

## 6.6.0 — 2026-06-14

### Added
- **BIOP object-carousel layer** (`carousel::biop`) — ISO/IEC 13818-6 §11 as
  profiled by TR 101 202 (#64). Parses the BIOP messages inside reassembled
  carousel modules into a virtual file tree: `Ior` + tagged profiles (BIOP /
  Lite Options, CORBA-CDR, big-endian, DVB alias type_ids), `ObjectLocation` /
  `ConnBinder` / `Tap` / `NsapAddress` / `ServiceLocation`; `BiopMessage`
  (Directory / File / ServiceGateway / Stream / StreamEvent), `ModuleInfo`
  (with `compressed_module_descriptor` detection), `ServiceGatewayInfo` (the DSI
  `private_data`, deferred from #50), and a `CarouselFs` walker (`resolve` /
  `file_bytes`). All with symmetric Parse/Serialize + round-trip tests, a
  byte-exact live-capture oracle against `m6-single.ts`, and `spec_tables` TOML
  drift-guards. New optional **`flate2`** feature (off by default) for
  transparent zlib module decompression — default and `--no-default-features`
  builds are unaffected.
- Criterion benchmark suite (`benches/si_hot_paths.rs`) covering `SiDemux::feed`
  on both real fixture streams, `AnyTableSection::parse` dispatch, `parse_loop`
  descriptor-loop walk, and `DvbText::decode` — dev-only, no API change (#62).

## 6.5.0 — 2026-06-13

### Added
- `carousel::GroupInfoIndication` + `GroupInfo` — typed parse/serialize for the
  SSU DSI `privateData` field (TS 102 006 §8.1.1 Table 6). Carries
  `NumberOfGroups` entries, each with a `CompatibilityDescriptor`, per-group
  info bytes, and private data bytes. Parse via
  `GroupInfoIndication::parse(dsi.private_data)` (#50).
- `DataBroadcastIdDescriptor::id_selector_decoded()` — decode-on-demand accessor
  returning a typed `IdSelector` (`IdSelector::Ssu` for `data_broadcast_id =
  0x000A` per TS 102 006 §7.1 Table 4, else `IdSelector::Raw`). The
  `id_selector` field stays `&'a [u8]` (raw, byte-identical round-trip) — purely
  additive, zero regression for non-SSU callers (#50).
- `descriptors::data_broadcast_id::SsuIdSelector` + `SsuOuiEntry` + `IdSelector`
  types (with their own `Parse`/`Serialize`) + `DATA_BROADCAST_ID_SSU` (`0x000A`)
  constant (#50).
- `S2Xv2SatelliteDeliverySystem` — typed body for extension tag `0x24`
  (`S2Xv2_satellite_delivery_system_descriptor`, Tables 144a–144c, §6.4.6.5.3).
  All conditional blocks typed: scrambling sequence selector + index (modes 1/2),
  timeslice_number (modes 2/5), channel-bond secondary-system-ID loop,
  superframe block with `SFFI`/`beamhopping_time_plan_id` sub-fields (modes 4/5).
  Trailing `reserved_zero_future_use` bytes preserved verbatim in `reserved_tail`.
  `S2Xv2Mode` enum (Table 144c); `S2Xv2Superframe` inner struct. (#53)
- Updated Group (c) deferral comment in `descriptors/extension/mod.rs` to
  explicitly list specs-not-yet-vendored items; removed stale `0x24` deferred entry.

## 6.4.0 — 2026-06-13

### Changed (BREAKING)
- `PmtSection` now exposes `section_number: u8` and `last_section_number: u8`
  fields (ISO/IEC 13818-1 §2.4.4.8 wire bytes `[6]`/`[7]`). Both shall be
  `0x00` for conformant PMTs but are now parsed and preserved for round-trip
  fidelity instead of being silently discarded on parse and hardcoded to zero
  on serialize (#181).
- `PmtSection` is now `#[non_exhaustive]` — struct literal construction outside
  this crate requires the `..` rest pattern or a builder; future field additions
  will not break downstream code (#181).

### Added
- `DescriptorTag` now implements `TryFrom<u8>` (via `num_enum::TryFromPrimitive`),
  matching `TableId`. Additive — converts a known descriptor-tag byte to the
  typed value (`Err` for unallocated tags).
- Spec-table data mirrors under `dvb-si/spec_tables/` (`table_id.toml`,
  `descriptor_tag.toml`, `stream_type.toml`) — reviewable, spec-cited
  value→name tables — plus `tests/spec_drift.rs` drift tests that byte-sweep
  each enum and fail CI if the Rust enum and its TOML ever diverge (#158).
- Extended `spec_tables/` drift coverage to 30 additional code-backing enums:
  `ServiceType`, `SubtitlingType`, `TeletextType`, `LinkageType`, `AudioType`,
  `AnnouncementType`, `AlignmentType`, `Ac3ServiceType`, `Ac4ChannelMode`,
  `CridType`, `ControlRemoteAccess`, `ScramblingMode`, `Polarization`,
  `FecOuter`, `TsGsMode`, `S2XMode`, `T2SisoMiso`, `ShDiversityMode`,
  `C2TuningFrequencyType`, `UriLinkageType`, `ExtensionTag`, `RunningStatus`,
  `IntActionType`, `UntActionType`, `ControlCode`, `ReferenceType`,
  `CridAuthorityPolicy`, `LinkType`, `DescriptorType`, `TvaRunningStatus` (#158).

## 6.3.0 — 2026-06-13

### Added
- `resync::TsResync` (feature `ts`) — stateful 188/204-byte TS byte-stream
  resynchroniser: requires 5 consecutive stride-aligned sync bytes to lock,
  detects 204-byte Reed-Solomon-coded packets and strips the 16 parity bytes,
  and reports packets/resyncs/dropped-byte stats (#61). ISO/IEC 13818-1 §2.4.3.2.

## 6.2.0 — 2026-06-13

### Changed
- The `si_dump` example has moved into the new `dvb-tools` binary crate as
  `dvb-tools dump` (#59). The example file and its `[[example]]` manifest
  entry are removed; behaviour is unchanged.

## 6.1.0 — 2026-06-12

The SI **output half** — the crate can now emit a transport stream, not just
parse one (#56) — plus a spec-transcription / coded-enum correctness pass and a
pre-release audit.

> Breaking-by-intent in a minor (6.0.0 had no released consumers): the
> **fabricated** `ScramblingMode` 0x04/0x05 variants are removed (EN 300 468
> Table 87 reserves them), and `DataStreamAlignmentDescriptor.alignment_type`
> becomes the typed `AlignmentType` (was raw `u8`; `.alignment()` removed).

### Changed
- `ca_system_name` / `private_data_specifier_name` lookups are now generated at
  build time from vendored TSDuck `.names` data (#141) — same
  `Option<&'static str>` signature, fuller + drift-free coverage, attribution in
  `registries/README.md`.
- **`DataStreamAlignmentDescriptor.alignment_type` is now typed `AlignmentType`**
  (was raw `pub u8`); `AlignmentType` is `#[non_exhaustive]` with a `Reserved(u8)`
  catch-all (was a closed enum + `Option` accessor), so reserved bytes round-trip
  losslessly. The now-redundant `.alignment()` accessor is removed. (Breaking;
  pre-release audit follow-up.) `ServiceType` 0x11 name corrected to the verbatim
  spec "HD digital television service".

### Added
- **`mux::SectionPacketizer`** (#56, feature `ts`) — packs serialized PSI/SI
  sections into 188-byte TS packets: PUSI + `pointer_field` placement, section
  concatenation, per-PID continuity counters, 0xFF tail stuffing. The byte-exact
  inverse of `SectionReassembler` (ISO/IEC 13818-1 §2.4.4). Buffer-reuse
  `packetize_into` + allocating `packetize`.
- **`mux::SiMux`** (#56, feature `ts`) — section-repetition scheduler on a
  caller-supplied clock (no clock dependency). Per-PID entries of
  `(sections, interval)`; `poll(now)` emits packets for every entry whose
  interval has elapsed, with continuous per-PID continuity. Spec-cited default
  intervals (`*_MAX_INTERVAL`): NIT/BAT/SDT/EIT/TDT/TOT from TR 101 211
  §4.4.1/§4.4.2, PAT/PMT 100 ms from TS 101 154 §4.1.7, and the 25 ms
  inter-section floor (`MIN_SECTION_INTERVAL`, EN 300 468 §5.1.4.1).
- service_type 0x20 (HEVC UHD), audio_type 0x04–0x7F user-private + 0x80–0x84
  (Primary/Native/Emergency/Primary commentary/Alternate commentary), subtitling
  0x15/0x25 (plano-stereoscopic HD).

### Fixed
- **`ts::SectionReassembler`** no longer drops a valid near-maximal section
  (~4048–4096 B) whose final continuation packet is padded with `0xFF` stuffing
  (#148). A new section cannot start in a continuation packet, so trailing
  stuffing past the section's declared length is now ignored instead of counted
  toward the `MAX_SECTION_SIZE` guard.
- Removed fabricated scrambling_mode 0x04/0x05 ("DVB-CSA3 minimal/fully
  enhanced" — EN 300 468 Table 87 marks 0x04–0x0F reserved); corrected AudioType
  / data_stream_alignment spec citations and two alignment_type names.

## 6.0.0 — 2026-06-11

The "decode completeness" major. 5.0 *typed* every wire field but still handed
back bare integers for spec-enumerated codes (running_status, service_type,
stream_type, content genre, …) — forcing every consumer to re-implement the ETSI
lookup tables. 6.0 decodes them in the library: single coded fields become typed
enums (lossless via a `Reserved(u8)`/`UserDefined(u8)` variant, so round-trip is
byte-identical), and multi-field / registry values gain decode accessors. See
[`docs/release-notes/v6.0.0.md`](../docs/release-notes/v6.0.0.md).

### Changed (breaking) — coded fields are now typed enums
Each enum carries `from_u8`/`to_u8` (or `from_u16`) + `name()`, with unknown wire
values preserved in a catch-all variant for byte-identical serialization.
- **`RunningStatus`** (EN 300 468 Table 6) on `SdtService`, `EitEvent`,
  `SitService`, `RstEvent`, the `collect` views, and `epg::EpgEvent`.
- **`ServiceType`** (Table 87) on `ServiceDescriptor` and `ServiceListEntry`.
- **PMT `stream_type` → `StreamType`** (ISO/IEC 13818-1 Table 2-34: MPEG-2 video,
  H.264, HEVC, AAC ADTS/LATM, AC-3, E-AC-3, …).
- **`epg::Crid.crid_type` → `CridType`** (TS 102 323 Table 117) — and the
  previously-**swapped** doc is corrected (0x01 item-of-content, 0x02 series,
  0x03 recommendation).
- Descriptor codes: `SubtitlingType`, `AudioType`, `TeletextType`, linkage
  (`LinkageType`/`HandOverType`/`LinkType`/`TargetIdType`), `ScramblingMode`,
  content-identifier `CridType`, `tva_id` running status, application/announcement
  types, FTA control.
- Delivery-system consistency: satellite `fec_inner` → `FecInner` (shared with
  cable); S2X reuses the satellite `Polarization`/`RollOff` enums; T2/SH/C2/C2-bundle
  coded fields typed (Tables 116–137); AIT/INT/UNT/RNT/RCT/protection-message/SAT
  codes; `compatibility` descriptor/specifier/sub-descriptor types (byte-identity
  preserved — the M6 carousel fixture still round-trips).

### Added — decode accessors (additive)
- **Content genre** (Table 28): `ContentEntry::genre()`/`genre_name()` and the
  same on `epg::ContentNibble`.
- **`epg::Rating::minimum_age()`** (0x01..=0x0F → `value + 3`); the `value` doc is
  corrected (it is the rating code, not the age).
- BCD `frequency_hz()`/`orbital_position_deg()`/`symbol_rate_sps()` on the S2X
  delivery descriptor (parity with its siblings); `_hz()`/`_deg()` accessors on
  `cell_frequency_link`/`cell_list`.
- Component-type decode on AC-3/E-AC-3/AAC; ancillary/adaptation-field bitflag
  decode; DTS rate/surround decode.
- Best-effort, non-exhaustive registry name lookups: `ca_system_name()` and
  `private_data_specifier_name()` (TR 101 162); `data_broadcast_id_name()`.

### Hardening & raw→typed (internal audit pass)
- **Malformed input is rejected, not silently dropped:** the SAT bit reader/writer
  error on over-run (was silent-zero/skip); `PatSection` errors on a partial
  trailing entry; `CatSection::ca_descriptors()` errors on a descriptor overrun.
  Conformant input and all fixtures are unaffected.
- **More raw bytes → typed (raw `pub` fields removed):** `private_data_indicator`
  → `u32`; `default_authority` / `service_identifier` / `telephone` / CIT strings
  → `DvbText`; EIT `start_time()`/`duration()` and TDT/TOT/`local_time_offset`
  decoded accessors (non-`chrono`, so available in every feature config); MPE
  `MacAddress` / `Checksum` newtypes (`Display`).
- `ac4_channel_mode` → enum (EN 300 468 Table D.12); `uri_linkage_type` → enum
  (TS 101 162) with `has_polling_interval()`.
- **DSM-CC SSI=0** sections now carry the verbatim 4-byte checksum instead of
  being force-validated as CRC-32 (ISO/IEC 13818-6); SSI=1 byte-identity preserved.
- Text decoder: an unsupported ISO 8859 part now yields `U+FFFD` rather than
  plausible Latin-1 garbage (ISO 8859-1 itself still decodes correctly).
- `SECTION_B1_FLAGS_SHORT` const replaces the hardcoded `0x70` in the short-form
  tables; `SiDemux` uses `entry()` + a reused scratch buffer (no per-feed alloc).

### Added — body-agnostic descriptor scan (#139)
- `DescriptorLoop::contains_tag(tag)` and `raw_tags()` — walk the TLV structure
  for tag-presence without typed-parsing each body (an empty/truncated body no
  longer hides the tag, e.g. AC-3 `0x6A` / E-AC-3 `0x7A` detection).

### Documentation
- Runnable doctests on the new decode accessors (`RunningStatus`, `StreamType`,
  `ServiceType`, `ContentEntry::genre_name`) and a `dvb-common` crate-root
  quickstart. The `si_dump` example now prints decoded `stream_type` /
  `running_status` names on real captures.

### Note
The symmetric `Parse`/`Serialize` contract is unchanged — **every table already
serializes** (build a `PatSection`/`PmtSection`/`CaDescriptor` and call
`serialize_into`, CRC included); the typed AC-3 (0x6A) and E-AC-3 (0x7A)
descriptors are reachable via `AnyDescriptor`. These are not new in 6.0; they are
now documented in the README.

## 5.0.0 — 2026-06-11

The "type everything and harden" major. Every raw structured loop that was
previously exposed as `&[u8]` is now fully typed; the section parsers are
hardened against attacker-controlled length fields; private table_ids,
descriptors, extension bodies and T2-MI payloads are runtime-registrable; and
the public surface is cleaned up for long-term API stability. This release
bundles ~50 changes and four adversarial audit rounds since 4.3.0 — see
[`docs/release-notes/v5.0.0.md`](../docs/release-notes/v5.0.0.md) for the
narrative and a migration guide.

### Added
- **Runtime extensibility — four registries + un-sealed `*Def` traits.** A
  third-party crate can plug its own wire types into the existing dispatch with
  zero changes here (see [`docs/extending.md`](../docs/extending.md)):
  - `DescriptorRegistry` (PDS-scoped via `register_for_pds`) walked through
    `DescriptorLoop::iter_with` / `iter_with_extensions`; private extended
    descriptors surface as `ExtIterItem::CustomExtension` (#120).
  - `TableRegistry` for private `table_id`s, via `AnyTableSection::parse_with`
    and `SectionEvent::table_section_with` (#121).
  - `ExtensionRegistry` for private `descriptor_tag_extension` bodies (#120).
  - The `TableDef` / `DescriptorDef` / `ExtensionBodyDef` traits are un-sealed;
    registered values are recovered via the inherent `downcast_ref` on the
    `dyn *Object` (correct under both serde and `--no-default-features`) (#123).
- **Six extension descriptors (`descriptor_tag` 0x7F) now typed** instead of
  `ExtensionBody::Raw`: T2-MI (`T2mi`, 0x11), video depth range
  (`VideoDepthRange`, 0x10), VVC subpictures (`VvcSubpictures`, 0x23), image
  icon (`ImageIcon`, 0x00), service prominence (`ServiceProminence`, 0x22), and
  SH delivery system (`ShDeliverySystem`, 0x05) (#82–84, #90–92). The
  `ExtensionBodyDef` trait + macro-driven, drift-tested dispatch underpins them
  (ADR-0001; #86, #88, #110).
- **`compatibility_descriptor()` typed** (`CompatibilityDescriptor`) wherever it
  appears — DSI / DII carousel messages and UNT platform entries — per ISO/IEC
  13818-6 / TS 102 006 Table 15 (#133).
- **`collect/` module** (split from `collect.rs`) with the multi-section table
  collectors and the `epg` EPG extractor (#113).
- **Documentation:** ADR-0001/0002 (dispatch design), a reserved-bit policy at
  each crate root, and `docs/extending.md` ("Adding a parser crate") (#85, #89,
  #104, #134).

### Changed (breaking)
- **Unified cross-crate error model** (#112): structured `thiserror` variants
  consistent across all crates (`BufferTooShort`, `SectionLengthOverflow`,
  `InvalidDescriptor`, …). Error type / variants changed.
- **Raw structured loops are now typed** (replacing public `&[u8]` slices) in:
  `target_region` / `target_region_name`, `network_change_notify`,
  `T2_delivery_system`, `audio_preselection`, `S2X_satellite_delivery_system`,
  `TTML_subtitling` (#96–101); six SI tables (#111); `enhanced_AC-3` (0x7A) and
  `linkage` (0x4A) conditional blocks (#114); `rnt` provider/authority names
  → `DvbText` (#133).
- **`#[non_exhaustive]` sweep** across growth-prone enums, the `collect`
  outputs, and the dispatch enums; matching them now needs a wildcard arm
  (#109, #127, #130).
- **serde key conventions tightened:** `rename_all = "camelCase"` is reserved
  for the top-level dispatch / registry enums only; body enums and structs
  serialize PascalCase variants / snake_case fields. `epg` structs, `IconLocation`,
  `VbiService` changed accordingly (#127, #130, #131, #135).
- **Extension body types renamed** to drop the misleading `Descriptor` suffix:
  `T2mi`, `VideoDepthRange`, `VvcSubpictures` (#135).
- **`ts.rs` adopts the `Parse`/`Serialize` contract** (#117).

### Removed (breaking)
- The dead legacy `dvb_si::traits::Descriptor` and `dvb_si::traits::Table`
  traits (and `descriptor_length()`, `Table::PID`, `Table::TABLE_ID`) — use
  `DescriptorDef::TAG` / `TableDef::TABLE_ID_RANGES` or the module-level consts
  (#135).
- Redundant wire-derived public fields now computed on serialize:
  `RctSection::link_count`, the SAT `ephemeris_data_count` / `metadata_flag`,
  and `SogiEntry::{target_region_flag, service_flag}` (#135).
- `Error::InvalidBcd` (dead) and the unused `DescriptorRegistry.ext` field
  (#127, #131).

### Fixed
- **Section-length underflow DoS** floored across the long-form tables via the
  shared `crate::tables::check_section_length` helper — a `section_length`
  smaller than header+CRC no longer underflows into an out-of-bounds slice /
  panic (#119, #125, #132).
- **`extended_event` parse panic** on a short declared length (direct `Parse`
  call); **SAT `section_length` overflow** now guarded in `usize` before the
  `u16` cast (also `eit`, `rct`) (#135 + follow-up).
- Reserved-bit emission corrected (DVB long-form bits → 1; extension-body
  RFU bits; T2/SH delivery flags) with a byte-identity conformance test against
  real captures (#93, #98, #108); `vbi_data` 0x03 / reserved-bit bugs (#131).
- `EpgStore` / `EitCollector` growth bounded and `now_and_next` ordering
  corrected (#107); `SectionSetCollector` capacity capped (#130).
- Robust registry downcast under `--no-default-features` (the serde-bound
  blanket-impl footgun) (#123); AC-4 toc documented as opaque codec carriage
  (#103).

### Performance
- `TsResync::feed` is now O(n) via a read cursor (#105).

## 4.3.0 — 2026-06-10

Decoded accessors and analysis building blocks across the workspace; all
additive (one perf change, one internal cleanup), no breaking changes.

### Added
- **`epg` module (feature `chrono`)** — an `EpgStore` convenience layer over
  `EitCollector` (#51). Keyed by `(original_network_id, transport_stream_id,
  service_id)`, it maintains a deduplicated, time-ordered event list and decodes
  the commonly needed fields per event: short-event name/text, extended-event
  text concatenated across fragments per EN 300 468 §6.2.15, content genre,
  parental ratings, and CRIDs. `now_and_next(key, at)` returns the on-air and
  next events; `feed_sdt` joins service names from the SDT; `services()`
  enumerates cached services; `retain_services` / `clear` bound long-running
  memory. Serialize-only serde export.
- **`resync` module** — `TsResync`, a byte-stream resynchroniser that recovers
  188-byte TS packet alignment from arbitrary input (junk prefixes, mid-stream
  loss) and detects/strips 204-byte Reed-Solomon packets, with resync/dropped
  stats. Sync byte 0x47 per ISO/IEC 13818-1 §2.4.3.2 (#61).
- **Typed adaptation field + PCR** — `TsPacket::adaptation_field()` decodes the
  discontinuity / random-access / ES-priority flags, PCR/OPCR (`Pcr`, with
  `as_27mhz()`), and splice countdown per ISO/IEC 13818-1:2007 §2.4.3.4 (the
  field was previously skipped) (#48).

### Changed
- `SiDemux::feed` now does a single `pids` map lookup per packet (was 2+N) by
  draining all sections under one borrow then processing them — behaviour
  unchanged (#44).
- Removed the dead write-only `expected` field from `SectionReassembler` and
  corrected its doc comment (no behavioural change) (#46).

## 4.2.0 — 2026-06-09

Hardening of the DSM-CC `ModuleReassembler` against hostile carousel input
(2026-06-09 audit findings, #42 / #43).

### Fixed

- **`ModuleReassembler` slot growth is now bounded** (#42). The aggregate byte
  budget counts only announced `moduleSize` data bytes, so announcements with
  zero (or tiny) `moduleSize` could grow the slot map without limit. Zero-size
  announcements are now rejected (a module with no data has nothing to
  reassemble), and a new slot-count cap (`DEFAULT_MAX_SLOTS`, tunable via
  `ModuleReassembler::with_max_slots`) bounds the map itself.
  `pending_bytes()` docs now state exactly what it counts — data-buffer bytes,
  not total retained memory — and the lossy-stream starvation behaviour of the
  skip-until-space policy is documented on the type.
- **`note_dii` stale-version detection is O(1) per announced module** (#43).
  It previously scanned every in-progress slot for each announced module —
  O(n²) across a DII, a CPU-DoS vector at 65 535 modules per DII. Slots are
  now keyed by `(downloadId, moduleId)` with the version held inside the
  slot, so version replacement is a single keyed lookup. Behaviour is
  unchanged: same-version re-announcements keep accumulated blocks, version
  bumps restart the module, mismatched-version DDBs are ignored.

## 4.1.0 — 2026-06-09

Decoded getters and symmetric `set_*` encoders so consumers stop hand-decoding
BCD/MJD wire fields (#37, #38). No wire-format change; purely additive.

### Added

- **Time:** `EitEvent::duration()` / `set_duration()` (ungated — a duration is
  plain elapsed seconds); `EitEvent::start_time()` gains `set_start_time()`;
  `TotSection::utc_time()` / `set_utc_time()` (the raw field existed but, unlike
  TDT, had no decoder); `TdtSection::set_utc_time()`; and
  `CompleteEitEvent::start_time()` / `duration()` on the collect-layer view.
- **Delivery descriptors:** `SatelliteDeliverySystemDescriptor` /
  `CableDeliverySystemDescriptor` gain `frequency_hz()`, `symbol_rate_sps()`
  (+ satellite `orbital_position_deg()`); `TerrestrialDeliverySystemDescriptor`
  gains `centre_frequency_hz()`; all with `set_*` encoders. Frequencies are
  exposed in **Hz** (`u64`) because cable (100 Hz) and terrestrial (10 Hz) are
  finer than 1 kHz and an integer kHz would silently round.
- **Other BCD/packed fields:**
  `FrequencyListDescriptor::centre_frequencies_hz()` /
  `set_centre_frequencies_hz()` (interpreted per `coding_type`);
  `LocalTimeOffsetEntry::local_time_offset()` / `next_time_offset()` /
  `time_of_change()` with setters (the two offsets share one polarity bit);
  `PdcDescriptor::pil_day()` / `pil_month()` / `pil_hour()` / `pil_minute()` /
  `set_pil()`.
- `Error::ValueOutOfRange` for decoded values a `set_*` accessor cannot encode.

### Changed

- The shared BCD/MJD codec now lives in `dvb-common` (`bcd` / `time`); the
  `chrono` feature enables `dvb-common/chrono`. The MJD/BCD helpers formerly
  duplicated in `tables::eit` and `tables::tdt` are removed in favour of it.

## 4.0.0 — 2026-06-08

A deliberate API break that separates **one wire section** from a **complete
logical table**. In 3.x the single-section parsers carried table names (`Nit`,
`Sdt`, `AnyTable`) even though each returns one PSI/SI section; consumers that
deduped by `version_number` silently dropped every section after the first and
truncated multi-section NIT/BAT/SDT/EIT tables (the live Astra 19.2°E failure in
#32 — half the transponders and whole networks lost). 4.0 renames the
single-section parsers to `*Section`, and adds a `collect` module that assembles
sections `0..=last_section_number` into a complete, owned table. See
[`MIGRATION-4.0.md`](MIGRATION-4.0.md) for every breaking change with
before/after code. (#32)

### Breaking

- **Single-section parser types are now `*Section`.** Every one-section parser
  uses a `Section` suffix and Rust CamelCase acronyms: `Nit` → `NitSection`,
  `Sdt` → `SdtSection`, `Pat` → `PatSection`, … (all 23 — `Ait`, `Bat`, `Cat`,
  `Cit`, `Container`, `Dit`, `Eit`, `Int`, `MpeFec`, `MpeIfec`, `Nit`, `Pat`,
  `Pmt`, `Rct`, `Rnt`, `Rst`, `Sat`, `Sdt`, `Sit`, `St`, `Tdt`, `Tot`, `Tsdt`,
  `Unt`). `MpeDatagramSection`, `DsmccSection`, `ProtectionMessageSection`, and
  `DownloadableFontInfoSection` already had section-shaped names. There are no
  compatibility aliases. See
  [MIGRATION-4.0.md §1](MIGRATION-4.0.md#1-section-parser-types-are-now-section).
- **`AnyTable` → `AnyTableSection`.** The dynamic dispatcher parses exactly one
  complete section; the enum and its variants are renamed accordingly, and
  `parse_as` moves with it. serde variant keys gain the section suffix
  (`AnyTableSection::PatSection` → `{"patSection": …}`). See
  [MIGRATION-4.0.md §2](MIGRATION-4.0.md#2-anytable-is-now-anytablesection).
- **`SectionEvent::table()` → `table_section()`.** Demux still emits changed
  sections, not complete logical tables; feed `event.bytes()` to a collector for
  the whole table. See
  [MIGRATION-4.0.md §3](MIGRATION-4.0.md#3-sectioneventtable-is-now-table_section).
- **`TableId` variants are now CamelCase.** `TableId::PAT` → `TableId::Pat`,
  `TableId::MPE_FEC` → `TableId::MpeFec`, … keeping the byte-value enum visually
  distinct from the parser types. Long semantic variants
  (`NetworkInformationActual`, …) are unchanged. See
  [MIGRATION-4.0.md §7](MIGRATION-4.0.md#7-tableid-variants-use-camelcase).

### Added

- **`collect` module — section → complete multi-section table.** The next rung
  above the single-section parsers: assemble sections `0..=last_section_number`
  of one version into a complete, owned value, fixing the multi-section
  truncation class of bug (#32) once for every consumer.
  - `SectionSetCollector` — feed complete section bytes with `push_section` /
    `push_section_with_pid`; returns `Some(CompleteSectionSet)` only when all
    sections of the current version have arrived (validates the long-form CRC
    before retaining bytes; discards the partial set and restarts on a version
    bump).
  - `CompleteSectionSet` — owns the original section bytes so parsed views keep
    borrowing from them: generic `.table::<T>()` (any long-form section parser,
    in section-number order), `.section_bytes()` for re-serialization, and the
    complete logical helpers `.nit()` / `.bat()` / `.sdt()` / `.eit()` (and
    `_with_registry` variants) producing `CompleteNit` / `CompleteBat` /
    `CompleteSdt` / `CompleteEit` with entries flattened across all sections.
  - `EitCollector` / `CompletedEit` — EIT is complete only when every schedule
    `table_id` through `last_table_id` has completed, not when one `table_id`
    has all its sections. EIT schedule sub-tables version independently, so
    `CompleteEitSchedule` exposes per-`table_id` versions
    (`table_versions()` / `tables()`); `retain_logical` / `clear` prune
    long-running EPG state on the caller's retention boundary.
  - `ParsedDescriptorLoop` — descriptor loops in complete logical views stay
    typed (`.descriptors()` → `&[Result<AnyDescriptor>]`) without losing the
    original wire bytes (`.raw()`).

  See [MIGRATION-4.0.md §4–§6](MIGRATION-4.0.md#4-multi-section-tables-use-collect).

## 3.1.2 — 2026-06-07

### Fixed
- `ts::SectionReassembler::feed` now completes a section that spans **into a
  PUSI packet**. When a section started in packet A and spilled into packet B,
  and B was itself PUSI=1 (new sections begin in it), the `pointer_field` tail
  bytes belonging to A's section were skipped and the buffer cleared — so the
  spanning section was dropped (ISO/IEC 13818-1 §2.4.4: those bytes complete
  the in-progress section before new ones begin). Complements the 3.1.1
  within-payload concatenation fix; together they close all of #29. On a real
  EIT-heavy DVB-T2 capture this recovered emitted sections 51 → 237 (484
  completed), all CRC-valid. (#29)

## 3.1.1 — 2026-06-07

### Fixed
- `ts::SectionReassembler::feed` now extracts **all** sections concatenated in
  a single TS payload, not just the first. Sections packed after the
  `pointer_field` (legal per EN 300 468 §5.1.4; common on EMM PIDs) were
  silently dropped — table-agnostic SI/EMM data loss. Consumers should drain
  with `while let Some(s) = r.pop_section()`. (#29)

## 3.1.0 — 2026-06-05

The 3.x line: descriptor loops become typed, zero-copy `DescriptorLoop`s; serde
goes Serialize-only across the workspace (JSON is a display/export format); the
SIT service loop is typed; `Cat`/`Tsdt`/`Sit` move from owned to borrowed; and a
new optional `yoke` feature lets a parsed view outlive its input buffer. Wire
parsing is byte-identical throughout — only field types and JSON output change.
See [`MIGRATION-3.1.md`](MIGRATION-3.1.md) for every breaking change with
before/after code. (#21, #23, #27)

### Breaking

- **`DescriptorLoop<'a>` replaces raw descriptor-loop byte fields.** Every true
  SI descriptor loop on a table is now `DescriptorLoop<'a>` instead of `&'a [u8]`
  (or `Vec<u8>`). Call `.iter()` to walk it into typed `AnyDescriptor`s, `.raw()`
  for the wire bytes, or rely on `Deref<Target=[u8]>` for length/indexing.
  `parse_loop` is unchanged and still works on free slices.
  Affected fields: `SdtService.descriptors`, `EitEvent.descriptors`,
  `PmtStream.es_info`, `Pmt.program_info`, `NitTransportStream.descriptors`,
  `Nit.network_descriptors`, `BatTransportStream.descriptors`,
  `Bat.bouquet_descriptors`, `AitApplication.descriptors`,
  `Ait.common_descriptors`, `Tot.descriptors`, `Rct.descriptors`,
  `Rnt.common_descriptors`, `Int.platform_descriptors`,
  `Unt.common_descriptors`, `Cat.descriptors`, `Tsdt.descriptors`,
  `Sit.transmission_info_descriptors`.
  See [MIGRATION-3.1.md §1](MIGRATION-3.1.md#1-descriptor-loop-fields-u8--vecu8--descriptorloopa).
- **`Cat`, `Tsdt`, `Sit` are now borrowed (`<'a>`).** They previously owned their
  loops (`Vec<u8>`) and had no lifetime; they now borrow the section bytes like
  every other table. See
  [MIGRATION-3.1.md §2](MIGRATION-3.1.md#2-three-tables-moved-from-owned-to-borrowed).
- **`Deserialize` dropped — serde is Serialize-only.** Every `Deserialize`
  derive/impl is removed (including the manual `text::LangCode` impl) along with
  the now-dead `serde(borrow)` / `serde(bound(deserialize = …))` attributes. JSON
  is a display/export format; reconstruct values by re-`parse`-ing the wire bytes.
  `Serialize` output is unchanged. Affected container structs include `Sdt`,
  `SdtService`, `Eit`, `EitEvent`, `Pmt`, `PmtStream`, `Nit`,
  `NitTransportStream`, `Bat`, `BatTransportStream`, `Ait`, `AitApplication`,
  `Tot`, `Rct`, `Rnt`, `Int`, `Unt`, `Cat`, `Tsdt`, `Sit`. See
  [MIGRATION-3.1.md §3](MIGRATION-3.1.md#3-deserialize-dropped--serde-is-serialize-only).
- **serde JSON shape change.** A descriptor-loop field serializes as an array of
  typed, decoded descriptors (camelCase variant keys) instead of an array of raw
  bytes; per-entry parse errors surface as `{"parseError": "…"}`. See
  [MIGRATION-3.1.md §4](MIGRATION-3.1.md#4-serde-json-shape-change).
- **SIT service loop is typed.** `Sit.service_loop: &'a [u8]` is replaced by
  `Sit.services: Vec<SitService<'a>>`, where
  `SitService { service_id: u16, running_status: u8, descriptors: DescriptorLoop<'a> }`
  — mirroring `SdtService` (EN 300 468 §7.1.2, Table 164). The serialized JSON
  drops the raw `service_loop` byte array in favour of typed `services` entries,
  each with their own typed `descriptors` sequence. See
  [MIGRATION-3.1.md §5](MIGRATION-3.1.md#5-sit-service-loop-is-typed).

### Added

- `descriptors::DescriptorLoop<'a>` — a borrowed, zero-copy descriptor-loop
  newtype (the table-loop analogue of `DvbText`): `new`/`raw`/`iter`,
  `Deref<[u8]>`, `From<&[u8]>`, `IntoIterator`, a cheap `Debug`
  (`DescriptorLoop(<N bytes>)`), and serialize-only serde over the typed walk.
- **`yoke` feature (off by default, additive).** `yoke::Yokeable` is derived on
  every public zero-copy view type — all table views (`Pmt`/`PmtStream`, `Sdt`/
  `SdtService`, `Eit`/`EitEvent`, `Nit`/`Bat`/`Cat`/`Tsdt`/`Sit`/`Ait`/`Tot`/
  `Int`/`Unt`/`Rct`/`Rnt`/`Cit`/`Container`, the DSM-CC/MPE/protection/SAT/font
  sections, …), every borrowing descriptor struct, `DescriptorLoop`, `DvbText`,
  and the `AnyTable` / `AnyDescriptor` enums. A new `owned` module adds
  `Owned<Y>`, a `'static`, `Send + Sync`, cheaply-`Clone` bundle of the backing
  `Arc<[u8]>` and the parsed view: own a parsed table past the input buffer's
  borrow (struct field, cache, `watch`/broadcast channel, cross-thread) without
  re-parsing or a hand-written mirror type. The feature is optional and adds no
  dependencies to default builds. See
  [MIGRATION-3.1.md §6](MIGRATION-3.1.md#6-owning-a-parsed-view-the-yoke-feature). (#27)

## 2.1.0 — 2026-06-05

### Added
- `AnyTable::name()` — macro-generated diagnostic label from each type's
  `TableDef::NAME` (`"PROGRAM_ASSOCIATION"`, …); `"UNKNOWN"` for the
  fallthrough variant. (#18)
- `AnyDescriptor::name()` — same, with `"CUSTOM"` for runtime-registered
  `Other` descriptors.
- `si_dump` example simplified accordingly (the 35-line variant match is gone).

## 2.0.0 — 2026-06-05

Typed, trait-driven client API: feed 188-byte TS packets, get decoded `SectionEvent`s
with structured typed tables and descriptors. See [`MIGRATION-2.0.md`](MIGRATION-2.0.md)
for every breaking change with before/after code. Tracks issue #16.

### Breaking

- **`DvbText<'a>` replaces `&'a [u8]` for Annex-A text fields.** Fields such as
  `event_name`, `service_name`, `provider_name`, `network_name`, and all multilingual
  name fields are now `DvbText<'a>`. Call `.raw()` to recover the wire bytes, `.decode()`
  for a `Cow<str>`, or rely on `Deref<Target=[u8]>` for length/indexing — all work as
  before. See [MIGRATION-2.0.md §1](MIGRATION-2.0.md#1-text-fields-u8--dvbtexta).
- **`LangCode` replaces `[u8; 3]` for language / country codes.** The raw bytes are in
  `.0`; `as_str()` returns a lossy `Cow<str>`; `Deref<Target=[u8;3]>` still works. See
  [MIGRATION-2.0.md §2](MIGRATION-2.0.md#2-language--country-codes-u8-3--langcode).
- **`Deserialize` dropped on text-bearing structs.** Structs that contain `DvbText` now
  derive `Serialize` only (re-encoding decoded UTF-8 back to DVB charset bytes is lossy).
  Affected: `BouquetNameDescriptor`, `ComponentDescriptor`, `DataBroadcastDescriptor`,
  `ExtendedEventDescriptor`, `ExtensionDescriptor`,
  `MultilingualBouquetNameDescriptor`, `MultilingualComponentDescriptor`,
  `MultilingualNetworkNameDescriptor`, `MultilingualServiceNameDescriptor`,
  `NetworkNameDescriptor`, `ServiceDescriptor`, `ShortEventDescriptor`. See
  [MIGRATION-2.0.md §3](MIGRATION-2.0.md#3-deserialize-dropped-on-text-bearing-structs).
- **Subset `Descriptor` enum removed — replaced by `AnyDescriptor` + `parse_loop`.**
  The 1.x `descriptors::Descriptor` enum (a handful of context-free tags) is gone.
  Use `descriptors::parse_loop(loop_bytes)` for a lazy iterator that covers all
  0x05–0x7F tags plus `Unknown` passthrough, and `AnyDescriptor` as the item type. See
  [MIGRATION-2.0.md §4](MIGRATION-2.0.md#4-subset-descriptor-enum-removed--anydescriptor--parse_loop).
- **serde JSON shape changed.** `DvbText` serializes as a decoded UTF-8 string (not a
  byte array); `LangCode` serializes as a 3-char string. `AnyTable` / `AnyDescriptor`
  use external camelCase tagging (`{"shortEvent":{…}}`); inner struct field names stay
  `snake_case`. See
  [MIGRATION-2.0.md §5](MIGRATION-2.0.md#5-serde-json-shape-change).
- **`pid::well_known` constants are now `Pid` values, not `u16`.** Call `.value()` or
  `u16::from(pid)` to recover the raw integer. See
  [MIGRATION-2.0.md §6](MIGRATION-2.0.md#6-pidwell_known-constants-u16--pid).

### Added

- **`demux::SiDemux`** (feature `ts`) — PID-filtered, version-gated, PAT-following
  section pump. Feed 188-byte TS packets with `SiDemux::feed(&packet)`, iterate over
  `SectionEvent` values for changed sections only. Builder: `follow_pat`, `dvb_si_pids`,
  `.pid(Pid)`, `emit_repeats`, `gate_capacity`. Stats in `SiDemux::stats()`.
- **`demux::SectionEvent`** — owning-`Bytes` event: `.pid()`, `.table_id()`,
  `.version()`, `.table() -> Result<AnyTable<'_>>`, `.parse::<T>()`.
- **`tables::AnyTable`** — macro-generated dispatch enum; `AnyTable::parse(bytes)`
  dispatches on `table_id` across all 29 implemented table types; `Unknown` fallthrough.
  Driven by `TableDef` trait + `declare_tables!` macro (single source of truth, drift
  tested).
- **`descriptors::AnyDescriptor`** — macro-generated dispatch enum covering all
  0x05–0x7F tags plus `LogicalChannel` (opt-in via `DescriptorRegistry`) and `Unknown`.
  Driven by `DescriptorDef` trait + `declare_descriptors!` macro.
- **`descriptors::parse_loop`** — lazy `DescriptorIter` over a raw descriptor loop.
  Never panics; per-descriptor parse errors yield `Err` and iteration continues;
  truncated tail yields one `Err` then fuses; unknown tags become `Unknown`.
- **`descriptors::DescriptorRegistry`** — runtime registration of private/context-
  dependent descriptors (`register::<T>()`); `.with_logical_channel()` enables the 0x83
  built-in. `Other { tag, value }` variant supports `downcast_ref::<T>()`; `erased-serde`
  serializes custom types when `serde` feature is active.
- **`pid::Pid`** newtype — `Copy`, `Ord`, `Hash`, `Display` as `0xNNNN`; `well_known`
  constants upgraded to this type.
- **`examples/si_dump.rs`** — CLI demux tool: `cargo run -p dvb-si --example si_dump
  -- file.ts [--json]`. Prints one line per changed section; `--json` emits decoded JSON.
- **End-to-end capture tests** — `tests/demux_e2e.rs` runs the full M6 HbbTV fixture
  through `SiDemux`, asserts typed variant coverage, double-feed produces zero events,
  and EIT descriptor JSON contains decoded text strings.
- **Hostility suite** — `tests/hostility.rs`: 10 000 PRNG-garbage + every truncation
  length into `SiDemux` / `parse_loop` — no panics, counters advance. Issue #16.

## 1.1.0 — 2026-06-04

**Coverage milestone: every allocated `descriptor_tag` in EN 300 468 V1.19.1
Table 12 (0x40–0x7F) is implemented** — 41 new descriptor modules, each with
a symmetric `Parse`/`Serialize` pair, spec-cited layout, and round-trip tests.

### Added

**Descriptors** (EN 300 468 unless noted)
- 0x42 stuffing, 0x45 VBI_data, 0x46 VBI_teletext, 0x49 country_availability,
  0x4B NVOD_reference, 0x4C time_shifted_service, 0x4F time_shifted_event,
  0x51 mosaic (typed cell/elementary-cell loops + cell_linkage variants),
  0x53 CA_identifier, 0x57 telephone, 0x5B–0x5E multilingual
  network_name/bouquet_name/service_name/component, 0x5F
  private_data_specifier, 0x60 service_move, 0x61 short_smoothing_buffer,
  0x63 partial_transport_stream, 0x64 data_broadcast, 0x65 scrambling,
  0x66 data_broadcast_id, 0x67 transport_stream, 0x68 DSNG, 0x69 PDC,
  0x6B ancillary_data, 0x6C cell_list, 0x6D cell_frequency_link,
  0x6E announcement_support, 0x70 adaptation_field_data,
  0x72 service_availability, 0x7B DTS (Annex G), 0x7C AAC (Annex H),
  0x7E FTA_content_management
- 0x6F application_signalling, 0x71 service_identifier (TS 102 809)
- 0x74 related_content, 0x75 TVA_id (TS 102 323)
- 0x77 time_slice_fec_identifier, 0x78 ECM_repetition_rate (EN 301 192)
- 0x7D XAIT_location (TS 102 727, newly vendored)
- 0x7F extension descriptor — typed `descriptor_tag_extension` discriminant
  with 14 typed extension bodies (T2/C2/C2-bundle/S2X delivery systems,
  supplementary_audio, network_change_notify, message, target_region(_name),
  service_relocated, URI_linkage, AC-4, audio_preselection, TTML_subtitling)
  and a raw-preserving fallthrough: unknown tag_extensions round-trip
  byte-exact.

**Text** — full Annex A Table A.3 selector coverage: 0x12 KS X 1001 (EUC-KR),
0x13 GB-2312 (via GBK), 0x14 Big5, 0x1F encoding_type_id escape; two-byte
control codes (U+E080–U+E09F, Table A.2) now honored alongside the
single-byte set.

### Fixed (audit round 5, pre-release)
- extension/S2X_satellite_delivery_system (0x7F/0x17): byte-1 bit layout
  corrected to Table 140 — S2X_mode at bits [7:6] and
  scrambling_sequence_selector at bit [5] (were [4:3] and [2]).
- extension/C2_bundle_delivery_system (0x7F/0x16): bundle entries are 8 bytes
  per Table 139, not 9 — multi-entry descriptors no longer misalign.
- extension/supplementary_audio (0x7F/0x06): flags-byte bit 1 is plain
  `reserved_future_use` → now serialized as 1 per the crate convention.
- Doc-cite corrections: time_shifted_event §6.2.44, time_shifted_service
  §6.2.45, multilingual_component §6.2.23, multilingual_service_name §6.2.25,
  related_content §10.4.1.

## 1.0.1 — 2026-06-04

Docs-only: README rewritten around an explicit implementation matrix — per
table_id status, carousel message coverage, typed-descriptor list, spec
grounding. No code changes.

## 1.0.0 — 2026-06-04

First substantive release covering the MPEG-2 PSI and DVB SI table set, the
common descriptors, and the DVB-allocated companion tables.

### Added

**Framing**
- `Section<'a>` — long/short-form PSI/SI section header with CRC-32 validation
- `TsPacket<'a>` + `SectionReassembler` under feature `ts`

**Tables** (each with `Parse` + `Serialize` round-trip tests)
- MPEG-2 PSI: `Pat` (0x00), `Cat` (0x01), `Pmt` (0x02), `Tsdt` (0x03)
- DVB SI: `Nit` (0x40/0x41), `Sdt` (0x42/0x46), `Bat` (0x4A), `Eit` (0x4E–0x6F),
  `Tdt` (0x70), `Rst` (0x71), `St` (0x72), `Tot` (0x73), `Dit` (0x7E), `Sit` (0x7F)
- `Sat` — Satellite Access Table family (0x4D, §5.2.11)
- `Ait` (0x74, TS 102 809), `DsmccSection` (0x3A–0x3F)
- Companion tables: `Unt` (0x4B, TS 102 006), `Int` (0x4C, EN 301 192),
  `Container` (0x75), `Rct` (0x76), `Cit` (0x77), `Rnt` (0x79) (TS 102 323)
- MPE family: `MpeDatagramSection` (0x3E, EN 301 192 §7 — typed IP/MAC view),
  `MpeFec` (0x78, EN 301 192 §9.9), `MpeIfec` (0x7A, TS 102 772)
- `ProtectionMessageSection` (0x7B, TS 102 809 §9) — authentication-message
  and certificate-collection variants discriminated by table_id_extension
- `DownloadableFontInfoSection` (0x7C, EN 303 560; table_id per EN 300 468
  V1.19.1 Table 2 NOTE 2 — the spec's own 0x4C was an acknowledged accident)
- **Coverage milestone: every allocated table_id in EN 300 468 V1.19.1
  Table 2 is implemented.**
- `carousel` module — DSM-CC data-carousel download protocol (ISO/IEC
  13818-6 §7.2/§7.3, DVB-profiled): typed `Dsi`/`Dii`/`DownloadDataBlock`
  messages over the `tables::dsmcc` section framing, plus
  `ModuleReassembler` for DDB→module collection. Layout provenance in
  `docs/iso_13818_6_carousel.md`; pinned against the live m6-single.ts
  capture and round-tripped byte-exact against broadcast bytes.

**Descriptors** — typed parsers for the common DVB + MPEG-2 descriptors
(network_name, service, service_list, linkage, short/extended_event, component,
content, parental_rating, CA, satellite/cable/terrestrial/S2 delivery system,
local_time_offset, subtitling, teletext, AC-3 / Enhanced AC-3, logical_channel,
default_authority, content_identifier, registration, stream_identifier,
data_stream_alignment, frequency_list, bouquet_name, private_data_indicator,
iso_639_language). Descriptors not yet typed pass through as raw bytes.

**Text** — Annex A subset: ISO 6937 (with diacritic combining), ISO 8859-n,
UTF-8 (selector 0x15), UCS-2 BE (0x11); Annex A.2 control codes.

**CRC** — Annex C MPEG-2 CRC-32 (from `dvb_common`).

**Typed constants** — `TableId`, `DescriptorTag`, `pid::well_known`.

**Feature flags** — `chrono`, `ts`, `serde`.

### Fixed
- `Tsdt` (0x03): removed a phantom `descriptor_loop_length` field — per
  ISO/IEC 13818-1 §2.4.4.12 descriptors run directly from byte 8 to the CRC,
  bounded by `section_length`.
- `Nit` / `Sdt`: added the `section_number` / `last_section_number` fields
  (previously parsed-and-discarded, serialized as 0), so multi-section
  sub-tables round-trip faithfully.
- `St` (0x72): byte-1 reserved nibble now `0x70` (reserved_future_use = 1),
  matching the other short-form sections (DIT/RST/TDT/TOT).
- `satellite_delivery_system` (0x43): corrected table cites — Polarization
  Table 38, Roll-off Table 39, Modulation system Table 40, Modulation type
  Table 41; `symbol_rate_bcd` masked to 28 bits on serialize.
- `s2_satellite_delivery_system` (0x79): `reserved_zero_future_use` (bit 5) now
  serialized as 0 per §6.2.13.3 Table 42 (was incorrectly 1).
- `content_identifier` (0x76): dropped the unreachable `CridLocation::Reserved`
  variant (reserved locations have no defined length and are rejected on parse).
- Doc-cite fixes: `teletext` type coding is Table 102 (not 99); `bouquet_name`
  §6.2.6, `network_name` §6.2.28, `parental_rating` §6.2.30, `service_list`
  §6.2.36, `stream_identifier` §6.2.41.
- `St`: accepts any `data_byte` value per §5.2.8 ("may take any value and has
  no meaning") — previously rejected non-zero stuffing, breaking real 0xFF fill.
- `Tot`: serialize emits `section_syntax_indicator = 0` per §5.2.6 (the TOT
  exception: SSI=0 yet CRC_32 present); was emitting SSI=1.
- `Sat`: PID corrected to 0x001B (EN 300 468 Table 1) — was wrongly 0x0010
  (the NIT PID); `pid::well_known::SAT` added.
- `Cat`: descriptor loop now preserved raw (`descriptors: Vec<u8>`) so non-CA
  descriptors round-trip; typed CA view via `Cat::ca_descriptors()`.
- `Cit`: dropped the desync-prone `prepend_strings_length` field (derived from
  the slice on serialize, guarded ≤ 255).
- `Bat`: no longer verifies CRC inside `parse` — crate-wide contract is that
  CRC validation belongs to `Section::validate_crc`; BAT was the lone
  inconsistent exception.
- `Nit`/`Bat`: per-entry `BufferTooShort` had `need`/`have` swapped.
- Removed the advertised-but-unimplemented `smallvec` and `rayon` feature
  flags (no code used either dependency).
- Text decoding: the default Latin table is now glyph-for-glyph faithful to
  EN 300 468 V1.19.1 Figure A.1 (transcribed in `docs/en_300_468.md`). Notable
  corrections: 0xA4 → € U+20AC (was ¤), 0xFC → þ (was œ), 0xFD → ŧ (was ı),
  0xFF → soft hyphen; quotes/arrows/fractions and the D/E/F rows no longer
  fall back to Latin-1. The full non-spacing row (macron, breve, dot, ring,
  double acute, ogonek, caron) is handled with precomposed forms and a
  base + combining-mark fallback. 0xA8 = ¤ confirmed correct against the PDF.

### Notes
- Tables and descriptors parse their outer structure with typed fields; nested
  descriptor and repeated loops are borrowed as raw `&[u8]` slices for the
  consumer to walk with the descriptor parsers.
