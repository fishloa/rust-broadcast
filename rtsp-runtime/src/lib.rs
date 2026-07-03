//! Sans-IO RTSP 1.0 session engine — RFC 2326 (Real Time Streaming Protocol).
//!
//! This crate fills the gap the ecosystem leaves: a **driveable** RTSP session
//! engine, a client *and* a server. Message parse/serialize is delegated to the
//! mature [`rtsp_types`] and [`sdp_types`] codecs; authentication math to
//! [`http_auth`]. What lives here is the part nothing else provides — the client
//! and server **session state machines** (RFC 2326 Appendix A), `CSeq`
//! correlation, `Transport` negotiation (§12.39), interleaved RTP/RTCP framing
//! (§10.12), and Basic/Digest auth wiring (§14).
//!
//! # The sans-IO contract
//!
//! No sockets live in the core. You drive the engine with bytes and read back
//! bytes + typed events:
//!
//! - [`ClientSession`] — request-builder methods (`options`/`describe`/`setup`/
//!   `play`/`pause`/`teardown`/`get_parameter`) return the outbound request
//!   bytes to write; [`ClientSession::handle_data`] consumes inbound bytes
//!   (responses and interleaved `$` frames) and returns [`ClientEvent`]s. It
//!   correlates `CSeq`, advances the state machine on `2xx`, resets to `Init` on
//!   `3xx`, transparently answers `401` challenges, and captures the `Session`
//!   id/timeout from the SETUP response.
//! - [`ServerSession`] — [`ServerSession::handle_request`] takes inbound request
//!   bytes and returns the response bytes plus [`ServerEvent`]s, validating the
//!   method against the server state table (`455` otherwise), allocating a
//!   session on SETUP, and negotiating `Transport`.
//!
//! An optional `tokio` (+ `tls` for `rtsps://`) socket adapter that drives real
//! connections over this same core is planned for a later release; the features
//! are declared but currently unused.
//!
//! # Module map
//!
//! - [`state`] — [`SessionState`] and the client/server transition functions
//!   (RFC 2326 Appendix A; `docs/state-machines.md`).
//! - [`transport`] — the typed [`Transport`] header (§12.39;
//!   `docs/transport-header.md`).
//! - [`interleaved`] — [`InterleavedFrame`] and the streaming demultiplexer
//!   (§10.12; `docs/interleaved-framing.md`).
//! - [`auth`] — [`Credentials`] and the [`Authenticator`] over `http-auth`
//!   (§14; `docs/auth.md`).
//! - [`client`] — [`ClientSession`] and [`ClientEvent`].
//! - [`server`] — [`ServerSession`] and [`ServerEvent`].
//! - [`error`] — the [`Error`] enum and [`Result`] alias.
//!
//! Methods, status codes, and their state effects are catalogued in
//! `docs/methods-and-status.md`.

#![forbid(unsafe_code)]

pub mod auth;
pub mod client;
pub mod error;
pub mod interleaved;
pub mod server;
pub mod state;
pub mod transport;

pub use auth::{Authenticator, Credentials};
pub use client::{ClientEvent, ClientSession};
pub use error::{Error, Result};
pub use interleaved::InterleavedFrame;
pub use server::{ServerEvent, ServerSession};
pub use state::SessionState;
pub use transport::{Delivery, LowerTransport, Transport, TransportSpec};

// Re-export the underlying codec types callers need to inspect events.
pub use rtsp_types::{Method, StatusCode};

/// The RFC this engine implements.
pub const RFC: &str = "RFC 2326";
