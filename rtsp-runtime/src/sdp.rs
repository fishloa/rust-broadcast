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
    fn default_port(scheme: &str) -> u16 {
        if scheme == "rtsps" {
            crate::RTSPS_DEFAULT_PORT
        } else {
            crate::RTSP_DEFAULT_PORT
        }
    }
    let same_authority = |u: &url::Url| {
        let rp = request_url
            .port()
            .unwrap_or(default_port(request_url.scheme()));
        let up = u.port().unwrap_or(default_port(u.scheme()));
        u.scheme() == request_url.scheme() && u.host() == request_url.host() && up == rp
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

#[cfg(test)]
mod tests {
    use super::*;
    use url::Url;

    #[test]
    fn control_helper_aggregate_or_missing() {
        let base = Url::parse("rtsp://cam/base").unwrap();
        assert_eq!(
            resolve_control(&base, &base, None).unwrap(),
            base.to_string()
        );
        assert_eq!(
            resolve_control(&base, &base, Some("*")).unwrap(),
            base.to_string()
        );
    }

    fn video() -> sdp_types::Media {
        let mut media = sdp_types::Media::new(
            sdp_types::MediaType::Video,
            0,
            sdp_types::TransportProto::RtpAvp,
            96,
        );
        media.add_attribute_with_value("control", "streamid=0");
        media.add_attribute_with_value("rtpmap", "96 H264/90000");
        media.add_attribute_with_value("fmtp", "96 packetization-mode=1");
        media
    }

    fn audio() -> sdp_types::Media {
        let mut media = sdp_types::Media::new(
            sdp_types::MediaType::Audio,
            0,
            sdp_types::TransportProto::RtpAvp,
            97,
        );
        media.add_attribute_with_value("control", "streamid=1");
        media.add_attribute_with_value("rtpmap", "97 MPEG4-GENERIC/48000/2");
        media
    }

    #[test]
    fn a_built_session_description_carries_every_field_and_parses_back() {
        let sdp = session_description(
            "cam",
            Origin::with_ip_addr(0, 0, std::net::Ipv4Addr::LOCALHOST),
            Connection::from_ip_addr(std::net::IpAddr::V4(std::net::Ipv4Addr::LOCALHOST)),
            vec![video(), audio()],
        )
        .expect("rendered");

        let expected = "v=0\r\n\
                        o=- 0 0 IN IP4 127.0.0.1\r\n\
                        s=cam\r\n\
                        c=IN IP4 127.0.0.1\r\n\
                        t=0 0\r\n\
                        m=video 0 RTP/AVP 96\r\n\
                        a=control:streamid=0\r\n\
                        a=rtpmap:96 H264/90000\r\n\
                        a=fmtp:96 packetization-mode=1\r\n\
                        m=audio 0 RTP/AVP 97\r\n\
                        a=control:streamid=1\r\n\
                        a=rtpmap:97 MPEG4-GENERIC/48000/2\r\n";
        assert_eq!(sdp, expected);

        let parsed = Session::parse(sdp.as_bytes()).expect("the rendered SDP parses back");
        assert_eq!(parsed.session_name, "cam");
        assert_eq!(parsed.medias.len(), 2);
        assert_eq!(parsed.medias[0].media, "video");
        assert_eq!(parsed.medias[1].media, "audio");
    }

    #[test]
    fn an_explicit_default_port_on_a_control_url_stays_on_the_request_authority() {
        let request = Url::parse("rtsp://h/a").unwrap();
        assert_eq!(
            resolve_control(&request, &request, Some("rtsp://h:554/b")).as_deref(),
            Some("rtsp://h:554/b")
        );
        let request = Url::parse("rtsps://h/a").unwrap();
        assert_eq!(
            resolve_control(&request, &request, Some("rtsps://h:322/b")).as_deref(),
            Some("rtsps://h:322/b")
        );
    }

    #[test]
    fn a_control_url_on_the_other_schemes_default_port_crosses_authority() {
        let request = Url::parse("rtsp://h/a").unwrap();
        assert_eq!(
            resolve_control(&request, &request, Some("rtsp://h:322/b")),
            None
        );
        let request = Url::parse("rtsps://h/a").unwrap();
        assert_eq!(
            resolve_control(&request, &request, Some("rtsps://h:554/b")),
            None
        );
    }
}
