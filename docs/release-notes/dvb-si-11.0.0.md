# dvb-si 11.0.0

_Released 2026-10-05._

**Major (breaking).** This release makes every `dvb-si` serializer refuse to emit a frame it cannot describe correctly: an over-range length or count, or a flag-gated field with no value, is now an error instead of a silently wrapped, masked or invented byte. It also types several fields that were raw bytes (BIOP/carousel, RCT, AIT, INT/UNT) and fixes a large set of parser and spec-fidelity defects found by the 2026-09 audit. You must act if you (a) match on `dvb_si::Error` exhaustively, (b) read or construct any of the types listed under "Breaking changes", or (c) relied on a serializer producing bytes for an out-of-range value. If you only parse and read common fields, expect a clean build apart from the new `Error` variant.

Read together with: [mpeg-ts 0.5.0](mpeg-ts-0.5.0.md) and [broadcast-common 9.4.0](broadcast-common-9.4.0.md) (both are required dependencies at new versions), and the sibling lockstep notes [dvb-t2mi 11.0.0](dvb-t2mi-11.0.0.md), [dvb-bbframe 11.0.0](dvb-bbframe-11.0.0.md), [dvb-conformance 11.0.0](dvb-conformance-11.0.0.md) and [dvb-tools 11.0.0](dvb-tools-11.0.0.md). Move the whole lockstep set together.

## Dependency and feature changes

```toml
# dvb-si/Cargo.toml, previous release (10.0.1) -> 11.0.0
-broadcast-common = { version = "9.3", default-features = false }
+broadcast-common = { version = "9.4", default-features = false }   # broadcast_common::len helpers
-mpeg-ts          = { version = "0.4", default-features = false }
+mpeg-ts          = { version = "0.5", default-features = false }   # caret epoch change
```

`mpeg-ts` crosses a caret epoch (0.4 to 0.5). `dvb_si::Error` and the `collect` error type have `From<mpeg_ts::Error>` impls, so a dependency graph resolving two `mpeg-ts` copies binds the wrong one; check with `cargo tree -i mpeg-ts` and move every crate that depends on it together. The `list_services` example now declares `required-features = ["ts"]`. No default feature changed.

## Breaking changes

### 1. `dvb_si::Error` gains `FieldOverflow`

`Error::FieldOverflow` is `#[error(transparent)]` over `broadcast_common::len::FieldOverflow` (#1129). Any serializer can now return it when a length or count does not fit its wire field. An exhaustive `match` on `Error` needs a new arm; `ValueOutOfRange`, `SectionLengthOverflow` and `OutputBufferTooSmall` already existed.

### 2. Serializers reject instead of truncating, masking or inventing values

What changed (#1129, #1001, #1003, #1077):

- All table serializers (pat, cat, tsdt, pmt, nit, bat, sdt, ait, int, sit, dsmcc, st, rst, downloadable_font_info, protection_message, eit; cit, container, mpe, mpe_fec, mpe_ifec, rnt, tot, unt; inner loops in cit/rct) range-check `section_length` and nested loop lengths/counts. Previously an oversized body wrapped modulo 4096 and was written with a valid CRC over bytes the header did not cover.
- About 80 descriptor serializers range-check the 8-bit `descriptor_length`, and nested lengths/counts inside bodies, including bit-packed sub-byte fields (for example `audio_preselection`'s `num_preselections`, `telephone`'s packed length fields, `network_change_notify`, `service_prominence`, `video_depth_range`, `cp`'s `CP_PID`, `logical_channel_number`).
- Carousel/BIOP/IOR serializers check `messageSize`, `messageBody_length` and the IOR profile length fields.
- Flag-gated fields no longer fall back to `unwrap_or(0)`: metadata_pointer, avc_timing_and_HRD, video_stream, the s2/s2x/s2xv2 satellite delivery descriptors, ac4, uri_linkage, and DTS-HD (`component_type_flag`/`language_code_flag` set with no value) now return `ValueOutOfRange`. `ExtensionDescriptor` serialization rejects a `tag_extension` that disagrees with its typed body.
- The MPE-FEC/MPE-IFEC `real_time_parameters` encode returns `Error::FieldOverflow` for `delta_t` over 12 bits or `address`/`prev_burst_size` over 18 bits.
- Sub-byte `Reserved(v)` payloads are masked at the shift, so a wide user value (for example `Bandwidth::Reserved(0xFE)`) no longer bleeds into the neighbouring field.
- `multilingual_network_name`, `multilingual_service_name` and AIT `application_name` over-range text now yield `Error::FieldOverflow` rather than `Error::InvalidDescriptor`; `teletext_descriptor` and `VBI_teletext_descriptor` over 255 body bytes yield `Error::FieldOverflow` (the VBI copy used to return `InvalidDescriptor`). Wire output and parse results for valid input are unchanged (#1141).

Who is affected: anyone who builds descriptors/tables from user data and ignored or `unwrap`ped the serialize result on the assumption it could not fail, or who matched on `InvalidDescriptor` for the cases above. Migration: propagate the `Result`, and handle `FieldOverflow`/`ValueOutOfRange`.

### 3. `ServiceGatewayInfo::to_bytes` returns `Result`

`carousel::biop::message::ServiceGatewayInfo` now implements `Serialize` (it was the one wire type without it). `to_bytes` used to panic on more than 255 `service_context` entries and silently truncate a `user_info` over 65,535 bytes.

```rust
// before
let bytes: Vec<u8> = sgi.to_bytes();
// after
let bytes: Vec<u8> = sgi.to_bytes()?;
```

### 4. Typed BIOP/carousel fields (r02-W12)

```rust
// ServiceGatewayInfo::download_taps: &[u8] (count byte + tap bytes)  ->  Vec<Tap<'a>>
for tap in &sgi.download_taps { /* typed Tap, no hand-parsing */ }

// DirectoryObjectData::entries: Vec<(Vec<u8>, u16, Vec<u8>)>  ->  Vec<DirectoryEntry>
for e in &dir.entries { let (name, module, key) = (&e.name, e.module_id, &e.object_key); }

// ModuleInfo::user_info: &[u8]  ->  descriptors::DescriptorLoop<'a>

// CompressedModuleDescriptor { body: &[u8] }
//   -> { compression_method: u8, original_size: u32, zlib_data: &[u8] }
// ModuleInfo::compressed_module_descriptor() now returns
//   Option<Result<CompressedModuleDescriptor<'_>>>   // a short body is an Err, not a panic
```

`ModuleInfo::descriptors()` now delegates to `DescriptorLoop::raw_tags()` instead of a private duplicate walker.

### 5. INT and UNT share `TargetOperationalLoop` (r02-W22)

`tables::int::IntLoopEntry` is now a type alias of the new `tables::TargetOperationalLoop` (same fields: `target_descriptors`, `operational_descriptors`), so INT code is source-compatible. UNT changed type:

```rust
// before: UntPlatform::target_operational_pairs: Vec<(DescriptorLoop, DescriptorLoop)>
for (target, operational) in &platform.target_operational_pairs { ... }
// after:  Vec<TargetOperationalLoop>
for pair in &platform.target_operational_pairs {
    let (target, operational) = (&pair.target_descriptors, &pair.operational_descriptors);
}
```

### 6. `collect::CompleteNit` / `CompleteBat` descriptors merge every section

`network_descriptors` / `bouquet_descriptors` reflected only section 0's loop although EN 300 468 §5.2.1/§5.2.2 give every section its own loop. `ParsedDescriptorLoop` now merges all contributing sections, and `raw()` returns `&[DescriptorLoop<'a>]`, one per section. Code that treated `raw()` as a single loop must iterate.

### 7. RCT: `DvbBinaryLocator` fields removed, trailing bytes rejected

`identifier_type` and `inline_service` were stored beside `identifier` and `service`, so a caller could set them inconsistently with the enum variant. Both fields are removed; read them with `DvbBinaryLocator::identifier_type()` and `inline_service()`. Serialize now also validates that `windows` is `Some` only when `identifier` is `DvbLocatorIdentifier::None` and `scheduled_time_reliability` is true. Separately, `LinkInfo`/`RctSection` now reject trailing bytes between the descriptor loop and `link_info_length` / the CRC (they were silently dropped on re-serialize).

### 8. AIT: `ApplicationType::UserDefined` removed

`application_type` is a 15-bit wire field, so `from_u16` could never produce the documented `0x8000..=0xFFFF` `UserDefined(u16)`. The variant is gone; `ApplicationType::Reserved(u16)` now means "any other value" (`0x0000..=0x7FFF`). Serialize rejects a directly constructed value that does not fit 15 bits.

```rust
// before
ApplicationType::Reserved(v) | ApplicationType::UserDefined(v) => ...
// after
ApplicationType::Reserved(v) => ...
```

### 9. Removed items

- `tables::pmt::PID` and `tables::dsmcc::PID`: the placeholder `0x0000` equalled the real PAT PID, so filtering on it picked up PAT traffic. PMT PIDs come from the PAT; DSM-CC has no well-known PID.
- `descriptors::private_data_indicator::private_data_specifier_name` (a re-export of the DVB PDS registry lookup, which does not apply to this ISO/IEC 13818-1 field). Call `descriptors::private_data_specifier::private_data_specifier_name` directly if you want the DVB lookup for a `private_data_specifier_descriptor`.
- `ExtendedEventLinkageEntry` no longer stores `target_id_type`, `original_network_id_flag` or `service_id_flag`; they are derived from its `TargetId` (r03-W5, #1077).
- `VbiTeletextEntry` is now `pub type VbiTeletextEntry = TeletextEntry` (same fields; `TeletextEntry` gains `Copy`). Source-compatible unless you implemented traits on both.

## Behaviour changes that are not API breaks

- Parsers now reject, rather than silently drop, trailing or inconsistent bytes in a number of places, so a non-conformant stream that used to parse may now error: `application_usage`, `simple_application_boundary`, `AVC_timing_and_HRD`, `AVC_video`, `data_broadcast`, `extended_event`, `xait_location`, `MPEG-2_AAC_audio`, `MPEG-4_audio`, `MPEG-4_video`, the `xait_pid`/`service_relocated` extension bodies; `HEVC_video_descriptor` (a 12-byte body, or a 13-byte body disagreeing with `temporal_layer_subset_flag`); `FmxBufferSize_descriptor` bodies too short; `tables::sat` beamhopping plans with a short `plan_length`; `tables::protection_message` hash-length mismatches; `downloadable_font_info` length-delimited bodies using fixed-layout type codes 0x00 to 0x02; and BIOP IOR/profile-body slack (r03-W9, r03-W12, r03-W15, #1077).
- `FlexMuxTiming_descriptor` body length is now the spec's 8 bytes (was 10), and its serializer no longer leaks 2 stale buffer bytes (#1053).
- Reserved bits: parsers drop them (only spec-meaningful bits are stored), so re-serializing a stream with non-standard reserved-bit values normalises them; this is now stated at the crate root (r03-W8). ISO/IEC 13818-1 reserved bits are now emitted as `1` (hevc_video byte 12, hierarchy padding, audio_stream, avc_video byte 3), matching `ca`/`ac3` (r03-W7).
- `text::decode_dvb_string`: ISO 8859-9 and 8859-11 no longer go through windows-1254/874, so 0x80..=0x9F stay C1 controls and DVB Annex A's 0x86/0x87/0x8A codes are recognised in Turkish/Thai text.
- `EpgEvent` gained `extended_by_language: Vec<ExtendedEventLanguage>`. Extended-event fragments are grouped by `ISO_639_language_code`; `extended_text`/`extended_items` now equal the first language's entry rather than a language-blind concatenation.
- Capacity caps now evict instead of going silent. `collect::SectionSetCollector`/`EitCollector` evict the least-recently-touched key (`evicted_for_capacity()`, `sections_evicted_for_capacity()`, `schedules_evicted_for_capacity()`); `epg::EpgStore` evicts the least-recently-touched service and the earliest-starting event (`services_evicted_for_capacity()`, `events_evicted_for_capacity()`), and `feed_sdt` now respects `max_services`. `EpgStore`'s default and `with_max_services` derive the collector key cap as `max_services * 17` (#1002).
- `carousel::ModuleReassembler` records `(download_id, module_id) -> completed_version`, so a repeated DII of an already-delivered version no longer re-emits the module every cycle. New `retain`/`clear` prune slots and records for withdrawn modules.
- `carousel::biop::message::decompress_zlib` is capped at the new `MAX_DECOMPRESSED_MODULE_SIZE` (64 MiB); use the new `decompress_zlib_bounded(data, max_len)` for a tighter descriptor-declared size.
- `demux::SiDemux::follow_pat` now stops watching PMT PIDs a changed PAT no longer lists (never a well-known SI PID or an explicit `.pid(...)`).
- `AitDescriptorLoop`'s serde impl now surfaces parse errors as `{"parseError": ...}` like `DescriptorLoop`.

## Fixes

Parsing defects (wrong values for conformant input):

- `content_labeling_descriptor` 33-bit `content_time_base_value` / `metadata_time_base_value` read from the wrong bit position (`FE 00 00 00 01` decoded as 8522825728, not 1) (#1004).
- `J2K_video_descriptor` extended-capability body had `still_mode`/`interlaced_video` in the wrong position, misreading every later field when `extended_capability_flag = 1` (#1005).
- `Metadata_STD_descriptor` 22-bit leak-rate/buffer-size fields folded 2 reserved bits into the value (`C0 00 01` decoded as 12,582,913, not 1) (#1006).
- `FlexMuxTiming_descriptor` rejected every conformant descriptor (#1053). The four items above are verified against a `tstabcomp`-compiled PMT fixture (`fixtures/dvb-si/tsduck-w4-spec-misread-pmt.{xml,bin}`).
- `EitCollector` schedule-range reset dropped already-completed sub-tables permanently; it now re-feeds them.
- `dsmcc` `private_indicator` is round-tripped instead of forced to 0 when `section_syntax_indicator` is set.
- `descriptors::hierarchy::HierarchyType::from_u8` panicked for values over 15; it now masks to 4 bits.
- `transport_protocol_descriptor`'s truncated HTTP selector is now `Unknown` instead of a partial `Http` (r03-W14).
- `DTS-HD` `substream_length` now bounds its substream, and the serializer no longer writes it one byte short.

Serializer defects (corrupt or panicking output):

- `CaDescriptor` `ca_pid` over 13 bits silently wrapped into a different PID; now rejected.
- `IdSelector::serialize_into_at` and DTS-HD `SubstreamInfo::serialize_into` panicked on a short buffer; they now return `Error::OutputBufferTooSmall`.
- Length-guard bypass: eight table serializers cast to `u16` before comparing against the 12-bit maximum, so 65,536-byte bodies passed the guard (#1129).
- RCT/CIT nested lengths (`uri_length`, `number_items`, `promotional_text_length`, `link_info_length`, `number_of_links`, `unique_string_length`) truncated with no guard (#1129, #1003).
- Serializers that reserved bytes from a flag/depth field but wrote them from an `Option` could return `Ok(len)` with stale buffer bytes when the two disagreed (hevc_video `temporal_sub`, CPCM USI flags, T2 cells, `target_region_name`) (r03-W6).
- Over-wide values now error instead of corrupting a byte (`MB_buffer_size`, `TB_leak_rate`, `maximum_bitrate`, `country_region_id`, hevc_video fields, `network_change_notify`, `service_prominence`, `video_depth_range`, DTS-HD with more than 8 assets) (r03-W3).

Robustness:

- BIOP/IOR wire-length arithmetic uses checked addition, so on a 32-bit target `messageBody_length = 0xFFFFFFFF` can no longer wrap `usize` past the bounds check; it returns `Error::SectionLengthOverflow` (64-bit targets were not affected).

## Internal and tooling

Not user-visible API: `tables::write_section_length` and `descriptors::write_descriptor_header` are crate-private helpers that every serializer now routes its outer length write through; a source-scan test (`dvb-si/tests/serializer_length_truncation.rs`) fails CI if an unchecked `.len() as u8/u16/u32` returns under `src/descriptors/`, `src/tables/` or `src/carousel/`; section-header bit masks are named constants in `tables/mod.rs` with a drift guard (r02-W16); MPE-FEC and MPE-IFEC share one `real_time_parameters` codec (r02-W22); `teletext` and `VBI_teletext` share one entry loop and the `multilingual_*`/`application_name` descriptors share one `lang_text` reader/writer (#1141).

---

Published from tag `dvb-si-v11.0.0`.
