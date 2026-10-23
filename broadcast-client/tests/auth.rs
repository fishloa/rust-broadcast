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
    let h = auth.request_headers(&same);
    assert!(
        h.iter()
            .any(|(k, v)| k.eq_ignore_ascii_case("authorization") && v == "Bearer secret-token")
    );
    let other = Url::parse("http://other.example/live.ts").unwrap();
    let h2 = auth.request_headers(&other);
    assert!(
        !h2.iter()
            .any(|(k, _)| k.eq_ignore_ascii_case("authorization"))
    );
}

#[test]
fn bearer_preemptive() {
    let mut session = Session::open("http://example/live.ts", Config::default(), 0).unwrap();
    session.set_credentials(Credentials::bearer("secret-token"));
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
    session.set_credentials(Credentials::new("user", "pass"));
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
    session.set_credentials(Credentials::new("user", "pass"));
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
    session.set_credentials(Credentials::new("user", "pass"));
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
    session.set_credentials(Credentials::new("user", "pass"));
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
    session.set_credentials(Credentials::new("user", "pass"));
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
    session.set_credentials(Credentials::bearer("t"));
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
    let mut session = Session::open("http://example/live.ts", Config::default(), 0).unwrap();
    let conn = match session.poll(0).unwrap() {
        Action::HttpRequest { conn, .. } => conn,
        _ => panic!(),
    };
    session.on_response(conn, 200, &[]);
    session.set_credentials(Credentials::bearer("new-token"));
    // no close action
    let mut saw_close = false;
    while let Some(a) = session.poll(0) {
        if matches!(a, Action::Close { .. }) {
            saw_close = true;
        }
    }
    assert!(!saw_close);
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
    while let Some(a) = session.poll(0) {
        if matches!(a, Action::WakeAt(_)) {
            saw_timer = true;
        }
    }
    assert!(saw_timer);
}

#[test]
fn debug_does_not_contain_secret() {
    let mut session = Session::open("http://example/live.ts", Config::default(), 0).unwrap();
    session.set_credentials(Credentials::bearer("supersecret-token"));
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
