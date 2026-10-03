//! URI resolution and query building for the HLS client core, on the `url`
//! crate (RFC 3986 §5 reference resolution) — `no_std` + `alloc`.
//!
//! A playlist URL that is itself relative (a caller that hands the client a
//! path rather than an absolute URL) is resolved against a synthetic base
//! (`hls-relative:///`) which is stripped from the result again, so a relative
//! base yields a relative result.

use alloc::string::{String, ToString};

use url::Url;

/// Base used when the playlist URL is itself relative; stripped from results.
const SYNTHETIC_BASE: &str = "hls-relative:///";
/// The synthetic base without its path slash.
const SYNTHETIC_ROOT: &str = "hls-relative://";

/// The synthetic-base form of a relative URL (or reference).
fn synthetic(relative: &str) -> Option<Url> {
    Url::parse(SYNTHETIC_BASE).ok()?.join(relative).ok()
}

/// Resolve `uri` (as it appears in a playlist) against `base` (the URL the
/// playlist itself was fetched from), per RFC 3986 §5.
///
/// An unresolvable pair returns `uri` unchanged.
pub(crate) fn resolve(base: &str, uri: &str) -> String {
    let (base_url, is_synthetic) = match Url::parse(base) {
        Ok(u) => (u, false),
        Err(_) => match synthetic(base) {
            Some(u) => (u, true),
            None => return uri.to_string(),
        },
    };
    match base_url.join(uri) {
        Ok(joined) if is_synthetic => strip_synthetic(
            joined.as_str(),
            base.starts_with('/') || uri.starts_with('/'),
        ),
        Ok(joined) => joined.to_string(),
        Err(_) => uri.to_string(),
    }
}

/// Remove the synthetic base from `s`. `rooted` says whether the caller's own
/// input had a leading `/` (and so the result keeps it).
fn strip_synthetic(s: &str, rooted: bool) -> String {
    match s.strip_prefix(SYNTHETIC_ROOT) {
        // A path result: `/a/b` (rooted) or `a/b`.
        Some(rest) if rest.starts_with('/') => {
            if rooted {
                rest.to_string()
            } else {
                rest[1..].to_string()
            }
        }
        // An authority survived (a `//cdn/x` reference): keep it protocol-relative.
        Some(rest) => alloc::format!("//{rest}"),
        // An absolute URL with a real scheme: nothing synthetic to remove.
        None => s.to_string(),
    }
}

/// Append one `key=value` pair (form-urlencoded) to `url`'s query; the
/// existing query is kept verbatim and a fragment stays last.
pub(crate) fn append_pair(url: &str, key: &str, value: &str) -> String {
    let (mut u, is_synthetic) = match Url::parse(url) {
        Ok(u) => (u, false),
        Err(_) => match synthetic(url) {
            Some(u) => (u, true),
            None => {
                let sep = if url.contains('?') { '&' } else { '?' };
                return alloc::format!("{url}{sep}{key}={value}");
            }
        },
    };
    u.query_pairs_mut().append_pair(key, value);
    if is_synthetic {
        strip_synthetic(u.as_str(), url.starts_with('/'))
    } else {
        u.to_string()
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
    fn dot_dot_segments_are_resolved_defect_7() {
        assert_eq!(
            resolve("http://h/a/b/p.m3u8", "../x.m4s"),
            "http://h/a/x.m4s"
        );
        assert_eq!(
            resolve("http://h/a/b/p.m3u8", "../../x.m4s"),
            "http://h/x.m4s"
        );
        assert_eq!(
            resolve("http://h/a/b/p.m3u8", "./c/../d.m4s"),
            "http://h/a/b/d.m4s"
        );
        assert_eq!(
            resolve("http://h/a/p.m3u8", "../../../x.m4s"),
            "http://h/x.m4s",
            "cannot climb above the root"
        );
        assert_eq!(
            resolve("http://h/a/p.m3u8", "/z/../y.m4s"),
            "http://h/y.m4s",
            "absolute-path references are normalised too"
        );
    }

    /// The old `uri.contains("://")` shortcut returned a relative reference unchanged
    /// when its query happened to contain a URL.
    #[test]
    fn a_scheme_separator_inside_the_query_does_not_make_a_reference_absolute() {
        assert_eq!(
            resolve("http://h/a/p.m3u8", "seg.m4s?redirect=http://x/y"),
            "http://h/a/seg.m4s?redirect=http://x/y"
        );
    }

    #[test]
    fn query_only_and_fragment_references_and_a_base_with_query_and_fragment() {
        assert_eq!(
            resolve("http://h/a/p.m3u8?x=1#top", "?y=2"),
            "http://h/a/p.m3u8?y=2"
        );
        assert_eq!(
            resolve("http://h/a/p.m3u8?x=1#top", "seg.m4s"),
            "http://h/a/seg.m4s"
        );
        assert_eq!(
            resolve("http://h:8080/a/p.m3u8", "seg.m4s"),
            "http://h:8080/a/seg.m4s"
        );
    }

    #[test]
    fn normalisation_is_applied_and_listed() {
        assert_eq!(
            resolve("http://h/a/p.m3u8", "HTTP://H.Example/X"),
            "http://h.example/X"
        );
        assert_eq!(
            resolve("http://h/a/p.m3u8", "my seg.m4s"),
            "http://h/a/my%20seg.m4s"
        );
        assert_eq!(
            resolve("http://h:80/a/p.m3u8", "s.m4s"),
            "http://h/a/s.m4s",
            "default port dropped"
        );
    }

    #[test]
    fn a_relative_base_resolves_against_a_synthetic_base_that_is_stripped() {
        assert_eq!(resolve("live/stream.m3u8", "seg.m4s"), "live/seg.m4s");
        assert_eq!(resolve("live/stream.m3u8", "../x.m4s"), "x.m4s");
        assert_eq!(resolve("/live/stream.m3u8", "seg.m4s"), "/live/seg.m4s");
        assert_eq!(resolve("live/stream.m3u8", "/abs/x.m4s"), "/abs/x.m4s");
        assert_eq!(
            resolve("live/stream.m3u8", "http://cdn/x.m4s"),
            "http://cdn/x.m4s"
        );
        assert_eq!(
            resolve("live/stream.m3u8", "//cdn/x.m4s"),
            "//cdn/x.m4s",
            "a protocol-relative reference keeps its authority"
        );
    }

    #[test]
    fn append_pair_adds_one_encoded_pair_and_keeps_the_existing_query_verbatim() {
        assert_eq!(
            append_pair("http://h/p.m3u8", "a", "1"),
            "http://h/p.m3u8?a=1"
        );
        assert_eq!(
            append_pair("http://h/p.m3u8?a=1", "b", "2"),
            "http://h/p.m3u8?a=1&b=2"
        );
        assert_eq!(
            append_pair("http://h/p.m3u8?a=x%20y&z", "b", "2"),
            "http://h/p.m3u8?a=x%20y&z&b=2"
        );
        assert_eq!(
            append_pair("http://h/p.m3u8", "k", "a b&c"),
            "http://h/p.m3u8?k=a+b%26c"
        );
        assert_eq!(
            append_pair("p.m3u8", "a", "1"),
            "p.m3u8?a=1",
            "relative playlist URL keeps working"
        );
    }
}
