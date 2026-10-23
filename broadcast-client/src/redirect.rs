//! Redirect handling.

use url::Url;

/// Resolve a redirect target.
pub fn resolve(req_url: &Url, headers: &[(String, String)], taken: u32, max: u32) -> Option<Url> {
    if taken >= max {
        return None;
    }
    let loc = headers
        .iter()
        .find(|(k, _)| k.eq_ignore_ascii_case("location"))
        .map(|(_, v)| v.clone())?;
    req_url.join(&loc).ok()
}
