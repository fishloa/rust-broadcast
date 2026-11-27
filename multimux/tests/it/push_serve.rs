//! `serve` wiring of push outputs (#3679): a route started through
//! `multimux::Server` with an `rtmp_push` / `rtsp_push` output dials its
//! destination with the URL's app and stream key, and its HTTP outputs stay
//! served beside it.

use std::net::SocketAddr;
use std::time::Duration;

use multimux::config::{Config, InputSpec, Route};
use multimux::dvr::DvrConfig;
use multimux::output::OutputKind;
use multimux::registry::SchemeRegistry;
use rtmp_runtime::server::{ServerEvent, ServerSession};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;

use crate::support::listener::bind_tcp;
use crate::support::shared_it::{fixture_string, poll_until_extinf};

const GUARD: Duration = Duration::from_secs(40);

fn route_with(outputs: Vec<OutputKind>) -> Route {
    Route {
        damage_policy: Default::default(),
        name: "cam".to_string(),
        input: InputSpec::File {
            path: fixture_string("ts/h264-aac.ts"),
            loop_file: true,
        },
        outputs,
        dvr: DvrConfig::default(),
        ts_hls_cut: Default::default(),
        ts_hls_idr_fallback: Default::default(),
        declared_bandwidth_bps: None,
        audio_renditions: Vec::new(),
        audio_rendition_sources: Vec::new(),
        closed_caption_overrides: Vec::new(),
        audio_profiles: Vec::new(),
        subtitles: true,
    }
}

fn config(bind: SocketAddr, outputs: Vec<OutputKind>) -> Config {
    Config::default()
        .with_bind(bind.to_string())
        .with_target_duration(Duration::from_millis(500))
        .with_part_target(Duration::from_millis(100))
        .with_window_segments(8)
        .with_routes(vec![route_with(outputs)])
}

fn serve(config: Config, bind_listener: TcpListener) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        let _ = multimux::Server::new(
            multimux::ServeRequest::new(config)
                .with_registry(SchemeRegistry::new())
                .with_prebound_media(bind_listener),
        )
        .serve()
        .await;
    })
}

async fn assert_http_outputs_still_served(bind_addr: SocketAddr) {
    let client = crate::support::http::client();
    let playlist = poll_until_extinf(&client, &format!("http://{bind_addr}/cam/media.m3u8")).await;
    assert!(playlist.contains("#EXTINF:"), "{playlist}");
}

#[derive(Default)]
struct Seen {
    connect_app: Option<String>,
    publish: Option<(String, String)>,
    media_events: usize,
}

/// A route with `rtmp_push` publishes to the URL's app and stream key and
/// ships media, while `llhls` is still served over HTTP.
#[tokio::test]
async fn an_rtmp_push_output_publishes_with_the_urls_app_and_key() {
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind rtmp");
    let rtmp_addr = listener.local_addr().expect("rtmp addr");
    let ingest = tokio::spawn(async move {
        let (mut sock, _) = listener.accept().await.expect("accept");
        let mut session = ServerSession::with_defaults();
        let mut seen = Seen::default();
        let mut buf = vec![0u8; 65536];
        while seen.media_events == 0 {
            let n = sock.read(&mut buf).await.expect("server read");
            assert_ne!(n, 0, "publisher closed before sending media");
            let (out, events) = session.handle_data(&buf[..n]).expect("decode");
            if !out.is_empty() {
                sock.write_all(&out).await.expect("reply");
            }
            for ev in events {
                match ev {
                    ServerEvent::Connected { app } => seen.connect_app = Some(app),
                    ServerEvent::Publish {
                        app, stream_key, ..
                    } => seen.publish = Some((app, stream_key)),
                    ServerEvent::Media { .. } => seen.media_events += 1,
                    _ => {}
                }
            }
        }
        seen
    });

    let (bind_addr, bind_listener) = bind_tcp();
    let server = serve(
        config(
            bind_addr,
            vec![
                OutputKind::LlHls,
                OutputKind::RtmpPush {
                    url: format!("rtmp://{rtmp_addr}/app/key"),
                    format: None,
                    reconnect: None,
                },
            ],
        ),
        bind_listener,
    );

    let seen = tokio::time::timeout(GUARD, ingest)
        .await
        .expect("the push output never reached the RTMP server")
        .expect("server task");
    assert_eq!(seen.connect_app.as_deref(), Some("app"));
    assert_eq!(seen.publish, Some(("app".to_string(), "key".to_string())));
    assert!(seen.media_events >= 1);
    assert_http_outputs_still_served(bind_addr).await;
    server.abort();
}

/// A route with `rtsp_push` dials the configured host and opens with an
/// `OPTIONS` for the URL's own path, while `llhls` is still served.
#[tokio::test]
async fn an_rtsp_push_output_dials_its_destination() {
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind rtsp");
    let rtsp_addr = listener.local_addr().expect("rtsp addr");
    let first_request = tokio::spawn(async move {
        let (mut sock, _) = listener.accept().await.expect("accept");
        let mut request = Vec::new();
        let mut buf = [0u8; 4096];
        while !request.windows(4).any(|w| w == b"\r\n\r\n") {
            let n = sock.read(&mut buf).await.expect("read");
            assert_ne!(n, 0, "client closed before finishing OPTIONS");
            request.extend_from_slice(&buf[..n]);
        }
        String::from_utf8(request).expect("utf8 request")
    });

    let (bind_addr, bind_listener) = bind_tcp();
    let server = serve(
        config(
            bind_addr,
            vec![
                OutputKind::LlHls,
                OutputKind::RtspPush {
                    url: format!("rtsp://{rtsp_addr}/ingest/key"),
                    format: None,
                    reconnect: None,
                },
            ],
        ),
        bind_listener,
    );

    let request = tokio::time::timeout(GUARD, first_request)
        .await
        .expect("the push output never reached the RTSP server")
        .expect("server task");
    let first_line = request.lines().next().expect("request line");
    assert_eq!(
        first_line,
        format!("OPTIONS rtsp://{rtsp_addr}/ingest/key RTSP/1.0")
    );
    assert_http_outputs_still_served(bind_addr).await;
    server.abort();
}
