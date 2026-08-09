//! Issue #942 acceptance: `crate::push::drive_push` negotiates a mixed
//! track set through a real RTMP server, carrying only what FLV can
//! actually hold.
//!
//! `multimux/tests/push_rtmp.rs` already proves `RtmpTransport::send_media`
//! produces byte-identical FLV over a real, independent RTMP server; that
//! test drives the transport directly, never through `drive_push`. This
//! file closes the gap issue #942 asked about: does the *production*
//! `drive_push` loop, going through `push::egress::PushTransportEgress`'s
//! `negotiate`, actually (a) carry the AVC+AAC tracks the fixture has and
//! (b) never even attempt to carry a track FLV cannot hold, rather than
//! either refusing the whole push or crashing trying to frame it?

use std::sync::Arc;
use std::time::Duration;

use broadcast_common::Unpackage;
use media_plane::trunk::{RetentionClass, Trunk, TrunkConfig};
use multimux::config::{PushFormat, ReconnectPolicy};
use multimux::push::{RtmpTransport, RtmpTransportConfig, drive_push};
use rtmp_runtime::server::{ServerEvent, ServerSession};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;
use tokio_util::sync::CancellationToken;
use transmux::FlvDemux;
use transmux::ir::TrackSpec;

const FLV: &[u8] = include_bytes!("../../fixtures/flv/av.flv");

/// HANG GUARD (workspace precedent, issue #826).
const GUARD: Duration = Duration::from_secs(20);

fn nz(n: usize) -> std::num::NonZeroUsize {
    std::num::NonZeroUsize::new(n).unwrap()
}

/// A `TrackSpec` no push output in this crate can carry (neither FLV's
/// AVC+AAC nor — deliberately, to make the point sharpest — anything TS
/// couldn't carry either would prove much; the point here is specifically
/// FLV's narrower restriction): opaque PES private data, exactly the
/// fallback `crate::push::mod`'s pre-#942 `media_from_samples` used to
/// synthesize for an unmatched track, now instead a genuinely *declared*
/// track this test asserts gets refused rather than silently mis-framed.
fn opaque_spec(track_id: u32) -> TrackSpec {
    TrackSpec::new(
        track_id,
        90_000,
        transmux::CodecConfig::Data {
            stream_type: 0x06,
            descriptors: Vec::new(),
            carriage: transmux::ir::DataCarriage::Pes,
        },
    )
}

#[tokio::test]
async fn drive_push_negotiates_and_carries_only_flv_compatible_tracks() {
    let mut demux = FlvDemux::new();
    let media = demux.unpackage(FLV).expect("demux av.flv fixture");
    assert_eq!(media.tracks.len(), 2, "fixture must carry AVC + AAC");

    // A real `Trunk`, exactly as a driver-backed ingest route mints one --
    // three tracks: the fixture's real AVC + AAC, plus one opaque track
    // `drive_push` must negotiate around, never attempt to frame.
    let trunk: Arc<Trunk> = Trunk::new(TrunkConfig::new(nz(256), nz(64), nz(8), nz(64), nz(64)));
    let opaque_track_id = 99;
    let mut specs: Vec<TrackSpec> = media.tracks.iter().map(|t| t.spec.clone()).collect();
    specs.push(opaque_spec(opaque_track_id));
    let writer = trunk
        .writer()
        .expect("trunk has a writer (nothing else holds it yet)");
    writer.set_tracks(specs);

    // The server side: a real, independent RTMP ingest implementation --
    // same shape as `push_rtmp.rs`'s own server task.
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
    let addr = listener.local_addr().expect("local addr");
    let server = tokio::spawn(async move {
        let (mut sock, _) = listener.accept().await.expect("accept");
        let mut session = ServerSession::with_defaults();
        let mut flv_bytes: Vec<u8> = Vec::new();
        let mut saw_publish = false;
        let mut buf = vec![0u8; 65536];
        loop {
            let n = match tokio::time::timeout(GUARD, sock.read(&mut buf)).await {
                Ok(Ok(0)) | Err(_) => break,
                Ok(Ok(n)) => n,
                Ok(Err(e)) => panic!("server read failed: {e}"),
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
            // Stop on a CONTENT condition, never a byte count. The publish
            // loop sends every AVC sample, then every AAC sample, then the
            // opaque one; `av.flv`'s video track alone exceeds any small
            // byte threshold, so `flv_bytes.len() > N` could fire after the
            // video tags and before the first audio tag -- leaving the
            // assertion below to recover one track and fail. That is exactly
            // what it did: ~1 run in 8 locally, and on both CI lanes as soon
            // as a job change let this test run at all.
            //
            // Waiting for both tracks keeps the opaque-track bite intact.
            // The opaque sample is published on every cycle immediately
            // after the real ones, so by the time audio has arrived
            // `drive_push` has already been offered it and either refused it
            // (2 tracks) or framed it (3, which the assertion catches). The
            // `GUARD` read timeout is the backstop if a regression means a
            // track never arrives at all.
            let carried = FlvDemux::new()
                .unpackage(&flv_bytes)
                .map(|m| m.tracks.len())
                .unwrap_or(0);
            if carried >= 2 && flv_bytes.len() > 4096 {
                break;
            }
        }
        (flv_bytes, saw_publish)
    });

    // The client side: `drive_push` itself -- the production entry point
    // `crate::origin::spawn_push_outputs` calls for a configured
    // `rtmp_push` output.
    let cfg = RtmpTransportConfig {
        app: "live".to_string(),
        stream_key: "test".to_string(),
    };
    let url = format!("rtmp://{addr}/live/test");
    let cancel = CancellationToken::new();
    let push_cancel = cancel.clone();
    let push_task = tokio::spawn(async move {
        drive_push::<RtmpTransport>(
            trunk,
            url,
            cfg,
            PushFormat::Ts,
            ReconnectPolicy {
                initial_backoff_ms: 100,
                max_backoff_ms: 1_000,
                max_attempts: None,
            },
            push_cancel,
        )
        .await
    });

    // Publish the fixture's real samples (and one opaque-track sample the
    // negotiated selection must exclude) on a loop -- `Trunk::subscribe`
    // starts from "now", not backlog, so `drive_push`'s cursor must already
    // be subscribed before a publish is visible to it; resending covers the
    // race the same way `dispatch_ingest.rs`'s own UDP-resend loop does.
    let opaque_sample = transmux::ir::Sample::new(
        bytes::Bytes::from_static(&[0xAA, 0xBB, 0xCC, 0xDD]),
        Some(0),
        Some(0),
        Some(3_000),
        true,
    );
    let publish_media = media.clone();
    let publish_task = tokio::spawn(async move {
        loop {
            for track in &publish_media.tracks {
                for sample in &track.samples {
                    writer.publish(track.spec.track_id, RetentionClass::Timed, sample.clone());
                }
            }
            writer.publish(
                opaque_track_id,
                RetentionClass::Timed,
                opaque_sample.clone(),
            );
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    });

    let (flv_bytes, saw_publish) = tokio::time::timeout(GUARD, server)
        .await
        .expect("server task must not hang")
        .expect("server task panicked");
    publish_task.abort();
    cancel.cancel();
    let _ = tokio::time::timeout(GUARD, push_task).await;

    assert!(saw_publish, "server must have accepted the publish");
    assert!(!flv_bytes.is_empty(), "server must have received media");

    // THE BITE: re-demux what the real server received. If `drive_push`
    // had tried to carry the opaque track through RTMP's FLV framing (the
    // pre-#942 shape had no structured way to refuse it at all), this
    // would either fail to demux or recover garbage frames; if `negotiate`
    // had refused the *whole* push over one non-carriable track (an
    // overly blunt fix), no media would ever have arrived at all.
    let mut demux2 = FlvDemux::new();
    let media2 = demux2
        .unpackage(&flv_bytes)
        .expect("re-demux the RTMP-received FLV tags");
    assert_eq!(
        media2.tracks.len(),
        2,
        "server must recover exactly AVC + AAC -- never a third, opaque track"
    );
    for track in &media2.tracks {
        assert!(
            media
                .tracks
                .iter()
                .any(|t| t.spec.track_id == track.spec.track_id),
            "every recovered track must be one of the fixture's real AVC/AAC tracks"
        );
        assert!(
            !track.samples.is_empty(),
            "each carried track must have real samples"
        );
    }
}
