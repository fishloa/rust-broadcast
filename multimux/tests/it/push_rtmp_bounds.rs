//! RTMP push through multimux's `RtmpTransport`: the IPv6 tcUrl the peer
//! actually receives (defect 8) and the write bound on a stalled peer.
use std::time::Duration;

use multimux::push::{PushTransport, RtmpTransport, RtmpTransportConfig};
use rtmp_runtime::amf0::{Amf0Value, Command};
use rtmp_runtime::chunk::ChunkAssembler;
use rtmp_runtime::server::{ServerEvent, ServerSession};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;

const GUARD: Duration = Duration::from_secs(5);
/// RTMP simple handshake bytes the client sends before its first chunk:
/// C0 (1) + C1 (1536) + C2 (1536) (Adobe RTMP 1.0 §5.2).
const HANDSHAKE_PREFIX: usize = 1 + 1536 + 1536;
/// RTMP message type id of an AMF0 command (Adobe RTMP 1.0 §7.1.1).
const MSG_COMMAND_AMF0: u8 = 20;

/// `write_timeout` is the config knob that mirrors the existing
/// `connect_timeout: Option<Duration>`.
fn config_with_write_bound() -> RtmpTransportConfig {
    RtmpTransportConfig {
        write_timeout: Some(Duration::from_millis(300)),
        ..Default::default()
    }
}

/// What the scripted peer does once the client's `publish` was accepted.
#[derive(Clone, Copy)]
enum AfterPublish {
    /// Read until the client closes, returning every raw byte received.
    DrainToEof,
    /// Hold the socket open and never read again.
    StopReading,
}

/// A real RTMP ingest (`ServerSession`) behind `listener`; returns the raw
/// bytes the client sent (DrainToEof) after the connection ends.
fn spawn_peer(listener: TcpListener, after: AfterPublish) -> tokio::task::JoinHandle<Vec<u8>> {
    tokio::spawn(async move {
        let (mut sock, _) = listener.accept().await.expect("accept");
        let mut session = ServerSession::with_defaults();
        let mut raw = Vec::new();
        let mut buf = vec![0u8; 65536];
        loop {
            let n = match sock.read(&mut buf).await {
                Ok(0) | Err(_) => return raw,
                Ok(n) => n,
            };
            raw.extend_from_slice(&buf[..n]);
            let (out, events) = session.handle_data(&buf[..n]).expect("server decode");
            if !out.is_empty() {
                sock.write_all(&out).await.expect("server reply");
            }
            let published = events
                .iter()
                .any(|e| matches!(e, ServerEvent::Publish { .. }));
            if published && matches!(after, AfterPublish::StopReading) {
                std::future::pending::<()>().await;
            }
        }
    })
}

/// The `tcUrl` property of the `connect` command in the client's raw bytes.
fn captured_tc_url(raw: &[u8]) -> String {
    assert!(
        raw.len() > HANDSHAKE_PREFIX,
        "client sent no chunks after the handshake"
    );
    // The client announces its own chunk size (SetChunkSize) before `connect`
    // and frames everything after it with that size.
    let messages = ChunkAssembler::new()
        .with_chunk_size(rtmp_runtime::client::ClientConfig::default().chunk_size)
        .push(&raw[HANDSHAKE_PREFIX..])
        .expect("the client's chunk stream decodes");
    for m in messages
        .iter()
        .filter(|m| m.message_type_id == MSG_COMMAND_AMF0)
    {
        let cmd = Command::parse(&m.payload).expect("amf0 command");
        if cmd.name != "connect" {
            continue;
        }
        let Some(Amf0Value::Object(props)) = cmd.arguments.first() else {
            panic!("connect has no command object: {cmd:?}")
        };
        return props
            .iter()
            .find_map(|(k, v)| match (k.as_str(), v) {
                ("tcUrl", Amf0Value::String(s)) => Some(s.clone()),
                _ => None,
            })
            .expect("connect carries tcUrl");
    }
    panic!("no connect command in the client's bytes");
}

/// CONTROL (IPv4): the extraction works on a healthy push.
#[tokio::test]
async fn control_the_captured_tc_url_is_readable_for_an_ipv4_target() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let peer = spawn_peer(listener, AfterPublish::DrainToEof);
    let cfg = RtmpTransportConfig {
        stream_key: "key".into(),
        ..Default::default()
    };
    let mut t = tokio::time::timeout(
        GUARD,
        RtmpTransport::connect(&format!("rtmp://127.0.0.1:{port}/live/key"), &cfg),
    )
    .await
    .expect("connect must not hang")
    .expect("connect");
    t.close();
    drop(t);
    let raw = tokio::time::timeout(GUARD, peer)
        .await
        .expect("peer")
        .expect("join");
    assert_eq!(
        captured_tc_url(&raw),
        format!("rtmp://127.0.0.1:{port}/live")
    );
}

/// Defect 8: an IPv6 push target's `tcUrl` must keep its brackets — what the
/// server receives, not what a helper computes.
///
/// Revert-check: build the tcUrl with `format!("rtmp://{host}:{port}/{app}")`
/// from `Url::host_str()` again (host WITHOUT its brackets) and this fails
/// with `rtmp://::1:PORT/live`.
#[tokio::test]
async fn an_ipv6_push_target_sends_a_bracketed_tc_url() {
    let Ok(listener) = TcpListener::bind("[::1]:0").await else {
        eprintln!(
            "SKIP an_ipv6_push_target_sends_a_bracketed_tc_url: no IPv6 loopback on this host"
        );
        return;
    };
    let port = listener.local_addr().unwrap().port();
    let peer = spawn_peer(listener, AfterPublish::DrainToEof);
    let cfg = RtmpTransportConfig {
        stream_key: "key".into(),
        ..Default::default()
    };
    let mut t = tokio::time::timeout(
        GUARD,
        RtmpTransport::connect(&format!("rtmp://[::1]:{port}/live/key"), &cfg),
    )
    .await
    .expect("connect must not hang")
    .expect("connect over IPv6");
    t.close();
    drop(t);
    let raw = tokio::time::timeout(GUARD, peer)
        .await
        .expect("peer")
        .expect("join");
    assert_eq!(captured_tc_url(&raw), format!("rtmp://[::1]:{port}/live"));
}

/// A peer that accepted the publish and then stopped reading fails `send` at
/// the write bound instead of blocking forever.
#[tokio::test]
async fn a_stalled_peer_fails_send_at_the_write_bound() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let _peer = spawn_peer(listener, AfterPublish::StopReading);
    let cfg = RtmpTransportConfig {
        stream_key: "key".into(),
        ..config_with_write_bound()
    };
    let mut t = tokio::time::timeout(
        GUARD,
        RtmpTransport::connect(&format!("rtmp://127.0.0.1:{port}/live/key"), &cfg),
    )
    .await
    .expect("connect must not hang")
    .expect("connect");
    let chunk = vec![0u8; 256 * 1024];
    let failed = tokio::time::timeout(GUARD, async {
        for _ in 0..4096 {
            // 4096 * 256 KiB = 1 GiB: far beyond any loopback buffer.
            if t.send(&chunk).await.is_err() {
                return true;
            }
        }
        false
    })
    .await
    .expect(
        "send must fail at the write bound, not block forever against a peer that stopped reading",
    );
    assert!(
        failed,
        "a peer that stopped reading must eventually fail a send"
    );
}
