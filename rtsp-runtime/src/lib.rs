#![doc = include_str!("../README.md")]
#![forbid(unsafe_code)]
#![cfg_attr(docsrs, feature(doc_cfg))]

pub mod auth;
pub mod client;
#[cfg(feature = "tokio")]
mod codec;
pub mod error;
mod framing;
mod headers_util;
pub mod interleaved;
#[cfg(feature = "tokio")]
pub mod io;
mod limits;
pub mod play;
mod rfc2326_lex;
pub mod sdp;
pub mod server;
pub mod session_header;
pub mod state;
pub mod transport;

pub use auth::{Authenticator, Credentials};
pub use client::{ClientEvent, ClientSession, HandleOutcome, RtpInfoEntry};
pub use error::{Error, IoContext, MessageKind, Result};
pub use interleaved::InterleavedFrame;
#[cfg(feature = "tokio")]
pub use io::{AsyncRtspClient, AsyncRtspServer, RtspClientConfig, RtspServerConfig};
pub use play::{MAX_TRACKS, PlannedSetup, PlayPlan, PlaySession, PlayTrack, SkippedMedia};
pub use server::{ServerEvent, ServerSession};
pub use session_header::SessionHeader;
pub use state::SessionState;
pub use transport::{Delivery, LowerTransport, Transport, TransportMode, TransportSpec};

pub use sdp::session_description;

// Re-export the underlying codec types callers need to inspect events.
pub use rtsp_types::{Method, StatusCode};

/// Default TCP port for `rtsp://` (RFC 2326 §1 / IANA `rtsp`).
pub const RTSP_DEFAULT_PORT: u16 = 554;

/// Default TCP port for `rtsps://` (RTSP over TLS; IANA `rtsps`).
pub const RTSPS_DEFAULT_PORT: u16 = 322;

/// The RFC this engine implements.
pub const RFC: &str = "RFC 2326";
