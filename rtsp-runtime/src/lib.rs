//! Sans-IO RTSP 1.0 session engine — RFC 2326 (Real Time Streaming Protocol).
//!
//! This crate fills the gap the ecosystem leaves: a **driveable** RTSP session
//! engine. Message parse/serialize is delegated to the mature [`rtsp_types`] and
//! [`sdp_types`] codecs; authentication math to [`http_auth`]. What lives here is
//! the part nothing else provides — the client and server **session state
//! machines** (RFC 2326 Appendix A), CSeq correlation, `Transport` negotiation,
//! interleaved RTP/RTCP framing (§10.12), and Basic/Digest auth wiring — all
//! sans-IO: you feed inbound bytes and wall-clock, the engine emits outbound
//! bytes and typed events. An optional `tokio` (+ `tls` for `rtsps://`) adapter
//! drives real sockets over the same core.
//!
//! Modules are built out per the epic (issue #521); this is the crate skeleton.

#![forbid(unsafe_code)]

/// Placeholder — the session engine, transport, interleaved framing, and auth
/// modules land per issue #521. Kept so the crate compiles as a workspace member.
pub const RFC: &str = "RFC 2326";
