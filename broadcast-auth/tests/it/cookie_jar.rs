//! The session cookie jar through the public `AuthState`/`CookieWatch`
//! surface: RFC 6265 §5.1.3 (domain), §5.1.4 (path), §5.2 (`Set-Cookie`),
//! §5.3 (storage limits) and §5.4 (the `Cookie` request header), plus the
//! `__Secure-`/`__Host-` prefix rules the jar enforces.
use broadcast_auth::AuthState;
use broadcast_common::stage::Timestamp;
use http::{HeaderMap, HeaderValue};
use url::Url;

const NOW_US: u64 = 1_700_000_000_000_000;

fn now() -> Timestamp {
    Timestamp::from_micros(NOW_US)
}

fn state(origin: &str) -> AuthState {
    AuthState::new(Url::parse(origin).unwrap(), Vec::new())
}

fn receive(state: &mut AuthState, url: &str, lines: &[&str]) {
    let mut headers = HeaderMap::new();
    for line in lines {
        headers.append(
            http::header::SET_COOKIE,
            HeaderValue::from_str(line).unwrap(),
        );
    }
    state.receive_cookies(&Url::parse(url).unwrap(), &headers, now());
}

fn cookie_header(state: &mut AuthState, url: &str) -> Option<String> {
    state
        .request_headers(&Url::parse(url).unwrap(), now())
        .into_iter()
        .find(|(k, _)| k.eq_ignore_ascii_case("cookie"))
        .map(|(_, v)| v)
}

fn stored(state: &AuthState) -> Vec<(String, String)> {
    state.cookies(now())
}

#[test]
fn secure_cookie_only_travels_over_https() {
    let mut jar = state("https://cdn.test/a");
    receive(&mut jar, "https://cdn.test/a", &["a=1; Secure"]);
    assert_eq!(
        cookie_header(&mut jar, "https://cdn.test/a"),
        Some("a=1".to_string())
    );
    assert_eq!(cookie_header(&mut jar, "http://cdn.test/a"), None);

    let mut plain = state("http://cdn.test/a");
    receive(&mut plain, "http://cdn.test/a", &["a=1; Secure"]);
    assert_eq!(stored(&plain), vec![]);
}

#[test]
fn cookie_name_prefixes_are_enforced() {
    let mut jar = state("https://cdn.test/");
    receive(&mut jar, "https://cdn.test/", &["__Secure-x=1"]);
    assert_eq!(stored(&jar), vec![], "__Secure- without Secure");
    receive(&mut jar, "https://cdn.test/", &["__Secure-x=1; Secure"]);
    assert_eq!(
        stored(&jar),
        vec![("__Secure-x".to_string(), "1".to_string())]
    );

    let mut host = state("https://cdn.test/");
    receive(&mut host, "https://cdn.test/", &["__Host-x=1; Secure; Path=/"]);
    assert_eq!(stored(&host), vec![("__Host-x".to_string(), "1".to_string())]);

    let mut bad_path = state("https://cdn.test/");
    receive(
        &mut bad_path,
        "https://cdn.test/",
        &["__Host-x=1; Secure; Path=/a"],
    );
    assert_eq!(stored(&bad_path), vec![], "__Host- requires Path=/");

    let mut with_domain = state("https://cdn.test/");
    receive(
        &mut with_domain,
        "https://cdn.test/",
        &["__Host-x=1; Secure; Path=/; Domain=cdn.test"],
    );
    assert_eq!(stored(&with_domain), vec![], "__Host- forbids Domain");
}

#[test]
fn cookie_is_scoped_to_its_origin() {
    let mut jar = state("https://cdn.test/");
    receive(&mut jar, "https://cdn.test/", &["sid=1"]);
    assert_eq!(cookie_header(&mut jar, "https://other.test/"), None);
    assert_eq!(cookie_header(&mut jar, "https://cdn.test:8443/"), None);
    assert_eq!(
        cookie_header(&mut jar, "https://cdn.test/x"),
        Some("sid=1".to_string())
    );
}

#[test]
fn domain_and_path_attributes_scope_the_cookie() {
    let mut jar = state("https://cdn.test/");
    receive(
        &mut jar,
        "https://cdn.test/a",
        &["own=1; Domain=cdn.test", "other=1; Domain=other.test"],
    );
    assert_eq!(
        cookie_header(&mut jar, "https://cdn.test/x"),
        Some("own=1".to_string()),
        "a suffix that does not match the origin is dropped"
    );

    receive(&mut jar, "https://cdn.test/a", &["deep=1; Path=/a"]);
    assert_eq!(
        cookie_header(&mut jar, "https://cdn.test/a/b"),
        Some("deep=1; own=1".to_string())
    );
    let siblings = cookie_header(&mut jar, "https://cdn.test/ab");
    assert_eq!(
        siblings,
        Some("own=1".to_string()),
        "Path=/a must not match /ab"
    );
}

#[test]
fn cookie_value_and_path_size_caps() {
    let big = "v".repeat(4095);
    let mut jar = state("https://cdn.test/");
    receive(
        &mut jar,
        "https://cdn.test/",
        &[
            &format!("b={big}"),
            &format!("c={}", "v".repeat(4096)),
        ],
    );
    assert_eq!(
        stored(&jar),
        vec![("b".to_string(), big.clone())],
        "4096 bytes is the largest accepted name+value"
    );

    let mut seeded = state("https://cdn.test/");
    seeded.seed_cookies(&[
        ("s".to_string(), big.clone()),
        ("t".to_string(), "v".repeat(4096)),
    ]);
    assert_eq!(stored(&seeded), vec![("s".to_string(), big)]);

    let longest = format!("/{}", "p".repeat(1023));
    assert_eq!(longest.len(), 1024);
    let mut paths = state("https://cdn.test/");
    receive(
        &mut paths,
        "https://cdn.test/",
        &[
            &format!("long=1; Path={longest}"),
            &format!("too-long=1; Path={longest}p"),
        ],
    );
    assert_eq!(
        stored(&paths),
        vec![("long".to_string(), "1".to_string())]
    );
    assert_eq!(
        cookie_header(&mut paths, &format!("https://cdn.test{longest}/leaf")),
        Some("long=1".to_string())
    );
}

#[test]
fn cookie_count_cap_evicts_the_oldest() {
    let mut jar = state("http://cdn.test/");
    let lines: Vec<String> = (1..=51).map(|i| format!("c{i}=v{i}")).collect();
    let refs: Vec<&str> = lines.iter().map(String::as_str).collect();
    receive(&mut jar, "http://cdn.test/", &refs);
    let kept = stored(&jar);
    assert_eq!(kept.len(), 50);
    assert_eq!(kept[0], ("c2".to_string(), "v2".to_string()));
    assert_eq!(kept[49], ("c51".to_string(), "v51".to_string()));
    assert!(!kept.iter().any(|(name, _)| name == "c1"));
}

#[test]
fn cookie_header_is_capped_at_16_kib_inclusive_at_the_boundary() {
    // Five pairs: 3 (name "nN" + "=") + value bytes, plus four "; " separators.
    // 4 * 3275 + 3276 + 4 * 2 == 16384 exactly.
    let mut jar = state("https://cdn.test/");
    let lines: Vec<String> = (0..5)
        .map(|i| {
            let value_len = if i == 4 { 3273 } else { 3272 };
            format!("n{i}={}", "v".repeat(value_len))
        })
        .collect();
    let refs: Vec<&str> = lines.iter().map(String::as_str).collect();
    receive(&mut jar, "https://cdn.test/", &refs);
    let header = cookie_header(&mut jar, "https://cdn.test/").unwrap();
    assert_eq!(header.len(), 16384);
    assert!(header.contains("; "), "pairs are separated by \"; \"");
    assert!(header.ends_with(&"v".repeat(3273)));
}

#[test]
fn same_name_on_different_paths_coexists_longest_first() {
    let mut jar = state("https://cdn.test/");
    receive(
        &mut jar,
        "https://cdn.test/",
        &["k=root; Path=/", "k=deep; Path=/a/b"],
    );
    assert_eq!(
        cookie_header(&mut jar, "https://cdn.test/a/b/c"),
        Some("k=deep; k=root".to_string())
    );
    receive(&mut jar, "https://cdn.test/", &["k=fresh; Path=/"]);
    assert_eq!(
        cookie_header(&mut jar, "https://cdn.test/a/b/c"),
        Some("k=deep; k=fresh".to_string())
    );

    let mut distinct = state("https://cdn.test/");
    receive(
        &mut distinct,
        "https://cdn.test/",
        &["first=1; Path=/", "second=2; Path=/"],
    );
    assert_eq!(
        stored(&distinct),
        vec![
            ("first".to_string(), "1".to_string()),
            ("second".to_string(), "2".to_string()),
        ],
        "same path, different names must not overwrite each other"
    );
}

#[test]
fn seeding_skips_prefixed_names_on_http() {
    let mut jar = state("http://cdn.test/");
    jar.seed_cookies(&[
        ("__Secure-x".to_string(), "1".to_string()),
        ("__Host-y".to_string(), "2".to_string()),
        ("plain".to_string(), "3".to_string()),
    ]);
    assert_eq!(stored(&jar), vec![("plain".to_string(), "3".to_string())]);
}

#[test]
fn deleting_one_cookie_leaves_its_siblings() {
    let mut jar = state("https://cdn.test/");
    receive(
        &mut jar,
        "https://cdn.test/",
        &["gone=1; Path=/x", "kept=2; Path=/x"],
    );
    receive(&mut jar, "https://cdn.test/", &["gone=0; Path=/x; Max-Age=0"]);
    assert_eq!(
        stored(&jar),
        vec![("kept".to_string(), "2".to_string())],
        "Max-Age=0 removes only its own name and path"
    );
}

#[test]
fn watch_and_debug_never_leak_values() {
    let mut jar = state("https://cdn.test/");
    jar.seed_cookies(&[("sid".to_string(), "abc".to_string())]);
    let watch = jar.cookie_watch();
    assert_eq!(
        watch.cookies(now()),
        vec![("sid".to_string(), "abc".to_string())]
    );
    let text = format!("{watch:?}");
    assert!(text.contains("CookieWatch"), "{text}");
    assert!(text.contains("CookieJar"), "{text}");
    assert!(!text.contains("abc"), "{text}");
}
