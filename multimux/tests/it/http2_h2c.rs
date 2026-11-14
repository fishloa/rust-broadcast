use multimux::route::RouteConfig;
use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;
use test_fixtures::process::wait_for_port;

use multimux::origin::{AppState, router};
use multimux::output::Output;
use multimux::output::hls::HlsOutput;
use multimux::route::RouteHandle;
use tokio::net::TcpListener;

fn make_state() -> Arc<AppState> {
    let store = Arc::new(
        RouteHandle::new(
            RouteConfig::default()
                .with_target_duration_secs(4.0)
                .with_part_target(std::time::Duration::from_millis(500))
                .with_window_segments(4),
        )
        .with_hold_cold_playlist(false)
        .with_declared_bandwidth_bps(5_000_000),
    );
    let program = media_plane::ProgramId(0);
    store.publish_new_program(program);
    store.set_init(program, vec![0xAA; 4]);
    store.add_part(
        program,
        transmux::PartInfo::new(vec![0x10; 4], 0.5, 1, 0).with_independent(true),
    );
    let mut streams = HashMap::new();
    streams.insert(
        "cam".to_string(),
        (
            store,
            vec![Arc::new(HlsOutput::default()) as Arc<dyn Output>],
        ),
    );
    Arc::new(AppState::new(streams))
}

#[tokio::test]
async fn h2c_fetches_playlist_and_blocking_reload_over_one_connection() {
    let state = make_state();
    let app = router(state);

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let token = tokio_util::sync::CancellationToken::new();
    let serve = tokio::spawn(multimux::origin::serve_hyper_util_with_timeout(
        listener,
        app,
        token.clone(),
        Duration::from_secs(10),
        Duration::from_secs(30),
    ));

    wait_for_port(addr).await;

    let client = reqwest::Client::builder()
        .http2_prior_knowledge()
        .build()
        .unwrap();
    let base = format!("http://{}", addr);

    let (r1, r2) = tokio::join!(
        client.get(format!("{}/cam/media.m3u8", base)).send(),
        client
            .get(format!("{}/cam/media.m3u8?_HLS_msn=1&_HLS_part=0", base))
            .send()
    );
    let r1 = r1.unwrap();
    let r2 = r2.unwrap();
    assert!(r1.status().is_success(), "playlist: {}", r1.status());
    assert!(r2.status().is_success(), "blocking reload: {}", r2.status());

    token.cancel();
    let _ = serve.await;
}

#[tokio::test]
async fn http1_client_still_works() {
    let state = make_state();
    let app = router(state);

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let token = tokio_util::sync::CancellationToken::new();
    let serve = tokio::spawn(multimux::origin::serve_hyper_util_with_timeout(
        listener,
        app,
        token.clone(),
        Duration::from_secs(10),
        Duration::from_secs(30),
    ));

    wait_for_port(addr).await;

    let client = reqwest::Client::new();
    let base = format!("http://{}", addr);
    let resp = client
        .get(format!("{}/cam/media.m3u8", base))
        .send()
        .await
        .unwrap();
    assert!(resp.status().is_success(), "http1: {}", resp.status());

    token.cancel();
    let _ = serve.await;
}
