//! WHIP (RFC 9725) — WebRTC-HTTP Ingestion Protocol.
//!
//! Sans-IO state machines for both client (encoder/ingester) and server
//! (media server endpoint) sides of the WHIP signalling exchange.

pub mod client;
pub mod server;

/// Content types used in WHIP signalling.
pub mod content_type {
    /// SDP offer/answer body (`Content-Type` on the initial POST/response).
    pub const SDP: &str = "application/sdp";
    /// Trickle ICE SDP fragment body (`Content-Type` on a PATCH).
    pub const TRICKLE_ICE: &str = "application/trickle-ice-sdpfrag";
    /// RFC 9457 problem-details error body.
    pub const PROBLEM_JSON: &str = "application/problem+json";
}

/// HTTP status codes used in WHIP.
pub mod status {
    /// Resource created; the SDP answer is in the response body.
    pub const CREATED: u16 = 201;
    /// Request succeeded with no response body (e.g. a successful PATCH/DELETE).
    pub const NO_CONTENT: u16 = 204;
    /// The resource has moved; retry against the `Location` header.
    pub const TEMPORARY_REDIRECT: u16 = 307;
    /// The request body or headers were malformed.
    pub const BAD_REQUEST: u16 = 400;
    /// The `If-Match` `ETag` precondition on the request failed.
    pub const PRECONDITION_FAILED: u16 = 412;
    /// The request was well-formed but semantically invalid (e.g. bad SDP).
    pub const UNPROCESSABLE_CONTENT: u16 = 422;
    /// The server requires an `If-Match` precondition the request omitted.
    pub const PRECONDITION_REQUIRED: u16 = 428;
    /// The server is temporarily unable to accept the request.
    pub const SERVICE_UNAVAILABLE: u16 = 503;
}
