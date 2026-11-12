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
use broadcast_client::auth::AuthState;
use reqwest::header::HeaderName;
use reqwest::{Client, Response, StatusCode};
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

/// Builds an origin-scoped [`AuthState`] for `credentials` (if any), bound to
/// `source_url` (RFC 6454 §4 origin scoping).
pub(crate) fn auth_state(credentials: Option<Credentials>, source_url: &Url) -> Option<AuthState> {
    credentials.map(|creds| {
        let mut state = AuthState::new(source_url.clone(), Vec::new());
        state.set_credentials(creds);
        state
    })
}

/// Returns a copy of `url` with its userinfo (username/password) removed, so
/// it is safe to use in the actual request line and in any error/log
/// message.
pub(crate) fn strip_userinfo(url: &Url) -> Result<Url> {
    broadcast_auth::strip_userinfo(url).map_err(|e| userinfo_error(e, url))
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
fn basic_base64(input: &str) -> String {
    const TABLE: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let bytes = input.as_bytes();
    let mut out = String::new();
    let mut i = 0;
    while i + 3 <= bytes.len() {
        let n = ((bytes[i] as u32) << 16) | ((bytes[i + 1] as u32) << 8) | (bytes[i + 2] as u32);
        out.push(TABLE[((n >> 18) & 63) as usize] as char);
        out.push(TABLE[((n >> 12) & 63) as usize] as char);
        out.push(TABLE[((n >> 6) & 63) as usize] as char);
        out.push(TABLE[(n & 63) as usize] as char);
        i += 3;
    }
    let rem = bytes.len() - i;
    if rem == 1 {
        let n = (bytes[i] as u32) << 16;
        out.push(TABLE[((n >> 18) & 63) as usize] as char);
        out.push(TABLE[((n >> 12) & 63) as usize] as char);
        out.push('=');
        out.push('=');
    } else if rem == 2 {
        let n = ((bytes[i] as u32) << 16) | ((bytes[i + 1] as u32) << 8);
        out.push(TABLE[((n >> 18) & 63) as usize] as char);
        out.push(TABLE[((n >> 12) & 63) as usize] as char);
        out.push(TABLE[((n >> 6) & 63) as usize] as char);
        out.push('=');
    }
    out
}

pub(crate) async fn authenticated_get(
    client: &Client,
    url: &str,
    auth: Option<&mut AuthState>,
) -> Result<Response> {
    authenticated_get_with(client, url, auth, &[]).await
}

/// [`authenticated_get`] with extra request headers (a `Range`, a conditional
/// `If-None-Match`) carried on both the first request and a Digest retry.
pub(crate) async fn authenticated_get_with(
    client: &Client,
    url: &str,
    auth: Option<&mut AuthState>,
    extra_headers: &[(HeaderName, String)],
) -> Result<Response> {
    let mut auth = auth;

    // First request.
    let mut had_auth = false;
    let mut headers: Vec<(String, String)> = if let Some(state) = auth.as_mut() {
        let mut h = state.request_headers(
            &match Url::parse(url) {
                Ok(u) => u,
                Err(_) => {
                    return Err(MultimuxError::Connect {
                        reason: format!("bad url {url}"),
                    });
                }
            },
            0,
        );
        had_auth = h.iter().any(|(k, _)| {
            k.eq_ignore_ascii_case("authorization") || k.eq_ignore_ascii_case("proxy-authorization")
        });
        if !had_auth
            && let Some(Credentials::Basic { username, password }) = state.credentials()
            && Url::parse(url).ok().map(|u| u.origin()) == Some(state.origin().clone().origin())
        {
            let token = basic_base64(&format!("{username}:{password}"));
            h.push(("Authorization".to_string(), format!("Basic {token}")));
            had_auth = true;
        }
        h
    } else {
        Vec::new()
    };
    for (name, value) in extra_headers {
        headers.push((name.to_string(), value.clone()));
    }

    let build = |c: &Client, headers: &[(String, String)]| {
        let mut req = c.get(url);
        for (k, v) in headers {
            req = req.header(k, v);
        }
        req
    };

    let mut response =
        build(client, &headers)
            .send()
            .await
            .map_err(|e| MultimuxError::Connect {
                reason: format!("GET {}: {e}", crate::redact::redact_url(url)),
            })?;

    // One Digest replay at most.
    if let Some(state) = auth.as_mut()
        && (response.status() == StatusCode::UNAUTHORIZED
            || response.status() == StatusCode::FORBIDDEN
            || response.status() == StatusCode::from_u16(407).unwrap())
    {
        // cross-origin redirect: do not answer
        let cross_origin =
            Url::parse(url).ok().map(|u| u.origin()) != Some(response.url().origin());
        if !cross_origin {
            let status = response.status().as_u16();
            let resp_headers: Vec<(String, String)> = response
                .headers()
                .iter()
                .filter_map(|(n, v)| Some((n.to_string(), v.to_str().ok()?.to_string())))
                .collect();
            let decision = state.handle_unauthorized(status, &resp_headers, had_auth, false);
            match decision {
                broadcast_client::auth::AuthDecision::Replay => {
                    let mut h2 = state.request_headers(&Url::parse(url).unwrap(), 0);
                    for (name, value) in extra_headers {
                        h2.push((name.to_string(), value.clone()));
                    }
                    response =
                        build(client, &h2)
                            .send()
                            .await
                            .map_err(|e| MultimuxError::Connect {
                                reason: format!(
                                    "GET {} (auth retry): {e}",
                                    crate::redact::redact_url(url)
                                ),
                            })?;
                }
                broadcast_client::auth::AuthDecision::ReplayStale => {
                    let mut h2 = state.request_headers(&Url::parse(url).unwrap(), 0);
                    for (name, value) in extra_headers {
                        h2.push((name.to_string(), value.clone()));
                    }
                    response =
                        build(client, &h2)
                            .send()
                            .await
                            .map_err(|e| MultimuxError::Connect {
                                reason: format!(
                                    "GET {} (auth retry): {e}",
                                    crate::redact::redact_url(url)
                                ),
                            })?;
                }
                broadcast_client::auth::AuthDecision::Wait(_) => {
                    return Ok(response);
                }
                _ => {}
            }
        }
    }
    Ok(response)
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
        auth_state(Some(creds), &Url::parse(source_url).unwrap())
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
        let mut creds = scoped(
            Credentials::Bearer {
                token: "tok-secret".into(),
            },
            &a_url,
        );
        let c = client();

        // Same origin: the token is sent.
        authenticated_get(&c, &format!("{a_url}/seg"), creds.as_mut())
            .await
            .unwrap();
        assert_eq!(
            a_seen.lock().unwrap().as_slice(),
            [Some("Bearer tok-secret".to_string())]
        );

        // An absolute URL naming another origin: an anonymous request.
        authenticated_get(&c, &format!("{b_url}/seg"), creds.as_mut())
            .await
            .unwrap();
        assert_eq!(b_seen.lock().unwrap().as_slice(), [None]);

        // A redirect from the configured origin to another origin: the first
        // hop carries the token, the redirected hop does not.
        a_seen.lock().unwrap().clear();
        b_seen.lock().unwrap().clear();
        authenticated_get(&c, &format!("{a_url}/start"), creds.as_mut())
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
        let mut creds = auth_state(
            Some(Credentials::Basic {
                username: "user".into(),
                password: "hunter2-password".into(),
            }),
            &Url::parse(&a_url).unwrap(),
        );
        authenticated_get(&client(), &format!("{b_url}/seg"), creds.as_mut())
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
        let mut creds = scoped(Credentials::new("user", "hunter2-password"), &a_url);
        let response = authenticated_get(&client(), &format!("{a_url}/start"), creds.as_mut())
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
        let mut creds = scoped(Credentials::new("user", "hunter2-password"), &a_url);
        authenticated_get(&client(), &format!("{a_url}/challenge"), creds.as_mut())
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
        let mut creds = scoped(
            Credentials::Basic {
                username: "user".into(),
                password: "pass".into(),
            },
            &a_url,
        );
        let resp = authenticated_get(&client(), &format!("{a_url}/seg"), creds.as_mut())
            .await
            .unwrap();
        assert!(resp.status().is_success());
        let seen = a_seen.lock().unwrap();
        assert_eq!(seen.as_slice(), [Some("Basic dXNlcjpwYXNz".to_string())]);
    }

    #[tokio::test]
    async fn golden_bearer_token_sends_authorization() {
        let (a_url, a_seen) = origin(None).await;
        let mut creds = scoped(
            Credentials::Bearer {
                token: "tok-secret".into(),
            },
            &a_url,
        );
        let resp = authenticated_get(&client(), &format!("{a_url}/seg"), creds.as_mut())
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
        let mut creds = scoped(Credentials::new("user", "pass"), &a_url);
        let _ = authenticated_get(&client(), &format!("{a_url}/challenge"), creds.as_mut()).await;
        let seen = a_seen.lock().unwrap();
        assert_eq!(seen.len(), 2);
        assert_eq!(seen[0], None);
        assert!(seen[1].as_deref().unwrap_or("").starts_with("Digest "));
    }

    #[tokio::test]
    async fn golden_cross_origin_gets_no_authorization() {
        let (b_url, b_seen) = origin(None).await;
        let (a_url, _a_seen) = origin(None).await;
        let mut creds = scoped(
            Credentials::Bearer {
                token: "tok-secret".into(),
            },
            &a_url,
        );
        authenticated_get(&client(), &format!("{b_url}/seg"), creds.as_mut())
            .await
            .unwrap();
        assert_eq!(b_seen.lock().unwrap().as_slice(), [None]);
    }

    #[tokio::test]
    async fn golden_second_401_stops_after_one_replay() {
        let (a_url, a_seen) = origin(None).await;
        let mut creds = scoped(Credentials::new("user", "pass"), &a_url);
        // /challenge always 401; current code resends once then returns the 401.
        let resp = authenticated_get(&client(), &format!("{a_url}/challenge"), creds.as_mut())
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
        let seen = a_seen.lock().unwrap();
        assert_eq!(seen.len(), 2);
    }
}
