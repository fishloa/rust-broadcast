use broadcast_client::{Config, Credentials, Session};
use url::Url;

/// The examples' single clock instant as a [`Timestamp`] for the session boundary.
fn now(us: u64) -> broadcast_common::stage::Timestamp {
    broadcast_common::stage::Timestamp::from_micros(us)
}

fn main() {
    let mut session = Session::open(
        Url::parse("http://example/live.ts").unwrap(),
        Config::default(),
        now(0),
    )
    .unwrap();
    if let Some(broadcast_client::Action::HttpRequest { conn, .. }) = session.poll(now(0)) {
        let mut headers = http::HeaderMap::new();
        headers.insert(
            http::header::WWW_AUTHENTICATE,
            "Bearer realm=\"example\", resource_metadata=\"https://resource.example.com/.well-known/oauth-protected-resource\""
                .parse()
                .expect("a valid header value"),
        );
        session.on_response(conn, http::StatusCode::UNAUTHORIZED, &headers, now(0));
    }
    while let Some(e) = session.next_event() {
        println!("event: {e:?}");
    }
    session
        .set_credentials(Credentials::bearer("secret-token"))
        .unwrap();
    if let Some(a) = session.poll(now(0)) {
        println!("action after creds: {a:?}");
    }
}
