//! DVB Service Information (ETSI EN 300 468 V1.20.1) and MPEG-2 PSI — a typed
//! parser **and** builder for every table and descriptor a DVB transport stream
//! carries.
//!
//! `dvb-si` turns the signalling inside an MPEG transport stream — PAT/PMT,
//! NIT/SDT/BAT, the EIT programme guide, the TDT/TOT clock, the HbbTV AIT, SSU
//! and IP/MAC notification tables, DSM-CC data and object carousels — into typed
//! Rust values whose text fields decode to UTF-8 and whose descriptor loops walk
//! into typed descriptors, and turns those values back into byte-identical wire
//! sections. Every layout is cited to its spec clause and round-trip tested
//! against real broadcast captures.
//!
//! # At a glance
//!
//! The counts below are generated from the crate's own dispatch lists by
//! `tools/gen-dvbsi-coverage.py` (CI fails when they go stale). The full
//! per-code reference — standard, verified clause, Rust type and status for every
//! table_id and descriptor tag, plus every allocated code that is **not**
//! implemented — is
//! [`docs/COVERAGE.md`](https://github.com/fishloa/rust-broadcast/blob/main/dvb-si/docs/COVERAGE.md).
//!
//! <!-- BEGIN GENERATED: dvb-si-at-a-glance (tools/gen-dvbsi-coverage.py) -->
//!
//! | | |
//! |---|---|
//! | Section types (table_id) | **31** typed `*Section` parsers covering **72** table_id values — every allocated EN 300 468 Table 2 id |
//! | Descriptors (tag 0x00–0x7F) | **115** auto-dispatched: **51** MPEG-2 / DSM-CC (0x02–0x3F) + all **64** DVB tags (0x40–0x7F) |
//! | DVB extension descriptors (0x7F) | **30** typed `descriptor_tag_extension` bodies — every one EN 300 468 V1.20.1 Table 109 allocates |
//! | MPEG-2 extension descriptors (0x3F) | **22** typed `extension_descriptor_tag` bodies (7 MPEG-H 3D audio, plus HEVC/VVC/EVC/LCEVC/JPEG XS timing and structure, green, quality, OD update, timed metadata, media service kind) |
//! | AIT descriptors (TS 102 809 / MHP) | **21** typed in the separate AIT descriptor namespace |
//! | INT / UNT / RNT descriptors | **26** typed in their own scoped namespaces (`AnyIntDescriptor`, `AnyUntDescriptor`, `AnyRntDescriptor`) |
//! | Private / regional descriptors | **9** PDS- or registration-scoped opt-ins (NorDig, EACEM/D-Book, China GY/T + T/AI AVS2/AVS3, T/UWA) |
//! | Text (EN 300 468 Annex A) | **7** charset decoders covering every Table A.3 selector + Annex A.1 control codes |
//! | DSM-CC carousel | **12** layers: section framing, DSI/DII/DDB, module reassembly, BIOP object carousel, carousel file tree |
//! | Registries | **171** CA_system_id and **174** private_data_specifier names (TSDuck data, build-time) |
//! | Standards | **19** standards / families — see the table below |
//! | Known gaps | **9** allocated codes not yet typed — listed in [`docs/COVERAGE.md`](https://github.com/fishloa/rust-broadcast/blob/main/dvb-si/docs/COVERAGE.md#not-implemented) |
//!
//! ### Standards supported
//!
//! | Standard | Title | Edition implemented | Scope covered | Status |
//! |---|---|---|---|---|
//! | ETSI EN 300 468 | DVB Service Information (SI) | V1.20.1 (2026-09); RFU defaults audited back to V1.3.1 | all SI tables (NIT, BAT, SDT, EIT, TDT, TOT, RST, ST, SAT, DIT, SIT), Table 12 descriptors 0x40–0x7F, Table 109 extension descriptors, Annex A text, Annexes D/G/H/L audio descriptors | Full (98/98 allocated codes) |
//! | Rec. ITU-T H.222.0 \| ISO/IEC 13818-1 | MPEG-2 Systems — PSI and descriptors | 08/2023 text (clause numbers in COVERAGE.md); MPEG-H 3D audio descriptors 06/2021 | PAT, CAT, PMT, TSDT; Table 2-45 descriptors; Table 2-110 extension descriptors | Partial (80/85 allocated codes) |
//! | ISO/IEC 13818-6 | DSM-CC (data/object carousel) | as profiled by ETSI TR 101 202 V1.2.1 (13818-6 itself is not vendored) | DSM-CC sections, DSI/DII/DDB, module reassembly, BIOP object carousel, carousel_identifier + association_tag descriptors | Partial (3/5 allocated codes) |
//! | ETSI EN 301 192 | DVB data broadcasting | V1.7.1 | MPE datagram, MPE-FEC, INT section; time_slice_fec_identifier + ECM_repetition_rate descriptors; DII carousel descriptors | Full (28/28 allocated codes) |
//! | ETSI TS 102 006 | DVB System Software Update (SSU) | V1.4.1 | UNT section, compatibilityDescriptor, SSU data_broadcast_id selector | Full (16/16 allocated codes) |
//! | ETSI TS 102 809 | Application signalling (HbbTV AIT) | V1.3.1 | AIT section, AIT descriptor namespace, application_signalling + service_identifier descriptors, protection message section + descriptor | Full (16/16 allocated codes) |
//! | ETSI TS 102 727 | DVB-MHP 1.1.3 | V1.1.1 (descriptor subset) | DVB-J application + location AIT descriptors, XAIT_location, XAIT_PID | Partial (11/12 allocated codes) |
//! | ETSI TS 102 323 | TV-Anytime carriage in DVB | V1.4.1 | container, RCT, CIT, RNT sections; default_authority, related_content, TVA_id, content_identifier descriptors | Full (11/11 allocated codes) |
//! | ETSI TS 102 772 | MPE-IFEC | V1.1.1 | MPE-IFEC section | Full (1/1 allocated codes) |
//! | ETSI EN 303 560 | DVB TTML subtitling | V1.1.1 | TTML_subtitling descriptor, downloadable font info section (DFIS) | Full (2/2 allocated codes) |
//! | ETSI TS 102 825-9 | CPCM system adaptation | V1.2.1 | cpcm_delivery_signalling extension descriptor | Full (1/1 allocated codes) |
//! | ETSI TS 101 154 | Audio/video coding in DVB | V2.10.1 | PMT `StreamType` values incl. AVS2/AVS3 (0xD2–0xD5); AVS private_data_specifier rules | Partial |
//! | ETSI TS 101 162 | DVB identifiers | registry data from TSDuck (see registries/README.md) | CA_system_id and private_data_specifier value → name lookups | Partial |
//! | NorDig Unified Requirements | Nordic IRD requirements | v3.2.2 (2025-11) | logical channel descriptor v1 (0x83) + v2 (0x87), PDS 0x00000029 | Full (scoped) |
//! | EACEM / D-Book | Logical channel numbering | private (PDS 0x00000028); no vendored spec | logical_channel descriptor (0x83) | Full (scoped) |
//! | GY/T 270-2013 | China DTV closed captions | 2013 | caption_service_descriptor (0x86), `StreamType::ChinaClosedCaption` | Full (scoped) |
//! | GY/T 420-2025, GY/T 422-2025 | China UHD encapsulation / multi-audio | 2025 | AVS2_video (0x40 under 'AVSV'), AVS3_video; GY/T 422 audio_preselection profile | Full (scoped) |
//! | T/AI 109.6-2022, T/AI 109.7-2024 | AVS3 media format / audio transport | 2022 / 2024 | AVS3_video (0xD1), AVS3_audio (0xD2), AVS2_audio (0xD3) | Full (scoped) |
//! | T/UWA 012.2-2023 | UWA UHD TS encapsulation | 2023 | older AVS3 video descriptor (tag 62) | Full (scoped) |
//!
//! Status: **Full** = every code the edition allocates is typed; **Partial** = some are (the tally counts allocated codes, so a single untyped one shows); **Full (scoped)** = typed, but only decoded inside the private_data_specifier / registration scope the standard requires (opt in via `DescriptorRegistry`); **Not implemented** = bytes are preserved but not typed. Every count above is generated by `tools/gen-dvbsi-coverage.py` from the crate's dispatch lists and CI fails if it goes stale.
//!
//! <!-- END GENERATED: dvb-si-at-a-glance -->
//!
//! # Quick start
//!
//! ## Parse one section, then serialize it back
//!
//! The bytes are the PAT of a real French TNT capture committed in the
//! repository. Every parser has a symmetric serializer, so the round trip is
//! byte-identical:
//!
//! ```
//! use broadcast_common::{Parse, Serialize};
//! use dvb_si::tables::AnyTableSection;
//! use dvb_si::tables::pat::PatSection;
//!
//! let section: [u8; 36] = [
//!     0x00, 0xB0, 0x21, 0x00, 0x06, 0xE5, 0x00, 0x00, 0x00, 0x00, 0xE0, 0x10,
//!     0x06, 0x01, 0xE0, 0x64, 0x06, 0x06, 0xE2, 0x58, 0x06, 0x08, 0xE1, 0xF4,
//!     0x06, 0x09, 0xE2, 0xBC, 0x06, 0x0A, 0xE0, 0xC8, 0x98, 0xF3, 0xCC, 0x2D,
//! ];
//!
//! // Known type: parse it directly. Unknown type: dispatch on table_id.
//! let pat = PatSection::parse(&section)?;
//! assert_eq!(pat.entries.len(), 6); // the NIT pointer + five programmes
//! assert!(matches!(AnyTableSection::parse(&section)?, AnyTableSection::PatSection(_)));
//!
//! let mut out = vec![0u8; pat.serialized_len()];
//! pat.serialize_into(&mut out)?;
//! assert_eq!(out, section);
//! # Ok::<(), dvb_si::Error>(())
//! ```
//!
//! ## Pump TS packets
//!
//! Build a [`demux::SiDemux`] (feature `ts`), feed it 188-byte packets, match on
//! [`tables::AnyTableSection`], walk descriptor loops with
//! [`descriptors::DescriptorLoop::iter`], and decode text with
//! [`text::DvbText::decode`]:
//!
//! ```
//! use dvb_si::demux::SiDemux;
//! use dvb_si::descriptors::AnyDescriptor;
//! use dvb_si::tables::AnyTableSection;
//!
//! let mut demux = SiDemux::builder().build();
//!
//! // In real code, `packet` is each aligned 188-byte packet from your TS source
//! // (file, UDP, tuner). Here we hand-build one PAT packet to keep the doctest
//! // self-contained — see `dvb-tools dump` (in the `dvb-tools` crate) for the
//! // file-reading loop.
//! # let packet = build_pat_packet();
//! for event in demux.feed(&packet) {
//!     match event.table_section() {
//!         Ok(AnyTableSection::SdtSection(sdt)) => {
//!             for service in &sdt.services {
//!                 for item in service.descriptors.iter().flatten() {
//!                     if let AnyDescriptor::Service(svc) = item {
//!                         // DvbText decodes EN 300 468 Annex A → UTF-8.
//!                         println!("service: {}", svc.service_name.decode());
//!                     }
//!                 }
//!             }
//!         }
//!         Ok(AnyTableSection::PatSection(pat)) => {
//!             println!("PAT v{} on {}", event.version().unwrap_or(0), event.pid());
//!             assert_eq!(pat.entries.len(), 1);
//!         }
//!         Ok(other) => { let _ = other; }
//!         Err(_) => {} // malformed section
//!     }
//! }
//!
//! # // Minimal PAT-in-a-TS-packet builder used by the doctest above.
//! # fn build_pat_packet() -> [u8; 188] {
//! #     use broadcast_common::Serialize;
//! #     use dvb_si::tables::pat::{PatSection, PatEntry};
//! #     use mpeg_ts::pid::Pid;
//! #     const PMT_PID: Pid = Pid::new(0x0100);
//! #     let pat = PatSection::new(1, 0, true, 0, 0, vec![PatEntry::new(1, PMT_PID)]);
//! #     let mut section = vec![0u8; pat.serialized_len()];
//! #     pat.serialize_into(&mut section).unwrap();
//! #     const TS_SYNC_BYTE: u8 = 0x47;
//! #     const PAYLOAD_UNIT_START_INDICATOR: u8 = 0x40;
//! #     const PID_LOW_BYTE: u8 = 0x00;
//! #     const PAYLOAD_ONLY: u8 = 0x10;
//! #     const POINTER_FIELD_START: u8 = 0x00;
//! #     const STUFFING_BYTE: u8 = 0xFF;
//! #     let mut pkt = [STUFFING_BYTE; 188];
//! #     pkt[0] = TS_SYNC_BYTE;
//! #     pkt[1] = PAYLOAD_UNIT_START_INDICATOR;
//! #     pkt[2] = PID_LOW_BYTE;
//! #     pkt[3] = PAYLOAD_ONLY;
//! #     pkt[4] = POINTER_FIELD_START;
//! #     pkt[5..5 + section.len()].copy_from_slice(&section);
//! #     pkt
//! # }
//! ```
//!
//! ## Private descriptors inside their scope
//!
//! Private tags are never auto-dispatched; a [`descriptors::DescriptorRegistry`]
//! opts in, scoped by the `private_data_specifier_descriptor` in front of them:
//!
//! ```
//! use dvb_si::descriptors::nordig::NordigLogicalChannelV1;
//! use dvb_si::descriptors::{AnyDescriptor, DescriptorLoop, DescriptorRegistry};
//!
//! // PDS 0x00000029 (NorDig), then NorDig LCD v1: service 1, visible, LCN 5.
//! let bytes = [0x5F, 0x04, 0x00, 0x00, 0x00, 0x29, 0x83, 0x04, 0x00, 0x01, 0xC0, 0x05];
//! let mut registry = DescriptorRegistry::new();
//! registry.with_nordig_lcn();
//! let items: Vec<_> = DescriptorLoop::new(&bytes)
//!     .iter_with(&registry)
//!     .collect::<Result<_, _>>()?;
//! match &items[1] {
//!     AnyDescriptor::Other { value, .. } => {
//!         let lcd = value.downcast_ref::<NordigLogicalChannelV1>().unwrap();
//!         assert_eq!(lcd.entries[0].logical_channel_number, 5);
//!     }
//!     other => panic!("expected the NorDig LCD, got {other:?}"),
//! }
//! # Ok::<(), dvb_si::Error>(())
//! ```
//!
//! # Architecture
//!
//! ```text
//! TS packets ─▶ demux::SiDemux ─▶ SectionEvent
//!                                    │ .table_section()
//!                                    ▼
//!                              tables::AnyTableSection  (PatSection, SdtSection, …)
//!                                    │ section.<loop field> : DescriptorLoop
//!                                    ▼
//!                          descriptors::parse_loop ─▶ AnyDescriptor
//!                                    │ field : DvbText / LangCode
//!                                    ▼
//!                              text::DvbText::decode() ─▶ UTF-8 String
//!
//! SectionEvent.bytes() ─▶ collect::SectionSetCollector ─▶ CompleteSectionSet
//!                                                        │ .table::<T>()
//!                                                        ├ .nit() / .bat() / .sdt() / .eit()
//!                                                        ▼
//!                                                  complete logical tables
//! ```
//!
//! | Module | What it holds |
//! |---|---|
//! | [`tables`] | one `*Section` type per table_id (PSI, SI, AIT, UNT, INT, MPE, TV-Anytime, …) + [`tables::AnyTableSection`] dispatch + [`tables::TableRegistry`] |
//! | [`descriptors`] | one module per descriptor tag; [`descriptors::AnyDescriptor`] / [`descriptors::DescriptorLoop`]; [`descriptors::extension`] (tag 0x7F bodies), [`descriptors::mpeg2_extension`] (tag 0x3F, MPEG-H), [`descriptors::ait`] (AIT namespace); [`descriptors::DescriptorRegistry`] / [`descriptors::ExtensionRegistry`] for private ones |
//! | [`carousel`] | DSM-CC download protocol (DSI/DII/DDB), [`carousel::ModuleReassembler`], BIOP object carousel and [`carousel::CarouselFs`] |
//! | [`compatibility`] | the DSM-CC / SSU `compatibilityDescriptor()` structure |
//! | [`text`] | [`text::DvbText`] / [`text::LangCode`] — EN 300 468 Annex A decoding |
//! | [`collect`] | multi-section collection: [`collect::SectionSetCollector`], [`collect::EitCollector`], complete NIT/BAT/SDT/EIT views |
//! | `demux` | [`SiDemux`](crate::demux::SiDemux): PID-filtered, CRC-checked, version-gated section pump (feature `ts`) |
//! | `epg` | `EpgStore`: bounded EPG with now/next and schedules (feature `chrono`) |
//! | `owned` | `Owned<T>`: keep a zero-copy view past its buffer (feature `yoke`) |
//! | [`table_id`] / [`descriptor_tag`] | the typed [`TableId`] / [`DescriptorTag`] enums |
//! | [`traits`] | `TableDef` / `DescriptorDef` — the per-type consts the dispatch lists are generated from |
//! | [`error`] | the structured [`Error`] every parser and serializer returns |
//!
//! # Standards
//!
//! ETSI EN 300 468 V1.20.1 (DVB SI, incl. Annex A text and Annexes D/G/H/L
//! audio descriptors), Rec. ITU-T H.222.0 | ISO/IEC 13818-1 (PSI + descriptors,
//! MPEG-H 3D audio), ISO/IEC 13818-6 as profiled by ETSI TR 101 202 (DSM-CC),
//! ETSI EN 301 192 (data broadcasting: MPE, MPE-FEC, INT), ETSI TS 102 006
//! (SSU/UNT), ETSI TS 102 809 (AIT, protection messages), ETSI TS 102 727 (MHP
//! subset), ETSI TS 102 323 (TV-Anytime: container, RCT, CIT, RNT), ETSI TS
//! 102 772 (MPE-IFEC), ETSI EN 303 560 (TTML subtitling, DFIS), ETSI TS
//! 102 825-9 (CPCM), and the PDS-scoped regional extensions (NorDig, EACEM /
//! D-Book, China GY/T 270 / 420 / 422, T/AI 109.6 / 109.7, T/UWA 012.2). The
//! table above gives each one's edition and status; `docs/COVERAGE.md` lists
//! every code.
//!
//! # Which API do I use?
//!
//! - **A TS source** → [`demux::SiDemux`], then `SectionEvent::table_section()`.
//! - **Complete section bytes** → [`tables::AnyTableSection::parse`] (or a
//!   specific `tables::<t>::<T>Section::parse`; `parse_with` adds a
//!   [`tables::TableRegistry`]).
//! - **A table spanning sections** → [`collect`]; **an EPG** → `epg::EpgStore`.
//! - **A descriptor loop** → [`descriptors::DescriptorLoop::iter`] (or
//!   [`descriptors::parse_loop`] on a bare slice); unknown tags become
//!   [`descriptors::AnyDescriptor::Unknown`], never a panic.
//! - **Private / regional descriptors** →
//!   [`descriptors::DescriptorLoop::iter_with`] a [`descriptors::DescriptorRegistry`].
//! - **Emitting SI** → build the struct with `new(..)`, then
//!   [`Serialize::serialize_into`](broadcast_common::Serialize::serialize_into);
//!   packetise with `mpeg-ts`.
//! - **JSON** → the `serde` feature (serialize-only; re-parse from wire bytes).
//!
//! # RFU policy
//!
//! One rule for every reserved region (owner decision, supersedes r03-W8/#1077
//! "drop on parse, emit the spec value"):
//!
//! - **Parse is tolerant.** Any value of a `reserved`, `reserved_future_use`
//!   or `reserved_zero_future_use` region is accepted (no rejection) — unlike
//!   `dvb-t2mi`, which validates RFU bits.
//! - **Serialize preserves what was parsed.** The reserved bits are captured
//!   in typed, spec-named fields (`reserved`, `reserved_future_use`,
//!   `reserved_flags`, ... right-aligned to their bit width) and re-emitted
//!   verbatim, so a descriptor from a real sender that deviates from the
//!   edition this crate was transcribed from (e.g. an AC-3 descriptor with
//!   `reserved_flags = 1111`, an MHP-era `application_signalling_descriptor`
//!   whose 16-bit `application_type` has bit 15 clear, a DVB-S descriptor with
//!   `roll_off = 01`) re-serializes **byte-identically**. There is no raw
//!   passthrough blob: every preserved bit is a named, typed field.
//! - **Constructed values carry the edition value.** A struct built with
//!   `new(..)` / `Default` takes the value the governing edition prescribes:
//!   `reserved` / `reserved_future_use` bits are **1** (the DVB / H.222.0
//!   convention that future equipment sees a "1" for unimplemented bits),
//!   `reserved_zero_future_use` bits are **0**, with the explicit per-field
//!   exceptions documented on the field (AC-3 `reserved_flags` is `0000` in
//!   every edition; the J2K extended-branch `reserved(5)` is `00000`).
//! - Where a newer edition gives reserved bits a meaning (the carve-outs
//!   listed in `docs/reserved-bits-edition-audit.md` §4) the bits are typed
//!   with that meaning instead.
//!
//! The per-edition evidence for every default lives in
//! `docs/reserved-bits-edition-audit.md`; the `real_capture_byte_identity`
//! test re-serializes every descriptor found in the real-capture archive and
//! requires byte-identity.
//!
//! # Design notes
//!
//! - **Zero-copy, symmetric.** Every wire type implements
//!   [`Parse`](broadcast_common::Parse) (borrowing from the input) and
//!   [`Serialize`](broadcast_common::Serialize); every type has a round-trip
//!   test. Over-wide values are `Error::FieldOverflow` on serialize, never
//!   masked.
//! - **One dispatch list per namespace.** [`tables::AnyTableSection`],
//!   [`descriptors::AnyDescriptor`], the extension bodies and
//!   [`descriptors::AnyAitDescriptor`] are generated from single `declare_*!`
//!   lists with drift tests, so dispatch (and the generated docs) cannot drift
//!   from the implemented set.
//! - **Labels.** Every spec/field enum has `name()` and a `Display` from
//!   `broadcast_common::impl_spec_display!`.
//! - **`#[non_exhaustive]`** public types with `new(..)` constructors, so
//!   additions are not breaking.
//!
//! # Features
//!
//! | Feature | Default | Enables |
//! |---|---|---|
//! | `std` | on | Links `std`. Off → `#![no_std]` + `alloc`; the non-Latin text charsets and wall-clock time need `std`. |
//! | `chrono` | on | MJD + BCD time fields also decode to `chrono::DateTime<Utc>` (EIT `start_time_chrono()`, TDT/TOT `utc_time()`), local time offsets to `chrono::Duration`; the `epg` module. Off → `broadcast_common::time::MjdBcdDateTime` and raw bytes only. |
//! | `ts` | on | [`demux::SiDemux`] over 188-byte TS packets (framing from the `mpeg-ts` crate). Off → bring your own complete section bytes. |
//! | `serde` | on | **Serialize-only** — for display/export (JSON via serde_json); parsing FROM JSON is deliberately unsupported, re-parse from wire bytes. `Serialize` on every table/descriptor; [`text::DvbText`] serializes as its **decoded** UTF-8 string (camelCase JSON). |
//! | `yoke` | off | `yoke::Yokeable` on every zero-copy view type + the `owned::Owned` wrapper — own a parsed view past the input buffer's borrow (store/cache/send across threads) without re-parsing or a mirror type. |
//! | `flate2` | off | zlib decompression of `compressed_module_descriptor` modules in object carousels (implies `std`). |
//!
//! ```toml
//! dvb-si = { version = "11", default-features = false }  # no_std + alloc
//! ```
//!
//! MSRV 1.95.0. `MIGRATION-4.0.md` covers the 3.x → 4.0 API break.

#![warn(missing_docs)]
#![cfg_attr(not(feature = "std"), no_std)]
#![cfg_attr(docsrs, feature(doc_cfg))]
// Runnable examples, embedded so they render on docs.rs and stay in sync with
// the actual `examples/*.rs` files (shown, not compiled).
#![doc = "\n# Examples\n"]
#![doc = "Two runnable examples ship with this crate (`cargo run -p dvb-si --example <name>`).\n"]
#![doc = "\n## `build_and_parse_pat`\n\n```rust,ignore"]
#![doc = include_str!("../examples/build_and_parse_pat.rs")]
#![doc = "```\n\n## `list_services`\n\n```rust,ignore"]
#![doc = include_str!("../examples/list_services.rs")]
#![doc = "```"]
// The README is the crates.io landing page; `cfg(doctest)` is set only while
// rustdoc collects doctests, so every Rust snippet in it is compiled and run
// by `cargo test --doc` without the README text appearing on docs.rs.
#![cfg_attr(doctest, doc = include_str!("../README.md"))]

extern crate alloc;

pub mod carousel;
pub mod collect;
pub mod compatibility;
pub mod descriptor_tag;
pub mod descriptors;
#[cfg(feature = "chrono")]
pub mod epg;
pub mod error;
pub(crate) mod h222_bits;
mod registry_names;
pub mod table_id;
pub mod tables;
pub mod text;
pub mod traits;
pub(crate) mod wire;

#[cfg(feature = "yoke")]
pub mod owned;

#[cfg(feature = "ts")]
pub mod demux;

pub use descriptor_tag::DescriptorTag;
pub use error::{Error, Result};
pub use table_id::TableId;
