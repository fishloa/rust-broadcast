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
//! shared model `rtsp-runtime` and `hls_runtime::client::tokio_client`
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

/// A credential bound to the origin it was configured for (RFC 6454 §4:
/// scheme, host, port).
///
/// Pull sources fetch many URLs — playlist entries, segment URIs, manifest
/// `BaseURL`s — and any of them may be absolute and name another host. The
/// credential belongs to the configured source only, so it is attached to a
/// request only when the request's origin is exactly the source's origin
/// (issue #1142): a different host, port or scheme gets an anonymous request.
#[derive(Clone)]
pub(crate) struct ScopedCredentials {
    credentials: Credentials,
    origin: url::Origin,
}

impl ScopedCredentials {
    /// Whether `url` addresses the origin these credentials are bound to.
    fn applies_to(&self, url: &str) -> bool {
        Url::parse(url).is_ok_and(|u| u.origin() == self.origin)
    }
}

/// Manual `Debug`: only the origin renders, never the credential.
impl std::fmt::Debug for ScopedCredentials {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ScopedCredentials")
            .field("origin", &self.origin.ascii_serialization())
            .field("credentials", &"<redacted>")
            .finish()
    }
}

/// Binds `credentials` (if any) to the origin of `source_url` — the
/// configured source URL, userinfo already stripped.
pub(crate) fn scope_credentials(
    credentials: Option<Credentials>,
    source_url: &Url,
) -> Option<ScopedCredentials> {
    credentials.map(|credentials| ScopedCredentials {
        credentials,
        origin: source_url.origin(),
    })
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
    credentials: Option<&ScopedCredentials>,
) -> Result<Response> {
    // Credentials never leave the origin they were configured for.
    let credentials = credentials
        .filter(|scoped| scoped.applies_to(url))
        .map(|scoped| &scoped.credentials);
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
    // A challenge from a host a redirect led to is not answered with this
    // source's credentials.
    if !scoped_origin_matches(url, response.url()) {
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

/// Whether the response's final URL is on the same origin as the request URL
/// (i.e. no cross-origin redirect was followed).
fn scoped_origin_matches(request_url: &str, final_url: &Url) -> bool {
    Url::parse(request_url).is_ok_and(|u| u.origin() == final_url.origin())
}

#[cfg(test)]
mod tests {
    use super::*;

    use axum::Router;
    use axum::http::{HeaderMap, StatusCode as AxumStatus, header};
    use axum::response::{IntoResponse, Redirect};
    use axum::routing::get;
    use std::sync::{Arc, Mutex};

    /// What a loopback origin saw: one entry per request, the `Authorization`
    /// header value (or `None`).
    type Seen = Arc<Mutex<Vec<Option<String>>>>;

    fn note(seen: &Seen, headers: &HeaderMap) {
        seen.lock().unwrap().push(
            headers
                .get(header::AUTHORIZATION)
                .map(|v| v.to_str().unwrap().to_string()),
        );
    }

    /// Starts a loopback origin: `/start` redirects to `redirect_to` (if any),
    /// `/seg` answers 200, `/challenge` answers `401` with a Digest
    /// challenge. Returns its base URL and the record of what it saw.
    async fn origin(redirect_to: Option<String>) -> (String, Seen) {
        let seen: Seen = Arc::default();
        let (s1, s2, s3) = (seen.clone(), seen.clone(), seen.clone());
        let app = Router::new()
            .route(
                "/start",
                get(move |h: HeaderMap| async move {
                    note(&s1, &h);
                    match redirect_to {
                        Some(to) => Redirect::to(&to).into_response(),
                        None => AxumStatus::OK.into_response(),
                    }
                }),
            )
            .route(
                "/seg",
                get(move |h: HeaderMap| async move {
                    note(&s2, &h);
                    AxumStatus::OK
                }),
            )
            .route(
                "/challenge",
                get(move |h: HeaderMap| async move {
                    note(&s3, &h);
                    (
                        AxumStatus::UNAUTHORIZED,
                        [(
                            header::WWW_AUTHENTICATE,
                            r#"Digest realm="r", nonce="abc", qop="auth", algorithm=MD5"#,
                        )],
                    )
                }),
            );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });
        (format!("http://{addr}"), seen)
    }

    fn client() -> Client {
        Client::builder()
            .redirect(crate::source::redirect_policy())
            .build()
            .unwrap()
    }

    fn scoped(creds: Credentials, source_url: &str) -> Option<ScopedCredentials> {
        scope_credentials(Some(creds), &Url::parse(source_url).unwrap())
    }

    /// The configured credential reaches its own origin (positive control) but
    /// never a different host/port, whether that origin is reached by a
    /// redirect or named directly by an absolute URL in a playlist/manifest
    /// (#1142).
    ///
    /// Biting test: drop the `applies_to` filter in `authenticated_get` and
    /// the second origin records the `Authorization` header.
    #[tokio::test]
    async fn credentials_never_leave_the_configured_origin() {
        let (b_url, b_seen) = origin(None).await;
        let (a_url, a_seen) = origin(Some(format!("{b_url}/seg"))).await;
        let creds = scoped(Credentials::bearer("tok-secret"), &a_url);
        let c = client();

        // Same origin: the token is sent.
        authenticated_get(&c, &format!("{a_url}/seg"), creds.as_ref())
            .await
            .unwrap();
        assert_eq!(
            a_seen.lock().unwrap().as_slice(),
            [Some("Bearer tok-secret".to_string())]
        );

        // An absolute URL naming another origin: an anonymous request.
        authenticated_get(&c, &format!("{b_url}/seg"), creds.as_ref())
            .await
            .unwrap();
        assert_eq!(b_seen.lock().unwrap().as_slice(), [None]);

        // A redirect from the configured origin to another origin: the first
        // hop carries the token, the redirected hop does not.
        a_seen.lock().unwrap().clear();
        b_seen.lock().unwrap().clear();
        authenticated_get(&c, &format!("{a_url}/start"), creds.as_ref())
            .await
            .unwrap();
        assert_eq!(
            a_seen.lock().unwrap().as_slice(),
            [Some("Bearer tok-secret".to_string())]
        );
        assert_eq!(b_seen.lock().unwrap().as_slice(), [None]);
    }

    /// Basic credentials are scoped the same way.
    #[tokio::test]
    async fn basic_credentials_are_scoped_to_their_origin() {
        let (b_url, b_seen) = origin(None).await;
        let (a_url, _a_seen) = origin(None).await;
        let creds = scoped(Credentials::new("user", "hunter2-password"), &a_url);
        // `Credentials::new` is Digest-flavoured, so use an explicit Basic.
        let creds = creds.map(|mut s| {
            s.credentials = Credentials::Basic {
                username: "user".into(),
                password: "hunter2-password".into(),
            };
            s
        });
        authenticated_get(&client(), &format!("{b_url}/seg"), creds.as_ref())
            .await
            .unwrap();
        assert_eq!(b_seen.lock().unwrap().as_slice(), [None]);
    }

    /// A Digest challenge from an origin a redirect led to is not answered:
    /// the credential is never used against it, and nothing is resent.
    ///
    /// Biting test: drop the `scoped_origin_matches` check and the original
    /// origin receives a second, `Authorization`-bearing request.
    #[tokio::test]
    async fn a_digest_challenge_from_a_redirect_target_is_not_answered() {
        let (b_url, b_seen) = origin(None).await;
        let (a_url, a_seen) = origin(Some(format!("{b_url}/challenge"))).await;
        let creds = scoped(Credentials::new("user", "hunter2-password"), &a_url);
        let response = authenticated_get(&client(), &format!("{a_url}/start"), creds.as_ref())
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
        assert_eq!(a_seen.lock().unwrap().as_slice(), [None]);
        assert_eq!(b_seen.lock().unwrap().as_slice(), [None]);
    }

    /// A Digest challenge from the configured origin itself is still answered
    /// (positive control for the test above).
    #[tokio::test]
    async fn a_digest_challenge_from_the_configured_origin_is_answered() {
        let (a_url, a_seen) = origin(None).await;
        let creds = scoped(Credentials::new("user", "hunter2-password"), &a_url);
        authenticated_get(&client(), &format!("{a_url}/challenge"), creds.as_ref())
            .await
            .unwrap();
        let seen = a_seen.lock().unwrap();
        assert_eq!(seen.len(), 2, "{seen:?}");
        assert_eq!(seen[0], None);
        assert!(
            seen[1].as_deref().is_some_and(|v| v.starts_with("Digest ")),
            "{seen:?}"
        );
    }

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
