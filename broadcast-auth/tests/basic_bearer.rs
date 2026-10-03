use base64::Engine as _;
use broadcast_auth::{
    AuthResult, Authenticator, Credentials, Error, RequestContext, Verifier, respond,
};

fn verify(v: &Verifier, header: &str) -> AuthResult {
    let headers = [("authorization", header)];
    v.verify(&RequestContext::new("GET", "/x").with_headers(&headers))
}

fn basic_verifier() -> Verifier {
    Verifier::new(
        Credentials::Basic {
            username: "admin".into(),
            password: "pa:ss".into(),
        },
        "r",
    )
}

fn b64(s: &[u8]) -> String {
    base64::engine::general_purpose::STANDARD.encode(s)
}

/// Defect: header injection through the client's Bearer value.
#[test]
fn bearer_token_with_crlf_is_refused_by_the_client() {
    for bad in ["tok\r\nX-Evil: 1", "tok\nx", "tok\u{0}", "tök"] {
        let mut auth = Authenticator::from_challenge("", Credentials::bearer(bad)).unwrap();
        let err = auth
            .authorization(&RequestContext::new("GET", "/x"))
            .unwrap_err();
        assert!(matches!(err, Error::InvalidBearerToken), "{bad:?}: {err:?}");
    }
    assert_eq!(
        respond(
            "",
            &RequestContext::new("GET", "/x"),
            Credentials::bearer("abc.DEF-1_2~+/=")
        )
        .unwrap(),
        "Bearer abc.DEF-1_2~+/="
    );
}

#[test]
fn basic_scheme_is_case_insensitive_and_password_may_contain_a_colon() {
    let v = basic_verifier();
    let payload = b64(b"admin:pa:ss");
    for scheme in ["Basic", "basic", "BASIC"] {
        assert_eq!(
            verify(&v, &format!("{scheme} {payload}")),
            AuthResult::Ok,
            "{scheme}"
        );
    }
    assert_eq!(verify(&v, &format!("Basic   {payload}")), AuthResult::Ok);
    assert_eq!(
        verify(&v, &format!("Basic {}", b64(b"admin:wrong"))),
        AuthResult::Unauthorized
    );
    assert_eq!(
        verify(&v, &format!("Basic {}", b64(b"nobody:pa:ss"))),
        AuthResult::Unauthorized
    );
}

#[test]
fn basic_garbage_is_unauthorized_not_a_panic() {
    let v = basic_verifier();
    let non_utf8 = format!("Basic {}", b64(&[0xff, 0xfe, b':', 0x80]));
    for h in [
        "Basic",
        "Basic ",
        "Basic !!!",
        "Basic YWRtaW4=",  // no colon
        non_utf8.as_str(), // not UTF-8
        "Bearer abc",
        "Digest x=1",
        "",
    ] {
        assert_eq!(verify(&v, h), AuthResult::Unauthorized, "{h:?}");
    }
}

#[test]
fn bearer_comparison_trims_and_is_exact() {
    let v = Verifier::new(Credentials::bearer("tok-123"), "r");
    assert_eq!(verify(&v, "Bearer tok-123"), AuthResult::Ok);
    assert_eq!(verify(&v, "bearer   tok-123"), AuthResult::Ok);
    assert_eq!(verify(&v, "Bearer tok-123 "), AuthResult::Ok);
    assert_eq!(verify(&v, "Bearer tok-1234"), AuthResult::Unauthorized);
    assert_eq!(verify(&v, "Bearer tok-12"), AuthResult::Unauthorized);
    assert_eq!(verify(&v, "Bearer"), AuthResult::Unauthorized);
}

/// A configured token with surrounding spaces still verifies (both sides are
/// compared trimmed, as before the headers migration).
#[test]
fn a_configured_token_with_surrounding_spaces_verifies() {
    let v = Verifier::new(Credentials::bearer("  tok-1 "), "r");
    assert_eq!(verify(&v, "Bearer tok-1"), AuthResult::Ok);
    assert_eq!(verify(&v, "Bearer   tok-1  "), AuthResult::Ok);
    assert_eq!(verify(&v, "Bearer tok-2"), AuthResult::Unauthorized);
}
