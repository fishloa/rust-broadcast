//! RTMP push output — real-protocol payload-format test (issue #934).
//!
//! Before this fix, `multimux::push::drive_push` muxed **every** push output
//! with `transmux::TsMux` and `push::rtmp::RtmpTransport::send` shipped the
//! resulting MPEG-2 TS bytes as an RTMP `send_video` message payload — a
//! payload no RTMP server can decode (RTMP Audio/Video messages carry FLV
//! `AudioTagHeader`/`VideoTagHeader` bodies, not TS). This test exercises the
//! fix end-to-end over a **real** TCP loopback connection, using a real
//! independent RTMP server implementation (`rtmp_runtime::server::ServerSession`)
//! on the receiving end — not a mock, not an inspection of our own client's
//! byte construction — so the only way this test passes is if a genuine RTMP
//! server can decode what `RtmpTransport` sends.
//!
//! The fixture (`fixtures/flv/av.flv`, real H.264 + AAC, shared with
//! `transmux/tests/flv.rs`) is demuxed to get real `TrackSpec`s + samples,
//! pushed through `RtmpTransport::setup` (metadata + sequence headers) and
//! `send_media` (per-frame payloads), received by `ServerSession` (which
//! reconstructs FLV tags from the RTMP messages it decodes), and the
//! reconstructed FLV byte stream is re-demuxed with `transmux::FlvDemux` —
//! asserting the round-tripped samples are byte-identical to the source.
//! **The bite**: feed `ServerSession` raw MPEG-2 TS bytes (the pre-fix
//! defect) instead of FLV video/audio payloads and this re-demux fails
//! outright or the sample counts/bytes stop matching — there is no way for
//! wrong container framing to accidentally produce a byte-identical
//! round-trip.

use std::time::Duration;

use broadcast_common::Unpackage;
use multimux::push::{PushTransport, RtmpTransport, RtmpTransportConfig};
use rtmp_runtime::server::{ServerEvent, ServerSession};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;
use transmux::FlvDemux;
use transmux::ir::TrackSpec;

const FLV: &[u8] = include_bytes!("../../fixtures/flv/av.flv");

/// HANG GUARD (workspace precedent, issue #826): every blocking wait below is
/// bounded, so a broken connect/publish/media path fails the test rather than
/// hanging the suite.
const GUARD: Duration = Duration::from_secs(15);

#[tokio::test]
async fn rtmp_push_ships_flv_framed_media_a_real_rtmp_server_can_decode() {
    let mut demux = FlvDemux::new();
    let media = demux.unpackage(FLV).expect("demux av.flv fixture");
    assert_eq!(media.tracks.len(), 2, "fixture must carry AVC + AAC");

    let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
    let addr = listener.local_addr().expect("local addr");

    // The server side: a real, independent RTMP ingest implementation.
    // Reads raw bytes off the wire, drives `ServerSession`, and collects
    // every `Media` event's FLV tag bytes in arrival order.
    let server = tokio::spawn(async move {
        let (mut sock, _) = listener.accept().await.expect("accept");
        let mut session = ServerSession::with_defaults();
        let mut flv_bytes: Vec<u8> = Vec::new();
        let mut saw_publish = false;
        let mut buf = vec![0u8; 65536];
        loop {
            let n = match sock.read(&mut buf).await {
                Ok(0) => break,
                Ok(n) => n,
                Err(e) => panic!("server read failed: {e}"),
            };
            let (out, events) = session
                .handle_data(&buf[..n])
                .expect("server-side RTMP decode must not error");
            if !out.is_empty() {
                sock.write_all(&out).await.expect("server write reply");
            }
            for ev in events {
                match ev {
                    ServerEvent::Publish { .. } => saw_publish = true,
                    ServerEvent::Media { flv } => flv_bytes.extend_from_slice(&flv),
                    _ => {}
                }
            }
        }
        (flv_bytes, saw_publish)
    });

    // The client side: exactly what `crate::origin::spawn_push_outputs` +
    // `crate::push::drive_push` drive for a configured `rtmp_push` output.
    let cfg = RtmpTransportConfig {
        app: "live".to_string(),
        stream_key: "test".to_string(),
    };
    let url = format!("rtmp://{addr}/live/test");
    let mut transport = tokio::time::timeout(GUARD, RtmpTransport::connect(&url, &cfg))
        .await
        .expect("connect must not hang")
        .expect("connect");

    let tracks: Vec<TrackSpec> = media.tracks.iter().map(|t| t.spec.clone()).collect();
    tokio::time::timeout(GUARD, transport.setup(&tracks))
        .await
        .expect("setup must not hang")
        .expect("setup (onMetaData + sequence headers)");

    // One `send_media` call carrying every sample — `drive_push` normally
    // splits this into several smaller batches, but each batch goes through
    // this exact call, so one larger batch exercises the same path.
    tokio::time::timeout(GUARD, transport.send_media(&media))
        .await
        .expect("send_media must not hang")
        .expect("send_media");

    // Close the client side so the server's `read` sees EOF and returns.
    transport.close();
    drop(transport);

    let (flv_bytes, saw_publish) = tokio::time::timeout(GUARD, server)
        .await
        .expect("server task must not hang")
        .expect("server task panicked");
    assert!(saw_publish, "server must have accepted the publish");
    assert!(!flv_bytes.is_empty(), "server must have received media");

    // THE BITE: a real, independent RTMP server decoded what `RtmpTransport`
    // sent into FLV tags; re-demuxing those tags must recover the source
    // media byte-identically. MPEG-2 TS bytes sent as a video/audio payload
    // (the pre-#934 defect) would not survive this round-trip: either
    // `FlvDemux` errors outright on a malformed AVCVIDEOPACKET/AACAUDIODATA
    // body, or the recovered sample boundaries/bytes simply don't match.
    let mut demux2 = FlvDemux::new();
    let media2 = demux2
        .unpackage(&flv_bytes)
        .expect("re-demux the RTMP-received FLV tags");
    assert_eq!(
        media2.tracks.len(),
        2,
        "server must recover both AVC + AAC tracks"
    );
    for (a, b) in media.tracks.iter().zip(&media2.tracks) {
        assert_eq!(
            a.samples.len(),
            b.samples.len(),
            "track {} sample count round-trips over real RTMP",
            a.track_id()
        );
        for (i, (sa, sb)) in a.samples.iter().zip(&b.samples).enumerate() {
            assert_eq!(
                sa.data,
                sb.data,
                "track {} sample {i} bytes round-trip over real RTMP",
                a.track_id()
            );
        }
    }
    assert_eq!(media.tracks[0].samples.len(), 75, "video sample count");
    assert_eq!(media.tracks[1].samples.len(), 131, "audio sample count");
}
