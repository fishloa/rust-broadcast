//! `rtmp[s]://host[:port]/app/stream-key` -> typed target, built with the `url` crate.
//! Replaces `format!("rtmp://{host}:{port}/{app}")`, which produces `rtmp://::1:1935/live`
//! for an IPv6 address (defect 8) and leaks userinfo/query into `tcUrl`.

#[cfg(feature = "tokio")]
use std::net::{IpAddr, SocketAddr};

use url::{Host, Url};

/// IANA-assigned RTMP port, used when the URL names none.
pub const RTMP_DEFAULT_PORT: u16 = 1935;
/// Port an `rtmps://` URL without one connects to: RTMP over TLS is carried on
/// the HTTPS port (443), the de-facto deployment (no IANA assignment exists).
pub const RTMPS_DEFAULT_PORT: u16 = 443;
/// URL scheme of plain RTMP.
pub const SCHEME_RTMP: &str = "rtmp";
/// URL scheme of RTMP over TLS.
pub const SCHEME_RTMPS: &str = "rtmps";

/// A parsed RTMP publish target.
#[derive(Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct RtmpTarget {
    /// Host, never a pre-formatted string. `rtmp` is a non-special URL scheme, so
    /// the `url` crate reports an IPv4 literal as `Host::Domain("127.0.0.1")`
    /// (`resolve` handles it, `lookup_host` accepts literals); only a bracketed IPv6
    /// literal becomes `Host::Ipv6`.
    pub host: Host<String>,
    /// TCP port; 1935 (`rtmp`) or 443 (`rtmps`) when the URL has none.
    pub port: u16,
    /// Whether the scheme is `rtmps`: the connection must run over TLS, which
    /// the caller supplies (the tokio adapter's [`AsyncRtmpClient::connect`]
    /// opens plain TCP and refuses a TLS target).
    ///
    /// [`AsyncRtmpClient::connect`]: crate::io::AsyncRtmpClient::connect
    pub tls: bool,
    /// The RTMP `app`, percent-decoded (RFC 3986 §2.1).
    pub app: String,
    /// The path remainder, percent-decoded (RFC 3986 §2.1), plus `?query` kept
    /// as written: the query is an opaque string the origin parses itself.
    pub stream_key: String,
    /// `rtmp[s]://host[:port]/app`, IPv6 bracketed, userinfo/query/fragment removed.
    pub tc_url: String,
}

/// Manual `Debug`: the stream key (path remainder plus query) is the publish
/// secret and never renders (issue #1142).
impl core::fmt::Debug for RtmpTarget {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("RtmpTarget")
            .field("host", &self.host)
            .field("port", &self.port)
            .field("app", &self.app)
            .field("stream_key", &"<redacted>")
            .field("tc_url", &self.tc_url)
            .finish()
    }
}

/// Why an RTMP URL was rejected.
#[derive(Debug, thiserror::Error, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum RtmpUrlError {
    /// The text is not a URL.
    #[error("invalid URL: {0}")]
    Parse(String),
    /// The scheme is not `rtmp`.
    #[error("unsupported scheme {0:?} (only rtmp and rtmps)")]
    Scheme(String),
    /// A percent-decoded path segment is not valid UTF-8.
    #[error("percent-decoded path is not valid UTF-8")]
    PathNotUtf8,
    /// The URL has no host.
    #[error("URL has no host")]
    NoHost,
    /// The URL has no app segment.
    #[error("URL has no app segment")]
    NoApp,
    /// The URL has no stream key after the app.
    #[error("URL has no stream key")]
    NoStreamKey,
}

impl RtmpTarget {
    /// Parses `rtmp[s]://host[:port]/app/stream-key[?query]`. The app is the
    /// first path segment; the stream key is the rest of the path, with the
    /// query appended as written. Path text is percent-decoded (RFC 3986
    /// §2.1: `a%20b` names `a b`); a key that is only a query is no key.
    pub fn parse(url: &str) -> Result<Self, RtmpUrlError> {
        let base = parse_base(url)?;
        let mut segs = base.path_segments().ok_or(RtmpUrlError::NoApp)?;
        let app = decode(
            segs.next()
                .filter(|s| !s.is_empty())
                .ok_or(RtmpUrlError::NoApp)?,
        )?;
        let rest: Vec<&str> = segs.collect();
        let mut key = decode(&rest.join("/"))?;
        if key.is_empty() {
            return Err(RtmpUrlError::NoStreamKey);
        }
        if let Some(q) = base.query() {
            key.push('?');
            key.push_str(q);
        }
        Self::build(&base, &app, &key)
    }

    /// Host and port from `base` (any path/query is ignored), `app` and
    /// `stream_key` given separately.
    pub fn from_parts(base: &str, app: &str, stream_key: &str) -> Result<Self, RtmpUrlError> {
        Self::build(&parse_base(base)?, app, stream_key)
    }

    fn build(base: &Url, app: &str, stream_key: &str) -> Result<Self, RtmpUrlError> {
        let host = base.host().ok_or(RtmpUrlError::NoHost)?.to_owned();
        let tls = base.scheme() == SCHEME_RTMPS;
        let mut tc = base.clone();
        tc.set_path(&format!("/{app}"));
        tc.set_query(None);
        tc.set_fragment(None);
        let _ = tc.set_username("");
        let _ = tc.set_password(None);
        Ok(RtmpTarget {
            host,
            port: base.port().unwrap_or(if tls {
                RTMPS_DEFAULT_PORT
            } else {
                RTMP_DEFAULT_PORT
            }),
            tls,
            app: app.to_string(),
            stream_key: stream_key.to_string(),
            tc_url: tc.to_string(),
        })
    }

    /// Resolved socket addresses (DNS only for `Host::Domain`).
    #[cfg(feature = "tokio")]
    pub async fn resolve(&self) -> std::io::Result<Vec<SocketAddr>> {
        match &self.host {
            Host::Ipv4(a) => Ok(vec![SocketAddr::new(IpAddr::V4(*a), self.port)]),
            Host::Ipv6(a) => Ok(vec![SocketAddr::new(IpAddr::V6(*a), self.port)]),
            Host::Domain(d) => Ok(tokio::net::lookup_host((d.as_str(), self.port))
                .await?
                .collect()),
        }
    }
}

fn parse_base(url: &str) -> Result<Url, RtmpUrlError> {
    let u = Url::parse(url).map_err(|e| RtmpUrlError::Parse(e.to_string()))?;
    if u.scheme() != SCHEME_RTMP && u.scheme() != SCHEME_RTMPS {
        return Err(RtmpUrlError::Scheme(u.scheme().to_string()));
    }
    Ok(u)
}

fn decode(raw: &str) -> Result<String, RtmpUrlError> {
    percent_encoding::percent_decode_str(raw)
        .decode_utf8()
        .map(std::borrow::Cow::into_owned)
        .map_err(|_| RtmpUrlError::PathNotUtf8)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plain_host_default_port() {
        let t = RtmpTarget::parse("rtmp://example.test/live/key").unwrap();
        assert_eq!(
            (t.port, t.app.as_str(), t.stream_key.as_str()),
            (1935, "live", "key")
        );
        assert_eq!(t.tc_url, "rtmp://example.test/live");
    }

    /// Defect 8: an IPv6 host must stay bracketed in tcUrl; building it from a bare
    /// `IpAddr` + port (`format!("rtmp://{ip}:{port}/{app}")`) yields `rtmp://::1:1935/live`.
    #[test]
    fn ipv6_host_is_bracketed_in_tc_url() {
        let t = RtmpTarget::parse("rtmp://[::1]:1935/live/key").unwrap();
        assert_eq!(t.host, url::Host::<String>::Ipv6("::1".parse().unwrap()));
        assert_eq!(t.tc_url, "rtmp://[::1]:1935/live");
        let t = RtmpTarget::parse("rtmp://[2001:db8::7]/live/key").unwrap();
        assert_eq!(t.tc_url, "rtmp://[2001:db8::7]/live");
        assert_eq!(t.port, 1935);
    }

    #[test]
    fn ipv4_literal_is_a_domain_host_for_this_non_special_scheme() {
        let t = RtmpTarget::parse("rtmp://127.0.0.1:1936/live/k").unwrap();
        assert_eq!(t.host, url::Host::<String>::Domain("127.0.0.1".into()));
        assert_eq!(t.tc_url, "rtmp://127.0.0.1:1936/live");
    }

    #[test]
    fn userinfo_query_and_fragment_never_reach_tc_url() {
        let t = RtmpTarget::parse("rtmp://user:s3cret@host:1936/live/key?token=a/b#frag").unwrap();
        assert_eq!(t.tc_url, "rtmp://host:1936/live");
        assert_eq!(t.stream_key, "key?token=a/b");
        assert!(!t.tc_url.contains("s3cret"));
    }

    #[test]
    fn stream_key_keeps_extra_path_segments_and_trailing_slash_is_tolerated() {
        assert_eq!(
            RtmpTarget::parse("rtmp://h/live/a/b/c").unwrap().stream_key,
            "a/b/c"
        );
        assert_eq!(
            RtmpTarget::parse("rtmp://h/live/key/").unwrap().stream_key,
            "key/"
        );
    }

    #[test]
    fn missing_pieces_and_wrong_scheme_are_errors() {
        assert!(matches!(
            RtmpTarget::parse("rtmp://h/"),
            Err(RtmpUrlError::NoApp)
        ));
        assert!(matches!(
            RtmpTarget::parse("rtmp://h/live"),
            Err(RtmpUrlError::NoStreamKey)
        ));
        assert!(matches!(
            RtmpTarget::parse("http://h/live/k"),
            Err(RtmpUrlError::Scheme(_))
        ));
        assert!(matches!(
            RtmpTarget::parse("not a url"),
            Err(RtmpUrlError::Parse(_))
        ));
    }

    #[test]
    fn from_parts_takes_host_and_port_from_base_only() {
        let t = RtmpTarget::from_parts("rtmp://[::1]:1940/ignored/path", "live", "k").unwrap();
        assert_eq!(t.tc_url, "rtmp://[::1]:1940/live");
        assert_eq!(
            (t.app.as_str(), t.stream_key.as_str(), t.port),
            ("live", "k", 1940)
        );
    }

    #[cfg(feature = "tokio")]
    #[tokio::test]
    async fn resolve_uses_the_literal_address_without_dns() {
        let t = RtmpTarget::parse("rtmp://[::1]:1935/live/key").unwrap();
        assert_eq!(
            t.resolve().await.unwrap(),
            vec!["[::1]:1935".parse::<SocketAddr>().unwrap()]
        );
        let t = RtmpTarget::parse("rtmp://127.0.0.1:1/live/key").unwrap();
        assert_eq!(
            t.resolve().await.unwrap(),
            vec!["127.0.0.1:1".parse::<SocketAddr>().unwrap()]
        );
    }

    /// `rtmps` is a real scheme (RTMP over TLS, default port 443).
    #[test]
    fn rtmps_is_accepted_with_the_tls_default_port() {
        let t = RtmpTarget::parse("rtmps://h.example/live/key").unwrap();
        assert!(t.tls);
        assert_eq!(t.port, RTMPS_DEFAULT_PORT);
        assert_eq!(t.tc_url, "rtmps://h.example/live");
        let t = RtmpTarget::parse("rtmps://h.example:8443/live/key").unwrap();
        assert_eq!(t.port, 8443);
        assert!(!RtmpTarget::parse("rtmp://h.example/live/key").unwrap().tls);
    }

    /// RFC 3986 §2.1: percent-encoded path octets are decoded; the query stays
    /// as written. A key that is only a query is no key.
    #[test]
    fn path_text_is_percent_decoded_and_the_query_is_kept_raw() {
        let t = RtmpTarget::parse("rtmp://h/li%20ve/a/b%20c?token=a%26b").unwrap();
        assert_eq!(t.app, "li ve");
        assert_eq!(t.stream_key, "a/b c?token=a%26b");
        assert!(matches!(
            RtmpTarget::parse("rtmp://h/live/?token=x"),
            Err(RtmpUrlError::NoStreamKey)
        ));
        assert!(matches!(
            RtmpTarget::parse("rtmp://h/live/%FF"),
            Err(RtmpUrlError::PathNotUtf8)
        ));
    }

    #[cfg(feature = "tokio")]
    #[tokio::test]
    async fn the_plain_tcp_adapter_refuses_a_tls_target() {
        let t = RtmpTarget::parse("rtmps://127.0.0.1:1/live/key").unwrap();
        let err = crate::io::AsyncRtmpClient::connect(
            &t,
            broadcast_common::timeouts::Timeouts::default(),
        )
        .await
        .err()
        .expect("a refusal");
        assert_eq!(err.kind(), std::io::ErrorKind::Unsupported);
    }
}
