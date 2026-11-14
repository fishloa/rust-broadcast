//! Wire goldens for the push transports, recorded **before** the RTMP
//! `onMetaData`/FLV codec gate and the `srt://` URL parser moved out of
//! `multimux` into `transmux` and `srt-runtime` (issue #2369).
//!
//! - `fixtures/golden/rtmp/push-*.txt`: the exact `onMetaData` (type 18),
//!   sequence-header and first media message bodies `RtmpTransport` publishes,
//!   captured off a real loopback socket by an independent RTMP server
//!   (`rtmp_runtime::server::ServerSession`). The bytes must not change when
//!   the builders move.
//! - `fixtures/golden/srt/push-url-table.txt`: the parse outcome of every
//!   `srt://` URL shape the push accepts or rejects.
//!
//! Regenerate only with a reviewed diff, via `REGENERATE_PUSH_GOLDENS=1`
//! (RTMP) and `REGENERATE_PUSH_URL_TABLE=1` (SRT).

use std::fmt::Write as _;
use std::time::Duration;

use broadcast_common::Unpackage;
use multimux::push::{PushTransport, RtmpTransport, RtmpTransportConfig};
use rtmp_runtime::server::{ServerEvent, ServerSession};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;
use transmux::FlvDemux;
use transmux::ir::TrackSpec;

static FLV: std::sync::LazyLock<Vec<u8>> =
    std::sync::LazyLock::new(|| test_fixtures::read("flv/av.flv"));

const GUARD: Duration = Duration::from_secs(15);

/// Adobe FLV v10.1 §E.4.1/§E.2 framing, used to split the server's
/// reconstructed FLV byte stream into tags.
const FLV_HEADER_LEN: usize = 9;
const FLV_PREV_TAG_SIZE_LEN: usize = 4;
const FLV_TAG_HEADER_LEN: usize = 11;
const TAG_TYPE_SCRIPT: u8 = 18;

fn hex(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        let _ = write!(out, "{b:02x}");
    }
    out
}

/// `(tag_type, timestamp)` of the first `count` tags in `flv`.
fn first_tag_headers(flv: &[u8], count: usize) -> Vec<(u8, u32)> {
    let mut headers = Vec::with_capacity(count);
    let mut at = FLV_HEADER_LEN + FLV_PREV_TAG_SIZE_LEN;
    while headers.len() < count && at + FLV_TAG_HEADER_LEN <= flv.len() {
        let tag_type = flv[at];
        let data_size =
            (u32::from(flv[at + 1]) << 16) | (u32::from(flv[at + 2]) << 8) | u32::from(flv[at + 3]);
        let timestamp = (u32::from(flv[at + 7]) << 24)
            | (u32::from(flv[at + 4]) << 16)
            | (u32::from(flv[at + 5]) << 8)
            | u32::from(flv[at + 6]);
        headers.push((tag_type, timestamp));
        at += FLV_TAG_HEADER_LEN + data_size as usize + FLV_PREV_TAG_SIZE_LEN;
    }
    headers
}

/// The body (without the 11-byte tag header) of the `n`th tag in `flv`.
fn tag_body(flv: &[u8], n: usize) -> &[u8] {
    let mut at = FLV_HEADER_LEN + FLV_PREV_TAG_SIZE_LEN;
    for _ in 0..n {
        let data_size =
            (u32::from(flv[at + 1]) << 16) | (u32::from(flv[at + 2]) << 8) | u32::from(flv[at + 3]);
        at += FLV_TAG_HEADER_LEN + data_size as usize + FLV_PREV_TAG_SIZE_LEN;
    }
    let data_size =
        (u32::from(flv[at + 1]) << 16) | (u32::from(flv[at + 2]) << 8) | u32::from(flv[at + 3]);
    &flv[at + FLV_TAG_HEADER_LEN..at + FLV_TAG_HEADER_LEN + data_size as usize]
}

#[tokio::test]
async fn rtmp_push_wire_bodies_match_the_recorded_golden() {
    let mut demux = FlvDemux::new();
    let media = demux
        .unpackage(FLV.as_slice())
        .expect("demux av.flv fixture");

    let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
    let addr = listener.local_addr().expect("local addr");

    let server = tokio::spawn(async move {
        let (mut sock, _) = listener.accept().await.expect("accept");
        let mut session = ServerSession::with_defaults();
        let mut flv_bytes: Vec<u8> = Vec::new();
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
                if let ServerEvent::Media { flv } = ev {
                    flv_bytes.extend_from_slice(&flv);
                }
            }
        }
        flv_bytes
    });

    let cfg = RtmpTransportConfig::default()
        .with_app("live".to_string())
        .with_stream_key("test".to_string());
    let url = format!("rtmp://{addr}/live/test");
    let mut transport = tokio::time::timeout(GUARD, RtmpTransport::connect(&url, &cfg))
        .await
        .expect("connect must not hang")
        .expect("connect");

    let tracks: Vec<TrackSpec> = media.tracks.iter().map(|t| t.spec.clone()).collect();
    tokio::time::timeout(GUARD, transport.setup(&tracks))
        .await
        .expect("setup must not hang")
        .expect("setup");

    tokio::time::timeout(GUARD, transport.send_media(&media))
        .await
        .expect("send_media must not hang")
        .expect("send_media");

    transport.close();
    drop(transport);

    let flv_bytes = tokio::time::timeout(GUARD, server)
        .await
        .expect("server task must not hang")
        .expect("server task panicked");

    let headers = first_tag_headers(&flv_bytes, 4);
    let meta_tag = headers
        .iter()
        .position(|(t, _)| *t == TAG_TYPE_SCRIPT)
        .expect("a script (onMetaData) tag must be the first thing published");

    let mut record = String::new();
    let _ = writeln!(
        record,
        "onMetaData_body {}",
        hex(tag_body(&flv_bytes, meta_tag))
    );
    let _ = writeln!(record, "tag_headers {:?}", &headers[meta_tag..]);
    let _ = writeln!(
        record,
        "sequence_video_body {}",
        hex(tag_body(&flv_bytes, meta_tag + 1))
    );
    let _ = writeln!(
        record,
        "sequence_audio_body {}",
        hex(tag_body(&flv_bytes, meta_tag + 2))
    );
    let _ = writeln!(
        record,
        "first_media_body {}",
        hex(tag_body(&flv_bytes, meta_tag + 3))
    );

    let path = "golden/rtmp/push-av-flv.txt";
    let golden = test_fixtures::path(path);
    if std::env::var_os("REGENERATE_PUSH_GOLDENS").is_some() {
        std::fs::write(&golden, &record).expect("write golden");
        return;
    }
    let expected = test_fixtures::read_to_string(path);
    assert_eq!(
        record, expected,
        "the RTMP push wire bytes changed; if that is intended, regenerate \
         {path} with a reviewed diff"
    );
}
