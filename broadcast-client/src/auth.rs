//! Origin-scoped credentials and challenge handling.

use alloc::string::String;
use alloc::vec::Vec;

use url::Url;

use broadcast_auth::challenge::parse_challenges;
use broadcast_auth::{Authenticator, Credentials};

use crate::cookies::CookieJar;
use crate::headers;
use crate::{Event, Status};

const COOKIE: &str = "Cookie";
const AUTHORIZATION: &str = "Authorization";
const PROXY_AUTHORIZATION: &str = "Proxy-Authorization";
const WWW_AUTHENTICATE: &str = "www-authenticate";
const PROXY_AUTHENTICATE: &str = "proxy-authenticate";
const STATUS_PROXY_AUTH_REQUIRED: u16 = 407;
const CHALLENGE_SEPARATOR: &str = ", ";

/// Whether a request's headers carry credentials for the origin or for a proxy.
pub(crate) fn carries_credentials(headers: &[(String, String)]) -> bool {
    headers.iter().any(|(k, _)| {
        k.eq_ignore_ascii_case(AUTHORIZATION) || k.eq_ignore_ascii_case(PROXY_AUTHORIZATION)
    })
}

fn authenticator_for(lines: &[String], creds: &Credentials) -> Option<Authenticator> {
    let combined = lines.join(CHALLENGE_SEPARATOR);
    Authenticator::from_challenge(&combined, creds.clone())
        .ok()
        .or_else(|| {
            lines
                .iter()
                .find_map(|line| Authenticator::from_challenge(line, creds.clone()).ok())
        })
}

/// Result of handling a 401/407.
#[non_exhaustive]
pub enum AuthDecision {
    /// Replay the request with credentials.
    Replay,
    /// Replay the request after a `stale=true` re-challenge; the caller must mark the request
    /// lineage so a second stale answer is rejected instead of replayed again.
    ReplayStale,
    /// Wait for credentials.
    Wait(Vec<Event>),
}

/// Origin-scoped auth state.
pub struct AuthState {
    origin: Url,
    extra_headers: Vec<(String, String)>,
    credentials: Option<Credentials>,
    authenticator: Option<Authenticator>,
    last_challenge: Option<String>,
    proxy: bool,
    waiting: bool,
    jar: CookieJar,
}

impl AuthState {
    /// Create auth state.
    pub fn new(origin: Url, extra_headers: Vec<(String, String)>) -> Self {
        let jar = CookieJar::new(&origin);
        Self {
            origin,
            extra_headers,
            credentials: None,
            authenticator: None,
            last_challenge: None,
            proxy: false,
            waiting: false,
            jar,
        }
    }

    /// Seed the jar with cookies sent to the origin on path `/`.
    pub fn seed_cookies(&mut self, cookies: &[(String, String)]) {
        self.jar.seed(cookies);
    }

    /// Store the `Set-Cookie` lines of a response; ignored unless the response is from the origin.
    pub fn receive_cookies(&mut self, url: &Url, headers: &[(String, String)], now_us: u64) {
        self.jar.receive(url, headers, now_us);
    }

    /// Live cookies as `(name, value)` for persistence.
    pub fn cookies(&self, now_us: u64) -> Vec<(String, String)> {
        self.jar.export(now_us)
    }

    /// Origin.
    pub fn origin(&self) -> &Url {
        &self.origin
    }

    /// The configured credentials.
    pub(crate) fn credentials(&self) -> Option<&Credentials> {
        self.credentials.as_ref()
    }

    /// Waiting flag.
    pub fn is_waiting(&self) -> bool {
        self.waiting
    }

    /// Set credentials; returns whether it was waiting.
    pub fn set_credentials(&mut self, creds: Credentials) -> bool {
        let was_waiting = self.waiting;
        self.authenticator = match (&creds, &self.last_challenge) {
            (Credentials::Bearer { .. }, _) | (_, None) => None,
            (_, Some(line)) => Authenticator::from_challenge(line, creds.clone()).ok(),
        };
        self.credentials = Some(creds);
        self.waiting = false;
        was_waiting
    }

    /// Request headers for a URL.
    pub fn request_headers(&mut self, url: &Url, now_us: u64) -> Vec<(String, String)> {
        let same_origin = url.origin() == self.origin.origin();
        let mut headers: Vec<(String, String)> = if same_origin {
            self.extra_headers
                .iter()
                .filter(|(k, _)| !headers::is_reserved(k))
                .cloned()
                .collect()
        } else {
            Vec::new()
        };
        if let Some(cookie) = self.jar.header_for(url, now_us) {
            headers.push((COOKIE.to_string(), cookie));
        }
        let name = if self.proxy {
            PROXY_AUTHORIZATION
        } else {
            AUTHORIZATION
        };
        if let Some(creds) = self.credentials.as_ref().filter(|_| same_origin) {
            match creds {
                Credentials::Bearer { token } => {
                    headers.push((name.to_string(), format!("Bearer {token}")));
                }
                Credentials::Basic { .. } | Credentials::Digest { .. } => {
                    if let Some(auth) = &mut self.authenticator {
                        let target = &url[url::Position::BeforePath..url::Position::AfterQuery];
                        let ctx = broadcast_auth::RequestContext::new("GET", target);
                        if let Ok(val) = auth.authorization(&ctx) {
                            headers.push((name.to_string(), val));
                        }
                    }
                }
                _ => {}
            }
        }
        headers
    }

    /// Handle 401/407. `stale_replayed` is whether this request lineage already got one stale
    /// replay.
    pub fn handle_unauthorized(
        &mut self,
        status: u16,
        headers: &[(String, String)],
        had_auth: bool,
        stale_replayed: bool,
    ) -> AuthDecision {
        self.proxy = status == STATUS_PROXY_AUTH_REQUIRED;
        let preferred = if self.proxy {
            PROXY_AUTHENTICATE
        } else {
            WWW_AUTHENTICATE
        };
        let select = |wanted: &[&str]| -> Vec<String> {
            headers
                .iter()
                .filter(|(k, _)| wanted.iter().any(|w| k.eq_ignore_ascii_case(w)))
                .map(|(_, v)| v.clone())
                .collect()
        };
        let mut challenge_lines = select(&[preferred]);
        if challenge_lines.is_empty() {
            challenge_lines = select(&[WWW_AUTHENTICATE, PROXY_AUTHENTICATE]);
        }

        let challenges: Vec<broadcast_auth::Challenge> = challenge_lines
            .iter()
            .flat_map(|l| parse_challenges(&[l.as_str()]).challenges)
            .collect();

        if !challenge_lines.is_empty() {
            self.last_challenge = Some(challenge_lines.join(CHALLENGE_SEPARATOR));
        }

        let stale = challenges.iter().any(|c| {
            c.other_params
                .iter()
                .any(|(k, v)| k.eq_ignore_ascii_case("stale") && v.eq_ignore_ascii_case("true"))
        });

        // No credentials configured: ask the host for them.
        if self.credentials.is_none() {
            let chosen = challenges
                .iter()
                .find(|c| c.resource_metadata.is_some())
                .or_else(|| challenges.first());
            let scheme = chosen
                .map(|c| c.scheme.clone())
                .unwrap_or(broadcast_auth::Scheme::Bearer);
            let realm = chosen.and_then(|c| c.realm.clone());
            let resource_metadata = chosen.and_then(|c| c.resource_metadata.clone());
            self.waiting = true;
            self.authenticator = None;
            return AuthDecision::Wait(vec![
                Event::AuthRequired {
                    scheme,
                    realm,
                    origin: self.origin.clone(),
                    resource_metadata,
                },
                Event::Status(Status::WaitingForCredentials),
            ]);
        }

        // Credentials present but rejected (not a stale re-challenge, or a second one): wait.
        if had_auth && (!stale || stale_replayed) {
            self.waiting = true;
            self.authenticator = None;
            return AuthDecision::Wait(vec![
                Event::AuthRejected { status },
                Event::Status(Status::WaitingForCredentials),
            ]);
        }

        // First (unauthenticated) 401 or stale re-challenge: build authenticator
        // and replay once.
        if let Some(creds) = self.credentials.as_ref()
            && let Some(auth) = authenticator_for(&challenge_lines, creds)
        {
            self.authenticator = Some(auth);
            self.waiting = false;
            return if had_auth && stale {
                AuthDecision::ReplayStale
            } else {
                AuthDecision::Replay
            };
        }
        self.waiting = true;
        self.authenticator = None;
        AuthDecision::Wait(vec![
            Event::AuthRejected { status },
            Event::Status(Status::WaitingForCredentials),
        ])
    }
}

impl core::fmt::Debug for AuthState {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("AuthState")
            .field("origin", &self.origin)
            .field("waiting", &self.waiting)
            .field("has_credentials", &self.credentials.is_some())
            .field("has_authenticator", &self.authenticator.is_some())
            .field("jar", &self.jar)
            .finish_non_exhaustive()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use url::Url;

    #[test]
    fn origin_scoped_request_headers() {
        let origin = Url::parse("http://example/live.ts").unwrap();
        let mut auth = AuthState::new(origin.clone(), vec![]);
        auth.set_credentials(Credentials::bearer("secret-token"));
        let same = Url::parse("http://example/other.ts").unwrap();
        let h = auth.request_headers(&same, 0);
        assert!(h.iter().any(|(k, v)| k.eq_ignore_ascii_case("authorization") && v == "Bearer secret-token"));
        let other = Url::parse("http://other.example/live.ts").unwrap();
        let h2 = auth.request_headers(&other, 0);
        assert!(
            !h2.iter()
                .any(|(k, _)| k.eq_ignore_ascii_case("authorization"))
        );
    }

    #[test]
    fn auth_state_debug_redacts() {
        let origin = Url::parse("http://example/live.ts").unwrap();
        let mut auth = AuthState::new(origin, vec![]);
        auth.set_credentials(Credentials::bearer("supersecret-token"));
        let dbg = format!("{:?}", auth);
        assert!(!dbg.contains("supersecret-token"));
    }

    #[test]
    fn extra_headers_never_override_auth_state() {
        let origin = Url::parse("http://example/live.ts").unwrap();
        let mut auth = AuthState::new(
            origin.clone(),
            vec![
                ("Cookie".to_string(), "injected=1".to_string()),
                ("authorization".to_string(), "Bearer evil".to_string()),
                ("X-Keep".to_string(), "1".to_string()),
            ],
        );
        auth.seed_cookies(&[("sid".to_string(), "real".to_string())]);
        auth.set_credentials(Credentials::bearer("tok"));
        let headers = auth.request_headers(&origin, 0);
        let named = |n: &str| -> Vec<&str> {
            headers
                .iter()
                .filter(|(k, _)| k.eq_ignore_ascii_case(n))
                .map(|(_, v)| v.as_str())
                .collect()
        };
        assert_eq!(named("cookie"), vec!["sid=real"]);
        assert_eq!(named("authorization"), vec!["Bearer tok"]);
        assert_eq!(named("x-keep"), vec!["1"]);

        let mut no_creds = AuthState::new(
            origin.clone(),
            vec![("Authorization".to_string(), "Bearer evil".to_string())],
        );
        assert!(
            no_creds
                .request_headers(&origin, 0)
                .iter()
                .all(|(k, _)| !k.eq_ignore_ascii_case("authorization"))
        );
    }
}
