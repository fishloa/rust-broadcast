# dvb-si

ETSI EN 300 468 DVB Service Information parser **and builder**, plus the
MPEG-2 PSI tables it builds on, the DVB-allocated companion tables, and the
DSM-CC data carousel.

**Complete coverage: every allocated `table_id` in EN 300 468 V1.19.1
Table 2 (29 table types; 28 dispatched by `AnyTable` + the type-keyed MPE datagram view) and every allocated `descriptor_tag` in Table 12
(0x40–0x7F, 64 descriptors) is implemented**, each with a symmetric
`Parse` / `Serialize` pair and round-trip tests. Layouts are derived from the
ETSI specs (vendored in the repo and transcribed into reviewable markdown) and
validated against live broadcast captures.

## Table coverage

Every table: typed header fields, symmetric parse/serialize, round-trip
tested. Per the crate's zero-copy convention, descriptor loops and repeated
sub-structures are borrowed `&[u8]` slices the caller walks with the
descriptor parsers — noted below only where a table goes further or stays
deliberately raw.

| table_id | Table | Spec | Status |
|---|---|---|---|
| 0x00 | PAT — Program Association | ISO/IEC 13818-1 | ✅ full |
| 0x01 | CAT — Conditional Access | ISO/IEC 13818-1 | ✅ full + typed `ca_descriptors()` view |
| 0x02 | PMT — Program Map | ISO/IEC 13818-1 | ✅ full (typed ES loop) |
| 0x03 | TSDT — TS Description | ISO/IEC 13818-1 | ✅ full |
| 0x3A–0x3F | DSM-CC sections | ISO/IEC 13818-6 / EN 301 192 | ✅ framing; 0x3B/0x3C payloads typed via [`carousel`](#dsm-cc-data-carousel); 0x3E typed as MPE |
| 0x3E | MPE datagram_section (typed IP/MAC view) | EN 301 192 §7 | ✅ full (MAC reassembly, LLC/SNAP flag, SSI-aware trailer) |
| 0x40/0x41 | NIT actual/other | EN 300 468 §5.2.1 | ✅ full (typed TS loop) |
| 0x42/0x46 | SDT actual/other | EN 300 468 §5.2.3 | ✅ full (typed service loop) |
| 0x4A | BAT — Bouquet Association | EN 300 468 §5.2.2 | ✅ full (typed TS loop) |
| 0x4B | UNT — Update Notification (SSU) | TS 102 006 | ✅ full |
| 0x4C | INT — IP/MAC Notification | EN 301 192 | ✅ full |
| 0x4D | SAT — Satellite Access family | EN 300 468 §5.2.11 | ✅ header + `SatTableId` discriminant typed; variant bodies raw (bit-packed orbital data, layout in docs) |
| 0x4E–0x6F | EIT p/f + schedule, actual/other | EN 300 468 §5.2.4 | ✅ full (typed event loop; `chrono`-gated MJD+BCD `start_time()`) |
| 0x70 | TDT — Time and Date | EN 300 468 §5.2.5 | ✅ full |
| 0x71 | RST — Running Status | EN 300 468 §5.2.7 | ✅ full (typed event loop) |
| 0x72 | ST — Stuffing | EN 300 468 §5.2.8 | ✅ full |
| 0x73 | TOT — Time Offset | EN 300 468 §5.2.6 | ✅ full (incl. the SSI=0-with-CRC framing exception) |
| 0x74 | AIT — Application Information | TS 102 809 | ✅ full (typed application loop), validated vs live HbbTV capture |
| 0x75 | Container | TS 102 323 | ✅ full |
| 0x76 | RCT — Related Content | TS 102 323 | ✅ full |
| 0x77 | CIT — Content Identifier | TS 102 323 | ✅ full |
| 0x78 | MPE-FEC | EN 301 192 §9.9 | ✅ full (typed real_time_parameters) |
| 0x79 | RNT — Resolution Notification | TS 102 323 | ✅ full |
| 0x7A | MPE-IFEC | TS 102 772 | ✅ full (typed real_time_parameters) |
| 0x7B | Protection message | TS 102 809 §9 | ✅ full — authentication-message + certificate-collection variants by table_id_extension |
| 0x7C | DFIS — Downloadable Font Info | EN 303 560 | ✅ full (typed font_info loop; table_id per EN 300 468 Table 2 NOTE 2) |
| 0x7E | DIT — Discontinuity Information | EN 300 468 | ✅ full |
| 0x7F | SIT — Selection Information | EN 300 468 | ✅ full |

Remaining table_id values are *reserved* or *user-defined* in EN 300 468
V1.19.1 Table 2 — there is nothing standardized left to implement.

## DSM-CC data carousel

The `carousel` module types the download-protocol payloads carried inside
DSM-CC sections (ISO/IEC 13818-6 §7.2/§7.3 as profiled by DVB — TR 101 202,
TS 102 006 SSU, TS 102 809 object carousels):

| Message | messageId | Status |
|---|---|---|
| DSI — DownloadServerInitiate | 0x1006 | ✅ full (privateData raw: SSU GroupInfoIndication / OC ServiceGatewayInfo) |
| DII — DownloadInfoIndication | 0x1002 | ✅ full (typed module loop) |
| DDB — DownloadDataBlock | 0x1003 | ✅ full |
| `ModuleReassembler` | — | ✅ DDB → complete modules per DII geometry: version-aware, out-of-order tolerant, per-module + aggregate memory caps |

Validated **byte-exact** against a live French-TNT (M6 HbbTV) capture in the
test suite. BIOP object-carousel payloads above this layer are out of scope.

## Descriptors

**Every allocated `descriptor_tag` in EN 300 468 V1.19.1 Table 12
(0x40–0x7F) is implemented** — plus the MPEG-2 descriptors that matter in SI
context and the de-facto private logical_channel_descriptor. Each parses into
a typed struct with a symmetric serializer and round-trip tests; any
unallocated/unknown tag passes through as raw bytes (tag + payload preserved).
Per the crate's zero-copy convention, free-form byte fields (names, selector
tails) stay borrowed `&[u8]`; notes below only where a sub-structure is
deliberately kept raw.

| tag | Descriptor | Spec | Status |
|---|---|---|---|
| 0x05 | registration | ISO/IEC 13818-1 | ✅ full |
| 0x06 | data_stream_alignment | ISO/IEC 13818-1 | ✅ full |
| 0x09 | CA | ISO/IEC 13818-1 | ✅ full |
| 0x0A | ISO_639_language | ISO/IEC 13818-1 | ✅ full |
| 0x0F | private_data_indicator | ISO/IEC 13818-1 | ✅ full |
| 0x40 | network_name | EN 300 468 | ✅ full |
| 0x41 | service_list | EN 300 468 | ✅ full |
| 0x42 | stuffing | EN 300 468 | ✅ full |
| 0x43 | satellite_delivery_system | EN 300 468 | ✅ full |
| 0x44 | cable_delivery_system | EN 300 468 | ✅ full |
| 0x45 | VBI_data | EN 300 468 | ✅ full (typed service loop; one-byte line entries raw per §6.2.47) |
| 0x46 | VBI_teletext | EN 300 468 | ✅ full |
| 0x47 | bouquet_name | EN 300 468 | ✅ full |
| 0x48 | service | EN 300 468 | ✅ full |
| 0x49 | country_availability | EN 300 468 | ✅ full |
| 0x4A | linkage | EN 300 468 | ✅ full |
| 0x4B | NVOD_reference | EN 300 468 | ✅ full |
| 0x4C | time_shifted_service | EN 300 468 | ✅ full |
| 0x4D | short_event | EN 300 468 | ✅ full |
| 0x4E | extended_event | EN 300 468 | ✅ full |
| 0x4F | time_shifted_event | EN 300 468 | ✅ full |
| 0x50 | component | EN 300 468 | ✅ full |
| 0x51 | mosaic | EN 300 468 | ✅ full (typed cell + elementary-cell loops, typed cell_linkage variants) |
| 0x52 | stream_identifier | EN 300 468 | ✅ full |
| 0x53 | CA_identifier | EN 300 468 | ✅ full |
| 0x54 | content | EN 300 468 | ✅ full |
| 0x55 | parental_rating | EN 300 468 | ✅ full |
| 0x56 | teletext | EN 300 468 | ✅ full |
| 0x57 | telephone | EN 300 468 | ✅ full (bit-packed length fields typed) |
| 0x58 | local_time_offset | EN 300 468 | ✅ full |
| 0x59 | subtitling | EN 300 468 | ✅ full |
| 0x5A | terrestrial_delivery_system | EN 300 468 | ✅ full |
| 0x5B | multilingual_network_name | EN 300 468 | ✅ full |
| 0x5C | multilingual_bouquet_name | EN 300 468 | ✅ full |
| 0x5D | multilingual_service_name | EN 300 468 | ✅ full |
| 0x5E | multilingual_component | EN 300 468 | ✅ full |
| 0x5F | private_data_specifier | EN 300 468 | ✅ full |
| 0x60 | service_move | EN 300 468 | ✅ full |
| 0x61 | short_smoothing_buffer | EN 300 468 | ✅ full |
| 0x62 | frequency_list | EN 300 468 | ✅ full |
| 0x63 | partial_transport_stream | EN 300 468 §7.2.1 | ✅ full |
| 0x64 | data_broadcast | EN 300 468 | ✅ full (selector raw — interpretation depends on data_broadcast_id) |
| 0x65 | scrambling | EN 300 468 | ✅ full |
| 0x66 | data_broadcast_id | EN 300 468 / EN 301 192 | ✅ full (id_selector tail raw) |
| 0x67 | transport_stream | EN 300 468 | ✅ full |
| 0x68 | DSNG | EN 300 468 | ✅ full |
| 0x69 | PDC | EN 300 468 | ✅ full |
| 0x6A | AC-3 | EN 300 468 Annex D | ✅ full |
| 0x6B | ancillary_data | EN 300 468 | ✅ full |
| 0x6C | cell_list | EN 300 468 | ✅ full (both loops typed, 12+12-bit extents unpacked) |
| 0x6D | cell_frequency_link | EN 300 468 | ✅ full (both loops typed) |
| 0x6E | announcement_support | EN 300 468 | ✅ full |
| 0x6F | application_signalling | TS 102 809 | ✅ full |
| 0x70 | adaptation_field_data | EN 300 468 | ✅ full |
| 0x71 | service_identifier | TS 102 809 | ✅ full |
| 0x72 | service_availability | EN 300 468 | ✅ full |
| 0x73 | default_authority | TS 102 323 | ✅ full |
| 0x74 | related_content | TS 102 323 | ✅ full |
| 0x75 | TVA_id | TS 102 323 | ✅ full |
| 0x76 | content_identifier | TS 102 323 | ✅ full |
| 0x77 | time_slice_fec_identifier | EN 301 192 | ✅ full |
| 0x78 | ECM_repetition_rate | EN 301 192 | ✅ full |
| 0x79 | S2_satellite_delivery_system | EN 300 468 | ✅ full |
| 0x7A | enhanced_AC-3 | EN 300 468 Annex D | ✅ full |
| 0x7B | DTS | EN 300 468 Annex G | ✅ full |
| 0x7C | AAC | EN 300 468 Annex H | ✅ full |
| 0x7D | XAIT_location | TS 102 727 | ✅ full |
| 0x7E | FTA_content_management | EN 300 468 | ✅ full |
| 0x7F | extension | EN 300 468 §6.2.18.1 | ✅ typed discriminant + typed bodies below; unknown tag_extensions round-trip raw |
| 0x83 | logical_channel | EACEM/NorDig private | ✅ full |

### Extension descriptor registry (tag 0x7F)

The first payload byte (`descriptor_tag_extension`) selects a sub-descriptor
(EN 300 468 §6.4). A body is typed only when its syntax table is vendored
under `docs/`; everything else is preserved byte-exact as `Raw` and
round-trips losslessly.

| tag_ext | Extension | Status |
|---|---|---|
| 0x04 | T2_delivery_system | ✅ typed (first level; cell loop raw) |
| 0x06 | supplementary_audio | ✅ typed |
| 0x07 | network_change_notify | ✅ typed (cell loop raw) |
| 0x08 | message | ✅ typed |
| 0x09 | target_region | ✅ typed (region loop raw) |
| 0x0A | target_region_name | ✅ typed (region loop raw) |
| 0x0B | service_relocated | ✅ typed |
| 0x0D | C2_delivery_system | ✅ typed |
| 0x13 | URI_linkage | ✅ typed (uri/private split) |
| 0x15 | AC-4 | ✅ typed (first level; toc/extra raw) |
| 0x16 | C2_bundle_delivery_system | ✅ typed (full fixed loop) |
| 0x17 | S2X_satellite_delivery_system | ✅ typed (primary channel; bonding tail raw) |
| 0x19 | audio_preselection | ✅ typed (preselection loop raw) |
| 0x20 | TTML_subtitling | ✅ typed (EN 303 560) |
| 0x00 image_icon · 0x05 SH_delivery_system · 0x10 video_depth_range · 0x11 T2MI · 0x22–0x24 | niche; deferred | raw-preserved |
| 0x01–0x03 CPCM (TS 102 825) · 0x0C XAIT_PID (TS 102 727) · 0x0E/0x0F/0x21 DTS family · 0x14 CI_ancillary (TS 103 205) · 0x18 protection_message (TS 102 809) | spec not vendored | raw-preserved |

## Text decoding

**Full EN 300 468 Annex A Table A.3 selector coverage:**

| Selector | Table | Decoding |
|---|---|---|
| (none, first byte ≥ 0x20) | default Latin, Figure A.1 | **glyph-for-glyph** (ISO 6937 superset — € at 0xA4, full non-spacing diacritic row with precomposed forms + combining-mark fallback, every position pinned by tests) |
| 0x01–0x0B | ISO/IEC 8859-5 … -15 | via `encoding_rs` (0x08 is reserved — no ISO 8859-12) |
| 0x10 | ISO/IEC 8859-n (two-byte selector) | via `encoding_rs` |
| 0x11 | ISO/IEC 10646 BMP | UCS-2 BE |
| 0x12 | KS X 1001 (Korean) | EUC-KR |
| 0x13 | GB-2312-1980 (Simplified Chinese) | GBK (GB-2312 superset) |
| 0x14 | Big5 (Traditional Chinese) | Big5 |
| 0x15 | UTF-8 | passthrough |
| 0x1F | `encoding_type_id` escape | id byte consumed; body U+FFFD (no registered broadcast ids) |
| reserved (0x08, 0x0C–0x0F, 0x16–0x1E) | — | U+FFFD per byte |

Annex A.1 control codes are honored for both the single-byte (0x80–0x9F) and
two-byte (U+E080–U+E09F PUA, Table A.2) tables: emphasis markers dropped,
CR/LF → space, reserved controls stripped.

## Spec grounding

Every layout is cited. The repo vendors the ETSI PDFs and transcribes their
syntax tables into reviewable markdown
([`docs/`](https://github.com/fishloa/rust-dvb/tree/main/dvb-si/docs)) —
each spec below links both the ETSI deliverable and the in-repo
transcription:

| Spec | ETSI deliverable | Transcription |
|---|---|---|
| EN 300 468 V1.19.1 (2025-02) — DVB SI | [PDF](https://www.etsi.org/deliver/etsi_en/300400_300499/300468/01.19.01_60/en_300468v011901p.pdf) | [en_300_468.md](https://github.com/fishloa/rust-dvb/blob/main/dvb-si/docs/en_300_468.md) |
| EN 301 192 v1.7.1 — data broadcasting | [PDF](https://www.etsi.org/deliver/etsi_en/301100_301199/301192/01.07.01_60/en_301192v010701p.pdf) | [en_301_192.md](https://github.com/fishloa/rust-dvb/blob/main/dvb-si/docs/en_301_192.md) |
| TS 102 006 v1.7.1 — System Software Update | [PDF](https://www.etsi.org/deliver/etsi_ts/102000_102099/102006/01.07.01_60/ts_102006v010701p.pdf) | [ts_102_006_ssu.md](https://github.com/fishloa/rust-dvb/blob/main/dvb-si/docs/ts_102_006_ssu.md) |
| TS 102 323 v1.4.1 — TV-Anytime carriage | [PDF](https://www.etsi.org/deliver/etsi_ts/102300_102399/102323/01.04.01_60/ts_102323v010401p.pdf) | [ts_102_323_tva.md](https://github.com/fishloa/rust-dvb/blob/main/dvb-si/docs/ts_102_323_tva.md) |
| TS 102 809 v1.3.1 — application signalling | [PDF](https://www.etsi.org/deliver/etsi_ts/102800_102899/102809/01.03.01_60/ts_102809v010301p.pdf) | [ts_102_809_apps.md](https://github.com/fishloa/rust-dvb/blob/main/dvb-si/docs/ts_102_809_apps.md) |
| TS 102 772 v1.1.1 — MPE-IFEC | [PDF](https://www.etsi.org/deliver/etsi_ts/102700_102799/102772/01.01.01_60/ts_102772v010101p.pdf) | [ts_102_772_mpe_ifec.md](https://github.com/fishloa/rust-dvb/blob/main/dvb-si/docs/ts_102_772_mpe_ifec.md) |
| EN 303 560 v1.1.1 — TTML subtitling | [PDF](https://www.etsi.org/deliver/etsi_en/303500_303599/303560/01.01.01_60/en_303560v010101p.pdf) | [en_303_560_ttml.md](https://github.com/fishloa/rust-dvb/blob/main/dvb-si/docs/en_303_560_ttml.md) |
| TS 102 727 v1.1.1 — MHP (XAIT) | [PDF](https://www.etsi.org/deliver/etsi_ts/102700_102799/102727/01.01.01_60/ts_102727v010101p.pdf) | vendored PDF only (cites give page + table) |
| TR 101 202 v1.2.1 — data broadcasting guidelines | [PDF](https://www.etsi.org/deliver/etsi_tr/101200_101299/101202/01.02.01_60/tr_101202v010201p.pdf) | profile semantics for `carousel` (no syntax tables) |
| ISO/IEC 13818-6 — DSM-CC | not freely redistributable | [iso_13818_6_carousel.md](https://github.com/fishloa/rust-dvb/blob/main/dvb-si/docs/iso_13818_6_carousel.md) (provenance-documented hand transcription) |

The crate has been through five adversarial spec-audit rounds; fixture tests
run against real transponder captures.

## Demux in 10 lines

Feed 188-byte TS packets to [`SiDemux`]; it filters by PID, reassembles
sections, validates CRCs, follows the PAT to PMT PIDs, and version-gates so a
steady carousel emits each table only once. You get a `SectionEvent` per
**changed** section:

```rust
use dvb_si::demux::SiDemux;
use dvb_si::tables::AnyTable;

let mut demux = SiDemux::builder().build();
for packet in ts_packets {                      // each aligned 188-byte packet
    for event in demux.feed(&packet) {          // changed sections only
        if let Ok(AnyTable::Pat(pat)) = event.table() {
            println!("PAT v{} on {}", event.version().unwrap_or(0), event.pid());
            let _ = pat;
        }
    }
}
```

See [`examples/si_dump.rs`](examples/si_dump.rs) for a complete file-reading CLI
(`cargo run -p dvb-si --example si_dump -- file.ts [--json]`).

## Typed dispatch

You rarely match table_ids or descriptor_tags by hand. `AnyTable::parse`
dispatches a complete section to the right typed table; a table's descriptor-loop
field is a `DescriptorLoop` whose `.iter()` yields `AnyDescriptor` values (typed
where known, `Unknown` otherwise, never panicking — `parse_loop` does the same
for a free byte slice); and `DescriptorRegistry` lets you plug in private
descriptors at runtime. All are generated from a single declarative list so the
dispatcher can never drift from the implemented set.

```rust
use dvb_si::descriptors::AnyDescriptor;

for item in eit_event.descriptors.iter() {       // DescriptorLoop::iter()
    match item? {
        AnyDescriptor::ShortEvent(se) => println!("{}", se.event_name.decode()),
        AnyDescriptor::Unknown { tag, .. } => eprintln!("unknown 0x{tag:02X}"),
        _ => {}
    }
}
```

> **Upgrading from 2.x?** Table descriptor-loop fields are now `DescriptorLoop`
> (call `.iter()` / `.raw()`), three tables (`Cat`, `Tsdt`, `Sit`) became
> borrowed, and those loops serialize as typed JSON arrays. See
> **[MIGRATION-3.0.md](MIGRATION-3.0.md)**. (1.x → 2.0:
> [MIGRATION-2.0.md](MIGRATION-2.0.md).)

## Usage

```rust
use dvb_common::Parse;
use dvb_si::tables::sdt::Sdt;

// `section_bytes`: one complete SDT section, e.g. from `SectionReassembler`.
let sdt = Sdt::parse(section_bytes)?;
for service in &sdt.services {
    println!("service_id = {}", service.service_id);
}
```

## Principles

- **Spec fidelity.** Every field in a section's syntax appears in the parsed struct.
- **Parse and construct.** Every parser has a symmetric serializer; round-trip is tested.
- **Zero-copy where possible.** Parsed types borrow from the input via `<'a>` lifetimes.
- **No magic numbers.** Every hex literal outside `#[cfg(test)]` is a named constant or enum.

## Features

Default: `chrono` (MJD+BCD → `DateTime<Utc>`), `ts` (TS packet +
`SectionReassembler`), `serde`.

`serde` is **Serialize-only** — for display/export (JSON via `serde_json`);
parsing FROM JSON is deliberately unsupported, re-parse from the wire bytes.

```toml
dvb-si = { version = "3.0", default-features = false }  # tight build
```

## Family

[`dvb-common`](https://crates.io/crates/dvb-common) — traits + CRC-32\
[`dvb-t2mi`](https://crates.io/crates/dvb-t2mi) — T2-MI, all 12 packet types\
[`dvb-bbframe`](https://crates.io/crates/dvb-bbframe) — S2/S2X/T2 BBFRAME\
For GSE see the existing [`dvb-gse`](https://crates.io/crates/dvb-gse) crate.

## License

Licensed under either of MIT or Apache-2.0, at your option.
