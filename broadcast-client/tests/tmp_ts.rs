use tshttp_runtime::{Config, TsHttpClient};

#[test]
fn ts() {
    let bytes = include_bytes!("../../fixtures/ts/m6-single.ts");
    let mut c = TsHttpClient::new("u", Config::default(), 0);
    let _ = c.poll();
    c.on_response(200, 0);
    let _ = c.poll_event();
    c.feed(bytes, 0);
    let mut n=0; let mut samples=0;
    while let Some(e)=c.poll_event() {
        n+=1;
        if matches!(e, tshttp_runtime::Event::Demux(transmux::DemuxEvent::Sample{..})) { samples+=1; }
    }
    println!("ts events={} samples={}", n, samples);
    assert!(samples>0);
}
