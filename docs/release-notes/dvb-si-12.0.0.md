---
status: DRAFT
---
# dvb-si 12.0.0

_Released 2026-10-16._

**Breaking release.** Twelfth lockstep major. `dvb-si` now brings EN 300 468 up to **V1.20.1**, preserves every `reserved`/`reserved_future_use` bit of every table and descriptor in typed fields (a real sender's section re-serializes byte-identically, CRC recomputed), types every PID as `mpeg_ts::pid::Pid`, and makes serializers reject values wider than their wire field (`fit_mask`, ~220 sites) instead of silently masking them. It also adds the Chinese AVS/GY/T descriptor families, MPEG-H 3D audio, and the MPEG-2 `Extension_descriptor`.

## Why this version

- **Proposed version:** 11.1.0 -> **12.0.0** (major-class: semver major).
- **Own API/behaviour change class:** major (typed Pid, reserved-bit fields, non_exhaustive tables, CellCenter->CellCentre, fit_mask serializer errors).
- **cargo-semver-checks vs live 11.1.0 (all features):** MAJOR (9 major, 0 minor lint failures). (Lint-based: it misses type changes of existing fields/arguments, so the CHANGELOG BREAKING list is the primary evidence.)
- **Epoch cascade:** none needed (own breaking change). Siblings it depends on that move epoch in this wave: broadcast-common, mpeg-ts.
- **Caret epoch:** 11 -> 12; every workspace dependent must move with it.

## Dependency changes

```toml
-broadcast-common = "9.4"
+broadcast-common = "10.0"
-mpeg-ts = "0.5"
+mpeg-ts = "0.6"
```

## Breaking changes and migration

### 1. PID fields are `mpeg_ts::pid::Pid` (#1159)

```rust
// before
let pid: u16 = stream.elementary_pid;
let entry = PatEntry { program_number: 1, pid: 0x0100 };
// after
let pid: mpeg_ts::pid::Pid = stream.elementary_pid;      // .value() for the raw u16
let entry = PatEntry::new(1, Pid::try_new(0x0100)?);      // Pid::new(x) masks to 13 bits; try_new rejects > 0x1FFF
```

`PatEntry::pid`, `PmtStream::elementary_pid`, `PmtSection::pcr_pid`, `CatCaEntry::ca_pid`, `CaDescriptor::ca_pid`, `Cp::cp_pid`, `PatSection::nit_pid()` (now `Option<Pid>`), `collect::*::pid`, `epg::EpgStore::feed_with_pid` and every per-table `PID` const are `Pid`. `CatCaEntry` no longer derives `Default`. `XaitPid::xait_pid` stays a raw `u16` (TS 102 727 Table 95 defines one 16-bit field).

### 2. Tables and descriptors preserve reserved bits (typed fields; structs are `#[non_exhaustive]`)

Every section struct gains a `bits: SectionBits` (PSI default `private_indicator = 0`, EN 300 468 tables `1`) and per-entry `reserved*` fields; descriptors capture each reserved region in a spec-named right-aligned field. **Struct-literal construction outside dvb-si no longer compiles.**

```rust
// before
let pat = PatSection { transport_stream_id: 1, version_number: 0, current_next_indicator: true, section_number: 0, last_section_number: 0, entries: vec![PatEntry { program_number: 1, pid: 0x100 }] };
// after
let mut pat = PatSection::new(1, 0, true, 0, 0, vec![PatEntry::new(1, Pid::try_new(0x100)?)]);
pat.bits = SectionBits::psi();            // only if you need a non-default
```

Constructors added: `PatSection::new`, `PmtStream::new`, `CatSection::new`, `SdtSection::new`, `SdtService::new`, `NitSection::new`, `BatSection::new`, `EitSection::new`, ... (full list in the CHANGELOG). Structural changes: `MosaicLogicalCell.elementary_cell_ids: Vec<u8>` -> `elementary_cells: Vec<ElementaryCell>`; `TtmlSubtitling.font_ids` -> `Option<Vec<TtmlFont>>`; `RelatedContentDescriptor` is no longer a unit struct; `C2DeliverySystem` is no longer `Copy`; `ShModulationMode::Tdm` / `ShInterleaver::Type1` are `#[non_exhaustive]` (use `::tdm()` / `::type1()`). **AC-3 `reserved_flags` now defaults to `0000`** (every EN 300 468 edition says "should always be set to 0"; dvb-si wrongly emitted `1111`). `satellite_delivery_system` re-emits `roll_off` on DVB-S too.

### 3. Serializers reject over-range values (`FieldOverflow`)

```rust
// before: Bandwidth::Reserved(0xFE) was masked into a valid-looking value on serialize
// after:  Error::FieldOverflow   (parse output always fits, so parse -> serialize is unaffected)
```

Applies to ~220 `(value & MASK)` sites (#1197) plus `ibp_descriptor.max_gop_length`, NorDig LCN, `smoothing_buffer`, INT `platform_id`, UNT `OUI`, S2Xv2 scrambling indices, SAT bit-writer counts and signed latitudes, etc. (#1077). Guarded by `tests/serialize_mask_guard.rs` and `tests/serialize_mask_overflow.rs`.

### 4. EN 300 468 V1.20.1 (#1222, #1229)

- The 2038 MJD wrap rule applies to every time field (EIT `start_time`, TDT/TOT `UTC_time`, `time_of_change`, ...); dates before 1948-08-05 are not representable.
- SAT names follow v1.20.1 spelling: `CellCenter` -> `CellCentre`, `CellFragment::center` -> `centre`, `center_latitude`/`center_longitude` -> `centre_latitude`/`centre_longitude` (serde keys follow).
- `Preselection::audio_rendering_indication` is the typed `AudioRenderingIndication` (Table 111) instead of a `u8`.
- `StSection` gains `section_syntax_indicator`; the SAT bodies gain `reserved_zero_future_use*` fields.

### 5. Removed placeholders

The ten `PID = 0x0000` placeholder constants on `ait`, `container`, `downloadable_font_info`, `int`, `mpe`, `mpe_fec`, `mpe_ifec`, `protection_message`, `rct` and `unt` (they aliased the PAT PID) are removed (#1077), as is the hidden `owned::doc` helper.

## What changed (from CHANGELOG `[Unreleased]`)

#### Added
- MPEG-2 `Extension_descriptor` (tag 0x3F, H.222.0 clause 2.6.90) with the seven MPEG-H 3D audio descriptors (`extension_descriptor_tag` 0x08-0x0E, clauses 2.6.106-2.6.118): typed `MpegH3dAudio`, `MpegH3dAudioScene`, `MpegH3dAudioTextLabel`, `MpegH3dAudioMultiStream`, `MpegH3dAudioDrcLoudness`, opaque `MpegH3dAudioConfig` / `MpegH3dAudioCommand`, verbatim `Other` for the remaining extension tags, all reserved bits preserved. The descriptor is auto-dispatched by `parse_loop` (`AnyDescriptor::Mpeg2Extension`) and `DescriptorTag::Mpeg2Extension` (0x3F) is new. Verified byte-for-byte against the PMTs of the Fraunhofer-IIS MPEG-H transport streams (fetch-on-demand `tools/fetch-test-streams.sh mpegh-*`) and TSDuck-compiled vectors.

#### Changed (BREAKING, #1229 - GY/T 422-2025 multi-audio)
- **BREAKING:** `Preselection::audio_rendering_indication` is now the typed `AudioRenderingIndication` (EN 300 468 Table 111 / GY/T 422-2025 Table 2: no preference, stereo, two-dimensional, three-dimensional, headphones, reserved(n)) instead of a raw `u8`.

#### Added (#1229)
- GY/T 422-2025 Table 1 (the Chinese multi-audio `audio_preselection_descriptor`, tag 0x7F ext 0x19) is a wire-compatible subset of the existing descriptor (the four DVB flags and `text_label_present` are reserved zero there); `Preselection::is_gy_t_422_profile()` and a spec-vector round-trip test (Table 1, verified on the rendered page) document and check it. Transcription: `docs/descriptors/gy_t_422/audio-preselection-descriptor.md`.
#### Added (#1231 - AVS2 and older AVS3 descriptors)
- `descriptors::avs2_video::Avs2VideoDescriptor` (tag 0x40, GY/T 420-2025 Table 3, 5-byte body) scoped by an 'AVSV' registration_descriptor or private_data_specifier (`DescriptorRegistry::with_avs2_video()`; unscoped 0x40 stays `network_name`).
- `descriptors::avs2_audio::Avs2AudioDescriptor` (tag 0xD3, T/AI 109.7-2024 Table 10: text/language/version blocks) scoped by 'AVSA' (`with_avs2_audio()`). `anc_data_block()` is defined in GB/T 33475.3 Annex A.2 (not vendored), so the bytes after the version block are kept undivided in `trailing`.
- `descriptors::tuwa_avs3_video::TuwaAvs3VideoDescriptor` (tag 62, T/UWA 012.2-2023 Table 1, with the typed library-stream references `TuwaLibraryRef::{Peid, StreamId}`), PDS-agnostic opt-in `with_tuwa_avs3_video()`.
- `AnyDescriptor::{Avs2Video, Avs2Audio, TuwaAvs3Video}` variants (opt-in, not auto-dispatched). Transcriptions in `docs/descriptors/gy_t_420/`, `t_ai_109_7/`, `t_uwa_012_2/`.

#### Added (#1224 - AVS3 audio)
- `descriptors::avs3_audio::Avs3AudioDescriptor`: the AVS3_audio_descriptor (tag 0xD2) from T/AI 109.7-2024 clause 8.1.3.4 Table 4 (spec located free at aitisa.org.cn): `audio_codec_id`-selected bodies (0, 1 with the optional 24-bit `sampling_frequency`, 2 with the Table 5 `content_type` variants and `total_bitrate`), `resolution`, opaque `addition_info`; reserved bits captured on parse, all-ones defaults when hand-built. Private descriptor: not auto-dispatched; enable with `DescriptorRegistry::with_avs3_audio()`, which scopes it to `PDS_AVSA` ('AVSA', 0x41565341) as ETSI TS 101 154 V2.10.1 section 4.1.8.35 requires. `AnyDescriptor::Avs3Audio` variant added (`@no_dispatch`). The Chinese documents scope it with a `registration_descriptor` 'AVSA' instead of a private_data_specifier; the registry accepts both (a registration whose format_identifier is 'AVSA'/'AVSV' opens the same scope; any other registration leaves the context untouched). Transcription: `docs/descriptors/t_ai_109_7/avs3-audio-descriptor.md`.
- `StreamType::Avs2Video` (0xD2, GY/T 420-2025 7.2.1), `Avs2Audio` (0xD3, T/AI 109.7 8.2), `Avs3Video` (0xD4) and `Avs3Audio` (0xD5) (TS 101 154 V2.10.1 section 4.1.8; T/AI 109.6 / 109.7 clause 8.1.2). Note 0x33 is VVC, not AVS3.
- `descriptors::avs3_video::Avs3VideoDescriptor` (tag 0xD1, T/AI 109.6-2022 clause 9.3.2 Table 9 / GY/T 420-2025 clause 7.3.3.2 Table 6) with typed `Avs3FrameRateCode`; 7- and 8-byte bodies (T/AI 109.6 Table 9 omits `colour_primaries`, its own semantics and GY/T 420 include it - both re-emitted faithfully); `DescriptorRegistry::with_avs3_video()` for 'AVSV' (PDS or registration). `AnyDescriptor::Avs3Video` variant. Transcription: `docs/descriptors/t_ai_109_6/avs3-video-descriptor.md`.
- `PDS_AVSV` (0x41565356, 'AVSV'): the specifier that precedes the AVS3_video_descriptor and the `registration_descriptor` format_identifier for AVS3 HDR; tests for the 'AVSV'/'AVSA' registration format identifiers.
#### Added (#1227 - China closed caption, GY/T 270-2013)
- `descriptors::china_caption_service::ChinaCaptionServiceDescriptor` (tag 0x86, clause 6.4 Table 8, `char_set` Table 9): per-service `language`/`caption_service_number`/`wide_aspect_ratio`/typed `ChinaCaptionCharSet`, trailing `caption_service_pid`, all reserved runs preserved. The tag is context-dependent (ATSC uses it differently), so it is not auto-dispatched: `DescriptorRegistry::with_china_caption_service()`. `AnyDescriptor::ChinaCaptionService` variant. Transcription: `docs/descriptors/gy_t_270/caption-service-descriptor.md`.
- `StreamType::ChinaClosedCaption` (0x80, clause 6.2: `cc_data()` in a dedicated caption PES). The `cc_data()` link layer (Table 10) is bit-identical to `cc-data`'s existing parser.

#### Changed (BREAKING, #1222 - EN 300 468 v1.20.1)
- Brought up to ETSI EN 300 468 V1.20.1 (2026-09); clause-by-clause delta in `docs/en300468-v1.20.1-delta.md`. v1.20.1 adds no tags, table_ids or wire layouts.
- **BREAKING (behavioural):** the 2038 MJD wrap rule (clauses 5.2.4/5.2.5/5.2.6, 6.2.20, 6.4.9) now applies to every time field (EIT `start_time`, TDT/TOT `UTC_time`, local_time_offset `time_of_change`, network_change_notify `start_time_of_change`): when the msb of the 16-bit MJD field is 0 the MJD is `0x10000` + the field, so `0x03D1123456` is 2040-12-25 12:34:56. Dates before 1948-08-05 are no longer representable (implemented in `broadcast_common::time`).
- **BREAKING:** SAT cell-fragment names follow v1.20.1 spelling: `CellCenter` -> `CellCentre`, `CellFragment::center` -> `centre`, `center_latitude`/`center_longitude` -> `centre_latitude`/`centre_longitude` (serde keys follow).
- SAT `satellite_position_v3_info`: `usable_start/stop_time_flag` (when `metadata_flag == 0`) and `interpolation_type`/`interpolation_degree` (when `interpolation_flag == 0`) are documented as "not used" (was "reserved"); values are preserved verbatim, now pinned by a test.

#### Added (#1222)
- `PDS_AVSA` (`0x41565341`, 'AVSA'): the private_data_specifier that precedes the AVS-P3 audio descriptor (Annex O).
- Tests carrying the v1.20.1 Table 26 component types (AVS3-P3 audio `0x9/0x1/0x20-0x22`, T/UWA 005 DMI `0xB/0xF/0x0A`) byte-exact through `ComponentDescriptor`.

#### Changed (BREAKING, #1197)
- **BREAKING (behavioural):** every remaining single-field `(value & MASK)` in a serializer is a checked `broadcast_common::len::fit_mask(..)?` that returns `Error::FieldOverflow` instead of silently dropping the high bits of an over-range value (the #1129 class; ~220 sites across the descriptors and tables, including bare `(field << n)` shifts such as `satellite_table_id`, `reference_type`, `c2_system_tuning_frequency_type`). Hand-built values that were wider than their field (reserved runs, `Reserved(v)` enum payloads such as `Bandwidth::Reserved(0xFE)` / `FecInner::Reserved(0x3E)`, 5-bit `stream_content_ext`, ...) used to be masked and now fail; values produced by `parse` always fit. Guarded by `tests/serialize_mask_guard.rs` (lexical, reasoned allow-list) and `tests/serialize_mask_overflow.rs` (behavioural).
- **BREAKING:** the SAT (`tables::sat`) body structs gain typed `reserved_zero_future_use*` fields for every reserved run of EN 300 468 Tables 11c/11d/11e/11g/11h (captured on parse, re-emitted verbatim, hand-built default `0`): `PositionV2Satellite`, `PositionSystem::Orbital`, `CellCenter`, `CellFragment`, `NewDeliverySystem`, `ObsolescentDeliverySystem`, `TimeAssociationBody`, `BeamhoppingPlan`, `BeamhoppingMode::{Mode0, Mode1, Mode2}`, `PositionV3Body`, `PositionV3Satellite`, `PositionV3Metadata`, `UsableTime`, `EphemerisData`, `CovarianceData`. The SAT bit writer no longer truncates: an over-range value (a count, a 9-bit `*_ext` above 0x1FF, a signed latitude out of range) is `Error::FieldOverflow`, and `first_occurence` without a `center` is `ValueOutOfRange`.
- **BREAKING:** `StSection` gains `section_syntax_indicator` (EN 300 468 Table 11: the stuffing section's indicator "may take either the value 0b1 or 0b0"); captured on parse, re-emitted verbatim, `new()` keeps `false`.

#### Fixed (#1197)
- DTS `sample_rate_code` 0b1001 (88,2 kHz): the decision is now documented with the TS 102 114 / EN 300 468 V1.20.1 evidence (`docs/reserved-bits-edition-audit.md` section 12); no behaviour change.
- Local-archive evidence for the SSU selector (real zero reserved runs preserved) and `logical_channel` v1 (`reserved` = 1 in every real capture): `tests/local_archive_reserved_defaults.rs`.

#### Changed
- Internal: adapted to `broadcast-common`'s removal of the panicking `Serialize::to_bytes` (tests, examples and docs now use `try_to_bytes`); requires the new `broadcast-common` major (umbrella #1143).

#### Fixed

- `--no-default-features --features chrono` now builds for bare-metal (`thumbv7em-none-eabi`): `epg` was missing `alloc::vec` and carried a serde-only `format`/`ServiceData` unconditionally (#1181).
#### Changed (BREAKING)
- **BREAKING:** the ten `PID = 0x0000` placeholder constants that aliased the PAT PID are removed from the `ait`, `container`, `downloadable_font_info`, `int`, `mpe`, `mpe_fec`, `mpe_ifec`, `protection_message`, `rct` and `unt` table modules (a caller filtering on one picked up PAT traffic); these tables have no fixed PID (audit r02-W15 / r02-O12, #1077). The hidden `owned::doc` doctest helper module is removed too (r02-O7).
- **BREAKING (behavioural):** serializers reject a value wider than its wire field instead of masking it into a different valid-looking value (audit r03-W3 class, #1077): `ibp_descriptor.max_gop_length` (14 bits), NorDig LCN V1 (14) / V2 (10), `smoothing_buffer` (22), `metadata_std` (22), `multiplex_buffer_utilization` (15), `mvc_extension` (10 / 3), `target_background_grid` and `video_window` (14 / 14 / 4), INT `platform_id` (24), UNT `OUI` (24), RCT `dvb_service_triplet_id` (10), `content_labeling` time base (33), S2Xv2 scrambling indices (20). Parse stays tolerant; legal values round-trip byte-identically.
#### Fixed
- DTS descriptor `sample_rate_hz()` for SFREQ `0b1001` is 88200 Hz (the table prints the digit transposition "88,02 kHz"; EN 300 468's own DTS-HD table lists 88,2 kHz) (r03-W11).
- BIOP directory / stream / stream-event serializers use the checked `fit_*` helpers like the file message (r02-W13).
#### Performance
- Section collectors copy a section into an `Arc` only when it is retained: a duplicate carousel repeat allocates nothing (r02-O1, `tests/collect_alloc.rs`).
- Annex A text decode: no intermediate `Vec<u16>` for UCS-2 and no second `String` when no control code is present (r02-O4: 8 -> 1 and 15 -> 2 allocations for 200-character strings, `tests/text_alloc.rs`).
- SAT body sizing trial-encodes on the stack: a `serialized_len` + `serialize_into` pair no longer heap-allocates 16 KiB (r02-O2, `tests/sat_alloc.rs`).
- One `tables::write_crc` helper and one `CRC_LEN` replace ~25 copies of the CRC trailer code (r02-W10); residual bare section-header masks are named (r02-W16).
#### Changed (BREAKING)

- **PID fields are the typed `mpeg_ts::pid::Pid`** (typed-ids wave 1b): `PatEntry::pid`, `PmtStream::elementary_pid`, `PmtSection::pcr_pid`, `CatCaEntry::ca_pid`, `CaDescriptor::ca_pid`, `Cp::cp_pid`, `PatSection::nit_pid()` (now `Option<Pid>`), and every per-table `PID` const (`pat::PID`, `eit::PID`, ...). The matching constructors (`PatEntry::new`, `PmtStream::new`, `PmtSection::new`, `CaDescriptor::new`, `Cp::new`) take `Pid`, so an over-13-bit value is unrepresentable (use `Pid::try_new` at a user boundary); the old serialize-time `FieldOverflow` for `ca_pid`/`cp_pid` is gone with it. Parse masks the 13 wire bits into the `Pid` (the 3 reserved bits stay in their typed reserved fields); serde and `dvb-tools` output are byte-identical (`Pid` serializes transparently as the number). `CatCaEntry` no longer derives `Default` (`Pid` has none). `XaitPid::xait_pid` stays a raw `u16`: TS 102 727 Table 95 defines one 16-bit field with no reserved split and the crate round-trips it verbatim. Migration: drop `Pid::new(entry.pid)` / `.value()` adaptors at call sites; pass `Pid::new(..)`/`Pid::try_new(..)` where a literal was used. Gate: `real_capture_byte_identity` now compares every PAT/PMT/CA PID against an independent 13-bit wire extraction over all real captures; `tests/typed_pid.rs` pins the boundary.
#### Changed (breaking)

- **Tables preserve every `reserved` / `reserved_future_use` bit of the section header and body in typed fields** (reserved-bits audit §3.6): parse is tolerant and `serialize` re-emits what was parsed, so a real sender's section re-serializes byte-identically (CRC is recomputed); values built with `new()` / `Default` carry the spec fill (all ones, except the PSI `'0'` bit, `reserved_zero_future_use` = 0).
  - New `tables::SectionBits { private_indicator, reserved_b1, reserved_b5 }` (`#[non_exhaustive]`, `SectionBits::psi()` / `::dvb()`). The bit after `section_syntax_indicator` is H.222.0's `private_indicator` ("user definable" in all nine editions) and EN 300 468's `reserved_future_use`: PSI sections (PAT/PMT/CAT/TSDT) default it to `0`, EN 300 468 tables to `1`. New `bits` field on `PatSection`, `PmtSection`, `CatSection`, `TsdtSection`, `SdtSection`, `NitSection`, `BatSection`, `EitSection`, `TdtSection`, `TotSection`, `RstSection`, `StSection`, `DitSection`, `SitSection`, `AitSection`, `IntSection`, `UntSection`, `RntSection`, `ProtectionMessageSection`, `DownloadableFontInfoSection`.
  - The private-section tables that already carried `private_indicator` (`DsmccSection`, `ContainerSection`, `CitSection`, `RctSection` (`table_id_extension_flag`), `SatSection`, `MpeDatagramSection`, `MpeFecSection`, `MpeIfecSection`) gain `reserved_b1` / `reserved_b5` instead.
  - Body reserved fields are typed on the entry they belong to: `PatEntry::reserved` (3), `PmtStream::{reserved_pid, reserved_es_info_length}`, `PmtSection::{reserved_pcr_pid, reserved_program_info_length}`, `CatSection::reserved_extension` (the 16 reserved bits at bytes 3-4), `SdtService::reserved_future_use` (6), `SdtSection::reserved_future_use` (8), the `reserved_future_use(4)` before every descriptor-loop length in NIT/BAT/SIT/AIT/INT/UNT/RNT/RCT/TOT/`TargetOperationalLoop`/`ResolutionProvider`, `RstEntry::reserved_future_use` (5), `DitSection::reserved_future_use` (7), `SitService::reserved_future_use` (1), `CridAuthority::reserved` (2), RCT `LinkInfo`/`DvbBinaryLocator` reserved bits, MPE-FEC byte 4/5 reserved bits, `SatSection::reserved_zero_future_use` (8), DFIS `StyleWeight`/`FileUri` `reserved_zero_future_use`, protection-message loop/count nibbles.
  - **Migration:** the touched table structs are now `#[non_exhaustive]` (struct-literal construction no longer compiles outside the crate): use the new constructors `PatSection::new`, `PatEntry::new`, `PmtStream::new`, `CatSection::new`, `TsdtSection::new`, `SdtSection::new`, `SdtService::new`, `NitSection::new`, `NitTransportStream::new`, `BatSection::new`, `BatTransportStream::new`, `EitSection::new`, `RstSection::new`, `RstEntry::new`, `DitSection::new`, `SitSection::new`, `SitService::new`, `AitSection::new`, `AitApplication::new`, `IntSection::new`, `UntSection::new`, `UntPlatform::new`, `TargetOperationalLoop::new`, `RntSection::new`, `ResolutionProvider::new`, `CridAuthority::new`, `RctSection::new`, `LinkInfo::new`, `DvbBinaryLocator::new`, `CitSection::new`, `ContainerSection::new`, `DsmccSection::new`, `MpeDatagramSection::new`, `MpeFecSection::new`, `MpeIfecSection::new`, `ProtectionMessageSection::new`, `ProtectionMessageBody::certificate_collection`, `DownloadableFontInfoSection::new`, `FontInfo::{style_weight, file_uri}`, `SatSection::new` (`TdtSection::new`/`TotSection::new` already existed); the reserved fields stay `pub` for off-spec senders. `CatSection` / `PatSection` `Default` keep their meaning (`PatSection::default()` is now the PSI-filled value). `ProtectionMessageBody::AuthenticationMessage` / `CertificateCollection` and `FontInfo::{StyleWeight, FileUri}` gained fields, so struct-literal construction and exhaustive patterns need updating (add `..`).
  - serde output is unchanged (every reserved field is `serde(skip)`).
  - Real captures: every table section of the local `.test-streams` archive now re-serializes byte-identically except `section_syntax_indicator` of the random-bits stress capture `test-062` (a structural bit, not a reserved bit); TDT `test-120`, NIT `test-016`, PAT `test-012`/`test-065`, SDT `test-170`, EIT `test-109`/`test-170`/`test-171`, AIT `test-170` and DSM-CC/ST `test-062` left the `TOLERATED` list.
  - Not yet preserved: bit-level `reserved_zero_future_use` runs inside the `SAT` bodies (Position/Cell-fragment/Time-association/Beamhopping, `BitWriter`-emitted zeros).
#### Changed (BREAKING)

- **Descriptors now preserve reserved bits (owner decision; supersedes r03-W7/r03-W8 "drop on parse, emit the spec value").** Parse stays tolerant; every `reserved` / `reserved_future_use` / `reserved_zero_future_use` region of every descriptor under `descriptors/` (incl. `extension/` and `ait/`) is captured in a typed, spec-named, right-aligned field (`reserved`, `reserved_future_use`, `reserved_flags`, `reserved_zero_future_use_6`, ...) and re-emitted verbatim, so real-sender descriptors re-serialize byte-identically; descriptors built with `new(..)`/`Default` carry the governing edition's value (audit: `docs/reserved-bits-edition-audit.md`). Every real descriptor in the local capture archive now re-serializes byte-identically (before: AC-3 47/62, application_signalling 15/52, supplementary_audio 37/47, satellite 10/678, CA 2/715, TVA_id 1/1 differed; after: 0).
  - **AC-3 `reserved_flags` default is now `0000`** (every EN 300 468 edition V1.4.1-V1.20.1 and TS 102 366 say "should always be set to 0"; dvb-si wrongly emitted `1111`). `application_signalling` entries gain `reserved_future_use_1` (MHP-era senders write 0 before the 15-bit `application_type`) and `reserved_future_use_3`. `satellite_delivery_system` now re-emits `roll_off` on DVB-S too (sender `01` round-trips; a constructed DVB-S descriptor uses `Alpha035` = `00`).
  - All changed structs are `#[non_exhaustive]` with `new(..)` and/or `Default`: **struct-literal construction outside dvb-si no longer compiles** (use `new`/`Default` + field assignment). New `*_DEFAULT` consts replace the old private `RESERVED_*` masks (extension-module consts are module-prefixed to avoid glob collisions).
  - Structural changes: `MosaicLogicalCell.elementary_cell_ids: Vec<u8>` -> `elementary_cells: Vec<ElementaryCell>`; `TtmlSubtitling.font_ids` -> `Option<Vec<TtmlFont>>`; `RelatedContentDescriptor` is no longer a unit struct (`trailing`); `C2DeliverySystem` is no longer `Copy`; `ShModulationMode::Tdm` / `ShInterleaver::Type1` are `#[non_exhaustive]` (use `ShModulationMode::tdm` / `ShInterleaver::type1`).
  - Tail bytes that parse used to drop silently are now kept in typed `trailing` fields and re-emitted: `video_stream`, `protection_message`, `related_content`, `c2_delivery_system`, `s2_satellite_delivery_system`.
  - New per-descriptor tests (parse non-default reserved -> byte-identical re-serialize; default-constructed -> spec value), mutation-verified; `real_capture_byte_identity` `TOLERATED` no longer lists any descriptor.


#### Changed (BREAKING)

- **BREAKING (#1159):** `TsHeader::pid` (re-exported via `mpeg-ts`) is now `Pid`; `collect::*::pid` / `epg::EpgStore::feed_with_pid` take `Option<Pid>` instead of `Option<u16>`.
#### Fixed

- **`S2Xv2_satellite_delivery_system` `satellite_id` over 24 bits is rejected (#1129).** It was written as three bytes of `value >> 16 / >> 8 / as u8`, silently dropping the top byte; it now returns `Error::FieldOverflow`. `CompatibilityDescriptor` length/count narrowing now goes through `broadcast_common::len::fit_*`, and `tests/serializer_length_truncation.rs` scans all of `src/` (it covered only `descriptors/`, `tables/` and `carousel/`).
- `hevc_video`: byte 12's `reserved(2)` (bits `[3:2]`) and the temporal sub-block's two `reserved(5)` fields are now all ones per H.222.0 Table 2-113. Real captures (9 HEVC_video_descriptors in the local TSDuck archive carry `0x9F` + `1F 1F`) contradicted the earlier r03-W7 reading that cleared bit 3 and the sub-block reserved bits, so they did not re-serialize byte-identically. Parse unaffected.

#### Added

- `tests/table_reserved_bits.rs`: single-bit-flip property over every table type (real captured sections plus built sections): every flipped bit must survive `parse -> serialize`; spec-fill defaults; real-sender sections from the local archive.
- `tests/real_capture_byte_identity.rs`: re-serializes every table section and every descriptor in every PMT/SDT/NIT/BAT/EIT/CAT/TSDT/TOT loop of every capture under the local `.test-streams/` archive (skips when absent) and asserts byte-identity, with an explicit `TOLERATED` allowlist of reserved-bit deviations by real senders keyed by (capture, descriptor/table). Backed by a `#[doc(hidden)]` `AnyDescriptor::reserialize()`.

## Merged changes since `v11.1.0`

- tests: consume deterministic excerpts of the france-tnt/hotbird captures instead of the 540 MB LFS originals (#1263) (#1264)
- tests: one fixture resolver (public -> private/fixtures -> .test-streams); move local-only captures to the private repo (#1259) (#1260)
- docs/test(dvb-si): NorDig LCD reserved bits verified against NorDig Unified Requirements v3.2.2; correct EACEM-vs-NorDig evidence; bump private (#1253)
- feat(dvb-si): MPEG-2 Extension_descriptor (0x3F) + MPEG-H 3D audio descriptors (#1251)
- feat(dvb-si,transmux): China closed caption (GY/T 270-2013) (C1) (#1228)
- feat(dvb-si): AVS2 video/audio descriptors + T/UWA 012.2 tag-62 AVS3 descriptor (C3) (#1233)
- O1: ordering hygiene — registry tables in wire-value order + ordering guard (#1238)
- feat(dvb-si): GY/T 422-2025 multi-audio subset check, typed AudioRenderingIndication (#1229) (#1230)
- feat(dvb-si): AVS3 audio/video descriptors, AVS2/AVS3 stream types, extended PES ids (G1b) (#1226)
- feat(dvb-si): EN 300 468 v1.20.1 (2038 MJD wrap, SAT centre/occurrence, new component types) (#1223)
- test: shared test-fixtures loader crate, migrate hand-written loaders, drop CRC copies (#1214) (#1215)
- dvb-si: checked fit_mask for serializer masks, SAT reserved runs, stuffing SSI, reserved-default evidence (#1197) (#1199)
- ttml-subtitle: populate/round-trip xml_declaration; drop stale dvb-si hasher TODO (#1190)
- Cleanups: remove panicking Serialize::to_bytes (BREAKING, #1143) + serde default-features off in 32 no_std manifests with CI thumbv7em serde matrix (#1183) (#1185)
- fix(transmux,dvb-si): no_std optional features build on thumbv7em (cenc, sample-aes, chrono) + CI step; dedupe cenc box-nav helpers (#1181, #982) (#1184)
- Systemic umbrellas T1/T2/T14: serializer width guards (28 crates), hostile-count DoS fix, secrets hygiene, origin-scoped credentials, secure ingest default (BREAKING items marked, NO release) (#1182)
- Audit critical-bug verification pass: 72 issues re-verified against main; fixes for #1012 #1022 #1034 #1035 #1056 + regression tests (NO release) (#1180)
- Audit per-crate rollups: verified against main, 54 commits of real fixes across ~40 crates (BREAKING items marked, NO release) (#1179)
- Typed ids wave 2: TrackId/ProgramNumber + HLS naming newtypes, Pid inside dvb-si (BREAKING, NO release) (#1178)
- Typed ids wave 1: Pid newtype (mpeg-ts and all non-dvb-si APIs) + reserved-bit preservation for all dvb-si descriptors and tables (BREAKING, NO release) (#1177)
- docs(dvb-si): reserved-bit edition audit (EN 300 468 V1.3.1-V1.20.1, H.222.0 1995-2023, AIT/SSU/MHP/RCT editions) (#1176)
- Audit systemic wave 2: T16 resync, largesize, ttml drop, WHIP/WHEP DELETE/PATCH/ICE restart, mpeg-ts/pes reserved+stuffing preservation, real-capture guards (NO release) (#1175)
