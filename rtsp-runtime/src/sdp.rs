//! The one SDP session builder (RFC 4566 §5).
//!
//! A session description is an RTSP concern: it is the `Content-Type:
//! application/sdp` body of the `DESCRIBE` reply (RFC 2326 §10.2) and the
//! `ANNOUNCE` request (RFC 2326 §10.3). RFC 4566 §5 fixes the field order
//! (`v=`, `o=`, `s=`, `i=`, `u=`, `e=`, `p=`, the optional `c=`, `b=`, the
//! time description, session attributes, then each media description), so it
//! lives in the crate that speaks RTSP: a crate owning a container's payload
//! formats builds only the per-track media descriptions (`m=` with its
//! `a=rtpmap`/`a=fmtp`) and hands them over here.
//!
//! The renderer is `sdp-types`' `Session::write`, so the bytes are that
//! library's: one CRLF-terminated line per field, each `a=` on its own line,
//! and a `t=0 0` when the time description is empty.

use sdp_types::{Connection, Media, Origin, Session};

/// Render a session description (RFC 4566 §5) over `medias`.
///
/// `origin` is the `o=` line, `connection` the session-level `c=` line and
/// `name` the session name (`s=`). A caller that transmits elsewhere builds
/// the connection from the address it transmits to
/// ([`Connection::from_ip_addr`]), so the address family is derived from the
/// typed address and no CR, LF or space can reach the line.
///
/// Fails only if the writer reports an IO error, or the bytes it produced are
/// not UTF-8; the latter is checked, never assumed lossily.
pub fn session_description(
    name: &str,
    origin: Origin,
    connection: Connection,
    medias: Vec<Media>,
) -> crate::Result<String> {
    let mut session = Session::new(origin, name);
    session.set_connection(connection);
    session.medias = medias;

    let mut out = Vec::new();
    session
        .write(&mut out)
        .map_err(|source| crate::Error::SdpWrite {
            what: "write",
            source,
        })?;
    String::from_utf8(out).map_err(|source| crate::Error::SdpWrite {
        what: "utf-8",
        source: std::io::Error::new(std::io::ErrorKind::InvalidData, source),
    })
}
