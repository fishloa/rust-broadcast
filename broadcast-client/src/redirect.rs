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
