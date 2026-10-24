mod support;

use broadcast_client::{Action, Config, Session};
use url::Url;

#[test]
fn resolve_301_with_location() {
    let req = Url::parse("http://example/live.ts").unwrap();
    let headers = vec![("Location".to_string(), "/moved.ts".to_string())];
    let target = broadcast_client::redirect::resolve(&req, &headers, 0, 5).unwrap();
    assert_eq!(target.as_str(), "http://example/moved.ts");
}

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
fn resolve_relative_location() {
    let req = Url::parse("http://example/a/live.ts").unwrap();
    let headers = vec![("Location".to_string(), "moved.ts".to_string())];
    let target = broadcast_client::redirect::resolve(&req, &headers, 0, 5).unwrap();
    assert_eq!(target.as_str(), "http://example/a/moved.ts");
}

#[test]
fn resolve_max_redirects_returns_none() {
    let req = Url::parse("http://example/live.ts").unwrap();
    let headers = vec![("Location".to_string(), "/moved.ts".to_string())];
    assert!(broadcast_client::redirect::resolve(&req, &headers, 5, 5).is_none());
}

#[test]
fn resolve_without_location_returns_none() {
    let req = Url::parse("http://example/live.ts").unwrap();
    let headers: Vec<(String, String)> = vec![];
    assert!(broadcast_client::redirect::resolve(&req, &headers, 0, 5).is_none());
}

#[test]
fn redirect_limit_in_session() {
    let mut session = Session::open("http://example/live.ts", Config::default(), 0).unwrap();
    let mut conn = match session.poll(0).unwrap() {
        Action::HttpRequest { conn, .. } => conn,
        _ => panic!(),
    };
    for i in 0..5 {
        session.on_response(conn, 301, &[("Location".to_string(), format!("/r{i}.ts"))]);
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
    session.on_response(conn, 302, &[]);
    let mut saw_http = false;
    while let Some(a) = session.poll(0) {
        if matches!(a, Action::HttpRequest { .. }) {
            saw_http = true;
        }
    }
    assert!(!saw_http);
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
