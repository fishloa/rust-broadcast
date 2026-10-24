use broadcast_client::{Config, Credentials, Session};

fn main() {
    let mut session = Session::open("http://example/live.ts", Config::default(), 0).unwrap();
    if let Some(broadcast_client::Action::HttpRequest { conn, .. }) = session.poll(0) {
        let headers = vec![(
            "WWW-Authenticate".to_string(),
            "Bearer realm=\"example\", resource_metadata=\"https://resource.example.com/.well-known/oauth-protected-resource\"".to_string(),
        )];
        session.on_response(conn, 401, &headers);
    }
    while let Some(e) = session.next_event() {
        println!("event: {e:?}");
    }
    session
        .set_credentials(Credentials::bearer("secret-token"))
        .unwrap();
    if let Some(a) = session.poll(0) {
        println!("action after creds: {a:?}");
    }
}
