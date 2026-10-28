mod support;

use broadcast_client::{Config, Credentials, Error, Session};
use support::hls_host::*;
use support::ts_probe::{self, *};

const CDN_SEG_URL: &str = "http://cdn.test/seg/index1.ts";
const RESERVED_SPELLINGS: [&str; 14] = [
    "Authorization",
    "AUTHORIZATION",
    "Cookie",
    "cOOkie",
    "Host",
    "HOST",
    "Content-Length",
    "content-length",
    "Transfer-Encoding",
    "TRANSFER-ENCODING",
    "Connection",
    "connection",
    "Range",
    "RANGE",
];

fn pair(n: &str, v: &str) -> (String, String) {
    (n.to_string(), v.to_string())
}

fn with_headers(headers: Vec<(String, String)>) -> Config {
    {
        let mut new_config = Config::default();
        new_config.extra_headers = headers;
        new_config
    }
}

fn open_err(headers: Vec<(String, String)>) -> Error {
    Session::open(TS_URL, with_headers(headers), 0).expect_err("open must fail")
}

fn hls_with_headers() -> Config {
    {
        let mut new_config = hls_config();
        new_config.extra_headers = vec![pair("X-Trace", "t-1")];
        new_config
    }
}

#[test]
fn extra_headers_sent_on_every_origin_request() {
    let mut s = ts_probe::open(TS_URL, with_headers(vec![pair("X-Trace", "t-1")]));
    let first = one(&mut s, 0);
    assert_eq!(first.header("x-trace"), Some("t-1"));
    ts_probe::respond(&mut s, &first, 200, &[]);
    let second = reconnect(&mut s, SEC);
    assert_eq!(second.header("x-trace"), Some("t-1"));

    let mut redirected = ts_probe::open(TS_URL, with_headers(vec![pair("X-Trace", "t-1")]));
    let r1 = one(&mut redirected, 0);
    ts_probe::respond(
        &mut redirected,
        &r1,
        302,
        &[location("http://example/next.ts")],
    );
    assert_eq!(one(&mut redirected, 0).header("x-trace"), Some("t-1"));

    let mut h = Host::new(TS_PLAYLIST_URL, hls_with_headers());
    h.ok(TS_PLAYLIST_URL, ts_playlist());
    h.ok(TS_SEG0_URL, ts_segment(0));
    h.ok(TS_SEG1_URL, ts_segment(1));
    h.run_to_end();
    assert_eq!(h.seen.len(), 3);
    for seen in &h.seen {
        let values: Vec<&str> = seen
            .headers
            .iter()
            .filter(|(k, _)| k == "X-Trace")
            .map(|(_, v)| v.as_str())
            .collect();
        assert_eq!(values, vec!["t-1"], "{}", seen.url);
    }
}

#[test]
fn extra_headers_not_sent_cross_origin() {
    let config = with_headers(vec![pair("X-Trace", "t-1")]);
    let mut s = ts_probe::open(TS_URL, config);
    let first = one(&mut s, 0);
    assert_eq!(first.header("x-trace"), Some("t-1"));
    ts_probe::respond(
        &mut s,
        &first,
        302,
        &[location("http://other.test/live.ts")],
    );
    let other = one(&mut s, 0);
    assert_eq!(other.url, "http://other.test/live.ts");
    assert_eq!(other.header("x-trace"), None);
    assert!(other.headers.is_empty(), "{:?}", other.headers);

    let playlist = format!(
        "#EXTM3U\n#EXT-X-VERSION:3\n#EXT-X-TARGETDURATION:2\n#EXT-X-MEDIA-SEQUENCE:0\n\
         #EXT-X-PLAYLIST-TYPE:VOD\n#EXTINF:2.0,\nindex0.ts\n#EXTINF:1.0,\n{CDN_SEG_URL}\n\
         #EXT-X-ENDLIST\n"
    );
    let mut h = Host::new(TS_PLAYLIST_URL, hls_with_headers());
    h.ok(TS_PLAYLIST_URL, playlist.into_bytes());
    h.ok(TS_SEG0_URL, ts_segment(0));
    h.ok(CDN_SEG_URL, ts_segment(1));
    h.run_to_end();
    let cdn = h.requests_to(CDN_SEG_URL);
    assert_eq!(cdn.len(), 1);
    assert!(cdn[0].headers.is_empty(), "{:?}", cdn[0].headers);
    assert!(
        h.requests_to(TS_SEG0_URL)[0]
            .headers
            .iter()
            .any(|(k, _)| k == "X-Trace")
    );
}

#[test]
fn reserved_names_rejected_at_open() {
    for name in RESERVED_SPELLINGS {
        match open_err(vec![pair(name, "x")]) {
            Error::ReservedHeader { name: got } => assert_eq!(got, name),
            other => panic!("{name}: unexpected {other:?}"),
        }
    }
}

#[test]
fn reserved_name_among_valid_ones_rejects_the_whole_open() {
    let err = open_err(vec![
        pair("X-A", "1"),
        pair("Range", "bytes=0-"),
        pair("X-B", "2"),
    ]);
    assert!(matches!(err, Error::ReservedHeader { ref name } if name == "Range"));
}

#[test]
fn invalid_header_name_rejected() {
    for name in ["bad name", "", "x:y", "caf\u{e9}", "a\nb", "(paren)"] {
        match open_err(vec![pair(name, "v")]) {
            Error::InvalidHeader { name: got } => assert_eq!(got, name),
            other => panic!("{name:?}: unexpected {other:?}"),
        }
    }
}

#[test]
fn invalid_header_value_with_crlf_rejected() {
    for value in [
        "a\r\nInjected: 1",
        "line\nfeed",
        "carriage\rreturn",
        "nul\0byte",
    ] {
        match open_err(vec![pair("X-Ok", value)]) {
            Error::InvalidHeader { name } => assert_eq!(name, "X-Ok"),
            other => panic!("{value:?}: unexpected {other:?}"),
        }
    }
}

#[test]
fn valid_headers_accepted_at_open() {
    let config = with_headers(vec![
        pair("X-Api-Key", "k e y"),
        pair("User-Agent", "player/1.0 (tv)"),
        pair("X-Empty", ""),
    ]);
    let mut s = ts_probe::open(TS_URL, config);
    let first = one(&mut s, 0);
    assert_eq!(first.header("x-api-key"), Some("k e y"));
    assert_eq!(first.header("user-agent"), Some("player/1.0 (tv)"));
    assert_eq!(first.header("x-empty"), Some(""));
}

#[test]
fn extra_headers_never_override_session_cookie_or_authorization() {
    let config = {
        let mut new_config = Config::default();
        new_config.extra_headers = vec![pair("X-Keep", "1")];
        new_config.initial_cookies = vec![pair("sid", "real")];
        new_config
    };
    let mut s = ts_probe::open(TS_URL, config);
    s.set_credentials(Credentials::bearer("tok")).unwrap();
    let req = one(&mut s, 0);
    assert_eq!(req.count("cookie"), 1);
    assert_eq!(req.count("authorization"), 1);
    assert_eq!(req.cookie(), Some("sid=real"));
    assert_eq!(req.header("authorization"), Some("Bearer tok"));
}

#[test]
fn extra_headers_order_preserved() {
    let config = {
        let mut new_config = Config::default();
        new_config.extra_headers = vec![pair("X-B", "2"), pair("X-A", "1"), pair("X-C", "3")];
        new_config.initial_cookies = vec![pair("sid", "1")];
        new_config
    };
    let mut s = ts_probe::open(TS_URL, config);
    let req = one(&mut s, 0);
    let names: Vec<&str> = req.headers.iter().map(|(k, _)| k.as_str()).collect();
    assert_eq!(names, vec!["X-B", "X-A", "X-C", "Cookie"]);
}

#[test]
fn debug_redacts_extra_header_values() {
    let config = with_headers(vec![pair("X-Api-Key", "SUPERSECRETVALUE")]);
    let dbg = format!("{config:?}");
    assert!(!dbg.contains("SUPERSECRETVALUE"), "{dbg}");
    assert!(dbg.contains("X-Api-Key"), "{dbg}");
}
