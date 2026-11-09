//! Bug audit H reproductions: each test fails on main.
use broadcast_auth::{AuthResult, Credentials, RequestContext, SignedUrlKeySet, Verifier, respond};
use std::net::{IpAddr, SocketAddr};

fn keys() -> SignedUrlKeySet {
    SignedUrlKeySet::new([("k".to_string(), vec![7u8; 32])]).unwrap()
}

fn authz(v: &Verifier, header: &str) -> AuthResult {
    let headers = [("authorization", header)];
    v.verify(&RequestContext::new("GET", "/x").with_headers(&headers))
}

#[test]
fn h_a1_request_context_debug_leaks_signed_url_sig_and_cookie() {
    let ks = keys();
    let q = ks.sign("k", "/a", u64::MAX / 2, None).unwrap();
    let sig = q
        .split("sig=")
        .nth(1)
        .unwrap()
        .split('&')
        .next()
        .unwrap()
        .to_string();
    let uri = format!("/a?{q}");
    let headers = [("cookie", "session=abcSECRET")];
    let ctx = RequestContext::new("GET", &uri).with_headers(&headers);
    let d = format!("{ctx:?}");
    assert!(!d.contains(&sig), "signed-url sig leaked: {d}");
    assert!(!d.contains("abcSECRET"), "cookie leaked: {d}");
}

#[test]
fn h_a2_ip_bound_signed_url_rejects_v4_mapped_peer() {
    let ks = keys();
    let ip: IpAddr = "192.0.2.5".parse().unwrap();
    let q = ks.sign("k", "/a", u64::MAX / 2, Some(ip)).unwrap();
    let v = Verifier::signed_url(ks);
    let uri = format!("/a?{q}");
    let peer: SocketAddr = "[::ffff:192.0.2.5]:1234".parse().unwrap();
    let ctx = RequestContext::new("GET", &uri).with_peer_addr(peer);
    assert_eq!(v.verify(&ctx), AuthResult::Ok);
}

#[test]
fn h_a3_digest_absolute_form_uri_host_is_not_checked() {
    let v = Verifier::new(Credentials::new("u", "p"), "r");
    let h = respond(
        &v.challenge(),
        &RequestContext::new("GET", "http://other-host.example/x"),
        Credentials::new("u", "p"),
    )
    .unwrap();
    assert_eq!(authz(&v, &h), AuthResult::Unauthorized);
}

#[test]
fn h_a4_blank_bearer_token_accepts_a_blank_credential() {
    for token in ["", "   "] {
        let v = Verifier::new(Credentials::bearer(token), "r");
        assert_eq!(authz(&v, "Bearer "), AuthResult::Unauthorized, "{token:?}");
    }
}
