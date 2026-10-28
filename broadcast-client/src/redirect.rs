//! Redirect handling.

use url::Url;

const SCHEME_HTTP: &str = "http";
const SCHEME_HTTPS: &str = "https";
const SCHEME_RTSP: &str = "rtsp";
const SCHEME_RTSPS: &str = "rtsps";

/// Resolve a redirect target. Only `http` and `https` targets are followed, and an `https`
/// request is never redirected to `http`.
pub fn resolve(req_url: &Url, headers: &[(String, String)], taken: u32, max: u32) -> Option<Url> {
    if taken >= max {
        return None;
    }
    let loc = headers
        .iter()
        .find(|(k, _)| k.eq_ignore_ascii_case("location"))
        .map(|(_, v)| v.clone())?;
    let target = req_url.join(&loc).ok()?;
    let allowed = match target.scheme() {
        SCHEME_HTTPS => true,
        SCHEME_HTTP => req_url.scheme() != SCHEME_HTTPS,
        _ => false,
    };
    allowed.then_some(target)
}

/// Resolve an RTSP redirect `Location`. Only `rtsp` and `rtsps` targets with a host are
/// followed, and an `rtsps` request is never redirected to `rtsp`.
pub fn resolve_rtsp(req_url: &Url, location: &str, taken: u32, max: u32) -> Option<Url> {
    if taken >= max {
        return None;
    }
    let target = req_url.join(location).ok()?;
    target.host()?;
    let allowed = match target.scheme() {
        SCHEME_RTSPS => true,
        SCHEME_RTSP => req_url.scheme() != SCHEME_RTSPS,
        _ => false,
    };
    allowed.then_some(target)
}

#[cfg(test)]
mod tests {
    use super::*;
    use url::Url;

    #[test]
    fn resolve_301_with_location() {
        let req = Url::parse("http://example/live.ts").unwrap();
        let headers = vec![("Location".to_string(), "/moved.ts".to_string())];
        let target = resolve(&req, &headers, 0, 5).unwrap();
        assert_eq!(target.as_str(), "http://example/moved.ts");
    }

    #[test]
    fn resolve_relative_location() {
        let req = Url::parse("http://example/a/live.ts").unwrap();
        let headers = vec![("Location".to_string(), "moved.ts".to_string())];
        let target = resolve(&req, &headers, 0, 5).unwrap();
        assert_eq!(target.as_str(), "http://example/a/moved.ts");
    }

    #[test]
    fn resolve_max_redirects_returns_none() {
        let req = Url::parse("http://example/live.ts").unwrap();
        let headers = vec![("Location".to_string(), "/moved.ts".to_string())];
        assert!(resolve(&req, &headers, 5, 5).is_none());
    }

    #[test]
    fn resolve_without_location_returns_none() {
        let req = Url::parse("http://example/live.ts").unwrap();
        let headers: Vec<(String, String)> = vec![];
        assert!(resolve(&req, &headers, 0, 5).is_none());
    }

    #[test]
    fn redirects_only_follow_http_and_https_and_never_downgrade() {
        let https = Url::parse("https://example/live.ts").unwrap();
        let http = Url::parse("http://example/live.ts").unwrap();
        let to = |target: &str| vec![("Location".to_string(), target.to_string())];
        let resolve = |from: &Url, target: &str| resolve(from, &to(target), 0, 5);
        for bad in [
            "file:///etc/passwd",
            "ftp://example/live.ts",
            "rtsp://example/live",
            "javascript:alert(1)",
        ] {
            assert!(resolve(&http, bad).is_none(), "{bad}");
        }
        assert!(resolve(&https, "http://example/live.ts").is_none());
        assert!(resolve(&http, "https://example/live.ts").is_some());
        assert!(resolve(&https, "https://other/live.ts").is_some());
        assert!(resolve(&http, "http://other/live.ts").is_some());
        assert!(resolve(&https, "/relative.ts").is_some());
    }
}
