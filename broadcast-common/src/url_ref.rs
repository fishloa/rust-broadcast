//! Guarded RFC 3986 §5 reference resolution (issue #2458).
//!
//! One resolver for every manifest reader in the workspace. The `url` crate
//! implements RFC 3986 §5 (as the WHATWG URL algorithm) via [`Url::join`], but
//! it *deletes* a raw TAB, CR or LF instead of rejecting it, so `a\r\nb` would
//! join to a clean-looking `ab` and a manifest reference could become a
//! request to a URL the manifest never named. [`resolve`] therefore rejects any
//! reference (or base) containing an ASCII control character *before* joining;
//! otherwise the result is exactly `Url::join`'s.
//!
//! `url` is `no_std` + `alloc`, so this module needs neither `std` nor the
//! crate's `std` feature.

use alloc::string::ToString;

use url::Url;

/// Why a reference could not be resolved.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum Error {
    /// The reference (or the base) contained an ASCII control character, which
    /// RFC 3986 §2 does not allow in a URI.
    ForbiddenChar(char),
    /// The base or the joined reference did not parse as a URL.
    Parse,
    /// The joined URL's scheme is not `http`/`https`, so a fetch would go to a
    /// transport the manifest reader never speaks (`file:`, `ftp:`, `data:`).
    Scheme,
    /// The joined URL would fetch `http` over an `https` base's connection
    /// identity, downgrading the reference's transport.
    SchemeDowngrade,
}

impl Error {
    /// Stable label for this error kind.
    pub fn name(&self) -> &'static str {
        match self {
            Self::ForbiddenChar(_) => "forbidden char",
            Self::Parse => "parse",
            Self::Scheme => "scheme",
            Self::SchemeDowngrade => "scheme downgrade",
        }
    }
}

crate::impl_spec_display!(Error);

impl core::error::Error for Error {}

/// Result alias for this module.
pub type Result<T> = core::result::Result<T, Error>;

/// The first character of `s` that cannot appear in a URI: an ASCII control
/// character (CR, LF, TAB, other C0, DEL) or a space.
///
/// RFC 3986 §2 restricts a URI to a US-ASCII subset with neither, and the
/// WHATWG parser behind [`Url::join`] deletes TAB, CR and LF outright while
/// percent-encoding a space — in both cases joining a *different* URL than the
/// text names, so a reference carrying one is refused before the join. The
/// space is included because a manifest reader that normalises XML attribute
/// whitespace (XML 1.0 §3.3.3 turns a TAB, LF or CR in an attribute into a
/// space) hands the resolver exactly such a renamed reference.
pub fn first_forbidden_char(s: &str) -> Option<char> {
    s.chars().find(|c| c.is_ascii_control() || *c == ' ')
}

/// Resolve `reference` against `base`, per RFC 3986 §5.2.
///
/// # Errors
/// [`Error::ForbiddenChar`] when the reference or the base contains a control
/// character; [`Error::Parse`] when the two do not join into a URL.
pub fn resolve(base: &Url, reference: &str) -> Result<Url> {
    if let Some(c) = first_forbidden_char(reference) {
        return Err(Error::ForbiddenChar(c));
    }
    if let Some(c) = first_forbidden_char(base.as_str()) {
        return Err(Error::ForbiddenChar(c));
    }
    base.join(reference).map_err(|_| Error::Parse)
}

/// Resolve `reference` against the URL text `base`, returning the joined URL
/// as text.
///
/// # Errors
/// As [`resolve`], plus [`Error::Parse`] when `base` is not a URL.
pub fn resolve_str(base: &str, reference: &str) -> Result<alloc::string::String> {
    let base = Url::parse(base).map_err(|_| Error::Parse)?;
    resolve(&base, reference).map(|joined| joined.to_string())
}

/// Resolve `reference` against `base`, requiring the result to be an
/// `http`/`https` URL and refusing an `https` -> `http` downgrade.
///
/// A manifest reference can name any scheme (`file:`, `ftp:`, `data:`), and a
/// client that forwards whatever the resolver produced would fetch it over a
/// transport it never intended; an `https` playlist naming an absolute `http:`
/// resource would also drop the confidentiality the playlist was fetched with.
/// A relative reference keeps the base's scheme by RFC 3986 §5.2.2.
///
/// # Errors
/// As [`resolve`], plus [`Error::Scheme`] when the joined URL is not
/// `http`/`https`, or [`Error::SchemeDowngrade`] when an `https` base resolves
/// to `http`.
pub fn resolve_http(base: &Url, reference: &str) -> Result<Url> {
    let joined = resolve(base, reference)?;
    match joined.scheme() {
        "http" | "https" => {}
        _ => return Err(Error::Scheme),
    }
    if base.scheme() == "https" && joined.scheme() == "http" {
        return Err(Error::SchemeDowngrade);
    }
    Ok(joined)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn base() -> Url {
        Url::parse("http://h/a/b/playlist.mpd").expect("base parses")
    }

    #[test]
    fn control_characters_are_rejected_before_joining() {
        for evil in [
            "seg\r\n1.m4s",
            "seg\n1.m4s",
            "seg\r1.m4s",
            "seg\t1.m4s",
            "seg\u{0}1.m4s",
            "seg\u{1b}[31m",
            "seg\u{7f}",
            "seg 1.m4s",
            "/di\r\nr/seg.m4s",
            "http://cdn/seg\r\n.m4s",
        ] {
            let first = first_forbidden_char(evil).expect("has one");
            assert_eq!(
                resolve(&base(), evil),
                Err(Error::ForbiddenChar(first)),
                "{evil:?}"
            );
            assert_eq!(
                resolve_str("http://h/a/b/playlist.mpd", evil),
                Err(Error::ForbiddenChar(first)),
                "{evil:?}"
            );
        }
    }

    #[test]
    fn a_clean_reference_carries_no_forbidden_character() {
        assert_eq!(first_forbidden_char("seg1.m4s"), None);
        assert_eq!(first_forbidden_char("my-seg1.m4s"), None);
        assert_eq!(first_forbidden_char("./a/../b.m4s?q=1#f"), None);
        assert_eq!(
            resolve(&base(), "my seg1.m4s"),
            Err(Error::ForbiddenChar(' ')),
            "a space would be percent-encoded into a different URL"
        );
    }

    #[test]
    fn relative_references_resolve_as_url_join() {
        let base = base();
        for r in [
            "seg1.m4s",
            "./seg1.m4s",
            "../seg1.m4s",
            "../../seg1.m4s",
            "../../../../seg1.m4s",
            "a/./b/../c.m4s",
        ] {
            assert_eq!(resolve(&base, r), Ok(base.join(r).expect("joins")), "{r:?}");
        }
        assert_eq!(
            resolve(&base, "seg1.m4s").expect("joins").as_str(),
            "http://h/a/b/seg1.m4s"
        );
        assert_eq!(
            resolve(&base, "../seg1.m4s").expect("joins").as_str(),
            "http://h/a/seg1.m4s"
        );
        assert_eq!(
            resolve(&base, "../../seg1.m4s").expect("joins").as_str(),
            "http://h/seg1.m4s"
        );
        assert_eq!(
            resolve(&base, "../../../../seg1.m4s")
                .expect("joins")
                .as_str(),
            "http://h/seg1.m4s",
            "cannot climb above the root"
        );
    }

    #[test]
    fn absolute_and_authority_references_resolve_as_url_join() {
        let base = base();
        for r in [
            "https://cdn.example/seg1.m4s",
            "//cdn.example/seg1.m4s",
            "/segs/seg1.m4s",
            "?q=1",
            "#frag",
        ] {
            assert_eq!(resolve(&base, r), Ok(base.join(r).expect("joins")), "{r:?}");
        }
    }

    #[test]
    fn resolve_str_renders_the_joined_url() {
        assert_eq!(
            resolve_str("http://h/a/b/playlist.mpd", "seg1.m4s"),
            Ok("http://h/a/b/seg1.m4s".to_string())
        );
        assert_eq!(
            resolve_str("http://h/a/b/playlist.mpd", "seg\r\n1.m4s"),
            Err(Error::ForbiddenChar('\r'))
        );
        assert_eq!(resolve_str("not a url", "seg1.m4s"), Err(Error::Parse));
    }

    #[test]
    fn an_unparseable_reference_is_a_parse_error() {
        assert_eq!(resolve(&base(), "http://[::1"), Err(Error::Parse));
    }

    #[test]
    fn labels() {
        assert_eq!(Error::ForbiddenChar('\r').name(), "forbidden char");
        assert_eq!(Error::ForbiddenChar('\r').to_string(), "forbidden char");
        assert_eq!(Error::Parse.name(), "parse");
        assert_eq!(Error::Scheme.name(), "scheme");
        assert_eq!(Error::SchemeDowngrade.name(), "scheme downgrade");
        assert_eq!(Error::Scheme.to_string(), "scheme");
    }

    #[test]
    fn resolve_http_accepts_http_https_and_relative_references() {
        let http = base();
        for r in ["seg1.m4s", "/other.m4s", "https://cdn.example/a.m4s"] {
            assert_eq!(
                resolve_http(&http, r),
                resolve(&http, r),
                "http base accepts {r:?}"
            );
        }
        let https = Url::parse("https://h/a/b/playlist.m3u8").expect("base parses");
        for r in ["seg1.m4s", "/other.m4s", "https://cdn.example/a.m4s"] {
            assert_eq!(
                resolve_http(&https, r),
                resolve(&https, r),
                "https base accepts {r:?}"
            );
        }
        assert_eq!(
            resolve_http(&http, "http://cdn.example/a.m4s")
                .expect("http base accepts http")
                .as_str(),
            "http://cdn.example/a.m4s"
        );
    }

    /// A manifest naming a non-HTTP transport is refused rather than fetched:
    /// `file:`/`ftp:`/`data:` never reach a client.
    #[test]
    fn resolve_http_refuses_a_non_http_scheme() {
        for r in [
            "file:///etc/passwd",
            "ftp://cdn.example/a.m4s",
            "data:text/plain,secret",
            "srt://host:9000?streamid=x",
        ] {
            assert_eq!(resolve_http(&base(), r), Err(Error::Scheme), "{r:?}");
        }
    }

    /// An `https` playlist may not name an absolute `http:` resource (a
    /// relative reference keeps `https`; an explicit upgrade is fine).
    #[test]
    fn resolve_http_refuses_an_https_to_http_downgrade() {
        let https = Url::parse("https://h/a/b/playlist.m3u8").expect("base parses");
        assert_eq!(
            resolve_http(&https, "http://cdn.example/a.m4s"),
            Err(Error::SchemeDowngrade)
        );
        assert_eq!(
            resolve_http(&https, "seg1.m4s")
                .expect("relative keeps https")
                .scheme(),
            "https"
        );
        assert_eq!(
            resolve_http(&base(), "https://cdn.example/a.m4s")
                .expect("an upgrade is allowed")
                .scheme(),
            "https"
        );
    }

    /// The control-character guard still runs first.
    #[test]
    fn resolve_http_keeps_the_control_character_guard() {
        assert_eq!(
            resolve_http(&base(), "seg\r\n1.m4s"),
            Err(Error::ForbiddenChar('\r'))
        );
    }
}
