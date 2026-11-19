//! Secret hygiene and the #204 label pair: `Debug` names its type and never a
//! secret, `name()`/`Display` agree with the spec tokens, and a Bearer
//! challenge keeps every RFC 6750 §3 parameter.
use broadcast_auth::{
    AuthState, Authenticator, BearerError, Credentials, Error, HttpAuthError, Scheme,
    parse_challenges,
};
use url::Url;

#[test]
fn scheme_labels_match_their_wire_tokens() {
    assert_eq!(Scheme::Basic.name(), "Basic");
    assert_eq!(Scheme::Digest.name(), "Digest");
    assert_eq!(Scheme::Bearer.name(), "Bearer");
    let other = Scheme::Other("Negotiate".to_string());
    assert_eq!(other.name(), "Negotiate");
    for scheme in [Scheme::Basic, Scheme::Digest, Scheme::Bearer, other] {
        assert_eq!(scheme.to_string(), scheme.name());
    }
}

#[test]
fn bearer_error_labels_match_the_rfc6750_codes() {
    assert_eq!(BearerError::InvalidRequest.name(), "invalid_request");
    assert_eq!(BearerError::InvalidToken.name(), "invalid_token");
    assert_eq!(BearerError::InsufficientScope.name(), "insufficient_scope");
    let other = BearerError::Other("expired".to_string());
    assert_eq!(other.name(), "expired");
    for error in [
        BearerError::InvalidRequest,
        BearerError::InvalidToken,
        BearerError::InsufficientScope,
        other,
    ] {
        assert_eq!(error.to_string(), error.name());
    }
}

#[test]
fn http_auth_error_displays_its_diagnostic() {
    assert_eq!(HttpAuthError("boom".to_string()).to_string(), "boom");
}

#[test]
fn bearer_challenge_keeps_error_uri() {
    let parsed = parse_challenges(&["Bearer error=\"invalid_token\", error_uri=\"https://x/e\""]);
    assert!(parsed.errors.is_empty(), "{:?}", parsed.errors);
    assert_eq!(parsed.challenges.len(), 1);
    assert_eq!(parsed.challenges[0].error, Some(BearerError::InvalidToken));
    assert_eq!(
        parsed.challenges[0].error_uri.as_deref(),
        Some("https://x/e")
    );
}

#[test]
fn a_duplicate_error_uri_is_refused() {
    let parsed =
        parse_challenges(&["Bearer error_uri=\"https://x/a\", error_uri=\"https://x/b\""]);
    assert!(parsed.challenges.is_empty());
    assert_eq!(
        parsed.errors,
        vec![Error::DuplicateParameter {
            name: "error_uri".to_string()
        }]
    );
}

#[test]
fn authenticator_debug_names_the_scheme_and_never_the_secret() {
    let basic = Authenticator::from_challenge(
        "Basic realm=\"r\"",
        Credentials::Basic {
            username: "admin".to_string(),
            password: "hunter2".to_string(),
        },
    )
    .unwrap();
    let text = format!("{basic:?}");
    assert!(text.contains("Authenticator"), "{text}");
    assert!(text.contains("Basic"), "{text}");
    assert!(!text.contains("hunter2"), "{text}");

    let bearer = Authenticator::from_challenge("", Credentials::bearer("tok-value")).unwrap();
    let text = format!("{bearer:?}");
    assert!(text.contains("Authenticator"), "{text}");
    assert!(text.contains("Bearer"), "{text}");
    assert!(!text.contains("tok-value"), "{text}");
}

#[test]
fn auth_state_debug_names_itself_and_hides_secrets() {
    let mut state = AuthState::new(
        Url::parse("http://h/").unwrap(),
        vec![("X-Api-Key".to_string(), "header-secret".to_string())],
    );
    state.set_credentials(Credentials::Digest {
        username: "admin".to_string(),
        password: "pw-secret".to_string(),
    });
    let text = format!("{state:?}");
    assert!(text.contains("AuthState"), "{text}");
    assert!(!text.contains("pw-secret"), "{text}");
    assert!(!text.contains("header-secret"), "{text}");
}
