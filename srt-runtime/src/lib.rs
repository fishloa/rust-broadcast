//! `srt-runtime` — SRT (Secure Reliable Transport) packet codecs.
//!
//! Spec grounding: [`draft-sharabayko-srt-01`](https://datatracker.ietf.org/doc/html/draft-sharabayko-srt-01)
//! (free, redistributable IETF Internet-Draft), vendored at
//! `specs/ietf_draft_sharabayko_srt_01.txt`; the curated field tables this
//! crate implements against live in `specs/rules/srt-rules.md`.
//!
//! # Scope of this release
//!
//! This release adds a sans-IO **ARQ (Automatic Repeat reQuest) reliability
//! engine** (§4.8/§4.8.1/§4.8.2/§4.10), a **TSBPD delivery scheduler**
//! (§4.5/§4.6/§4.7, curated at `specs/rules/srt-tsbpd.md`), a **LiveCC packet
//! pacing controller** (§5.1, curated at `specs/rules/srt-livecc.md`), a
//! **Rendezvous handshake state machine** (§4.3.2, curated at
//! `specs/rules/srt-rendezvous.md`), and the **HSv5 Caller-Listener handshake
//! state machine** (§4.3.1) on top of the packet codecs (§3, Packet Structure
//! — the 16-byte SRT header, the data packet §3.1, and every control packet
//! type in §3.2, including Handshake with its extension messages: Handshake
//! Extension §3.2.1.1, Key Material §3.2.1.2/§3.2.2, Stream ID §3.2.1.3, Group
//! Membership §3.2.1.4).
//!
//! - [`arq::Sender`] / [`arq::Receiver`] drive the loss-detection, ACK/NAK/
//!   ACKACK exchange, and RTT/RTTVar estimation (§4.8, §4.10) over the
//!   existing [`packet`] codecs — see the `arq` module doc for the full rule
//!   mapping and its explicit non-goals (TLPKTDROP, RTO/congestion control,
//!   send-queue overflow sizing).
//! - [`tsbpd::TsbpdScheduler`] drives receiver-side delivery timing (§4.5,
//!   rule 9's `PktTsbpdTime` formula) and too-late packet drop (§4.6,
//!   `TLPKTDROP_THRESHOLD`) — see the `tsbpd` module doc for the full rule
//!   mapping and its explicit non-goals (drift estimation, sender-side
//!   TLPKTDROP, wrapping-period `TsbpdTimeBase` adjustment).
//! - [`livecc::LiveCC`] drives sender-side packet pacing (§5.1): the
//!   `PKT_SND_PERIOD` inter-packet send interval computed from a running EWMA
//!   of `AvgPayloadSize` and the configured `MAX_BW` (§5.1.1's `MAXBW_SET` /
//!   `INPUTBW_SET` / `INPUTBW_ESTIMATED` modes) — see the `livecc` module doc
//!   for the full formula mapping.
//! - [`filecc::FileCc`] drives sender-side **file/bulk-transfer** congestion
//!   control (§5.2): the two-phase Slow Start / Congestion Avoidance
//!   AIMD algorithm that grows/shrinks both `CWND_SIZE` and `PKT_SND_PERIOD`
//!   — the window-based sibling of [`livecc::LiveCC`]'s pacing-only model —
//!   see the `filecc` module doc for the full formula mapping and its two
//!   flagged spec gaps.
//! - [`caller::CallerHandshake`] / [`listener::ListenerHandshake`] drive the
//!   induction → conclusion exchange (§4.3.1.1/§4.3.1.2): building the wire
//!   packets via the existing [`packet`] codecs, validating the peer's
//!   replies, and exposing [`handshake_sm::NegotiatedParams`] on success.
//! - [`rendezvous::RendezvousHandshake`] drives the symmetric peer-to-peer
//!   exchange (§4.3.2): both sides run the same engine; the cookie contest
//!   (greater cookie wins) resolves which one plays
//!   [`rendezvous::RendezvousRole::Initiator`] vs
//!   [`rendezvous::RendezvousRole::Responder`] at runtime, through the
//!   `Waving -> Attention -> Initiated -> Connected` states.
//!
//! The optional `crypto` feature (off by default) adds the §6 payload
//! **encryption** path on top of that: AES-CTR encrypt/decrypt, RFC 3394 AES
//! key wrap/unwrap of the SEK, and PBKDF2 (HMAC-SHA1) KEK derivation — see
//! [`crypto`]. [`packet::KeyMaterial`] still only carries the wrapped-key
//! *bytes*; [`crypto`] is what actually wraps/unwraps and encrypts/decrypts
//! them. The same feature also wires that primitive into the handshake
//! exchange — [`handshake_sm::CryptoConfig`] (§6.1.5 Key Material Exchange,
//! piggybacked on [`caller::CallerHandshake`]/[`listener::ListenerHandshake`]'s
//! existing CONCLUSION extension flow, reusing [`packet::KeyMaterial`]
//! rather than inventing a new wire message) — and adds a sans-IO
//! SEK-rotation driver for §6.1.6 KM Refresh, [`km_refresh::KmRefreshDriver`].
//!
//! **Explicit follow-ups, not attempted here:**
//! - CUBIC/BBR or any other alternative file-transfer congestion-control
//!   algorithm (§5.2 names them as applicable alternatives to
//!   [`filecc::FileCc`]'s default algorithm but does not describe them).
//! - Wiring [`filecc::FileCc`] / [`livecc::LiveCC`] into a real send-queue
//!   scheduler ([`arq::Sender`] has no congestion-control hook today).
//! - Wiring the negotiated SEK from [`handshake_sm::NegotiatedParams`] (or
//!   [`km_refresh::KmRefreshDriver`]'s events) into [`io`]'s tokio adapter to
//!   actually encrypt/decrypt data-packet payloads end-to-end over a real
//!   socket — the handshake negotiation and the rotation state machine are
//!   both sans-IO and fully wired/tested; driving real payload
//!   encryption from them through `io.rs` (additive, `crypto`-feature-gated,
//!   without touching the existing ARQ/TSBPD paths) is a follow-up.
//!
//! **Permanently out of scope:** the Version-4 legacy Rendezvous path (§4.3.2).
//! Only the current HSv5 Rendezvous flow ([`rendezvous`]) is implemented; V4 is
//! a legacy interop path for pre-HSv5 peers and is not planned.
//!
//! # The sans-IO contract
//!
//! No sockets: [`packet::SrtPacket::parse`] takes the bytes of one UDP
//! datagram and returns a typed packet; the packet's `serialize_into` writes
//! it back out. [`caller::CallerHandshake`] / [`listener::ListenerHandshake`]
//! / [`rendezvous::RendezvousHandshake`] extend the same contract to the
//! handshake *exchange* — `start`/`feed` consume typed packets and return
//! bytes to send plus typed [`handshake_sm::HandshakeOutput`] events;
//! timeouts/retransmits are driven by caller-supplied `tick()` calls, never a
//! wall-clock read from inside the crate.
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
//! - [`arq`] — [`arq::Sender`] / [`arq::Receiver`] (§4.8 ARQ, §4.10 RTT),
//!   [`arq::seq`] (wrap-safe sequence arithmetic), [`arq::rtt::RttEstimator`].
//! - [`tsbpd`] — [`tsbpd::TsbpdScheduler`]: sans-IO TSBPD delivery timing and
//!   too-late packet drop (§4.5/§4.6).
//! - [`livecc`] — [`livecc::LiveCC`] / [`livecc::MaxBwConfig`]: sans-IO LiveCC
//!   packet pacing (§5.1).
//! - [`filecc`] — [`filecc::FileCc`] / [`filecc::Phase`]: sans-IO FileCC
//!   window + pacing congestion control (§5.2).
//! - [`handshake_sm`] — shared handshake types: [`handshake_sm::HandshakeConfig`],
//!   [`handshake_sm::NegotiatedParams`], [`handshake_sm::HandshakeOutput`],
//!   [`handshake_sm::RejectionReason`] (§4.3, Table 7).
//! - [`caller`] — [`caller::CallerHandshake`] (§4.3.1, Caller role).
//! - [`listener`] — [`listener::ListenerHandshake`] (§4.3.1, Listener role).
//! - [`rendezvous`] — [`rendezvous::RendezvousHandshake`] (§4.3.2).
//! - [`error`] — the [`Error`] enum and [`Result`] alias.
//! - [`crypto`] (feature `crypto`) — §6 payload encryption primitives.
//! - [`km_refresh`] (feature `crypto`) — [`km_refresh::KmRefreshDriver`]: the
//!   sans-IO §6.1.6 KM Refresh (SEK-rotation) state machine.
//! - [`io`] (feature `tokio`) — [`io::SrtSocket`] / [`io::SrtListener`]: an
//!   async UDP socket adapter driving the sans-IO handshake + ARQ + TSBPD
//!   engines end-to-end over real sockets.

#![cfg_attr(not(feature = "std"), no_std)]
#![forbid(unsafe_code)]
#![warn(missing_docs)]
#![cfg_attr(docsrs, feature(doc_cfg))]

extern crate alloc;

pub mod arq;
pub mod caller;
#[cfg(feature = "crypto")]
#[cfg_attr(docsrs, doc(cfg(feature = "crypto")))]
pub mod crypto;
pub mod error;
pub mod filecc;
pub mod handshake_sm;
#[cfg(feature = "crypto")]
#[cfg_attr(docsrs, doc(cfg(feature = "crypto")))]
pub mod km_refresh;
pub mod listener;
pub mod livecc;
pub mod packet;
pub mod rendezvous;
pub mod tsbpd;

#[cfg(feature = "tokio")]
#[cfg_attr(docsrs, doc(cfg(feature = "tokio")))]
pub mod io;

pub use caller::{CallerHandshake, CallerHandshakeState};
pub use error::{Error, Result};
pub use handshake_sm::{HandshakeConfig, HandshakeOutput, NegotiatedParams, RejectionReason};
pub use listener::{ListenerHandshake, ListenerHandshakeState};
pub use packet::SrtPacket;
pub use rendezvous::{RendezvousHandshake, RendezvousHandshakeState, RendezvousRole};

/// The Internet-Draft this crate implements packet structure from.
pub const SPEC: &str = "draft-sharabayko-srt-01";
