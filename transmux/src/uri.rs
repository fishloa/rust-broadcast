//! RFC 3986 URI-reference parsing and resolution — the piece a DASH client
//! needs to turn a relative `SegmentTemplate`/`BaseURL` reference into the
//! absolute URL it fetches.
//!
//! This is deliberately a *subset* of RFC 3986, sized to what a media client
//! resolves: hierarchical `http(s)`/`file`-style references with a scheme,
//! optional authority, path, query and fragment. Nothing here validates a
//! scheme's own syntax, percent-encodes or decodes, or performs the
//! case/percent normalisation of §6.2.2 — a reference is resolved exactly as
//! written, which is what §5.2 specifies.
//!
//! # Spec
//!
//! - **§3** syntax components (`scheme`, `authority`, `path`, `query`,
//!   `fragment`); **§3.3** path forms, including the `path-abempty`,
//!   `path-absolute` and `path-noscheme` distinction between an empty, an
//!   absolute and a relative path.
//! - **§5.2.2** `Transform References` — the resolution algorithm.
//! - **§5.2.3** `Merge Paths`.
//! - **§5.2.4** `Remove Dot Segments`.
//! - **§5.3** component recomposition.
//! - **§5.4.1** the normal-example table, used verbatim as the test oracle.
//!
//! `no_std` + `alloc`.

use alloc::string::{String, ToString};

/// A parsed URI reference (RFC 3986 §3).
///
/// Each of `authority`, `query` and `fragment` is `Option`, preserving the
/// §5.3 distinction between a component that is **undefined** (its delimiter
/// was absent) and one that is **empty** (the delimiter was present and
/// immediately followed by the next one) — resolution depends on it (`?y` and
/// an empty reference resolve differently).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct UriReference {
    /// The scheme, without its `:`. Lower-cased on parse (§3.1: scheme names
    /// are case-insensitive, canonical form lowercase).
    pub scheme: Option<String>,
    /// Everything between `//` and the next `/`, `?` or `#`.
    pub authority: Option<String>,
    /// The path, always present (possibly empty).
    pub path: String,
    /// Everything after the first `?`, excluding the fragment.
    pub query: Option<String>,
    /// Everything after the first `#`.
    pub fragment: Option<String>,
}

impl UriReference {
    /// Parse a URI reference (RFC 3986 §3 / §4.1).
    ///
    /// Never fails: any string is a syntactically valid reference under the
    /// generic grammar, since an unrecognised prefix is simply a relative
    /// path (§4.1 — "A URI-reference is either a URI or a relative reference").
    pub fn parse(input: &str) -> Self {
        // §3.5: the fragment is delimited by the first '#' and is not part of
        // the query, so split it off before looking for '?'.
        let (rest, fragment) = match input.split_once('#') {
            Some((before, frag)) => (before, Some(frag.to_string())),
            None => (input, None),
        };
        let (rest, query) = match rest.split_once('?') {
            Some((before, q)) => (before, Some(q.to_string())),
            None => (rest, None),
        };

        // §3.1: a scheme is ALPHA *( ALPHA / DIGIT / "+" / "-" / "." ) followed
        // by ':' — and it must appear before the first '/', '?' or '#' (the
        // latter two are already split off), or it is part of the path.
        let (scheme, rest) = match split_scheme(rest) {
            Some((s, tail)) => (Some(s.to_ascii_lowercase()), tail),
            None => (None, rest),
        };

        // §3.2/§3.3: an authority is present only for a "//" prefix; without
        // it the two slashes belong to the path (the `path-noscheme` rule
        // cannot begin with "//", so this is unambiguous).
        let (authority, path) = match rest.strip_prefix("//") {
            Some(after) => {
                let end = after.find(['/', '?', '#']).unwrap_or(after.len());
                (Some(after[..end].to_string()), after[end..].to_string())
            }
            None => (None, rest.to_string()),
        };

        UriReference {
            scheme,
            authority,
            path,
            query,
            fragment,
        }
    }

    /// Recompose the reference into a string (RFC 3986 §5.3).
    pub fn to_uri_string(&self) -> String {
        let mut out = String::new();
        if let Some(scheme) = &self.scheme {
            out.push_str(scheme);
            out.push(':');
        }
        if let Some(authority) = &self.authority {
            out.push_str("//");
            out.push_str(authority);
        }
        out.push_str(&self.path);
        if let Some(query) = &self.query {
            out.push('?');
            out.push_str(query);
        }
        if let Some(fragment) = &self.fragment {
            out.push('#');
            out.push_str(fragment);
        }
        out
    }

    /// True if the reference has a scheme (i.e. is a URI, not a relative
    /// reference — §4.1).
    pub fn is_absolute(&self) -> bool {
        self.scheme.is_some()
    }
}

/// Split an `ALPHA *( ALPHA / DIGIT / "+" / "-" / "." ) ":"` scheme prefix off
/// `input`, returning it (without the colon) and the remainder.
///
/// `None` when `input` does not begin with a valid scheme — in particular when
/// the first `:` appears after a `/`, which per §3.3 makes it part of the path
/// rather than a scheme separator.
fn split_scheme(input: &str) -> Option<(&str, &str)> {
    let colon = input.find(':')?;
    let candidate = &input[..colon];
    let mut chars = candidate.chars();
    let first = chars.next()?;
    if !first.is_ascii_alphabetic() {
        return None;
    }
    if !chars.all(|c| c.is_ascii_alphanumeric() || matches!(c, '+' | '-' | '.')) {
        return None;
    }
    Some((candidate, &input[colon + 1..]))
}

/// Resolve `reference` against `base` (RFC 3986 §5.2.2 `Transform
/// References`), returning the target URI as a string.
///
/// The base is expected to be absolute (it must have a scheme, §5.1); a
/// relative `base` still resolves, producing a relative result, which is the
/// literal behaviour of the pseudocode.
///
/// A strict parser is used: a reference that restates the base's scheme is
/// treated as an absolute URI rather than the legacy "same scheme" loophole
/// (§5.4.2's trailing `http:g` note), so `"http:g"` against
/// `http://a/b/c/d;p?q` gives `http:g`.
pub fn resolve(base: &str, reference: &str) -> String {
    let base = UriReference::parse(base);
    let r = UriReference::parse(reference);
    let mut target = UriReference::default();

    if r.scheme.is_some() {
        target.scheme = r.scheme.clone();
        target.authority = r.authority.clone();
        target.path = remove_dot_segments(&r.path);
        target.query = r.query.clone();
    } else {
        if r.authority.is_some() {
            target.authority = r.authority.clone();
            target.path = remove_dot_segments(&r.path);
            target.query = r.query.clone();
        } else {
            if r.path.is_empty() {
                target.path = base.path.clone();
                target.query = if r.query.is_some() {
                    r.query.clone()
                } else {
                    base.query.clone()
                };
            } else {
                if r.path.starts_with('/') {
                    target.path = remove_dot_segments(&r.path);
                } else {
                    target.path = remove_dot_segments(&merge(&base, &r.path));
                }
                target.query = r.query.clone();
            }
            target.authority = base.authority.clone();
        }
        target.scheme = base.scheme.clone();
    }
    target.fragment = r.fragment.clone();

    target.to_uri_string()
}

/// Merge a relative-path reference into the base path (RFC 3986 §5.2.3).
///
/// With a defined authority and an empty base path the result is `/` + the
/// reference; otherwise everything after the base path's last `/` is dropped
/// and the reference appended.
pub fn merge(base: &UriReference, reference_path: &str) -> String {
    let mut out = String::new();
    if base.authority.is_some() && base.path.is_empty() {
        out.push('/');
    } else {
        if let Some(slash) = base.path.rfind('/') {
            out.push_str(&base.path[..=slash]);
        }
    }
    out.push_str(reference_path);
    out
}

/// Remove the special `.` and `..` complete path segments (RFC 3986 §5.2.4).
///
/// A faithful transcription of the standard's two-buffer pseudocode: the input
/// buffer is consumed from the front, and the output buffer accumulates whole
/// segments. The `.`/`..` prefixes the steps match are *complete* path segments
/// — a partial one (`g.`, `.g`, `..g`) is an ordinary segment and survives.
///
/// Implemented with a byte cursor over `path` rather than by repeatedly
/// re-allocating a shrinking prefix, since §5.2.4's steps only ever consume
/// from the front of the input.
pub fn remove_dot_segments(path: &str) -> String {
    let bytes = path.as_bytes();
    let mut pos = 0usize;
    let mut output = String::with_capacity(path.len());

    while pos < bytes.len() {
        let rest = &path[pos..];
        // 2A: a leading "../" or "./" is removed.
        if rest.starts_with("../") {
            pos += 3;
            continue;
        }
        if rest.starts_with("./") {
            pos += 2;
            continue;
        }
        // 2B: a leading "/./" is replaced by "/".
        if rest.starts_with("/./") {
            // Consume only the "." — leaving "/..." in place, i.e. 2B's
            // "replace that prefix with /".
            pos += 2;
            continue;
        }
        // 2B: "/." as a complete segment — "replace that prefix with /",
        // i.e. the trailing dot goes and the slash becomes the last segment.
        if rest == "/." {
            output.push('/');
            pos = bytes.len();
            continue;
        }
        // 2C: a leading "/../" is replaced by "/", popping the output segment.
        if rest.starts_with("/../") {
            pop_last_segment(&mut output);
            pos += 3;
            continue;
        }
        // 2C: "/.." as a complete segment, likewise — pop the output segment
        // and leave the "/" as the new last one.
        if rest == "/.." {
            pop_last_segment(&mut output);
            output.push('/');
            pos = bytes.len();
            continue;
        }
        // 2D: the input is exactly "." or "..".
        if rest == "." || rest == ".." {
            pos = bytes.len();
            continue;
        }
        // 2E: move the first segment (with its leading "/", if any) across.
        //
        // The segment ends at the next `/`, searched for from the *second
        // character* — `rest[1..]` would slice at byte 1 and panic whenever the
        // first character is multi-byte, which `resolve` reaches directly with
        // a non-ASCII relative reference. `char_indices().skip(1)` yields
        // character boundaries instead of byte offsets.
        let cut = match rest.char_indices().skip(1).find(|&(_, c)| c == '/') {
            Some((at, _)) => at,
            None => rest.len(),
        };
        output.push_str(&rest[..cut]);
        pos += cut;
    }
    output
}

/// Drop the last path segment of `output` and its preceding `/` (the "remove
/// the last segment" step of §5.2.4 2C).
fn pop_last_segment(output: &mut String) {
    match output.rfind('/') {
        Some(slash) => output.truncate(slash),
        None => output.clear(),
    }
}

/// The RFC 3986 §5.4.1 normal-example table, as `(reference, expected)` pairs
/// against the base `http://a/b/c/d;p?q`.
///
/// Exposed publicly so a caller (or a downstream crate) can re-run the standard
/// conformance table against this implementation.
pub const RFC3986_NORMAL_EXAMPLES: &[(&str, &str)] = &[
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

/// The base URI the §5.4.1 table is defined against.
pub const RFC3986_NORMAL_EXAMPLES_BASE: &str = "http://a/b/c/d;p?q";

/// The RFC 3986 §5.4.2 abnormal-example table, `(reference, expected)` pairs,
/// against the same base.
pub const RFC3986_ABNORMAL_EXAMPLES: &[(&str, &str)] = &[
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

/// Resolve a `BaseURL` chain (outermost first) followed by a segment reference.
///
/// Each entry resolves against the running result, so a not-yet-absolute entry
/// composes with the ones above it and an absolute one resets the chain
/// (RFC 3986 §5.2 applied in order). Empty and whitespace-only entries are
/// skipped — ISO/IEC 23009-1 §5.6.5 makes several `BaseURL`s alternates for the
/// same level, so an empty one contributes nothing rather than resolving the
/// rest against nothing.
///
/// With no usable entry the reference is returned as-is, on the caller's
/// assumption that it is already absolute.
pub fn resolve_segment(base_chain: &[String], reference: &str) -> String {
    let mut current = String::new();
    for b in base_chain {
        let b = b.trim();
        if b.is_empty() {
            continue;
        }
        current = if current.is_empty() {
            b.to_string()
        } else {
            resolve(&current, b)
        };
    }
    if current.is_empty() {
        return reference.to_string();
    }
    resolve(&current, reference)
}

/// The first control character or whitespace in `s`, if any.
///
/// RFC 3986 §2 restricts a URI to a small US-ASCII subset, and a raw CR, LF, tab
/// or space has no meaning in one. Accepting them lets a reference smuggle a
/// second request line or header into anything that later assembles an HTTP
/// request from the resolved URL (CRLF injection), so the fallible resolvers
/// reject them. [`resolve`] cannot — it returns a plain `String` — and passes
/// them through unchanged, which is why a caller that builds requests should
/// use [`try_resolve`] and [`try_resolve_segment`] instead.
pub fn first_forbidden_char(s: &str) -> Option<char> {
    s.chars().find(|c| c.is_control() || c.is_whitespace())
}

/// [`resolve`], but rejecting a base or reference containing a control
/// character or whitespace (see [`first_forbidden_char`]).
///
/// Returns `None` for such input rather than a URL a request builder would
/// later split across lines.
pub fn try_resolve(base: &str, reference: &str) -> Option<String> {
    if first_forbidden_char(base).is_some() || first_forbidden_char(reference).is_some() {
        return None;
    }
    Some(resolve(base, reference))
}

/// [`resolve_segment`], but rejecting a control character or whitespace in the
/// chain or the reference (see [`try_resolve`]).
pub fn try_resolve_segment(base_chain: &[String], reference: &str) -> Option<String> {
    if first_forbidden_char(reference).is_some()
        || base_chain.iter().any(|b| first_forbidden_char(b).is_some())
    {
        return None;
    }
    Some(resolve_segment(base_chain, reference))
}
