//! The session description over an [`RtpOutput`]'s media sections.
//!
//! transmux describes payload formats only (RFC 6184 §8.1, RFC 3640 §4.1): the
//! session-level lines are the RTSP crate's job, so the tests that need the
//! whole document assemble it here with that one builder. `o=`/`c=` name the
//! loopback, the session name is `transmux RTP`.

use std::net::IpAddr;

use transmux::RtpOutput;

/// `"transmux RTP"` — the session name the tests' session description carries.
pub const SESSION_NAME: &str = "transmux RTP";

/// Render `out`'s media sections as a session description whose `c=` names
/// `connection` (RFC 4566 §5.7).
pub fn session(out: RtpOutput, connection: IpAddr) -> String {
    let origin = sdp_types::Origin::with_ip_addr(0, 0, std::net::Ipv4Addr::LOCALHOST);
    rtsp_runtime::sdp::session_description(
        SESSION_NAME,
        origin,
        sdp_types::Connection::from_ip_addr(connection),
        out.medias,
    )
    .expect("render the SDP session description")
}

/// Render `out`'s media sections as a session description with the loopback
/// `c=` line, the shape every RTP test that inspects the document expects.
pub fn local_session(out: RtpOutput) -> String {
    session(out, IpAddr::V4(std::net::Ipv4Addr::LOCALHOST))
}
