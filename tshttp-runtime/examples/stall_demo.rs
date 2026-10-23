use tshttp_runtime::{Config, TsHttpClient};

fn main() {
    let mut client = TsHttpClient::new("http://example/live.ts", Config::default(), 0);
    let _ = client.poll();
    client.on_response(200, &[], 0);
    let _ = client.poll_event();
    const D: &[u8] = include_bytes!("../../fixtures/ts/h264_aac.ts");
    client.feed(D, 0);

    // Advance time to trigger stall notice and reconnect.
    client.poll_timeout(5000);
    let mut stall_notices = 0;
    let mut starved = 0;
    while let Some(e) = client.poll_event() {
        match e {
            tshttp_runtime::Event::StallNotice { .. } => stall_notices += 1,
            tshttp_runtime::Event::Starved(_) => starved += 1,
            _ => {}
        }
    }
    // stall reconnect at 30s
    client.poll_timeout(30000);
    let mut aborted = false;
    let mut reconnects = 0;
    while let Some(a) = client.poll() {
        match a {
            tshttp_runtime::Action::Abort => aborted = true,
            tshttp_runtime::Action::Reconnect { after_ms } => {
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
