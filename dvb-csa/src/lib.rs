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
//!   byte-identical output from this crate (and the reverse). See
//!   `tests/golden_vectors.rs`.
//!
//! A round-trip test (`descramble(scramble(x)) == x`) proves nothing for a
//! cipher: it passes for any invertible function. The libdvbcsa known-answer
//! vectors are the gate.
//!
//! A second oracle (a TSDuck-scrambled capture) is **not** currently wired
//! up: `tests/fixtures/france-tnt-scrambled-0x02d0.ts` is committed, but the
//! control word it was scrambled with was never recorded, so the fixture
//! cannot be decrypted and no test references it. See its `PROVENANCE.md`.
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
//! The scalar path processes one 64-bit block at a time. The optional
//! `bitsliced` feature adds [`bitsliced`], a batch fast path that transposes
//! the data and evaluates the cipher as a boolean circuit, so every gate acts
//! on [`bitsliced::LANES`] (64) payloads at once.
//!
//! **The unit of parallelism is the payload, not the block.** CSA2 offers
//! little independence *within* one payload: scrambling's block cipher is a
//! reverse CBC (`C[i] = E(P[i] ^ C[i+1])`) and the stream cipher is a chained
//! LFSR, both strictly sequential. Only descrambling's block half is
//! independent per block, and the stream cipher — the sequential part — is
//! about two thirds of the work. So [`bitsliced`] exposes
//! [`scramble_batch`](bitsliced::scramble_batch) /
//! [`descramble_batch`](bitsliced::descramble_batch) over up to 64
//! *independent* payloads, and there is deliberately no bitsliced
//! single-payload entry point.
//!
//! Measured by `benches/throughput.rs` on an Apple M2 Ultra, rustc 1.86.0,
//! over a batch of 64 x 184-byte TS payloads:
//!
//! | Operation  | Scalar        | Bitsliced     | Speed-up |
//! |------------|---------------|---------------|----------|
//! | scramble   | 15.7 MiB/s    | 91.3 MiB/s    | **5.8x** |
//! | descramble | 15.1 MiB/s    | 99.8 MiB/s    | **6.6x** |
//!
//! A batch materially smaller than 64 payloads leaves lanes idle and scales
//! down accordingly; a single payload gains nothing.
//!
//! The bitsliced path is bit-exact with the scalar path — see
//! [`bitsliced`] for the three gates that hold it there.
#![cfg_attr(not(feature = "std"), no_std)]
#![cfg_attr(docsrs, feature(doc_cfg))]
#![warn(missing_docs)]

#[cfg(feature = "bitsliced")]
#[cfg_attr(docsrs, doc(cfg(feature = "bitsliced")))]
pub mod bitsliced;
mod block;
pub mod csa;
pub mod error;
pub mod key;
mod stream;
mod tables;
pub mod ts;

pub use csa::{descramble, scramble};
pub use error::Error;
pub use key::ControlWord;
