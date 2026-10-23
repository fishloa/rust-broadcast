use broadcast_auth::challenge::{BearerError, Scheme, parse_challenges};
use broadcast_auth::{Authenticator, Credentials, Error, RequestContext};

#[test]
/// RFC 9110 §11.6.1.
fn rfc9110_two_challenges_one_line() {
    let cs = parse_challenges(&[
        r#"Basic realm="simple", Newauth realm="apps", type=1, title="Login to \"apps\"""#,
    ])
    .unwrap();
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
    let cs = parse_challenges(&[r#"Basic realm="a""#, r#"Bearer realm="b""#]).unwrap();
    assert_eq!(cs.len(), 2);
    assert_eq!(cs[0].scheme, Scheme::Basic);
    assert_eq!(cs[1].scheme, Scheme::Bearer);
}

#[test]
/// RFC 9728 §5.1.
fn rfc9728_5_1_example_resource_metadata() {
    let cs = parse_challenges(&[r#"Bearer resource_metadata="https://resource.example.com/.well-known/oauth-protected-resource""#])
        .unwrap();
    assert_eq!(
        cs[0].resource_metadata.as_ref().unwrap().as_str(),
        "https://resource.example.com/.well-known/oauth-protected-resource"
    );
}

#[test]
/// RFC 9728 §5.1.
fn rfc9728_resource_metadata_on_non_bearer_scheme() {
    let cs = parse_challenges(&[r#"DPoP resource_metadata="https://resource.example.com/.well-known/oauth-protected-resource""#])
        .unwrap();
    assert_eq!(cs[0].scheme, Scheme::Other("DPoP".to_string()));
    assert!(cs[0].resource_metadata.is_some());
}

#[test]
/// RFC 6750 §3.
fn rfc6750_expired_token_example() {
    let cs = parse_challenges(&[r#"Bearer realm="example", error="invalid_token", error_description="The access token expired""#])
        .unwrap();
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
    let cs = parse_challenges(&[r#"Bearer realm="example""#]).unwrap();
    assert_eq!(cs[0].realm.as_deref(), Some("example"));
    assert!(cs[0].error.is_none());
}

#[test]
/// RFC 6750 §3.
fn rfc6750_insufficient_scope_with_scope() {
    let cs = parse_challenges(&[
        r#"Bearer realm="example", error="insufficient_scope", scope="read write""#,
    ])
    .unwrap();
    assert_eq!(cs[0].scope.as_deref(), Some("read write"));
    assert_eq!(cs[0].error, Some(BearerError::InsufficientScope));
}

#[test]
/// RFC 9110 §11.1.
fn scheme_and_param_names_case_insensitive() {
    let cs = parse_challenges(&[r#"BEARER Realm="x", RESOURCE_METADATA="https://a/b""#]).unwrap();
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
    let cs = parse_challenges(&[r#"Newauth realm="apps", type=1, title="t""#]).unwrap();
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
    let err = parse_challenges(&[r#"Bearer realm="a", realm="b""#]).unwrap_err();
    assert!(matches!(err, Error::DuplicateParameter { name } if name == "realm"));
}

#[test]
/// RFC 6750 §3.
fn duplicate_scope_rejected() {
    let err = parse_challenges(&[r#"Bearer scope="a", scope="b""#]).unwrap_err();
    assert!(matches!(err, Error::DuplicateParameter { name } if name == "scope"));
}

#[test]
/// RFC 6750 §3.
fn duplicate_error_rejected() {
    let err = parse_challenges(&[r#"Bearer error="invalid_token", error="invalid_request""#])
        .unwrap_err();
    assert!(matches!(err, Error::DuplicateParameter { name } if name == "error"));
}

#[test]
/// RFC 9728 §5.1.
fn duplicate_resource_metadata_rejected() {
    let err = parse_challenges(&[
        r#"Bearer resource_metadata="https://a/b", resource_metadata="https://c/d""#,
    ])
    .unwrap_err();
    assert!(matches!(err, Error::DuplicateParameter { name } if name == "resource_metadata"));
}

#[test]
/// RFC 9728 §5.1.
fn resource_metadata_relative_or_garbage_rejected() {
    assert!(matches!(
        parse_challenges(&[r#"Bearer resource_metadata="/relative""#]).unwrap_err(),
        Error::InvalidResourceMetadata
    ));
    assert!(matches!(
        parse_challenges(&[r#"Bearer resource_metadata="""#]).unwrap_err(),
        Error::InvalidResourceMetadata
    ));
    assert!(matches!(
        parse_challenges(&[r#"Bearer resource_metadata="not a url""#]).unwrap_err(),
        Error::InvalidResourceMetadata
    ));
}

#[test]
/// RFC 9110 §11.6.1.
fn bearer_token68_style_value_without_params_is_not_an_error() {
    let cs = parse_challenges(&["Bearer"]).unwrap();
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
    assert!(parse_challenges(&[]).unwrap().is_empty());
    assert!(parse_challenges(&[""]).unwrap().is_empty());
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
