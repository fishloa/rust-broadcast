use alloc::string::ToString;
use alloc::vec::Vec;

use url::Url;

use crate::auth::AuthState;

const ORIGIN: &str = "https://a.example.com/live/x.m3u8";

fn state_with(origin: &str, line: &str) -> AuthState {
    let url = Url::parse(origin).unwrap();
    let mut auth = AuthState::new(url.clone(), Vec::new());
    auth.receive_cookies(&url, &[("Set-Cookie".to_string(), line.to_string())], 0);
    auth
}

fn cookie_to(auth: &mut AuthState, url: &str) -> Option<alloc::string::String> {
    let url = Url::parse(url).unwrap();
    auth.request_headers(&url, 0)
        .into_iter()
        .find(|(k, _)| k.eq_ignore_ascii_case("cookie"))
        .map(|(_, v)| v)
}

const NON_ORIGIN_HOSTS: [&str; 8] = [
    "https://b.example.com/seg.ts",
    "https://example.com/",
    "https://x.a.example.com/",
    "https://notexample.com/",
    "https://example.com.evil.test/",
    "https://evil.test/",
    "https://com/",
    "http://a.example.com:8080/live/x.m3u8",
];

fn assert_origin_only(line: &str, expected: &str) {
    let mut auth = state_with(ORIGIN, line);
    assert_eq!(cookie_to(&mut auth, ORIGIN).as_deref(), Some(expected));
    for url in NON_ORIGIN_HOSTS {
        assert_eq!(cookie_to(&mut auth, url), None, "{line} -> {url}");
    }
}

#[test]
fn domain_cookie_matching_the_origin_is_sent_only_to_the_origin() {
    assert_origin_only("sid=1; Domain=example.com; Path=/", "sid=1");
    assert_origin_only("sid=1; Domain=.a.example.com; Path=/", "sid=1");
}

#[test]
fn host_only_cookie_is_sent_only_to_the_origin() {
    assert_origin_only("sid=1; Path=/", "sid=1");
}

#[test]
fn public_suffix_domain_is_stored_but_never_leaves_the_origin() {
    for line in ["sid=1; Domain=com; Path=/", "sid=1; Domain=.com; Path=/"] {
        assert_origin_only(line, "sid=1");
    }
    let uk = "https://a.example.co.uk/live/x.m3u8";
    let mut auth = state_with(uk, "sid=1; Domain=co.uk; Path=/");
    assert_eq!(cookie_to(&mut auth, uk).as_deref(), Some("sid=1"));
    assert_eq!(cookie_to(&mut auth, "https://b.other.co.uk/"), None);
    assert_eq!(cookie_to(&mut auth, "https://evil.co.uk/"), None);
}

#[test]
fn secure_domain_cookie_is_not_sent_over_http() {
    let mut auth = state_with(ORIGIN, "sid=1; Domain=example.com; Path=/; Secure");
    assert_eq!(
        cookie_to(&mut auth, "http://a.example.com/live/x.m3u8"),
        None
    );
    assert_eq!(cookie_to(&mut auth, ORIGIN).as_deref(), Some("sid=1"));
}

#[test]
fn foreign_domain_cookie_is_refused() {
    let url = Url::parse(ORIGIN).unwrap();
    let mut auth = AuthState::new(url.clone(), Vec::new());
    for line in [
        "a=1; Domain=other.com",
        "a=1; Domain=b.example.com",
        "a=1; Domain=x.a.example.com",
    ] {
        auth.receive_cookies(&url, &[("Set-Cookie".to_string(), line.to_string())], 0);
    }
    assert_eq!(auth.cookies(0), Vec::new());
}

#[test]
fn path_scope_still_applies_at_the_origin() {
    let mut auth = state_with(ORIGIN, "sid=1; Domain=example.com; Path=/live");
    assert_eq!(cookie_to(&mut auth, ORIGIN).as_deref(), Some("sid=1"));
    assert_eq!(
        cookie_to(&mut auth, "https://a.example.com/other/seg.ts"),
        None
    );
}

#[test]
fn seeded_cookies_stay_host_only() {
    let url = Url::parse(ORIGIN).unwrap();
    let mut auth = AuthState::new(url, Vec::new());
    auth.seed_cookies(&[("s".to_string(), "1".to_string())]);
    assert_eq!(cookie_to(&mut auth, ORIGIN).as_deref(), Some("s=1"));
    assert_eq!(cookie_to(&mut auth, "https://b.example.com/"), None);
}
