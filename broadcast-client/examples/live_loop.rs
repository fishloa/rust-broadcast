use broadcast_client::{Config, Session};

fn main() {
    let mut session = Session::open("http://example/live.ts", Config::default(), 0).unwrap();
    let mut opens = 0;
    let mut samples = 0;
    let mut events = 0;
    while let Some(action) = session.poll(0) {
        match action {
            broadcast_client::Action::HttpRequest { conn, .. } => {
                opens += 1;
                let bytes = include_bytes!("../../fixtures/ts/h264_aac.ts");
                session.on_response(conn, 200, &[], 0);
                session.on_data(conn, bytes, 0);
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
    println!("live_loop: opens={opens} samples={samples} events={events}");
}
