//! BaseURL and URL-reference resolution (ISO/IEC 23009-1 §5.6.5 over RFC 3986
//! §5, as implemented by the `url` crate — SP3). `std` only.
//!
//! A DASH client turns a relative `SegmentTemplate`/`BaseURL` reference into the
//! URL it fetches by joining a `BaseURL` chain onto the MPD's own URL. With no
//! location (an in-memory MPD) the join happens against a fixed synthetic
//! root, `transmux-relative:///`, and [`render`] — the ONE place that knows
//! about it — turns the result back into a reference.
//!
//! # Containment with no base
//!
//! A relative input never resolves to anything but a relative result: RFC 3986
//! §5.2.4 `remove_dot_segments` drops a `..` that would climb above the root,
//! so `../../../etc/passwd` is `etc/passwd`, never `/etc/passwd` (a consumer
//! that does `local_dir.join(resolved)` must not see an absolute path from an
//! untrusted MPD). A result is absolute-path (`/x`) only when the MPD itself
//! wrote an absolute-path reference (`/x`) in the chain or as the reference.
//! Percent-encoded dot segments (`%2e%2e`) are dot segments, as in WHATWG.
//! An input using the synthetic scheme itself is rejected (`None`).
//!
//! The `url` crate implements the WHATWG URL algorithm; it differs from strict
//! RFC 3986 in two §5.4 rows (see `tests/base_url.rs`).

use url::Url;

/// Scheme of the synthetic root (never leaves this module's results).
const SYNTHETIC_SCHEME: &str = "transmux-relative";
/// The synthetic root a base-less resolution joins against.
const SYNTHETIC_BASE: &str = "transmux-relative:///";

/// The first control character or whitespace in `s`, if any.
///
/// RFC 3986 §2 restricts a URI to a small US-ASCII subset, and a raw CR, LF, tab
/// or space has no meaning in one. The WHATWG parser behind [`resolve`]
/// silently DELETES tab, CR and LF instead of rejecting them (so
/// `Url::join("a\r\nHost: x")` would yield a clean-looking `aHost:%20x`), which
/// is why this check runs on the raw text BEFORE any join and cannot be left to
/// the parser. Accepting such characters would let a reference smuggle a second
/// request line or header into anything that later assembles an HTTP request
/// from the resolved URL (CRLF injection).
pub fn first_forbidden_char(s: &str) -> Option<char> {
    s.chars().find(|c| c.is_control() || c.is_whitespace())
}

/// Resolve `reference` against `base` (RFC 3986 §5.2 as the `url` crate
/// implements it). `base` is the MPD's own URL (a source URL, or
/// `Url::from_file_path`); `None` for an in-memory MPD, in which case a result
/// that stays relative is returned relative. `None` is returned when the
/// reference contains a control character or whitespace
/// ([`first_forbidden_char`]) or does not parse.
pub fn resolve(base: Option<&Url>, reference: &str) -> Option<String> {
    resolve_chain(base, &[], reference)
}

/// Resolve a `BaseURL` chain (outermost first) followed by a segment
/// reference. Each entry is joined onto the running result, so a relative
/// entry composes with the ones above it and an absolute one resets the chain.
/// Empty and whitespace-only entries are skipped — ISO/IEC 23009-1 §5.6.5 makes
/// several `BaseURL`s alternates for the same level, so an empty one
/// contributes nothing. `None` for a control character or whitespace in any
/// entry or the reference ([`first_forbidden_char`]), for an input that names
/// the synthetic scheme, or for a join error. With `base == None` a relative
/// outcome stays relative and contained (see the module docs).
pub fn resolve_chain(base: Option<&Url>, chain: &[String], reference: &str) -> Option<String> {
    // `BaseURL` element text routinely carries surrounding whitespace/newlines
    // from the XML layout, so entries are trimmed (and blank ones dropped)
    // BEFORE the forbidden-character guard; an interior control character or
    // space still rejects the whole resolution. The reference is checked as is.
    let entries: Vec<&str> = chain
        .iter()
        .map(|b| b.trim())
        .filter(|b| !b.is_empty())
        .collect();
    if first_forbidden_char(reference).is_some()
        || entries.iter().any(|b| first_forbidden_char(b).is_some())
        || entries
            .iter()
            .chain(core::iter::once(&reference))
            .any(|r| names_synthetic_scheme(r))
    {
        return None;
    }
    let mut current = match base {
        Some(b) => b.clone(),
        None => Url::parse(SYNTHETIC_BASE).ok()?,
    };
    // Set once the MPD itself wrote an absolute-path reference (`/x`).
    let mut absolute_path = false;
    for entry in entries.iter().copied().chain(core::iter::once(reference)) {
        if is_absolute_path_reference(entry) {
            absolute_path = true;
        }
        current = current.join(entry).ok()?;
    }
    Some(render(&current, absolute_path))
}

/// `true` for `/x` (but not `//host/x`): an absolute-path reference.
fn is_absolute_path_reference(r: &str) -> bool {
    r.starts_with('/') && !r.starts_with("//")
}

/// `true` when `r` starts with the synthetic scheme (`transmux-relative:`),
/// case-insensitively.
fn names_synthetic_scheme(r: &str) -> bool {
    r.len() > SYNTHETIC_SCHEME.len()
        && r.as_bytes()[..SYNTHETIC_SCHEME.len()].eq_ignore_ascii_case(SYNTHETIC_SCHEME.as_bytes())
        && r.as_bytes()[SYNTHETIC_SCHEME.len()] == b':'
}

/// The single place the synthetic root is stripped. A URL that is not under it
/// is its own text. A synthetic URL with an authority (`//host/x`) keeps its
/// `//host/x` form. Otherwise the path is returned as a reference: relative
/// (the leading `/` of the synthetic root removed, so nothing can climb out of
/// it) unless `absolute_path` says the MPD wrote an absolute-path reference.
pub fn render(url: &Url, absolute_path: bool) -> String {
    let text = url.as_str();
    let Some(rest) = text
        .strip_prefix(SYNTHETIC_SCHEME)
        .and_then(|r| r.strip_prefix(':'))
    else {
        return text.to_owned();
    };
    // `rest` is `//<authority><path>[?query][#fragment]`.
    let Some(after_slashes) = rest.strip_prefix("//") else {
        return rest.to_owned();
    };
    if !after_slashes.starts_with('/') {
        return rest.to_owned(); // `//host/...`: a scheme-relative reference
    }
    if absolute_path {
        after_slashes.to_owned()
    } else {
        after_slashes[1..].to_owned()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn synthetic_base_parses() {
        assert!(Url::parse(SYNTHETIC_BASE).is_ok());
    }

    #[test]
    fn render_cases() {
        let r = |s: &str, abs: bool| render(&Url::parse(s).unwrap(), abs);
        assert_eq!(
            r("transmux-relative:///a/b.m4s?q=1#f", false),
            "a/b.m4s?q=1#f"
        );
        assert_eq!(r("transmux-relative:///x/y", true), "/x/y");
        assert_eq!(r("transmux-relative://h/x", false), "//h/x");
        assert_eq!(r("https://h/a/b", false), "https://h/a/b");
    }
}
