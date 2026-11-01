//! RFC 3986 §5.4 reference-resolution vectors and BaseURL-chain behaviour for
//! `transmux::base_url` (SP3). The two tables moved here verbatim from the
//! deleted `src/uri.rs`; the `url` crate implements the WHATWG URL algorithm,
//! which differs from strict RFC 3986 in exactly the rows listed in
//! `WHATWG_DIFFERENCES`.
#![cfg(feature = "std")]

use transmux::base_url::{first_forbidden_char, resolve, resolve_chain};
use url::Url;

const BASE: &str = "http://a/b/c/d;p?q";

const RFC3986_NORMAL_EXAMPLES: &[(&str, &str)] = &[
    ("g:h", "g:h"),
    ("g", "http://a/b/c/g"),
    ("./g", "http://a/b/c/g"),
    ("g/", "http://a/b/c/g/"),
    ("/g", "http://a/g"),
    ("//g", "http://g"),
    ("?y", "http://a/b/c/d;p?y"),
    ("g?y", "http://a/b/c/g?y"),
    ("#s", "http://a/b/c/d;p?q#s"),
    ("g#s", "http://a/b/c/g#s"),
    ("g?y#s", "http://a/b/c/g?y#s"),
    (";x", "http://a/b/c/;x"),
    ("g;x", "http://a/b/c/g;x"),
    ("g;x?y#s", "http://a/b/c/g;x?y#s"),
    ("", "http://a/b/c/d;p?q"),
    (".", "http://a/b/c/"),
    ("./", "http://a/b/c/"),
    ("..", "http://a/b/"),
    ("../", "http://a/b/"),
    ("../g", "http://a/b/g"),
    ("../..", "http://a/"),
    ("../../", "http://a/"),
    ("../../g", "http://a/g"),
];

const RFC3986_ABNORMAL_EXAMPLES: &[(&str, &str)] = &[
    ("../../../g", "http://a/g"),
    ("../../../../g", "http://a/g"),
    ("/./g", "http://a/g"),
    ("/../g", "http://a/g"),
    ("g.", "http://a/b/c/g."),
    (".g", "http://a/b/c/.g"),
    ("g..", "http://a/b/c/g.."),
    ("..g", "http://a/b/c/..g"),
    ("./../g", "http://a/b/g"),
    ("./g/.", "http://a/b/c/g/"),
    ("g/./h", "http://a/b/c/g/h"),
    ("g/../h", "http://a/b/c/h"),
    ("g;x=1/./y", "http://a/b/c/g;x=1/y"),
    ("g;x=1/../y", "http://a/b/c/y"),
    ("g?y/./x", "http://a/b/c/g?y/./x"),
    ("g?y/../x", "http://a/b/c/g?y/../x"),
    ("g#s/./x", "http://a/b/c/g#s/./x"),
    ("g#s/../x", "http://a/b/c/g#s/../x"),
];

/// Rows where WHATWG (the `url` crate) legitimately differs from RFC 3986:
/// `//g` serialises with a root path; the same-scheme reference `http:g` is
/// treated as relative for special schemes (§5.4.2's own "strict parser" note).
const WHATWG_DIFFERENCES: [(&str, &str); 2] = [("//g", "http://g/"), ("http:g", "http://a/b/c/g")];

fn run(rows: &[(&str, &str)]) {
    let base = Url::parse(BASE).unwrap();
    for (reference, rfc) in rows {
        let expected = WHATWG_DIFFERENCES
            .iter()
            .find(|(r, _)| r == reference)
            .map_or(*rfc, |(_, w)| *w);
        assert_eq!(
            resolve(Some(&base), reference).as_deref(),
            Some(expected),
            "{reference:?}"
        );
    }
}

#[test]
fn rfc3986_5_4_1_normal_examples() {
    assert_eq!(RFC3986_NORMAL_EXAMPLES.len(), 23);
    run(RFC3986_NORMAL_EXAMPLES);
}

#[test]
fn rfc3986_5_4_2_abnormal_examples() {
    assert_eq!(RFC3986_ABNORMAL_EXAMPLES.len(), 18);
    run(RFC3986_ABNORMAL_EXAMPLES);
    let base = Url::parse(BASE).unwrap();
    assert_eq!(
        resolve(Some(&base), "http:g").as_deref(),
        Some("http://a/b/c/g")
    );
}

#[test]
fn base_chain_each_level_resolves_against_the_one_before() {
    let chain = vec![
        "https://cdn.example.com/vod/".to_string(),
        "period-1/".to_string(),
        "video/".to_string(),
    ];
    assert_eq!(
        resolve_chain(None, &chain, "seg-1.m4s").as_deref(),
        Some("https://cdn.example.com/vod/period-1/video/seg-1.m4s")
    );
    assert_eq!(
        resolve_chain(None, &chain, "../audio/seg-1.m4s").as_deref(),
        Some("https://cdn.example.com/vod/period-1/audio/seg-1.m4s")
    );
    let reset = vec![
        "https://cdn.example.com/vod/".to_string(),
        "https://other.example.net/".to_string(),
    ];
    assert_eq!(
        resolve_chain(None, &reset, "seg.m4s").as_deref(),
        Some("https://other.example.net/seg.m4s")
    );
    assert_eq!(
        resolve_chain(None, &[], "https://x/y.m4s").as_deref(),
        Some("https://x/y.m4s")
    );
    // Empty / whitespace-only BaseURLs contribute nothing.
    let blanks = vec![String::new(), "  ".to_string(), "https://h/d/".to_string()];
    assert_eq!(
        resolve_chain(None, &blanks, "s.m4s").as_deref(),
        Some("https://h/d/s.m4s")
    );
}

#[test]
fn trailing_slash_is_significant() {
    let mpd = Url::parse("https://cdn.example.com/vod/index.mpd").unwrap();
    assert_eq!(
        resolve(Some(&mpd), "seg.m4s").as_deref(),
        Some("https://cdn.example.com/vod/seg.m4s")
    );
    let dir = Url::parse("https://cdn.example.com/vod/").unwrap();
    assert_eq!(
        resolve(Some(&dir), "seg.m4s").as_deref(),
        Some("https://cdn.example.com/vod/seg.m4s")
    );
    let file = Url::parse("https://cdn.example.com/vod").unwrap();
    assert_eq!(
        resolve(Some(&file), "seg.m4s").as_deref(),
        Some("https://cdn.example.com/seg.m4s")
    );
}

#[test]
fn the_mpd_url_or_a_file_url_is_the_base() {
    let mpd = Url::parse("https://cdn.example.com/vod/index.mpd").unwrap();
    let chain = vec!["period-1/".to_string()];
    assert_eq!(
        resolve_chain(Some(&mpd), &chain, "seg.m4s").as_deref(),
        Some("https://cdn.example.com/vod/period-1/seg.m4s")
    );
    let file = Url::from_file_path("/media/a/index.mpd").unwrap();
    assert_eq!(
        resolve(Some(&file), "seg.m4s").as_deref(),
        Some("file:///media/a/seg.m4s")
    );
}

// --- the synthetic base (option (a)) -----------------------------------------

#[test]
fn relative_results_stay_relative_when_there_is_no_base() {
    assert_eq!(resolve(None, "seg/1.m4s").as_deref(), Some("seg/1.m4s"));
    assert_eq!(
        resolve(None, "seg.m4s?x=1#f").as_deref(),
        Some("seg.m4s?x=1#f")
    );
    assert_eq!(
        resolve_chain(None, &["video/".to_string()], "seg-1.m4s").as_deref(),
        Some("video/seg-1.m4s")
    );
    assert_eq!(
        resolve_chain(None, &["a/".to_string(), "../b/".to_string()], "s.m4s").as_deref(),
        Some("b/s.m4s")
    );
}

#[test]
fn an_absolute_path_reference_is_not_mistaken_for_a_relative_one() {
    assert_eq!(resolve(None, "/abs/x.m4s").as_deref(), Some("/abs/x.m4s"));
    assert_eq!(
        resolve(None, "//cdn.example/x.m4s").as_deref(),
        Some("//cdn.example/x.m4s")
    );
    assert_eq!(resolve(None, "http://x/y").as_deref(), Some("http://x/y"));
}

/// Containment: with no base, a relative input never resolves to an absolute
/// path (RFC 3986 §5.2.4 drops a `..` that would climb above the root). A
/// consumer doing `local_dir.join(resolved)` must not see `/etc/passwd`.
#[test]
fn climbing_above_the_root_stays_relative_and_contained() {
    assert_eq!(resolve(None, "../x").as_deref(), Some("x"));
    assert_eq!(
        resolve(None, "../../../etc/passwd").as_deref(),
        Some("etc/passwd")
    );
    assert_eq!(
        resolve(None, "a/../../../etc/passwd").as_deref(),
        Some("etc/passwd")
    );
    assert_eq!(resolve(None, "..").as_deref(), Some(""));
    let chain = vec!["a/".to_string(), "../../../b/".to_string()];
    assert_eq!(
        resolve_chain(None, &chain, "../../s.m4s").as_deref(),
        Some("s.m4s")
    );
}

/// The url crate treats percent-encoded dot segments as dots; they behave
/// exactly like `..` / `.` and cannot be used to escape either.
#[test]
fn encoded_dot_segments_behave_like_plain_ones() {
    assert_eq!(resolve(None, "%2e%2e/x").as_deref(), Some("x"));
    assert_eq!(resolve(None, "%2E%2E/%2e%2e/x").as_deref(), Some("x"));
    assert_eq!(resolve(None, ".%2e/x").as_deref(), Some("x"));
    assert_eq!(resolve(None, "a/%2e/b").as_deref(), Some("a/b"));
    assert_eq!(resolve(None, "%2e%2e/x"), resolve(None, "../x"));
}

/// An absolute-path reference the MPD itself wrote stays absolute (and is
/// still clamped at the root); a relative one never becomes absolute after it.
#[test]
fn absolute_path_references_are_absolute_only_when_written_so() {
    assert_eq!(resolve(None, "/abs/x.m4s").as_deref(), Some("/abs/x.m4s"));
    assert_eq!(resolve(None, "/../../x").as_deref(), Some("/x"));
    let chain = vec!["/a/".to_string()];
    assert_eq!(
        resolve_chain(None, &chain, "../../s.m4s").as_deref(),
        Some("/s.m4s")
    );
    assert_eq!(
        resolve_chain(None, &chain, "b/s.m4s").as_deref(),
        Some("/a/b/s.m4s")
    );
    // A later relative entry does not undo an earlier absolute-path one.
    let chain = vec!["/a/".to_string(), "b/".to_string()];
    assert_eq!(
        resolve_chain(None, &chain, "s.m4s").as_deref(),
        Some("/a/b/s.m4s")
    );
}

/// The synthetic scheme is an implementation detail: an input naming it is
/// refused instead of being "stripped" into a relative result.
#[test]
fn inputs_naming_the_synthetic_scheme_are_rejected() {
    for evil in [
        "transmux-relative:///transmux-relative-root/../../evil/a.m4s",
        "transmux-relative:///evil/a.m4s",
        "TRANSMUX-RELATIVE:///x",
        "transmux-relative://host/x",
    ] {
        assert_eq!(resolve(None, evil), None, "{evil}");
        assert_eq!(
            resolve_chain(None, &[evil.to_string()], "a.m4s"),
            None,
            "{evil}"
        );
    }
    // A name that merely starts with the same letters is an ordinary relative path.
    assert_eq!(
        resolve(None, "transmux-relative-root/a.m4s").as_deref(),
        Some("transmux-relative-root/a.m4s")
    );
    assert_eq!(
        resolve(None, "transmux-relative/a").as_deref(),
        Some("transmux-relative/a")
    );
}

/// Scheme-relative `//host` keeps its authority and is not made relative.
#[test]
fn scheme_relative_references_keep_their_host() {
    assert_eq!(
        resolve(None, "//cdn.example/x.m4s").as_deref(),
        Some("//cdn.example/x.m4s")
    );
    assert_eq!(
        resolve(None, "//cdn.example/../../x").as_deref(),
        Some("//cdn.example/x")
    );
    let chain = vec!["//cdn.example/dir/".to_string()];
    assert_eq!(
        resolve_chain(None, &chain, "s.m4s").as_deref(),
        Some("//cdn.example/dir/s.m4s")
    );
    assert_eq!(
        resolve_chain(None, &chain, "../../../s.m4s").as_deref(),
        Some("//cdn.example/s.m4s")
    );
}

/// Query and fragment survive, alone or after a path, with and without `..`.
#[test]
fn query_and_fragment_are_preserved_on_relative_results() {
    assert_eq!(resolve(None, "?y").as_deref(), Some("?y"));
    assert_eq!(resolve(None, "#f").as_deref(), Some("#f"));
    assert_eq!(resolve(None, "").as_deref(), Some(""));
    assert_eq!(
        resolve(None, "../x.m4s?a=1&b=../2#frag").as_deref(),
        Some("x.m4s?a=1&b=../2#frag")
    );
    assert_eq!(
        resolve(None, "/x.m4s?a=1#f").as_deref(),
        Some("/x.m4s?a=1#f")
    );
    let chain = vec!["dir/index.mpd?token=abc".to_string()];
    assert_eq!(
        resolve_chain(None, &chain, "s.m4s?n=1").as_deref(),
        Some("dir/s.m4s?n=1")
    );
}

#[test]
fn non_ascii_references_are_percent_encoded_by_the_url_crate() {
    assert_eq!(resolve(None, "é.m4s").as_deref(), Some("%C3%A9.m4s"));
    let base = Url::parse("http://a/b/").unwrap();
    assert_eq!(
        resolve(Some(&base), "é.m4s").as_deref(),
        Some("http://a/b/%C3%A9.m4s")
    );
}

// --- hostile input -------------------------------------------------------------

/// WHY the guard must run before joining: the WHATWG parser deletes tab, CR and
/// LF instead of rejecting them.
#[test]
fn the_url_crate_silently_strips_crlf_so_the_guard_cannot_be_delegated() {
    assert_eq!(
        Url::parse("http://a/b\r\nc").unwrap().as_str(),
        "http://a/bc"
    );
}

#[test]
fn control_characters_and_whitespace_are_rejected_before_parsing() {
    let base = Url::parse("http://a/b/").unwrap();
    for evil in [
        "seg\r\nHost: evil.example",
        "seg\nX: y",
        "seg\rX: y",
        "seg\tX: y",
        "seg X: y",
        "\u{0}",
        "seg\u{7f}",
        "seg\u{1b}[31m",
    ] {
        assert!(first_forbidden_char(evil).is_some(), "{evil:?}");
        assert_eq!(resolve(Some(&base), evil), None, "{evil:?}");
        assert_eq!(resolve(None, evil), None, "{evil:?}");
    }
    assert_eq!(
        resolve_chain(None, &["http://a/\nb/".to_string()], "s"),
        None
    );
    assert_eq!(
        resolve_chain(None, &["x/".to_string()], "s\r\nHost: e"),
        None
    );
    assert_eq!(
        resolve(Some(&base), "seg.m4s").as_deref(),
        Some("http://a/b/seg.m4s")
    );
    // A percent-encoded control character is not raw text and passes through.
    assert_eq!(
        resolve(Some(&base), "seg%0D%0AHost:evil").as_deref(),
        Some("http://a/b/seg%0D%0AHost:evil")
    );
}

#[test]
fn unparseable_references_are_none_not_a_panic() {
    let base = Url::parse("http://a/b/").unwrap();
    for bad in [
        "http://[::1",
        "http://",
        "http://a:99999999/",
        "http://exa mple/",
    ] {
        assert_eq!(resolve(Some(&base), bad), None, "{bad:?}");
    }
    // Backslash is a path separator for special schemes (WHATWG): pinned.
    assert_eq!(
        resolve(Some(&base), "x\\y.m4s").as_deref(),
        Some("http://a/b/x/y.m4s")
    );
}

#[test]
fn crate_root_re_exports_exist() {
    assert_eq!(
        transmux::resolve_url_reference(Some(&Url::parse("http://a/b/c/d;p?q").unwrap()), "g")
            .as_deref(),
        Some("http://a/b/c/g")
    );
    assert_eq!(
        transmux::resolve_base_url_chain(None, &["http://a/".to_string()], "s.m4s").as_deref(),
        Some("http://a/s.m4s")
    );
}
