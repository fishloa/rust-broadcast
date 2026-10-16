# dvb-si coverage reference

<!-- GENERATED FILE — do not edit by hand. Regenerate with
     `python3 tools/gen-dvbsi-coverage.py`; `cargo test -p dvb-si --test docs_coverage_drift`
     fails when this file is stale. -->

Everything `dvb-si` parses **and** serializes, generated from the crate's own dispatch lists
(`declare_tables!`, `declare_descriptors!`, `declare_extension_bodies!`,
`declare_ait_descriptors!`, `Mpeg2ExtensionBody`) and each module's `//!` spec citation,
joined with the allocation lists the vendored standards define
([`coverage/allocations.tsv`](coverage/allocations.tsv)). Rows the standards allocate but the
crate does not type are listed as **Not implemented** — nothing is hidden.

Columns: **Clause (verified)** is the clause of the named edition, checked against the spec
text; **Source cites** is what the module's own doc comment says (see the
[citation cross-check](#citation-cross-check) for disagreements). Status legend:

- **Full** — typed struct, symmetric `Parse`/`Serialize`, round-trip tested, auto-dispatched.
- **Partial** — typed, but a nested sub-structure is deliberately kept as bytes (reason given).
- **Typed-opaque** — dispatched to its own type, whose payload is a byte run defined by another standard.
- **Opt-in** — typed, but only decoded inside the private_data_specifier / registration scope
  its standard requires; enable it on a `DescriptorRegistry`.
- **Not implemented** — the bytes are preserved (round-trip stays lossless) but not typed.

## Summary

| | |
|---|---|
| Section types (table_id) | **31** typed `*Section` parsers covering **72** table_id values — every allocated EN 300 468 Table 2 id |
| Descriptors (tag 0x00–0x7F) | **116** auto-dispatched: **52** MPEG-2 / DSM-CC (0x02–0x3F) + all **64** DVB tags (0x40–0x7F) |
| DVB extension descriptors (0x7F) | **30** typed `descriptor_tag_extension` bodies — every one EN 300 468 V1.20.1 Table 109 allocates |
| MPEG-2 extension descriptors (0x3F) | **22** typed `extension_descriptor_tag` bodies (7 MPEG-H 3D audio, plus HEVC/VVC/EVC/LCEVC/JPEG XS timing and structure, green, quality, OD update, timed metadata, media service kind) |
| AIT descriptors (TS 102 809 / MHP) | **21** typed in the separate AIT descriptor namespace |
| INT / UNT / RNT descriptors | **26** typed in their own scoped namespaces (`AnyIntDescriptor`, `AnyUntDescriptor`, `AnyRntDescriptor`) |
| Private / regional descriptors | **9** PDS- or registration-scoped opt-ins (NorDig, EACEM/D-Book, China GY/T + T/AI AVS2/AVS3, T/UWA) |
| Text (EN 300 468 Annex A) | **7** charset decoders covering every Table A.3 selector + Annex A.1 control codes |
| DSM-CC carousel | **12** layers: section framing, DSI/DII/DDB, module reassembly, BIOP object carousel, carousel file tree |
| Registries | **171** CA_system_id and **174** private_data_specifier names (TSDuck data, build-time) |
| Standards | **19** standards / families — see the table below |
| Known gaps | **9** allocated codes not yet typed — listed in [`docs/COVERAGE.md`](https://github.com/fishloa/rust-broadcast/blob/main/dvb-si/docs/COVERAGE.md#not-implemented) |

## Standards

| Standard | Title | Edition implemented | Scope covered | Status |
|---|---|---|---|---|
| ETSI EN 300 468 | DVB Service Information (SI) | V1.20.1 (2026-09); RFU defaults audited back to V1.3.1 | all SI tables (NIT, BAT, SDT, EIT, TDT, TOT, RST, ST, SAT, DIT, SIT), Table 12 descriptors 0x40–0x7F, Table 109 extension descriptors, Annex A text, Annexes D/G/H/L audio descriptors | Full (98/98 allocated codes) |
| Rec. ITU-T H.222.0 \| ISO/IEC 13818-1 | MPEG-2 Systems — PSI and descriptors | 08/2023 text (clause numbers in COVERAGE.md); MPEG-H 3D audio descriptors 06/2021 | PAT, CAT, PMT, TSDT; Table 2-45 descriptors; Table 2-110 extension descriptors | Partial (80/85 allocated codes) |
| ISO/IEC 13818-6 | DSM-CC (data/object carousel) | as profiled by ETSI TR 101 202 V1.2.1 (13818-6 itself is not vendored) | DSM-CC sections, DSI/DII/DDB, module reassembly, BIOP object carousel, carousel_identifier + association_tag descriptors | Partial (3/5 allocated codes) |
| ETSI EN 301 192 | DVB data broadcasting | V1.7.1 | MPE datagram, MPE-FEC, INT section; time_slice_fec_identifier + ECM_repetition_rate descriptors; DII carousel descriptors | Full (29/29 allocated codes) |
| ETSI TS 102 006 | DVB System Software Update (SSU) | V1.4.1 | UNT section, compatibilityDescriptor, SSU data_broadcast_id selector | Full (16/16 allocated codes) |
| ETSI TS 102 809 | Application signalling (HbbTV AIT) | V1.3.1 | AIT section, AIT descriptor namespace, application_signalling + service_identifier descriptors, protection message section + descriptor | Full (16/16 allocated codes) |
| ETSI TS 102 727 | DVB-MHP 1.1.3 | V1.1.1 (descriptor subset) | DVB-J application + location AIT descriptors, XAIT_location, XAIT_PID | Partial (11/12 allocated codes) |
| ETSI TS 102 323 | TV-Anytime carriage in DVB | V1.4.1 | container, RCT, CIT, RNT sections; default_authority, related_content, TVA_id, content_identifier descriptors | Full (11/11 allocated codes) |
| ETSI TS 102 772 | MPE-IFEC | V1.1.1 | MPE-IFEC section | Full (1/1 allocated codes) |
| ETSI EN 303 560 | DVB TTML subtitling | V1.1.1 | TTML_subtitling descriptor, downloadable font info section (DFIS) | Full (2/2 allocated codes) |
| ETSI TS 102 825-9 | CPCM system adaptation | V1.2.1 | cpcm_delivery_signalling extension descriptor | Full (1/1 allocated codes) |
| ETSI TS 101 154 | Audio/video coding in DVB | V2.10.1 | PMT `StreamType` values incl. AVS2/AVS3 (0xD2–0xD5); AVS private_data_specifier rules | Partial |
| ETSI TS 101 162 | DVB identifiers | registry data from TSDuck (see registries/README.md) | CA_system_id and private_data_specifier value → name lookups | Partial |
| NorDig Unified Requirements | Nordic IRD requirements | v3.2.2 (2025-11) | logical channel descriptor v1 (0x83) + v2 (0x87), PDS 0x00000029 | Full (scoped) |
| EACEM / D-Book | Logical channel numbering | private (PDS 0x00000028); no vendored spec | logical_channel descriptor (0x83) | Full (scoped) |
| GY/T 270-2013 | China DTV closed captions | 2013 | caption_service_descriptor (0x86), `StreamType::ChinaClosedCaption` | Full (scoped) |
| GY/T 420-2025, GY/T 422-2025 | China UHD encapsulation / multi-audio | 2025 | AVS2_video (0x40 under 'AVSV'), AVS3_video; GY/T 422 audio_preselection profile | Full (scoped) |
| T/AI 109.6-2022, T/AI 109.7-2024 | AVS3 media format / audio transport | 2022 / 2024 | AVS3_video (0xD1), AVS3_audio (0xD2), AVS2_audio (0xD3) | Full (scoped) |
| T/UWA 012.2-2023 | UWA UHD TS encapsulation | 2023 | older AVS3 video descriptor (tag 62) | Full (scoped) |

## Tables (table_id)

`AnyTableSection::parse` dispatches every implemented row except the MPE datagram view;
`TableRegistry` adds private table_ids at runtime. User-defined (0x80–0xFE) and reserved
values are omitted.

| table_id | Section | Standard | Clause (verified) | Source cites | Rust type | Status |
|---|---|---|---|---|---|---|
| 0x00 | program_association_section | ITU-T H.222.0 (08/2023) | 2.4.4.4 | MPEG-2 ISO/IEC 13818-1 (H.222.0 08/2023) §2.4.4.4 | [`PatSection`](../src/tables/pat.rs) | Full |
| 0x01 | conditional_access_section | ITU-T H.222.0 (08/2023) | 2.4.4.7 | ISO/IEC 13818-1 (H.222.0 08/2023) §2.4.4.7 | [`CatSection`](../src/tables/cat.rs) | Full |
| 0x02 | TS_program_map_section | ITU-T H.222.0 (08/2023) | 2.4.4.9 | MPEG-2 ISO/IEC 13818-1 (H.222.0 08/2023) §2.4.4.9 | [`PmtSection`](../src/tables/pmt.rs) | Full |
| 0x03 | TS_description_section | ITU-T H.222.0 (08/2023) | 2.4.4.13 | ISO/IEC 13818-1 (H.222.0 08/2023) §2.4.4.13 | [`TsdtSection`](../src/tables/tsdt.rs) | Full |
| 0x04 | ISO_IEC_14496_scene_description_section | ITU-T H.222.0 (08/2023) | 2.11.3.8 | Rec. ITU-T H.222.0 (08/2023) \| ISO/IEC 13818-1:2023 §2.11.3.8, Table 2-153 (table_id 0x04 scene description, 0x05 object descriptor, 0x08 other … | [`Iso14496Section`](../src/tables/iso14496_section.rs) | Partial — payload (`SL_Packet()` / `M4MuxPacket()`s, selected by the carrying PMT) kept opaque (ISO/IEC 14496-1) |
| 0x05 | ISO_IEC_14496_object_descriptor_section | ITU-T H.222.0 (08/2023) | 2.11.3.8 | Rec. ITU-T H.222.0 (08/2023) \| ISO/IEC 13818-1:2023 §2.11.3.8, Table 2-153 (table_id 0x04 scene description, 0x05 object descriptor, 0x08 other … | [`Iso14496Section`](../src/tables/iso14496_section.rs) | Partial — payload (`SL_Packet()` / `M4MuxPacket()`s, selected by the carrying PMT) kept opaque (ISO/IEC 14496-1) |
| 0x06 | Metadata_section | ITU-T H.222.0 (08/2023) | 2.12.6 | Rec. ITU-T H.222.0 (08/2023) \| ISO/IEC 13818-1:2023 §2.12.6, Tables 2-158/2-159 (table_id 0x06); syntax in … | [`MetadataSection`](../src/tables/metadata_section.rs) | Partial — `metadata_byte`s (the metadata Access Unit) kept opaque |
| 0x07 | IPMP_Control_Information_section (ISO/IEC 13818-11) | ITU-T H.222.0 (08/2023) | 2.4.4.5 | — | — | Not implemented — dispatched as `AnyTableSection::Unknown` (section bytes kept) |
| 0x08 | ISO_IEC_14496_section | ITU-T H.222.0 (08/2023) | 2.11.3.8 | Rec. ITU-T H.222.0 (08/2023) \| ISO/IEC 13818-1:2023 §2.11.3.8, Table 2-153 (table_id 0x04 scene description, 0x05 object descriptor, 0x08 other … | [`Iso14496Section`](../src/tables/iso14496_section.rs) | Partial — payload (`SL_Packet()` / `M4MuxPacket()`s, selected by the carrying PMT) kept opaque (ISO/IEC 14496-1) |
| 0x09 | ISO/IEC 23001-11 green access unit section | ITU-T H.222.0 (08/2023) | 2.18.1 | Rec. ITU-T H.222.0 (08/2023) \| ISO/IEC 13818-1:2023 §2.18.1–2.18.3, Tables 2-162/2-163 (table_id 0x09); syntax in … | [`GreenAccessUnitSection`](../src/tables/green_access_unit_section.rs) | Partial — `Green_AU()` kept opaque (its loops are bounded by the out-of-band Green extension descriptor) |
| 0x0A | ISO/IEC 23001-10 quality access unit section | ITU-T H.222.0 (08/2023) | 2.4.4.5 | — | — | Not implemented — dispatched as `AnyTableSection::Unknown` (section bytes kept) |
| 0x0B | ISO/IEC 23001-13 media orchestration access unit section | ITU-T H.222.0 (08/2023) | 2.4.4.5 | — | — | Not implemented — dispatched as `AnyTableSection::Unknown` (section bytes kept) |
| 0x38–0x39 | DSM-CC sections (ISO/IEC 13818-6 range) | ISO/IEC 13818-6 | — | — | — | Not implemented — dispatched as `AnyTableSection::Unknown` (section bytes kept) |
| 0x3A–0x3F | DSM-CC sections (multiprotocol encapsulation, U-N messages, DDB, stream descriptors, private) | ISO/IEC 13818-6 / ETSI EN 301 192 V1.7.1 | — | ISO/IEC 13818-6 + ETSI EN 301 192 §9 | [`DsmccSection`](../src/tables/dsmcc.rs) | Full |
| 0x40 | network_information_section (actual) | ETSI EN 300 468 V1.20.1 | 5.2.1 | ETSI EN 300 468 §5.2.1 | [`NitSection`](../src/tables/nit.rs) | Full |
| 0x41 | network_information_section (other) | ETSI EN 300 468 V1.20.1 | 5.2.1 | ETSI EN 300 468 §5.2.1 | [`NitSection`](../src/tables/nit.rs) | Full |
| 0x42 | service_description_section (actual) | ETSI EN 300 468 V1.20.1 | 5.2.3 | ETSI EN 300 468 §5.2.3 | [`SdtSection`](../src/tables/sdt.rs) | Full |
| 0x46 | service_description_section (other) | ETSI EN 300 468 V1.20.1 | 5.2.3 | ETSI EN 300 468 §5.2.3 | [`SdtSection`](../src/tables/sdt.rs) | Full |
| 0x4A | bouquet_association_section | ETSI EN 300 468 V1.20.1 | 5.2.2 | ETSI EN 300 468 §5.2.2 | [`BatSection`](../src/tables/bat.rs) | Full |
| 0x4B | update_notification_section (UNT) | ETSI TS 102 006 V1.4.1 | 9.4 | ETSI TS 102 006 V1.4.1 §9.4 | [`UntSection`](../src/tables/unt.rs) | Full |
| 0x4C | IP/MAC_notification_section (INT) | ETSI EN 301 192 V1.7.1 | 8.4 | ETSI EN 301 192 V1.7.1 §8.4 | [`IntSection`](../src/tables/int.rs) | Full |
| 0x4D | satellite_access_section (SAT) | ETSI EN 300 468 V1.20.1 | 5.2.11 | ETSI EN 300 468 §5.2.11 | [`SatSection`](../src/tables/sat.rs) | Full |
| 0x4E | event_information_section (actual, present/following) | ETSI EN 300 468 V1.20.1 | 5.2.4 | ETSI EN 300 468 §5.2.4 | [`EitSection`](../src/tables/eit.rs) | Full |
| 0x4F | event_information_section (other, present/following) | ETSI EN 300 468 V1.20.1 | 5.2.4 | ETSI EN 300 468 §5.2.4 | [`EitSection`](../src/tables/eit.rs) | Full |
| 0x50–0x5F | event_information_section (actual, schedule) | ETSI EN 300 468 V1.20.1 | 5.2.4 | ETSI EN 300 468 §5.2.4 | [`EitSection`](../src/tables/eit.rs) | Full |
| 0x60–0x6F | event_information_section (other, schedule) | ETSI EN 300 468 V1.20.1 | 5.2.4 | ETSI EN 300 468 §5.2.4 | [`EitSection`](../src/tables/eit.rs) | Full |
| 0x70 | time_date_section (TDT) | ETSI EN 300 468 V1.20.1 | 5.2.5 | ETSI EN 300 468 §5.2.5 | [`TdtSection`](../src/tables/tdt.rs) | Full |
| 0x71 | running_status_section (RST) | ETSI EN 300 468 V1.20.1 | 5.2.7 | ETSI EN 300 468 V1.20.1 §5.2.7 | [`RstSection`](../src/tables/rst.rs) | Full |
| 0x72 | stuffing_section (ST) | ETSI EN 300 468 V1.20.1 | 5.2.8 | ETSI EN 300 468 §5.2.8 | [`StSection`](../src/tables/st.rs) | Full |
| 0x73 | time_offset_section (TOT) | ETSI EN 300 468 V1.20.1 | 5.2.6 | ETSI EN 300 468 §5.2.6 | [`TotSection`](../src/tables/tot.rs) | Full |
| 0x74 | application_information_section (AIT) | ETSI TS 102 809 V1.3.1 | 5.3.4 | ETSI TS 102 809 §5.3.4 | [`AitSection`](../src/tables/ait.rs) | Full |
| 0x75 | container_section | ETSI TS 102 323 V1.4.1 | 7.3.1.4 | ETSI TS 102 323 v1.4.1 §7.3.1.4 (table_id 0x75) | [`ContainerSection`](../src/tables/container.rs) | Full |
| 0x76 | related_content_section (RCT) | ETSI TS 102 323 V1.4.1 | 10.4 | ETSI TS 102 323 v1.4.1 §10.4 | [`RctSection`](../src/tables/rct.rs) | Full |
| 0x77 | content_identifier_section (CIT) | ETSI TS 102 323 V1.4.1 | 12.2 | ETSI TS 102 323 v1.4.1 §12.2 | [`CitSection`](../src/tables/cit.rs) | Full |
| 0x78 | MPE-FEC_section | ETSI EN 301 192 V1.7.1 | 9.9 | ETSI EN 301 192 v1.7.1 §9.9 (table_id 0x78) | [`MpeFecSection`](../src/tables/mpe_fec.rs) | Full |
| 0x79 | resolution_provider_notification_section (RNT) | ETSI TS 102 323 V1.4.1 | 5.2.2 | ETSI TS 102 323 V1.4.1 §5.2.2 | [`RntSection`](../src/tables/rnt.rs) | Full |
| 0x7A | MPE-IFEC_section | ETSI TS 102 772 V1.1.1 | 5.2 | ETSI TS 102 772 v1.1.1 §5.2 (table_id 0x7A) | [`MpeIfecSection`](../src/tables/mpe_ifec.rs) | Full |
| 0x7B | protection_message_section | ETSI TS 102 809 V1.3.1 | 9 | ETSI TS 102 809 v1.3.1 §9 (table_id 0x7B) | [`ProtectionMessageSection`](../src/tables/protection_message.rs) | Full |
| 0x7C | downloadable_font_info_section (DFIS) | ETSI EN 303 560 V1.1.1 | — | ETSI EN 303 560 v1.1.1 §5.3.2.3.1 (table_id 0x7C) | [`DownloadableFontInfoSection`](../src/tables/downloadable_font_info.rs) | Full |
| 0x7E | discontinuity_information_section (DIT) | ETSI EN 300 468 V1.20.1 | 7.1.1 | ETSI EN 300 468 V1.20.1 §7.1.1 | [`DitSection`](../src/tables/dit.rs) | Full |
| 0x7F | selection_information_section (SIT) | ETSI EN 300 468 V1.20.1 | 7.1.2 | ETSI EN 300 468 §7.1.2 | [`SitSection`](../src/tables/sit.rs) | Full |

## Descriptors — MPEG-2 / DSM-CC (0x00–0x3F)

Allocated by Rec. ITU-T H.222.0 Table 2-45. Implemented tags are dispatched by
`AnyDescriptor` / `DescriptorLoop::iter` / `parse_loop`.

| tag | Descriptor | Standard | Clause (verified) | Source cites | Rust type | Status |
|---|---|---|---|---|---|---|
| 0x02 | video_stream_descriptor | ITU-T H.222.0 (08/2023) | 2.6.2 | ISO/IEC 13818-1 §2.6.2 (tag 0x02) | [`VideoStreamDescriptor`](../src/descriptors/video_stream.rs) | Full |
| 0x03 | audio_stream_descriptor | ITU-T H.222.0 (08/2023) | 2.6.4 | ISO/IEC 13818-1 §2.6.4 (tag 0x03) | [`AudioStreamDescriptor`](../src/descriptors/audio_stream.rs) | Full |
| 0x04 | hierarchy_descriptor | ITU-T H.222.0 (08/2023) | 2.6.6 | ISO/IEC 13818-1 §2.6.6 (tag 0x04) | [`HierarchyDescriptor`](../src/descriptors/hierarchy.rs) | Full |
| 0x05 | registration_descriptor | ITU-T H.222.0 (08/2023) | 2.6.8 | ISO/IEC 13818-1 (H.222.0 08/2023) §2.6.8 (tag 0x05) | [`RegistrationDescriptor`](../src/descriptors/registration.rs) | Full |
| 0x06 | data_stream_alignment_descriptor | ITU-T H.222.0 (08/2023) | 2.6.10 | ISO/IEC 13818-1 §2.6.10 (tag 0x06) | [`DataStreamAlignmentDescriptor`](../src/descriptors/data_stream_alignment.rs) | Full |
| 0x07 | target_background_grid_descriptor | ITU-T H.222.0 (08/2023) | 2.6.12 | ISO/IEC 13818-1 §2.6.12 (tag 0x07) | [`TargetBackgroundGridDescriptor`](../src/descriptors/target_background_grid.rs) | Full |
| 0x08 | video_window_descriptor | ITU-T H.222.0 (08/2023) | 2.6.14 | ISO/IEC 13818-1 §2.6.14 (tag 0x08) | [`VideoWindowDescriptor`](../src/descriptors/video_window.rs) | Full |
| 0x09 | CA_descriptor | ITU-T H.222.0 (08/2023) | 2.6.16 | ISO/IEC 13818-1 §2.6.16 (tag 0x09) | [`CaDescriptor`](../src/descriptors/ca.rs) | Full |
| 0x0A | ISO_639_language_descriptor | ITU-T H.222.0 (08/2023) | 2.6.18 | MPEG-2 ISO/IEC 13818-1 (H.222.0 08/2023) §2.6.18 (tag 0x0A) | [`Iso639LanguageDescriptor`](../src/descriptors/iso_639_language.rs) | Full |
| 0x0B | system_clock_descriptor | ITU-T H.222.0 (08/2023) | 2.6.20 | ISO/IEC 13818-1 §2.6.20 (tag 0x0B) | [`SystemClockDescriptor`](../src/descriptors/system_clock.rs) | Full |
| 0x0C | multiplex_buffer_utilization_descriptor | ITU-T H.222.0 (08/2023) | 2.6.22 | ISO/IEC 13818-1 §2.6.22 (tag 0x0C) | [`MultiplexBufferUtilizationDescriptor`](../src/descriptors/multiplex_buffer_utilization.rs) | Full |
| 0x0D | copyright_descriptor | ITU-T H.222.0 (08/2023) | 2.6.24 | ISO/IEC 13818-1 §2.6.24 (tag 0x0D) | [`CopyrightDescriptor`](../src/descriptors/copyright.rs) | Full |
| 0x0E | maximum_bitrate_descriptor | ITU-T H.222.0 (08/2023) | 2.6.26 | ISO/IEC 13818-1 §2.6.26 (tag 0x0E) | [`MaximumBitrateDescriptor`](../src/descriptors/maximum_bitrate.rs) | Full |
| 0x0F | private_data_indicator_descriptor | ITU-T H.222.0 (08/2023) | 2.6.28 | ISO/IEC 13818-1 §2.6.28 (tag 0x0F) | [`PrivateDataIndicatorDescriptor`](../src/descriptors/private_data_indicator.rs) | Full |
| 0x10 | smoothing_buffer_descriptor | ITU-T H.222.0 (08/2023) | 2.6.30 | ISO/IEC 13818-1 §2.6.30 (tag 0x10) | [`SmoothingBufferDescriptor`](../src/descriptors/smoothing_buffer.rs) | Full |
| 0x11 | STD_descriptor | ITU-T H.222.0 (08/2023) | 2.6.32 | ISO/IEC 13818-1 §2.6.32 (tag 0x11) | [`StdDescriptor`](../src/descriptors/std.rs) | Full |
| 0x12 | IBP_descriptor | ITU-T H.222.0 (08/2023) | 2.6.34 | ISO/IEC 13818-1 §2.6.34 (tag 0x12) | [`IbpDescriptor`](../src/descriptors/ibp.rs) | Full |
| 0x13 | carousel_identifier_descriptor | ISO/IEC 13818-6 | — | ISO/IEC 13818-6 / ETSI TR 101 202 §4.7.7.1 (tag 0x13) | [`CarouselIdentifierDescriptor`](../src/descriptors/carousel_identifier.rs) | Full |
| 0x14 | association_tag_descriptor | ISO/IEC 13818-6 | — | ISO/IEC 13818-6 §11.4.1 / ETSI TR 101 202 §4.7.7.2 (tag 0x14) | [`AssociationTagDescriptor`](../src/descriptors/association_tag.rs) | Full |
| 0x15 | deferred_association_tags_descriptor | ETSI EN 301 192 V1.7.1 | 11.3.3 | ETSI EN 301 192 V1.7.1 §11.3.3, Tables 64 and 65 (tag 0x15; "the decimal value of 21") | [`DeferredAssociationTagsDescriptor`](../src/descriptors/deferred_association_tags.rs) | Full |
| 0x16–0x1A | other ISO/IEC 13818-6 descriptors (Table 2-45 "19..26 defined in ISO/IEC 13818-6"; 13818-6 is not vendored and its layouts for tags 0x16-0x1A appear in no vendored spec: 0x17-0x1A are the stream-event / NPT descriptors, and TS 102 809 B.2.4.2 says NPT is obsolete and gives no layout) | ISO/IEC 13818-6 | — | — | — | Not implemented — surfaces as `AnyDescriptor::Unknown { tag, body }` — bytes kept, round-trips losslessly |
| 0x1B | MPEG-4_video_descriptor | ITU-T H.222.0 (08/2023) | 2.6.36 | ISO/IEC 13818-1 §2.6.36 (tag 0x1B) | [`Mpeg4VideoDescriptor`](../src/descriptors/mpeg4_video.rs) | Full |
| 0x1C | MPEG-4_audio_descriptor | ITU-T H.222.0 (08/2023) | 2.6.38 | ISO/IEC 13818-1 §2.6.38 (tag 0x1C) | [`Mpeg4AudioDescriptor`](../src/descriptors/mpeg4_audio.rs) | Full |
| 0x1D | IOD_descriptor | ITU-T H.222.0 (08/2023) | 2.6.40 | ISO/IEC 13818-1 §2.6.40, Table 2-75 (tag 0x1D) | [`IodDescriptor`](../src/descriptors/iod.rs) | Partial — `InitialObjectDescriptor()` kept opaque (ISO/IEC 14496-1) |
| 0x1E | SL_descriptor | ITU-T H.222.0 (08/2023) | 2.6.42 | ISO/IEC 13818-1 §2.6.42, Table 2-76 (tag 0x1E) | [`SlDescriptor`](../src/descriptors/sl.rs) | Full |
| 0x1F | FMC_descriptor | ITU-T H.222.0 (08/2023) | 2.6.44 | ISO/IEC 13818-1 §2.6.44, Table 2-77 (tag 0x1F) | [`FmcDescriptor`](../src/descriptors/fmc.rs) | Full |
| 0x20 | external_ES_ID_descriptor | ITU-T H.222.0 (08/2023) | 2.6.46 | ISO/IEC 13818-1 §2.6.46, Table 2-78 (tag 0x20) | [`ExternalEsIdDescriptor`](../src/descriptors/external_es_id.rs) | Full |
| 0x21 | MuxCode_descriptor | ITU-T H.222.0 (08/2023) | 2.6.48 | ISO/IEC 13818-1 §2.6.48, Table 2-79 (tag 0x21) | [`MuxcodeDescriptor`](../src/descriptors/muxcode.rs) | Partial — `MuxCodeTableEntry()` records kept opaque (ISO/IEC 14496-1) |
| 0x22 | M4MuxBufferSize_descriptor | ITU-T H.222.0 (08/2023) | 2.6.50 | ISO/IEC 13818-1 §2.6.50, Table 2-80 (tag 0x22) | [`FmxBufferSizeDescriptor`](../src/descriptors/fmx_buffer_size.rs) | Partial — `FlexMuxBufferDescriptor()` records kept as raw bytes (ISO/IEC 14496-1 §11.2) |
| 0x23 | multiplexBuffer_descriptor | ITU-T H.222.0 (08/2023) | 2.6.52 | ISO/IEC 13818-1 §2.6.52, Table 2-81 (tag 0x23) | [`MultiplexBufferDescriptor`](../src/descriptors/multiplex_buffer.rs) | Full |
| 0x24 | content_labeling_descriptor | ITU-T H.222.0 (08/2023) | 2.6.56 | ISO/IEC 13818-1 §2.6.56, Table 2-83 (tag 0x24) | [`ContentLabelingDescriptor`](../src/descriptors/content_labeling.rs) | Full |
| 0x25 | metadata_pointer_descriptor | ITU-T H.222.0 (08/2023) | 2.6.58 | ISO/IEC 13818-1 §2.6.58, Table 2-86 (tag 0x25) | [`MetadataPointerDescriptor`](../src/descriptors/metadata_pointer.rs) | Full |
| 0x26 | metadata_descriptor | ITU-T H.222.0 (08/2023) | 2.6.60 | ISO/IEC 13818-1 §2.6.60, Table 2-89 (tag 0x26) | [`MetadataDescriptor`](../src/descriptors/metadata.rs) | Full |
| 0x27 | metadata_STD_descriptor | ITU-T H.222.0 (08/2023) | 2.6.62 | ISO/IEC 13818-1 §2.6.62, Table 2-91 (tag 0x27) | [`MetadataStdDescriptor`](../src/descriptors/metadata_std.rs) | Full |
| 0x28 | AVC_video_descriptor | ITU-T H.222.0 (08/2023) | 2.6.64 | ISO/IEC 13818-1 §2.6.64 (tag 0x28) | [`AvcVideoDescriptor`](../src/descriptors/avc_video.rs) | Full |
| 0x29 | IPMP_descriptor (ISO/IEC 13818-11) | ISO/IEC 13818-11 | — | — | — | Not implemented — surfaces as `AnyDescriptor::Unknown { tag, body }` — bytes kept, round-trips losslessly |
| 0x2A | AVC_timing_and_HRD_descriptor | ITU-T H.222.0 (08/2023) | 2.6.66 | ISO/IEC 13818-1 §2.6.66 (tag 0x2A) | [`AvcTimingAndHrdDescriptor`](../src/descriptors/avc_timing_and_hrd.rs) | Full |
| 0x2B | MPEG-2_AAC_audio_descriptor | ITU-T H.222.0 (08/2023) | 2.6.68 | ISO/IEC 13818-1 §2.6.68 (tag 0x2B) | [`Mpeg2AacAudioDescriptor`](../src/descriptors/mpeg2_aac_audio.rs) | Full |
| 0x2C | M4MuxTiming_descriptor | ITU-T H.222.0 (08/2023) | 2.6.54 | ISO/IEC 13818-1 §2.6.54, Table 2-82 (tag 0x2C) | [`FlexMuxTimingDescriptor`](../src/descriptors/flex_mux_timing.rs) | Full |
| 0x2D | MPEG-4_text_descriptor | ITU-T H.222.0 (08/2023) | 2.6.70 | Rec. ITU-T H.222.0 (08/2023) \| ISO/IEC 13818-1:2023 §2.6.70–2.6.71, Table 2-96 (tag 0x2D); syntax in … | [`Mpeg4TextDescriptor`](../src/descriptors/mpeg4_text.rs) | Partial — `textConfig()` kept opaque (ISO/IEC 14496-17, not vendored) |
| 0x2E | MPEG-4_audio_extension_descriptor | ITU-T H.222.0 (08/2023) | 2.6.72 | Rec. ITU-T H.222.0 (08/2023) \| ISO/IEC 13818-1:2023 §2.6.72–2.6.73, Table 2-97 (tag 0x2E); syntax in … | [`Mpeg4AudioExtensionDescriptor`](../src/descriptors/mpeg4_audio_extension.rs) | Partial — `audioSpecificConfig()` kept opaque (ISO/IEC 14496-3, not vendored) |
| 0x2F | auxiliary_video_stream_descriptor | ITU-T H.222.0 (08/2023) | 2.6.74 | ISO/IEC 13818-1 §2.6.74, Table 2-98 (tag 0x2F) | [`AuxiliaryVideoStreamDescriptor`](../src/descriptors/auxiliary_video_stream.rs) | Full |
| 0x30 | SVC_extension_descriptor | ITU-T H.222.0 (08/2023) | 2.6.76 | ISO/IEC 13818-1 §2.6.76, Table 2-99 (tag 0x30) | [`SvcExtensionDescriptor`](../src/descriptors/svc_extension.rs) | Full |
| 0x31 | MVC_extension_descriptor | ITU-T H.222.0 (08/2023) | 2.6.78 | ISO/IEC 13818-1 §2.6.78, Table 2-100 (tag 0x31) | [`MvcExtensionDescriptor`](../src/descriptors/mvc_extension.rs) | Full |
| 0x32 | J2K_video_descriptor | ITU-T H.222.0 (08/2023) | 2.6.80 | ISO/IEC 13818-1 §2.6.80, Table 2-101 (tag 0x32) | [`J2kVideoDescriptor`](../src/descriptors/j2k_video.rs) | Full |
| 0x33 | MVC_operation_point_descriptor | ITU-T H.222.0 (08/2023) | 2.6.82 | Rec. ITU-T H.222.0 (08/2023) \| ISO/IEC 13818-1:2023 §2.6.82–2.6.83, Table 2-103 (tag 0x33); syntax in … | [`MvcOperationPointDescriptor`](../src/descriptors/mvc_operation_point.rs) | Full |
| 0x34 | MPEG2_stereoscopic_video_format_descriptor | ITU-T H.222.0 (08/2023) | 2.6.84 | Rec. ITU-T H.222.0 (08/2023) \| ISO/IEC 13818-1:2023 | [`Mpeg2StereoscopicVideoFormatDescriptor`](../src/descriptors/stereoscopic.rs) | Full |
| 0x35 | stereoscopic_program_info_descriptor | ITU-T H.222.0 (08/2023) | 2.6.86 | Rec. ITU-T H.222.0 (08/2023) \| ISO/IEC 13818-1:2023 | [`StereoscopicProgramInfoDescriptor`](../src/descriptors/stereoscopic.rs) | Full |
| 0x36 | stereoscopic_video_info_descriptor | ITU-T H.222.0 (08/2023) | 2.6.88 | Rec. ITU-T H.222.0 (08/2023) \| ISO/IEC 13818-1:2023 | [`StereoscopicVideoInfoDescriptor`](../src/descriptors/stereoscopic.rs) | Full |
| 0x37 | transport_profile_descriptor | ITU-T H.222.0 (08/2023) | 2.6.93 | Rec. ITU-T H.222.0 (08/2023) \| ISO/IEC 13818-1:2023 §2.6.93–2.6.94, Tables 2-111/2-112 (tag 0x37); syntax in … | [`TransportProfileDescriptor`](../src/descriptors/transport_profile.rs) | Full |
| 0x38 | HEVC_video_descriptor | ITU-T H.222.0 (08/2023) | 2.6.95 | ISO/IEC 13818-1 §2.6.95 (tag 0x38) | [`HevcVideoDescriptor`](../src/descriptors/hevc_video.rs) | Full |
| 0x39 | VVC_video_descriptor | ITU-T H.222.0 (08/2023) | 2.6.129 | Rec. ITU-T H.222.0 (08/2023) \| ISO/IEC 13818-1:2023 §2.6.129–2.6.130, Tables 2-133…2-138 (tag 0x39); syntax in … | [`VvcVideoDescriptor`](../src/descriptors/vvc_video.rs) | Full |
| 0x3A | EVC_video_descriptor | ITU-T H.222.0 (08/2023) | 2.6.133 | Rec. ITU-T H.222.0 (08/2023) \| ISO/IEC 13818-1:2023 §2.6.133–2.6.134, Table 2-140 (tag 0x3A); syntax in … | [`EvcVideoDescriptor`](../src/descriptors/evc_video.rs) | Full |
| 0x3F | extension_descriptor (MPEG-2) | ITU-T H.222.0 (08/2023) | 2.6.90 | Rec. ITU-T H.222.0 (2023) \| ISO/IEC 13818-1 clause 2.6.90, Table 2-109, tag **0x3F** (63); the descriptor's second-level selector … | [`Mpeg2ExtensionDescriptor`](../src/descriptors/mpeg2_extension.rs) | Full |

## Descriptors — DVB (0x40–0x7F)

Allocated by ETSI EN 300 468 V1.20.1 Table 12.

| tag | Descriptor | Standard | Clause (verified) | Source cites | Rust type | Status |
|---|---|---|---|---|---|---|
| 0x40 | network_name_descriptor | ETSI EN 300 468 V1.20.1 | 6.2.27 | ETSI EN 300 468 V1.20.1 §6.2.27 (tag 0x40) | [`NetworkNameDescriptor`](../src/descriptors/network_name.rs) | Full |
| 0x41 | service_list_descriptor | ETSI EN 300 468 V1.20.1 | 6.2.35 | ETSI EN 300 468 V1.20.1 §6.2.35 (tag 0x41) | [`ServiceListDescriptor`](../src/descriptors/service_list.rs) | Full |
| 0x42 | stuffing_descriptor | ETSI EN 300 468 V1.20.1 | 6.2.40 | ETSI EN 300 468 V1.20.1 §6.2.40 (tag 0x42) | [`StuffingDescriptor`](../src/descriptors/stuffing.rs) | Full |
| 0x43 | satellite_delivery_system_descriptor | ETSI EN 300 468 V1.20.1 | 6.2.13.2 | ETSI EN 300 468 §6.2.13.2 (tag 0x43) | [`SatelliteDeliverySystemDescriptor`](../src/descriptors/satellite_delivery_system.rs) | Full |
| 0x44 | cable_delivery_system_descriptor | ETSI EN 300 468 V1.20.1 | 6.2.13.1 | ETSI EN 300 468 §6.2.13.1 (tag 0x44) | [`CableDeliverySystemDescriptor`](../src/descriptors/cable_delivery_system.rs) | Full |
| 0x45 | VBI_data_descriptor | ETSI EN 300 468 V1.20.1 | 6.2.47 | ETSI EN 300 468 §6.2.47 (tag 0x45) | [`VbiDataDescriptor`](../src/descriptors/vbi_data.rs) | Full |
| 0x46 | VBI_teletext_descriptor | ETSI EN 300 468 V1.20.1 | 6.2.48 | ETSI EN 300 468 §6.2.48 (tag 0x46) | [`VbiTeletextDescriptor`](../src/descriptors/vbi_teletext.rs) | Full |
| 0x47 | bouquet_name_descriptor | ETSI EN 300 468 V1.20.1 | 6.2.4 | ETSI EN 300 468 V1.20.1 §6.2.4 (tag 0x47) | [`BouquetNameDescriptor`](../src/descriptors/bouquet_name.rs) | Full |
| 0x48 | service_descriptor | ETSI EN 300 468 V1.20.1 | 6.2.33 | ETSI EN 300 468 §6.2.33 (tag 0x48) | [`ServiceDescriptor`](../src/descriptors/service.rs) | Full |
| 0x49 | country_availability_descriptor | ETSI EN 300 468 V1.20.1 | 6.2.10 | ETSI EN 300 468 §6.2.10 (tag 0x49) | [`CountryAvailabilityDescriptor`](../src/descriptors/country_availability.rs) | Full |
| 0x4A | linkage_descriptor | ETSI EN 300 468 V1.20.1 | 6.2.19 | ETSI EN 300 468 §6.2.19 (tag 0x4A) | [`LinkageDescriptor`](../src/descriptors/linkage.rs) | Partial — linkage_type 0x09/0x0A (SSU), 0x0B/0x0C (IP/MAC), 0x20 (TTML fonts) and user-defined conditional blocks kept as raw `LinkageData::Other` |
| 0x4B | NVOD_reference_descriptor | ETSI EN 300 468 V1.20.1 | 6.2.26 | ETSI EN 300 468 V1.20.1 §6.2.26 (tag 0x4B) | [`NvodReferenceDescriptor`](../src/descriptors/nvod_reference.rs) | Full |
| 0x4C | time_shifted_service_descriptor | ETSI EN 300 468 V1.20.1 | 6.2.45 | ETSI EN 300 468 §6.2.45 (tag 0x4C) | [`TimeShiftedServiceDescriptor`](../src/descriptors/time_shifted_service.rs) | Full |
| 0x4D | short_event_descriptor | ETSI EN 300 468 V1.20.1 | 6.2.37 | ETSI EN 300 468 §6.2.37 (tag 0x4D) | [`ShortEventDescriptor`](../src/descriptors/short_event.rs) | Full |
| 0x4E | extended_event_descriptor | ETSI EN 300 468 V1.20.1 | 6.2.15 | ETSI EN 300 468 V1.20.1 §6.2.15 (tag 0x4E) | [`ExtendedEventDescriptor`](../src/descriptors/extended_event.rs) | Full |
| 0x4F | time_shifted_event_descriptor | ETSI EN 300 468 V1.20.1 | 6.2.44 | ETSI EN 300 468 §6.2.44 (tag 0x4F) | [`TimeShiftedEventDescriptor`](../src/descriptors/time_shifted_event.rs) | Full |
| 0x50 | component_descriptor | ETSI EN 300 468 V1.20.1 | 6.2.8 | ETSI EN 300 468 §6.2.8 (tag 0x50) | [`ComponentDescriptor`](../src/descriptors/component.rs) | Full |
| 0x51 | mosaic_descriptor | ETSI EN 300 468 V1.20.1 | 6.2.21 | ETSI EN 300 468 §6.2.21 (tag 0x51) | [`MosaicDescriptor`](../src/descriptors/mosaic.rs) | Full |
| 0x52 | stream_identifier_descriptor | ETSI EN 300 468 V1.20.1 | 6.2.39 | ETSI EN 300 468 V1.20.1 §6.2.39 (tag 0x52) | [`StreamIdentifierDescriptor`](../src/descriptors/stream_identifier.rs) | Full |
| 0x53 | CA_identifier_descriptor | ETSI EN 300 468 V1.20.1 | 6.2.5 | ETSI EN 300 468 V1.20.1 §6.2.5 (tag 0x53) | [`CaIdentifierDescriptor`](../src/descriptors/ca_identifier.rs) | Full |
| 0x54 | content_descriptor | ETSI EN 300 468 V1.20.1 | 6.2.9 | ETSI EN 300 468 §6.2.9 (tag 0x54) | [`ContentDescriptor`](../src/descriptors/content.rs) | Full |
| 0x55 | parental_rating_descriptor | ETSI EN 300 468 V1.20.1 | 6.2.28 | ETSI EN 300 468 V1.20.1 §6.2.28 (tag 0x55) | [`ParentalRatingDescriptor`](../src/descriptors/parental_rating.rs) | Full |
| 0x56 | teletext_descriptor | ETSI EN 300 468 V1.20.1 | 6.2.43 | ETSI EN 300 468 V1.20.1 §6.2.43 (tag 0x56) | [`TeletextDescriptor`](../src/descriptors/teletext.rs) | Full |
| 0x57 | telephone_descriptor | ETSI EN 300 468 V1.20.1 | 6.2.42 | ETSI EN 300 468 §6.2.42 (tag 0x57) | [`TelephoneDescriptor`](../src/descriptors/telephone.rs) | Full |
| 0x58 | local_time_offset_descriptor | ETSI EN 300 468 V1.20.1 | 6.2.20 | ETSI EN 300 468 §6.2.20 (tag 0x58) | [`LocalTimeOffsetDescriptor`](../src/descriptors/local_time_offset.rs) | Full |
| 0x59 | subtitling_descriptor | ETSI EN 300 468 V1.20.1 | 6.2.41 | ETSI EN 300 468 V1.20.1 §6.2.41 (tag 0x59) | [`SubtitlingDescriptor`](../src/descriptors/subtitling.rs) | Full |
| 0x5A | terrestrial_delivery_system_descriptor | ETSI EN 300 468 V1.20.1 | 6.2.13.4 | ETSI EN 300 468 §6.2.13.4 (tag 0x5A) | [`TerrestrialDeliverySystemDescriptor`](../src/descriptors/terrestrial_delivery_system.rs) | Full |
| 0x5B | multilingual_network_name_descriptor | ETSI EN 300 468 V1.20.1 | 6.2.24 | ETSI EN 300 468 §6.2.24 (tag 0x5B) | [`MultilingualNetworkNameDescriptor`](../src/descriptors/multilingual_network_name.rs) | Full |
| 0x5C | multilingual_bouquet_name_descriptor | ETSI EN 300 468 V1.20.1 | 6.2.22 | ETSI EN 300 468 §6.2.22 (tag 0x5C) | [`MultilingualBouquetNameDescriptor`](../src/descriptors/multilingual_bouquet_name.rs) | Full |
| 0x5D | multilingual_service_name_descriptor | ETSI EN 300 468 V1.20.1 | 6.2.25 | ETSI EN 300 468 §6.2.25 (tag 0x5D) | [`MultilingualServiceNameDescriptor`](../src/descriptors/multilingual_service_name.rs) | Full |
| 0x5E | multilingual_component_descriptor | ETSI EN 300 468 V1.20.1 | 6.2.23 | ETSI EN 300 468 §6.2.23 (tag 0x5E) | [`MultilingualComponentDescriptor`](../src/descriptors/multilingual_component.rs) | Full |
| 0x5F | private_data_specifier_descriptor | ETSI EN 300 468 V1.20.1 | 6.2.31 | ETSI EN 300 468 §6.2.31 (tag 0x5F) | [`PrivateDataSpecifierDescriptor`](../src/descriptors/private_data_specifier.rs) | Full |
| 0x60 | service_move_descriptor | ETSI EN 300 468 V1.20.1 | 6.2.36 | ETSI EN 300 468 V1.20.1 §6.2.36 (tag 0x60) | [`ServiceMoveDescriptor`](../src/descriptors/service_move.rs) | Full |
| 0x61 | short_smoothing_buffer_descriptor | ETSI EN 300 468 V1.20.1 | 6.2.38 | ETSI EN 300 468 §6.2.38 (tag 0x61) | [`ShortSmoothingBufferDescriptor`](../src/descriptors/short_smoothing_buffer.rs) | Full |
| 0x62 | frequency_list_descriptor | ETSI EN 300 468 V1.20.1 | 6.2.17 | ETSI EN 300 468 §6.2.17 (tag 0x62) | [`FrequencyListDescriptor`](../src/descriptors/frequency_list.rs) | Full |
| 0x63 | partial_transport_stream_descriptor | ETSI EN 300 468 V1.20.1 | 7.2.1 | ETSI EN 300 468 §7.2.1 (tag 0x63) | [`PartialTransportStreamDescriptor`](../src/descriptors/partial_transport_stream.rs) | Full |
| 0x64 | data_broadcast_descriptor | ETSI EN 300 468 V1.20.1 | 6.2.11 | ETSI EN 300 468 V1.20.1 §6.2.11 (tag 0x64) | [`DataBroadcastDescriptor`](../src/descriptors/data_broadcast.rs) | Partial — selector_byte kept raw (its meaning depends on data_broadcast_id) |
| 0x65 | scrambling_descriptor | ETSI EN 300 468 V1.20.1 | 6.2.32 | ETSI EN 300 468 §6.2.32 (tag 0x65) | [`ScramblingDescriptor`](../src/descriptors/scrambling.rs) | Full |
| 0x66 | data_broadcast_id_descriptor | ETSI EN 300 468 V1.20.1 | 6.2.12 | ETSI EN 300 468 V1.20.1 §6.2.12 (tag 0x66) | [`DataBroadcastIdDescriptor`](../src/descriptors/data_broadcast_id.rs) | Partial — id_selector typed for SSU (`data_broadcast_id` 0x000A); other ids kept as `IdSelector::Raw` |
| 0x67 | transport_stream_descriptor | ETSI EN 300 468 V1.20.1 | 6.2.46 | ETSI EN 300 468 §6.2.46 (tag 0x67, Table 105, PDF p. 109) | [`TransportStreamDescriptor`](../src/descriptors/transport_stream.rs) | Full |
| 0x68 | DSNG_descriptor | ETSI EN 300 468 V1.20.1 | 6.2.14 | ETSI EN 300 468 V1.20.1 §6.2.14 (tag 0x68, Table 52, PDF p. 78) | [`DsngDescriptor`](../src/descriptors/dsng.rs) | Full |
| 0x69 | PDC_descriptor | ETSI EN 300 468 V1.20.1 | 6.2.30 | ETSI EN 300 468 §6.2.30 (tag 0x69, Table 84, PDF p. 97) | [`PdcDescriptor`](../src/descriptors/pdc.rs) | Full |
| 0x6A | AC-3_descriptor | ETSI EN 300 468 V1.20.1 | D.3 | ETSI EN 300 468 V1.20.1 Annex D §D.3 (tag 0x6A) | [`Ac3Descriptor`](../src/descriptors/ac3.rs) | Full |
| 0x6B | ancillary_data_descriptor | ETSI EN 300 468 V1.20.1 | 6.2.2 | ETSI EN 300 468 V1.20.1 §6.2.2 (tag 0x6B, Table 15, PDF p. 55) | [`AncillaryDataDescriptor`](../src/descriptors/ancillary_data.rs) | Full |
| 0x6C | cell_list_descriptor | ETSI EN 300 468 V1.20.1 | 6.2.7 | ETSI EN 300 468 §6.2.7 (tag 0x6C, Table 24, PDF p. 58) | [`CellListDescriptor`](../src/descriptors/cell_list.rs) | Full |
| 0x6D | cell_frequency_link_descriptor | ETSI EN 300 468 V1.20.1 | 6.2.6 | ETSI EN 300 468 V1.20.1 §6.2.6 (tag 0x6D, Table 23, PDF p. 58) | [`CellFrequencyLinkDescriptor`](../src/descriptors/cell_frequency_link.rs) | Full |
| 0x6E | announcement_support_descriptor | ETSI EN 300 468 V1.20.1 | 6.2.3 | ETSI EN 300 468 §6.2.3 (tag 0x6E, Table 17, PDF p. 56) | [`AnnouncementSupportDescriptor`](../src/descriptors/announcement_support.rs) | Full |
| 0x6F | application_signalling_descriptor | ETSI TS 102 809 V1.3.1 | 5.3.5.1 | ETSI TS 102 809 V1.3.1 §5.3.5.1, Table 17 (tag 0x6F) | [`ApplicationSignallingDescriptor`](../src/descriptors/application_signalling.rs) | Full |
| 0x70 | adaptation_field_data_descriptor | ETSI EN 300 468 V1.20.1 | 6.2.1 | ETSI EN 300 468 §6.2.1 (tag 0x70, Table 13, PDF p. 54) | [`AdaptationFieldDataDescriptor`](../src/descriptors/adaptation_field_data.rs) | Full |
| 0x71 | service_identifier_descriptor | ETSI TS 102 809 V1.3.1 | 6.2.1 | ETSI TS 102 809 V1.3.1 §6.2.1, Table 39 (tag 0x71) | [`ServiceIdentifierDescriptor`](../src/descriptors/service_identifier.rs) | Full |
| 0x72 | service_availability_descriptor | ETSI EN 300 468 V1.20.1 | 6.2.34 | ETSI EN 300 468 V1.20.1 §6.2.34 (tag 0x72, Table 90, PDF p. 101) | [`ServiceAvailabilityDescriptor`](../src/descriptors/service_availability.rs) | Full |
| 0x73 | default_authority_descriptor | ETSI TS 102 323 V1.4.1 | 6.3.3 | ETSI TS 102 323 V1.4.1 §6.3.3 (tag 0x73) | [`DefaultAuthorityDescriptor`](../src/descriptors/default_authority.rs) | Full |
| 0x74 | related_content_descriptor | ETSI TS 102 323 V1.4.1 | — | ETSI TS 102 323 §10.4.1, Table 108 (tag 0x74) | [`RelatedContentDescriptor`](../src/descriptors/related_content.rs) | Full |
| 0x75 | TVA_id_descriptor | ETSI TS 102 323 V1.4.1 | 11.2.4 | ETSI TS 102 323 §11.2.4, Table 114 (tag 0x75) | [`TvaIdDescriptor`](../src/descriptors/tva_id.rs) | Full |
| 0x76 | content_identifier_descriptor | ETSI TS 102 323 V1.4.1 | — | ETSI TS 102 323 §12.1 (tag 0x76) | [`ContentIdentifierDescriptor`](../src/descriptors/content_identifier.rs) | Full |
| 0x77 | time_slice_fec_identifier_descriptor | ETSI EN 301 192 V1.7.1 | — | ETSI EN 301 192 §9.5 Table 40 / ETSI TS 102 772 §6.2 Table 4 (tag 0x77) | [`TimeSliceFecIdentifierDescriptor`](../src/descriptors/time_slice_fec_identifier.rs) | Full |
| 0x78 | ECM_repetition_rate_descriptor | ETSI EN 301 192 V1.7.1 | — | ETSI EN 301 192 §9.9, Table 44 (tag 0x78) | [`EcmRepetitionRateDescriptor`](../src/descriptors/ecm_repetition_rate.rs) | Full |
| 0x79 | S2_satellite_delivery_system_descriptor | ETSI EN 300 468 V1.20.1 | 6.2.13.3 | ETSI EN 300 468 §6.2.13.3 (tag 0x79) | [`S2SatelliteDeliverySystemDescriptor`](../src/descriptors/s2_satellite_delivery_system.rs) | Full |
| 0x7A | enhanced_AC-3_descriptor | ETSI EN 300 468 V1.20.1 | D.5 | ETSI EN 300 468 V1.20.1 Annex D §D.5 (tag 0x7A) | [`EnhancedAc3Descriptor`](../src/descriptors/enhanced_ac3.rs) | Full |
| 0x7B | DTS_descriptor | ETSI EN 300 468 V1.20.1 | G.2.1 | ETSI EN 300 468 V1.20.1 Annex G §G.2.1, Table G.1 (tag 0x7B) | [`DtsDescriptor`](../src/descriptors/dts.rs) | Full |
| 0x7C | AAC_descriptor | ETSI EN 300 468 V1.20.1 | H.2.1 | ETSI EN 300 468 V1.20.1 Annex H §H.2.1, Table H.1 (tag 0x7C) | [`AacDescriptor`](../src/descriptors/aac.rs) | Full |
| 0x7D | XAIT_location_descriptor | ETSI TS 102 727 V1.1.1 | — | ETSI TS 102 727 §10.17.6, Table 96 (tag 0x7D) | [`XaitLocationDescriptor`](../src/descriptors/xait_location.rs) | Full |
| 0x7E | FTA_content_management_descriptor | ETSI EN 300 468 V1.20.1 | 6.2.18 | ETSI EN 300 468 §6.2.18.1 (tag 0x7E, Table 57, PDF p. 82) | [`FtaContentManagementDescriptor`](../src/descriptors/fta_content_management.rs) | Full |
| 0x7F | extension_descriptor | ETSI EN 300 468 V1.20.1 | 6.2.16 | ETSI EN 300 468 V1.20.1 §6.2.16 (tag 0x7F) | [`ExtensionDescriptor`](../src/descriptors/extension/mod.rs) | Full |

## DVB extension descriptors (tag 0x7F, `descriptor_tag_extension`)

Allocated by ETSI EN 300 468 V1.20.1 Table 109 (0x12 and 0x1A–0x1F are reserved there).
`ExtensionRegistry` registers private extension bodies.

| tag_ext | Descriptor | Standard | Clause (verified) | Source cites | Rust type | Status |
|---|---|---|---|---|---|---|
| 0x00 | image_icon_descriptor | ETSI EN 300 468 V1.20.1 | 6.4.7 | ETSI EN 300 468 §6.4.7 (tag_extension 0x00) | [`ImageIcon`](../src/descriptors/extension/image_icon.rs) | Full |
| 0x01 | cpcm_delivery_signalling_descriptor | ETSI TS 102 825-9 V1.2.1 | — | ETSI TS 102 825-9 §4.1.5, Table 2 (tag_extension 0x01); typed CPCM USI decode per ETSI TS 102 825-4 §5.4 Table 8 | [`CpcmDeliverySignalling`](../src/descriptors/extension/cpcm_delivery_signalling.rs) | Full |
| 0x02 | CP_descriptor | ETSI EN 300 468 V1.20.1 | 6.4.3 | ETSI EN 300 468 §6.4.3, Table 113 (tag_extension 0x02) | [`Cp`](../src/descriptors/extension/cp.rs) | Full |
| 0x03 | CP_identifier_descriptor | ETSI EN 300 468 V1.20.1 | 6.4.4 | ETSI EN 300 468 V1.20.1 §6.4.4, Table 114 (tag_extension 0x03) | [`CpIdentifier`](../src/descriptors/extension/cp_identifier.rs) | Full |
| 0x04 | T2_delivery_system_descriptor | ETSI EN 300 468 V1.20.1 | 6.4.6.3 | ETSI EN 300 468 §6.4.6.3 (tag_extension 0x04) | [`T2DeliverySystem`](../src/descriptors/extension/t2_delivery_system.rs) | Full |
| 0x05 | SH_delivery_system_descriptor | ETSI EN 300 468 V1.20.1 | 6.4.6.2 | ETSI EN 300 468 §6.4.6.2 (tag_extension 0x05) | [`ShDeliverySystem`](../src/descriptors/extension/sh_delivery_system.rs) | Full |
| 0x06 | supplementary_audio_descriptor | ETSI EN 300 468 V1.20.1 | 6.4.11 | ETSI EN 300 468 §6.4.11 (tag_extension 0x06) | [`SupplementaryAudio`](../src/descriptors/extension/supplementary_audio.rs) | Full |
| 0x07 | network_change_notify_descriptor | ETSI EN 300 468 V1.20.1 | 6.4.9 | ETSI EN 300 468 §6.4.9 (tag_extension 0x07) | [`NetworkChangeNotify`](../src/descriptors/extension/network_change_notify.rs) | Full |
| 0x08 | message_descriptor | ETSI EN 300 468 V1.20.1 | 6.4.8 | ETSI EN 300 468 V1.20.1 §6.4.8 (tag_extension 0x08) | [`Message`](../src/descriptors/extension/message.rs) | Full |
| 0x09 | target_region_descriptor | ETSI EN 300 468 V1.20.1 | 6.4.12 | ETSI EN 300 468 §6.4.12 (tag_extension 0x09) | [`TargetRegion`](../src/descriptors/extension/target_region.rs) | Full |
| 0x0A | target_region_name_descriptor | ETSI EN 300 468 V1.20.1 | 6.4.13 | ETSI EN 300 468 §6.4.13 (tag_extension 0x0A) | [`TargetRegionName`](../src/descriptors/extension/target_region_name.rs) | Full |
| 0x0B | service_relocated_descriptor | ETSI EN 300 468 V1.20.1 | 6.4.10 | ETSI EN 300 468 §6.4.10 (tag_extension 0x0B) | [`ServiceRelocated`](../src/descriptors/extension/service_relocated.rs) | Full |
| 0x0C | XAIT_PID_descriptor | ETSI TS 102 727 V1.1.1 | 10.17.3 | ETSI TS 102 727 §10.17.3, Table 95 (extension descriptor: tag 0x7F, tag_extension 0x0C) | [`XaitPid`](../src/descriptors/extension/xait_pid.rs) | Full |
| 0x0D | C2_delivery_system_descriptor | ETSI EN 300 468 V1.20.1 | 6.4.6.1 | ETSI EN 300 468 §6.4.6.1 (tag_extension 0x0D) | [`C2DeliverySystem`](../src/descriptors/extension/c2_delivery_system.rs) | Full |
| 0x0E | DTS-HD_descriptor | ETSI EN 300 468 V1.20.1 | G.3.1 | ETSI EN 300 468 V1.20.1 Annex G §G.3.1, Tables G.6–G.10 (tag_extension 0x0E) | [`DtsHd`](../src/descriptors/extension/dts_hd.rs) | Full |
| 0x0F | DTS_Neural_descriptor | ETSI EN 300 468 V1.20.1 | L.1 | ETSI EN 300 468 Annex L.1, Table L.1 (tag_extension 0x0F) | [`DtsNeural`](../src/descriptors/extension/dts_neural.rs) | Full |
| 0x10 | video_depth_range_descriptor | ETSI EN 300 468 V1.20.1 | 6.4.16 | ETSI EN 300 468 §6.4.16.1 (tag_extension 0x10) | [`VideoDepthRange`](../src/descriptors/extension/video_depth_range.rs) | Full |
| 0x11 | T2MI_descriptor | ETSI EN 300 468 V1.20.1 | 6.4.14 | ETSI EN 300 468 §6.4.14 (tag_extension 0x11) | [`T2mi`](../src/descriptors/extension/t2mi.rs) | Full |
| 0x13 | URI_linkage_descriptor | ETSI EN 300 468 V1.20.1 | 6.4.15 | ETSI EN 300 468 V1.20.1 §6.4.15 (tag_extension 0x13) | [`UriLinkage`](../src/descriptors/extension/uri_linkage.rs) | Full |
| 0x14 | CI_ancillary_data_descriptor | ETSI EN 300 468 V1.20.1 | 6.4.2 | ETSI EN 300 468 V1.20.1 §6.4.2, Table 112 (tag_extension 0x14) | [`CiAncillaryData`](../src/descriptors/extension/ci_ancillary_data.rs) | Full |
| 0x15 | AC-4_descriptor | ETSI EN 300 468 V1.20.1 | D.7 | ETSI EN 300 468 V1.20.1 Annex D §D.7 (tag_extension 0x15) | [`Ac4`](../src/descriptors/extension/ac4.rs) | Partial — `ac4_toc()` kept opaque (its syntax is in ETSI TS 103 190-2; issue #102) |
| 0x16 | C2_bundle_delivery_system_descriptor | ETSI EN 300 468 V1.20.1 | 6.4.6.4 | ETSI EN 300 468 §6.4.6.4 (tag_extension 0x16) | [`C2BundleDeliverySystem`](../src/descriptors/extension/c2_bundle_delivery_system.rs) | Full |
| 0x17 | S2X_satellite_delivery_system_descriptor | ETSI EN 300 468 V1.20.1 | 6.4.6.5.2 | ETSI EN 300 468 §6.4.6.5.2 (tag_extension 0x17) | [`S2XSatelliteDeliverySystem`](../src/descriptors/extension/s2x_satellite_delivery_system.rs) | Full |
| 0x18 | protection_message_descriptor | ETSI TS 102 809 V1.3.1 | — | ETSI TS 102 809 §9.3.3, Table 40 (tag_extension 0x18) | [`ProtectionMessage`](../src/descriptors/extension/protection_message.rs) | Full |
| 0x19 | audio_preselection_descriptor | ETSI EN 300 468 V1.20.1 | 6.4.1 | ETSI EN 300 468 §6.4.1 (tag_extension 0x19) | [`AudioPreselection`](../src/descriptors/extension/audio_preselection.rs) | Full |
| 0x20 | TTML_subtitling_descriptor | ETSI EN 303 560 V1.1.1 | — | ETSI EN 303 560 §5.2.1.1 (tag_extension 0x20) | [`TtmlSubtitling`](../src/descriptors/extension/ttml_subtitling.rs) | Full |
| 0x21 | DTS-UHD_descriptor | ETSI EN 300 468 V1.20.1 | G.5.1 | ETSI EN 300 468 V1.20.1 Annex G §G.5.1, Table G.15 (tag_extension 0x21) | [`DtsUhd`](../src/descriptors/extension/dts_uhd.rs) | Full |
| 0x22 | service_prominence_descriptor | ETSI EN 300 468 V1.20.1 | 6.4.18 | ETSI EN 300 468 §6.4.18 (tag_extension 0x22) | [`ServiceProminence`](../src/descriptors/extension/service_prominence.rs) | Full |
| 0x23 | vvc_subpictures_descriptor | ETSI EN 300 468 V1.20.1 | 6.4.17 | ETSI EN 300 468 §6.4.17 (tag_extension 0x23) | [`VvcSubpictures`](../src/descriptors/extension/vvc_subpictures.rs) | Full |
| 0x24 | S2Xv2_satellite_delivery_system_descriptor | ETSI EN 300 468 V1.20.1 | 6.4.6.5.3 | ETSI EN 300 468 §6.4.6.5.3 (tag_extension 0x24) | [`S2Xv2SatelliteDeliverySystem`](../src/descriptors/extension/s2xv2_satellite_delivery_system.rs) | Full |

## MPEG-2 extension descriptors (tag 0x3F, `extension_descriptor_tag`)

Allocated by Rec. ITU-T H.222.0 (08/2023) Table 2-110.

| ext_tag | Descriptor | Standard | Clause (verified) | Source cites | Rust type | Status |
|---|---|---|---|---|---|---|
| 0x02 | ODUpdate_descriptor | ITU-T H.222.0 (08/2023) | 2.6.92 | Rec. ITU-T H.222.0 (08/2023) \| ISO/IEC 13818-1:2023, extension_descriptor_tag | [`OdUpdate`](../src/descriptors/mpeg2_ext_misc.rs) | Typed-opaque — `ObjectDescriptorUpdate()` kept as bytes (ISO/IEC 14496-1 §7.2.5.5.2, not vendored) |
| 0x03 | HEVC_timing_and_HRD_descriptor | ITU-T H.222.0 (08/2023) | 2.6.97 | Rec. ITU-T H.222.0 (08/2023) §2.6.97, §2.6.131, §2.6.135 | [`HevcTimingAndHrd`](../src/descriptors/h222_timing.rs) | Full |
| 0x04 | af_extensions_descriptor | ITU-T H.222.0 (08/2023) | 2.6.99 | Rec. ITU-T H.222.0 (08/2023) \| ISO/IEC 13818-1:2023, extension_descriptor_tag | [`AfExtensions`](../src/descriptors/mpeg2_ext_misc.rs) | Full |
| 0x05 | HEVC_operation_point_descriptor | ITU-T H.222.0 (08/2023) | 2.6.100 | Rec. ITU-T H.222.0 (08/2023) \| ISO/IEC 13818-1:2023 | [`HevcOperationPoint`](../src/descriptors/mpeg2_ext_hevc.rs) | Full |
| 0x06 | HEVC_hierarchy_extension_descriptor | ITU-T H.222.0 (08/2023) | 2.6.102 | Rec. ITU-T H.222.0 (08/2023) \| ISO/IEC 13818-1:2023 | [`HevcHierarchyExtension`](../src/descriptors/mpeg2_ext_hevc.rs) | Full |
| 0x07 | green_extension_descriptor | ITU-T H.222.0 (08/2023) | 2.6.104 | Rec. ITU-T H.222.0 (08/2023) \| ISO/IEC 13818-1:2023, extension_descriptor_tag | [`GreenExtension`](../src/descriptors/mpeg2_ext_misc.rs) | Full |
| 0x08 | MPEG-H_3dAudio_descriptor | ITU-T H.222.0 (08/2023) | 2.6.106 | Rec. ITU-T H.222.0 (06/2021 \| 08/2023) \| ISO/IEC 13818-1 clauses 2.6.106–2.6.118, Tables 2-121…2-127 | [`MpegH3dAudio`](../src/descriptors/mpegh_3d_audio.rs) | Full |
| 0x09 | MPEG-H_3dAudio_config_descriptor | ITU-T H.222.0 (08/2023) | 2.6.108 | Rec. ITU-T H.222.0 (06/2021 \| 08/2023) \| ISO/IEC 13818-1 clauses 2.6.106–2.6.118, Tables 2-121…2-127 | [`MpegH3dAudioConfig`](../src/descriptors/mpegh_3d_audio.rs) | Typed-opaque — payload kept as bytes (its syntax is defined in ISO/IEC 23008-3) |
| 0x0A | MPEG-H_3dAudio_scene_descriptor | ITU-T H.222.0 (08/2023) | 2.6.110 | Rec. ITU-T H.222.0 (06/2021 \| 08/2023) \| ISO/IEC 13818-1 clauses 2.6.106–2.6.118, Tables 2-121…2-127 | [`MpegH3dAudioScene`](../src/descriptors/mpegh_3d_audio.rs) | Full |
| 0x0B | MPEG-H_3dAudio_text_label_descriptor | ITU-T H.222.0 (08/2023) | 2.6.112 | Rec. ITU-T H.222.0 (06/2021 \| 08/2023) \| ISO/IEC 13818-1 clauses 2.6.106–2.6.118, Tables 2-121…2-127 | [`MpegH3dAudioTextLabel`](../src/descriptors/mpegh_3d_audio.rs) | Full |
| 0x0C | MPEG-H_3dAudio_multi-stream_descriptor | ITU-T H.222.0 (08/2023) | 2.6.114 | Rec. ITU-T H.222.0 (06/2021 \| 08/2023) \| ISO/IEC 13818-1 clauses 2.6.106–2.6.118, Tables 2-121…2-127 | [`MpegH3dAudioMultiStream`](../src/descriptors/mpegh_3d_audio.rs) | Full |
| 0x0D | MPEG-H_3dAudio_drc_loudness_descriptor | ITU-T H.222.0 (08/2023) | 2.6.116 | Rec. ITU-T H.222.0 (06/2021 \| 08/2023) \| ISO/IEC 13818-1 clauses 2.6.106–2.6.118, Tables 2-121…2-127 | [`MpegH3dAudioDrcLoudness`](../src/descriptors/mpegh_3d_audio.rs) | Full |
| 0x0E | MPEG-H_3dAudio_command_descriptor | ITU-T H.222.0 (08/2023) | 2.6.118 | Rec. ITU-T H.222.0 (06/2021 \| 08/2023) \| ISO/IEC 13818-1 clauses 2.6.106–2.6.118, Tables 2-121…2-127 | [`MpegH3dAudioCommand`](../src/descriptors/mpegh_3d_audio.rs) | Typed-opaque — payload kept as bytes (its syntax is defined in ISO/IEC 23008-3) |
| 0x0F | quality_extension_descriptor | ITU-T H.222.0 (08/2023) | 2.6.119 | Rec. ITU-T H.222.0 (08/2023) \| ISO/IEC 13818-1:2023, extension_descriptor_tag | [`QualityExtension`](../src/descriptors/mpeg2_ext_misc.rs) | Full |
| 0x10 | virtual_segmentation_descriptor | ITU-T H.222.0 (08/2023) | 2.6.120 | — | — | Not implemented — kept as `Mpeg2ExtensionBody::Other` — round-trips losslessly |
| 0x11 | timed_metadata_extension_descriptor | ITU-T H.222.0 (08/2023) | — | Rec. ITU-T H.222.0 (08/2023) \| ISO/IEC 13818-1:2023, extension_descriptor_tag | [`TimedMetadataExtension`](../src/descriptors/mpeg2_ext_misc.rs) | Typed-opaque — body kept as bytes (ISO/IEC 23001-13 clause 8.2.3, not vendored) |
| 0x12 | HEVC_tile_substream_descriptor | ITU-T H.222.0 (08/2023) | 2.6.122 | Rec. ITU-T H.222.0 (08/2023) \| ISO/IEC 13818-1:2023 | [`HevcTileSubstream`](../src/descriptors/mpeg2_ext_hevc.rs) | Full |
| 0x13 | HEVC_subregion_descriptor | ITU-T H.222.0 (08/2023) | 2.6.125 | — | — | Not implemented — kept as `Mpeg2ExtensionBody::Other` — round-trips losslessly |
| 0x14 | JXS_video_descriptor | ITU-T H.222.0 (08/2023) | 2.6.127 | Rec. ITU-T H.222.0 (08/2023) \| ISO/IEC 13818-1:2023, extension_descriptor_tag | [`JxsVideo`](../src/descriptors/mpeg2_ext_misc.rs) | Full |
| 0x15 | VVC_timing_and_HRD_descriptor | ITU-T H.222.0 (08/2023) | 2.6.131 | Rec. ITU-T H.222.0 (08/2023) §2.6.97, §2.6.131, §2.6.135 | [`VvcTimingAndHrd`](../src/descriptors/h222_timing.rs) | Full |
| 0x16 | EVC_timing_and_HRD_descriptor | ITU-T H.222.0 (08/2023) | 2.6.135 | Rec. ITU-T H.222.0 (08/2023) §2.6.97, §2.6.131, §2.6.135 | [`EvcTimingAndHrd`](../src/descriptors/h222_timing.rs) | Full |
| 0x17 | LCEVC_video_descriptor | ITU-T H.222.0 (08/2023) | 2.6.137 | Rec. ITU-T H.222.0 (08/2023) \| ISO/IEC 13818-1:2023 | [`LcevcVideo`](../src/descriptors/mpeg2_ext_codecs.rs) | Full |
| 0x18 | LCEVC_linkage_descriptor | ITU-T H.222.0 (08/2023) | 2.6.139 | Rec. ITU-T H.222.0 (08/2023) \| ISO/IEC 13818-1:2023 | [`LcevcLinkage`](../src/descriptors/mpeg2_ext_codecs.rs) | Typed-opaque — payload kept as bytes (its syntax is defined in ISO/IEC 23008-3) |
| 0x19 | media_service_kind_descriptor | ITU-T H.222.0 (08/2023) | 2.6.141 | Rec. ITU-T H.222.0 (08/2023) \| ISO/IEC 13818-1:2023 | [`MediaServiceKind`](../src/descriptors/mpeg2_ext_codecs.rs) | Full |

## Private and regional descriptors (PDS / registration scoped)

These tags are private (0x80–0xFE) or re-use a public tag inside a private scope, so they
are **never auto-dispatched**: outside their scope they stay `AnyDescriptor::Unknown` (or
the public descriptor — e.g. 0x40 stays `network_name`). Enable them on a
`DescriptorRegistry`; a `private_data_specifier_descriptor` earlier in the same loop (or a
`registration_descriptor` for the AVS family) sets the scope automatically.

| tag | Rust type | Standard (source cites) | Scope | Opt-in | Status |
|---|---|---|---|---|---|
| 0x3E | [`TuwaAvs3VideoDescriptor`](../src/descriptors/tuwa_avs3_video.rs) | Older AVS3 video descriptor of T/UWA 012.2-2023 clause 5.4.1 (Table 1), tag 62 (0x3E) - the "Bai Cheng Qian Ping" UHD system descriptor that predates … | none — explicit, PDS-agnostic opt-in | `DescriptorRegistry::with_tuwa_avs3_video()` | Opt-in |
| 0x40 | [`Avs2VideoDescriptor`](../src/descriptors/avs2_video.rs) | GY/T 420-2025 clause 7.2.3 (Table 3), tag 0x40 (64), carried in the PMT ES_info loop of an AVS2 video elementary stream (stream_type 0xD2, see … | `PDS_AVSV` (private_data_specifier or registration_descriptor) | `DescriptorRegistry::with_avs2_video()` | Opt-in |
| 0x83 | [`LogicalChannelDescriptor`](../src/descriptors/logical_channel.rs) | EACEM/NorDig/D-Book private (tag 0x83) | any PDS (global) or a chosen PDS (`with_logical_channel_for_pds`, e.g. `PDS_EACEM` / `PDS_NORDIG`) | `DescriptorRegistry::with_logical_channel()`, `DescriptorRegistry::with_logical_channel_for_pds()` | Opt-in |
| 0x83 | [`NordigLogicalChannelV1`](../src/descriptors/nordig.rs) | NorDig Unified Requirements for Integrated Receiver Decoders v3.2.2 (2025-11) §12.2.9.2–12.2.9.3, Tables 12.12 and 12.14 (tags 0x83, 0x87; the syntax … | `PDS_NORDIG` (private_data_specifier or registration_descriptor) | `DescriptorRegistry::with_nordig_lcn()` | Opt-in |
| 0x86 | [`ChinaCaptionServiceDescriptor`](../src/descriptors/china_caption_service.rs) | GY/T 270-2013 clause 6.4 (Table 8, character sets Table 9), tag 0x86 | none — explicit, PDS-agnostic opt-in | `DescriptorRegistry::with_china_caption_service()` | Opt-in |
| 0x87 | [`NordigLogicalChannelV2`](../src/descriptors/nordig.rs) | NorDig Unified Requirements for Integrated Receiver Decoders v3.2.2 (2025-11) §12.2.9.2–12.2.9.3, Tables 12.12 and 12.14 (tags 0x83, 0x87; the syntax … | `PDS_NORDIG` (private_data_specifier or registration_descriptor) | `DescriptorRegistry::with_nordig_lcn()` | Opt-in |
| 0xD1 | [`Avs3VideoDescriptor`](../src/descriptors/avs3_video.rs) | T/AI 109.6-2022 clause 9.3.2 (Table 9) and GY/T 420-2025 clause 7.3.3.2 (Table 6), tag 0xD1 (209), carried in the PMT ES_info loop of an AVS3 video … | `PDS_AVSV` (private_data_specifier or registration_descriptor) | `DescriptorRegistry::with_avs3_video()` | Opt-in |
| 0xD2 | [`Avs3AudioDescriptor`](../src/descriptors/avs3_audio.rs) | T/AI 109.7-2024 clause 8.1.3.4 (Table 4), tag 0xD2 (210), carried in the PMT ES_info loop of an AVS3-P3 audio elementary stream (stream_type 0xD5 … | `PDS_AVSA` (private_data_specifier or registration_descriptor) | `DescriptorRegistry::with_avs3_audio()` | Opt-in |
| 0xD3 | [`Avs2AudioDescriptor`](../src/descriptors/avs2_audio.rs) | T/AI 109.7-2024 clause 8.2.3.4 (Table 10), tag 0xD3 (211), carried in the PMT ES_info loop of an AVS2 audio elementary stream (stream_type 0xD3, see … | `PDS_AVSA` (private_data_specifier or registration_descriptor) | `DescriptorRegistry::with_avs2_audio()` | Opt-in, partial — `anc_data_block()` kept in `trailing` (defined in a non-vendored standard) |

## AIT descriptors (own tag namespace)

Walked by `parse_ait_loop` / `AnyAitDescriptor`. Allocated by ETSI TS 102 809 V1.3.1
Table 38 (HbbTV) plus the MHP-only tags of ETSI TS 102 727 V1.1.1.

| tag | Descriptor | Standard | Clause (verified) | Source cites | Rust type | Status |
|---|---|---|---|---|---|---|
| 0x00 | application_descriptor | ETSI TS 102 809 V1.3.1 | 5.3.5.3 | ETSI TS 102 809 §5.3.5.3, Table 20 (AIT tag 0x00) | [`ApplicationDescriptor`](../src/descriptors/ait/application.rs) | Full |
| 0x01 | application_name_descriptor | ETSI TS 102 809 V1.3.1 | 5.3.5.6.1 | ETSI TS 102 809 V1.3.1 §5.3.5.6.1, Table 24 (AIT tag 0x01) | [`ApplicationNameDescriptor`](../src/descriptors/ait/application_name.rs) | Full |
| 0x02 | transport_protocol_descriptor | ETSI TS 102 809 V1.3.1 | 5.3.6 | ETSI TS 102 809 §5.3.6.1, Table 28 (AIT tag 0x02) | [`TransportProtocolDescriptor`](../src/descriptors/ait/transport_protocol.rs) | Full |
| 0x03 | DVB-J_application_descriptor | ETSI TS 102 727 V1.1.1 | 10.9.1 | ETSI TS 102 727 §10.9.1, Table 84 (AIT tag 0x03) | [`DvbJApplicationDescriptor`](../src/descriptors/ait/dvb_j_application.rs) | Full |
| 0x04 | DVB-J_application_location_descriptor | ETSI TS 102 727 V1.1.1 | 10.9.2 | ETSI TS 102 727 §10.9.2, Table 85 (AIT tag 0x04) | [`DvbJApplicationLocationDescriptor`](../src/descriptors/ait/dvb_j_application_location.rs) | Full |
| 0x05 | external_application_authorisation_descriptor | ETSI TS 102 809 V1.3.1 | 5.3.5.7 | ETSI TS 102 809 §5.3.5.7, Table 26 (AIT tag 0x05) | [`ExternalApplicationAuthorisationDescriptor`](../src/descriptors/ait/external_application_authorisation.rs) | Full |
| 0x06 | application_recording_descriptor | ETSI TS 102 809 V1.3.1 | 5.3.5.4 | ETSI TS 102 809 V1.3.1 §5.3.5.4, Table 21 (AIT tag 0x06; storage_properties values Table 22) | [`ApplicationRecordingDescriptor`](../src/descriptors/ait/application_recording.rs) | Full |
| 0x08 | DVB-HTML_application_descriptor | ETSI TS 102 727 V1.1.1 | 10.10.1 | — | — | Not implemented — surfaces as `AnyAitDescriptor::Unknown` — bytes kept |
| 0x09 | DVB-HTML_application_location_descriptor | ETSI TS 102 727 V1.1.1 | 10.10.2 | ETSI TS 102 727 V1.1.1 §10.10.2, Table 87 (AIT tag 0x09; physical-root semantics Table 88) | [`DvbHtmlApplicationLocationDescriptor`](../src/descriptors/ait/dvb_html_application_location.rs) | Full |
| 0x0A | DVB-HTML_application_boundary_descriptor | ETSI TS 102 727 V1.1.1 | 10.10.3 | ETSI TS 102 727 V1.1.1 §10.10.3, Table 89 (AIT tag 0x0A) | [`DvbHtmlApplicationBoundaryDescriptor`](../src/descriptors/ait/dvb_html_application_boundary.rs) | Full |
| 0x0B | application_icons_descriptor | ETSI TS 102 809 V1.3.1 | 5.3.5.6.2 | ETSI TS 102 809 V1.3.1 §5.3.5.6.2, Table 25 (AIT tag 0x0B); the icon_flags values are §5.2.8.2 Table 7 (PDF p.43 / p.23, transcribed in … | [`ApplicationIconsDescriptor`](../src/descriptors/ait/application_icons.rs) | Full |
| 0x0C | pre-fetch_descriptor | ETSI TS 102 727 V1.1.1 | 10.8.3.2 | ETSI TS 102 727 V1.1.1 §10.8.3.2, Table 82 (AIT tag 0x0C) | [`PrefetchDescriptor`](../src/descriptors/ait/prefetch.rs) | Full |
| 0x0D | DII_location_descriptor | ETSI TS 102 727 V1.1.1 | 10.8.3.3 | ETSI TS 102 727 V1.1.1 §10.8.3.3, Table 83 (AIT tag 0x0D) | [`DiiLocationDescriptor`](../src/descriptors/ait/dii_location.rs) | Full |
| 0x10 | application_storage_descriptor | ETSI TS 102 809 V1.3.1 | 5.3.10.1 | ETSI TS 102 809 V1.3.1 §5.3.10.1, Table 37 (AIT tag 0x10; same layout as §5.2.11.2 Table 14, storage_property values Table 15) | [`ApplicationStorageDescriptor`](../src/descriptors/ait/application_storage.rs) | Full |
| 0x11 | IP_signalling_descriptor | ETSI TS 102 727 V1.1.1 | 10.8.2 | ETSI TS 102 727 V1.1.1 §10.8.2, Table 81 (AIT tag 0x11) | [`IpSignallingDescriptor`](../src/descriptors/ait/ip_signalling.rs) | Full |
| 0x12 | provider_export_descriptor | ETSI TS 102 727 V1.1.1 | 10.15 | ETSI TS 102 727 V1.1.1 §10.15, Table 93 (AIT tag 0x12) | [`ProviderExportDescriptor`](../src/descriptors/ait/provider_export.rs) | Full |
| 0x13 | provider_usage_descriptor | ETSI TS 102 727 V1.1.1 | 10.15 | ETSI TS 102 727 V1.1.1 §10.15, Table 94 (AIT tag 0x13) | [`ProviderUsageDescriptor`](../src/descriptors/ait/provider_usage.rs) | Full |
| 0x14 | graphics_constraints_descriptor | ETSI TS 102 809 V1.3.1 | 5.3.5.8 | ETSI TS 102 809 V1.3.1 §5.3.5.8, Table 27 (AIT tag 0x14; same layout as §5.2.9.2 Table 8, byte values Table 9) | [`GraphicsConstraintsDescriptor`](../src/descriptors/ait/graphics_constraints.rs) | Full |
| 0x15 | simple_application_location_descriptor | ETSI TS 102 809 V1.3.1 | 5.3.7 | ETSI TS 102 809 §5.3.7.0, Table 33 (AIT tag 0x15) | [`SimpleApplicationLocationDescriptor`](../src/descriptors/ait/simple_application_location.rs) | Full |
| 0x16 | application_usage_descriptor | ETSI TS 102 809 V1.3.1 | 5.3.5.5 | ETSI TS 102 809 §5.3.5.5, Table 23 (AIT tag 0x16) | [`ApplicationUsageDescriptor`](../src/descriptors/ait/application_usage.rs) | Full |
| 0x17 | simple_application_boundary_descriptor | ETSI TS 102 809 V1.3.1 | 5.3.8 | ETSI TS 102 809 §5.3.8, Table 35 (AIT tag 0x17) | [`SimpleApplicationBoundaryDescriptor`](../src/descriptors/ait/simple_application_boundary.rs) | Full |
| 0x5F | private_data_specifier_descriptor | ETSI EN 300 468 V1.20.1 | 6.2.31 | ETSI EN 300 468 §6.2.31 (tag 0x5F) | [`PrivateDataSpecifierDescriptor`](../src/descriptors/private_data_specifier.rs) | Full |

## INT, UNT, data-carousel and RNT scoped descriptors

These standards define their own descriptor tag spaces (EN 301 192 and TS 102 006 forbid
MPEG descriptors 0x00–0x3F in the INT/UNT; TS 102 323 Table 4 overrides 0x40–0x42 inside the
RNT). Each table's loops are walked by its own namespace (`parse_int_loop` / `AnyIntDescriptor`,
`parse_unt_loop` / `AnyUntDescriptor`, `parse_rnt_loop` / `AnyRntDescriptor`), which falls back
to the SI dispatch (`Si`) for the tags the spec hands back to EN 300 468. All 11 DSM-CC
data-carousel descriptors (EN 301 192 Table 48) are typed in `descriptors::carousel`.

### INT (ETSI EN 301 192 V1.7.1 Table 19)

| tag | Descriptor | Standard | Clause (verified) | Source cites | Rust type | Status |
|---|---|---|---|---|---|---|
| 0x06 | target_smartcard_descriptor | ETSI EN 301 192 V1.7.1 | 8.4.5.5 | ETSI EN 301 192 V1.7.1 §8.4.5.5 Table 23 (INT) / ETSI TS 102 006 §9.5.2.1 Table 20 (UNT) (tag 0x06) | [`TargetSmartcardDescriptor`](../src/descriptors/target/smartcard.rs) | Full |
| 0x07 | target_MAC_address_descriptor | ETSI EN 301 192 V1.7.1 | 8.4.5.6 | ETSI EN 301 192 V1.7.1 §8.4.5.6 Table 24 (INT) / ETSI TS 102 006 §9.5.2.2 Table 21 (UNT) (tag 0x07) | [`TargetMacAddressDescriptor`](../src/descriptors/target/mac_address.rs) | Full |
| 0x08 | target_serial_number_descriptor | ETSI EN 301 192 V1.7.1 | 8.4.5.4 | ETSI EN 301 192 V1.7.1 §8.4.5.4 Table 22 (INT) / ETSI TS 102 006 §9.5.2.5 Table 24 (UNT) (tag 0x08) | [`TargetSerialNumberDescriptor`](../src/descriptors/target/serial_number.rs) | Full |
| 0x09 | target_IP_address_descriptor | ETSI EN 301 192 V1.7.1 | 8.4.5.8 | ETSI EN 301 192 V1.7.1 §8.4.5.8 Table 26 (INT) / ETSI TS 102 006 §9.5.2.3 Table 22 (UNT) (tag 0x09) | [`TargetIpAddressDescriptor`](../src/descriptors/target/ip_address.rs) | Full |
| 0x0A | target_IPv6_address_descriptor | ETSI EN 301 192 V1.7.1 | 8.4.5.11 | ETSI EN 301 192 V1.7.1 §8.4.5.11 Table 29 (INT) / ETSI TS 102 006 §9.5.2.4 Table 23 (UNT) (tag 0x0A) | [`TargetIpv6AddressDescriptor`](../src/descriptors/target/ipv6_address.rs) | Full |
| 0x0C | IP/MAC_platform_name_descriptor | ETSI EN 301 192 V1.7.1 | 8.4.5.2 | ETSI EN 301 192 V1.7.1 §8.4.5.2 Table 20 (tag 0x0C) | [`IpMacPlatformNameDescriptor`](../src/descriptors/int/platform_name.rs) | Full |
| 0x0D | IP/MAC_platform_provider_name_descriptor | ETSI EN 301 192 V1.7.1 | 8.4.5.3 | ETSI EN 301 192 V1.7.1 §8.4.5.3 Table 21 (tag 0x0D) | [`IpMacPlatformProviderNameDescriptor`](../src/descriptors/int/platform_provider_name.rs) | Full |
| 0x0E | target_MAC_address_range_descriptor | ETSI EN 301 192 V1.7.1 | 8.4.5.7 | ETSI EN 301 192 V1.7.1 §8.4.5.7 Table 25 (tag 0x0E) | [`TargetMacAddressRangeDescriptor`](../src/descriptors/int/target_mac_address_range.rs) | Full |
| 0x0F | target_IP_slash_descriptor | ETSI EN 301 192 V1.7.1 | 8.4.5.9 | ETSI EN 301 192 V1.7.1 §8.4.5.9 Table 27 (tag 0x0F) | [`TargetIpSlashDescriptor`](../src/descriptors/int/target_ip_slash.rs) | Full |
| 0x10 | target_IP_source_slash_descriptor | ETSI EN 301 192 V1.7.1 | 8.4.5.10 | ETSI EN 301 192 V1.7.1 §8.4.5.10 Table 28 (tag 0x10) | [`TargetIpSourceSlashDescriptor`](../src/descriptors/int/target_ip_source_slash.rs) | Full |
| 0x11 | target_IPv6_slash_descriptor | ETSI EN 301 192 V1.7.1 | 8.4.5.12 | ETSI EN 301 192 V1.7.1 §8.4.5.12 Table 30 (tag 0x11) | [`TargetIpv6SlashDescriptor`](../src/descriptors/int/target_ipv6_slash.rs) | Full |
| 0x12 | target_IPv6_source_slash_descriptor | ETSI EN 301 192 V1.7.1 | 8.4.5.13 | ETSI EN 301 192 V1.7.1 §8.4.5.13 Table 31 (tag 0x12) | [`TargetIpv6SourceSlashDescriptor`](../src/descriptors/int/target_ipv6_source_slash.rs) | Full |
| 0x13 | IP/MAC_stream_location_descriptor | ETSI EN 301 192 V1.7.1 | 8.4.5.14 | ETSI EN 301 192 V1.7.1 §8.4.5.14 Table 32 (tag 0x13) | [`IpMacStreamLocationDescriptor`](../src/descriptors/int/stream_location.rs) | Full |
| 0x14 | ISP_access_mode_descriptor | ETSI EN 301 192 V1.7.1 | 8.4.5.16 | ETSI EN 301 192 V1.7.1 §8.4.5.16 Table 36 (tag 0x14), access_mode per Table 37 | [`IspAccessModeDescriptor`](../src/descriptors/int/isp_access_mode.rs) | Full |
| 0x15 | IP/MAC_generic_stream_location_descriptor | ETSI EN 301 192 V1.7.1 | 8.4.5.15 | ETSI EN 301 192 V1.7.1 §8.4.5.15 Table 33 (tag 0x15), modulation_system_type per Table 34, PHY_stream_id per Table 35 | [`IpMacGenericStreamLocationDescriptor`](../src/descriptors/int/generic_stream_location.rs) | Full |

### UNT / SSU (ETSI TS 102 006 V1.4.1 Table 19)

| tag | Descriptor | Standard | Clause (verified) | Source cites | Rust type | Status |
|---|---|---|---|---|---|---|
| 0x01 | scheduling_descriptor | ETSI TS 102 006 V1.4.1 | 9.5.2.9 | ETSI TS 102 006 §9.5.2.9 Table 31 (UNT tag 0x01), time units per Table 32 | [`SchedulingDescriptor`](../src/descriptors/unt/scheduling.rs) | Full |
| 0x02 | update_descriptor | ETSI TS 102 006 V1.4.1 | 9.5.2.6 | ETSI TS 102 006 §9.5.2.6 Table 25 (UNT tag 0x02), update_flag per Table 26, update_method per Table 27 | [`UpdateDescriptor`](../src/descriptors/unt/update.rs) | Full |
| 0x03 | ssu_location_descriptor | ETSI TS 102 006 V1.4.1 | 9.5.2.7 | ETSI TS 102 006 §9.5.2.7 Table 29 (UNT tag 0x03) | [`SsuLocationDescriptor`](../src/descriptors/unt/ssu_location.rs) | Full |
| 0x04 | message_descriptor | ETSI TS 102 006 V1.4.1 | 9.5.2.12 | ETSI TS 102 006 §9.5.2.12 Table 35 (UNT tag 0x04) | [`MessageDescriptor`](../src/descriptors/unt/message.rs) | Full |
| 0x05 | ssu_event_name_descriptor | ETSI TS 102 006 V1.4.1 | 9.5.2.11 | ETSI TS 102 006 §9.5.2.11 Table 34 (UNT tag 0x05) | [`SsuEventNameDescriptor`](../src/descriptors/unt/ssu_event_name.rs) | Full |
| 0x06 | target_smartcard_descriptor | ETSI TS 102 006 V1.4.1 | 9.5.2.1 | ETSI EN 301 192 V1.7.1 §8.4.5.5 Table 23 (INT) / ETSI TS 102 006 §9.5.2.1 Table 20 (UNT) (tag 0x06) | [`TargetSmartcardDescriptor`](../src/descriptors/target/smartcard.rs) | Full |
| 0x07 | target_MAC_address_descriptor | ETSI TS 102 006 V1.4.1 | 9.5.2.2 | ETSI EN 301 192 V1.7.1 §8.4.5.6 Table 24 (INT) / ETSI TS 102 006 §9.5.2.2 Table 21 (UNT) (tag 0x07) | [`TargetMacAddressDescriptor`](../src/descriptors/target/mac_address.rs) | Full |
| 0x08 | target_serial_number_descriptor | ETSI TS 102 006 V1.4.1 | 9.5.2.5 | ETSI EN 301 192 V1.7.1 §8.4.5.4 Table 22 (INT) / ETSI TS 102 006 §9.5.2.5 Table 24 (UNT) (tag 0x08) | [`TargetSerialNumberDescriptor`](../src/descriptors/target/serial_number.rs) | Full |
| 0x09 | target_IP_address_descriptor | ETSI TS 102 006 V1.4.1 | 9.5.2.3 | ETSI EN 301 192 V1.7.1 §8.4.5.8 Table 26 (INT) / ETSI TS 102 006 §9.5.2.3 Table 22 (UNT) (tag 0x09) | [`TargetIpAddressDescriptor`](../src/descriptors/target/ip_address.rs) | Full |
| 0x0A | target_IPv6_address_descriptor | ETSI TS 102 006 V1.4.1 | 9.5.2.4 | ETSI EN 301 192 V1.7.1 §8.4.5.11 Table 29 (INT) / ETSI TS 102 006 §9.5.2.4 Table 23 (UNT) (tag 0x0A) | [`TargetIpv6AddressDescriptor`](../src/descriptors/target/ipv6_address.rs) | Full |
| 0x0B | ssu_subgroup_association_descriptor | ETSI TS 102 006 V1.4.1 | 9.5.2.8 | ETSI TS 102 006 §9.5.2.8 Table 30 (UNT tag 0x0B) | [`SsuSubgroupAssociationDescriptor`](../src/descriptors/unt/subgroup_association.rs) | Full |
| 0x0C | enhanced_message_descriptor | ETSI TS 102 006 V1.4.1 | 9.5.2.14 | ETSI TS 102 006 §9.5.2.14 Table 37 (UNT tag 0x0C) | [`EnhancedMessageDescriptor`](../src/descriptors/unt/enhanced_message.rs) | Full |
| 0x0D | ssu_uri_descriptor | ETSI TS 102 006 V1.4.1 | 9.5.2.15 | ETSI TS 102 006 §9.5.2.15 Table 38 (UNT tag 0x0D) | [`SsuUriDescriptor`](../src/descriptors/unt/ssu_uri.rs) | Full |

### DSM-CC data carousel (ETSI EN 301 192 V1.7.1 Table 48)

| tag | Descriptor | Standard | Clause (verified) | Source cites | Rust type | Status |
|---|---|---|---|---|---|---|
| 0x01 | type_descriptor | ETSI EN 301 192 V1.7.1 | 10.2.2 | ETSI EN 301 192 V1.7.1 §10.2.2, Table 50 (data-carousel tag 0x01) | [`TypeDescriptor`](../src/descriptors/carousel/type_descriptor.rs) | Full |
| 0x02 | name_descriptor | ETSI EN 301 192 V1.7.1 | 10.2.3 | ETSI EN 301 192 V1.7.1 §10.2.3, Table 51 (data-carousel tag 0x02) | [`NameDescriptor`](../src/descriptors/carousel/name.rs) | Full |
| 0x03 | info_descriptor | ETSI EN 301 192 V1.7.1 | 10.2.4 | ETSI EN 301 192 V1.7.1 §10.2.4, Table 52 (data-carousel tag 0x03) | [`InfoDescriptor`](../src/descriptors/carousel/info.rs) | Full |
| 0x04 | module_link_descriptor | ETSI EN 301 192 V1.7.1 | 10.2.5 | ETSI EN 301 192 V1.7.1 §10.2.5, Table 53 (data-carousel tag 0x04) | [`ModuleLinkDescriptor`](../src/descriptors/carousel/module_link.rs) | Full |
| 0x05 | CRC32_descriptor | ETSI EN 301 192 V1.7.1 | 10.2.6 | ETSI EN 301 192 V1.7.1 §10.2.6, Table 54 (data-carousel tag 0x05) | [`Crc32Descriptor`](../src/descriptors/carousel/crc32.rs) | Full |
| 0x06 | location_descriptor | ETSI EN 301 192 V1.7.1 | 10.2.7 | ETSI EN 301 192 V1.7.1 §10.2.7, Table 55 (data-carousel tag 0x06) | [`LocationDescriptor`](../src/descriptors/carousel/location.rs) | Full |
| 0x07 | est_download_time_descriptor | ETSI EN 301 192 V1.7.1 | 10.2.8 | ETSI EN 301 192 V1.7.1 §10.2.8, Table 56 (data-carousel tag 0x07) | [`EstDownloadTimeDescriptor`](../src/descriptors/carousel/est_download_time.rs) | Full |
| 0x08 | group_link_descriptor | ETSI EN 301 192 V1.7.1 | 10.2.9 | ETSI EN 301 192 V1.7.1 §10.2.9, Table 57 (data-carousel tag 0x08) | [`GroupLinkDescriptor`](../src/descriptors/carousel/group_link.rs) | Full |
| 0x09 | compressed_module_descriptor | ETSI EN 301 192 V1.7.1 | 10.2.11 | ETSI EN 301 192 V1.7.1 §10.2.11, Table 59 (data-carousel tag 0x09) | [`CompressedModuleDescriptor`](../src/descriptors/carousel/compressed_module.rs) | Full |
| 0x0A | SSU_module_type_descriptor | ETSI TS 102 006 V1.4.1 | 8.2.1 | ETSI TS 102 006 §8.2.1, Table 9 (data-carousel tag 0x0A, allocated by EN 301 192 Table 48; module types Table 10) | [`SsuModuleTypeDescriptor`](../src/descriptors/carousel/ssu_module_type.rs) | Full |
| 0x0B | subgroup_association_descriptor | ETSI TS 102 006 V1.4.1 | 9.6.2.1 | ETSI TS 102 006 §9.6.2.1, Table 39 (data-carousel tag 0x0B, allocated by EN 301 192 Table 48; DSI GroupInfoBytes) | [`SubgroupAssociationDescriptor`](../src/descriptors/carousel/subgroup_association.rs) | Full |

### RNT scope (ETSI TS 102 323 V1.4.1 Table 4)

| tag | Descriptor | Standard | Clause (verified) | Source cites | Rust type | Status |
|---|---|---|---|---|---|---|
| 0x40 | RAR_over_DVB_stream_descriptor | ETSI TS 102 323 V1.4.1 | 5.3.5 | ETSI TS 102 323 V1.4.1 §5.3.5 Table 12 (RNT tag 0x40) | [`RarOverDvbStreamDescriptor`](../src/descriptors/rnt/rar_over_dvb_stream.rs) | Full |
| 0x41 | RAR_over_IP_descriptor | ETSI TS 102 323 V1.4.1 | 5.3.6 | ETSI TS 102 323 V1.4.1 §5.3.6 Table 13 (RNT tag 0x41) | [`RarOverIpDescriptor`](../src/descriptors/rnt/rar_over_ip.rs) | Full |
| 0x42 | RNT_scan_descriptor | ETSI TS 102 323 V1.4.1 | 5.3.7 | ETSI TS 102 323 V1.4.1 §5.3.7 Table 14 (RNT tag 0x42) | [`RntScanDescriptor`](../src/descriptors/rnt/scan.rs) | Full |

## DSM-CC data and object carousel

| Type | What it is | Spec |
|---|---|---|
| [`DsmccSection`](../src/tables/dsmcc.rs) | DSM-CC section framing, table_id 0x3A–0x3F (CRC or checksum) | ISO/IEC 13818-6 §9 / ETSI EN 301 192 |
| [`Dsi`](../src/carousel/messages.rs) | DownloadServerInitiate (messageId 0x1006); privateData = GroupInfoIndication or ServiceGatewayInfo | ISO/IEC 13818-6 §7.3 / TR 101 202 |
| [`GroupInfoIndication`](../src/carousel/messages.rs) | SSU / data-carousel group list carried in the DSI | ETSI TS 102 006 / EN 301 192 |
| [`Dii`](../src/carousel/messages.rs) | DownloadInfoIndication (messageId 0x1002) with a typed module loop | ISO/IEC 13818-6 §7.3 |
| [`DownloadDataBlock`](../src/carousel/messages.rs) | DownloadDataBlock (messageId 0x1003) | ISO/IEC 13818-6 §7.3 |
| [`ModuleReassembler`](../src/carousel/reassembler.rs) | DDB → complete modules per DII geometry: version-aware, out-of-order tolerant, per-module + aggregate memory caps | — |
| [`CompatibilityDescriptor`](../src/compatibility.rs) | compatibilityDescriptor() of DSI/DII groups and UNT platforms | ETSI TS 102 006 Table 15 / ISO/IEC 13818-6 |
| [`ServiceGatewayInfo`](../src/carousel/biop/message.rs) | object-carousel DSI privateData | TR 101 202 Table 4.15 |
| [`ModuleInfo`](../src/carousel/biop/message.rs) | BIOP ModuleInfo (DII moduleInfoBytes), incl. compressed_module | TR 101 202 Table 4.14 |
| [`BiopMessage`](../src/carousel/biop/message.rs) | BIOP Directory / File / ServiceGateway / Stream / StreamEvent messages | ISO/IEC 13818-6 §11 / TR 101 202 §4.7 |
| [`Ior`](../src/carousel/biop/ior.rs) | IOP::IOR with tagged BIOP / Lite-Options profile bodies, object locations, taps | ISO/IEC 13818-6 §11 / TR 101 202 |
| [`CarouselFs`](../src/carousel/biop/fs.rs) | virtual file tree over a reassembled object carousel (`resolve` / `file_bytes`) | — |

## Text (EN 300 468 Annex A)

`DvbText` keeps the wire bytes and decodes on demand (`decode()`, `Display`, serde);
`LangCode` is the ISO 639-2 / ISO 3166 three-byte code. Annex A.1 control codes are honoured
for both the single-byte (0x80–0x9F) and two-byte (Table A.2) forms: emphasis dropped,
CR/LF → space, reserved controls stripped.

| Selector (Table A.3) | Character table | Decoder | Needs `std` |
|---|---|---|---|
| none (first byte ≥ 0x20) or 0x00 | Figure A.1 default Latin (ISO/IEC 6937 superset, € at 0xA4) | built-in, glyph-for-glyph; combining-diacritic row → precomposed or base + combining mark | no |
| 0x01–0x07, 0x09–0x0B; 0x10 0x00 nn | ISO/IEC 8859-5 … -15 (no -12); two-byte form any -n | `encoding_rs` (8859-1 built-in) | yes, except 8859-1 |
| 0x15 | UTF-8 encoding of ISO/IEC 10646 | passthrough (lossy on invalid bytes) | no |
| 0x11 | ISO/IEC 10646 Basic Multilingual Plane | built-in UCS-2 big-endian | no |
| 0x12 | KS X 1001 (Korean) | `encoding_rs` EUC-KR | yes |
| 0x13 | GB-2312-1980 (Simplified Chinese) | `encoding_rs` GBK (a GB-2312 superset) | yes |
| 0x14 | Big5 (Traditional Chinese) | `encoding_rs` Big5 | yes |
| 0x1F + encoding_type_id; reserved 0x08, 0x0C–0x0F, 0x16–0x1E | no registered broadcast encodings / reserved | U+FFFD (never panics) | no |

## Time and dates

- **MJD + BCD UTC** (EN 300 468 Annex C) for EIT `start_time`, TDT/TOT `UTC_time`,
  local_time_offset `time_of_change`, network_change_notify `start_time_of_change`:
  always decoded to `broadcast_common::time::MjdBcdDateTime` (e.g. `EitEvent::start_time()`,
  `TdtSection::utc_time_decoded()`), additionally to `chrono::DateTime<Utc>` with the
  `chrono` feature (`start_time_chrono()`, `utc_time()`); `*_raw()` returns the wire bytes.
- **2038 MJD wrap rule** (EN 300 468 V1.20.1 clauses 5.2.4–5.2.6, 6.2.20, 6.4.9): an MJD
  whose most significant bit is 0 is read as `0x10000 + MJD`.
- **BCD durations**: EIT `duration()` → `core::time::Duration`; local_time_offset
  `local_time_offset()` / `next_time_offset()` → `chrono::Duration` (feature `chrono`).

## Higher layers

| Item | What it does | Feature |
|---|---|---|
| [`SiDemux`](../src/demux.rs) | PID-filtered, CRC-checked, version-gated section pump over 188-byte TS packets; follows the PAT to PMT PIDs | `ts` |
| [`SectionSetCollector`](../src/collect/mod.rs) | assemble every long-form table across `section_number 0..=last_section_number` | always |
| [`EitCollector`](../src/collect/eit.rs) | EIT schedule collection across `last_table_id` (the rule section numbers alone miss) | always |
| [`CompleteNit`](../src/collect/nit.rs) | flattened logical NIT (also `CompleteBat`, `CompleteSdt`, `CompleteEit`) | always |
| [`EpgStore`](../src/epg.rs) | bounded EPG: deduplicated events, decoded extended text, now/next, SDT name join | `chrono` |
| [`DescriptorRegistry`](../src/descriptors/registry.rs) | runtime registration of private descriptor tags, PDS-scoped | always |
| [`ExtensionRegistry`](../src/descriptors/extension/registry.rs) | runtime registration of private descriptor_tag_extension bodies | always |
| [`TableRegistry`](../src/tables/registry.rs) | runtime registration of private table_ids | always |
| [`Owned`](../src/owned.rs) | keep a parsed zero-copy view past its buffer's borrow (`Send + Sync`, `'static`) | `yoke` |

## Not implemented

Every code the vendored standards allocate that `dvb-si` does not type yet (generated from
the rows above). Contributions follow `docs/CRATE-ACCEPTANCE.md`: spec transcription and a
real fixture first.

| Namespace | Count | Codes |
|---|---|---|
| table_id | 4 | 0x07 IPMP_Control_Information_section (ISO/IEC 13818-11), 0x0A ISO/IEC 23001-10 quality access unit section, 0x0B ISO/IEC 23001-13 media orchestration access unit section, 0x38–0x39 DSM-CC sections (ISO/IEC 13818-6 range) |
| Descriptor tags (SI / MPEG namespace) | 2 | 0x16–0x1A other ISO/IEC 13818-6 descriptors (Table 2-45 "19..26 defined in ISO/IEC 13818-6"; 13818-6 is not vendored and its layouts for tags 0x16-0x1A appear in no vendored spec: 0x17-0x1A are the stream-event / NPT descriptors, and TS 102 809 B.2.4.2 says NPT is obsolete and gives no layout), 0x29 IPMP_descriptor (ISO/IEC 13818-11) |
| MPEG-2 extension_descriptor_tag | 2 | 0x10 virtual_segmentation_descriptor, 0x13 HEVC_subregion_descriptor |
| AIT descriptors | 1 | 0x08 DVB-HTML_application_descriptor |

## Citation cross-check

Every module's `//!` citation is compared with the clause verified against the edition in
the Standard column. The generator (and so the `docs_coverage_drift` test) **fails** if a
module cites a clause that is neither the verified clause nor a sub-clause of it (issue
#1287); reasoned exceptions live in `CITATION_ALLOWLIST` in
`tools/gen-dvbsi-coverage.py`.

Every module citation that names a clause agrees with the verified clause.

255 implemented rows have their clause verified against the rendered PDF page
(`verified_page` column of `coverage/allocations.tsv`); 5 not-implemented rows
carry a clause that is not yet page-verified (nothing cites it).
