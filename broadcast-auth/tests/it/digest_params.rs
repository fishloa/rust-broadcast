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
    .expect("OS random source")
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
    broadcast_common::hex::hex_encode(&Md5::digest(s.as_bytes()))
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

/// The nonce of `v`'s current challenge.
fn challenge_nonce(v: &Verifier) -> String {
    let challenge = v.challenge();
    let start = challenge.find("nonce=\"").unwrap() + 7;
    challenge[start..start + challenge[start..].find('"').unwrap()].to_string()
}

fn pair(name: &str, value: &str) -> (String, String) {
    (name.to_string(), value.to_string())
}

/// The `response` for the configured `admin`/`cameras`/`12345` over the given
/// `nc`/`cnonce`.
fn response_for(nonce: &str, nc: &str, cnonce: &str) -> String {
    let ha1 = md5_hex(&format!("admin:{REALM}:12345"));
    let ha2 = md5_hex("GET:/x");
    md5_hex(&format!("{ha1}:{nonce}:{nc}:{cnonce}:auth:{ha2}"))
}

/// A `Digest` header whose response is always the correct one for the
/// configured identity, with `overrides` replacing or adding parameters: only
/// the field-level checks (not the hash compare) can reject it.
fn digest_header(nonce: &str, nc: &str, cnonce: &str, overrides: Vec<(String, String)>) -> String {
    let response = response_for(nonce, nc, cnonce);
    let mut params = vec![
        pair("username", "admin"),
        pair("realm", REALM),
        pair("nonce", nonce),
        pair("uri", "/x"),
        pair("algorithm", "MD5"),
        pair("nc", nc),
        pair("cnonce", cnonce),
        pair("qop", "auth"),
        pair("response", &response),
    ];
    for (name, value) in overrides {
        match params.iter_mut().find(|(n, _)| *n == name) {
            Some(slot) => slot.1 = value,
            None => params.push((name, value)),
        }
    }
    let rendered: Vec<String> = params.iter().map(|(n, v)| format!("{n}=\"{v}\"")).collect();
    format!("Digest {}", rendered.join(", "))
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

/// The Digest field checks reject before the hash hides them: a header whose
/// `response` is the CORRECT one for the configured identity, but which claims
/// a different username, must still be refused (RFC 7616 §3.4.1).
#[test]
fn a_wrong_claimed_username_is_rejected_even_with_a_correct_response() {
    let v = digest_verifier("admin", "12345");
    let nonce = challenge_nonce(&v);
    let h = digest_header(
        &nonce,
        "00000001",
        "0a4f113b",
        vec![pair("username", "mallory")],
    );
    assert_eq!(verify(&v, &h, "GET", "/x"), AuthResult::Unauthorized);
}

#[test]
fn a_wrong_claimed_realm_is_rejected_even_with_a_correct_response() {
    let v = digest_verifier("admin", "12345");
    let nonce = challenge_nonce(&v);
    let h = digest_header(
        &nonce,
        "00000001",
        "0a4f113b",
        vec![pair("realm", "elsewhere")],
    );
    assert_eq!(verify(&v, &h, "GET", "/x"), AuthResult::Unauthorized);
}

#[test]
fn an_empty_cnonce_is_rejected_even_with_a_matching_response() {
    let v = digest_verifier("admin", "12345");
    let nonce = challenge_nonce(&v);
    let h = digest_header(&nonce, "00000001", "", vec![]);
    assert_eq!(verify(&v, &h, "GET", "/x"), AuthResult::Unauthorized);
}

#[test]
fn an_empty_response_is_rejected() {
    let v = digest_verifier("admin", "12345");
    let nonce = challenge_nonce(&v);
    let h = digest_header(&nonce, "00000001", "0a4f113b", vec![pair("response", "")]);
    assert_eq!(verify(&v, &h, "GET", "/x"), AuthResult::Unauthorized);
}

/// RFC 7616 §3.4: `nc-value = 8LHEX`. Six hex digits are the case that slips
/// through a `len != 8 || !hex` guard turned into `&&`, because `from_str_radix`
/// still accepts them.
#[test]
fn nc_must_be_exactly_eight_hex_digits() {
    let v = digest_verifier("admin", "12345");
    let nonce = challenge_nonce(&v);
    for bad in ["000001", "0000000g", "000000001", "0x000001"] {
        let h = digest_header(&nonce, bad, "0a4f113b", vec![]);
        assert_eq!(
            verify(&v, &h, "GET", "/x"),
            AuthResult::Unauthorized,
            "nc={bad:?}"
        );
    }
    let ok = digest_header(&nonce, "00000001", "0a4f113b", vec![]);
    assert_eq!(verify(&v, &ok, "GET", "/x"), AuthResult::Ok);
}

/// The header size cap is inclusive: exactly 64 parameters parse and verify,
/// 65 are refused outright (the parsing DoS bound, off-by-one either way).
#[test]
fn exactly_64_digest_parameters_are_accepted_and_65_are_not() {
    let v = digest_verifier("admin", "12345");
    let nonce = challenge_nonce(&v);
    let filler = |count: usize| -> Vec<(String, String)> {
        (0..count).map(|i| pair(&format!("x{i}"), "1")).collect()
    };
    // 9 required parameters + 55 filler == 64.
    let at_cap = digest_header(&nonce, "00000001", "0a4f113b", filler(55));
    assert_eq!(verify(&v, &at_cap, "GET", "/x"), AuthResult::Ok);
    let over_cap = digest_header(&nonce, "00000001", "0a4f113b", filler(56));
    assert_eq!(verify(&v, &over_cap, "GET", "/x"), AuthResult::Unauthorized);
}

#[test]
fn whitespace_inside_an_unquoted_token_is_rejected() {
    let v = digest_verifier("admin", "12345");
    let h = answer(&v, "admin", "12345", "GET", "/x");
    let spaced = h.replace("qop=auth", "qop=au th");
    assert_ne!(h, spaced, "test setup: qop must change");
    assert_eq!(verify(&v, &spaced, "GET", "/x"), AuthResult::Unauthorized);
}
