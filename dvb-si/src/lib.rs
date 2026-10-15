//! ETSI EN 300 468 v1.20.1 DVB Service Information parser and builder.
//!
//! `dvb-si` turns a raw MPEG-TS into typed, decoded DVB sections and complete
//! logical tables: feed packets in, get back PAT/PMT/SDT/EIT/… section structs
//! whose text fields decode to UTF-8 and
//! whose descriptor loops walk into typed descriptors. Every layout is cited to
//! the ETSI spec and round-trip tested; the same types serialize back to bytes.
//!
//! # 30-second quickstart
//!
//! Build a [`demux::SiDemux`], feed it TS packets, match on
//! [`tables::AnyTableSection`], walk the descriptor loop with
//! [`descriptors::DescriptorLoop::iter`], and print decoded text via
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
//! # Layer map
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
//! Each layer is independently usable: a caller who already has complete section
//! bytes can skip [`demux`] and call [`tables::AnyTableSection::parse`] directly; a
//! caller with a bare descriptor loop can call [`descriptors::parse_loop`] on it.
//! Use [`collect`] when a table spans multiple sections.
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
//! # Features
//!
//! | Feature | Default | Enables |
//! |---|---|---|
//! | `chrono` | on | MJD + BCD time fields decode to `chrono::DateTime<Utc>` (EIT `start_time()`, TDT/TOT). Off → raw bytes. |
//! | `ts` | on | [`demux::SiDemux`], `mpeg_ts::ts::SectionReassembler`, TS packet parsing (framing now in the `mpeg-ts` crate). Off → bring your own complete section bytes. |
//! | `serde` | on | **Serialize-only** — for display/export (JSON via serde_json); parsing FROM JSON is deliberately unsupported, re-parse from wire bytes. `Serialize` on every table/descriptor; [`text::DvbText`] serializes as its **decoded** UTF-8 string (camelCase JSON). |
//! | `yoke` | off | `yoke::Yokeable` on every zero-copy view type + the `owned::Owned` wrapper — own a parsed view past the input buffer's borrow (store/cache/send across threads) without re-parsing or a mirror type. |
//!
//! ```toml
//! dvb-si = { version = "11", default-features = false }  # tight, no_std-ish build
//! ```
//!
//! # Entry points
//!
//! - [`demux::SiDemux`] — PID-filtered, version-gated section pump (feature `ts`).
//! - [`tables::AnyTableSection`] / [`descriptors::AnyDescriptor`] — trait-driven
//!   dispatch on table_id / descriptor_tag; [`descriptors::parse_loop`] walks a
//!   loop lazily.
//! - [`collect`] — generic multi-section collection plus complete NIT/BAT/SDT/EIT
//!   views with typed descriptor loops.
//! - [`descriptors::DescriptorRegistry`] — register private descriptors at runtime.
//! - [`descriptors::ExtensionRegistry`] — register private tag-extension
//!   sub-descriptors at runtime.
//! - [`tables::TableRegistry`] — register private table_ids at runtime.
//! - [`text::DvbText`] / [`text::LangCode`] — decoded-on-demand Annex A text.
//! - [`Parse`](broadcast_common::Parse) / [`Serialize`](broadcast_common::Serialize) — the two
//!   symmetric contracts every table and descriptor implements.
//! - [`tables`] — `*Section` parsers for PAT, PMT, CAT, TSDT, NIT, BAT, SDT,
//!   EIT, TDT, TOT, RST, DIT, SIT, ST, SAT, AIT, DSM-CC section, UNT, INT, RCT,
//!   CIT, RNT, Container, MPE datagram, MPE-FEC, MPE-IFEC, protection message,
//!   downloadable font info — every allocated table_id in EN 300 468 V1.20.1
//!   Table 2.
//! - [`descriptors`] — every DVB descriptor (tags 0x40..0x7F) plus MPEG-2 descriptors.
//! - [`carousel`] — DSM-CC data-carousel messages (DSI/DII/DDB) + module
//!   reassembly on top of the [`tables::dsmcc`] section framing.
//! - `mpeg_ts::pid::well_known` — reserved DVB/MPEG-2 PIDs (now in the `mpeg-ts` crate).
//! - [`table_id::TableId`] — typed table_id enum.
//! - [`descriptor_tag::DescriptorTag`] — typed descriptor_tag enum.
//!
//! See the crate README and `docs/` for the structured spec reference.
//! `MIGRATION-4.0.md` covers the 3.x → 4.0 API break.

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

extern crate alloc;

pub mod carousel;
pub mod collect;
pub mod compatibility;
pub mod descriptor_tag;
pub mod descriptors;
#[cfg(feature = "chrono")]
pub mod epg;
pub mod error;
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
