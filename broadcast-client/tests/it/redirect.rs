use broadcast_client::{Action, Config, Session};

#[test]
fn resolve_all_3xx_codes_session() {
    for code in [301u16, 302, 303, 307, 308] {
        let mut session = Session::open("http://example/live.ts", Config::default(), 0).unwrap();
        let conn = match session.poll(0).unwrap() {
            Action::HttpRequest { conn, .. } => conn,
            _ => panic!(),
        };
        session.on_response(
            conn,
            code,
            &[("Location".to_string(), "/moved.ts".to_string())],
            0,
        );
        let mut saw_new = false;
        while let Some(a) = session.poll(0) {
            if let Action::HttpRequest { url, .. } = a {
                assert!(url.ends_with("/moved.ts"));
                saw_new = true;
            }
        }
        assert!(saw_new, "code {code} did not redirect");
    }
}

#[test]
fn redirect_limit_in_session() {
    let mut session = Session::open("http://example/live.ts", Config::default(), 0).unwrap();
    let mut conn = match session.poll(0).unwrap() {
        Action::HttpRequest { conn, .. } => conn,
        _ => panic!(),
    };
    for i in 0..5 {
        session.on_response(
            conn,
            301,
            &[("Location".to_string(), format!("/r{i}.ts"))],
            0,
        );
        let mut next = None;
        while let Some(a) = session.poll(0) {
            if let Action::HttpRequest { conn: c, .. } = a {
                next = Some(c);
            }
        }
        conn = next.expect("redirect should issue new request");
    }
    // one more redirect exceeds max_redirects
    session.on_response(
        conn,
        301,
        &[("Location".to_string(), "/too-many.ts".to_string())],
        0,
    );
    let mut saw_http = false;
    while let Some(a) = session.poll(0) {
        if matches!(a, Action::HttpRequest { .. }) {
            saw_http = true;
        }
    }
    assert!(!saw_http, "redirect beyond max must not issue HttpRequest");
}

#[test]
fn redirect_without_location_is_connect_error() {
    let mut session = Session::open("http://example/live.ts", Config::default(), 0).unwrap();
    let conn = match session.poll(0).unwrap() {
        Action::HttpRequest { conn, .. } => conn,
        _ => panic!(),
    };
    session.on_response(conn, 302, &[], 0);
    let mut saw_http = false;
    let mut closes = Vec::new();
    while let Some(a) = session.poll(0) {
        match a {
            Action::HttpRequest { .. } => saw_http = true,
            Action::Close { conn } => closes.push(conn),
            _ => {}
        }
    }
    assert!(!saw_http);
    assert_eq!(closes, vec![conn]);
}

#[test]
fn cross_origin_redirect_drops_authorization() {
    use broadcast_client::Credentials;
    let mut session = Session::open("http://example/live.ts", Config::default(), 0).unwrap();
    session
        .set_credentials(Credentials::bearer("secret-token"))
        .unwrap();
    let conn = match session.poll(0).unwrap() {
        Action::HttpRequest { conn, headers, .. } => {
            assert!(headers.iter().any(
                |(k, v)| k.eq_ignore_ascii_case("authorization") && v == "Bearer secret-token"
            ));
            conn
        }
        _ => panic!(),
    };
    session.on_response(
        conn,
        302,
        &[(
            "Location".to_string(),
            "http://other.example/live.ts".to_string(),
        )],
        0,
    );
    let mut new_headers = None;
    while let Some(a) = session.poll(0) {
        if let Action::HttpRequest { headers, .. } = a {
            new_headers = Some(headers);
        }
    }
    let nh = new_headers.expect("redirect request");
    assert!(
        !nh.iter()
            .any(|(k, _)| k.eq_ignore_ascii_case("authorization")),
        "cross-origin redirect must drop Authorization"
    );
}
