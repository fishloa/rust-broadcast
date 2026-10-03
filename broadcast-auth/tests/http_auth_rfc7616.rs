//! SP2.4 verification gate: can `http-auth`'s `ChallengeParser` read RFC 7616
//! `Authorization: Digest` credentials, and does it round-trip our own
//! `WWW-Authenticate` challenge? Pins the facts the rest of the migration
//! depends on (see the plan, Task 1).

use broadcast_auth::{Credentials, Verifier};
use http_auth::{ChallengeParser, ChallengeRef};

/// RFC 7616 §3.9.1 (also http-auth's own `digest.rs` test vector), MD5.
const RFC7616_MD5: &str = "Digest username=\"Mufasa\", realm=\"http-auth@example.org\", \
    uri=\"/dir/index.html\", algorithm=MD5, \
    nonce=\"7ypf/xlj9XXwfDPEoM4URrv/xwf94BcCAzFZH4GiTo0v\", nc=00000001, \
    cnonce=\"f2/wE4q74E6zIJEtWaHKaf5wv/H5QzzpXusqGemxURZJ\", qop=auth, \
    response=\"8ca523f5e9506fed4657c9700eebdbec\", \
    opaque=\"FQhe/qaU925kfnzjCev0ciny7QMkPqMAFRtzCUYo5tdS\"";

/// RFC 7616 §3.9.1, SHA-256 variant.
const RFC7616_SHA256: &str = "Digest username=\"Mufasa\", realm=\"http-auth@example.org\", \
    uri=\"/dir/index.html\", algorithm=SHA-256, \
    nonce=\"7ypf/xlj9XXwfDPEoM4URrv/xwf94BcCAzFZH4GiTo0v\", nc=00000001, \
    cnonce=\"f2/wE4q74E6zIJEtWaHKaf5wv/H5QzzpXusqGemxURZJ\", qop=auth, \
    response=\"753927fa0e85d155564e2e272a28d1802ca10daf4496794697cf8db5856cb6c1\", \
    opaque=\"FQhe/qaU925kfnzjCev0ciny7QMkPqMAFRtzCUYo5tdS\"";

fn parse_one(header: &str) -> ChallengeRef<'_> {
    let mut all = ChallengeParser::new(header)
        .collect::<Result<Vec<_>, _>>()
        .expect("credentials must parse as one RFC 7235 auth-param list");
    assert_eq!(
        all.len(),
        1,
        "credentials are a single challenge-shaped list"
    );
    all.remove(0)
}

fn param(c: &ChallengeRef<'_>, name: &str) -> Option<String> {
    c.params
        .iter()
        .find(|(k, _)| k.eq_ignore_ascii_case(name))
        .map(|(_, v)| v.to_unescaped())
}

#[test]
fn rfc7616_md5_credentials_parse() {
    let c = parse_one(RFC7616_MD5);
    assert!(c.scheme.eq_ignore_ascii_case("Digest"));
    assert_eq!(c.params.len(), 10);
    assert_eq!(param(&c, "username").as_deref(), Some("Mufasa"));
    assert_eq!(param(&c, "realm").as_deref(), Some("http-auth@example.org"));
    assert_eq!(param(&c, "uri").as_deref(), Some("/dir/index.html"));
    assert_eq!(param(&c, "algorithm").as_deref(), Some("MD5"));
    assert_eq!(param(&c, "nc").as_deref(), Some("00000001"));
    assert_eq!(param(&c, "qop").as_deref(), Some("auth"));
    assert_eq!(
        param(&c, "response").as_deref(),
        Some("8ca523f5e9506fed4657c9700eebdbec")
    );
    assert_eq!(
        param(&c, "nonce").as_deref(),
        Some("7ypf/xlj9XXwfDPEoM4URrv/xwf94BcCAzFZH4GiTo0v")
    );
}

#[test]
fn rfc7616_sha256_credentials_parse() {
    let c = parse_one(RFC7616_SHA256);
    assert_eq!(param(&c, "algorithm").as_deref(), Some("SHA-256"));
    assert_eq!(
        param(&c, "response").as_deref(),
        Some("753927fa0e85d155564e2e272a28d1802ca10daf4496794697cf8db5856cb6c1")
    );
}

/// RFC 7616 §3.4: the extended `username*` parameter (RFC 5987 encoding).
#[test]
fn username_star_extended_parameter_parses() {
    let c = parse_one("Digest username*=UTF-8''J%C3%A4s%20Sch%C3%B6n, realm=\"r\"");
    assert_eq!(
        param(&c, "username*").as_deref(),
        Some("UTF-8''J%C3%A4s%20Sch%C3%B6n")
    );
}

/// A quoted-pair inside a quoted-string is unescaped (the old splitter had no
/// backslash handling at all).
#[test]
fn quoted_pairs_are_unescaped() {
    let c = parse_one(r#"Digest username="a\"b\\c", realm="r""#);
    assert_eq!(param(&c, "username").as_deref(), Some(r#"a"b\c"#));
}

/// `Basic` credentials are `token68`, which the parser documents it does not
/// support — so Basic/Bearer need another reader (`headers::Authorization`).
#[test]
fn basic_token68_credentials_are_not_parseable() {
    let r =
        ChallengeParser::new("Basic QWxhZGRpbjpvcGVuIHNlc2FtZQ==").collect::<Result<Vec<_>, _>>();
    assert!(r.is_err(), "token68 must be rejected: {r:?}");
}

/// The parser is ASCII-only (its doc: "Doesn't allow non-ASCII characters").
/// The old hand parser accepted a raw UTF-8 username; this pins the
/// behaviour change Task 10a lists in the CHANGELOG.
#[test]
fn raw_non_ascii_quoted_value_is_a_parse_error() {
    let r =
        ChallengeParser::new("Digest username=\"Jäs\", realm=\"r\"").collect::<Result<Vec<_>, _>>();
    assert!(r.is_err(), "{r:?}");
}

/// Our own server challenge is a well-formed RFC 7235 challenge: it is the
/// oracle Task 10d keeps using after the renderer gains escaping.
#[test]
fn own_digest_challenge_parses_as_one_challenge() {
    let v = Verifier::new(
        Credentials::Digest {
            username: "admin".into(),
            password: "12345".into(),
        },
        "cameras",
    );
    let header = v.challenge();
    let c = parse_one(&header);
    assert_eq!(c.scheme, "Digest");
    assert_eq!(param(&c, "realm").as_deref(), Some("cameras"));
    assert_eq!(param(&c, "qop").as_deref(), Some("auth"));
    assert_eq!(param(&c, "algorithm").as_deref(), Some("MD5"));
    assert_eq!(param(&c, "nonce").unwrap().len(), 96); // 48 raw bytes, hex
}
