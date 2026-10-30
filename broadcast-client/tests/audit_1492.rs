mod support;

use broadcast_client::{Action, Config, Credentials, Event, Session};
use support::ts_probe::{Req, TS_URL, location, one, open, respond, set_cookies};

const SEC: u64 = 1_000_000;
const OTHER_ORIGIN_URL: &str = "http://other.example/x.ts";
const REPLAY_BOUND: usize = 4;
const LOOP_PROBE: usize = 50;
const MAX_COOKIE_HEADER: usize = 16 * 1024;
const COOKIE_VALUE_BYTES: usize = 4_000;
const COOKIE_COUNT: usize = 50;
const BASIC_USER_PASS: &str = "Basic dXNlcjpwQHNz";
const BASIC_OTHER_PW: &str = "Basic b3RoZXI6cHc=";
const BASIC_CHALLENGE: &str = "Basic realm=\"r\"";

fn drain(session: &mut Session, now: u64) -> Vec<Action> {
    let mut out = Vec::new();
    while let Some(a) = session.poll(now) {
        out.push(a);
    }
    out
}

fn events(session: &mut Session) -> Vec<Event> {
    let mut out = Vec::new();
    while let Some(e) = session.next_event() {
        out.push(e);
    }
    out
}

fn closes(actions: &[Action]) -> Vec<u32> {
    actions
        .iter()
        .filter_map(|a| match a {
            Action::Close { conn } => Some(*conn),
            _ => None,
        })
        .collect()
}

fn http_requests(actions: &[Action]) -> Vec<Req> {
    actions
        .iter()
        .filter_map(|a| match a {
            Action::HttpRequest {
                conn, url, headers, ..
            } => Some(Req {
                now: 0,
                conn: *conn,
                url: url.clone(),
                headers: headers.clone(),
            }),
            _ => None,
        })
        .collect()
}

fn challenge(line: &str) -> Vec<(String, String)> {
    vec![("WWW-Authenticate".to_string(), line.to_string())]
}

fn cross_origin_401_chain(session: &mut Session) -> (usize, Vec<Action>) {
    let first = one(session, 0);
    respond(session, &first, 302, &[location(OTHER_ORIGIN_URL)]);
    let mut req = one(session, 0);
    let mut sent = 1;
    let mut all = Vec::new();
    for _ in 0..LOOP_PROBE {
        respond(session, &req, 401, &challenge(BASIC_CHALLENGE));
        let actions = drain(session, 0);
        let next = http_requests(&actions);
        all.extend(actions);
        match next.into_iter().next() {
            Some(r) => {
                sent += 1;
                req = r;
            }
            None => break,
        }
    }
    (sent, all)
}

#[test]
fn a_401_from_a_redirect_target_on_another_origin_does_not_loop_with_credentials() {
    let mut session = open(TS_URL, Config::default());
    session
        .set_credentials(Credentials::new("user", "pass"))
        .expect("credentials");
    let (sent, actions) = cross_origin_401_chain(&mut session);
    assert!(
        sent < REPLAY_BOUND,
        "{sent} requests answered by a foreign origin's 401 with no delay"
    );
    assert!(
        actions
            .iter()
            .any(|a| matches!(a, Action::WakeAt(at) if *at > 0)),
        "the retry must be delayed by backoff: {actions:?}"
    );
}

#[test]
fn a_401_from_another_origin_never_asks_for_credentials_of_the_session_origin() {
    let mut session = open(TS_URL, Config::default());
    let (sent, _) = cross_origin_401_chain(&mut session);
    assert!(sent < REPLAY_BOUND, "{sent}");
    let evs = events(&mut session);
    assert!(
        !evs.iter().any(|e| matches!(e, Event::AuthRequired { .. })),
        "{evs:?}"
    );
}

#[test]
fn every_non_success_final_status_closes_the_connection_once() {
    for status in [404u16, 410, 429, 500, 502, 503] {
        let mut session = open(TS_URL, Config::default());
        let first = one(&mut session, 0);
        respond(&mut session, &first, status, &[]);
        let actions = drain(&mut session, 0);
        assert_eq!(closes(&actions), vec![first.conn], "{status}: {actions:?}");
    }
}

fn refused_with_body(
    session: &mut Session,
    status: u16,
    headers: &[(String, String)],
) -> (Req, Vec<Action>) {
    let first = one(session, 0);
    respond(session, &first, status, headers);
    let at_response = drain(session, 0);
    (first, at_response)
}

fn text_plain(len: usize) -> Vec<(String, String)> {
    vec![
        ("Content-Type".to_string(), "text/plain".to_string()),
        ("Content-Length".to_string(), len.to_string()),
    ]
}

#[test]
fn a_refused_response_that_needs_no_body_closes_at_the_response() {
    for status in [429u16, 503] {
        let mut session = open(TS_URL, Config::default());
        let json = vec![("Content-Type".to_string(), "application/json".to_string())];
        let (first, actions) = refused_with_body(&mut session, status, &json);
        assert_eq!(closes(&actions), vec![first.conn], "{status}: {actions:?}");

        let mut session = open(TS_URL, Config::default());
        let (first, actions) = refused_with_body(&mut session, status, &text_plain(0));
        assert_eq!(
            closes(&actions),
            vec![first.conn],
            "{status} empty: {actions:?}"
        );
    }
}

#[test]
fn a_refused_text_plain_response_closes_once_its_body_is_complete() {
    for status in [429u16, 503] {
        let body = b"busy now";
        let mut session = open(TS_URL, Config::default());
        let (first, at_response) = refused_with_body(&mut session, status, &text_plain(body.len()));
        assert!(closes(&at_response).is_empty(), "{status}: {at_response:?}");
        session.on_data(first.conn, &body[..3], 1);
        assert!(closes(&drain(&mut session, 1)).is_empty());
        session.on_data(first.conn, &body[3..], 2);
        let after = drain(&mut session, 2);
        assert_eq!(closes(&after), vec![first.conn], "{status}: {after:?}");
        session.on_closed(first.conn, None, 3);
        assert!(
            closes(&drain(&mut session, 3)).is_empty(),
            "exactly one Close"
        );
    }
}

#[test]
fn a_refused_text_plain_response_without_a_length_closes_when_the_host_ends_it() {
    let mut session = open(TS_URL, Config::default());
    let headers = vec![("Content-Type".to_string(), "text/plain".to_string())];
    let (first, at_response) = refused_with_body(&mut session, 503, &headers);
    assert!(closes(&at_response).is_empty());
    session.on_data(first.conn, b"busy", 1);
    session.on_closed(first.conn, None, 2);
    assert!(
        closes(&drain(&mut session, 2)).is_empty(),
        "the host already closed it"
    );
}

#[test]
fn a_refused_text_plain_body_longer_than_the_cap_closes_when_the_cap_is_reached() {
    let cap = broadcast_client::MAX_SERVER_REFUSED_REASON_BYTES;
    let mut session = open(TS_URL, Config::default());
    let (first, at_response) = refused_with_body(&mut session, 503, &text_plain(cap * 10));
    assert!(closes(&at_response).is_empty());
    session.on_data(first.conn, &vec![b'x'; cap - 1], 1);
    assert!(closes(&drain(&mut session, 1)).is_empty());
    session.on_data(first.conn, b"x", 2);
    let after = drain(&mut session, 2);
    assert_eq!(closes(&after), vec![first.conn], "{after:?}");
    session.on_data(first.conn, &vec![b'x'; cap], 3);
    assert!(
        closes(&drain(&mut session, 3)).is_empty(),
        "exactly one Close"
    );
}

#[test]
fn a_redirect_to_a_non_http_scheme_is_a_connect_failure_with_no_new_request() {
    let mut session = open(TS_URL, Config::default());
    let first = one(&mut session, 0);
    respond(&mut session, &first, 302, &[location("file:///etc/passwd")]);
    let actions = drain(&mut session, 0);
    assert!(http_requests(&actions).is_empty(), "{actions:?}");
    assert_eq!(closes(&actions), vec![first.conn], "{actions:?}");
}

#[test]
fn the_total_cookie_header_is_capped() {
    let mut session = open(TS_URL, Config::default());
    let first = one(&mut session, 0);
    let lines: Vec<String> = (0..COOKIE_COUNT)
        .map(|n| format!("c{n}={}", "a".repeat(COOKIE_VALUE_BYTES)))
        .collect();
    let refs: Vec<&str> = lines.iter().map(String::as_str).collect();
    respond(&mut session, &first, 200, &set_cookies(&refs));
    session.reconnect(SEC);
    let req = one(&mut session, SEC);
    let cookie = req.cookie().expect("some cookies are sent");
    assert!(
        cookie.len() <= MAX_COOKIE_HEADER,
        "Cookie header is {} bytes",
        cookie.len()
    );
    assert!(cookie.len() > COOKIE_VALUE_BYTES, "cookies are still sent");
}

fn first_cookie_header(url: &str, cookies: &[(&str, &str)]) -> Option<String> {
    let config = {
        let mut new_config = Config::default();
        new_config.initial_cookies = cookies
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect();
        new_config
    };
    let mut session = open(url, config);
    one(&mut session, 0).cookie().map(str::to_string)
}

#[test]
fn seeded_cookies_obey_the_prefix_rules() {
    let seeded = [("__Secure-a", "1"), ("__Host-b", "2"), ("plain", "3")];
    assert_eq!(
        first_cookie_header("http://example/live.ts", &seeded).as_deref(),
        Some("plain=3"),
        "prefixed cookies need a secure origin"
    );
    let secure = first_cookie_header("https://example/live.ts", &seeded).expect("cookies");
    for expected in ["__Secure-a=1", "__Host-b=2", "plain=3"] {
        assert!(secure.contains(expected), "{secure}");
    }
}

#[test]
fn a_seeded_cookie_over_the_size_limit_is_dropped() {
    let big = "a".repeat(COOKIE_VALUE_BYTES + 100);
    let header = first_cookie_header("http://example/live.ts", &[("big", &big), ("ok", "1")]);
    assert_eq!(header.as_deref(), Some("ok=1"));
}

#[test]
fn userinfo_in_an_http_url_becomes_credentials_and_leaves_the_request_url() {
    let mut session = open("http://user:p%40ss@example/live.ts", Config::default());
    let first = one(&mut session, 0);
    assert_eq!(first.url, TS_URL);
    assert!(first.header("authorization").is_none());
    respond(&mut session, &first, 401, &challenge(BASIC_CHALLENGE));
    let replay = one(&mut session, 0);
    assert_eq!(replay.url, TS_URL);
    assert_eq!(replay.header("authorization"), Some(BASIC_USER_PASS));
}

#[test]
fn explicit_credentials_win_over_url_userinfo() {
    let mut session = open("http://user:p%40ss@example/live.ts", Config::default());
    session
        .set_credentials(Credentials::new("other", "pw"))
        .expect("credentials");
    let first = one(&mut session, 0);
    respond(&mut session, &first, 401, &challenge(BASIC_CHALLENGE));
    let replay = one(&mut session, 0);
    assert_eq!(replay.header("authorization"), Some(BASIC_OTHER_PW));
}
