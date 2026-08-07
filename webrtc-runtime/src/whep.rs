//! WHEP (draft-ietf-wish-whep) — WebRTC-HTTP Egress Protocol.
//!
//! Sans-IO state machines for both player (viewer) and server
//! (media server endpoint) sides of the WHEP signalling exchange.

pub mod player;
pub mod server;

/// Content types used in WHEP signalling.
pub mod content_type {
    pub const SDP: &str = "application/sdp";
    pub const TRICKLE_ICE: &str = "application/trickle-ice-sdpfrag";
    pub const PROBLEM_JSON: &str = "application/problem+json";
}

/// HTTP status codes specific to WHEP (superset of WHIP).
pub mod status {
    pub const CREATED: u16 = 201;
    pub const NO_CONTENT: u16 = 204;
    pub const TEMPORARY_REDIRECT: u16 = 307;
    pub const BAD_REQUEST: u16 = 400;
    pub const NOT_ACCEPTABLE: u16 = 406;
    pub const CONFLICT: u16 = 409;
    pub const PRECONDITION_FAILED: u16 = 412;
    pub const UNPROCESSABLE_CONTENT: u16 = 422;
    pub const PRECONDITION_REQUIRED: u16 = 428;
    pub const SERVICE_UNAVAILABLE: u16 = 503;
}
