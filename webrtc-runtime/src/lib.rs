//! WHIP (RFC 9725) + WHEP (draft-ietf-wish-whep) HTTP signalling engine.
//!
//! Sans-IO state machines for WebRTC-HTTP ingestion (WHIP) and egress (WHEP)
//! session establishment: SDP offer/answer exchange, Trickle ICE candidate
//! addition, ICE restart, and session teardown — all over plain HTTP.
//!
//! The requests and responses are `http` crate types ([`http::Method`],
//! [`http::StatusCode`], [`http::HeaderMap`]) with typed `headers` accessors, so
//! the core is `std` (it was `no_std` before the HTTP-types migration, SP2.2).
//! There is no IO adapter: no sockets, no async runtime, no TLS — the caller
//! drives HTTP and feeds [`HttpRequest`]/[`HttpResponse`] values to/from the
//! state machines.
//!
//! [`HttpRequest`]: whip::client::HttpRequest
//! [`HttpResponse`]: whip::client::HttpResponse

#![cfg_attr(docsrs, feature(doc_cfg))]
#![warn(missing_docs)]

extern crate alloc;

pub mod error;
mod http_util;
pub mod ice;
#[cfg(feature = "media")]
#[cfg_attr(docsrs, doc(cfg(feature = "media")))]
pub mod media;
pub mod whep;
pub mod whip;

pub use error::Error;
