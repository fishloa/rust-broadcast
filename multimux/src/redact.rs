//! Never let a credential-bearing URL escape into logs, errors, or `Debug`
//! output.
//!
//! A `rtsp://user:pass@host/path` source URL's userinfo (RFC 3986 §3.2.1) is
//! a live camera password. This module's [`redact_url`] is the one place
//! that turns such a URL into a safe-to-print form; every `Debug` impl and
//! error message that might otherwise embed a raw source URL goes through it.

/// Redacts the userinfo (`user[:pass]@`) portion of a URL-shaped string to
/// `***@`, leaving the scheme, host, and path intact — e.g.
/// `"rtsp://user:secret@host/s"` becomes `"rtsp://***@host/s"`.
///
/// Operates purely on the text (not a parsed [`url::Url`]), so it works
/// equally on a URL that failed to parse in the first place (the common case
/// for a connect-time error message) and one that parsed fine. Only the
/// authority component (between `://` and the next `/`) is searched for
/// `@`, so a literal `@` appearing later — in the path or query — is never
/// mistaken for a userinfo separator. If there is no `://` or no `@` in the
/// authority, the string is returned unchanged (nothing to redact).
pub(crate) fn redact_url(raw: &str) -> String {
    let Some(scheme_end) = raw.find("://") else {
        return raw.to_string();
    };
    let after_scheme = &raw[scheme_end + 3..];
    let authority_end = after_scheme.find('/').unwrap_or(after_scheme.len());
    let authority = &after_scheme[..authority_end];
    let Some(at) = authority.rfind('@') else {
        return raw.to_string();
    };
    let scheme = &raw[..scheme_end + 3];
    let rest = &after_scheme[at + 1..];
    format!("{scheme}***@{rest}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn redacts_userinfo_from_credentialed_url() {
        let redacted = redact_url("rtsp://user:secretpass@host/s");
        assert_eq!(redacted, "rtsp://***@host/s");
        assert!(!redacted.contains("user"));
        assert!(!redacted.contains("secretpass"));
    }

    #[test]
    fn leaves_url_without_userinfo_unchanged() {
        assert_eq!(redact_url("rtsp://host/s"), "rtsp://host/s");
    }

    #[test]
    fn leaves_non_url_string_unchanged() {
        assert_eq!(redact_url("not a url"), "not a url");
    }

    #[test]
    fn does_not_mistake_a_path_at_sign_for_userinfo() {
        // No userinfo here — the `@` is in the path, after the authority.
        assert_eq!(
            redact_url("rtsp://host/user@host/s"),
            "rtsp://host/user@host/s"
        );
    }

    #[test]
    fn redacts_username_only_credentials() {
        let redacted = redact_url("rtsp://user@host/s");
        assert_eq!(redacted, "rtsp://***@host/s");
        assert!(!redacted.contains("user"));
    }
}
