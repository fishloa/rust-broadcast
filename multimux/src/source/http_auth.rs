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

use broadcast_auth::Credentials;
use broadcast_auth::{AuthDecision, AuthState};
use reqwest::header::HeaderName;
use reqwest::{Client, RequestBuilder, Response, StatusCode};
use url::Url;

use crate::error::{MultimuxError, Result};

/// Extracts [`Credentials`] from `url`'s userinfo, if any.
///
/// Returns `Ok(None)` when the URL carries no username — the common case,
/// meaning "connect with no auth".
pub(crate) fn credentials_from_url(url: &Url) -> Result<Option<Credentials>> {
    broadcast_auth::credentials_from_url(url).map_err(|e| userinfo_error(e, url))
}

fn userinfo_error(e: broadcast_auth::Error, url: &Url) -> MultimuxError {
    match e {
        broadcast_auth::Error::UserinfoNotRemovable(component) => MultimuxError::Connect {
            reason: format!(
                "failed to strip {component} from URL {}",
                crate::redact::redact_url(url.as_str())
            ),
        },
        other => MultimuxError::Auth {
            reason: other.to_string(),
        },
    }
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

/// Binds `credentials` (if any) to the origin of `source_url`, the configured source URL with
/// userinfo already stripped. A credential is sent only to that origin (RFC 6454 §4, issue #1142);
/// any other host, port or scheme gets an anonymous request.
pub(crate) fn scope_credentials(
    credentials: Option<Credentials>,
    source_url: &Url,
) -> Option<AuthState> {
    credentials.map(|credentials| {
        let mut state = AuthState::new(source_url.clone(), Vec::new());
        state.set_credentials(credentials);
        state
    })
}

/// Returns a copy of `url` with its userinfo (username/password) removed, so
/// it is safe to use in the actual request line and in any error/log
/// message.
pub(crate) fn strip_userinfo(url: &Url) -> Result<Url> {
    broadcast_auth::strip_userinfo(url).map_err(|e| userinfo_error(e, url))
}

/// Performs `GET url`, answering a `401` challenge with `auth`'s credentials (if any) before
/// returning.
///
/// Each call works on its own clone of `auth`, so concurrent fetches never share a Digest nonce
/// and every request does its own one-shot replay.
///
/// - `Credentials::Basic`/`Bearer` are pre-applied on the first request. A `401` for either is
///   returned as-is (wrong credentials, not a missing challenge).
/// - `Credentials::Digest` sends the first request bare; on a `401` from the request's own origin
///   it replays once with the answer [`AuthState::handle_unauthorized`] prepares.
/// - `auth: None`, a request to another origin, or any non-`401` first response is returned
///   as-is.
pub(crate) async fn authenticated_get(
    client: &Client,
    url: &str,
    auth: Option<&AuthState>,
) -> Result<Response> {
    authenticated_get_with(client, url, auth, &[]).await
}

/// [`authenticated_get`] with extra request headers (a `Range`, a conditional
/// `If-None-Match`) carried on both the first request and a Digest retry.
pub(crate) async fn authenticated_get_with(
    client: &Client,
    url: &str,
    auth: Option<&AuthState>,
    extra_headers: &[(HeaderName, String)],
) -> Result<Response> {
    let mut state = auth.cloned();
    let had_auth = false;
    if let Some(state) = &mut state {
        state.preauthorize_basic();
    }
    let parsed = Url::parse(url).ok();
    let response = build_request(client, url, parsed.as_ref(), state.as_mut(), extra_headers)
        .send()
        .await
        .map_err(|e| MultimuxError::Connect {
            reason: format!("GET {}: {e}", crate::redact::redact_url(url)),
        })?;

    let (Some(state), Some(parsed)) = (state.as_mut(), parsed.as_ref()) else {
        return Ok(response);
    };
    let own_origin = state.origin().origin();
    // Feed response cookies on every response, regardless of status.
    let response_headers: Vec<(String, String)> = response
        .headers()
        .iter()
        .filter_map(|(k, v)| Some((k.as_str().to_owned(), v.to_str().ok()?.to_owned())))
        .collect();
    state.receive_cookies(parsed, &response_headers, 0);

    let status = response.status().as_u16();
    let is_challenge = status == StatusCode::UNAUTHORIZED.as_u16()
        || status == StatusCode::PROXY_AUTHENTICATION_REQUIRED.as_u16();
    if !is_challenge || parsed.origin() != own_origin || response.url().origin() != own_origin {
        return Ok(response);
    }
    // Only attempt a credential replay when credentials are configured for
    // this origin; otherwise leave the challenge response to the caller.
    if state.credentials().is_none() {
        return Ok(response);
    }
    let decision = state.handle_unauthorized(status, &response_headers, had_auth, false);
    if !matches!(decision, AuthDecision::Replay | AuthDecision::ReplayStale) {
        return Ok(response);
    }
    build_request(client, url, Some(parsed), Some(state), extra_headers)
        .send()
        .await
        .map_err(|e| MultimuxError::Connect {
            reason: format!("GET {} (auth retry): {e}", crate::redact::redact_url(url)),
        })
}

fn build_request(
    client: &Client,
    url: &str,
    parsed: Option<&Url>,
    state: Option<&mut AuthState>,
    extra_headers: &[(HeaderName, String)],
) -> RequestBuilder {
    let mut req = client.get(url);
    if let (Some(state), Some(parsed)) = (state, parsed) {
        for (name, value) in state.request_headers(parsed, 0) {
            req = req.header(name, value);
        }
    }
    for (name, value) in extra_headers {
        req = req.header(name, value);
    }
    req
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
            .redirect(crate::source::redirect_policy(true))
            .build()
            .unwrap()
    }

    fn scoped(creds: Credentials, source_url: &str) -> Option<AuthState> {
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
        let creds = scoped(
            Credentials::Basic {
                username: "user".into(),
                password: "hunter2-password".into(),
            },
            &a_url,
        );
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

    #[tokio::test]
    async fn golden_basic_same_origin_sends_authorization() {
        let (a_url, a_seen) = origin(None).await;
        let creds = scoped(
            Credentials::Basic {
                username: "user".into(),
                password: "pass".into(),
            },
            &a_url,
        );
        let resp = authenticated_get(&client(), &format!("{a_url}/seg"), creds.as_ref())
            .await
            .unwrap();
        assert!(resp.status().is_success());
        let seen = a_seen.lock().unwrap();
        assert_eq!(seen.as_slice(), [Some("Basic dXNlcjpwYXNz".to_string())]);
    }

    #[tokio::test]
    async fn golden_bearer_token_sends_authorization() {
        let (a_url, a_seen) = origin(None).await;
        let creds = scoped(
            Credentials::Bearer {
                token: "tok-secret".into(),
            },
            &a_url,
        );
        let resp = authenticated_get(&client(), &format!("{a_url}/seg"), creds.as_ref())
            .await
            .unwrap();
        assert!(resp.status().is_success());
        let seen = a_seen.lock().unwrap();
        assert_eq!(seen.as_slice(), [Some("Bearer tok-secret".to_string())]);
    }

    #[tokio::test]
    async fn golden_digest_first_bare_then_replay_with_digest() {
        let (a_url, a_seen) = origin(None).await;
        // Use /challenge which returns 401 with a Digest challenge.
        let creds = scoped(Credentials::new("user", "pass"), &a_url);
        let _ = authenticated_get(&client(), &format!("{a_url}/challenge"), creds.as_ref()).await;
        let seen = a_seen.lock().unwrap();
        assert_eq!(seen.len(), 2);
        assert_eq!(seen[0], None);
        assert!(seen[1].as_deref().unwrap_or("").starts_with("Digest "));
    }

    #[tokio::test]
    async fn golden_cross_origin_gets_no_authorization() {
        let (b_url, b_seen) = origin(None).await;
        let (a_url, _a_seen) = origin(None).await;
        let creds = scoped(
            Credentials::Bearer {
                token: "tok-secret".into(),
            },
            &a_url,
        );
        authenticated_get(&client(), &format!("{b_url}/seg"), creds.as_ref())
            .await
            .unwrap();
        assert_eq!(b_seen.lock().unwrap().as_slice(), [None]);
    }

    #[tokio::test]
    async fn golden_second_401_stops_after_one_replay() {
        let (a_url, a_seen) = origin(None).await;
        let creds = scoped(Credentials::new("user", "pass"), &a_url);
        // /challenge always 401; current code resends once then returns the 401.
        let resp = authenticated_get(&client(), &format!("{a_url}/challenge"), creds.as_ref())
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
        let seen = a_seen.lock().unwrap();
        assert_eq!(seen.len(), 2);
    }

    /// Mock origin that verifies the RFC 7616 Digest response against the
    /// password "hunter2-password" and answers 200 only on a match.
    async fn verifying_origin() -> (String, Seen) {
        let seen: Seen = Arc::default();
        let s = seen.clone();
        fn sha256_hex(s: &str) -> String {
            use sha2::{Digest, Sha256};
            Sha256::digest(s.as_bytes())
                .iter()
                .map(|b| format!("{:02x}", b))
                .collect()
        }
        let app = Router::new().route(
            "/v",
            get(move |h: HeaderMap| async move {
                note(&s, &h);
                let auth = h
                    .get(header::AUTHORIZATION)
                    .and_then(|v| v.to_str().ok())
                    .unwrap_or("");
                if auth.is_empty() {
                    return (
                        AxumStatus::UNAUTHORIZED,
                        [(
                            header::WWW_AUTHENTICATE,
                            r#"Digest realm="r", qop="auth", nonce="n1", algorithm=SHA-256"#,
                        )],
                    )
                        .into_response();
                }
                let mut username = String::new();
                let mut realm = String::new();
                let mut nonce = String::new();
                let mut uri = String::new();
                let mut nc = String::new();
                let mut cnonce = String::new();
                let mut qop = String::new();
                let mut response = String::new();
                let body = auth.strip_prefix("Digest ").unwrap_or(auth);
                for part in body.split(',') {
                    let part = part.trim();
                    if let Some((k, v)) = part.split_once('=') {
                        let val = v.trim().trim_matches('"').to_string();
                        match k.trim() {
                            "username" => username = val,
                            "realm" => realm = val,
                            "nonce" => nonce = val,
                            "uri" => uri = val,
                            "nc" => nc = val,
                            "cnonce" => cnonce = val,
                            "qop" => qop = val,
                            "response" => response = val,
                            _ => {}
                        }
                    }
                }
                let password = "hunter2-password";
                let ha1 = sha256_hex(&format!("{username}:{realm}:{password}"));
                let ha2 = sha256_hex(&format!("GET:{uri}"));
                let expected = sha256_hex(&format!("{ha1}:{nonce}:{nc}:{cnonce}:{qop}:{ha2}"));
                if expected == response {
                    AxumStatus::OK.into_response()
                } else {
                    (
                        AxumStatus::UNAUTHORIZED,
                        [(
                            header::WWW_AUTHENTICATE,
                            r#"Digest realm="r", qop="auth", nonce="n1", algorithm=SHA-256"#,
                        )],
                    )
                        .into_response()
                }
            }),
        );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });
        (format!("http://{addr}"), seen)
    }

    async fn proxy_origin() -> (String, Seen) {
        let seen: Seen = Arc::default();
        let s = seen.clone();
        let app = Router::new().route(
            "/p",
            get(move |h: HeaderMap| async move {
                note(&s, &h);
                (
                    AxumStatus::PROXY_AUTHENTICATION_REQUIRED,
                    [(
                        header::PROXY_AUTHENTICATE,
                        r#"Digest realm="r", qop="auth", nonce="n1", algorithm=SHA-256"#,
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

    #[tokio::test]
    async fn digest_response_verified_right_password_gets_200() {
        let (url, seen) = verifying_origin().await;
        let creds = scoped(Credentials::new("user", "hunter2-password"), &url);
        let resp = authenticated_get(&client(), &format!("{url}/v"), creds.as_ref())
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let seen = seen.lock().unwrap();
        assert_eq!(seen.len(), 2);
    }

    #[tokio::test]
    async fn digest_wrong_password_terminal_401_after_one_replay() {
        let (url, seen) = verifying_origin().await;
        let creds = scoped(Credentials::new("user", "wrong"), &url);
        let resp = authenticated_get(&client(), &format!("{url}/v"), creds.as_ref())
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
        let seen = seen.lock().unwrap();
        assert_eq!(seen.len(), 2);
    }

    #[tokio::test]
    async fn proxy_auth_407_is_not_handled_today() {
        let (url, seen) = proxy_origin().await;
        let creds = scoped(Credentials::new("user", "hunter2-password"), &url);
        let resp = authenticated_get(&client(), &format!("{url}/p"), creds.as_ref())
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::PROXY_AUTHENTICATION_REQUIRED);
        let seen = seen.lock().unwrap();
        assert_eq!(seen.len(), 2);
    }

    #[tokio::test]
    async fn same_origin_redirect_reapplies_credentials() {
        let seen: Seen = Arc::default();
        let s1 = seen.clone();
        let s2 = seen.clone();
        let app = Router::new()
            .route(
                "/start",
                get(move |h: HeaderMap| async move {
                    note(&s1, &h);
                    Redirect::to("/seg").into_response()
                }),
            )
            .route(
                "/seg",
                get(move |h: HeaderMap| async move {
                    note(&s2, &h);
                    AxumStatus::OK
                }),
            );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });
        let url = format!("http://{addr}");
        let creds = scoped(Credentials::bearer("tok-secret"), &url);
        let c = client();
        authenticated_get(&c, &format!("{url}/start"), creds.as_ref())
            .await
            .unwrap();
        let seen = seen.lock().unwrap();
        assert_eq!(
            seen.as_slice(),
            [
                Some("Bearer tok-secret".to_string()),
                Some("Bearer tok-secret".to_string())
            ]
        );
    }

    #[tokio::test]
    async fn stale_challenge_retries_once() {
        let seen: Seen = Arc::default();
        let s = seen.clone();
        let app = Router::new().route(
            "/s",
            get(move |h: HeaderMap| async move {
                note(&s, &h);
                (
                    AxumStatus::UNAUTHORIZED,
                    [(
                        header::WWW_AUTHENTICATE,
                        r#"Digest realm="r", qop="auth", nonce="n2", algorithm=SHA-256, stale=true"#,
                    )],
                )
            }),
        );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });
        let url = format!("http://{addr}");
        let creds = scoped(Credentials::new("user", "hunter2-password"), &url);
        let resp = authenticated_get(&client(), &format!("{url}/s"), creds.as_ref())
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
        let seen = seen.lock().unwrap();
        assert_eq!(seen.len(), 2);
    }
}
