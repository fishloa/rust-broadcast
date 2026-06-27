//! ETSI EN 300 468 v1.19.1 DVB Service Information parser and builder.
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
//! #     use dvb_common::Serialize;
//! #     use dvb_si::tables::pat::{PatSection, PatEntry};
//! #     const PMT_PID: u16 = 0x0100;
//! #     let pat = PatSection {
//! #         transport_stream_id: 1, version_number: 0, current_next_indicator: true,
//! #         section_number: 0, last_section_number: 0,
//! #         entries: vec![PatEntry { program_number: 1, pid: PMT_PID }],
//! #     };
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
//! DVB reserved-bit fields carry a semantic distinction:
//! - `reserved_future_use` bits are **emitted as 1** (the DVB convention that
//!   future equipment sees a "1" for unimplemented bits).
//! - `reserved_zero_future_use` bits are **emitted as 0**.
//!
//! Parsers **accept** any value (no rejection on non-zero RFU) — unlike
//! `dvb-t2mi`, which validates RFU bits. This crate prioritises forward
//! compatibility with future broadcast streams.
//!
//! # Features
//!
//! | Feature | Default | Enables |
//! |---|---|---|
//! | `chrono` | on | MJD + BCD time fields decode to `chrono::DateTime<Utc>` (EIT `start_time()`, TDT/TOT). Off → raw bytes. |
//! | `ts` | on | [`demux::SiDemux`], [`ts::SectionReassembler`], TS packet parsing. Off → bring your own complete section bytes. |
//! | `serde` | on | **Serialize-only** — for display/export (JSON via serde_json); parsing FROM JSON is deliberately unsupported, re-parse from wire bytes. `Serialize` on every table/descriptor; [`text::DvbText`] serializes as its **decoded** UTF-8 string (camelCase JSON). |
//! | `yoke` | off | `yoke::Yokeable` on every zero-copy view type + the `owned::Owned` wrapper — own a parsed view past the input buffer's borrow (store/cache/send across threads) without re-parsing or a mirror type. |
//!
//! ```toml
//! dvb-si = { version = "4.0", default-features = false }  # tight, no_std-ish build
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
//! - [`Parse`](dvb_common::Parse) / [`Serialize`](dvb_common::Serialize) — the two
//!   symmetric contracts every table and descriptor implements.
//! - [`tables`] — `*Section` parsers for PAT, PMT, CAT, TSDT, NIT, BAT, SDT,
//!   EIT, TDT, TOT, RST, DIT, SIT, ST, SAT, AIT, DSM-CC section, UNT, INT, RCT,
//!   CIT, RNT, Container, MPE datagram, MPE-FEC, MPE-IFEC, protection message,
//!   downloadable font info — every allocated table_id in EN 300 468 V1.19.1
//!   Table 2.
//! - [`descriptors`] — every DVB descriptor (tags 0x40..0x7F) plus MPEG-2 descriptors.
//! - [`carousel`] — DSM-CC data-carousel messages (DSI/DII/DDB) + module
//!   reassembly on top of the [`tables::dsmcc`] section framing.
//! - [`pid::well_known`] — reserved DVB/MPEG-2 PIDs.
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
pub mod pid {
    //! PID constants and utilities — re-exported from [`mpeg_ts::pid`].
    pub use mpeg_ts::pid::*;
}
mod registry_names;
pub mod section {
    //! Section reassembly — re-exported from [`mpeg_ts::section`].
    pub use mpeg_ts::section::*;
}
pub mod table_id;
pub mod tables;
pub mod text;
pub mod traits;

#[cfg(feature = "yoke")]
pub mod owned;

#[cfg(feature = "ts")]
pub mod demux;
#[cfg(feature = "ts")]
pub mod mux {
    //! Section packetizer / SI mux — re-exported from [`mpeg_ts::mux`].
    pub use mpeg_ts::mux::*;
}
#[cfg(feature = "ts")]
pub mod resync {
    //! TS stream resync — re-exported from [`mpeg_ts::resync`].
    pub use mpeg_ts::resync::*;
}
#[cfg(feature = "ts")]
pub mod ts {
    //! MPEG-TS packet framing — re-exported from [`mpeg_ts::ts`].
    pub use mpeg_ts::ts::*;
}

pub use descriptor_tag::DescriptorTag;
pub use error::{Error, Result};
pub use table_id::TableId;
