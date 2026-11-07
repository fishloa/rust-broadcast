use broadcast_client::tshttp::{Config, TsHttpClient};

static DATA: std::sync::LazyLock<Vec<u8>> =
    std::sync::LazyLock::new(|| test_fixtures::read("ts/h264-aac.ts"));

fn main() {
    let mut client = TsHttpClient::new("http://example/live.ts", Config::default(), 0);
    let mut opens = 0;
    let mut reconnects = 0;
    let mut samples = 0;
    let mut connected = false;

    // Initial open.
    if let Some(broadcast_client::tshttp::Action::Open { .. }) = client.poll() {
        opens += 1;
    }
    client.on_response(200, &[], 0);
    while let Some(e) = client.poll_event() {
        if matches!(e, broadcast_client::tshttp::Event::Connected) {
            connected = true;
        }
    }

    // Feed in 1316-byte chunks.
    for (i, chunk) in DATA.as_slice().chunks(1316).enumerate() {
        let now = (i as u64) * 10;
        client.feed(chunk, now);
        while let Some(e) = client.poll_event() {
            if matches!(
                e,
                broadcast_client::tshttp::Event::Demux(transmux::DemuxEvent::Sample { .. })
            ) {
                samples += 1;
            }
        }
    }

    // Caller-initiated reconnect mid-stream.
    client.reconnect(1000);
    while let Some(a) = client.poll() {
        match a {
            broadcast_client::tshttp::Action::Abort => {}
            broadcast_client::tshttp::Action::Reconnect { .. } => reconnects += 1,
            broadcast_client::tshttp::Action::Open { .. } => opens += 1,
            _ => {}
        }
    }

    println!(
        "tshttp_loop: opens={opens} reconnects={reconnects} samples={samples} connected={connected}"
    );
}
