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

#[test]
fn domain_cookie_reaches_sibling_and_parent_hosts() {
    let mut auth = state_with(ORIGIN, "sid=1; Domain=example.com; Path=/");
    assert_eq!(cookie_to(&mut auth, ORIGIN).as_deref(), Some("sid=1"));
    assert_eq!(
        cookie_to(&mut auth, "https://b.example.com/seg.ts").as_deref(),
        Some("sid=1")
    );
    assert_eq!(
        cookie_to(&mut auth, "https://example.com/").as_deref(),
        Some("sid=1")
    );
}

#[test]
fn domain_cookie_never_reaches_unrelated_hosts() {
    let mut auth = state_with(ORIGIN, "sid=1; Domain=example.com; Path=/");
    for url in [
        "https://notexample.com/",
        "https://example.com.evil.test/",
        "https://evil.test/",
        "https://com/",
    ] {
        assert_eq!(cookie_to(&mut auth, url), None, "{url}");
    }
}

#[test]
fn secure_domain_cookie_is_not_sent_over_http_to_a_sibling() {
    let mut auth = state_with(ORIGIN, "sid=1; Domain=example.com; Path=/; Secure");
    assert_eq!(cookie_to(&mut auth, "http://b.example.com/seg.ts"), None);
    assert_eq!(
        cookie_to(&mut auth, "https://b.example.com/seg.ts").as_deref(),
        Some("sid=1")
    );
}

#[test]
fn host_only_cookie_is_not_sent_to_a_sibling() {
    let mut auth = state_with(ORIGIN, "sid=1; Path=/");
    assert_eq!(cookie_to(&mut auth, ORIGIN).as_deref(), Some("sid=1"));
    for url in [
        "https://b.example.com/seg.ts",
        "https://example.com/",
        "https://x.a.example.com/",
    ] {
        assert_eq!(cookie_to(&mut auth, url), None, "{url}");
    }
}

#[test]
fn path_scope_applies_on_the_sibling() {
    let mut auth = state_with(ORIGIN, "sid=1; Domain=example.com; Path=/live");
    assert_eq!(
        cookie_to(&mut auth, "https://b.example.com/live/seg.ts").as_deref(),
        Some("sid=1")
    );
    assert_eq!(
        cookie_to(&mut auth, "https://b.example.com/other/seg.ts"),
        None
    );
    assert_eq!(cookie_to(&mut auth, "https://b.example.com/livex"), None);
}

#[test]
fn seeded_cookies_stay_host_only() {
    let url = Url::parse(ORIGIN).unwrap();
    let mut auth = AuthState::new(url, Vec::new());
    auth.seed_cookies(&[("s".to_string(), "1".to_string())]);
    assert_eq!(cookie_to(&mut auth, ORIGIN).as_deref(), Some("s=1"));
    assert_eq!(cookie_to(&mut auth, "https://b.example.com/"), None);
}
