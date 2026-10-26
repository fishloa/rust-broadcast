mod support;

use std::collections::BTreeSet;

use broadcast_client::{Action, Config, Credentials, Event, Session, Status};
use support::ts_probe::{TS_URL, location, one, open, requests, respond, set_cookies};

const SEC: u64 = 1_000_000;
const HTTPS_URL: &str = "https://example/live.ts";
const COOKIE_SECRET: &str = "cookie-secret-77";
const BEARER_SECRET: &str = "bearer-secret-88";
const BASIC_B64_USER_PASS: &str = "dXNlcjpwYXNz";
const DIGEST_SECRET: &str = "deadbeefcafe";
const STALE_DIGEST: &str = "Digest realm=\"r\", nonce=\"n1\", qop=\"auth\", stale=true";
const MAX_COOKIE_PATH: usize = 1024;

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

fn challenge(line: &str) -> Vec<(String, String)> {
    vec![("WWW-Authenticate".to_string(), line.to_string())]
}

#[test]
fn action_debug_prints_header_names_never_values() {
    let config = {
        let mut new_config = Config::default();
        new_config.initial_cookies = vec![("sid".to_string(), COOKIE_SECRET.to_string())];
        new_config
    };
    let mut session = open(TS_URL, config);
    session
        .set_credentials(Credentials::bearer(BEARER_SECRET))
        .ok();
    let action = session.poll(0).expect("request");
    let dbg = format!("{action:?}");
    assert!(!dbg.contains(BEARER_SECRET), "{dbg}");
    assert!(!dbg.contains(COOKIE_SECRET), "{dbg}");
    assert!(dbg.contains("Authorization"), "{dbg}");
    assert!(dbg.contains("Cookie"), "{dbg}");
    assert!(dbg.contains("GET"), "{dbg}");
    assert!(dbg.contains("example/live.ts"), "{dbg}");
    assert!(dbg.contains("conn"), "{dbg}");
}

#[test]
fn action_debug_hides_basic_authorization_after_challenge() {
    let mut session = open(TS_URL, Config::default());
    let first = one(&mut session, 0);
    session
        .set_credentials(Credentials::new("user", "pass"))
        .ok();
    respond(&mut session, &first, 401, &challenge("Basic realm=\"r\""));
    let replay = requests(&mut session, 0);
    let authed = replay
        .iter()
        .find(|r| r.header("authorization").is_some())
        .expect("authorized replay");
    assert_eq!(
        authed.header("authorization"),
        Some(format!("Basic {BASIC_B64_USER_PASS}").as_str())
    );
    session.reconnect(1);
    for action in drain(&mut session, 1) {
        let dbg = format!("{action:?}");
        assert!(!dbg.contains(BASIC_B64_USER_PASS), "{dbg}");
    }
}

#[test]
fn send_action_debug_does_not_dump_bytes() {
    let bytes = format!(
        "DESCRIBE rtsp://h/x RTSP/1.0\r\nAuthorization: Digest response=\"{DIGEST_SECRET}\"\r\n\r\n"
    )
    .into_bytes();
    let len = bytes.len();
    let dbg = format!("{:?}", Action::Send { conn: 3, bytes });
    assert!(!dbg.contains(DIGEST_SECRET), "{dbg}");
    assert!(dbg.contains(&len.to_string()), "{dbg}");
    assert!(dbg.contains("Send"), "{dbg}");
}

#[test]
fn forbidden_closes_the_connection() {
    let mut session = open(TS_URL, Config::default());
    let first = one(&mut session, 0);
    respond(&mut session, &first, 403, &[]);
    let actions = drain(&mut session, 0);
    assert_eq!(closes(&actions), vec![first.conn], "{actions:?}");
}

#[test]
fn redirect_without_location_closes_the_connection_once() {
    let mut session = open(TS_URL, Config::default());
    let first = one(&mut session, 0);
    respond(&mut session, &first, 302, &[]);
    let actions = drain(&mut session, 0);
    assert_eq!(closes(&actions), vec![first.conn], "{actions:?}");
}

#[test]
fn redirect_over_the_limit_closes_the_connection_once() {
    let config = {
        let mut new_config = Config::default();
        new_config.max_redirects = 0;
        new_config
    };
    let mut session = open(TS_URL, config);
    let first = one(&mut session, 0);
    respond(
        &mut session,
        &first,
        302,
        &[location("http://example/elsewhere.ts")],
    );
    let actions = drain(&mut session, 0);
    assert_eq!(closes(&actions), vec![first.conn], "{actions:?}");
}

#[test]
fn reconnect_before_any_response_closes_old_conn_and_late_200_is_ignored() {
    let mut session = open(TS_URL, Config::default());
    let old = one(&mut session, 0);
    session.reconnect(SEC);
    let after = drain(&mut session, SEC);
    assert!(closes(&after).contains(&old.conn), "{after:?}");
    let new_conn = after
        .iter()
        .find_map(|a| match a {
            Action::HttpRequest { conn, .. } => Some(*conn),
            _ => None,
        })
        .expect("reconnect request");
    assert_ne!(new_conn, old.conn);
    events(&mut session);

    session.on_response(old.conn, 200, &[], 0);
    session.on_data(old.conn, &[0x47; 188], SEC);
    let late = events(&mut session);
    assert!(
        !late
            .iter()
            .any(|e| matches!(e, Event::Status(Status::Establishing))),
        "a response for a closed connection took over: {late:?}"
    );

    session.on_response(new_conn, 200, &[], 0);
    let live = events(&mut session);
    assert_eq!(
        live.iter()
            .filter(|e| matches!(e, Event::Status(Status::Establishing)))
            .count(),
        1,
        "{live:?}"
    );
}

#[test]
fn close_drops_queued_requests_and_closes_each_conn_once() {
    let mut session = open(TS_URL, Config::default());
    let first = one(&mut session, 0);
    respond(
        &mut session,
        &first,
        302,
        &[location("http://example/moved.ts")],
    );
    session.close();
    let actions = drain(&mut session, 0);
    assert!(
        actions.iter().all(|a| matches!(a, Action::Close { .. })),
        "{actions:?}"
    );
    let mut conns = closes(&actions);
    conns.sort_unstable();
    assert_eq!(conns.len(), 2, "{actions:?}");
    assert_eq!(
        conns.iter().copied().collect::<BTreeSet<_>>().len(),
        2,
        "each conn closed once: {actions:?}"
    );
    assert!(conns.contains(&first.conn));
    assert!(session.poll(SEC).is_none());
}

#[test]
fn stale_rechallenge_is_replayed_at_most_once_then_rejected() {
    let mut session = open(TS_URL, Config::default());
    session
        .set_credentials(Credentials::new("user", "pass"))
        .ok();
    let mut sent = 0usize;
    let mut pending = requests(&mut session, 0);
    let mut seen = Vec::new();
    for _ in 0..20 {
        let Some(req) = pending.pop() else { break };
        sent += 1;
        respond(&mut session, &req, 401, &challenge(STALE_DIGEST));
        pending = requests(&mut session, 0);
        seen.extend(events(&mut session));
    }
    assert!(sent <= 3, "unbounded stale replay: {sent} requests");
    assert!(
        seen.iter()
            .any(|e| matches!(e, Event::AuthRejected { status: 401 })),
        "{seen:?}"
    );
    assert!(
        seen.iter()
            .any(|e| matches!(e, Event::Status(Status::WaitingForCredentials)))
    );
}

fn tracked_wakes(session: &mut Session, now: u64, wakes: &mut BTreeSet<u64>) -> Vec<Action> {
    let actions = drain(session, now);
    for a in &actions {
        if let Action::WakeAt(t) = a {
            wakes.insert(*t);
        }
    }
    actions
}

fn wake_only_run(
    session: &mut Session,
    wakes: &mut BTreeSet<u64>,
    max_wakes: usize,
) -> (Vec<Action>, Vec<Event>) {
    let mut all = Vec::new();
    let mut evs = Vec::new();
    let mut now = 0u64;
    for _ in 0..max_wakes {
        let Some(next) = wakes.iter().copied().find(|t| *t > now) else {
            break;
        };
        now = next;
        all.extend(tracked_wakes(session, now, wakes));
        evs.extend(events(session));
    }
    (all, evs)
}

#[test]
fn wake_only_host_sees_start_timeout() {
    let mut session = open(TS_URL, Config::default());
    let mut wakes = BTreeSet::new();
    let first = tracked_wakes(&mut session, 0, &mut wakes);
    let first_conn = first
        .iter()
        .find_map(|a| match a {
            Action::HttpRequest { conn, .. } => Some(*conn),
            _ => None,
        })
        .expect("first request");
    session.on_response(first_conn, 200, &[], SEC);
    tracked_wakes(&mut session, 0, &mut wakes);
    let (actions, evs) = wake_only_run(&mut session, &mut wakes, 4);
    assert!(closes(&actions).contains(&first_conn), "{actions:?}");
    assert!(
        evs.iter()
            .any(|e| matches!(e, Event::Status(Status::Reconnecting { .. }))),
        "{evs:?}"
    );
    assert!(
        actions
            .iter()
            .any(|a| matches!(a, Action::HttpRequest { .. })),
        "{actions:?}"
    );
}

#[test]
fn wake_only_host_sees_stall_notice_and_stall_reconnect() {
    let bytes = support::read_tf1_bytes();
    let (status, headers) = support::read_tf1_headers();
    let mut session = open(TS_URL, Config::default());
    let mut wakes = BTreeSet::new();
    let first = tracked_wakes(&mut session, 0, &mut wakes);
    let conn = first
        .iter()
        .find_map(|a| match a {
            Action::HttpRequest { conn, .. } => Some(*conn),
            _ => None,
        })
        .expect("first request");
    session.on_response(conn, status, &headers, SEC);
    for chunk in bytes.chunks(65_536) {
        session.on_data(conn, chunk, 0);
    }
    let started = events(&mut session);
    assert!(
        started.iter().any(|e| matches!(e, Event::Sample { .. })),
        "fixture produced no samples"
    );
    tracked_wakes(&mut session, 0, &mut wakes);
    let (actions, evs) = wake_only_run(&mut session, &mut wakes, 60);
    assert!(
        evs.iter()
            .any(|e| matches!(e, Event::Status(Status::Stalled { .. }))),
        "no stall notice reached a wake-only host: {evs:?}"
    );
    assert!(closes(&actions).contains(&conn), "{actions:?}");
    assert!(
        actions
            .iter()
            .any(|a| matches!(a, Action::HttpRequest { .. })),
        "no reconnect request: {actions:?}"
    );
}

#[test]
fn failed_first_open_then_success_is_not_a_discontinuity() {
    let mut session = open(TS_URL, Config::default());
    let first = one(&mut session, 0);
    respond(&mut session, &first, 503, &[]);
    drain(&mut session, 0);
    let retry_at = 3 * SEC;
    let second = one(&mut session, retry_at);
    events(&mut session);
    respond(&mut session, &second, 200, &[]);
    let evs = events(&mut session);
    assert!(
        evs.iter()
            .any(|e| matches!(e, Event::Status(Status::Establishing))),
        "{evs:?}"
    );
    assert!(
        !evs.iter().any(|e| matches!(e, Event::Discontinuity)),
        "{evs:?}"
    );
}

#[test]
fn reconnect_after_a_connected_stream_is_a_discontinuity() {
    let mut session = open(TS_URL, Config::default());
    let first = one(&mut session, 0);
    respond(&mut session, &first, 200, &[]);
    events(&mut session);
    session.reconnect(SEC);
    let second = one(&mut session, SEC);
    respond(&mut session, &second, 200, &[]);
    let evs = events(&mut session);
    assert!(
        evs.iter().any(|e| matches!(e, Event::Discontinuity)),
        "{evs:?}"
    );
}

#[test]
fn reconnect_reopens_the_original_url_not_the_redirect_target() {
    let mut session = open(TS_URL, Config::default());
    let first = one(&mut session, 0);
    respond(
        &mut session,
        &first,
        302,
        &[location("http://cdn.example/signed/live.ts?sig=1")],
    );
    let redirected = one(&mut session, 0);
    assert_eq!(redirected.url, "http://cdn.example/signed/live.ts?sig=1");
    respond(&mut session, &redirected, 200, &[]);
    session.reconnect(SEC);
    let again = one(&mut session, SEC);
    assert_eq!(again.url, TS_URL);
}

#[test]
fn authenticated_replay_after_redirect_targets_the_redirected_url() {
    let mut session = open(TS_URL, Config::default());
    session
        .set_credentials(Credentials::new("user", "pass"))
        .ok();
    let first = one(&mut session, 0);
    respond(
        &mut session,
        &first,
        302,
        &[location("http://example/other/live.ts")],
    );
    let redirected = one(&mut session, 0);
    respond(
        &mut session,
        &redirected,
        401,
        &challenge("Basic realm=\"r\""),
    );
    let replay = requests(&mut session, 0);
    let authed = replay
        .iter()
        .find(|r| r.header("authorization").is_some())
        .expect("replay");
    assert_eq!(authed.url, "http://example/other/live.ts");
}

#[test]
fn invalid_bearer_token_never_reaches_a_header() {
    let mut session = open(TS_URL, Config::default());
    let result = session.set_credentials(Credentials::bearer("tok\r\nX-Evil: 1"));
    let actions = drain(&mut session, 0);
    for a in &actions {
        if let Action::HttpRequest { headers, .. } = a {
            for (_, v) in headers {
                assert!(!v.contains('\n') && !v.contains('\r'), "{v:?}");
            }
        }
    }
    assert!(
        matches!(result, Err(broadcast_client::Error::InvalidCredentials)),
        "{result:?}"
    );
}

#[test]
fn control_characters_in_basic_credentials_are_rejected() {
    let mut session = open(TS_URL, Config::default());
    let result = session.set_credentials(Credentials::new("us\ner", "pass"));
    assert!(
        matches!(result, Err(broadcast_client::Error::InvalidCredentials)),
        "{result:?}"
    );
    let ok = session.set_credentials(Credentials::new("user", "pass"));
    assert!(ok.is_ok());
}

#[test]
fn cookie_name_with_control_character_is_rejected_at_open() {
    for name in ["a\u{1}b", "a\u{7f}b", "a\u{0}b", "a(b", "a\"b"] {
        let config = {
            let mut new_config = Config::default();
            new_config.initial_cookies = vec![(name.to_string(), "v".to_string())];
            new_config
        };
        let result = Session::open(TS_URL, config, 0);
        assert!(
            matches!(result, Err(broadcast_client::Error::InvalidHeader { .. })),
            "{name:?} accepted"
        );
    }
    let ok = {
        let mut new_config = Config::default();
        new_config.initial_cookies = vec![("good-name_1".to_string(), "v".to_string())];
        new_config
    };
    assert!(Session::open(TS_URL, ok, 0).is_ok());
}

fn jar_after(url: &str, lines: &[&str]) -> Vec<(String, String)> {
    let mut session = open(url, Config::default());
    let first = one(&mut session, 0);
    respond(&mut session, &first, 200, &set_cookies(lines));
    session.cookies()
}

fn names(jar: &[(String, String)]) -> Vec<&str> {
    jar.iter().map(|(n, _)| n.as_str()).collect()
}

#[test]
fn cookie_path_longer_than_cap_is_rejected() {
    let long = format!("a=1; Path=/{}", "p".repeat(MAX_COOKIE_PATH));
    let ok_path = format!("b=1; Path=/{}", "p".repeat(MAX_COOKIE_PATH - 1));
    let jar = jar_after(TS_URL, &[&long, &ok_path]);
    assert_eq!(names(&jar), vec!["b"], "{jar:?}");
}

#[test]
fn secure_prefix_requires_the_secure_attribute() {
    let jar = jar_after(
        HTTPS_URL,
        &[
            "__Secure-a=1; Path=/",
            "__Secure-b=1; Secure; Path=/",
            "__secure-c=1; Path=/",
        ],
    );
    assert_eq!(names(&jar), vec!["__Secure-b"], "{jar:?}");
}

#[test]
fn host_prefix_requires_secure_and_root_path() {
    let jar = jar_after(
        HTTPS_URL,
        &[
            "__Host-a=1; Secure",
            "__Host-b=1; Secure; Path=/x",
            "__Host-c=1; Path=/",
            "__Host-d=1; Secure; Path=/",
        ],
    );
    assert_eq!(names(&jar), vec!["__Host-d"], "{jar:?}");
}

#[test]
fn secure_cookie_from_an_http_origin_is_rejected() {
    let jar = jar_after(TS_URL, &["s=1; Secure; Path=/", "p=2; Path=/"]);
    assert_eq!(names(&jar), vec!["p"], "{jar:?}");
    let https = jar_after(HTTPS_URL, &["s=1; Secure; Path=/", "p=2; Path=/"]);
    assert_eq!(names(&https), vec!["s", "p"], "{https:?}");
}

#[test]
fn reconnect_backoff_that_overflows_microseconds_saturates() {
    let mut config = Config::default();
    config.ts_http.backoff_ms = vec![u64::MAX];
    config.ts_http.backoff_cap_ms = u64::MAX;
    let mut session = open(TS_URL, config);
    let first = one(&mut session, 0);
    respond(&mut session, &first, 503, &[]);
    let actions = drain(&mut session, SEC);
    let wake = actions
        .iter()
        .filter_map(|a| match a {
            Action::WakeAt(t) => Some(*t),
            _ => None,
        })
        .max()
        .expect("a wake for the backoff");
    assert!(wake >= u64::MAX / 2, "{wake}");
}

#[test]
fn credential_wait_freezes_timers_and_resume_restarts_the_start_timer() {
    let mut session = open(TS_URL, Config::default());
    let first = one(&mut session, 0);
    respond(&mut session, &first, 401, &challenge("Bearer realm=\"r\""));
    drain(&mut session, 0);
    events(&mut session);
    let late = 100 * SEC;
    assert!(drain(&mut session, late).is_empty());
    assert!(events(&mut session).is_empty());

    session.set_credentials(Credentials::bearer("tok")).unwrap();
    let mut wakes = BTreeSet::new();
    let actions = tracked_wakes(&mut session, late, &mut wakes);
    let conn = actions
        .iter()
        .find_map(|a| match a {
            Action::HttpRequest { conn, .. } => Some(*conn),
            _ => None,
        })
        .expect("request after credentials");
    session.on_response(conn, 200, &[], SEC);
    tracked_wakes(&mut session, late, &mut wakes);
    assert!(wakes.iter().all(|t| *t > late));
    let early = late + 14 * SEC;
    assert!(closes(&drain(&mut session, early)).is_empty());
    let due = late + 15 * SEC;
    assert!(closes(&drain(&mut session, due)).contains(&conn));
}
