//! Defect 6 (spec §3): the signed-URL `kid` was not percent-encoded.
use broadcast_auth::{AuthResult, RequestContext, SignedUrlKeySet, Verifier};
use std::net::{IpAddr, SocketAddr};

const SECRET: &[u8; 32] = b"01234567890123456789012345678901";
const NASTY_KID: &str = "team a/b&c=d%e+f";

fn keys(kid: &str) -> SignedUrlKeySet {
    SignedUrlKeySet::new([(kid.to_string(), SECRET.to_vec())]).unwrap()
}
fn far_future() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs()
        + 3600
}
fn verify(verifier: &Verifier, uri: &str, peer: Option<SocketAddr>) -> AuthResult {
    let mut ctx = RequestContext::new("GET", uri);
    if let Some(p) = peer {
        ctx = ctx.with_peer_addr(p);
    }
    verifier.verify(&ctx)
}

#[test]
fn kid_with_reserved_characters_round_trips() {
    let query = keys(NASTY_KID)
        .sign(NASTY_KID, "/s/m.m3u8", far_future(), None)
        .unwrap();
    // Exactly three parameters: the kid cannot smuggle extra ones.
    let pairs: Vec<(String, String)> = form_urlencoded::parse(query.as_bytes())
        .into_owned()
        .collect();
    assert_eq!(pairs.len(), 3, "{query}");
    assert_eq!(pairs[1], ("kid".to_string(), NASTY_KID.to_string()));
    let verifier = Verifier::signed_url(keys(NASTY_KID));
    assert_eq!(
        verify(&verifier, &format!("/s/m.m3u8?{query}"), None),
        AuthResult::Ok
    );
}

#[test]
fn kid_cannot_inject_an_ip_binding_or_expiry() {
    let kid = "k&ip=198.51.100.9&exp=1";
    let query = keys(kid).sign(kid, "/p", far_future(), None).unwrap();
    let pairs: Vec<_> = form_urlencoded::parse(query.as_bytes()).collect();
    assert_eq!(
        pairs.iter().filter(|(k, _)| k == "ip").count(),
        0,
        "{query}"
    );
    assert_eq!(
        pairs.iter().filter(|(k, _)| k == "exp").count(),
        1,
        "{query}"
    );
}

#[test]
fn ipv6_binding_round_trips_percent_encoded() {
    let ip: IpAddr = "2001:db8::1".parse().unwrap();
    let k = keys("key-a");
    let query = k.sign("key-a", "/p", far_future(), Some(ip)).unwrap();
    assert!(query.contains("ip=2001%3Adb8%3A%3A1"), "{query}");
    let verifier = Verifier::signed_url(keys("key-a"));
    let peer: SocketAddr = "[2001:db8::1]:443".parse().unwrap();
    assert_eq!(
        verify(&verifier, &format!("/p?{query}"), Some(peer)),
        AuthResult::Ok
    );
    let other: SocketAddr = "[2001:db8::2]:443".parse().unwrap();
    assert_eq!(
        verify(&verifier, &format!("/p?{query}"), Some(other)),
        AuthResult::Unauthorized
    );
}

#[test]
fn percent_encoded_kid_minted_elsewhere_verifies() {
    let k = keys("a-b");
    let query = k.sign("a-b", "/p", far_future(), None).unwrap();
    let hand_encoded = query.replace("kid=a-b", "kid=%61%2Db");
    assert_ne!(query, hand_encoded);
    let verifier = Verifier::signed_url(keys("a-b"));
    assert_eq!(
        verify(&verifier, &format!("/p?{hand_encoded}"), None),
        AuthResult::Ok
    );
}

#[test]
fn duplicate_kid_first_wins_and_bare_ip_key_is_rejected() {
    let k = keys("key-a");
    let query = k.sign("key-a", "/p", far_future(), None).unwrap();
    let verifier = Verifier::signed_url(keys("key-a"));
    assert_eq!(
        verify(&verifier, &format!("/p?{query}&kid=other"), None),
        AuthResult::Ok
    );
    // A bare `ip` key means "no value": it must not silently disable IP binding.
    assert_eq!(
        verify(&verifier, &format!("/p?{query}&ip"), None),
        AuthResult::Unauthorized
    );
}

/// Documented breaking difference: URLs minted by the old code with a literal
/// `+` in the kid are now read as a space and rejected.
#[test]
fn legacy_literal_plus_in_kid_is_no_longer_accepted() {
    let kid = "a+b";
    let query = keys(kid).sign(kid, "/p", far_future(), None).unwrap();
    let legacy = query.replace("kid=a%2Bb", "kid=a+b");
    let verifier = Verifier::signed_url(keys(kid));
    assert_eq!(
        verify(&verifier, &format!("/p?{query}"), None),
        AuthResult::Ok
    );
    assert_eq!(
        verify(&verifier, &format!("/p?{legacy}"), None),
        AuthResult::Unauthorized
    );
}
