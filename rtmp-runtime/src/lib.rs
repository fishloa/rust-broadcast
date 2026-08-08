//! Sans-IO RTMP 1.0 **ingest** (publish) session engine — Adobe Real-Time
//! Messaging Protocol.
//!
//! Spec grounding: Adobe RTMP 1.0, transcribed at
//! [`docs/rtmp.md`](../docs/rtmp.md) (handshake §5.2, chunk stream §5.3,
//! protocol control messages §5.4, message format §6, message types §7.1,
//! command messages §7.2). AMF0 encoding (used by command/data messages) is
//! `[AMF0]` per that same document's provenance section.
//!
//! # Scope of this release
//!
//! This crate implements two roles, both **publish** (ingest) only:
//!
//! - An **ingest server** ([`server::ServerSession`]): a broadcaster pushes a
//!   stream in via `connect`/`createStream`/`publish`; this engine drives the
//!   handshake and session state machine and hands back typed audio/video/
//!   metadata messages as FLV bytes.
//! - A **publish client** ([`client::ClientSession`]): the other end of that
//!   same exchange — it drives the client-side handshake, auto-advances
//!   `connect` → `createStream` → `publish`, and offers
//!   `send_audio`/`send_video`/`send_metadata` once publishing. It has no
//!   `tokio` socket adapter of its own (unlike [`server::ServerSession`], which
//!   gets one via feature `tokio`) — callers drive its sans-IO
//!   `handle_data`/`start` directly over their own transport.
//!
//! An egress (play) role — pulling a stream, on either the client or server
//! side — is on the roadmap but not implemented yet.
//!
//! # The sans-IO contract
//!
//! No sockets live in the core. You drive the engine with bytes and read back
//! bytes + typed events: feed inbound bytes in, get outbound bytes to write
//! plus a stream of typed events out — mirroring the
//! [`rtsp_runtime`](https://docs.rs/rtsp-runtime) sans-IO client/server split
//! in this same workspace.
//!
//! An optional `tokio` socket adapter (feature `tokio`) drives real
//! connections over this same core.
//!
//! # Module map
//!
//! - [`handshake`] — the C0/C1/C2 + S0/S1/S2 handshake (§5.2).
//! - [`chunk`] — the chunk stream: basic header, message header (4 `fmt`
//!   variants), extended timestamp (§5.3).
//! - [`message`] — RTMP message assembly from chunks, protocol control
//!   messages (§5.4), and the message type catalogue (§6, §7.1).
//! - [`amf0`] — AMF0 value encoding/decoding, used by command and data
//!   messages (`[AMF0]`).
//! - [`server`] — the ingest server session state machine (`connect` →
//!   `createStream` → `publish`, §7.2).
//! - [`client`] — the publish client session state machine, the other end of
//!   that same exchange (issue #744).
//! - `io` (feature `tokio`) — the async socket adapter driving the sans-IO
//!   server session over a real `tokio::net::TcpStream`. There is no
//!   equivalent client adapter; [`client::ClientSession`] is sans-IO only.
//! - [`error`] — the [`RtmpError`] type.
//!
//! The handshake/chunk/message/amf0/server sans-IO engine and the `tokio`
//! adapter (feature `tokio`) are all implemented (#738 Tasks 1-9); the
//! publish client engine (#744) is implemented on top of the same core.

#![forbid(unsafe_code)]
#![warn(missing_docs)]
#![cfg_attr(docsrs, feature(doc_cfg))]

pub mod amf0;
pub mod chunk;
pub mod client;
pub mod error;
pub mod handshake;
#[cfg(feature = "tokio")]
#[cfg_attr(docsrs, doc(cfg(feature = "tokio")))]
pub mod io;
pub mod message;
pub mod server;

pub use error::RtmpError;

/// The Adobe RTMP specification version this engine implements.
pub const RTMP_VERSION: &str = "RTMP 1.0";
