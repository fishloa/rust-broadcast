use broadcast_client::{Config, Session};

#[test]
fn ex() {
    let bytes = include_bytes!("../../fixtures/ts/m6-single.ts");
    let mut session = Session::open("http://example/live.ts", Config::default(), 0).unwrap();
    let conn = match session.poll(0).unwrap() {
        broadcast_client::Action::HttpRequest { conn, .. } => conn,
        _ => panic!(),
    };
    session.on_response(conn, 200, &[]);
    session.on_data(conn, bytes, 0);
    let mut n=0; let mut samples=0;
    while let Some(e)=session.next_event() {
        n+=1;
        if matches!(e, broadcast_client::Event::Sample{..}) { samples+=1; }
    }
    println!("events={} samples={}", n, samples);
    assert!(samples>0);
}
