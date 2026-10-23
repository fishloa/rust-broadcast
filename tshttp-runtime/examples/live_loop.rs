use tshttp_runtime::{Config, TsHttpClient};

const DATA: &[u8] = include_bytes!("../../fixtures/ts/h264_aac.ts");

fn main() {
    let mut client = TsHttpClient::new("http://example/live.ts", Config::default(), 0);
    let mut opens = 0;
    let mut reconnects = 0;
    let mut samples = 0;
    let mut connected = false;

    // Initial open.
    if let Some(tshttp_runtime::Action::Open { .. }) = client.poll() {
        opens += 1;
    }
    client.on_response(200, &[], 0);
    while let Some(e) = client.poll_event() {
        if matches!(e, tshttp_runtime::Event::Connected) {
            connected = true;
        }
    }

    // Feed in 1316-byte chunks.
    for (i, chunk) in DATA.chunks(1316).enumerate() {
        let now = (i as u64) * 10;
        client.feed(chunk, now);
        while let Some(e) = client.poll_event() {
            if matches!(
                e,
                tshttp_runtime::Event::Demux(transmux::DemuxEvent::Sample { .. })
            ) {
                samples += 1;
            }
        }
    }

    // Caller-initiated reconnect mid-stream.
    client.reconnect(1000);
    while let Some(a) = client.poll() {
        match a {
            tshttp_runtime::Action::Abort => {}
            tshttp_runtime::Action::Reconnect { .. } => reconnects += 1,
            tshttp_runtime::Action::Open { .. } => opens += 1,
            _ => {}
        }
    }

    println!(
        "live_loop: opens={opens} reconnects={reconnects} samples={samples} connected={connected}"
    );
}
