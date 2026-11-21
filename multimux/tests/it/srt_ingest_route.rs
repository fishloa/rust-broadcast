//! A listener-mode `InputSpec::Srt` route served through `multimux::Server`
//! (#3680): the route is not ready before a publisher arrives, a real SRT
//! caller publishing TS reaches a served media playlist, and `/readyz` turns
//! 200.

use std::net::SocketAddr;
use std::time::Duration;

use multimux::config::{Config, InputSpec, Route};
use multimux::dvr::DvrConfig;
use multimux::output::OutputKind;
use multimux::registry::SchemeRegistry;
use srt_runtime::HandshakeConfig;
use srt_runtime::io::{SrtSocket, SrtSocketConfig};

use crate::support::listener::bind_tcp;
use crate::support::shared_it::poll_until_extinf;

const GUARD: Duration = Duration::from_secs(40);
const SRT_PAYLOAD_TS_PACKETS: usize = 7;
const TS_PACKET_BYTES: usize = 188;

fn config(bind: SocketAddr, srt_listen: SocketAddr) -> Config {
    Config::default()
        .with_bind(bind.to_string())
        .with_target_duration(Duration::from_millis(500))
        .with_part_target(Duration::from_millis(100))
        .with_window_segments(8)
        .with_routes(vec![Route {
            name: "cam".to_string(),
            input: InputSpec::Srt {
                listen: Some(srt_listen.to_string()),
                remote: None,
                stream_id: None,
                latency: None,
            },
            outputs: vec![OutputKind::LlHls],
            dvr: DvrConfig::default(),
            ts_hls_cut: Default::default(),
            ts_hls_idr_fallback: Default::default(),
            declared_bandwidth_bps: None,
            audio_renditions: Vec::new(),
            audio_rendition_sources: Vec::new(),
            closed_caption_overrides: Vec::new(),
            audio_profiles: Vec::new(),
            subtitles: true,
        }])
}

async fn status(client: &reqwest::Client, url: &str) -> reqwest::StatusCode {
    client.get(url).send().await.expect("GET").status()
}

#[tokio::test]
async fn an_srt_listener_route_serves_a_published_stream() {
    let (bind_addr, bind_listener) = bind_tcp();
    let srt_addr: SocketAddr = ([127, 0, 0, 1], test_fixtures::process::free_udp_port()).into();
    let server = tokio::spawn(async move {
        let _ = multimux::Server::new(
            multimux::ServeRequest::new(config(bind_addr, srt_addr))
                .with_registry(SchemeRegistry::new())
                .with_prebound_media(bind_listener),
        )
        .serve()
        .await;
    });

    let client = crate::support::http::client();
    let readyz = format!("http://{bind_addr}/readyz");
    let mut up = false;
    for _ in 0..200 {
        if client.get(&readyz).send().await.is_ok() {
            up = true;
            break;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    assert!(up, "serve never answered /readyz");
    assert_eq!(
        status(&client, &readyz).await,
        reqwest::StatusCode::SERVICE_UNAVAILABLE
    );
    assert!(!server.is_finished(), "serve exited without a publisher");

    let ts = test_fixtures::read("ts/h264-aac.ts");
    let publisher = tokio::spawn(async move {
        let cfg = HandshakeConfig::default();
        let mut sock = None;
        for _ in 0..400 {
            match SrtSocket::connect(srt_addr, SrtSocketConfig::new(cfg.clone())).await {
                Ok(s) => {
                    sock = Some(s);
                    break;
                }
                Err(_) => tokio::time::sleep(Duration::from_millis(25)).await,
            }
        }
        let mut sock = sock.expect("connect to the route's SRT listener");
        for chunk in ts.chunks(SRT_PAYLOAD_TS_PACKETS * TS_PACKET_BYTES) {
            sock.send(chunk).await.expect("send TS payload");
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
        std::future::pending::<()>().await;
    });

    let playlist = tokio::time::timeout(
        GUARD,
        poll_until_extinf(&client, &format!("http://{bind_addr}/cam/media.m3u8")),
    )
    .await
    .expect("bounded");
    assert!(playlist.contains("#EXTINF:"), "{playlist}");
    assert_eq!(
        status(&client, &readyz).await,
        reqwest::StatusCode::OK,
        "a route with a live publisher is ready"
    );
    assert!(!server.is_finished());

    publisher.abort();
    server.abort();
}
