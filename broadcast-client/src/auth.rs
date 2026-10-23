//! Origin-scoped credentials and challenge handling.

use alloc::string::String;
use alloc::vec::Vec;

use url::Url;

use broadcast_auth::challenge::parse_challenges;
use broadcast_auth::{Authenticator, Credentials};

use crate::{Event, Status};

/// Result of handling a 401/407.
#[non_exhaustive]
pub enum AuthDecision {
    /// Replay the request with credentials.
    Replay,
    /// Wait for credentials.
    Wait(Vec<Event>),
}

/// Origin-scoped auth state.
pub struct AuthState {
    origin: Url,
    extra_headers: Vec<(String, String)>,
    credentials: Option<Credentials>,
    authenticator: Option<Authenticator>,
    waiting: bool,
}

impl AuthState {
    /// Create auth state.
    pub fn new(origin: Url, extra_headers: Vec<(String, String)>) -> Self {
        Self {
            origin,
            extra_headers,
            credentials: None,
            authenticator: None,
            waiting: false,
        }
    }

    /// Origin.
    pub fn origin(&self) -> &Url {
        &self.origin
    }

    /// Waiting flag.
    pub fn is_waiting(&self) -> bool {
        self.waiting
    }

    /// Set credentials; returns whether it was waiting.
    pub fn set_credentials(&mut self, creds: Credentials) -> bool {
        let was_waiting = self.waiting;
        self.credentials = Some(creds);
        self.authenticator = None;
        self.waiting = false;
        was_waiting
    }

    /// Request headers for a URL.
    pub fn request_headers(&mut self, url: &Url) -> Vec<(String, String)> {
        let mut headers = self.extra_headers.clone();
        if url.origin() == self.origin.origin()
            && let Some(creds) = &self.credentials
        {
            match creds {
                    Credentials::Bearer { token } => {
                        headers.push(("Authorization".to_string(), format!("Bearer {token}")));
                    }
                    Credentials::Basic { .. } | Credentials::Digest { .. } => {
                        if let Some(auth) = &mut self.authenticator {
                            let ctx = broadcast_auth::RequestContext::new("GET", url.as_str());
                            if let Ok(val) = auth.authorization(&ctx) {
                                headers.push(("Authorization".to_string(), val));
                            }
                        }
                    }
                    _ => {}
                }
        }
        headers
    }

    /// Handle 401/407.
    pub fn handle_unauthorized(
        &mut self,
        status: u16,
        headers: &[(String, String)],
        had_auth: bool,
    ) -> AuthDecision {
        let challenge_headers: Vec<(String, String)> = headers
            .iter()
            .filter(|(k, _)| {
                k.eq_ignore_ascii_case("www-authenticate")
                    || k.eq_ignore_ascii_case("proxy-authenticate")
            })
            .cloned()
            .collect();

        let challenge_lines: Vec<String> =
            challenge_headers.iter().map(|(_, v)| v.clone()).collect();

        let challenges: Vec<broadcast_auth::Challenge> = challenge_lines
            .iter()
            .flat_map(|l| parse_challenges(&[l.as_str()]).unwrap_or_default())
            .collect();

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

        // Credentials present but rejected (not a stale re-challenge): wait.
        if had_auth && !stale {
            self.waiting = true;
            self.authenticator = None;
            return AuthDecision::Wait(vec![
                Event::AuthRejected { status },
                Event::Status(Status::WaitingForCredentials),
            ]);
        }

        // First (unauthenticated) 401 or stale re-challenge: build authenticator
        // and replay once.
        if let Some(line) = challenge_lines.first()
            && let Some(creds) = self.credentials.clone()
            && let Ok(auth) = Authenticator::from_challenge(line, creds)
        {
            self.authenticator = Some(auth);
            self.waiting = false;
            return AuthDecision::Replay;
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
            .finish_non_exhaustive()
    }
}
