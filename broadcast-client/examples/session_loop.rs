use broadcast_client::{Config, Session};
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
    let mut opens = 0;
    let mut samples = 0;
    let mut events = 0;
    while let Some(action) = session.poll(now(0)) {
        match action {
            broadcast_client::Action::HttpRequest { conn, .. } => {
                opens += 1;
                let bytes = test_fixtures::read("ts/h264-aac.ts");
                let bytes = &bytes[..];
                session.on_response(conn, 200, &[], now(0));
                session.on_data(conn, bytes, now(0));
            }
            broadcast_client::Action::Close { .. } => {}
            broadcast_client::Action::WakeAt(_) => {}
            _ => {}
        }
    }
    while let Some(e) = session.next_event() {
        events += 1;
        if matches!(e, broadcast_client::Event::Sample { .. }) {
            samples += 1;
        }
    }
    println!("session_loop: opens={opens} samples={samples} events={events}");
}
