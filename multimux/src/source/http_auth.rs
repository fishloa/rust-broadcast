//! Shared HTTP auth glue for multimux's HTTP-based input sources —
//! [`crate::source::ts_http`] and [`crate::source::hls_pull`] (issue #663
//! P3c, `docs/superpowers/specs/2026-07-18-multimux-hub-design.md` §"Shared
//! auth layer").
//!
//! `reqwest` answers HTTP Basic (RFC 7617) and Bearer (RFC 6750) natively via
//! its own request-builder helpers, but not Digest (RFC 7616) — computing a
//! Digest response needs the server's `WWW-Authenticate` challenge first.
//! `authenticated_get` performs that one round-trip (send once; on a
//! `401`, read the challenge, compute the answer, resend once) so neither
//! HTTP source hand-rolls Digest: the actual challenge parsing and response
//! computation is entirely [`broadcast_auth`]'s (issue #663 P3b) — the same
//! shared model `rtsp-runtime` and `ll_hls_runtime::client::tokio_client`
//! use, so every credentialed client in the workspace answers a challenge
//! through the same code.
//!
//! Credentials come from the ingest URL's userinfo (RFC 3986 §3.2.1) —
//! `credentials_from_url`/`strip_userinfo` generalise
//! [`crate::source::rtsp`]'s own (RTSP-specific) userinfo handling to any
//! URL.

use broadcast_auth::{Authenticator, Credentials, RequestContext};
use reqwest::header::{AUTHORIZATION, WWW_AUTHENTICATE};
use reqwest::{Client, Response, StatusCode};
use url::Url;

use crate::error::{MultimuxError, Result};

/// Extracts [`Credentials`] from `url`'s userinfo, if any.
///
/// Returns `Ok(None)` when the URL carries no username — the common case,
/// meaning "connect with no auth". Mirrors
/// [`crate::source::rtsp`]'s `extract_credentials`.
pub(crate) fn credentials_from_url(url: &Url) -> Result<Option<Credentials>> {
    if url.username().is_empty() {
        return Ok(None);
    }
    let username = percent_decode(url.username())?;
    let password = match url.password() {
        Some(p) => percent_decode(p)?,
        None => String::new(),
    };
    Ok(Some(Credentials::new(username, password)))
}

/// Percent-decodes a URL userinfo component (RFC 3986 §2.1) to UTF-8.
///
/// The error message deliberately does **not** echo `s` — it is (part of) a
/// still percent-encoded credential, and even encoded it must never appear
/// in an error/log.
fn percent_decode(s: &str) -> Result<String> {
    percent_encoding::percent_decode_str(s)
        .decode_utf8()
        .map(|c| c.into_owned())
        .map_err(|e| MultimuxError::Auth {
            reason: format!("invalid percent-encoded userinfo: {e}"),
        })
}

/// Resolves the credentials a connection actually authenticates with:
/// config-supplied `config_auth` (e.g. [`crate::config::AuthSpec`], already
/// converted) takes precedence over `url_credentials` (whatever
/// [`credentials_from_url`] extracted from the ingest URL's own userinfo) —
/// config auth is the *only* way to supply a Bearer token (RFC 6750 has no
/// URL-userinfo form), so when both are given, config wins rather than the
/// URL silently shadowing it.
pub(crate) fn resolve_credentials(
    config_auth: Option<Credentials>,
    url_credentials: Option<Credentials>,
) -> Option<Credentials> {
    config_auth.or(url_credentials)
}

/// Returns a copy of `url` with its userinfo (username/password) removed, so
/// it is safe to use in the actual request line and in any error/log
/// message. Mirrors [`crate::source::rtsp`]'s `strip_userinfo`.
pub(crate) fn strip_userinfo(url: &Url) -> Result<Url> {
    let mut clean = url.clone();
    clean
        .set_username("")
        .map_err(|()| MultimuxError::Connect {
            reason: format!(
                "failed to strip username from URL {}",
                crate::redact::redact_url(url.as_str())
            ),
        })?;
    clean
        .set_password(None)
        .map_err(|()| MultimuxError::Connect {
            reason: format!(
                "failed to strip password from URL {}",
                crate::redact::redact_url(url.as_str())
            ),
        })?;
    Ok(clean)
}

/// Performs `GET url`, answering a `401` challenge with `credentials` (if
/// any) before returning.
///
/// - `Credentials::Basic`/`Bearer` are pre-applied on the very first request
///   (no round-trip needed — RFC 7617/RFC 6750). A `401` for either of these
///   is returned as-is (it means wrong credentials, not a missing
///   challenge).
/// - `Credentials::Digest` sends the first request bare; only if the server
///   answers `401` **with** a `WWW-Authenticate` header does this compute
///   the `Authorization` response (via [`Authenticator`]) and resend once. A
///   `401` with no such header, or a challenge/response failure, is surfaced
///   as [`MultimuxError::Auth`].
/// - `credentials: None`, or any non-`401` first response, is returned
///   as-is.
pub(crate) async fn authenticated_get(
    client: &Client,
    url: &str,
    credentials: Option<&Credentials>,
) -> Result<Response> {
    let build = |c: &Client| {
        let mut req = c.get(url);
        req = match credentials {
            Some(Credentials::Basic { username, password }) => {
                req.basic_auth(username, Some(password))
            }
            Some(Credentials::Bearer { token }) => req.bearer_auth(token),
            _ => req,
        };
        req
    };

    let response = build(client)
        .send()
        .await
        .map_err(|e| MultimuxError::Connect {
            reason: format!("GET {}: {e}", crate::redact::redact_url(url)),
        })?;

    let Some(creds @ Credentials::Digest { .. }) = credentials else {
        return Ok(response);
    };
    if response.status() != StatusCode::UNAUTHORIZED {
        return Ok(response);
    }
    let Some(challenge) = response
        .headers()
        .get(WWW_AUTHENTICATE)
        .and_then(|v| v.to_str().ok())
    else {
        return Ok(response);
    };

    let value = Authenticator::from_challenge(challenge, creds.clone())
        .and_then(|mut a| a.authorization(&RequestContext::new("GET", url)))
        .map_err(|e| MultimuxError::Auth {
            reason: format!("digest response: {e}"),
        })?;

    client
        .get(url)
        .header(AUTHORIZATION, value)
        .send()
        .await
        .map_err(|e| MultimuxError::Connect {
            reason: format!("GET {} (digest retry): {e}", crate::redact::redact_url(url)),
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn credentials_from_url_extracts_userinfo() {
        let url = Url::parse("http://user:secret@host/stream.ts").unwrap();
        let creds = credentials_from_url(&url).unwrap().unwrap();
        assert_eq!(
            creds,
            Credentials::Digest {
                username: "user".into(),
                password: "secret".into(),
            }
        );
    }

    #[test]
    fn credentials_from_url_is_none_without_userinfo() {
        let url = Url::parse("http://host/stream.ts").unwrap();
        assert!(credentials_from_url(&url).unwrap().is_none());
    }

    #[test]
    fn strip_userinfo_removes_credentials() {
        let url = Url::parse("http://user:secret@host/stream.ts").unwrap();
        let clean = strip_userinfo(&url).unwrap();
        assert_eq!(clean.as_str(), "http://host/stream.ts");
    }

    #[test]
    fn resolve_credentials_prefers_config_auth_over_url_userinfo() {
        let config_auth = Some(Credentials::bearer("tok"));
        let url_creds = Some(Credentials::new("user", "pass"));
        assert_eq!(
            resolve_credentials(config_auth.clone(), url_creds),
            config_auth
        );
    }

    #[test]
    fn resolve_credentials_falls_back_to_url_userinfo_without_config_auth() {
        let url_creds = Some(Credentials::new("user", "pass"));
        assert_eq!(resolve_credentials(None, url_creds.clone()), url_creds);
    }

    #[test]
    fn resolve_credentials_is_none_with_neither_source() {
        assert!(resolve_credentials(None, None).is_none());
    }
}
