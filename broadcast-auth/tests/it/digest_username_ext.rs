//! Digest `username*` (RFC 7616 §3.4.4, RFC 8187 ext-value).
use broadcast_auth::{AuthResult, Credentials, RequestContext, Verifier};
use md5::{Digest as _, Md5};

const REALM: &str = "cameras";
/// RFC 7616 §3.4.4's own example value for "Jäs Schön".
const RFC_EXT: &str = "UTF-8''J%C3%A4s%20Sch%C3%B6n";
const RFC_USER: &str = "Jäs Schön";

fn md5_hex(s: &str) -> String {
    hex::encode(Md5::digest(s.as_bytes()))
}

fn verifier(user: &str) -> Verifier {
    Verifier::new(
        Credentials::Digest {
            username: user.into(),
            password: "pw".into(),
        },
        REALM,
    )
}

/// A correctly-hashed header whose username parameter text is `user_param`
/// verbatim (e.g. `username*=UTF-8''J%C3%A4s`), the HASH being over `hashed_user`.
fn header(v: &Verifier, user_param: &str, hashed_user: &str, extra: &str) -> String {
    let (method, uri) = ("GET", "/x");
    let challenge = v.challenge();
    let start = challenge.find("nonce=\"").unwrap() + 7;
    let nonce = &challenge[start..start + challenge[start..].find('"').unwrap()];
    let (nc, cnonce) = ("00000001", "0a4f113b");
    let ha1 = md5_hex(&format!("{hashed_user}:{REALM}:pw"));
    let ha2 = md5_hex(&format!("{method}:{uri}"));
    let response = md5_hex(&format!("{ha1}:{nonce}:{nc}:{cnonce}:auth:{ha2}"));
    format!(
        "Digest {user_param}, realm=\"{REALM}\", nonce=\"{nonce}\", uri=\"{uri}\", algorithm=MD5, \
         nc={nc}, cnonce=\"{cnonce}\", qop=auth, response=\"{response}\"{extra}"
    )
}

fn verify(v: &Verifier, h: &str) -> AuthResult {
    let headers = [("authorization", h)];
    v.verify(&RequestContext::new("GET", "/x").with_headers(&headers))
}

#[test]
fn rfc7616_ext_value_example_authenticates_a_non_ascii_user_end_to_end() {
    let v = verifier(RFC_USER);
    let h = header(&v, &format!("username*={RFC_EXT}"), RFC_USER, "");
    assert_eq!(verify(&v, &h), AuthResult::Ok);
    // Wrong password for the same user still fails (HA1 is over the decoded name).
    let bad = header(&v, &format!("username*={RFC_EXT}"), "someone-else", "");
    assert_eq!(verify(&v, &bad), AuthResult::Unauthorized);
    // A different decoded user is not the configured one.
    let other = verifier("Jas");
    assert_eq!(verify(&other, &h), AuthResult::Unauthorized);
}

#[test]
fn charset_is_case_insensitive_and_a_language_tag_is_allowed() {
    let v = verifier(RFC_USER);
    for ext in [
        "utf-8''J%C3%A4s%20Sch%C3%B6n",
        "Utf-8'de'J%C3%A4s%20Sch%C3%B6n",
    ] {
        let h = header(&v, &format!("username*={ext}"), RFC_USER, "");
        assert_eq!(verify(&v, &h), AuthResult::Ok, "{ext}");
    }
}

#[test]
fn an_ascii_user_may_also_use_the_extended_form() {
    let v = verifier("admin");
    let h = header(&v, "username*=UTF-8''admin", "admin", "");
    assert_eq!(verify(&v, &h), AuthResult::Ok);
}

#[test]
fn username_and_username_star_together_are_an_error() {
    let v = verifier(RFC_USER);
    let h = header(
        &v,
        &format!("username=\"admin\", username*={RFC_EXT}"),
        RFC_USER,
        "",
    );
    assert_eq!(verify(&v, &h), AuthResult::Unauthorized);
}

#[test]
fn only_the_utf8_charset_is_accepted() {
    let v = verifier("Jäs");
    for ext in [
        "ISO-8859-1''J%E4s",
        "US-ASCII''Jas",
        "''J%C3%A4s",
        "J%C3%A4s",
    ] {
        let h = header(&v, &format!("username*={ext}"), "Jäs", "");
        assert_eq!(verify(&v, &h), AuthResult::Unauthorized, "{ext}");
    }
}

#[test]
fn malformed_percent_encoding_is_rejected_not_passed_through() {
    let v = verifier("J%zzs");
    // `%zz` would survive `percent_decode_str` as literal text and could match a
    // configured name containing it; the syntax check must reject it first.
    for ext in [
        "UTF-8''J%zzs",
        "UTF-8''J%C3%2",
        "UTF-8''J%",
        "UTF-8''J%C3%A4s%",
        "UTF-8''J s",
    ] {
        let h = header(&v, &format!("username*={ext}"), "J%zzs", "");
        assert_eq!(verify(&v, &h), AuthResult::Unauthorized, "{ext}");
    }
}

#[test]
fn percent_encoded_bytes_that_are_not_utf8_are_rejected() {
    let v = verifier("x");
    for ext in ["UTF-8''%FF%FE", "UTF-8''%C3", "UTF-8''%C0%80"] {
        let h = header(&v, &format!("username*={ext}"), "x", "");
        assert_eq!(verify(&v, &h), AuthResult::Unauthorized, "{ext}");
    }
}

/// Bites the strict-UTF-8 requirement: with a LOSSY decode `%FF%FE` becomes
/// `\u{FFFD}\u{FFFD}`, which a user configured with exactly that name (and a
/// matching hash) would authenticate as. Strict decoding must reject it.
#[test]
fn lossy_replacement_characters_never_authenticate() {
    let name = "\u{FFFD}\u{FFFD}";
    let v = verifier(name);
    let h = header(&v, "username*=UTF-8''%FF%FE", name, "");
    assert_eq!(verify(&v, &h), AuthResult::Unauthorized);
}

#[test]
fn userhash_true_is_rejected_and_userhash_false_is_not() {
    let v = verifier("admin");
    let on = header(&v, "username=\"admin\"", "admin", ", userhash=true");
    assert_eq!(verify(&v, &on), AuthResult::Unauthorized);
    let off = header(&v, "username=\"admin\"", "admin", ", userhash=false");
    assert_eq!(verify(&v, &off), AuthResult::Ok);
}

/// Raw UTF-8 in `username` stays a syntax error (Task 10a) — only `username*` carries non-ASCII.
#[test]
fn raw_non_ascii_username_is_still_rejected() {
    let v = verifier(RFC_USER);
    let h = header(&v, &format!("username=\"{RFC_USER}\""), RFC_USER, "");
    assert_eq!(verify(&v, &h), AuthResult::Unauthorized);
}
