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
/// The `control` attribute value naming a session-less presentation's one
/// track: `trackID=0` (RFC 2326 Appendix C.1.1's `trackID` form).
pub const TRACK_CONTROL: &str = "trackID=0";

/// The track's own control URL: `base` with [`TRACK_CONTROL`] appended as a
/// final path segment (RFC 2326 §12.39 `control`, Appendix C.1.1).
///
/// Built with [`url::Url::path_segments_mut`], so a trailing slash never
/// doubles, an IPv6 host stays bracketed and the query is preserved. Fails for
/// a URL with no authority the segments can be set on — a cannot-be-a-base
/// URL such as `rtsp:cam`, which `Url::parse` accepts but which names no host
/// to dial.
pub fn control_url(base: &url::Url) -> crate::Result<String> {
    let mut url = base.clone();
    {
        let mut segments = url
            .path_segments_mut()
            .map_err(|()| crate::Error::InvalidUrl {
                what: "rtsp URL has no authority (expected rtsp://host[:port]/path)",
            })?;
        segments.pop_if_empty();
        segments.push(TRACK_CONTROL);
    }
    Ok(url.to_string())
}

/// Resolve a per-media / session `a=control` value to a SETUP URI using the
/// broadcast-client semantics (RFC 2326 §C.1.1): an aggregate control value
/// (`"*"`) and a missing value both resolve to `base`; any other value is
/// joined onto `base` and kept only when it shares `request_url`'s authority
/// (scheme/host/port). Returns `None` when the joined URL crosses authorities.
pub fn resolve_control(
    request_url: &url::Url,
    base: &url::Url,
    control: Option<&str>,
) -> Option<String> {
    let default_port = if request_url.scheme() == "rtsps" {
        322
    } else {
        554
    };
    // rtsp-runtime constants not available here; use literals matching defaults.
    let same_authority = |u: &url::Url| {
        let rp = request_url.port().unwrap_or(default_port);
        let up = u.port().unwrap_or(if u.scheme() == "rtsps" { 322 } else { 554 });
        u.scheme() == request_url.scheme()
            && u.host() == request_url.host()
            && up == rp
    };
    match control {
        None => Some(request_url.to_string()),
        Some("*") => Some(base.to_string()),
        Some(c) => base
            .join(c)
            .ok()
            .filter(|u| same_authority(u))
            .map(|u| u.to_string()),
    }
}
