//! New client adapter (SP6.1) against `AsyncRtmpServer` over real loopback sockets,
//! and the bounded failure modes over `duplex` + paused time.
#![cfg(feature = "tokio")]

use std::io::ErrorKind;
use std::time::Duration;

use broadcast_common::timeouts::Timeouts;
use rtmp_runtime::amf0::{Amf0Value, Command};
use rtmp_runtime::chunk::ChunkAssembler;
use rtmp_runtime::handshake::Handshake;
use rtmp_runtime::io::{
    AcceptedRtmpConnection, AsyncRtmpClient, AsyncRtmpServer, RtmpClientConfig, RtmpConnection,
    RtmpServerConfig,
};
use rtmp_runtime::message::{ChunkSize, MessageTypeId};
use rtmp_runtime::server::ServerEvent;
use rtmp_runtime::target::RtmpTarget;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

/// The tag type, timestamp and body of one FLV tag (Adobe FLV v10.1 Annex
/// E.4.1), skipping the file header when the run starts with one.
fn flv_tag(flv: &[u8]) -> (u8, u32, &[u8]) {
    let start = if flv.starts_with(b"FLV") { 13 } else { 0 };
    let tag_type = flv[start];
    let data_size =
        u32::from_be_bytes([0, flv[start + 1], flv[start + 2], flv[start + 3]]) as usize;
    let low = u32::from_be_bytes([0, flv[start + 4], flv[start + 5], flv[start + 6]]);
    let timestamp = low | (u32::from(flv[start + 7]) << 24);
    (
        tag_type,
        timestamp,
        &flv[start + 11..start + 11 + data_size],
    )
}

async fn publish_roundtrip(bind: &str, url_host: &str) {
    let server = match AsyncRtmpServer::bind(bind, RtmpServerConfig::default()).await {
        Ok(s) => s,
        // Only a host genuinely without IPv6 loopback may skip; anything else is a failure.
        Err(e) if e.kind() == ErrorKind::AddrNotAvailable || e.raw_os_error() == Some(97) => {
            eprintln!("skipping {bind}: {e}");
            return;
        }
        Err(e) => panic!("bind {bind}: {e}"),
    };
    let port = server.local_addr().unwrap().port();
    let accepted = tokio::spawn(async move {
        let mut conn = server.accept().await.unwrap().into_connection();
        let mut seen = Vec::new();
        while let Some(batch) = conn.next_events().await.unwrap() {
            seen.extend(batch);
            if seen.iter().any(|e| matches!(e, ServerEvent::Media { .. })) {
                break;
            }
        }
        seen
    });
    let target = RtmpTarget::parse(&format!("rtmp://{url_host}:{port}/live/testkey")).unwrap();
    let mut c = AsyncRtmpClient::connect(RtmpClientConfig::new(target))
        .await
        .unwrap();
    c.publish().await.unwrap();
    c.send_video(
        rtmp_runtime::ids::Timestamp::from_millis(0),
        &[0x17, 0x01, 0, 0, 0, 0xDE, 0xAD],
    )
    .await
    .unwrap();
    let seen = accepted.await.unwrap();
    assert!(
        seen.iter()
            .any(|e| matches!(e, ServerEvent::Connected { app } if app == "live"))
    );
    assert!(
        seen.iter().any(
            |e| matches!(e, ServerEvent::Publish { stream_key, .. } if stream_key == "testkey")
        )
    );
    assert!(seen.iter().any(|e| matches!(e, ServerEvent::Media { .. })));
}

#[tokio::test]
async fn publishes_over_ipv4_loopback() {
    publish_roundtrip("127.0.0.1:0", "127.0.0.1").await;
}

/// Defect 8 end to end: the bracketed IPv6 URL both resolves and connects.
#[tokio::test]
async fn publishes_over_ipv6_loopback() {
    publish_roundtrip("[::1]:0", "[::1]").await;
}

#[tokio::test(start_paused = true)]
async fn publish_to_a_peer_that_never_answers_times_out_at_handshake() {
    let (client_io, _server_io) = tokio::io::duplex(8192);
    let target = RtmpTarget::parse("rtmp://h/live/k").unwrap();
    let config = RtmpClientConfig::new(target)
        .with_timeouts(Timeouts::default().with_handshake(Duration::from_secs(4)));
    assert_eq!(
        config.timeouts.handshake,
        Duration::from_secs(4),
        "the config carries the handshake bound"
    );
    let mut c = AsyncRtmpClient::from_stream(client_io, config);
    let t0 = tokio::time::Instant::now();
    let err = c.publish().await.expect_err("handshake deadline");
    assert_eq!(err.kind(), ErrorKind::TimedOut, "{err}");
    assert_eq!(t0.elapsed(), Duration::from_secs(4));
}

#[tokio::test]
async fn a_server_that_rejects_publish_surfaces_connection_refused() {
    let server = AsyncRtmpServer::bind(
        "127.0.0.1:0",
        RtmpServerConfig::default().with_session(
            rtmp_runtime::server::RtmpServerConfig::default()
                .with_expected_stream_key(Some("right".into())),
        ),
    )
    .await
    .unwrap();
    let port = server.local_addr().unwrap().port();
    tokio::spawn(async move {
        let mut conn = server.accept().await.unwrap().into_connection();
        while let Ok(Some(_)) = conn.next_events().await {}
    });
    let target = RtmpTarget::parse(&format!("rtmp://127.0.0.1:{port}/live/wrong")).unwrap();
    let mut c = AsyncRtmpClient::connect(RtmpClientConfig::new(target))
        .await
        .unwrap();
    let err = c.publish().await.expect_err("BadName");
    assert_eq!(err.kind(), ErrorKind::ConnectionRefused, "{err}");
}

/// A client already publishing against a server that stops reading: `send_video` must hit the
/// write bound (review: the `queue` flush was unbounded and untested). Virtual-time hang guard so a
/// missing bound fails instead of hanging.
#[tokio::test(start_paused = true)]
async fn send_video_to_a_peer_that_stops_reading_times_out_the_write() {
    let (client_io, server_io) = tokio::io::duplex(4096);
    tokio::spawn(async move {
        let mut conn = RtmpConnection::from_stream(server_io, RtmpServerConfig::default());
        while let Ok(Some(batch)) = conn.next_events().await {
            if batch
                .iter()
                .any(|e| matches!(e, ServerEvent::Publish { .. }))
            {
                std::future::pending::<()>().await; // stop reading, keep the pipe open
            }
        }
    });
    let target = RtmpTarget::parse("rtmp://h/live/k").unwrap();
    let mut c = AsyncRtmpClient::from_stream(
        client_io,
        RtmpClientConfig::new(target)
            .with_timeouts(Timeouts::default().with_write(Duration::from_secs(4))),
    );
    c.publish().await.unwrap();
    let t0 = tokio::time::Instant::now();
    let big = vec![0x42u8; 256 * 1024];
    let err = tokio::time::timeout(
        Duration::from_secs(60),
        c.send_video(rtmp_runtime::ids::Timestamp::from_millis(0), &big),
    )
    .await
    .expect("send_video must be bounded by `write`, not hang")
    .expect_err("peer is not reading");
    assert_eq!(err.kind(), ErrorKind::TimedOut, "{err}");
    assert_eq!(t0.elapsed(), Duration::from_secs(4));
}

/// The client's `next_events` honours `read_idle` once publishing (a silent server).
#[tokio::test(start_paused = true)]
async fn next_events_on_a_silent_server_times_out_at_read_idle() {
    let (client_io, server_io) = tokio::io::duplex(64 * 1024);
    tokio::spawn(async move {
        let mut conn = RtmpConnection::from_stream(server_io, RtmpServerConfig::default());
        while let Ok(Some(batch)) = conn.next_events().await {
            if batch
                .iter()
                .any(|e| matches!(e, ServerEvent::Publish { .. }))
            {
                std::future::pending::<()>().await;
            }
        }
    });
    let target = RtmpTarget::parse("rtmp://h/live/k").unwrap();
    let mut c = AsyncRtmpClient::from_stream(
        client_io,
        RtmpClientConfig::new(target)
            .with_timeouts(Timeouts::default().with_read_idle(Duration::from_secs(5))),
    );
    c.publish().await.unwrap();
    let t0 = tokio::time::Instant::now();
    let err = loop {
        match tokio::time::timeout(Duration::from_secs(60), c.next_events())
            .await
            .expect("bounded by read_idle")
        {
            Ok(Some(_)) => continue,
            Ok(None) => panic!("closed"),
            Err(e) => break e,
        }
    };
    assert_eq!(err.kind(), ErrorKind::TimedOut, "{err}");
    assert!(t0.elapsed() <= Duration::from_secs(5));
}
/// Audit 22 (finding 6): the audio and metadata halves of the adapter — both
/// the sans-IO `encode_*` frames and the flushing `send_*` calls — reach a
/// real server with their payload and timestamp intact.
#[tokio::test]
async fn audio_and_metadata_traverse_the_adapter() {
    let server = AsyncRtmpServer::bind("127.0.0.1:0", RtmpServerConfig::default())
        .await
        .expect("bind loopback");
    let port = server.local_addr().expect("addr").port();
    let server_task = tokio::spawn(async move {
        let mut conn = server.accept().await.expect("accept").into_connection();
        let mut events = Vec::new();
        let mut media = 0usize;
        while let Ok(Ok(Some(batch))) =
            tokio::time::timeout(Duration::from_secs(5), conn.next_events()).await
        {
            media += batch
                .iter()
                .filter(|e| matches!(e, ServerEvent::Media { .. }))
                .count();
            events.extend(batch);
            if media >= 2 {
                break;
            }
        }
        events
    });
    let target = RtmpTarget::parse(&format!("rtmp://127.0.0.1:{port}/live/testkey")).unwrap();
    let mut client = AsyncRtmpClient::connect(RtmpClientConfig::new(target))
        .await
        .expect("connect");
    client.publish().await.expect("publish");

    let audio = client
        .encode_audio(rtmp_runtime::ids::Timestamp::from_millis(7), &[0xAF, 0x02])
        .expect("encode audio");
    let mut assembler = ChunkAssembler::new();
    assembler.set_chunk_size(ChunkSize::new(4096).unwrap());
    let decoded = assembler.push(&audio).expect("decode audio frame");
    assert_eq!(decoded.len(), 1);
    assert_eq!(decoded[0].message_type_id, MessageTypeId::Audio);
    assert_eq!(decoded[0].payload, vec![0xAF, 0x02]);
    assert_eq!(decoded[0].timestamp.millis(), 7);

    let metadata = client
        .encode_metadata(&[
            ("width".to_string(), Amf0Value::Number(1280.0)),
            ("height".to_string(), Amf0Value::Number(720.0)),
        ])
        .expect("encode metadata");
    let mut assembler = ChunkAssembler::new();
    assembler.set_chunk_size(ChunkSize::new(4096).unwrap());
    let decoded = assembler.push(&metadata).expect("decode metadata frame");
    assert_eq!(decoded.len(), 1);
    assert_eq!(decoded[0].message_type_id, MessageTypeId::DataAmf0);
    let mut expected = vec![0x02, 0x00, 0x0D];
    expected.extend_from_slice(b"@setDataFrame");
    expected.extend_from_slice(&[0x02, 0x00, 0x0A]);
    expected.extend_from_slice(b"onMetaData");
    assert!(
        decoded[0].payload.starts_with(&expected),
        "{:?}",
        decoded[0].payload
    );

    client
        .send_audio(rtmp_runtime::ids::Timestamp::from_millis(21), &[0xAF, 0x03])
        .await
        .expect("send audio");
    client
        .send_metadata(&[("width".to_string(), Amf0Value::Number(640.0))])
        .await
        .expect("send metadata");

    let seen = tokio::time::timeout(Duration::from_secs(30), server_task)
        .await
        .expect("server must finish")
        .expect("server task");
    let medias: Vec<&Vec<u8>> = seen
        .iter()
        .filter_map(|e| match e {
            ServerEvent::Media { flv } => Some(flv),
            _ => None,
        })
        .collect();
    assert_eq!(medias.len(), 2, "{seen:?}");

    let audio_tag = medias
        .iter()
        .find(|flv| flv_tag(flv).0 == 8)
        .expect("audio tag");
    assert_eq!(flv_tag(audio_tag), (8, 21, &[0xAF, 0x03][..]));

    let meta_tag = medias
        .iter()
        .find(|flv| flv_tag(flv).0 == 18)
        .expect("metadata tag");
    let (_, _, body) = flv_tag(meta_tag);
    assert!(body.windows(5).any(|w| w == b"width"), "{body:?}");
    assert!(!body.windows(6).any(|w| w == b"height"), "{body:?}");
}

/// Audit 22 (finding 6): the adapter's `connect` carries the `tcUrl` derived
/// from the target, not the client's `rtmp://localhost/<app>` fallback.
#[tokio::test]
async fn adapter_sends_the_configured_tc_url_in_connect() {
    let (client_io, mut server_io) = tokio::io::duplex(64 * 1024);
    let target = RtmpTarget::parse("rtmp://ingest.example/live/key").unwrap();
    let mut client = AsyncRtmpClient::from_stream(client_io, RtmpClientConfig::new(target));

    let server = tokio::spawn(async move {
        let mut c0_c1 = vec![0u8; 1 + rtmp_runtime::handshake::HANDSHAKE_PACKET_LEN];
        server_io.read_exact(&mut c0_c1).await.expect("C0+C1");
        let mut handshake = Handshake::new();
        let (s0_s1_s2, _, _) = handshake.read(&c0_c1).expect("S0+S1+S2");
        server_io
            .write_all(&s0_s1_s2)
            .await
            .expect("write S0+S1+S2");
        let mut c2 = vec![0u8; rtmp_runtime::handshake::HANDSHAKE_PACKET_LEN];
        server_io.read_exact(&mut c2).await.expect("C2");

        let mut assembler = ChunkAssembler::new();
        assembler.set_chunk_size(ChunkSize::new(4096).unwrap());
        let mut buf = [0u8; 4096];
        loop {
            let n = server_io.read(&mut buf).await.expect("read");
            if n == 0 {
                return None;
            }
            assembler.feed(&buf[..n]);
            while let Some(message) = assembler.next_message().expect("message") {
                if message.message_type_id != MessageTypeId::CommandAmf0 {
                    continue;
                }
                let command = Command::parse(&message.payload).expect("command");
                if command.name == "connect" {
                    return Some(command);
                }
            }
        }
    });

    let client_task = tokio::spawn(async move { client.publish().await });
    let command = tokio::time::timeout(Duration::from_secs(30), server)
        .await
        .expect("server must finish")
        .expect("server task")
        .expect("connect command");
    let _ = tokio::time::timeout(Duration::from_secs(30), client_task).await;

    let tc_url = command
        .arguments
        .first()
        .and_then(|argument| match argument {
            Amf0Value::Object(pairs) => pairs
                .iter()
                .find(|(key, _)| key == "tcUrl")
                .map(|(_, value)| value.clone()),
            _ => None,
        });
    assert_eq!(
        tc_url,
        Some(Amf0Value::String("rtmp://ingest.example/live".to_string()))
    );
}

/// Audit 22 (finding 8): the debug output names the connection's peer.
#[test]
fn accepted_connection_debug_names_the_peer() {
    let (client_io, _server_io) = tokio::io::duplex(64);
    let conn = RtmpConnection::from_stream(client_io, RtmpServerConfig::default());
    let accepted = AcceptedRtmpConnection::new(conn, "127.0.0.1:1935".parse().unwrap());
    let text = format!("{accepted:?}");
    assert!(text.contains("peer"), "{text}");
    assert!(text.contains("127.0.0.1:1935"), "{text}");
}
