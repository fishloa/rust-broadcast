use broadcast_auth::{Credentials, Verifier};
use http_auth::ChallengeParser;

fn parse_one(header: &str) -> http_auth::ChallengeRef<'_> {
    let mut all = ChallengeParser::new(header)
        .collect::<Result<Vec<_>, _>>()
        .unwrap_or_else(|e| panic!("{header:?} is not a well-formed challenge: {e}"));
    assert_eq!(all.len(), 1, "{header:?}");
    all.remove(0)
}

fn realm_of(c: &http_auth::ChallengeRef<'_>) -> String {
    c.params
        .iter()
        .find(|(k, _)| *k == "realm")
        .unwrap()
        .1
        .to_unescaped()
}

fn verifiers(realm: &str) -> [(&'static str, Verifier); 2] {
    [
        (
            "Basic",
            Verifier::new(
                Credentials::Basic {
                    username: "u".into(),
                    password: "p".into(),
                },
                realm,
            ),
        ),
        (
            "Digest",
            Verifier::new(
                Credentials::Digest {
                    username: "u".into(),
                    password: "p".into(),
                },
                realm,
            ),
        ),
    ]
}

#[test]
fn crlf_and_quote_in_the_realm_cannot_split_or_break_the_header() {
    for (scheme, v) in verifiers("x\"\r\nSet-Cookie: a=b") {
        let header = v.challenge();
        assert!(
            !header.contains('\r') && !header.contains('\n'),
            "{header:?}"
        );
        let c = parse_one(&header);
        assert_eq!(c.scheme, scheme);
        assert_eq!(realm_of(&c), "x\"Set-Cookie: a=b");
    }
}

#[test]
fn backslash_comma_and_space_round_trip() {
    for (_, v) in verifiers(r#"Region, East \ "Beta""#) {
        assert_eq!(
            realm_of(&parse_one(&v.challenge())),
            r#"Region, East \ "Beta""#
        );
    }
}

#[test]
fn an_ordinary_realm_is_rendered_exactly_as_before() {
    let [(_, basic), (_, digest)] = verifiers("cameras");
    assert_eq!(basic.challenge(), "Basic realm=\"cameras\"");
    assert!(
        digest
            .challenge()
            .starts_with("Digest realm=\"cameras\", nonce=\"")
    );
}

/// HTAB is legal `qdtext` (RFC 9110 §5.6.4): it must survive rendering.
#[test]
fn a_tab_in_the_realm_is_kept() {
    for (_, v) in verifiers("east\twest") {
        let header = v.challenge();
        assert!(header.contains("east\twest"), "{header:?}");
        assert_eq!(realm_of(&parse_one(&header)), "east\twest");
    }
}

/// The realm a client echoes back is the one the server compares, even when
/// the configured realm carried CR/LF (dropped from the rendering): such a
/// verifier must still authenticate.
#[test]
fn a_realm_with_crlf_still_authenticates_a_digest_client() {
    use broadcast_auth::{AuthResult, RequestContext, respond};
    let v = Verifier::new(
        Credentials::Digest {
            username: "u".into(),
            password: "p".into(),
        },
        "a\r\nb\tc",
    );
    let h = respond(
        &v.challenge(),
        &RequestContext::new("GET", "/x"),
        Credentials::new("u", "p"),
    )
    .unwrap();
    let headers = [("authorization", h.as_str())];
    assert_eq!(
        v.verify(&RequestContext::new("GET", "/x").with_headers(&headers)),
        AuthResult::Ok
    );
}
