//! Drive a real RTMP publish through `AsyncRtmpServer`/`RtmpConnection`.
//!
//! Originally the #738 Task 8 harness that captured
//! `rtmp-runtime/tests/fixtures/obs-publish.bin` by hand-rolling the socket
//! plumbing; that fixture is now committed (see
//! `tests/fixtures/PROVENANCE.md`), so this example instead demonstrates the
//! Task 9 tokio adapter (`rtmp_runtime::io::AsyncRtmpServer`) as its real
//! consumer: bind, accept one publisher, and print each `ServerEvent` as the
//! adapter drives the session over the socket.
//!
//! Run this, then in another terminal drive a real RTMP publisher at it, e.g.:
//! ```text
//! ffmpeg -re -i fixtures/ts/h264_aac.ts -t 2 -c copy -f flv rtmp://127.0.0.1:1935/live/testkey
//! ```
//!
//! `required-features = ["tokio"]` (see `Cargo.toml`) — the sans-IO core
//! itself needs no socket/runtime; only this example does.

use rtmp_runtime::io::AsyncRtmpServer;
use rtmp_runtime::server::{ServerConfig, ServerEvent};

const LISTEN_ADDR: &str = "127.0.0.1:1935";

#[tokio::main(flavor = "current_thread")]
async fn main() -> std::io::Result<()> {
    let server = AsyncRtmpServer::bind(LISTEN_ADDR, ServerConfig::default()).await?;
    eprintln!("capture_publish: listening on {LISTEN_ADDR}, waiting for one RTMP publish...");

    let mut conn = server.accept().await?;
    eprintln!(
        "capture_publish: accepted connection from {}",
        conn.peer_addr()?
    );

    let mut media_count = 0usize;
    while let Some(events) = conn.next_events().await? {
        for event in &events {
            match event {
                ServerEvent::Connected { app } => {
                    eprintln!("capture_publish: Connected {{ app: {app:?} }}");
                }
                ServerEvent::Publish {
                    app,
                    stream_key,
                    stream_id,
                } => {
                    eprintln!(
                        "capture_publish: Publish {{ app: {app:?}, stream_key: {stream_key:?}, stream_id: {stream_id} }}"
                    );
                }
                ServerEvent::Media { flv } => {
                    media_count += 1;
                    eprintln!(
                        "capture_publish: Media #{media_count} ({} bytes)",
                        flv.len()
                    );
                }
                ServerEvent::Eof => {
                    eprintln!("capture_publish: Eof");
                }
                other => {
                    eprintln!("capture_publish: unrecognised event: {other:?}");
                }
            }
        }
    }
    eprintln!("capture_publish: connection closed ({media_count} Media events total)");

    Ok(())
}
