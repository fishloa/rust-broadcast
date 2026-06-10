//! ETSI TS 102 773 v1.4.1 DVB-T2 Modulator Interface (T2-MI) parser + builder.
//!
//! Entry points:
//! - [`Parse`](dvb_common::Parse) / [`Serialize`](dvb_common::Serialize) — the two
//!   symmetric contracts every payload type implements.
//! - [`packet`] — T2-MI packet header and type parsing.
//! - [`payload`] — BBFrame, L1, FEF, timestamp, and addressing payload types.
//! - [`payload::PayloadRegistry`] / [`pump::T2miEvent::payload_with`] — register
//!   private packet types and dispatch through the registry.
//! - [`payload::AnyPayload::dispatch_with`] — registry-aware dispatch for custom types.
//! - [`crc`] — CRC-32 per Annex A.
//!
//! # RFU policy
//!
//! Payload parsers REJECT non-zero reserved (rfu) bits with
//! `ReservedBitsViolation` and serialize them as 0 — with one deliberate
//! exception: individual addressing (0x21) PRESERVES its leading rfu byte
//! verbatim so gateway streams round-trip byte-exact (see
//! `payload::individual_addressing`).
//!
//! # Quickstart: pump a TS, get typed payloads
//!
//! [`pump::T2miPump`] filters a TS by PID, reassembles + CRC-validates T2-MI
//! packets, and hands back events whose [`payload`](pump::T2miEvent::payload)
//! dispatches to a typed [`payload::AnyPayload`]:
//!
//! ```no_run
//! # #[cfg(feature = "ts")] {
//! use dvb_t2mi::pump::T2miPump;
//! use dvb_t2mi::payload::AnyPayload;
//!
//! let mut pump = T2miPump::new(0x0006); // T2-MI PID from the PMT
//! # let ts_packets: Vec<[u8; 188]> = Vec::new();
//! for packet in &ts_packets {          // each aligned 188-byte TS packet
//!     for event in pump.feed_ts(packet) {
//!         if let Ok(AnyPayload::Bbframe(bb)) = event.payload() {
//!             println!("BBFrame plp_id={}", bb.plp_id);
//!         }
//!     }
//! }
//! # }
//! ```
//!
//! # The full signal chain
//!
//! T2-MI carries DVB-T2 BBFrames, which carry the inner MPEG-TS, which carries
//! SI. The crates compose end to end — T2-MI here, BBFrame extraction in
//! [`dvb-bbframe`](https://docs.rs/dvb-bbframe), SI demux in
//! [`dvb-si`](https://docs.rs/dvb-si):
//!
//! ```text
//! TS (T2-MI PID) ─▶ T2miPump ─▶ AnyPayload::Bbframe
//!                                   │ bb.bbframe
//!                                   ▼
//!                          dvb_bbframe::Bbheader::parse + up_iter
//!                                   │ inner TS packets
//!                                   ▼
//!                          dvb_si::demux::SiDemux ─▶ AnyTableSection
//! ```
//!
//! A complete, working version of this chain (synthetic fixture, every layer
//! built and asserted) lives in `dvb-t2mi/tests/chain.rs`.
//!
//! # Features
//!
//! | Feature | Default | Enables |
//! |---|---|---|
//! | `ts` | on | [`pump::T2miPump`] — PID-filtered TS reassembly + CRC validation. Off → bring your own complete T2-MI packet bytes. |
//! | `serde` | on | **Serialize-only** — for display/export (JSON via serde_json); parsing FROM JSON is deliberately unsupported, re-parse from wire bytes. `Serialize` on every packet/payload type. |
//! | `yoke` | off | [`yoke::Yokeable`] on the zero-copy payload view types — own a parsed T2-MI payload past the input buffer's borrow without re-parsing. |
//!
//! # Header-only example
//!
//! ```
//! use dvb_t2mi::packet::Header;
//! use dvb_common::Parse;
//! let buf = [0x00u8, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00];
//! let hdr = Header::parse(&buf[..]).unwrap();
//! assert_eq!(hdr.payload_len_bits, 0);
//! ```

#![warn(missing_docs)]

pub mod crc;
pub mod error;
pub mod packet;
pub mod payload;
pub mod traits;

#[cfg(feature = "ts")]
pub mod pump;

#[cfg(feature = "ts")]
pub mod ts;

pub use error::{Error, Result};
