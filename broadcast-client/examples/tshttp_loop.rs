use broadcast_client::tshttp::{Config, TsHttpClient};
use broadcast_common::stage::Timestamp;

/// This example's microsecond clock as the runtime's [`Timestamp`].
fn instant(us: u64) -> Timestamp {
    Timestamp::from_micros(us)
}

static DATA: std::sync::LazyLock<Vec<u8>> =
    std::sync::LazyLock::new(|| test_fixtures::read("ts/h264-aac.ts"));

fn main() {
    let mut client = TsHttpClient::new("http://example/live.ts", Config::default(), instant(0));
    let mut opens = 0;
    let mut reconnects = 0;
    let mut samples = 0;
    let mut connected = false;

    // Initial open.
    if let Some(broadcast_client::tshttp::Action::Open { .. }) = client.poll() {
        opens += 1;
    }
    client.on_response(200, &[], instant(0));
    while let Some(e) = client.poll_event() {
        if matches!(e, broadcast_client::tshttp::Event::Connected) {
            connected = true;
        }
    }

    // Feed in 1316-byte chunks.
    for (i, chunk) in DATA.as_slice().chunks(1316).enumerate() {
        client.feed(chunk, Timestamp::from_micros((i as u64) * 10));
        while let Some(e) = client.poll_event() {
            if matches!(
                e,
                broadcast_client::tshttp::Event::Demux(ev) if matches!(ev.as_ref(), transmux::DemuxEvent::Sample { .. })
            ) {
                samples += 1;
            }
        }
    }

    // Caller-initiated reconnect mid-stream.
    client.reconnect(instant(100));
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
