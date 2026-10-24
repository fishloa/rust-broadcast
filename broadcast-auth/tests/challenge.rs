use broadcast_auth::challenge::{BearerError, Challenge, Scheme, parse_challenges};
use broadcast_auth::{Authenticator, Credentials, Error, RequestContext};

fn ok(lines: &[&str]) -> Vec<Challenge> {
    let parsed = parse_challenges(lines);
    assert!(
        parsed.errors.is_empty(),
        "unexpected errors: {:?}",
        parsed.errors
    );
    parsed.challenges
}

fn first_err(lines: &[&str]) -> Error {
    let mut parsed = parse_challenges(lines);
    assert!(!parsed.errors.is_empty(), "expected an error");
    parsed.errors.remove(0)
}

#[test]
/// RFC 9110 §11.6.1.
fn rfc9110_two_challenges_one_line() {
    let cs =
        ok(&[r#"Basic realm="simple", Newauth realm="apps", type=1, title="Login to \"apps\"""#]);
    assert_eq!(cs.len(), 2);
    assert_eq!(cs[0].scheme, Scheme::Basic);
    assert_eq!(cs[0].realm.as_deref(), Some("simple"));
    assert!(cs[0].other_params.is_empty());
    assert_eq!(cs[1].scheme, Scheme::Other("Newauth".to_string()));
    assert_eq!(cs[1].realm.as_deref(), Some("apps"));
    assert_eq!(
        cs[1].other_params,
        vec![
            ("type".to_string(), "1".to_string()),
            ("title".to_string(), "Login to \"apps\"".to_string()),
        ]
    );
}

#[test]
/// RFC 9110 §11.6.1.
fn multiple_header_lines_preserve_order() {
    let cs = ok(&[r#"Basic realm="a""#, r#"Bearer realm="b""#]);
    assert_eq!(cs.len(), 2);
    assert_eq!(cs[0].scheme, Scheme::Basic);
    assert_eq!(cs[1].scheme, Scheme::Bearer);
}

#[test]
/// RFC 9728 §5.1.
fn rfc9728_5_1_example_resource_metadata() {
    let cs = ok(&[
        r#"Bearer resource_metadata="https://resource.example.com/.well-known/oauth-protected-resource""#,
    ]);
    assert_eq!(
        cs[0].resource_metadata.as_ref().unwrap().as_str(),
        "https://resource.example.com/.well-known/oauth-protected-resource"
    );
}

#[test]
/// RFC 9728 §5.1.
fn rfc9728_resource_metadata_on_non_bearer_scheme() {
    let cs = ok(&[
        r#"DPoP resource_metadata="https://resource.example.com/.well-known/oauth-protected-resource""#,
    ]);
    assert_eq!(cs[0].scheme, Scheme::Other("DPoP".to_string()));
    assert!(cs[0].resource_metadata.is_some());
}

#[test]
/// RFC 6750 §3.
fn rfc6750_expired_token_example() {
    let cs = ok(&[
        r#"Bearer realm="example", error="invalid_token", error_description="The access token expired""#,
    ]);
    assert_eq!(cs[0].realm.as_deref(), Some("example"));
    assert_eq!(cs[0].error, Some(BearerError::InvalidToken));
    assert_eq!(
        cs[0].error_description.as_deref(),
        Some("The access token expired")
    );
}

#[test]
/// RFC 6750 §3.
fn rfc6750_no_credentials_example() {
    let cs = ok(&[r#"Bearer realm="example""#]);
    assert_eq!(cs[0].realm.as_deref(), Some("example"));
    assert!(cs[0].error.is_none());
}

#[test]
/// RFC 6750 §3.
fn rfc6750_insufficient_scope_with_scope() {
    let cs = ok(&[r#"Bearer realm="example", error="insufficient_scope", scope="read write""#]);
    assert_eq!(cs[0].scope.as_deref(), Some("read write"));
    assert_eq!(cs[0].error, Some(BearerError::InsufficientScope));
}

#[test]
/// RFC 9110 §11.1.
fn scheme_and_param_names_case_insensitive() {
    let cs = ok(&[r#"BEARER Realm="x", RESOURCE_METADATA="https://a/b""#]);
    assert_eq!(cs[0].scheme, Scheme::Bearer);
    assert_eq!(cs[0].realm.as_deref(), Some("x"));
    assert_eq!(
        cs[0].resource_metadata.as_ref().unwrap().as_str(),
        "https://a/b"
    );
}

#[test]
/// RFC 9110 §11.6.1.
fn unknown_scheme_and_unknown_params_preserved() {
    let cs = ok(&[r#"Newauth realm="apps", type=1, title="t""#]);
    assert_eq!(cs[0].scheme, Scheme::Other("Newauth".to_string()));
    assert_eq!(
        cs[0].other_params,
        vec![
            ("type".to_string(), "1".to_string()),
            ("title".to_string(), "t".to_string())
        ]
    );
}

#[test]
/// RFC 6750 §3.
fn duplicate_realm_rejected() {
    let err = first_err(&[r#"Bearer realm="a", realm="b""#]);
    assert!(matches!(err, Error::DuplicateParameter { name } if name == "realm"));
}

#[test]
/// RFC 6750 §3.
fn duplicate_scope_rejected() {
    let err = first_err(&[r#"Bearer scope="a", scope="b""#]);
    assert!(matches!(err, Error::DuplicateParameter { name } if name == "scope"));
}

#[test]
/// RFC 6750 §3.
fn duplicate_error_rejected() {
    let err = first_err(&[r#"Bearer error="invalid_token", error="invalid_request""#]);
    assert!(matches!(err, Error::DuplicateParameter { name } if name == "error"));
}

#[test]
/// RFC 9728 §5.1.
fn duplicate_resource_metadata_rejected() {
    let err =
        first_err(&[r#"Bearer resource_metadata="https://a/b", resource_metadata="https://c/d""#]);
    assert!(matches!(err, Error::DuplicateParameter { name } if name == "resource_metadata"));
}

#[test]
/// RFC 9728 §5.1.
fn resource_metadata_relative_or_garbage_rejected() {
    assert!(matches!(
        first_err(&[r#"Bearer resource_metadata="/relative""#]),
        Error::InvalidResourceMetadata
    ));
    assert!(matches!(
        first_err(&[r#"Bearer resource_metadata="""#]),
        Error::InvalidResourceMetadata
    ));
    assert!(matches!(
        first_err(&[r#"Bearer resource_metadata="not a url""#]),
        Error::InvalidResourceMetadata
    ));
}

#[test]
/// RFC 9110 §11.6.1.
fn bearer_without_params_is_not_an_error() {
    let cs = ok(&["Bearer"]);
    assert_eq!(cs.len(), 1);
    assert_eq!(cs[0].scheme, Scheme::Bearer);
    assert!(cs[0].realm.is_none());
    assert!(cs[0].scope.is_none());
    assert!(cs[0].error.is_none());
    assert!(cs[0].error_description.is_none());
    assert!(cs[0].error_uri.is_none());
    assert!(cs[0].resource_metadata.is_none());
    assert!(cs[0].other_params.is_empty());
}

#[test]
/// RFC 9110 §11.6.1.
fn empty_input_returns_empty_vec() {
    assert!(ok(&[]).is_empty());
    assert!(ok(&[""]).is_empty());
}

#[test]
/// RFC 7616 §3.4.1.
fn basic_digest_challenges_still_drive_authenticator() {
    let challenge = r#"Digest realm="IP Camera", nonce="dcd98b7102dd2f0e8b11d0f600bfb0c093", qop="auth", algorithm=MD5"#;
    let mut auth =
        Authenticator::from_challenge(challenge, Credentials::new("admin", "12345")).unwrap();
    let ctx = RequestContext::new("DESCRIBE", "rtsp://camera.example.com/live");
    let value = auth.authorization(&ctx).unwrap();
    assert!(value.starts_with("Digest "));
}

#[test]
/// RFC 9728 §3: the protected resource metadata URL "MUST use the https scheme".
fn resource_metadata_non_https_rejected() {
    for bad in [
        "http://resource.example.com/.well-known/oauth-protected-resource",
        "file:///etc/passwd",
        "javascript:alert(1)",
    ] {
        let line = format!(r#"Bearer resource_metadata="{bad}""#);
        let parsed = parse_challenges(&[line.as_str()]);
        assert!(
            parsed.challenges.is_empty(),
            "{bad} must not yield a challenge"
        );
        assert!(
            matches!(parsed.errors.as_slice(), [Error::InvalidResourceMetadata]),
            "{bad} must be rejected, got {:?}",
            parsed.errors
        );
    }
}

#[test]
/// RFC 9110 §11.6.1: a recipient ignoring one bad challenge still honours the rest.
fn bad_challenge_does_not_discard_valid_ones() {
    let parsed = parse_challenges(&[
        r#"Basic realm="ok""#,
        r#"Bearer realm="a", realm="b""#,
        r#"Digest realm="d""#,
    ]);
    assert_eq!(parsed.challenges.len(), 2);
    assert_eq!(parsed.challenges[0].scheme, Scheme::Basic);
    assert_eq!(parsed.challenges[1].scheme, Scheme::Digest);
    assert!(matches!(
        parsed.errors.as_slice(),
        [Error::DuplicateParameter { name }] if name == "realm"
    ));
}

#[test]
/// RFC 9110 §11.6.1: the token68 form is unsupported by `http-auth`; it is
/// reported as an error and never discards challenges on other lines.
fn token68_form_is_reported_and_other_lines_survive() {
    let parsed = parse_challenges(&["Bearer abc123==", r#"Basic realm="ok""#]);
    assert_eq!(parsed.challenges.len(), 1);
    assert_eq!(parsed.challenges[0].scheme, Scheme::Basic);
    assert!(matches!(
        parsed.errors.as_slice(),
        [Error::ChallengeParse(_)]
    ));
}

#[test]
fn challenge_count_is_capped() {
    let lines: Vec<String> = (0..17).map(|i| format!(r#"Basic realm="r{i}""#)).collect();
    let refs: Vec<&str> = lines.iter().map(String::as_str).collect();
    let parsed = parse_challenges(&refs);
    assert_eq!(
        parsed.challenges.len(),
        broadcast_auth::challenge::MAX_CHALLENGES
    );
    assert!(matches!(
        parsed.errors.as_slice(),
        [Error::TooManyChallenges { max: 16 }]
    ));
    assert_eq!(parse_challenges(&refs[..16]).errors.len(), 0);
}
