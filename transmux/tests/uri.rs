//! RFC 3986 §5 reference-resolution gate for [`transmux::uri`].
//!
//! The oracle is the standard's own §5.4 example tables, quoted verbatim — 23
//! "normal" rows and 18 "abnormal" ones against the base `http://a/b/c/d;p?q`.
//! They are transcribed as literal `(reference, expected)` pairs rather than
//! recomputed, so the test asserts the published values directly.

use transmux::uri::{
    RFC3986_ABNORMAL_EXAMPLES, RFC3986_NORMAL_EXAMPLES, RFC3986_NORMAL_EXAMPLES_BASE, UriReference,
    first_forbidden_char, merge, remove_dot_segments, resolve, resolve_segment, try_resolve,
    try_resolve_segment,
};

/// Every row of the RFC 3986 §5.4.1 "Normal Examples" table, verbatim.
#[test]
fn rfc3986_5_4_1_normal_examples() {
    let base = RFC3986_NORMAL_EXAMPLES_BASE;
    assert_eq!(base, "http://a/b/c/d;p?q");
    assert_eq!(RFC3986_NORMAL_EXAMPLES.len(), 23, "the table has 23 rows");

    for (reference, expected) in RFC3986_NORMAL_EXAMPLES {
        assert_eq!(
            &resolve(base, reference),
            expected,
            "§5.4.1: resolving {reference:?} against {base:?}"
        );
    }
}

/// Every row of the RFC 3986 §5.4.2 "Abnormal Examples" table, verbatim
/// (excluding the two `http:g` rows, which the RFC marks as parser-dependent:
/// a strict parser keeps `http:g`, which is the behaviour implemented).
#[test]
fn rfc3986_5_4_2_abnormal_examples() {
    let base = RFC3986_NORMAL_EXAMPLES_BASE;
    assert_eq!(RFC3986_ABNORMAL_EXAMPLES.len(), 18);
    for (reference, expected) in RFC3986_ABNORMAL_EXAMPLES {
        assert_eq!(
            &resolve(base, reference),
            expected,
            "§5.4.2: resolving {reference:?} against {base:?}"
        );
    }
    // A strict parser keeps a reference that restates the base's scheme.
    assert_eq!(resolve(base, "http:g"), "http:g");
}

/// §5.2.4's two worked `remove_dot_segments` examples, transcribed with the
/// intermediate buffer states the standard prints.
#[test]
fn rfc3986_5_2_4_worked_examples() {
    assert_eq!(remove_dot_segments("/a/b/c/./../../g"), "/a/g");
    assert_eq!(remove_dot_segments("mid/content=5/../6"), "mid/6");
}

/// §5.2.3's merge rules: an authority plus an empty base path yields `/` + the
/// reference; otherwise everything after the base path's last `/` is dropped.
#[test]
fn rfc3986_5_2_3_merge_paths() {
    let with_authority_empty_path = UriReference::parse("http://example.com");
    assert_eq!(
        merge(&with_authority_empty_path, "g"),
        "/g",
        "authority + empty base path"
    );

    let with_path = UriReference::parse("http://a/b/c/d;p?q");
    assert_eq!(
        merge(&with_path, "g"),
        "/b/c/g",
        "everything after the last /"
    );
}

/// §5.3's undefined-vs-empty distinction: `?y` replaces the query, `#s` keeps
/// it, and an empty reference reproduces the base exactly.
#[test]
fn component_undefined_versus_empty() {
    let base = "http://a/b/c/d;p?q";

    // `?y` has an (empty) path and a defined query -> the base's query is
    // replaced, not inherited.
    assert_eq!(resolve(base, "?y"), "http://a/b/c/d;p?y");
    // `#s` has an empty path and an undefined query -> the base's query is kept.
    assert_eq!(resolve(base, "#s"), "http://a/b/c/d;p?q#s");
    // An empty reference is a same-document reference: the base verbatim.
    assert_eq!(resolve(base, ""), base);

    // Parsing preserves the distinction.
    let parsed = UriReference::parse("http://a/b?y");
    assert_eq!(parsed.query.as_deref(), Some("y"));
    let parsed = UriReference::parse("http://a/b#y");
    assert_eq!(parsed.query, None, "no query delimiter was present");
    assert_eq!(parsed.fragment.as_deref(), Some("y"));
    let parsed = UriReference::parse("http://a/b?");
    assert_eq!(
        parsed.query.as_deref(),
        Some(""),
        "a present but empty query is Some(\"\")"
    );
}

/// Segment resolution through a `BaseURL` chain: each level resolves against
/// the one before it, and an absolute level resets the chain.
#[test]
fn resolve_segment_walks_the_base_chain() {
    let chain = vec![
        "https://cdn.example.com/vod/".to_string(),
        "period-1/".to_string(),
        "video/".to_string(),
    ];
    assert_eq!(
        resolve_segment(&chain, "seg-1.m4s"),
        "https://cdn.example.com/vod/period-1/video/seg-1.m4s"
    );
    assert_eq!(
        resolve_segment(&chain, "../audio/seg-1.m4s"),
        "https://cdn.example.com/vod/period-1/audio/seg-1.m4s"
    );
    // An absolute BaseURL at a deeper level discards everything above it.
    let chain = vec![
        "https://cdn.example.com/vod/".to_string(),
        "https://other.example.net/".to_string(),
    ];
    assert_eq!(
        resolve_segment(&chain, "seg.m4s"),
        "https://other.example.net/seg.m4s"
    );
    // No chain at all leaves the reference untouched.
    assert_eq!(resolve_segment(&[], "https://x/y.m4s"), "https://x/y.m4s");
}

/// A trailing slash is significant: `dir` replaces the last segment of the
/// base path, `dir/` extends it.
#[test]
fn trailing_slash_is_significant() {
    let base = "https://cdn.example.com/vod/index.mpd";
    assert_eq!(
        resolve(base, "seg.m4s"),
        "https://cdn.example.com/vod/seg.m4s"
    );
    assert_eq!(
        resolve(base, "dir/seg.m4s"),
        "https://cdn.example.com/vod/dir/seg.m4s"
    );
    assert_eq!(
        resolve("https://cdn.example.com/vod/", "seg.m4s"),
        "https://cdn.example.com/vod/seg.m4s"
    );
    assert_eq!(
        resolve("https://cdn.example.com/vod", "seg.m4s"),
        "https://cdn.example.com/seg.m4s",
        "without the trailing slash 'vod' is a file, not a directory"
    );
}

// ---------------------------------------------------------------------------
// review round 3: hostile / non-ASCII input must never panic
// ---------------------------------------------------------------------------

/// Every function in `uri` must be total over `&str`: no input a caller can
/// spell may panic, whatever bytes it contains.
///
/// `remove_dot_segments` sliced at byte 1 (`rest[1..]`) to skip a leading `/`,
/// which panics whenever the first character is multi-byte — reachable straight
/// from `resolve` with a non-ASCII relative reference (`"g:é"`) or base.
#[test]
fn non_ascii_input_never_panics() {
    // The exact reported repro, before any fuzzing.
    assert_eq!(remove_dot_segments("é"), "é");
    assert!(!remove_dot_segments("é").is_empty());
    let _ = resolve("http://a/b", "g:é");
    let _ = resolve("é", "é");

    // Multi-byte characters in every position that the algorithm slices:
    // leading, after a slash, inside a dot-segment, and as the whole input.
    for s in [
        "é",
        "é/",
        "/é",
        "/é/",
        "a/é",
        "é/a",
        "./é",
        "/./é",
        "/../é",
        "../é",
        "é.",
        ".é",
        "..é",
        "é..",
        "日本語/パス",
        "/日本語",
        "ÿ",
        "\u{10FFFF}",
        "👍/x",
        "x/👍",
    ] {
        let out = remove_dot_segments(s);
        assert!(
            out.len() <= s.len(),
            "remove_dot_segments must not grow its input ({s:?} -> {out:?})"
        );
        // And through the public API.
        let _ = resolve("http://a/b/c/d;p?q", s);
        let _ = resolve(s, "g");
        let _ = UriReference::parse(s).to_uri_string();
    }
}

/// All strings up to length 5 over a small alphabet mixing ASCII structure with
/// multi-byte characters must not panic in any `uri` entry point.
///
/// The alphabet is deliberately tiny (7 symbols) so the sweep covers
/// `7 + 7² + 7³ + 7⁴ + 7⁵` = 19 607 strings in well under a second, and
/// exhaustive rather than random — a panic found here is reproducible.
#[test]
fn exhaustive_short_inputs_never_panic() {
    const ALPHABET: [&str; 7] = ["a", "/", ".", "é", "?", "#", ":"];
    let base = "http://a/b/c/d;p?q";

    let mut tested = 0usize;
    let mut rec = |s: &str| {
        let out = remove_dot_segments(s);
        assert!(
            out.len() <= s.len(),
            "remove_dot_segments({s:?}) -> {out:?} grew"
        );
        let _ = resolve(base, s);
        let _ = resolve(s, base);
        let parsed = UriReference::parse(s);
        let _ = parsed.to_uri_string();
        let _ = merge(&UriReference::parse(base), s);
        tested += 1;
    };

    for a in ALPHABET {
        rec(a);
        for b in ALPHABET {
            rec(&format!("{a}{b}"));
            for c in ALPHABET {
                rec(&format!("{a}{b}{c}"));
                for d in ALPHABET {
                    rec(&format!("{a}{b}{c}{d}"));
                    for e in ALPHABET {
                        rec(&format!("{a}{b}{c}{d}{e}"));
                    }
                }
            }
        }
    }
    assert_eq!(
        tested,
        7 + 7 * 7 + 7 * 7 * 7 + 7 * 7 * 7 * 7 + 7 * 7 * 7 * 7 * 7
    );
}

/// Empty and very long inputs: the shortest and the longest things a caller can
/// hand over.
#[test]
fn empty_and_long_inputs_are_handled() {
    assert_eq!(remove_dot_segments(""), "");
    assert_eq!(resolve("http://a/b", ""), "http://a/b");
    assert_eq!(resolve("", ""), "");
    assert_eq!(remove_dot_segments("."), "");
    assert_eq!(remove_dot_segments(".."), "");

    // A path of many segments, all resolvable in one pass.
    let long: String = core::iter::repeat_n("seg/", 20_000).collect();
    let out = remove_dot_segments(&long);
    assert_eq!(out, long, "a dot-free path is unchanged");

    // A long run of dot-segments below the root collapses to "/".
    let deep: String = core::iter::repeat_n("../", 10_000).collect();
    assert_eq!(remove_dot_segments(&deep), "");

    let long_ref: String = core::iter::repeat_n("x", 100_000).collect();
    let resolved = resolve("http://a/b/", &long_ref);
    assert!(resolved.ends_with(&long_ref));
}

/// A URL carrying a control character or whitespace is refused by the fallible
/// resolvers.
///
/// RFC 3986 §2 restricts a URI to a small US-ASCII subset; a raw CR, LF, tab or
/// space has no meaning inside one, and accepting it lets a manifest smuggle a
/// second request line into whatever writes the HTTP request later.
#[test]
fn control_characters_and_whitespace_are_rejected() {
    let base = "http://a/b/";

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
        assert!(
            first_forbidden_char(evil).is_some(),
            "{evil:?} must be recognised as forbidden"
        );
        assert_eq!(
            try_resolve(base, evil),
            None,
            "a reference containing {evil:?} must be refused"
        );
    }
    // The base is checked too — a manifest cannot sneak one in that way either.
    assert_eq!(try_resolve("http://a/\r\nb/", "seg"), None);
    assert_eq!(
        try_resolve_segment(&["http://a/\nb/".to_string()], "s"),
        None
    );

    // Ordinary input still resolves through the fallible path.
    assert_eq!(
        try_resolve(base, "seg.m4s").as_deref(),
        Some("http://a/b/seg.m4s")
    );
    assert_eq!(
        try_resolve_segment(&["http://a/".to_string()], "s.m4s").as_deref(),
        Some("http://a/s.m4s")
    );
    // A percent-encoded control character is *not* raw text, so it is allowed
    // through — the caller's request builder is what must not decode it into a
    // separator. (The `%`-encoded form has no literal whitespace.)
    assert_eq!(
        try_resolve(base, "seg%0D%0AHost:evil").as_deref(),
        Some("http://a/b/seg%0D%0AHost:evil")
    );
}

/// The crate-root re-exports exist under their documented aliases.
#[test]
fn crate_root_re_exports_resolve() {
    assert_eq!(
        transmux::resolve_uri_reference("http://a/b/c/d;p?q", "g"),
        "http://a/b/c/g"
    );
    assert_eq!(
        transmux::resolve_uri_segment(&["http://a/".to_string()], "s.m4s"),
        "http://a/s.m4s"
    );
    assert_eq!(
        transmux::try_resolve_uri_reference("http://a/", "s\nm4s"),
        None
    );
    let parsed: transmux::UriReference = transmux::UriReference::parse("http://a/b?q#f");
    assert_eq!(parsed.to_uri_string(), "http://a/b?q#f");
}
