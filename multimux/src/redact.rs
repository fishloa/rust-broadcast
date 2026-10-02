//! Never let a credential-bearing URL escape into logs, errors, or `Debug`
//! output.
//!
//! A `rtsp://user:pass@host/path` source URL's userinfo (RFC 3986 §3.2.1) is
//! a live camera password. This module's [`redact_url`] is the one place
//! that turns such a URL into a safe-to-print form; every `Debug` impl and
//! error message that might otherwise embed a raw source URL goes through it.

/// The placeholder every redaction leaves in place of a secret.
pub(crate) const REDACTED: &str = "<redacted>";

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

/// Reduces a push/pull destination URL to `scheme://host[:port]` plus a
/// `/<redacted>` marker when anything followed (path, query, fragment) — safe to log
/// or `Debug`-print. Unlike [`redact_url`], which keeps the path, this also
/// hides what a destination URL commonly smuggles there: an RTMP stream key
/// (`rtmp://host/app/STREAMKEY`) or an SRT `streamid`/`passphrase` query.
/// The userinfo (`user:pass@`) is dropped entirely. Text with no `://` is
/// returned as `<redacted>` (a bare token could itself be the secret).
pub(crate) fn redact_destination(raw: &str) -> String {
    let Some(scheme_end) = raw.find("://") else {
        return REDACTED.to_string();
    };
    let scheme = &raw[..scheme_end + 3];
    let after_scheme = &raw[scheme_end + 3..];
    let authority_end = after_scheme
        .find(['/', '?', '#'])
        .unwrap_or(after_scheme.len());
    let authority = &after_scheme[..authority_end];
    let host = authority.rsplit_once('@').map_or(authority, |(_, h)| h);
    if authority_end < after_scheme.len() {
        format!("{scheme}{host}/{REDACTED}")
    } else {
        format!("{scheme}{host}")
    }
}

/// Remove every secret derived from `url` from `text` (an error message, a
/// `Display` of a transport error that may echo what it was asked to dial):
/// the whole URL, its userinfo, its path segments and its query keys/values
/// each become `<redacted>`. The scheme, host and port are kept. Tokens of
/// fewer than three bytes are left alone (they cannot be told from prose).
pub(crate) fn scrub_destination_secrets(text: &str, url: &str) -> String {
    let mut tokens: Vec<String> = vec![url.to_string()];
    if let Some(scheme_end) = url.find("://") {
        let after = &url[scheme_end + 3..];
        let authority_end = after.find(['/', '?', '#']).unwrap_or(after.len());
        let (authority, rest) = after.split_at(authority_end);
        if let Some((userinfo, _host)) = authority.rsplit_once('@') {
            tokens.push(userinfo.to_string());
            tokens.extend(userinfo.split(':').map(str::to_string));
        }
        tokens.push(rest.to_string());
        for piece in rest.split(['/', '?', '#', '&', '=']) {
            tokens.push(piece.to_string());
        }
    } else {
        // Scheme-less (`host:9000?streamid=..`): everything after the first
        // `?` or `/` is a secret candidate.
        if let Some(at) = url.find(['?', '/']) {
            tokens.push(url[at..].to_string());
            for piece in url[at..].split(['/', '?', '#', '&', '=']) {
                tokens.push(piece.to_string());
            }
        }
    }
    tokens.retain(|t| t.len() >= 3);
    // Longest first, so a whole URL goes before the pieces inside it.
    tokens.sort_by_key(|t| std::cmp::Reverse(t.len()));
    tokens.dedup();
    let mut out = text.to_string();
    for token in &tokens {
        out = out.replace(token.as_str(), REDACTED);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scrub_removes_every_secret_derived_from_the_url() {
        let url = "rtmp://admin:hunter2@live.example:1935/app/STREAMKEY123?token=abc&k=v";
        let text = format!(
            "connect to {url} failed: user admin pass hunter2 key STREAMKEY123 token=abc, \
             live.example:1935 refused"
        );
        let scrubbed = scrub_destination_secrets(&text, url);
        for secret in ["admin", "hunter2", "STREAMKEY123", "abc", "rtmp://admin"] {
            assert!(!scrubbed.contains(secret), "{secret} left in {scrubbed}");
        }
        assert!(scrubbed.contains("live.example:1935 refused"), "{scrubbed}");
        assert!(scrubbed.contains(REDACTED), "{scrubbed}");
        // Short tokens are prose, not secrets.
        assert_eq!(
            scrub_destination_secrets("k is v", "rtmp://h/a/k?k=v"),
            "k is v"
        );
    }

    #[test]
    fn destination_keeps_only_scheme_and_host() {
        assert_eq!(
            redact_destination("rtmp://user:pw@live.example:1935/app/SECRETKEY?k=v"),
            "rtmp://live.example:1935/<redacted>"
        );
        assert_eq!(
            redact_destination("srt://host:9000?streamid=secret&latency=200"),
            "srt://host:9000/<redacted>"
        );
        assert_eq!(redact_destination("rtsp://host:554"), "rtsp://host:554");
        assert_eq!(
            redact_destination("rtsp://u@host/p"),
            "rtsp://host/<redacted>"
        );
        assert_eq!(redact_destination("SECRETKEY"), "<redacted>");
    }

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
