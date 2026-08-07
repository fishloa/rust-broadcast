//! WHIP (RFC 9725) — WebRTC-HTTP Ingestion Protocol.
//!
//! Sans-IO state machines for both client (encoder/ingester) and server
//! (media server endpoint) sides of the WHIP signalling exchange.

pub mod client;
pub mod server;

/// Content types used in WHIP signalling.
pub mod content_type {
    pub const SDP: &str = "application/sdp";
    pub const TRICKLE_ICE: &str = "application/trickle-ice-sdpfrag";
    pub const PROBLEM_JSON: &str = "application/problem+json";
}

/// HTTP status codes used in WHIP.
pub mod status {
    pub const CREATED: u16 = 201;
    pub const NO_CONTENT: u16 = 204;
    pub const TEMPORARY_REDIRECT: u16 = 307;
    pub const BAD_REQUEST: u16 = 400;
    pub const PRECONDITION_FAILED: u16 = 412;
    pub const UNPROCESSABLE_CONTENT: u16 = 422;
    pub const PRECONDITION_REQUIRED: u16 = 428;
    pub const SERVICE_UNAVAILABLE: u16 = 503;
}
