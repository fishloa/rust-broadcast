//! `rtmp://host[:port]/app/stream-key` -> typed target, built with the `url` crate.
//! Replaces `format!("rtmp://{host}:{port}/{app}")`, which produces `rtmp://::1:1935/live`
//! for an IPv6 address (defect 8) and leaks userinfo/query into `tcUrl`.

use std::net::{IpAddr, SocketAddr};

use url::{Host, Url};

/// IANA-assigned RTMP port, used when the URL names none.
pub const RTMP_DEFAULT_PORT: u16 = 1935;
const SCHEME: &str = "rtmp";

/// A parsed RTMP publish target.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct RtmpTarget {
    /// Host, never a pre-formatted string. `rtmp` is a non-special URL scheme, so
    /// the `url` crate reports an IPv4 literal as `Host::Domain("127.0.0.1")`
    /// (`resolve` handles it, `lookup_host` accepts literals); only a bracketed IPv6
    /// literal becomes `Host::Ipv6`.
    pub host: Host<String>,
    /// TCP port; 1935 when the URL has none.
    pub port: u16,
    /// The RTMP `app`.
    pub app: String,
    /// Remaining path plus `?query`, raw (not percent-decoded).
    pub stream_key: String,
    /// `rtmp://host[:port]/app`, IPv6 bracketed, userinfo/query/fragment removed.
    pub tc_url: String,
}

/// Why an RTMP URL was rejected.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum RtmpUrlError {
    /// The text is not a URL.
    #[error("invalid URL: {0}")]
    Parse(String),
    /// The scheme is not `rtmp`.
    #[error("unsupported scheme {0:?} (only rtmp)")]
    Scheme(String),
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
    /// Parses `rtmp://host[:port]/app/stream-key[?query]`.
    pub fn parse(url: &str) -> Result<Self, RtmpUrlError> {
        let base = parse_base(url)?;
        let mut segs = base.path_segments().ok_or(RtmpUrlError::NoApp)?;
        let app = segs
            .next()
            .filter(|s| !s.is_empty())
            .ok_or(RtmpUrlError::NoApp)?
            .to_string();
        let rest: Vec<&str> = segs.collect();
        let mut key = rest.join("/");
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
        let mut tc = base.clone();
        tc.set_path(&format!("/{app}"));
        tc.set_query(None);
        tc.set_fragment(None);
        let _ = tc.set_username("");
        let _ = tc.set_password(None);
        Ok(RtmpTarget {
            host,
            port: base.port().unwrap_or(RTMP_DEFAULT_PORT),
            app: app.to_string(),
            stream_key: stream_key.to_string(),
            tc_url: tc.to_string(),
        })
    }

    /// Resolved socket addresses (DNS only for `Host::Domain`).
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
    if u.scheme() != SCHEME {
        return Err(RtmpUrlError::Scheme(u.scheme().to_string()));
    }
    Ok(u)
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
}
