//! Capture a real RTMP publish into a fixture file.
//!
//! Throwaway-but-kept recorder (#738 Task 8): binds `127.0.0.1:1935`, accepts
//! one inbound connection, drives it through the real sans-IO
//! [`rtmp_runtime::server::ServerSession`] (writing back whatever reply bytes
//! the session produces), and records every raw inbound byte it read off the
//! socket. On EOF (peer closed, e.g. after `deleteStream`/`FCUnpublish` or
//! just dropping the TCP connection) it writes the accumulated bytes to
//! `rtmp-runtime/tests/fixtures/obs-publish.bin` — the fixture
//! `tests/ingest_fixture.rs` replays offline.
//!
//! Run this, then in another terminal drive a real RTMP publisher at it, e.g.:
//! ```text
//! ffmpeg -re -i fixtures/ts/h264_aac.ts -t 2 -c copy -f flv rtmp://127.0.0.1:1935/live/testkey
//! ```
//!
//! `required-features = ["tokio"]` (see `Cargo.toml`) — the sans-IO core
//! itself needs no socket/runtime; only this capture example does.

use std::path::PathBuf;

use rtmp_runtime::server::ServerSession;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;

const LISTEN_ADDR: &str = "127.0.0.1:1935";
const FIXTURE_PATH: &str = "rtmp-runtime/tests/fixtures/obs-publish.bin";
const READ_BUF_LEN: usize = 64 * 1024;

#[tokio::main(flavor = "current_thread")]
async fn main() -> std::io::Result<()> {
    let listener = TcpListener::bind(LISTEN_ADDR).await?;
    eprintln!("capture_publish: listening on {LISTEN_ADDR}, waiting for one RTMP publish...");

    let (mut socket, peer) = listener.accept().await?;
    eprintln!("capture_publish: accepted connection from {peer}");

    let mut session = ServerSession::with_defaults();
    let mut recorded = Vec::new();
    let mut read_buf = vec![0u8; READ_BUF_LEN];
    let mut media_count = 0usize;

    loop {
        let n = socket.read(&mut read_buf).await?;
        if n == 0 {
            eprintln!("capture_publish: peer closed the connection (EOF)");
            break;
        }
        let inbound = &read_buf[..n];
        recorded.extend_from_slice(inbound);

        let (out, events) = match session.handle_data(inbound) {
            Ok(v) => v,
            Err(e) => {
                eprintln!("capture_publish: ServerSession error: {e}");
                break;
            }
        };
        if !out.is_empty() {
            socket.write_all(&out).await?;
        }

        let mut saw_eof = false;
        for event in &events {
            match event {
                rtmp_runtime::server::ServerEvent::Connected { app } => {
                    eprintln!("capture_publish: Connected {{ app: {app:?} }}");
                }
                rtmp_runtime::server::ServerEvent::Publish {
                    app,
                    stream_key,
                    stream_id,
                } => {
                    eprintln!(
                        "capture_publish: Publish {{ app: {app:?}, stream_key: {stream_key:?}, stream_id: {stream_id} }}"
                    );
                }
                rtmp_runtime::server::ServerEvent::Media { flv } => {
                    media_count += 1;
                    eprintln!(
                        "capture_publish: Media #{media_count} ({} bytes)",
                        flv.len()
                    );
                }
                rtmp_runtime::server::ServerEvent::Eof => {
                    eprintln!("capture_publish: Eof");
                    saw_eof = true;
                }
                other => {
                    eprintln!("capture_publish: unrecognised event: {other:?}");
                }
            }
        }
        if saw_eof {
            break;
        }
    }

    let path = PathBuf::from(FIXTURE_PATH);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(&path, &recorded)?;
    eprintln!(
        "capture_publish: wrote {} bytes ({} Media events) to {}",
        recorded.len(),
        media_count,
        path.display()
    );

    Ok(())
}
