//! `srt-runtime` — SRT (Secure Reliable Transport) packet codecs.
//!
//! Spec grounding: [`draft-sharabayko-srt-01`](https://datatracker.ietf.org/doc/html/draft-sharabayko-srt-01)
//! (free, redistributable IETF Internet-Draft), vendored at
//! `specs/ietf_draft_sharabayko_srt_01.txt`; the curated field tables this
//! crate implements against live in `specs/rules/srt-rules.md`.
//!
//! # Scope of this release
//!
//! This release adds a sans-IO **HSv5 Caller-Listener handshake state
//! machine** (§4.3.1) on top of the packet codecs shipped in `0.1.0` (§3,
//! Packet Structure — the 16-byte SRT header, the data packet §3.1, and every
//! control packet type in §3.2, including Handshake with its extension
//! messages: Handshake Extension §3.2.1.1, Key Material §3.2.1.2/§3.2.2,
//! Stream ID §3.2.1.3, Group Membership §3.2.1.4).
//!
//! - [`caller::CallerHandshake`] / [`listener::ListenerHandshake`] drive the
//!   induction → conclusion exchange (§4.3.1.1/§4.3.1.2): building the wire
//!   packets via the existing [`packet`] codecs, validating the peer's
//!   replies, and exposing [`handshake_sm::NegotiatedParams`] on success.
//!
//! The optional `crypto` feature (off by default) adds the §6 payload
//! **encryption** path on top of that: AES-CTR encrypt/decrypt, RFC 3394 AES
//! key wrap/unwrap of the SEK, and PBKDF2 (HMAC-SHA1) KEK derivation — see
//! [`crypto`]. [`packet::KeyMaterial`] still only carries the wrapped-key
//! *bytes*; [`crypto`] is what actually wraps/unwraps and encrypts/decrypts
//! them.
//!
//! **Explicit follow-ups, not attempted here:**
//! - The Rendezvous handshake (§4.3.2, WAVEAHAND/AGREEMENT) — only
//!   Caller-Listener (§4.3.1) is implemented.
//! - ARQ / loss handling, TSBPD delivery, congestion control (§4-§5).
//! - Wiring [`crypto`] into the handshake state machines / a per-connection
//!   SEK-rotation driver (§6.1.6 KM Refresh) — this release adds the crypto
//!   *primitives* only.
//! - A `tokio` socket adapter (mirroring `rtsp-runtime`'s `io` module).
//!
//! # The sans-IO contract
//!
//! No sockets: [`packet::SrtPacket::parse`] takes the bytes of one UDP
//! datagram and returns a typed packet; the packet's `serialize_into` writes
//! it back out. [`caller::CallerHandshake`] / [`listener::ListenerHandshake`]
//! extend the same contract to the handshake *exchange* — `start`/`feed`
//! consume typed packets and return bytes to send plus typed
//! [`handshake_sm::HandshakeOutput`] events; timeouts/retransmits are driven
//! by caller-supplied `tick()` calls, never a wall-clock read from inside the
//! crate.
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
//! - [`handshake_sm`] — shared handshake types: [`handshake_sm::HandshakeConfig`],
//!   [`handshake_sm::NegotiatedParams`], [`handshake_sm::HandshakeOutput`],
//!   [`handshake_sm::RejectionReason`] (§4.3, Table 7).
//! - [`caller`] — [`caller::CallerHandshake`] (§4.3.1, Caller role).
//! - [`listener`] — [`listener::ListenerHandshake`] (§4.3.1, Listener role).
//! - [`error`] — the [`Error`] enum and [`Result`] alias.
//! - [`crypto`] (feature `crypto`) — §6 payload encryption primitives.

#![cfg_attr(not(feature = "std"), no_std)]
#![forbid(unsafe_code)]
#![warn(missing_docs)]
#![cfg_attr(docsrs, feature(doc_cfg))]

extern crate alloc;

pub mod caller;
#[cfg(feature = "crypto")]
#[cfg_attr(docsrs, doc(cfg(feature = "crypto")))]
pub mod crypto;
pub mod error;
pub mod handshake_sm;
pub mod listener;
pub mod packet;

pub use caller::{CallerHandshake, CallerHandshakeState};
pub use error::{Error, Result};
pub use handshake_sm::{HandshakeConfig, HandshakeOutput, NegotiatedParams, RejectionReason};
pub use listener::{ListenerHandshake, ListenerHandshakeState};
pub use packet::SrtPacket;

/// The Internet-Draft this crate implements packet structure from.
pub const SPEC: &str = "draft-sharabayko-srt-01";
