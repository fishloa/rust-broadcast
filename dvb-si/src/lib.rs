//! ETSI EN 300 468 v1.19.1 DVB Service Information parser and builder.
//!
//! `dvb-si` turns a raw MPEG-TS into typed, decoded DVB tables: feed packets in,
//! get back PAT/PMT/SDT/EIT/… structs whose text fields decode to UTF-8 and
//! whose descriptor loops walk into typed descriptors. Every layout is cited to
//! the ETSI spec and round-trip tested; the same types serialize back to bytes.
//!
//! # 30-second quickstart
//!
//! Build a [`demux::SiDemux`], feed it TS packets, match on [`tables::AnyTable`],
//! walk the descriptor loop with [`descriptors::DescriptorLoop::iter`], and print
//! decoded text via [`text::DvbText::decode`]:
//!
//! ```
//! use dvb_si::demux::SiDemux;
//! use dvb_si::descriptors::AnyDescriptor;
//! use dvb_si::tables::AnyTable;
//!
//! let mut demux = SiDemux::builder().build();
//!
//! // In real code, `packet` is each aligned 188-byte packet from your TS source
//! // (file, UDP, tuner). Here we hand-build one PAT packet to keep the doctest
//! // self-contained — see `examples/si_dump.rs` for the file-reading loop.
//! # let packet = build_pat_packet();
//! for event in demux.feed(&packet) {
//!     match event.table() {
//!         Ok(AnyTable::Sdt(sdt)) => {
//!             for service in &sdt.services {
//!                 for item in service.descriptors.iter().flatten() {
//!                     if let AnyDescriptor::Service(svc) = item {
//!                         // DvbText decodes EN 300 468 Annex A → UTF-8.
//!                         println!("service: {}", svc.service_name.decode());
//!                     }
//!                 }
//!             }
//!         }
//!         Ok(AnyTable::Pat(pat)) => {
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
//! #     use dvb_si::tables::pat::{Pat, PatEntry};
//! #     let pat = Pat {
//! #         transport_stream_id: 1, version_number: 0, current_next_indicator: true,
//! #         section_number: 0, last_section_number: 0,
//! #         entries: vec![PatEntry { program_number: 1, pid: 0x0100 }],
//! #     };
//! #     let mut section = vec![0u8; pat.serialized_len()];
//! #     pat.serialize_into(&mut section).unwrap();
//! #     let mut pkt = [0xFFu8; 188];
//! #     pkt[0] = 0x47; pkt[1] = 0x40; pkt[2] = 0x00; pkt[3] = 0x10; pkt[4] = 0x00;
//! #     pkt[5..5 + section.len()].copy_from_slice(&section);
//! #     pkt
//! # }
//! ```
//!
//! # Layer map
//!
//! ```text
//! TS packets ─▶ demux::SiDemux ─▶ SectionEvent
//!                                    │ .table()
//!                                    ▼
//!                              tables::AnyTable  (PAT, PMT, SDT, EIT, …)
//!                                    │ table.<loop field> : &[u8]
//!                                    ▼
//!                          descriptors::parse_loop ─▶ AnyDescriptor
//!                                    │ field : DvbText / LangCode
//!                                    ▼
//!                              text::DvbText::decode() ─▶ UTF-8 String
//! ```
//!
//! Each layer is independently usable: a caller who already has complete section
//! bytes can skip [`demux`] and call [`tables::AnyTable::parse`] directly; a
//! caller with a bare descriptor loop can call [`descriptors::parse_loop`] on it.
//!
//! # Features
//!
//! | Feature | Default | Enables |
//! |---|---|---|
//! | `chrono` | on | MJD + BCD time fields decode to `chrono::DateTime<Utc>` (EIT `start_time()`, TDT/TOT). Off → raw bytes. |
//! | `ts` | on | [`demux::SiDemux`], [`ts::SectionReassembler`], TS packet parsing. Off → bring your own complete section bytes. |
//! | `serde` | on | **Serialize-only** — for display/export (JSON via serde_json); parsing FROM JSON is deliberately unsupported, re-parse from wire bytes. `Serialize` on every table/descriptor; [`text::DvbText`] serializes as its **decoded** UTF-8 string (camelCase JSON). |
//!
//! ```toml
//! dvb-si = { version = "2.0", default-features = false }  # tight, no_std-ish build
//! ```
//!
//! # Entry points
//!
//! - [`demux::SiDemux`] — PID-filtered, version-gated section pump (feature `ts`).
//! - [`tables::AnyTable`] / [`descriptors::AnyDescriptor`] — trait-driven dispatch
//!   on table_id / descriptor_tag; [`descriptors::parse_loop`] walks a loop lazily.
//! - [`descriptors::DescriptorRegistry`] — register private descriptors at runtime.
//! - [`text::DvbText`] / [`text::LangCode`] — decoded-on-demand Annex A text.
//! - [`Parse`](dvb_common::Parse) / [`Serialize`](dvb_common::Serialize) — the two
//!   symmetric contracts every table and descriptor implements.
//! - [`tables`] — PAT, PMT, CAT, TSDT, NIT, BAT, SDT, EIT, TDT, TOT, RST, DIT, SIT,
//!   ST, SAT, AIT, DSM-CC section, UNT, INT, RCT, CIT, RNT, Container, MPE
//!   datagram, MPE-FEC, MPE-IFEC, protection message, downloadable font info —
//!   every allocated table_id in EN 300 468 V1.19.1 Table 2.
//! - [`descriptors`] — every DVB descriptor (tags 0x40..0x7F) plus MPEG-2 descriptors.
//! - [`carousel`] — DSM-CC data-carousel messages (DSI/DII/DDB) + module
//!   reassembly on top of the [`tables::dsmcc`] section framing.
//! - [`pid::well_known`] — reserved DVB/MPEG-2 PIDs.
//! - [`table_id::TableId`] — typed table_id enum.
//! - [`descriptor_tag::DescriptorTag`] — typed descriptor_tag enum.
//!
//! See the crate README and `docs/` for the structured spec reference, and
//! `MIGRATION-2.0.md` for the 1.x → 2.0 upgrade guide.

#![warn(missing_docs)]

pub mod carousel;
pub mod descriptor_tag;
pub mod descriptors;
pub mod error;
pub mod pid;
pub mod section;
pub mod table_id;
pub mod tables;
pub mod text;
pub mod traits;

#[cfg(feature = "ts")]
pub mod demux;
#[cfg(feature = "ts")]
pub mod ts;

pub use descriptor_tag::DescriptorTag;
pub use error::{Error, Result};
pub use table_id::TableId;
