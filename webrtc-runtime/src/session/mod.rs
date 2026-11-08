//! The one WHIP/WHEP signalling engine: a client state machine and a server
//! state machine, each generic over a [`Profile`] that names the direction.
//!
//! WHIP (RFC 9725) is ingest: the client publishes and the server ingests.
//! WHEP (draft-ietf-wish-whep) is egress: the client plays and the server
//! serves. Both run the same `POST` offer, `Location`/`ETag` session resource,
//! trickle-ICE and ICE-restart `PATCH`, and `DELETE` lifecycle; the public
//! [`crate::whip`] and [`crate::whep`] types are aliases of these engines.
//!
//! The behaviours that exist only for one direction are selected by
//! [`Profile::EGRESS`]:
//!
//! | Behaviour | WHIP (`EGRESS = false`) | WHEP (`EGRESS = true`) |
//! |---|---|---|
//! | `POST` answered `406` | plain HTTP error | counter-offer (WHEP draft: server offers, player answers by `PATCH`) |
//! | `POST` answered `409` | plain HTTP error | [`crate::Error::NoPublisher`] |
//! | Server `PATCH` without `If-Match` | `428` ([`crate::Error::PreconditionRequired`], RFC 9725 §4.3.1) | accepted as a trickle update |
//!
//! The counter-offer states and events exist on the shared types but a WHIP
//! session never enters or emits them.

pub mod client;
pub mod server;

mod sealed {
    pub trait Sealed {}
}

/// The direction a session engine is parameterised by: [`Whip`] or [`Whep`].
pub trait Profile: sealed::Sealed + 'static {
    /// `true` for WHEP (the client consumes media), `false` for WHIP.
    const EGRESS: bool;
    /// Type name rendered by the client's `Debug`.
    const CLIENT_NAME: &'static str;
    /// Type name rendered by the server's `Debug`.
    const SERVER_NAME: &'static str;
}

/// WHIP direction marker (RFC 9725, ingest).
#[derive(Debug, Clone, Copy)]
#[non_exhaustive]
pub enum Whip {}

/// WHEP direction marker (draft-ietf-wish-whep, egress).
#[derive(Debug, Clone, Copy)]
#[non_exhaustive]
pub enum Whep {}

impl sealed::Sealed for Whip {}
impl sealed::Sealed for Whep {}

impl Profile for Whip {
    const EGRESS: bool = false;
    const CLIENT_NAME: &'static str = "WhipClient";
    const SERVER_NAME: &'static str = "WhipSession";
}

impl Profile for Whep {
    const EGRESS: bool = true;
    const CLIENT_NAME: &'static str = "WhepPlayer";
    const SERVER_NAME: &'static str = "WhepSession";
}

/// Content types used in WHIP and WHEP signalling.
pub mod content_type {
    /// SDP offer/answer body (`Content-Type` on the initial POST/response).
    pub const SDP: &str = "application/sdp";
    /// Trickle ICE SDP fragment body (`Content-Type` on a PATCH).
    pub const TRICKLE_ICE: &str = "application/trickle-ice-sdpfrag";
    /// RFC 9457 problem-details error body.
    pub const PROBLEM_JSON: &str = "application/problem+json";
}
