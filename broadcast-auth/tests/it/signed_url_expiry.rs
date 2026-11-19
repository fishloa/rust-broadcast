//! The signed-URL expiry gate runs against the real wall clock (`exp` is an
//! absolute Unix timestamp, see the module docs): an expired token must be
//! refused by `Verifier::verify` itself, never only by a caller-supplied time.
use broadcast_auth::{AuthResult, RequestContext, SignedUrlKeySet, Verifier};
use std::time::{SystemTime, UNIX_EPOCH};

const SECRET: &[u8; 32] = b"01234567890123456789012345678901";

fn keys() -> SignedUrlKeySet {
    SignedUrlKeySet::new([("k".to_string(), SECRET.to_vec())]).unwrap()
}

fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs()
}

fn verify(uri: &str) -> AuthResult {
    Verifier::signed_url(keys()).verify(&RequestContext::new("GET", uri))
}

#[test]
fn the_wall_clock_rejects_an_expired_token() {
    let exp = now() - 60;
    let query = keys().sign("k", "/s/m.m3u8", exp, None).unwrap();
    assert_eq!(
        verify(&format!("/s/m.m3u8?{query}")),
        AuthResult::Unauthorized
    );
}

#[test]
fn a_future_expiration_is_accepted() {
    let exp = now() + 3600;
    let query = keys().sign("k", "/s/m.m3u8", exp, None).unwrap();
    assert_eq!(verify(&format!("/s/m.m3u8?{query}")), AuthResult::Ok);
}

#[test]
fn exp_one_is_already_expired() {
    let query = keys().sign("k", "/s", 1, None).unwrap();
    assert_eq!(verify(&format!("/s?{query}")), AuthResult::Unauthorized);
}
