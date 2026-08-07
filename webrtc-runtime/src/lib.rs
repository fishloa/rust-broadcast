//! WHIP (RFC 9725) + WHEP (draft-ietf-wish-whep) HTTP signalling engine.
//!
//! Sans-IO state machines for WebRTC-HTTP ingestion (WHIP) and egress (WHEP)
//! session establishment: SDP offer/answer exchange, Trickle ICE candidate
//! addition, ICE restart, and session teardown — all over plain HTTP.
//!
//! The core is `no_std`-compatible (feature `std` on by default); an optional
//! `tokio` feature provides real HTTP client/server adapters.

#![cfg_attr(not(feature = "std"), no_std)]
#![cfg_attr(docsrs, feature(doc_cfg))]
#![warn(missing_docs)]

extern crate alloc;

pub mod error;
pub mod ice;
pub mod whep;
pub mod whip;

pub use error::Error;
