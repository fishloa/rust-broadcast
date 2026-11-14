use crate::support;

use std::sync::OnceLock;

use broadcast_client::Config;
use support::rtsp_host::{BASE_URL, Host, MediaFixture, samples_of};

const FNV_OFFSET: u64 = 0xcbf2_9ce4_8422_2325;
const FNV_PRIME: u64 = 0x0100_0000_01b3;

fn fixture() -> &'static MediaFixture {
    static FIXTURE: OnceLock<MediaFixture> = OnceLock::new();
    FIXTURE.get_or_init(MediaFixture::load)
}

fn server() -> support::rtsp_host::Server {
    support::rtsp_host::Server::new(&fixture().sdp)
}

#[test]
fn rtsp_pull_golden() {
    let mut host = Host::new(BASE_URL, Config::default(), server());
    host.connect();
    let frames = fixture().frames();
    host.deliver_all(&frames);
    let events = host.events();
    let samples = samples_of(&events);

    let mut rendered = String::new();
    for rec in &host.server.log {
        rendered.push_str(&format!("{} {}\n", rec.method, rec.uri));
        rendered.push_str(&format!("CSeq: {}\n", rec.cseq));
        for (name, value) in &rec.headers {
            rendered.push_str(&format!("{}: {}\n", name, value));
        }
        rendered.push_str("===\n");
    }
    rendered.push_str("SAMPLES\n");
    for (track, sample) in &samples {
        let digest = sample.data.iter().fold(FNV_OFFSET, |h, b| {
            (h ^ u64::from(*b)).wrapping_mul(FNV_PRIME)
        });
        rendered.push_str(&format!(
            "track={} dts={} pts={} sync={} len={} fnv={:016x}\n",
            track,
            sample
                .timing
                .and_then(|t| t.dts())
                .map(|v| v.to_string())
                .unwrap_or_else(|| "none".to_string()),
            sample
                .timing
                .and_then(|t| t.pts())
                .map(|v| v.to_string())
                .unwrap_or_else(|| "none".to_string()),
            sample.flags.is_sync,
            sample.data.len(),
            digest
        ));
    }

    let file = "golden/rtsp/pull-broadcast-client.txt";
    let path = test_fixtures::path(file);
    if std::env::var("REGENERATE_RTSP_PULL_GOLDEN").ok().as_deref() == Some("1") {
        if std::env::var("CI").is_ok() {
            panic!("refuse to regenerate golden in CI");
        }
        std::fs::write(&path, &rendered).expect("write golden");
        return;
    }
    let expected = std::fs::read_to_string(&path).expect("read golden");
    assert_eq!(rendered, expected, "{file} differs");
}
