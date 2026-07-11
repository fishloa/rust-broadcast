//! RTCP — RTP Control Protocol (RFC 3550 §6).
//!
//! Relocated to the standalone [`rtcp_packet`] crate (issue #654, part of
//! epic #653) — this module re-exports it unchanged so every existing
//! `transmux::rtcp::*` (and crate-root `transmux::*`) call site keeps
//! working. See `rtcp-packet/docs/rtcp.md` for the curated spec
//! transcription that is now the implementation/audit oracle for these
//! types, and `rtcp-packet`'s own crate-root doc for the field-by-field
//! wire-format summary.
//!
//! RTCP carries **no media** — this was never a hub `Package`/`Unpackage`
//! spoke, just the wire codec for the RTP control channel; the migration is
//! internal-only and does not change transmux's public API.

pub use rtcp_packet::*;
