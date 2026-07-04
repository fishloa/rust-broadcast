//! `srt-runtime` — SRT (Secure Reliable Transport) packet codecs.
//!
//! Spec grounding: [`draft-sharabayko-srt-01`](https://datatracker.ietf.org/doc/html/draft-sharabayko-srt-01)
//! (free, redistributable IETF Internet-Draft), vendored at
//! `specs/ietf_draft_sharabayko_srt_01.txt`; the curated field tables this
//! crate implements against live in `specs/rules/srt-rules.md`.
//!
//! # Scope of this release
//!
//! This is the **packet codec** deliverable only: typed, byte-exact
//! `parse`/`serialize` for every packet type in §3 (Packet Structure) — the
//! 16-byte SRT header, the data packet (§3.1), and every control packet type
//! (§3.2): Handshake (with its extension messages: Handshake Extension,
//! §3.2.1.1; Key Material, §3.2.1.2/§3.2.2; Stream ID, §3.2.1.3; Group
//! Membership, §3.2.1.4), Keep-Alive, ACK, NAK, Congestion Warning, Shutdown,
//! ACKACK, Message Drop Request, and Peer Error.
//!
//! **Explicit follow-ups, not attempted here:**
//! - The handshake **state machine** (caller/listener/rendezvous exchange,
//!   §4.3) — this crate parses/builds handshake *packets*, not a connection.
//! - ARQ / loss handling, TSBPD, congestion control (§4-§5).
//! - Actual AES key-wrap/unwrap **crypto** (§6) — [`packet::KeyMaterial`]
//!   carries the wrapped-key bytes opaquely.
//! - A `tokio` socket adapter (mirroring `rtsp-runtime`'s `io` module).
//!
//! # The sans-IO contract
//!
//! No sockets, no state machine: [`packet::SrtPacket::parse`] takes the bytes
//! of one UDP datagram and returns a typed packet; the packet's
//! `serialize_into` writes it back out. Everything here is a pure, allocating
//! (but not I/O-performing) parse/serialize pair.
//!
//! # Reserved-bit policy
//!
//! Fields the spec documents as fixed-value or reserved-for-future-use
//! (`Subtype` on every Control Type except User-Defined; the header
//! `Type-specific Information` word where a packet type does not use it; the
//! Key Material message's `S`/`V`/`PT`/`Sign`/`Resv1`/`Resv2`/`Resv3` fields)
//! are validated against their spec-mandated value on parse and are not
//! stored in the typed structs — they are reconstructed on serialize. A
//! non-compliant value is a structured [`error::Error`], never a panic.
//!
//! # Module map
//! - [`packet`] — [`packet::SrtPacket`], the data/control packet types, and
//!   their sub-structures (handshake extensions, Key Material, ACK variants,
//!   NAK loss-list coding).
//! - [`error`] — the [`Error`] enum and [`Result`] alias.

#![cfg_attr(not(feature = "std"), no_std)]
#![forbid(unsafe_code)]
#![warn(missing_docs)]
#![cfg_attr(docsrs, feature(doc_cfg))]

extern crate alloc;

pub mod error;
pub mod packet;

pub use error::{Error, Result};
pub use packet::SrtPacket;

/// The Internet-Draft this crate implements packet structure from.
pub const SPEC: &str = "draft-sharabayko-srt-01";
