use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use multimux::origin::{AppState, router};
use multimux::output::Output;
use multimux::output::llhls::LlHlsOutput;
use multimux::route::RouteHandle;
use tokio::net::TcpListener;

async fn wait_for_port(addr: std::net::SocketAddr) {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(10);
    loop {
        if tokio::net::TcpStream::connect(addr).await.is_ok() {
            return;
        }
        if tokio::time::Instant::now() >= deadline {
            panic!("nothing accepted a connection on {addr}");
        }
        tokio::task::yield_now().await;
    }
}

fn make_state() -> Arc<AppState> {
    let store = Arc::new(RouteHandle::new(4.0, 500, 4).with_declared_bandwidth_bps(5_000_000));
    let program = media_plane::ProgramId(0);
    store.publish_new_program(program);
    store.set_init(program, vec![0xAA; 4]);
    store.add_part(
        program,
        transmux::PartInfo {
            bytes: vec![0x10; 4],
            duration: 0.5,
            independent: true,
            segment_seq: 1,
            part_index: 0,
        },
    );
    let mut streams = HashMap::new();
    streams.insert(
        "cam".to_string(),
        (
            store,
            vec![Arc::new(LlHlsOutput::default()) as Arc<dyn Output>],
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
