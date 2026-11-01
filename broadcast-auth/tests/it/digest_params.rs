//! Digest `Authorization` parsing contract (Review Focus #1).
use broadcast_auth::{AuthResult, Credentials, RequestContext, Verifier, respond};
use md5::{Digest as _, Md5};

const REALM: &str = "cameras";

fn digest_verifier(user: &str, pw: &str) -> Verifier {
    Verifier::new(
        Credentials::Digest {
            username: user.into(),
            password: pw.into(),
        },
        REALM,
    )
}

fn verify(v: &Verifier, header: &str, method: &str, uri: &str) -> AuthResult {
    let headers = [("authorization", header)];
    v.verify(&RequestContext::new(method, uri).with_headers(&headers))
}

/// A header the http-auth client produced for `user`/`pw` against `v`'s challenge.
fn answer(v: &Verifier, user: &str, pw: &str, method: &str, uri: &str) -> String {
    respond(
        &v.challenge(),
        &RequestContext::new(method, uri),
        Credentials::new(user, pw),
    )
    .unwrap()
}

fn md5_hex(s: &str) -> String {
    hex::encode(Md5::digest(s.as_bytes()))
}

/// A hand-built, correctly-hashed `qop=auth`/MD5 header with `user` spelled
/// EXACTLY as given (the http-auth client refuses non-ASCII, so this is how a
/// browser's raw UTF-8 username is reproduced).
fn manual_header(v: &Verifier, user: &str, pw: &str, method: &str, uri: &str) -> String {
    let challenge = v.challenge();
    let start = challenge.find("nonce=\"").unwrap() + 7;
    let nonce = &challenge[start..start + challenge[start..].find('"').unwrap()];
    let (nc, cnonce) = ("00000001", "0a4f113b");
    let ha1 = md5_hex(&format!("{user}:{REALM}:{pw}"));
    let ha2 = md5_hex(&format!("{method}:{uri}"));
    let response = md5_hex(&format!("{ha1}:{nonce}:{nc}:{cnonce}:auth:{ha2}"));
    format!(
        "Digest username=\"{user}\", realm=\"{REALM}\", nonce=\"{nonce}\", uri=\"{uri}\", \
         algorithm=MD5, nc={nc}, cnonce=\"{cnonce}\", qop=auth, response=\"{response}\""
    )
}

#[test]
fn baseline_round_trip_still_verifies() {
    let v = digest_verifier("admin", "12345");
    let h = answer(&v, "admin", "12345", "GET", "/x");
    assert_eq!(verify(&v, &h, "GET", "/x"), AuthResult::Ok);
}

/// http-auth escapes `"` and `\` as quoted-pairs; the old splitter had no
/// backslash handling and mis-read the value.
#[test]
fn username_with_quote_and_backslash_round_trips() {
    let user = r#"a"b\c"#;
    let v = digest_verifier(user, "pw");
    let h = answer(&v, user, "pw", "GET", "/x");
    assert!(h.contains(r#"username="a\"b\\c""#), "{h}");
    assert_eq!(verify(&v, &h, "GET", "/x"), AuthResult::Ok);
}

/// RFC 7235 §2.1: auth-param names are case-insensitive.
#[test]
fn parameter_names_are_case_insensitive() {
    let v = digest_verifier("admin", "12345");
    let h = answer(&v, "admin", "12345", "GET", "/x");
    let (scheme, rest) = h.split_once(' ').unwrap();
    let upper: Vec<String> = rest
        .split(", ")
        .map(|p| {
            let (k, val) = p.split_once('=').unwrap();
            format!("{}={val}", k.to_uppercase())
        })
        .collect();
    let header = format!("{scheme} {}", upper.join(", "));
    assert_eq!(verify(&v, &header, "GET", "/x"), AuthResult::Ok);
}

/// The old parser let the LAST duplicate win, so `username="evil", …,
/// username="admin"` was read as `admin` by the verifier while a proxy in
/// front may have logged `evil`. Reject ambiguity outright.
#[test]
fn a_repeated_parameter_is_rejected() {
    let v = digest_verifier("admin", "12345");
    let h = answer(&v, "admin", "12345", "GET", "/x");
    let smuggled = h.replacen("Digest ", "Digest username=\"evil\", ", 1);
    assert_eq!(verify(&v, &smuggled, "GET", "/x"), AuthResult::Unauthorized);
}

#[test]
fn a_second_challenge_after_the_credentials_is_rejected() {
    let v = digest_verifier("admin", "12345");
    let h = answer(&v, "admin", "12345", "GET", "/x");
    assert_eq!(
        verify(&v, &format!("{h}, Basic realm=\"x\""), "GET", "/x"),
        AuthResult::Unauthorized
    );
}

/// DELIBERATE behaviour change (CHANGELOG, breaking): `http-auth`'s parser is
/// ASCII-only, so a raw UTF-8 `username` is a syntax error. RFC 7616 §3.4
/// wants `username*` for non-ASCII, implemented in Task 10e. See escalation E2.
#[test]
fn raw_non_ascii_username_is_rejected() {
    let v = digest_verifier("Jäs", "pw");
    let h = manual_header(&v, "Jäs", "pw", "GET", "/x");
    assert_eq!(verify(&v, &h, "GET", "/x"), AuthResult::Unauthorized);
}

#[test]
fn field_count_cap_still_applies() {
    let v = digest_verifier("admin", "12345");
    let h = answer(&v, "admin", "12345", "GET", "/x");
    let padding: String = (0..80).map(|i| format!(", x{i}=1")).collect();
    assert_eq!(
        verify(&v, &format!("{h}{padding}"), "GET", "/x"),
        AuthResult::Unauthorized
    );
}
