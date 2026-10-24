mod support;

use broadcast_auth::Scheme as AuthScheme;
use broadcast_client::auth::AuthState;
use broadcast_client::{Action, Config, Credentials, Event, Session, Status};
use url::Url;

#[test]
fn origin_scoped_request_headers() {
    let origin = Url::parse("http://example/live.ts").unwrap();
    let mut auth = AuthState::new(origin, vec![]);
    auth.set_credentials(Credentials::bearer("secret-token"));
    let same = Url::parse("http://example/other.ts").unwrap();
    let h = auth.request_headers(&same, 0);
    assert!(
        h.iter()
            .any(|(k, v)| k.eq_ignore_ascii_case("authorization") && v == "Bearer secret-token")
    );
    let other = Url::parse("http://other.example/live.ts").unwrap();
    let h2 = auth.request_headers(&other, 0);
    assert!(
        !h2.iter()
            .any(|(k, _)| k.eq_ignore_ascii_case("authorization"))
    );
}

#[test]
fn bearer_preemptive() {
    let mut session = Session::open("http://example/live.ts", Config::default(), 0).unwrap();
    session
        .set_credentials(Credentials::bearer("secret-token"))
        .unwrap();
    match session.poll(0).unwrap() {
        Action::HttpRequest { headers, .. } => {
            assert!(headers.iter().any(
                |(k, v)| k.eq_ignore_ascii_case("authorization") && v == "Bearer secret-token"
            ));
        }
        _ => panic!(),
    }
}

#[test]
fn basic_401_replay_then_200() {
    let mut session = Session::open("http://example/live.ts", Config::default(), 0).unwrap();
    session
        .set_credentials(Credentials::new("user", "pass"))
        .unwrap();
    let conn1 = match session.poll(0).unwrap() {
        Action::HttpRequest { conn, headers, .. } => {
            assert!(
                !headers
                    .iter()
                    .any(|(k, _)| k.eq_ignore_ascii_case("authorization"))
            );
            conn
        }
        _ => panic!(),
    };
    let challenge = "Basic realm=\"test\"";
    session.on_response(
        conn1,
        401,
        &[("WWW-Authenticate".to_string(), challenge.to_string())],
    );
    let mut conn2 = None;
    let mut saw_close = false;
    while let Some(a) = session.poll(0) {
        match a {
            Action::Close { conn } if conn == conn1 => saw_close = true,
            Action::HttpRequest { conn, headers, .. } => {
                assert!(headers.iter().any(
                    |(k, v)| k.eq_ignore_ascii_case("authorization") && v.starts_with("Basic ")
                ));
                conn2 = Some(conn);
            }
            Action::WakeAt(_) => {}
            _ => {}
        }
    }
    assert!(saw_close);
    let c2 = conn2.unwrap();
    session.on_response(c2, 200, &[]);
}

#[test]
fn digest_401_replay_then_200() {
    let mut session = Session::open("http://example/live.ts", Config::default(), 0).unwrap();
    session
        .set_credentials(Credentials::new("user", "pass"))
        .unwrap();
    let conn1 = match session.poll(0).unwrap() {
        Action::HttpRequest { conn, .. } => conn,
        _ => panic!(),
    };
    let challenge = "Digest realm=\"test\", nonce=\"abc123\", qop=\"auth\"";
    session.on_response(
        conn1,
        401,
        &[("WWW-Authenticate".to_string(), challenge.to_string())],
    );
    let mut conn2 = None;
    while let Some(a) = session.poll(0) {
        if let Action::HttpRequest { conn, headers, .. } = a {
            assert!(
                headers
                    .iter()
                    .any(|(k, v)| k.eq_ignore_ascii_case("authorization")
                        && v.starts_with("Digest "))
            );
            conn2 = Some(conn);
        }
    }
    let c2 = conn2.unwrap();
    session.on_response(c2, 200, &[]);
}

#[test]
fn basic_digest_preemptive_after_replay() {
    let mut session = Session::open("http://example/live.ts", Config::default(), 0).unwrap();
    session
        .set_credentials(Credentials::new("user", "pass"))
        .unwrap();
    let conn1 = match session.poll(0).unwrap() {
        Action::HttpRequest { conn, .. } => conn,
        _ => panic!(),
    };
    let challenge = "Basic realm=\"test\"";
    session.on_response(
        conn1,
        401,
        &[("WWW-Authenticate".to_string(), challenge.to_string())],
    );
    // consume replay
    let mut conn2 = None;
    while let Some(a) = session.poll(0) {
        if let Action::HttpRequest { conn, .. } = a {
            conn2 = Some(conn);
        }
    }
    let c2 = conn2.unwrap();
    session.on_response(c2, 200, &[]);
    // trigger reconnect -> new request should be preemptive
    session.reconnect(1_000);
    let mut saw_auth = false;
    while let Some(a) = session.poll(1_000) {
        if let Action::HttpRequest { headers, .. } = a
            && headers
                .iter()
                .any(|(k, v)| k.eq_ignore_ascii_case("authorization") && v.starts_with("Basic "))
        {
            saw_auth = true;
        }
    }
    assert!(saw_auth);
}

#[test]
fn digest_stale_rechallenge_replays() {
    let mut session = Session::open("http://example/live.ts", Config::default(), 0).unwrap();
    session
        .set_credentials(Credentials::new("user", "pass"))
        .unwrap();
    let conn1 = match session.poll(0).unwrap() {
        Action::HttpRequest { conn, .. } => conn,
        _ => panic!(),
    };
    let challenge = "Digest realm=\"test\", nonce=\"abc\", qop=\"auth\"";
    session.on_response(
        conn1,
        401,
        &[("WWW-Authenticate".to_string(), challenge.to_string())],
    );
    let mut conn2 = None;
    while let Some(a) = session.poll(0) {
        if let Action::HttpRequest { conn, .. } = a {
            conn2 = Some(conn);
        }
    }
    let c2 = conn2.unwrap();
    let stale_challenge = "Digest realm=\"test\", nonce=\"def\", qop=\"auth\", stale=true";
    session.on_response(
        c2,
        401,
        &[("WWW-Authenticate".to_string(), stale_challenge.to_string())],
    );
    let mut conn3 = None;
    let mut saw_close = false;
    while let Some(a) = session.poll(0) {
        match a {
            Action::Close { conn } if conn == c2 => saw_close = true,
            Action::HttpRequest { conn, headers, .. } => {
                assert!(
                    headers
                        .iter()
                        .any(|(k, v)| k.eq_ignore_ascii_case("authorization")
                            && v.starts_with("Digest "))
                );
                conn3 = Some(conn);
            }
            _ => {}
        }
    }
    assert!(saw_close);
    assert!(conn3.is_some());
}

#[test]
fn no_credentials_401_waits_no_timers() {
    let mut session = Session::open("http://example/live.ts", Config::default(), 0).unwrap();
    let conn = match session.poll(0).unwrap() {
        Action::HttpRequest { conn, .. } => conn,
        _ => panic!(),
    };
    let challenge = "Bearer realm=\"example\", resource_metadata=\"https://resource.example.com/.well-known/oauth-protected-resource\"";
    session.on_response(
        conn,
        401,
        &[("WWW-Authenticate".to_string(), challenge.to_string())],
    );
    let mut events = Vec::new();
    while let Some(e) = session.next_event() {
        events.push(e);
    }
    assert!(events.iter().any(|e| matches!(
        e,
        Event::AuthRequired {
            scheme: AuthScheme::Bearer,
            realm: Some(_),
            resource_metadata: Some(_),
            ..
        }
    )));
    assert!(
        events
            .iter()
            .any(|e| matches!(e, Event::Status(Status::WaitingForCredentials)))
    );
    // close action
    let mut saw_close = false;
    while let Some(a) = session.poll(0) {
        if matches!(a, Action::Close { .. }) {
            saw_close = true;
        }
        assert!(!matches!(a, Action::WakeAt(_) | Action::HttpRequest { .. }));
    }
    assert!(saw_close);
    for _ in 0..50 {
        assert!(session.poll(1_000_000).is_none());
    }
}

#[test]
fn no_credentials_407_waits() {
    let mut session = Session::open("http://example/live.ts", Config::default(), 0).unwrap();
    let conn = match session.poll(0).unwrap() {
        Action::HttpRequest { conn, .. } => conn,
        _ => panic!(),
    };
    let challenge = "Basic realm=\"proxy\"";
    session.on_response(
        conn,
        407,
        &[("Proxy-Authenticate".to_string(), challenge.to_string())],
    );
    let mut events = Vec::new();
    while let Some(e) = session.next_event() {
        events.push(e);
    }
    assert!(
        events
            .iter()
            .any(|e| matches!(e, Event::AuthRequired { .. }))
    );
    assert!(
        events
            .iter()
            .any(|e| matches!(e, Event::Status(Status::WaitingForCredentials)))
    );
}

#[test]
fn auth_rejected_after_bad_credentials() {
    let mut session = Session::open("http://example/live.ts", Config::default(), 0).unwrap();
    session
        .set_credentials(Credentials::new("user", "pass"))
        .unwrap();
    let conn1 = match session.poll(0).unwrap() {
        Action::HttpRequest { conn, .. } => conn,
        _ => panic!(),
    };
    let challenge = "Basic realm=\"test\"";
    session.on_response(
        conn1,
        401,
        &[("WWW-Authenticate".to_string(), challenge.to_string())],
    );
    let mut conn2 = None;
    while let Some(a) = session.poll(0) {
        if let Action::HttpRequest { conn, .. } = a {
            conn2 = Some(conn);
        }
    }
    let c2 = conn2.unwrap();
    session.on_response(
        c2,
        401,
        &[("WWW-Authenticate".to_string(), challenge.to_string())],
    );
    let mut events = Vec::new();
    while let Some(e) = session.next_event() {
        events.push(e);
    }
    assert!(
        events
            .iter()
            .any(|e| matches!(e, Event::AuthRejected { status: 401 }))
    );
    assert!(
        events
            .iter()
            .any(|e| matches!(e, Event::Status(Status::WaitingForCredentials)))
    );
}

#[test]
fn set_credentials_while_waiting_reopens() {
    let mut session = Session::open("http://example/live.ts", Config::default(), 0).unwrap();
    let conn = match session.poll(0).unwrap() {
        Action::HttpRequest { conn, .. } => conn,
        _ => panic!(),
    };
    let challenge = "Bearer realm=\"example\"";
    session.on_response(
        conn,
        401,
        &[("WWW-Authenticate".to_string(), challenge.to_string())],
    );
    // consume close
    while session.poll(0).is_some() {}
    session.set_credentials(Credentials::bearer("t")).unwrap();
    let mut saw_http = false;
    while let Some(a) = session.poll(0) {
        if matches!(a, Action::HttpRequest { .. }) {
            saw_http = true;
        }
    }
    assert!(saw_http);
}

#[test]
fn set_credentials_while_streaming_stored_not_interrupt() {
    let mut config = Config::default();
    config.ts_http.min_reconnect_interval_ms = 0;
    let mut session = Session::open("http://example/live.ts", config, 0).unwrap();
    let conn = match session.poll(0).unwrap() {
        Action::HttpRequest { conn, headers, .. } => {
            assert!(
                !headers
                    .iter()
                    .any(|(k, _)| k.eq_ignore_ascii_case("authorization"))
            );
            conn
        }
        _ => panic!(),
    };
    session.on_response(conn, 200, &[]);
    session
        .set_credentials(Credentials::bearer("new-token"))
        .unwrap();
    let mut saw_close = false;
    let mut saw_request = false;
    for now_us in [0, 1_000_000, 2_000_000, 5_000_000, 10_000_000] {
        while let Some(a) = session.poll(now_us) {
            match a {
                Action::Close { .. } => saw_close = true,
                Action::HttpRequest { .. } => saw_request = true,
                _ => {}
            }
        }
    }
    assert!(!saw_close);
    assert!(!saw_request);

    session.on_closed(conn, None, 11_000_000);
    let mut next_auth = None;
    for now_us in [11_000_000, 12_000_000, 20_000_000, 40_000_000] {
        while let Some(a) = session.poll(now_us) {
            if let Action::HttpRequest { headers, .. } = a {
                next_auth = headers
                    .iter()
                    .find(|(k, _)| k.eq_ignore_ascii_case("authorization"))
                    .map(|(_, v)| v.clone());
            }
        }
        if next_auth.is_some() {
            break;
        }
    }
    assert_eq!(
        next_auth.as_deref(),
        Some("Bearer new-token"),
        "the stored credentials are used for the next request"
    );
}

#[test]
fn forbidden_then_retry_policy() {
    let mut session = Session::open("http://example/live.ts", Config::default(), 0).unwrap();
    let conn = match session.poll(0).unwrap() {
        Action::HttpRequest { conn, .. } => conn,
        _ => panic!(),
    };
    session.on_response(conn, 403, &[]);
    let mut events = Vec::new();
    while let Some(e) = session.next_event() {
        events.push(e);
    }
    assert!(
        events
            .iter()
            .any(|e| matches!(e, Event::Forbidden { status: 403 }))
    );
    // retry policy produces a timer (WakeAt) via source.on_error
    let mut saw_timer = false;
    let mut closed = Vec::new();
    while let Some(a) = session.poll(0) {
        match a {
            Action::WakeAt(_) => saw_timer = true,
            Action::Close { conn } => closed.push(conn),
            _ => {}
        }
    }
    assert!(saw_timer);
    assert_eq!(closed, vec![conn]);
}

#[test]
fn debug_does_not_contain_secret() {
    let mut session = Session::open("http://example/live.ts", Config::default(), 0).unwrap();
    session
        .set_credentials(Credentials::bearer("supersecret-token"))
        .unwrap();
    let dbg = format!("{:?}", session);
    assert!(!dbg.contains("supersecret-token"));
}

#[test]
fn auth_state_debug_redacts() {
    let origin = Url::parse("http://example/live.ts").unwrap();
    let mut auth = AuthState::new(origin, vec![]);
    auth.set_credentials(Credentials::bearer("supersecret-token"));
    let dbg = format!("{:?}", auth);
    assert!(!dbg.contains("supersecret-token"));
}

fn first_request_headers(session: &mut Session, now: u64) -> (u32, Vec<(String, String)>) {
    while let Some(a) = session.poll(now) {
        if let Action::HttpRequest { conn, headers, .. } = a {
            return (conn, headers);
        }
    }
    panic!("no HttpRequest was issued");
}

fn authorization(headers: &[(String, String)]) -> Option<&str> {
    headers
        .iter()
        .find(|(k, _)| k.eq_ignore_ascii_case("authorization"))
        .map(|(_, v)| v.as_str())
}

fn challenged_session(challenge: &str) -> Session {
    let mut session = Session::open("http://example/live.ts", Config::default(), 0).unwrap();
    let (conn, headers) = first_request_headers(&mut session, 0);
    assert!(authorization(&headers).is_none());
    session.on_response(
        conn,
        401,
        &[("WWW-Authenticate".to_string(), challenge.to_string())],
    );
    while session.poll(0).is_some() {}
    let mut saw_required = false;
    while let Some(e) = session.next_event() {
        if matches!(e, Event::AuthRequired { .. }) {
            saw_required = true;
        }
    }
    assert!(saw_required);
    session
}

#[test]
fn basic_first_request_after_set_credentials_is_already_authorized() {
    let mut session = challenged_session("Basic realm=\"tv\"");
    session
        .set_credentials(Credentials::new("user", "pass"))
        .unwrap();
    let (_, headers) = first_request_headers(&mut session, 1);
    assert_eq!(authorization(&headers), Some("Basic dXNlcjpwYXNz"));
}

#[test]
fn digest_first_request_after_set_credentials_is_already_authorized() {
    let mut session = challenged_session("Digest realm=\"tv\", nonce=\"abc123\", qop=\"auth\"");
    session
        .set_credentials(Credentials::new("user", "pass"))
        .unwrap();
    let (_, headers) = first_request_headers(&mut session, 1);
    let auth = authorization(&headers).expect("Authorization on the first request");
    assert!(auth.starts_with("Digest "), "{auth}");
    assert!(auth.contains("username=\"user\""), "{auth}");
    assert!(auth.contains("realm=\"tv\""), "{auth}");
    assert!(auth.contains("nonce=\"abc123\""), "{auth}");
}

#[test]
fn reconnect_after_credentials_arrive_stays_preemptive() {
    let mut session = challenged_session("Basic realm=\"tv\"");
    session
        .set_credentials(Credentials::new("user", "pass"))
        .unwrap();
    let (conn, headers) = first_request_headers(&mut session, 1);
    assert!(authorization(&headers).is_some());
    session.on_response(conn, 200, &[]);
    session.reconnect(2);
    let (_, again) = first_request_headers(&mut session, 2);
    assert_eq!(authorization(&again), Some("Basic dXNlcjpwYXNz"));
}

#[test]
fn bearer_credentials_after_challenge_still_preemptive() {
    let mut session = challenged_session("Bearer realm=\"tv\"");
    session.set_credentials(Credentials::bearer("tok")).unwrap();
    let (_, headers) = first_request_headers(&mut session, 1);
    assert_eq!(authorization(&headers), Some("Bearer tok"));
}

#[test]
fn rejected_credentials_then_new_ones_use_the_new_challenge_state() {
    let mut session = challenged_session("Basic realm=\"tv\"");
    session
        .set_credentials(Credentials::new("user", "wrong"))
        .unwrap();
    let (conn, headers) = first_request_headers(&mut session, 1);
    assert!(authorization(&headers).is_some());
    session.on_response(
        conn,
        401,
        &[(
            "WWW-Authenticate".to_string(),
            "Basic realm=\"tv\"".to_string(),
        )],
    );
    while session.poll(1).is_some() {}
    let mut rejected = false;
    while let Some(e) = session.next_event() {
        if matches!(e, Event::AuthRejected { .. }) {
            rejected = true;
        }
    }
    assert!(rejected);
    session
        .set_credentials(Credentials::new("user", "right"))
        .unwrap();
    let (_, again) = first_request_headers(&mut session, 2);
    assert_eq!(authorization(&again), Some("Basic dXNlcjpyaWdodA=="));
}
