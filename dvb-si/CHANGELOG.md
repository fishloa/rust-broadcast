# Changelog

## Unreleased

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
