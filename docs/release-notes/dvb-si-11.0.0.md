# dvb-si 11.0.0

_Released 2026-10-05._

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
- Added a shared `dvb_si::tables::write_section_length` helper that every
  table serializer now routes its outer `section_length` write through, so
  the guard cannot be forgotten again for a future table.
- ~80 descriptor serializers (starting from the `network_name_descriptor`
  template) wrote the outer 8-bit `descriptor_length` header byte with no
  range check, so a body over 255 bytes wrapped mod 256 and misframed the
  rest of the descriptor loop (#1129). Added a shared
  `dvb_si::descriptors::write_descriptor_header` helper (tag + checked 8-bit
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
  missing validation. Added `Ior::parse_at` (returns the real consumed
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
Fixes GHSA-hxv4-gqm8-whw6 and GHSA-h6j8-r8j3-36xg.

---

Published from tag `dvb-si-v11.0.0`.
