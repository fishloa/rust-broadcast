mod support;

use broadcast_client::{Action, Config, Event, Session};
use support::*;

#[test]
fn smoke_buffer() {
    let bytes = read_tf1_bytes();
    let (status, headers) = read_tf1_headers();
    let mut session = Session::open("http://example/live.ts", Config::default(), 0).unwrap();
    let conn = match session.poll(0).unwrap() {
        Action::HttpRequest { conn, .. } => conn,
        _ => panic!(),
    };
    session.on_response(conn, status, &headers);
    for (i, chunk) in bytes.chunks(1316).enumerate() {
        session.on_data(conn, chunk, (i as u64) * 100_000);
    }
    let mut bufs = 0;
    while let Some(e) = session.next_event() {
        if matches!(e, Event::Buffer(_)) {
            bufs += 1;
        }
    }
    let _ = bufs;
}
