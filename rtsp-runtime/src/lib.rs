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
mod rfc2326_lex;
pub mod server;
pub mod session_header;
pub mod state;
pub mod transport;

pub use auth::{Authenticator, Credentials};
pub use client::{ClientEvent, ClientSession, HandleOutcome};
pub use error::{Error, Result};
pub use interleaved::InterleavedFrame;
#[cfg(feature = "tokio")]
pub use io::{
    AsyncRtspClient, AsyncRtspServer, RTSP_DEFAULT_PORT, RTSPS_DEFAULT_PORT, RtspTimeouts,
};
pub use server::{ServerEvent, ServerSession};
pub use session_header::SessionHeader;
pub use state::SessionState;
pub use transport::{Delivery, LowerTransport, Transport, TransportMode, TransportSpec};

// Re-export the underlying codec types callers need to inspect events.
pub use rtsp_types::{Method, StatusCode};

/// The RFC this engine implements.
pub const RFC: &str = "RFC 2326";
