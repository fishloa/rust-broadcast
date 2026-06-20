# dvb-si

ETSI EN 300 468 DVB Service Information parser **and builder**, plus the
MPEG-2 PSI tables it builds on, the DVB-allocated companion tables, and the
DSM-CC data carousel.

**Complete coverage: every allocated `table_id` in EN 300 468 V1.19.1
Table 2 (29 section types; 28 dispatched by `AnyTableSection` + the type-keyed MPE datagram view), every allocated `descriptor_tag` in Table 12
(0x40–0x7F, 64 descriptors), and the MPEG-2 systems descriptors (ISO/IEC 13818-1 §2.6, tags 0x02–0x12 plus the codec descriptors AVC/HEVC/AAC/MPEG-4 at 0x1B/0x1C/0x28/0x2A/0x2B/0x38) are implemented**, each with a symmetric
`Parse` / `Serialize` pair and round-trip tests. Layouts are derived from the
ETSI specs (vendored in the repo and transcribed into reviewable markdown) and
validated against live broadcast captures.

**MSRV: 1.81.** `no_std + alloc` supported — disable the default `std` feature for embedded targets (non-Latin text charsets and wall-clock time require `std`).

## API model

`dvb-si` makes the PSI/SI layering explicit:

- Section parsers are named `*Section`: `PatSection`, `NitSection`,
  `SdtSection`, `SitSection`, `EitSection`, and so on. Each value is exactly one
  wire section and still borrows from the input bytes where possible.
- `AnyTableSection` dispatches one complete section by `table_id`. Demux events
  expose this through `SectionEvent::table_section()`.
- `collect::SectionSetCollector` assembles every long-form multi-section table
  using the common `section_number` / `last_section_number` fields. A completed
  set owns the original section bytes and can be parsed generically:
  `complete.table::<PatSection>()`.
- `collect::CompleteNit`, `CompleteBat`, `CompleteSdt`, and `CompleteEit` add
  flattened logical-table views where that is more useful than a vector of
  sections. Their descriptor loops are parsed through `AnyDescriptor` while the
  raw descriptor-loop bytes remain available.
- `collect::EitCollector` handles the EIT schedule rule that spans table IDs
  through `last_table_id`; ordinary section-number collection is not enough for
  schedule EIT. EIT schedule sub-tables version independently, and the
  collector exposes `clear()` / `retain_logical()` for long-running EPG pruning.
- `epg::EpgStore` (feature `chrono`) builds an electronic programme guide over
  `EitCollector`: deduplicated, time-ordered events with decoded short and
  extended (§6.2.15 fragment-concatenated) text, content/ratings/CRIDs,
  `now_and_next(key, at)`, SDT service-name join, and `services()` to enumerate.
- `resync::TsResync` recovers 188-byte (and 204-byte Reed-Solomon) packet
  alignment from an arbitrary byte stream — junk prefixes, mid-stream loss.
- `TsPacket::adaptation_field()` decodes the adaptation field: discontinuity /
  random-access flags, PCR & OPCR (`Pcr::as_27mhz()`), and splice countdown.
- `mux::SectionPacketizer` / `mux::SiMux` are the byte-exact inverse: pack
  serialized section bytes back into 188-byte TS packets (ISO/IEC 13818-1 §2.4.4).
- `TableId` variants use Rust CamelCase names (`TableId::Pat`,
  `TableId::NetworkInformationActual`, `TableId::MpeFec`), while section parser
  types carry the `Section` suffix.

## Quickstart

```rust
use dvb_si::demux::SiDemux;
use dvb_si::descriptors::AnyDescriptor;
use dvb_si::tables::AnyTableSection;

let mut demux = SiDemux::builder().build();

// In real code, `packet` is each aligned 188-byte packet from your TS source
// (file, UDP, tuner). See `dvb-tools dump` for a complete file-reading CLI.
for event in demux.feed(&packet) {
    match event.table_section() {
        Ok(AnyTableSection::SdtSection(sdt)) => {
            for service in &sdt.services {
                for item in service.descriptors.iter().flatten() {
                    if let AnyDescriptor::Service(svc) = item {
                        // DvbText decodes EN 300 468 Annex A -> UTF-8.
                        println!("service: {}", svc.service_name.decode());
                    }
                }
            }
        }
        Ok(AnyTableSection::PatSection(pat)) => {
            println!("PAT v{} on {}", event.version().unwrap_or(0), event.pid());
        }
        _ => {}
    }
}
```

See [`dvb-tools dump`](../dvb-tools/) for a complete file-reading CLI
(`cargo run -p dvb-tools -- dump file.ts [--json]`).

## Layer map

```
TS packets ──► demux::SiDemux ──► SectionEvent
                                    │ .table_section()
                                    ▼
                              tables::AnyTableSection  (PatSection, SdtSection, …)
                                    │ section.<loop field> : DescriptorLoop
                                    ▼
                          descriptors::parse_loop ──► AnyDescriptor
                                    │ field : DvbText / LangCode
                                    ▼
                              text::DvbText::decode() ──► UTF-8 String

SectionEvent.bytes() ──► collect::SectionSetCollector ──► CompleteSectionSet
                                                         │ .table::<T>()
                                                         ├ .nit() / .bat() / .sdt() / .eit()
                                                         ▼
                                                   complete logical tables

serialized sections ──► mux::SectionPacketizer ──► 188-byte TS packets
```

## Section coverage

28 table_ids are dispatched by `AnyTableSection::parse`; `MpeDatagramSection` (0x3E)
is reachable via `AnyTableSection::parse_as` as noted below.
Every section parser has typed header fields, symmetric parse/serialize, and round-trip tests.
Per the crate's zero-copy convention, descriptor loops and repeated sub-structures borrow
from the input bytes. Flat descriptor loops are `DescriptorLoop` values that walk into typed
descriptors; notes below call out only tables that go further or deliberately keep a nested
structure raw.

| table_id | Table | Spec | Notes |
|---|---|---|---|
| 0x00 | PAT — Program Association | ISO/IEC 13818-1 | typed program→PID entries |
| 0x01 | CAT — Conditional Access | ISO/IEC 13818-1 | typed `ca_descriptors()` view |
| 0x02 | PMT — Program Map | ISO/IEC 13818-1 | typed ES loop |
| 0x03 | TSDT — TS Description | ISO/IEC 13818-1 | |
| 0x3A–0x3F | DSM-CC sections | ISO/IEC 13818-6 / EN 301 192 | framing; 0x3B/0x3C payloads typed via [`carousel`](#dsm-cc-data-carousel); 0x3E typed as MPE |
| 0x3E | MPE datagram_section (typed IP/MAC view) | EN 301 192 §7 | MAC reassembly, LLC/SNAP flag, SSI-aware trailer — accessible via `AnyTableSection::parse_as` |
| 0x40/0x41 | NIT actual/other | EN 300 468 §5.2.1 | typed TS loop |
| 0x42/0x46 | SDT actual/other | EN 300 468 §5.2.3 | typed service loop |
| 0x4A | BAT — Bouquet Association | EN 300 468 §5.2.2 | typed TS loop |
| 0x4B | UNT — Update Notification (SSU) | TS 102 006 | typed platform entries |
| 0x4C | INT — IP/MAC Notification | EN 301 192 | |
| 0x4D | SAT — Satellite Access family | EN 300 468 §5.2.11 | typed `SatBody`: Position V2/V3, cell fragment, time association, beamhopping |
| 0x4E–0x6F | EIT p/f + schedule, actual/other | EN 300 468 §5.2.4 | typed event loop; `chrono`-gated MJD+BCD `start_time()` |
| 0x70 | TDT — Time and Date | EN 300 468 §5.2.5 | |
| 0x71 | RST — Running Status | EN 300 468 §5.2.7 | typed event loop |
| 0x72 | ST — Stuffing | EN 300 468 §5.2.8 | |
| 0x73 | TOT — Time Offset | EN 300 468 §5.2.6 | incl. the SSI=0-with-CRC framing exception |
| 0x74 | AIT — Application Information | TS 102 809 | typed application loop; validated vs live HbbTV capture |
| 0x75 | Container | TS 102 323 | `ContainerSection` |
| 0x76 | RCT — Related Content | TS 102 323 | |
| 0x77 | CIT — Content Identifier | TS 102 323 | |
| 0x78 | MPE-FEC | EN 301 192 §9.9 | typed real_time_parameters |
| 0x79 | RNT — Resolution Notification | TS 102 323 | |
| 0x7A | MPE-IFEC | TS 102 772 | typed real_time_parameters |
| 0x7B | Protection message | TS 102 809 §9 | authentication-message + certificate-collection variants by table_id_extension |
| 0x7C | DFIS — Downloadable Font Info | EN 303 560 | typed font_info loop (table_id per EN 300 468 Table 2 NOTE 2) |
| 0x7E | DIT — Discontinuity Information | EN 300 468 | |
| 0x7F | SIT — Selection Information | EN 300 468 | |

Remaining table_id values are *reserved* or *user-defined* in EN 300 468
V1.19.1 Table 2 — there is nothing standardized left to implement.

## Descriptors

**71 descriptor tags are dispatched by `AnyDescriptor`** (via `DescriptorLoop::iter` /
`parse_loop`): 6 MPEG-2/DSM-CC tags, 64 DVB-allocated tags (0x40–0x7F), and the de-facto
private `logical_channel_descriptor` (0x83) — which exists as a variant but is not
auto-dispatched (register it via `DescriptorRegistry` with PDS context).
Unknown or unregistered tags pass through as `AnyDescriptor::Unknown { tag, body }` —
the raw payload is preserved and round-trips losslessly.

### MPEG-2 / DSM-CC descriptors (ISO/IEC 13818-1 / -6)

| tag | Descriptor |
|---|---|
| 0x05 | registration |
| 0x06 | data_stream_alignment |
| 0x09 | CA |
| 0x0A | ISO_639_language |
| 0x0F | private_data_indicator |
| 0x13 | carousel_identifier (ISO/IEC 13818-6) |

### DVB descriptors (EN 300 468, and companion specs)

All 64 allocated tags 0x40–0x7F from EN 300 468 V1.19.1 Table 12 are implemented.
Each parses into a typed struct with a symmetric serializer and round-trip tests;
any free-form byte fields (names, selector tails) stay borrowed `&[u8]`
per the crate's zero-copy convention.

| tag | Descriptor | Spec | Notes |
|---|---|---|---|
| 0x40 | network_name | EN 300 468 | |
| 0x41 | service_list | EN 300 468 | |
| 0x42 | stuffing | EN 300 468 | |
| 0x43 | satellite_delivery_system | EN 300 468 | |
| 0x44 | cable_delivery_system | EN 300 468 | |
| 0x45 | VBI_data | EN 300 468 | typed service loop; one-byte line entries raw per §6.2.47 |
| 0x46 | VBI_teletext | EN 300 468 | |
| 0x47 | bouquet_name | EN 300 468 | |
| 0x48 | service | EN 300 468 | |
| 0x49 | country_availability | EN 300 468 | |
| 0x4A | linkage | EN 300 468 | |
| 0x4B | NVOD_reference | EN 300 468 | |
| 0x4C | time_shifted_service | EN 300 468 | |
| 0x4D | short_event | EN 300 468 | |
| 0x4E | extended_event | EN 300 468 | |
| 0x4F | time_shifted_event | EN 300 468 | |
| 0x50 | component | EN 300 468 | |
| 0x51 | mosaic | EN 300 468 | typed cell + elementary-cell loops, typed cell_linkage variants |
| 0x52 | stream_identifier | EN 300 468 | |
| 0x53 | CA_identifier | EN 300 468 | |
| 0x54 | content | EN 300 468 | |
| 0x55 | parental_rating | EN 300 468 | |
| 0x56 | teletext | EN 300 468 | |
| 0x57 | telephone | EN 300 468 | bit-packed length fields typed |
| 0x58 | local_time_offset | EN 300 468 | |
| 0x59 | subtitling | EN 300 468 | |
| 0x5A | terrestrial_delivery_system | EN 300 468 | |
| 0x5B | multilingual_network_name | EN 300 468 | |
| 0x5C | multilingual_bouquet_name | EN 300 468 | |
| 0x5D | multilingual_service_name | EN 300 468 | |
| 0x5E | multilingual_component | EN 300 468 | |
| 0x5F | private_data_specifier | EN 300 468 | |
| 0x60 | service_move | EN 300 468 | |
| 0x61 | short_smoothing_buffer | EN 300 468 | |
| 0x62 | frequency_list | EN 300 468 | |
| 0x63 | partial_transport_stream | EN 300 468 §7.2.1 | |
| 0x64 | data_broadcast | EN 300 468 | selector raw (interpretation depends on data_broadcast_id) |
| 0x65 | scrambling | EN 300 468 | |
| 0x66 | data_broadcast_id | EN 300 468 / EN 301 192 | id_selector tail raw |
| 0x67 | transport_stream | EN 300 468 | |
| 0x68 | DSNG | EN 300 468 | |
| 0x69 | PDC | EN 300 468 | |
| 0x6A | AC-3 | EN 300 468 Annex D | |
| 0x6B | ancillary_data | EN 300 468 | |
| 0x6C | cell_list | EN 300 468 | both loops typed; 12+12-bit extents unpacked |
| 0x6D | cell_frequency_link | EN 300 468 | both loops typed |
| 0x6E | announcement_support | EN 300 468 | |
| 0x6F | application_signalling | TS 102 809 | |
| 0x70 | adaptation_field_data | EN 300 468 | |
| 0x71 | service_identifier | TS 102 809 | |
| 0x72 | service_availability | EN 300 468 | |
| 0x73 | default_authority | TS 102 323 | |
| 0x74 | related_content | TS 102 323 | |
| 0x75 | TVA_id | TS 102 323 | |
| 0x76 | content_identifier | TS 102 323 | |
| 0x77 | time_slice_fec_identifier | EN 301 192 | |
| 0x78 | ECM_repetition_rate | EN 301 192 | |
| 0x79 | S2_satellite_delivery_system | EN 300 468 | |
| 0x7A | enhanced_AC-3 | EN 300 468 Annex D | |
| 0x7B | DTS | EN 300 468 Annex G | |
| 0x7C | AAC | EN 300 468 Annex H | |
| 0x7D | XAIT_location | TS 102 727 | |
| 0x7E | FTA_content_management | EN 300 468 | |
| 0x7F | extension | EN 300 468 §6.2.18.1 | typed discriminant + typed bodies; see below |

### Private descriptors

| tag | Descriptor | Notes |
|---|---|---|
| 0x83 | logical_channel | EACEM/NorDig private; variant exists, not auto-dispatched — register via `DescriptorRegistry` |

### Extension descriptor registry (tag 0x7F)

The first payload byte (`descriptor_tag_extension`) selects a sub-descriptor
(EN 300 468 §6.4). **30 tag_extensions are fully typed**; everything else is
preserved byte-exact as `ExtensionBody::Raw` and round-trips losslessly. Private
`descriptor_tag_extension` values are surfaced via
`DescriptorLoop::iter_with_extensions(&desc_reg, &ext_reg)` as
`ExtIterItem::CustomExtension`.

| tag_ext | Extension | Spec |
|---|---|---|
| 0x00 | image_icon | Table 145, §6.4.7 |
| 0x01 | CPCM_delivery_signalling | TS 102 825-9 §4.1.5, Table 2 |
| 0x02 | CP | EN 300 468 §6.4.3, Table 113 |
| 0x03 | CP_identifier | EN 300 468 §6.4.6.1, Table 114 |
| 0x04 | T2_delivery_system | Table 133, §6.4.6.3 (cell loop unfolded) |
| 0x05 | SH_delivery_system | Table 119, §6.4.6.2 |
| 0x06 | supplementary_audio | Table 153, §6.4.11 |
| 0x07 | network_change_notify | Table 149, §6.4.9 (cell/change loop unfolded) |
| 0x08 | message | Table 148, §6.4.9 |
| 0x09 | target_region | Table 156, §6.4.12 (region loop unfolded) |
| 0x0A | target_region_name | Table 157, §6.4.13 (region loop unfolded) |
| 0x0B | service_relocated | Table 152, §6.4.10 |
| 0x0C | XAIT_PID | TS 102 727 Table 95, §10.17.3 (16-bit xait_PID) |
| 0x0D | C2_delivery_system | Table 115, §6.4.6.1 |
| 0x0E | DTS-HD | EN 300 468 Annex G.3, Tables G.6–G.10 |
| 0x0F | DTS-Neural | EN 300 468 Annex L.1, Table L.1 |
| 0x10 | video_depth_range | Table 160, §6.4.16.1 (range loop typed) |
| 0x11 | T2MI | Table 158, §6.4.14 |
| 0x13 | URI_linkage | Table 159, §6.4.16.1 |
| 0x14 | CI_ancillary_data | EN 300 468 §6.4.3, Table 112 |
| 0x15 | AC-4 | Annex D §D.5 (first level; toc/extra raw) |
| 0x16 | C2_bundle_delivery_system | Table 139, §6.4.6.4 |
| 0x17 | S2X_satellite_delivery_system | Table 140, §6.4.6.5.2 (channel-bond entries typed) |
| 0x18 | protection_message | TS 102 809 §9.3.3, Table 40 |
| 0x19 | audio_preselection | Table 110, §6.4.1 (preselection loop unfolded) |
| 0x20 | TTML_subtitling | EN 303 560 Table 1, §5.2.1.1 |
| 0x21 | DTS-UHD | EN 300 468 Annex G.5, Table G.15 |
| 0x22 | service_prominence | Table 162c, §6.4.18 (SOGI loop typed; target_region loop raw) |
| 0x23 | vvc_subpictures | Table 162a, §6.4.17 |
| 0x24 | S2Xv2_satellite_delivery_system | Tables 144a–144c, §6.4.6.5.3 |

Unallocated or not-yet-implemented `descriptor_tag_extension` values (e.g. `0x12`,
`0x1A`–`0x1F`, `0x25`+) are preserved byte-exact as `ExtensionBody::Raw` and
round-trip losslessly.

## DSM-CC data carousel

The `carousel` module types the download-protocol payloads carried inside
DSM-CC sections (ISO/IEC 13818-6 §7.2/§7.3 as profiled by DVB — TR 101 202,
TS 102 006 SSU, TS 102 809 object carousels):

| Message / Type | messageId | Notes |
|---|---|---|
| DSI — DownloadServerInitiate | 0x1006 | privateData raw: SSU GroupInfoIndication / OC ServiceGatewayInfo |
| DII — DownloadInfoIndication | 0x1002 | typed module loop |
| DDB — DownloadDataBlock | 0x1003 | |
| `ModuleReassembler` | — | DDB → complete modules per DII geometry: version-aware, out-of-order tolerant, per-module + aggregate memory caps |

The BIOP object-carousel layer (`carousel::biop`) is also implemented:
`ServiceGatewayInfo`, `BiopMessage` (Directory / File / ServiceGateway / Stream / StreamEvent),
IOR + tagged profiles, and the `CarouselFs` virtual-file-tree walker
(`resolve` / `file_bytes`); ISO/IEC 13818-6 §11 as profiled by TR 101 202.

Validated **byte-exact** against a live French-TNT (M6 HbbTV) capture in the
test suite.

## Text decoding

**Full EN 300 468 Annex A Table A.3 selector coverage:**

| Selector | Table | Decoding |
|---|---|---|
| (none, first byte ≥ 0x20) | default Latin, Figure A.1 | glyph-for-glyph (ISO 6937 superset — € at 0xA4, full non-spacing diacritic row with precomposed forms + combining-mark fallback) |
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

The non-Latin charsets (`encoding_rs`) require the `std` feature.

## Typed dispatch

You rarely match table_ids or descriptor_tags by hand. `AnyTableSection::parse`
dispatches a complete section to the right typed section parser; a section's descriptor-loop
field is a `DescriptorLoop` whose `.iter()` yields `AnyDescriptor` values (typed
where known, `Unknown` otherwise, never panicking — `parse_loop` does the same
for a free byte slice). `DescriptorRegistry` lets you plug in private
descriptors at runtime, `TableRegistry` does the same for private table_ids,
and `ExtensionRegistry` handles private tag-extension sub-descriptors (tag 0x7F).
All are generated from a single declarative list so the dispatcher can never drift from the
implemented set.

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

## Multi-section collection and EPG

```rust
use dvb_si::collect::{CompletedEit, EitCollector, SectionSetCollector};
use dvb_si::demux::SiDemux;
use dvb_si::tables::eit;
use dvb_si::tables::nit::{self, NitSection};

let mut demux = SiDemux::builder().build();
let mut nit_sections = SectionSetCollector::new();
let mut eit_sections = EitCollector::new();

for event in demux.feed(&packet) {
    match event.table_id() {
        nit::TABLE_ID_ACTUAL | nit::TABLE_ID_OTHER => {
            if let Some(complete) = nit_sections
                .push_section_with_pid(Some(event.pid().value()), event.bytes())
                .ok()
                .flatten()
            {
                let nit = complete.nit()?;            // flattened logical NIT
                let raw = complete.table::<NitSection>()?; // generic section view
                let _ = (nit, raw);
            }
        }
        eit::TABLE_ID_PF_ACTUAL..=eit::TABLE_ID_SCHEDULE_OTHER_LAST => {
            if let Some(done) = eit_sections
                .push_section_with_pid(Some(event.pid().value()), event.bytes())
                .ok()
                .flatten()
            {
                match done {
                    CompletedEit::PresentFollowing(set) => {
                        let eit = set.eit()?;
                        let _ = eit;
                    }
                    CompletedEit::Schedule(schedule) => {
                        for table in schedule.tables()? {
                            let _ = table.events;
                        }
                    }
                }
            }
        }
        _ => {}
    }
}

// Long-running EPG collectors can prune old state.
eit_sections.retain_logical(|key| application_still_needs(key));
```

## Owning parsed views across threads (feature `yoke`)

Zero-copy parsed types borrow the input slice via `<'a>`. The optional `yoke`
feature bundles the backing bytes and the borrowing view into one `'static`,
cheaply-`Clone`, `Send + Sync` value via the [`yoke`] crate:

```rust
use std::sync::Arc;
use dvb_si::{owned::Owned, tables::pmt::PmtSection};
use dvb_common::Parse;

let bytes: Arc<[u8]> = Arc::from(section);             // own the section
let pmt: Owned<PmtSection<'static>> = Owned::try_new(bytes, |b| PmtSection::parse(b))?;
let view: &PmtSection = pmt.get();                            // no re-parse, no lifetime
```

## Features

```toml
[dependencies]
dvb-si = { version = "7", default-features = false }  # tight build: no_std + alloc
```

| Feature | Default | Description |
|---|---|---|
| `std` | yes | Link std; `no_std + alloc` when off. Non-Latin text charsets and wall-clock time require `std`. |
| `chrono` | yes | MJD+BCD wire fields → `DateTime<Utc>` decoded accessors. Off → raw MJD/BCD bytes returned. |
| `ts` | yes | MPEG-TS packet types, `SectionReassembler`, `SiDemux`, `TsResync`, `SectionPacketizer`, `SiMux`. Off → bring your own section bytes. |
| `serde` | yes | `Serialize` on all parsed types (Serialize-only — re-parse from wire bytes to reconstruct). |
| `yoke` | no | `Yokeable` impls + `Owned<T>` wrapper to retain parsed views past the input buffer's borrow. |
| `flate2` | no | zlib decompression for compressed BIOP modules in object carousels. Off → compressed bytes exposed raw. Requires `std`. |

`serde` is **Serialize-only** — for display/export (JSON via `serde_json`);
parsing FROM JSON is deliberately unsupported — re-parse from the wire bytes.

## Principles

- **Spec fidelity.** Every field in a section's syntax appears in the parsed struct.
- **Parse and construct.** Every parser has a symmetric serializer; round-trip is tested.
- **Zero-copy where possible.** Parsed types borrow from the input via `<'a>` lifetimes.
- **No magic numbers.** Every hex literal outside `#[cfg(test)]` is a named constant or enum.

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
| TS 102 727 v1.1.1 — MHP (DVB-J, XAIT) | [PDF](https://www.etsi.org/deliver/etsi_ts/102700_102799/102727/01.01.01_60/ts_102727v010101p.pdf) | [ts_102_727_mhp.md](https://github.com/fishloa/rust-dvb/blob/main/dvb-si/docs/ts_102_727_mhp.md) (descriptor-syntax subset) |
| TR 101 202 v1.2.1 — data broadcasting guidelines | [PDF](https://www.etsi.org/deliver/etsi_tr/101200_101299/101202/01.02.01_60/tr_101202v010201p.pdf) | profile semantics for `carousel` (no syntax tables) |
| ISO/IEC 13818-6 — DSM-CC | not freely redistributable | [iso_13818_6_carousel.md](https://github.com/fishloa/rust-dvb/blob/main/dvb-si/docs/iso_13818_6_carousel.md) (provenance-documented hand transcription) |

The crate has been through five adversarial spec-audit rounds; fixture tests
run against real transponder captures.

## Upgrading

For 3.x → 4.0, see **[MIGRATION-4.0.md](MIGRATION-4.0.md)**:
section parser renames, `AnyTableSection`, `table_section()`, CamelCase
`TableId`, and multi-section collection. For older breaks, see
**[MIGRATION-3.1.md](MIGRATION-3.1.md)** and
**[MIGRATION-2.0.md](MIGRATION-2.0.md)**.

## Family

[`dvb-common`](https://crates.io/crates/dvb-common) — traits + CRC-32\
[`dvb-t2mi`](https://crates.io/crates/dvb-t2mi) — T2-MI, all 12 packet types\
[`dvb-bbframe`](https://crates.io/crates/dvb-bbframe) — S2/S2X/T2 BBFRAME\
[`dvb-conformance`](https://crates.io/crates/dvb-conformance) — TR 101 290 monitor\
[`dvb-tools`](https://crates.io/crates/dvb-tools) — CLI: dump / services / epg / pids / t2mi\
For GSE see the existing [`dvb-gse`](https://crates.io/crates/dvb-gse) crate.

## Examples

Run with `cargo run -p dvb-si --example <name>`:

- **`build_and_parse_pat`** — build a PAT, serialize it to wire bytes, parse it back (the symmetric contract, no TS source needed).
- **`list_services`** — demux a real capture, list its services (decoded names) and the table sections seen.

## License

Licensed under either of MIT or Apache-2.0, at your option.
