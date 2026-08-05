//! DVB Common Scrambling Algorithm (CSA2) — the cipher underneath conditional
//! access on DVB-S, DVB-T, and DVB-C.
//!
//! # Status: reverse-engineered, not spec-cited
//!
//! **DVB-CSA has no public normative specification.** The algorithm was
//! confidential and licensed through the ETSI custodian; every open
//! implementation is reverse-engineered. This crate therefore cannot follow
//! the workspace's usual cite-the-spec-clause discipline. Correctness is
//! established by agreement with **independent implementations** instead of
//! a standard reference:
//!
//! - **libdvbcsa** 1.1.0 — VideoLAN's reference free implementation.
//!   Committed known-answer vectors: encrypt with libdvbcsa, require
//!   byte-identical output from this crate (and the reverse).
//! - **TSDuck** 3.44 — a DVB test tool. Committed scrambled TS fixture:
//!   scramble an FTA capture with an invented control word using TSDuck,
//!   descramble with this crate, require byte-identical recovery.
//!
//! A round-trip test (`descramble(scramble(x)) == x`) proves nothing for a
//! cipher: it passes for any invertible function. The oracle fixtures are
//! the gate.
//!
//! # Algorithm overview
//!
//! DVB-CSA2 combines a **block cipher** and a **stream cipher**, both keyed
//! by the same 8-byte control word:
//!
//! - **Block cipher**: 56-round substitution/permutation network on 64-bit
//!   (8-byte) blocks, applied in a CBC-like chained mode across all
//!   complete 8-byte blocks of the payload.
//! - **Stream cipher**: LFSR-based byte-stream generator seeded from the
//!   nibble-swapped control word and the encrypted first block as IV,
//!   XOR'd with bytes 8..end of the payload.
//!
//! The combination order matters:
//! - **Encrypt**: block-cipher CBC (last block first), then stream-cipher XOR.
//! - **Decrypt**: stream-cipher XOR first, then block-cipher CBC undo.
//!
//! Payloads shorter than 8 bytes are passed through unchanged (per
//! libdvbcsa's behaviour).
//!
//! # Performance
//!
//! The default scalar path processes one 64-bit block at a time. The
//! `bitsliced` feature enables a bitsliced parallel path that processes
//! 64 blocks at once, differentially tested against the scalar reference.
#![cfg_attr(not(feature = "std"), no_std)]
#![cfg_attr(docsrs, feature(doc_cfg))]
#![warn(missing_docs)]

extern crate alloc;

// TODO: implement modules
