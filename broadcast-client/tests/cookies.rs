mod support;

use broadcast_client::{Config, Credentials, Event, Session};
use support::hls_host::*;
use support::ts_probe::{self, *};

const CDN_SEG_URL: &str = "http://cdn.test/seg/index1.ts";
const MAX_JAR: usize = 50;
const MAX_COOKIE_BYTES: usize = 4096;
const MAX_POLL_SECS: u64 = 120;

fn cookie_of(url: &str, lines: &[&str], target: &str) -> Option<String> {
    ts_probe::redirect_probe(url, lines, target)
        .cookie()
        .map(str::to_string)
}

fn session_with_cookies(url: &str, lines: &[&str]) -> Session {
    let mut s = open(url, Config::default());
    let first = one(&mut s, 0);
    ok_with_cookies(&mut s, &first, lines);
    s
}

fn pair(n: &str, v: &str) -> (String, String) {
    (n.to_string(), v.to_string())
}

#[test]
fn set_cookie_sent_on_next_request() {
    let mut s = open(TS_URL, Config::default());
    let first = one(&mut s, 0);
    assert_eq!(first.cookie(), None);
    ok_with_cookies(&mut s, &first, &["sid=abc; Path=/"]);
    let second = reconnect(&mut s, SEC);
    assert_eq!(second.cookie(), Some("sid=abc"));
    assert_eq!(second.count("cookie"), 1);
}

#[test]
fn multiple_set_cookie_lines_all_stored() {
    let mut s = session_with_cookies(TS_URL, &["a=1; Path=/", "b=2; Path=/", "c=3; Path=/"]);
    let next = reconnect(&mut s, SEC);
    assert_eq!(next.cookie(), Some("a=1; b=2; c=3"));
    assert_eq!(next.count("cookie"), 1);
    assert_eq!(
        s.cookies(),
        vec![pair("a", "1"), pair("b", "2"), pair("c", "3")]
    );
}

#[test]
fn same_name_path_replaces() {
    let mut s = session_with_cookies(
        TS_URL,
        &["sid=1; Path=/", "other=x; Path=/", "sid=2; Path=/"],
    );
    let next = reconnect(&mut s, SEC);
    assert_eq!(next.cookie(), Some("sid=2; other=x"));
    assert_eq!(s.cookies(), vec![pair("sid", "2"), pair("other", "x")]);
    ok_with_cookies(&mut s, &next, &["sid=3; Path=/"]);
    assert_eq!(s.cookies(), vec![pair("sid", "3"), pair("other", "x")]);
}

#[test]
fn same_name_different_path_coexists() {
    let mut s = session_with_cookies(
        "http://example/a/live.ts",
        &["sid=root; Path=/", "sid=deep; Path=/a"],
    );
    let next = reconnect(&mut s, SEC);
    assert_eq!(next.cookie(), Some("sid=deep; sid=root"));
    assert_eq!(s.cookies().len(), 2);
}

#[test]
fn longer_path_first_in_cookie_header() {
    let mut s = session_with_cookies(
        "http://example/a/b/live.ts",
        &[
            "g=1; Path=/",
            "m=2; Path=/a",
            "l=3; Path=/a/b",
            "t1=x; Path=/",
            "t2=y; Path=/",
        ],
    );
    let next = reconnect(&mut s, SEC);
    assert_eq!(next.cookie(), Some("l=3; m=2; g=1; t1=x; t2=y"));
}

#[test]
fn path_match_rules() {
    let base = "http://example/a/b/live.ts";

    assert_eq!(
        cookie_of(base, &["d=1"], "http://example/a/b/other.ts").as_deref(),
        Some("d=1"),
        "default-path of /a/b/live.ts is /a/b"
    );
    assert_eq!(
        cookie_of(base, &["d=1"], "http://example/a/b/x/y.ts").as_deref(),
        Some("d=1"),
        "cookie-path is a prefix followed by /"
    );
    assert_eq!(
        cookie_of(base, &["d=1"], "http://example/a/bc.ts"),
        None,
        "/a/b is a plain prefix of /a/bc.ts but not on a path boundary"
    );
    assert_eq!(
        cookie_of(base, &["d=1"], "http://example/a/other.ts"),
        None,
        "default-path /a/b does not match /a/other.ts"
    );
    assert_eq!(cookie_of(base, &["d=1"], "http://example/live.ts"), None);

    assert_eq!(
        cookie_of(base, &["p=1; Path=/a"], "http://example/a").as_deref(),
        Some("p=1"),
        "identical path"
    );
    assert_eq!(
        cookie_of(base, &["p=1; Path=/a"], "http://example/a/x.ts").as_deref(),
        Some("p=1")
    );
    assert_eq!(
        cookie_of(base, &["p=1; Path=/a"], "http://example/ab/x.ts"),
        None,
        "/a vs /ab"
    );
    assert_eq!(
        cookie_of(base, &["p=1; Path=/a/"], "http://example/a/x.ts").as_deref(),
        Some("p=1"),
        "cookie-path ending in /"
    );
    assert_eq!(
        cookie_of(base, &["p=1; Path=/a/"], "http://example/a"),
        None,
        "/a/ is not a prefix of /a"
    );

    assert_eq!(
        cookie_of("http://example/live.ts", &["r=1"], "http://example/z/w.ts").as_deref(),
        Some("r=1"),
        "single-slash uri-path defaults to /"
    );
    assert_eq!(
        cookie_of(base, &["bad=1; Path=relative"], "http://example/a/b/q.ts").as_deref(),
        Some("bad=1"),
        "a Path not starting with / falls back to the default-path"
    );
    assert_eq!(
        cookie_of(base, &["bad=1; Path=relative"], "http://example/a/c.ts"),
        None
    );
}

#[test]
fn secure_cookie_not_sent_over_http_sent_over_https() {
    let mut http = session_with_cookies(TS_URL, &["s=1; Secure; Path=/", "p=2; Path=/"]);
    let next = reconnect(&mut http, SEC);
    assert_eq!(next.cookie(), Some("p=2"));

    let mut https = session_with_cookies(
        "https://example/live.ts",
        &["s=1; Secure; Path=/", "p=2; Path=/"],
    );
    let next = reconnect(&mut https, SEC);
    assert_eq!(next.cookie(), Some("s=1; p=2"));
}

#[test]
fn max_age_zero_deletes() {
    let mut s = session_with_cookies(TS_URL, &["sid=1; Path=/", "keep=1; Path=/"]);
    let second = reconnect(&mut s, SEC);
    assert_eq!(second.cookie(), Some("sid=1; keep=1"));
    ok_with_cookies(&mut s, &second, &["sid=; Max-Age=0; Path=/"]);
    let third = reconnect(&mut s, 2 * SEC);
    assert_eq!(third.cookie(), Some("keep=1"));
    assert_eq!(s.cookies(), vec![pair("keep", "1")]);
    ok_with_cookies(&mut s, &third, &["keep=; Max-Age=-5; Path=/"]);
    let fourth = reconnect(&mut s, 3 * SEC);
    assert_eq!(fourth.cookie(), None);
    assert!(s.cookies().is_empty());
}

#[test]
fn max_age_zero_for_unknown_cookie_stores_nothing() {
    let s = session_with_cookies(TS_URL, &["ghost=1; Max-Age=0; Path=/"]);
    assert!(s.cookies().is_empty());
}

#[test]
fn max_age_expires_relative_to_now_us() {
    let stored_at = 5 * SEC;
    let mut s = open(TS_URL, Config::default());
    let first = one(&mut s, stored_at);
    ok_with_cookies(
        &mut s,
        &first,
        &["t=1; Max-Age=10; Path=/", "forever=1; Path=/"],
    );
    let expires_at = stored_at + 10 * SEC;

    let before = reconnect(&mut s, expires_at - 1);
    assert_eq!(before.cookie(), Some("t=1; forever=1"));
    assert_eq!(s.cookies().len(), 2);
    ok_with_cookies(&mut s, &before, &[]);

    let at = reconnect(&mut s, expires_at);
    assert_eq!(at.cookie(), Some("forever=1"));
    assert_eq!(s.cookies(), vec![pair("forever", "1")]);
}

#[test]
fn expires_attribute_ignored() {
    let mut s = session_with_cookies(
        TS_URL,
        &[
            "past=1; Expires=Thu, 01 Jan 1970 00:00:00 GMT; Path=/",
            "future=1; Expires=Fri, 31 Dec 2038 23:59:59 GMT; Path=/",
        ],
    );
    let next = reconnect(&mut s, 1_000 * SEC);
    assert_eq!(next.cookie(), Some("past=1; future=1"));
}

#[test]
fn domain_attribute_ignored_host_only() {
    let line = "d=1; Domain=.example.com; Path=/";
    assert_eq!(
        cookie_of(
            "http://example.com/live.ts",
            &[line],
            "http://sub.example.com/live.ts"
        ),
        None
    );
    assert_eq!(
        cookie_of(
            "http://www.example.com/live.ts",
            &[line],
            "http://example.com/live.ts"
        ),
        None
    );
    assert_eq!(
        cookie_of(
            "http://example.com/live.ts",
            &[line],
            "http://example.com/other.ts"
        )
        .as_deref(),
        Some("d=1"),
        "the cookie is still a host-only cookie of the origin"
    );
}

#[test]
fn not_sent_to_other_origin_redirect() {
    let line = "sid=1; Path=/";
    assert_eq!(
        cookie_of(TS_URL, &[line], "http://other.test/live.ts"),
        None
    );
    assert_eq!(
        cookie_of(TS_URL, &[line], "https://example/live.ts"),
        None,
        "scheme"
    );
    assert_eq!(
        cookie_of(TS_URL, &[line], "http://example:8080/live.ts"),
        None,
        "port"
    );
    assert_eq!(
        cookie_of(TS_URL, &[line], "http://example/next.ts").as_deref(),
        Some("sid=1")
    );
}

#[test]
fn set_cookie_on_401_response_is_stored() {
    let mut s = open(TS_URL, Config::default());
    let first = one(&mut s, 0);
    let mut headers = set_cookies(&["sid=from401; Path=/"]);
    headers.push((
        "WWW-Authenticate".to_string(),
        "Basic realm=\"tv\"".to_string(),
    ));
    respond(&mut s, &first, 401, &headers);
    assert!(requests(&mut s, 0).is_empty());
    let mut saw_required = false;
    while let Some(e) = s.next_event() {
        saw_required |= matches!(e, Event::AuthRequired { .. });
    }
    assert!(saw_required);
    assert_eq!(s.cookies(), vec![pair("sid", "from401")]);
    s.set_credentials(Credentials::new("user", "pass"));
    let retry = one(&mut s, 1);
    assert_eq!(retry.cookie(), Some("sid=from401"));
}

#[test]
fn set_cookie_from_other_origin_response_ignored() {
    let mut s = open(TS_URL, Config::default());
    let first = one(&mut s, 0);
    respond(
        &mut s,
        &first,
        302,
        &[location("http://other.test/live.ts")],
    );
    let other = one(&mut s, 0);
    assert_eq!(other.url, "http://other.test/live.ts");
    let mut headers = set_cookies(&["evil=1; Path=/"]);
    headers.push(location("http://example/live.ts"));
    respond(&mut s, &other, 302, &headers);
    let back = one(&mut s, 0);
    assert_eq!(back.url, TS_URL);
    assert_eq!(back.cookie(), None);
    assert!(s.cookies().is_empty());
    ok_with_cookies(&mut s, &back, &["good=1; Path=/"]);
    assert_eq!(s.cookies(), vec![pair("good", "1")]);
}

#[test]
fn initial_cookies_sent_on_first_request() {
    let config = Config {
        initial_cookies: vec![pair("a", "1"), pair("b", "2")],
        ..Config::default()
    };
    let mut s = open(TS_URL, config);
    let first = one(&mut s, 0);
    assert_eq!(first.cookie(), Some("a=1; b=2"));
    assert_eq!(first.count("cookie"), 1);
    assert_eq!(s.cookies(), vec![pair("a", "1"), pair("b", "2")]);
}

#[test]
fn initial_cookies_not_sent_cross_origin() {
    let config = Config {
        initial_cookies: vec![pair("a", "1")],
        ..Config::default()
    };
    let mut s = open(TS_URL, config);
    let first = one(&mut s, 0);
    respond(
        &mut s,
        &first,
        302,
        &[location("http://other.test/live.ts")],
    );
    assert_eq!(one(&mut s, 0).cookie(), None);
}

#[test]
fn invalid_initial_cookies_rejected_at_open() {
    for (name, value) in [
        ("a b", "1"),
        ("", "1"),
        ("a=b", "1"),
        ("a", "x;y"),
        ("a", "x\r\ny"),
    ] {
        let config = Config {
            initial_cookies: vec![pair(name, value)],
            ..Config::default()
        };
        assert!(
            matches!(
                Session::open(TS_URL, config, 0),
                Err(broadcast_client::Error::InvalidHeader { .. })
            ),
            "{name:?}={value:?}"
        );
    }
}

#[test]
fn cookies_export_roundtrip_seeds_new_session() {
    let mut s1 = open(TS_URL, Config::default());
    let first = one(&mut s1, 0);
    ok_with_cookies(
        &mut s1,
        &first,
        &["a=1; Path=/", "b=2; Path=/x", "gone=3; Max-Age=1; Path=/"],
    );
    let later = reconnect(&mut s1, 10 * SEC);
    let exported = s1.cookies();
    assert_eq!(exported, vec![pair("a", "1"), pair("b", "2")]);
    assert_eq!(later.cookie(), Some("a=1"));

    let config = Config {
        initial_cookies: exported,
        ..Config::default()
    };
    let mut s2 = open(TS_URL, config);
    let seeded = one(&mut s2, 0);
    assert_eq!(
        seeded.cookie(),
        Some("a=1; b=2"),
        "seeded cookies use path /"
    );
    assert_eq!(s2.cookies(), vec![pair("a", "1"), pair("b", "2")]);
    let mut s3 = open(
        "http://example/x/live.ts",
        Config {
            initial_cookies: s2.cookies(),
            ..Config::default()
        },
    );
    assert_eq!(
        one(&mut s3, 0).cookie(),
        Some("a=1; b=2"),
        "seeded cookies use path /"
    );
}

#[test]
fn cookie_limit_50_evicts_oldest() {
    let lines: Vec<String> = (0..=MAX_JAR)
        .map(|i| format!("c{i}=v{i}; Path=/"))
        .collect();
    let refs: Vec<&str> = lines.iter().map(String::as_str).collect();
    let mut s = session_with_cookies(TS_URL, &refs);
    let jar = s.cookies();
    assert_eq!(jar.len(), MAX_JAR);
    assert_eq!(jar.first(), Some(&pair("c1", "v1")));
    assert_eq!(jar.last(), Some(&pair("c50", "v50")));
    assert!(!jar.iter().any(|(n, _)| n == "c0"));
    let next = reconnect(&mut s, SEC);
    let header = next.cookie().expect("cookie header");
    assert_eq!(header.split("; ").count(), MAX_JAR);
    assert!(!header.contains("c0=v0"));
    assert!(header.starts_with("c1=v1; c2=v2"));
}

#[test]
fn replacing_a_cookie_at_the_limit_evicts_nothing() {
    let lines: Vec<String> = (0..MAX_JAR).map(|i| format!("c{i}=v{i}; Path=/")).collect();
    let refs: Vec<&str> = lines.iter().map(String::as_str).collect();
    let mut s = session_with_cookies(TS_URL, &refs);
    let next = reconnect(&mut s, SEC);
    ok_with_cookies(&mut s, &next, &["c0=new; Path=/"]);
    let jar = s.cookies();
    assert_eq!(jar.len(), MAX_JAR);
    assert_eq!(jar.first(), Some(&pair("c0", "new")));
}

#[test]
fn oversized_cookie_ignored() {
    let at_limit = format!("big={}; Path=/", "x".repeat(MAX_COOKIE_BYTES - "big".len()));
    let over = format!(
        "huge={}; Path=/",
        "y".repeat(MAX_COOKIE_BYTES - "huge".len() + 1)
    );
    let s = session_with_cookies(TS_URL, &[&over, &at_limit, "small=1; Path=/"]);
    let jar = s.cookies();
    let names: Vec<&str> = jar.iter().map(|(n, _)| n.as_str()).collect();
    assert_eq!(names, vec!["big", "small"]);
}

#[test]
fn debug_never_prints_cookie_value() {
    let config = Config {
        initial_cookies: vec![pair("sid", "SEEDSECRET")],
        extra_headers: vec![pair("X-Api-Key", "HEADERSECRET")],
        ..Config::default()
    };
    let config_dbg = format!("{config:?}");
    let mut s = open(TS_URL, config);
    let first = one(&mut s, 0);
    ok_with_cookies(&mut s, &first, &["r=RECVSECRET; Path=/"]);
    let session_dbg = format!("{s:?}");
    let auth = broadcast_client::auth::AuthState::new(
        url::Url::parse(TS_URL).unwrap(),
        vec![pair("X-Api-Key", "HEADERSECRET")],
    );
    let auth_dbg = format!("{auth:?}");
    for dbg in [&config_dbg, &session_dbg, &auth_dbg] {
        for secret in ["SEEDSECRET", "RECVSECRET", "HEADERSECRET"] {
            assert!(!dbg.contains(secret), "{secret} leaked in {dbg}");
        }
    }
    assert!(config_dbg.contains("Config"));
    assert!(session_dbg.contains("Session"));
}

fn cookie_playlist_headers() -> Vec<(String, String)> {
    set_cookies(&["sid=hls; Path=/"])
}

#[test]
fn cookie_applies_to_hls_segments_and_playlists_and_keys_requests() {
    let mut h = Host::new(TS_PLAYLIST_URL, hls_config());
    h.route(
        TS_PLAYLIST_URL,
        vec![Reply::OkHeaders(ts_playlist(), cookie_playlist_headers())],
    );
    h.ok(TS_SEG0_URL, ts_segment(0));
    h.ok(TS_SEG1_URL, ts_segment(1));
    h.run_to_end();
    assert_eq!(h.requests_to(TS_PLAYLIST_URL).len(), 1);
    assert_eq!(
        h.requests_to(TS_PLAYLIST_URL)[0]
            .headers
            .iter()
            .filter(|(k, _)| k.eq_ignore_ascii_case("cookie"))
            .count(),
        0
    );
    for url in [TS_SEG0_URL, TS_SEG1_URL] {
        let reqs = h.requests_to(url);
        assert_eq!(reqs.len(), 1, "{url}");
        let cookies: Vec<&str> = reqs[0]
            .headers
            .iter()
            .filter(|(k, _)| k.eq_ignore_ascii_case("cookie"))
            .map(|(_, v)| v.as_str())
            .collect();
        assert_eq!(cookies, vec!["sid=hls"], "{url}");
    }
    assert_eq!(h.session.cookies(), vec![pair("sid", "hls")]);
}

#[test]
fn hls_playlist_reload_and_init_and_cross_origin_cookie_scope() {
    let playlist = format!(
        "#EXTM3U\n#EXT-X-VERSION:3\n#EXT-X-TARGETDURATION:2\n#EXT-X-MEDIA-SEQUENCE:0\n\
         #EXT-X-PLAYLIST-TYPE:VOD\n#EXTINF:2.0,\nindex0.ts\n#EXTINF:1.0,\n{CDN_SEG_URL}\n\
         #EXT-X-ENDLIST\n"
    );
    let config = Config {
        initial_cookies: vec![pair("sid", "seed")],
        ..hls_config()
    };
    let mut h = Host::new(TS_PLAYLIST_URL, config);
    h.ok(TS_PLAYLIST_URL, playlist.into_bytes());
    h.ok(TS_SEG0_URL, ts_segment(0));
    h.ok(CDN_SEG_URL, ts_segment(1));
    h.run_to_end();
    let has_cookie = |s: &Seen| {
        s.headers
            .iter()
            .any(|(k, _)| k.eq_ignore_ascii_case("cookie"))
    };
    assert!(has_cookie(h.requests_to(TS_PLAYLIST_URL)[0]));
    assert!(has_cookie(h.requests_to(TS_SEG0_URL)[0]));
    assert!(
        !has_cookie(h.requests_to(CDN_SEG_URL)[0]),
        "no cookie to the CDN origin"
    );
}

#[test]
fn hls_cookie_survives_session_reconnect() {
    let mut h = Host::new(TS_PLAYLIST_URL, hls_config());
    h.route(
        TS_PLAYLIST_URL,
        vec![Reply::OkHeaders(ts_playlist(), cookie_playlist_headers())],
    );
    h.ok(TS_SEG0_URL, ts_segment(0));
    h.ok(TS_SEG1_URL, ts_segment(1));
    h.run_to_end();
    let before = h.seen.len();
    h.session.reconnect(h.now);
    h.collect();
    assert!(h.seen.len() > before);
    let reconnected = h.seen.last().expect("request after reconnect");
    assert_eq!(reconnected.url, TS_PLAYLIST_URL);
    assert!(
        reconnected
            .headers
            .iter()
            .any(|(k, v)| k.eq_ignore_ascii_case("cookie") && v == "sid=hls")
    );
}

#[test]
fn cookie_survives_reconnect() {
    let mut s = open(TS_URL, Config::default());
    let first = one(&mut s, 0);
    ok_with_cookies(&mut s, &first, &["sid=keep; Path=/"]);
    s.on_closed(first.conn, Some("reset".to_string()), SEC);
    let mut found = None;
    for t in 1..=MAX_POLL_SECS {
        let mut reqs = requests(&mut s, t * SEC);
        if !reqs.is_empty() {
            found = Some(reqs.remove(0));
            break;
        }
    }
    let again = found.expect("the source reconnects after a transport error");
    assert_eq!(again.cookie(), Some("sid=keep"));
    assert_eq!(again.count("cookie"), 1);
}

#[test]
fn cookie_header_does_not_leak_into_auth_decision() {
    let config = Config {
        initial_cookies: vec![pair("sid", "1")],
        ..Config::default()
    };
    let mut s = open(TS_URL, config);
    s.set_credentials(Credentials::new("user", "pass"));
    let first = one(&mut s, 0);
    assert_eq!(first.cookie(), Some("sid=1"));
    assert_eq!(first.header("authorization"), None);
    respond(
        &mut s,
        &first,
        401,
        &[(
            "WWW-Authenticate".to_string(),
            "Basic realm=\"tv\"".to_string(),
        )],
    );
    let replay = one(&mut s, 0);
    assert_eq!(replay.header("authorization"), Some("Basic dXNlcjpwYXNz"));
    assert_eq!(replay.cookie(), Some("sid=1"));
    let mut rejected = false;
    while let Some(e) = s.next_event() {
        rejected |= matches!(e, Event::AuthRejected { .. } | Event::AuthRequired { .. });
    }
    assert!(
        !rejected,
        "a Cookie header must not count as a rejected Authorization"
    );

    let mut plain = open(TS_URL, Config::default());
    plain.set_credentials(Credentials::new("user", "pass"));
    let p1 = one(&mut plain, 0);
    respond(
        &mut plain,
        &p1,
        401,
        &[(
            "WWW-Authenticate".to_string(),
            "Basic realm=\"tv\"".to_string(),
        )],
    );
    assert_eq!(
        one(&mut plain, 0).header("authorization"),
        replay.header("authorization")
    );
}

#[test]
fn set_cookie_header_name_is_case_insensitive_and_garbage_lines_ignored() {
    let mut s = open(TS_URL, Config::default());
    let first = one(&mut s, 0);
    let headers = vec![
        ("SET-COOKIE".to_string(), "up=1; Path=/".to_string()),
        ("set-cookie".to_string(), "no-equals-sign".to_string()),
        ("Set-Cookie".to_string(), "=emptyname".to_string()),
        ("X-Other".to_string(), "x=1".to_string()),
    ];
    respond(&mut s, &first, 200, &headers);
    assert_eq!(s.cookies(), vec![pair("up", "1")]);
}
