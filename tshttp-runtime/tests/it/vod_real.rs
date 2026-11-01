use tshttp_runtime::{Action, Config, Event, TsHttpClient};

fn cfg() -> Config {
    Config::default()
}

#[test]
fn real_finite_capture() {
    let bytes = test_fixtures::read("ts/h264_aac.ts");
    let mut oracle = transmux::StreamingTsDemux::new();
    oracle.feed(&bytes);
    oracle.finish();
    let mut oracle_samples = 0usize;
    while let Some(e) = oracle.poll_event() {
        if matches!(e, transmux::DemuxEvent::Sample { .. }) {
            oracle_samples += 1;
        }
    }

    let mut c = TsHttpClient::new("u", cfg(), 0);
    let _ = c.poll();
    c.on_response(
        200,
        &[("Content-Length".to_string(), bytes.len().to_string())],
        0,
    );
    let _ = c.poll_event();
    let _ = c.poll_event();
    for chunk in bytes.chunks(1316) {
        c.feed(chunk, 0);
    }
    c.on_eof(1);

    let mut samples = 0usize;
    let mut ended = false;
    let mut reconnects = 0usize;
    while let Some(e) = c.poll_event() {
        if let Event::Demux(transmux::DemuxEvent::Sample { .. }) = e {
            samples += 1;
        }
        if matches!(e, Event::Ended) {
            ended = true;
        }
    }
    while let Some(a) = c.poll() {
        if matches!(a, Action::Reconnect { .. }) {
            reconnects += 1;
        }
    }
    assert_eq!(samples, oracle_samples);
    assert!(ended);
    assert_eq!(reconnects, 0);
}

#[test]
fn real_live_capture() {
    let bytes = test_fixtures::read("ts/h264_aac.ts");
    let mut c = TsHttpClient::new("u", cfg(), 0);
    let _ = c.poll();
    c.on_response(200, &[], 0);
    let _ = c.poll_event();
    for chunk in bytes.chunks(1316) {
        c.feed(chunk, 0);
    }
    c.on_eof(60_000);
    let mut reconnect0 = false;
    while let Some(a) = c.poll() {
        if matches!(a, Action::Reconnect { after_ms: 0 }) {
            reconnect0 = true;
        }
    }
    assert!(reconnect0);
}
