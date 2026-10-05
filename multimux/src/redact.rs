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
/// A URL the `url` crate parses is redacted through it
/// (`Url::set_username`/`set_password(None)`, then re-serialized), so an
/// IPv6 host stays bracketed and every component is handled per RFC 3986.
/// A URL that fails to parse (the common case for a connect-time error
/// message) falls back to `redact_unparseable_userinfo`, a masking-only
/// text scrub of the `@`-delimited credential prefix. If there is nothing to
/// redact, the string is returned unchanged.
pub fn redact_url(raw: &str) -> String {
    if let Ok(mut url) = url::Url::parse(raw) {
        if url.username().is_empty() && url.password().is_none() {
            return raw.to_string();
        }
        // Keep the `***@` marker (the pre-url text scrub wrote it too), so a
        // reader sees a credential WAS present and was redacted, rather than
        // a bare host that looks like the URL never carried one.
        let _ = url.set_username("***");
        let _ = url.set_password(None);
        return url.to_string();
    }
    redact_unparseable_userinfo(raw)
}

/// The masking-only fallback for a URL-shaped string the `url` parser rejects
/// (redact.rs's documented contract: redaction must also work on a URL that
/// "failed to parse in the first place"). This is spec §9.5 documented
/// exception, allowlisted by name in the no-hand-roll guard.
///
/// The credential prefix before the authority's `@` becomes `***@` and the
/// rest of the authority + path/query is kept, exactly as [`redact_url`]'s
/// text scrub always did (`"rtsp://user:secret@host/s"` →
/// `"rtsp://***@host/s"`) — the host and path are NOT secrets, so they stay
/// legible; only the credential is masked. No `://` or no `@` in the authority
/// leaves the string unchanged.
fn redact_unparseable_userinfo(raw: &str) -> String {
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
///
/// A parseable URL goes through the `url` crate (host/port from the parsed
/// authority, never a hand split). A URL the parser rejects falls back to
/// `redact_unparseable_destination`, a MASKING-ONLY scrub that keeps no host.
pub fn redact_destination(raw: &str) -> String {
    if let Ok(mut url) = url::Url::parse(raw) {
        let scheme = format!("{}://", url.scheme());
        let Some(host) = url.host_str() else {
            return REDACTED.to_string();
        };
        let host = host.to_string();
        // The port only when the URL named one explicitly (`Url::port` is
        // `None` for a non-special scheme with no explicit port — rtmp/srt
        // have no registered default).
        let port = url.port().map(|p| format!(":{p}")).unwrap_or_default();
        let had_tail = !url.path().is_empty() && url.path() != "/"
            || url.query().is_some()
            || url.fragment().is_some();
        // Drop the userinfo so it can never be re-serialized.
        let _ = url.set_username("");
        let _ = url.set_password(None);
        if had_tail {
            format!("{scheme}{host}{port}/{REDACTED}")
        } else {
            format!("{scheme}{host}{port}")
        }
    } else {
        redact_unparseable_destination(raw)
    }
}

/// The masking-only fallback for [`redact_destination`] on a URL the parser
/// rejects (spec §9.5 documented exception, allowlisted by name in the guard).
/// The whole authority — userinfo AND host — collapses to the single mask
/// token, and anything after the authority (path, query, fragment) collapses
/// to `/<redacted>`: neither the host nor the tail is reconstructed from the
/// raw text, so an RTMP stream key (`rtmp://host/app/KEY`) or an SRT
/// `streamid` in the path/query can never reach a log line even when the URL as
/// a whole did not parse.
fn redact_unparseable_destination(raw: &str) -> String {
    let Some(scheme_end) = raw.find("://") else {
        return REDACTED.to_string();
    };
    let scheme = &raw[..scheme_end + 3];
    let after_scheme = &raw[scheme_end + 3..];
    let authority_end = after_scheme
        .find(['/', '?', '#'])
        .unwrap_or(after_scheme.len());
    let had_tail = authority_end < after_scheme.len();
    if had_tail {
        format!("{scheme}{REDACTED}/{REDACTED}")
    } else {
        format!("{scheme}{REDACTED}")
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

    /// Guard (I4): the two masking-only fallbacks must not reconstruct a
    /// secret from the raw text. Asserted by BEHAVIOUR, not by a source-string
    /// match: a respelling of the host extraction (e.g. `rfind('@')` rather
    /// than `rsplit_once('@')`) must still fail this.
    #[test]
    fn masking_fallbacks_do_not_reconstruct_a_secret() {
        // `redact_unparseable_userinfo` masks the credential and keeps host +
        // path; the credential must never survive.
        let userinfo = redact_unparseable_userinfo("rtsp://user:secretpass@host/s");
        assert!(
            !userinfo.contains("user") && !userinfo.contains("secretpass"),
            "the userinfo fallback must not echo the credential: {userinfo}"
        );
        assert_eq!(userinfo, "rtsp://***@host/s");

        // `redact_unparseable_destination` echoes NEITHER the host NOR the
        // path/query — a stream key or token in the tail must not survive.
        let dest = redact_unparseable_destination("rtmp://host/app/STREAMKEY123?token=abc");
        for secret in ["host", "app", "STREAMKEY123", "token", "abc"] {
            assert!(
                !dest.contains(secret),
                "the destination fallback must not echo {secret}: {dest}"
            );
        }
        assert_eq!(dest, "rtmp://<redacted>/<redacted>");
        // No tail: still no host.
        assert_eq!(
            redact_unparseable_destination("rtsp://cam.local:554"),
            "rtsp://<redacted>"
        );
    }
}
