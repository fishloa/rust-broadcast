use broadcast_client::tshttp::{Config, TsHttpClient};

fn main() {
    let mut client = TsHttpClient::new("http://example/live.ts", Config::default(), 0);
    let _ = client.poll();
    client.on_response(200, &[], 0);
    let _ = client.poll_event();
    static D: std::sync::LazyLock<Vec<u8>> =
        std::sync::LazyLock::new(|| test_fixtures::read("ts/h264-aac.ts"));
    client.feed(D.as_slice(), 0);

    // Advance time to trigger stall notice and reconnect.
    client.poll_timeout(5000);
    let mut stall_notices = 0;
    let mut starved = 0;
    while let Some(e) = client.poll_event() {
        match e {
            broadcast_client::tshttp::Event::StallNotice { .. } => stall_notices += 1,
            broadcast_client::tshttp::Event::Starved(_) => starved += 1,
            _ => {}
        }
    }
    // stall reconnect at 30s
    client.poll_timeout(30000);
    let mut aborted = false;
    let mut reconnects = 0;
    while let Some(a) = client.poll() {
        match a {
            broadcast_client::tshttp::Action::Abort => aborted = true,
            broadcast_client::tshttp::Action::Reconnect { after_ms } => {
                reconnects += 1;
                assert_eq!(after_ms, 0);
            }
            _ => {}
        }
    }
    println!(
        "stall_demo: stall_notices={stall_notices} starved={starved} aborted={aborted} reconnects={reconnects}"
    );
}
