//! URI resolution for the HLS client core, on the `url` crate (RFC 3986 §5
//! reference resolution) — `no_std` + `alloc`.
//!
//! The client's playlist base is an absolute [`Url`] (the config takes one),
//! so a segment reference always resolves against a real base and an
//! unresolvable reference is passed through unchanged rather than renamed.

use alloc::string::{String, ToString};

use url::Url;

use broadcast_common::url_ref;

/// Resolve `uri` (as it appears in a playlist) against `base` (the URL the
/// playlist itself was fetched from), per RFC 3986 §5.
///
/// An unresolvable pair returns `uri` unchanged.
pub(crate) fn resolve(base: &Url, uri: &str) -> String {
    match url_ref::resolve(base, uri) {
        Ok(joined) => joined.to_string(),
        Err(_) => uri.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn base(s: &str) -> Url {
        Url::parse(s).expect("base")
    }

    #[test]
    fn resolves_relative_against_directory() {
        assert_eq!(
            resolve(&base("http://h/live/stream.m3u8"), "seg0.m4s"),
            "http://h/live/seg0.m4s"
        );
    }

    #[test]
    fn resolves_relative_ignoring_query() {
        assert_eq!(
            resolve(&base("http://h/live/stream.m3u8?_HLS_msn=3"), "seg0.m4s"),
            "http://h/live/seg0.m4s"
        );
    }

    #[test]
    fn absolute_uri_passes_through() {
        assert_eq!(
            resolve(&base("http://h/live/stream.m3u8"), "https://cdn/seg0.m4s"),
            "https://cdn/seg0.m4s"
        );
    }

    #[test]
    fn protocol_relative_borrows_base_scheme() {
        assert_eq!(
            resolve(&base("https://h/live/stream.m3u8"), "//cdn/seg0.m4s"),
            "https://cdn/seg0.m4s"
        );
    }

    #[test]
    fn absolute_path_keeps_authority() {
        assert_eq!(
            resolve(&base("http://h/live/stream.m3u8"), "/segs/seg0.m4s"),
            "http://h/segs/seg0.m4s"
        );
    }

    #[test]
    fn dot_dot_segments_are_resolved_defect_7() {
        assert_eq!(
            resolve(&base("http://h/a/b/p.m3u8"), "../x.m4s"),
            "http://h/a/x.m4s"
        );
        assert_eq!(
            resolve(&base("http://h/a/b/p.m3u8"), "../../x.m4s"),
            "http://h/x.m4s"
        );
        assert_eq!(
            resolve(&base("http://h/a/b/p.m3u8"), "./c/../d.m4s"),
            "http://h/a/b/d.m4s"
        );
        assert_eq!(
            resolve(&base("http://h/a/p.m3u8"), "../../../x.m4s"),
            "http://h/x.m4s",
            "cannot climb above the root"
        );
        assert_eq!(
            resolve(&base("http://h/a/p.m3u8"), "/z/../y.m4s"),
            "http://h/y.m4s",
            "absolute-path references are normalised too"
        );
    }

    /// The old `uri.contains("://")` shortcut returned a relative reference unchanged
    /// when its query happened to contain a URL.
    #[test]
    fn a_scheme_separator_inside_the_query_does_not_make_a_reference_absolute() {
        assert_eq!(
            resolve(&base("http://h/a/p.m3u8"), "seg.m4s?redirect=http://x/y"),
            "http://h/a/seg.m4s?redirect=http://x/y"
        );
    }

    #[test]
    fn query_only_and_fragment_references_and_a_base_with_query_and_fragment() {
        assert_eq!(
            resolve(&base("http://h/a/p.m3u8?x=1#top"), "?y=2"),
            "http://h/a/p.m3u8?y=2"
        );
        assert_eq!(
            resolve(&base("http://h/a/p.m3u8?x=1#top"), "seg.m4s"),
            "http://h/a/seg.m4s"
        );
        assert_eq!(
            resolve(&base("http://h:8080/a/p.m3u8"), "seg.m4s"),
            "http://h:8080/a/seg.m4s"
        );
    }

    #[test]
    fn normalisation_is_applied_and_listed() {
        assert_eq!(
            resolve(&base("http://h/a/p.m3u8"), "HTTP://H.Example/X"),
            "http://h.example/X"
        );
        assert_eq!(
            resolve(&base("http://h/a/p.m3u8"), "my_seg.m4s"),
            "http://h/a/my_seg.m4s"
        );
        assert_eq!(
            resolve(&base("http://h/a/p.m3u8"), "my seg.m4s"),
            "my seg.m4s",
            "a reference with a forbidden character is passed through, never percent-encoded into a different URL"
        );
        assert_eq!(
            resolve(&base("http://h:80/a/p.m3u8"), "s.m4s"),
            "http://h/a/s.m4s",
            "default port dropped"
        );
    }
}
