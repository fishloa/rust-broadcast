//! Minimal relative-URI resolution + query-string helpers.
//!
//! This is deliberately *not* a general-purpose URL library (RFC 3986) — the
//! sans-IO core has zero dependencies, and playlist/segment URIs in practice
//! are one of exactly three shapes: absolute (`scheme://...`), absolute-path
//! (`/...`, same scheme+host), or relative (`seg0.m4s`, relative to the
//! playlist's own URL directory). This resolver only handles those three; a
//! caller whose origin serves anything stranger (`..`-segments, `;params`)
//! should resolve URIs itself before handing bytes to
//! [`crate::client::LlHlsClient::on_resource`].

use alloc::format;
use alloc::string::{String, ToString};

/// Resolve `uri` (as it appears in a playlist) against `base` (the URL the
/// playlist itself was fetched from).
pub(crate) fn resolve(base: &str, uri: &str) -> String {
    if uri.contains("://") {
        return uri.to_string();
    }
    if let Some(rest) = uri.strip_prefix("//") {
        // Protocol-relative: borrow base's scheme.
        return match base.find("://") {
            Some(idx) => format!("{}://{rest}", &base[..idx]),
            None => uri.to_string(),
        };
    }
    if let Some(path) = uri.strip_prefix('/') {
        // Absolute path: keep base's scheme+authority, replace the path.
        return match base.find("://") {
            Some(scheme_end) => {
                let authority_start = scheme_end + 3;
                let authority_end = base[authority_start..]
                    .find('/')
                    .map(|i| authority_start + i)
                    .unwrap_or(base.len());
                format!("{}/{path}", &base[..authority_end])
            }
            None => uri.to_string(),
        };
    }
    // Relative: replace everything after the last `/` in base's path
    // (query/fragment stripped first).
    let base_no_query = base.split(['?', '#']).next().unwrap_or(base);
    match base_no_query.rfind('/') {
        Some(idx) => format!("{}{uri}", &base_no_query[..=idx]),
        None => uri.to_string(),
    }
}

/// Append a raw (already-encoded) `key=value` pair to `url`'s query string.
pub(crate) fn append_query(url: &str, extra: &str) -> String {
    if url.contains('?') {
        format!("{url}&{extra}")
    } else {
        format!("{url}?{extra}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolves_relative_against_directory() {
        assert_eq!(
            resolve("http://h/live/stream.m3u8", "seg0.m4s"),
            "http://h/live/seg0.m4s"
        );
    }

    #[test]
    fn resolves_relative_ignoring_query() {
        assert_eq!(
            resolve("http://h/live/stream.m3u8?_HLS_msn=3", "seg0.m4s"),
            "http://h/live/seg0.m4s"
        );
    }

    #[test]
    fn absolute_uri_passes_through() {
        assert_eq!(
            resolve("http://h/live/stream.m3u8", "https://cdn/seg0.m4s"),
            "https://cdn/seg0.m4s"
        );
    }

    #[test]
    fn protocol_relative_borrows_base_scheme() {
        assert_eq!(
            resolve("https://h/live/stream.m3u8", "//cdn/seg0.m4s"),
            "https://cdn/seg0.m4s"
        );
    }

    #[test]
    fn absolute_path_keeps_authority() {
        assert_eq!(
            resolve("http://h/live/stream.m3u8", "/segs/seg0.m4s"),
            "http://h/segs/seg0.m4s"
        );
    }

    #[test]
    fn append_query_adds_question_mark_once() {
        assert_eq!(
            append_query("http://h/p.m3u8", "a=1"),
            "http://h/p.m3u8?a=1"
        );
        assert_eq!(
            append_query("http://h/p.m3u8?a=1", "b=2"),
            "http://h/p.m3u8?a=1&b=2"
        );
    }
}
