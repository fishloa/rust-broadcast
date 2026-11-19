//! The 401/407 decision table of `AuthState::handle_unauthorized` (RFC 7235
//! §11.6.1, RFC 7617 §2, RFC 7616 §3.4): which challenge header is read, when
//! the request is replayed, when it is replayed after a `stale=true`
//! re-challenge, and when the host is asked for credentials instead. Driven
//! through the public `AuthState` surface only.
use broadcast_auth::{
    AuthDecision, AuthEvent, AuthState, Credentials, Scheme, carries_credentials,
};
use broadcast_common::stage::Timestamp;
use http::{HeaderMap, HeaderValue, StatusCode};
use url::Url;

const DIGEST: &str = "Digest realm=\"cameras\", nonce=\"dcd98b7102dd2f0e8b11d0f600bfb0c093\", \
                      qop=\"auth\", algorithm=MD5";

fn origin() -> Url {
    Url::parse("http://cameras.test/live").unwrap()
}

fn map(pairs: &[(&str, &str)]) -> HeaderMap {
    let mut headers = HeaderMap::new();
    for (name, value) in pairs {
        headers.append(
            name.parse::<http::HeaderName>().unwrap(),
            HeaderValue::from_str(value).unwrap(),
        );
    }
    headers
}

fn basic_credentials() -> Credentials {
    Credentials::Basic {
        username: "admin".to_string(),
        password: "12345".to_string(),
    }
}

fn digest_credentials() -> Credentials {
    Credentials::Digest {
        username: "admin".to_string(),
        password: "12345".to_string(),
    }
}

fn state_with(creds: Option<Credentials>) -> AuthState {
    let mut state = AuthState::new(origin(), Vec::new());
    if let Some(creds) = creds {
        state.set_credentials(creds);
    }
    state
}

fn assert_rejected(decision: AuthDecision, status: StatusCode) {
    match decision {
        AuthDecision::Wait(events) => assert!(
            events.contains(&AuthEvent::AuthRejected { status }),
            "{events:?}"
        ),
        other => panic!("expected Wait(AuthRejected), got {other:?}"),
    }
}

#[test]
fn no_credentials_401_asks_the_host_for_them() {
    let mut state = state_with(None);
    let headers = map(&[("www-authenticate", "Basic realm=\"r\"")]);
    match state.handle_unauthorized(StatusCode::UNAUTHORIZED, &headers, false, false) {
        AuthDecision::Wait(events) => {
            assert_eq!(events.len(), 2, "{events:?}");
            match &events[0] {
                AuthEvent::AuthRequired(data) => {
                    assert_eq!(data.scheme, Scheme::Basic);
                    assert_eq!(data.realm.as_deref(), Some("r"));
                    assert_eq!(data.origin, origin());
                }
                other => panic!("expected AuthRequired, got {other:?}"),
            }
            assert!(events.contains(&AuthEvent::StatusWaitingForCredentials));
        }
        other => panic!("expected Wait, got {other:?}"),
    }
    assert!(state.is_waiting());
    assert!(!state.is_authenticated());
}

#[test]
fn first_basic_401_replays_and_a_second_one_is_rejected() {
    let creds = basic_credentials();
    let mut state = state_with(Some(creds.clone()));
    assert_eq!(state.credentials(), Some(&creds));
    let headers = map(&[("www-authenticate", "Basic realm=\"r\"")]);
    assert!(matches!(
        state.handle_unauthorized(StatusCode::UNAUTHORIZED, &headers, false, false),
        AuthDecision::Replay
    ));
    assert!(state.is_authenticated());
    assert!(!state.is_waiting());
    assert_rejected(
        state.handle_unauthorized(StatusCode::UNAUTHORIZED, &headers, true, false),
        StatusCode::UNAUTHORIZED,
    );
    assert!(state.is_waiting());
    assert!(!state.is_authenticated());
}

#[test]
fn digest_stale_rechallenge_is_replayed_once() {
    for challenge in [
        format!("{DIGEST}, stale=true"),
        format!("{DIGEST}, stale=TRUE"),
        format!("{DIGEST}, Stale=true"),
    ] {
        let mut state = state_with(Some(digest_credentials()));
        let headers = map(&[("www-authenticate", challenge.as_str())]);
        assert!(
            matches!(
                state.handle_unauthorized(StatusCode::UNAUTHORIZED, &headers, true, false),
                AuthDecision::ReplayStale
            ),
            "{challenge}"
        );
        assert!(state.is_authenticated(), "{challenge}");
        assert_rejected(
            state.handle_unauthorized(StatusCode::UNAUTHORIZED, &headers, true, true),
            StatusCode::UNAUTHORIZED,
        );
    }
}

#[test]
fn stale_without_prior_auth_is_a_plain_replay() {
    let challenge = format!("{DIGEST}, stale=true");
    let mut state = state_with(Some(digest_credentials()));
    let headers = map(&[("www-authenticate", challenge.as_str())]);
    assert!(matches!(
        state.handle_unauthorized(StatusCode::UNAUTHORIZED, &headers, false, false),
        AuthDecision::Replay
    ));
    assert!(state.is_authenticated());
}

#[test]
fn a_non_stale_digest_rejection_waits() {
    let mut state = state_with(Some(digest_credentials()));
    let headers = map(&[("www-authenticate", DIGEST)]);
    assert_rejected(
        state.handle_unauthorized(StatusCode::UNAUTHORIZED, &headers, true, false),
        StatusCode::UNAUTHORIZED,
    );
    assert!(state.is_waiting());
    assert!(!state.is_authenticated());
}

#[test]
fn proxy_status_answers_on_the_proxy_header() {
    let mut state = state_with(Some(basic_credentials()));
    let headers = map(&[("proxy-authenticate", "Basic realm=\"p\"")]);
    assert!(matches!(
        state.handle_unauthorized(StatusCode::PROXY_AUTHENTICATION_REQUIRED, &headers, false, false),
        AuthDecision::Replay
    ));
    let sent = state.request_headers(&origin(), Timestamp::from_micros(0));
    assert!(
        sent.iter()
            .any(|(k, v)| k.eq_ignore_ascii_case("proxy-authorization")
                && v.starts_with("Basic ")),
        "{sent:?}"
    );
    assert!(
        sent.iter()
            .all(|(k, _)| !k.eq_ignore_ascii_case("authorization"))
    );
}

#[test]
fn origin_status_answers_on_the_authorization_header() {
    let mut state = state_with(Some(basic_credentials()));
    let headers = map(&[("www-authenticate", "Basic realm=\"r\"")]);
    assert!(matches!(
        state.handle_unauthorized(StatusCode::UNAUTHORIZED, &headers, false, false),
        AuthDecision::Replay
    ));
    let sent = state.request_headers(&origin(), Timestamp::from_micros(0));
    assert!(
        sent.iter()
            .any(|(k, v)| k.eq_ignore_ascii_case("authorization") && v.starts_with("Basic ")),
        "{sent:?}"
    );
    assert!(
        sent.iter()
            .all(|(k, _)| !k.eq_ignore_ascii_case("proxy-authorization"))
    );
}

#[test]
fn each_status_prefers_its_own_challenge_header() {
    let headers = map(&[
        ("www-authenticate", "Basic realm=\"www\""),
        ("proxy-authenticate", "Basic realm=\"proxy\""),
    ]);
    let mut state = state_with(None);
    match state.handle_unauthorized(StatusCode::UNAUTHORIZED, &headers, false, false) {
        AuthDecision::Wait(events) => match &events[0] {
            AuthEvent::AuthRequired(data) => assert_eq!(data.realm.as_deref(), Some("www")),
            other => panic!("expected AuthRequired, got {other:?}"),
        },
        other => panic!("expected Wait, got {other:?}"),
    }
    let mut state = state_with(None);
    match state.handle_unauthorized(
        StatusCode::PROXY_AUTHENTICATION_REQUIRED,
        &headers,
        false,
        false,
    ) {
        AuthDecision::Wait(events) => match &events[0] {
            AuthEvent::AuthRequired(data) => assert_eq!(data.realm.as_deref(), Some("proxy")),
            other => panic!("expected AuthRequired, got {other:?}"),
        },
        other => panic!("expected Wait, got {other:?}"),
    }
}

#[test]
fn carries_credentials_sees_either_header() {
    assert!(carries_credentials(&map(&[("authorization", "Basic x")])));
    assert!(carries_credentials(&map(&[(
        "proxy-authorization",
        "Basic x"
    )])));
    assert!(!carries_credentials(&map(&[("x-api-key", "k")])));
    assert!(!carries_credentials(&HeaderMap::new()));
}

#[test]
fn accessors_return_the_seeded_session() {
    let now = Timestamp::from_micros(1_700_000_000_000_000);
    let creds = Credentials::bearer("tok");
    let mut state = AuthState::new(origin(), Vec::new());
    state.seed_cookies(&[
        ("sid".to_string(), "abc".to_string()),
        ("theme".to_string(), "dark".to_string()),
    ]);
    assert_eq!(
        state.cookies(now),
        vec![
            ("sid".to_string(), "abc".to_string()),
            ("theme".to_string(), "dark".to_string()),
        ]
    );
    let watch = state.cookie_watch();
    assert_eq!(watch.cookies(now), state.cookies(now));
    state.set_credentials(creds.clone());
    assert_eq!(state.credentials(), Some(&creds));
    assert_eq!(state.origin(), &origin());
    assert!(!state.is_waiting());
    assert!(!state.is_authenticated());
}

#[test]
fn an_answer_without_a_challenge_keeps_the_remembered_one() {
    let mut state = state_with(None);
    let challenge = map(&[("www-authenticate", DIGEST)]);
    state.handle_unauthorized(StatusCode::UNAUTHORIZED, &challenge, false, false);
    state.set_credentials(digest_credentials());
    assert!(state.is_authenticated());
    let none = HeaderMap::new();
    assert_rejected(
        state.handle_unauthorized(StatusCode::UNAUTHORIZED, &none, true, false),
        StatusCode::UNAUTHORIZED,
    );
    state.set_credentials(digest_credentials());
    assert!(
        state.is_authenticated(),
        "a 401 with no challenge must not discard the remembered challenge"
    );
}
#[test]
fn stale_false_is_not_a_stale_rechallenge() {
    let challenge = format!("{DIGEST}, stale=false");
    let mut state = state_with(Some(digest_credentials()));
    let headers = map(&[("www-authenticate", challenge.as_str())]);
    assert_rejected(
        state.handle_unauthorized(StatusCode::UNAUTHORIZED, &headers, true, false),
        StatusCode::UNAUTHORIZED,
    );
    assert!(state.is_waiting());
}
