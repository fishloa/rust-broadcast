//! The network TS ingest PES ceiling is a `Config` field, not a library
//! constant (issue #4146): a route built with a ceiling below one access
//! unit's PES drops that unit (the demux's `BudgetExceeded` discontinuity) and
//! keeps delivering the rest, while the same input with `None` passes every
//! unit through whole.
//!
//! The loopback harness is the one `multimux/src/source/ts_udp.rs`'s
//! `loopback_udp_samples_land_in_trunk` uses: the caller binds the UDP socket
//! and hands it to `TsUdpRoute::with_socket`, a sender flushes real TS bytes at
//! it, and `run_ts_udp` drives the route into a `RouteHandle` the test reads
//! back through the published `Trunk`. The socket comes from
//! `crate::support::udp_origin::bind_udp`; the stream is muxed by
//! `transmux::TsMuxStage` with `media_plane::ts_session::test_support`'s track
//! spec, and carries one access unit far above the test's ceiling.

use std::num::NonZeroUsize;
use std::sync::Arc;
use std::time::Duration;

use broadcast_common::Stage;
use broadcast_common::Timestamp;
use media_plane::trunk::SampleCursorItem;
use media_plane::ts_session::test_support::{handshake, track_spec, trunk_config};
use multimux::route::{RouteConfig, RouteHandle};
use multimux::source::ts_udp::{TsUdpRoute, run_ts_udp};
use tokio_util::sync::CancellationToken;
use transmux::mux_stage::MuxInput;
use transmux::pipeline::Sample;
use transmux::{TrackId, TsMuxStage};

const TRUNK_WAIT: Duration = Duration::from_secs(10);
const FRAME_RATE_FPS: u32 = 30;
const FRAME_COUNT: u32 = 30;
const LARGE_SAMPLE_INDEX: u32 = 10;
const LARGE_SAMPLE_BYTES: usize = 4096;
const SMALL_SAMPLE_BYTES: usize = 16;
const CEILING_BELOW_THE_LARGE_SAMPLE: usize = 1024;
const NAL_LENGTH_PREFIX_BYTES: usize = 4;

fn muxed_ts_with_one_large_access_unit() -> Vec<u8> {
    let spec = track_spec(TrackId::new(1));
    let frame_dur = u32::try_from(broadcast_common::clock33::PTS_HZ / u64::from(FRAME_RATE_FPS))
        .expect("a whole-frame duration fits a u32");
    let mut stage = TsMuxStage::new(Some(transmux::DEFAULT_PSI_INTERVAL));
    stage
        .feed(MuxInput::Track(Box::new(spec.clone())), Timestamp::ZERO)
        .expect("announce the track");
    for i in 0..FRAME_COUNT {
        let len = if i == LARGE_SAMPLE_INDEX {
            LARGE_SAMPLE_BYTES
        } else {
            SMALL_SAMPLE_BYTES
        };
        let mut payload = vec![0xAA; len - NAL_LENGTH_PREFIX_BYTES];
        if i == 0 {
            payload[0] = 0x65;
        }
        let mut data = ((len - NAL_LENGTH_PREFIX_BYTES) as u32)
            .to_be_bytes()
            .to_vec();
        data.extend(payload);
        let dts = i64::from(i) * i64::from(frame_dur);
        let sample =
            Sample::new(data, Some(dts), Some(dts), Some(frame_dur), i == 0).expect("valid timing");
        stage
            .feed(
                MuxInput::Sample {
                    track: spec.track_id,
                    sample,
                },
                Timestamp::ZERO,
            )
            .expect("mux the sample");
    }
    stage.finish().expect("finish the muxer");
    let mut bytes = Vec::new();
    while let Some(output) = stage.poll() {
        if let transmux::mux_stage::MuxOutput::Bytes(chunk) = output {
            bytes.extend_from_slice(&chunk);
        }
    }
    bytes
}

fn route_handle() -> Arc<RouteHandle> {
    Arc::new(RouteHandle::new(
        RouteConfig::default()
            .with_target_duration(Duration::from_secs(4))
            .with_part_target(Duration::from_millis(500))
            .with_window_segments(4),
    ))
}

/// Drives one `TsUdpRoute` over a loopback UDP socket and returns the length of
/// every timed sample that reached the route's published `Trunk`.
async fn timed_sample_lengths(max_pes_unit_bytes: Option<NonZeroUsize>) -> Vec<usize> {
    let (addr, socket) = crate::support::udp_origin::bind_udp().await;
    let route =
        TsUdpRoute::with_socket("pes-ceiling", socket).with_max_pes_unit_bytes(max_pes_unit_bytes);
    let handle = route_handle();
    let cancel = CancellationToken::new();
    let run = tokio::spawn({
        let handle = Arc::clone(&handle);
        let cancel = cancel.clone();
        async move { run_ts_udp(&route, trunk_config(), handshake(), &handle, cancel).await }
    });
    let ts_bytes = muxed_ts_with_one_large_access_unit();
    let sender = tokio::net::UdpSocket::bind("127.0.0.1:0")
        .await
        .expect("bind sender");
    let send_task = tokio::spawn(async move {
        for _ in 0..200 {
            for chunk in ts_bytes.chunks(7 * 188) {
                sender.send_to(chunk, addr).await.expect("send TS datagram");
            }
            tokio::task::yield_now().await;
        }
    });
    let mut lengths = Vec::new();
    if let Ok(trunk) = tokio::time::timeout(TRUNK_WAIT, handle.await_first_trunk()).await {
        let mut cursor = trunk.subscribe_from_backlog();
        loop {
            match cursor.poll() {
                Some(SampleCursorItem::Timed { sample, .. }) => lengths.push(sample.data.len()),
                Some(_) => {}
                None if send_task.is_finished() => break,
                None => tokio::task::yield_now().await,
            }
        }
    }
    cancel.cancel();
    send_task.abort();
    let _ = tokio::time::timeout(TRUNK_WAIT, run).await;
    lengths
}

/// A ceiling below the one large access unit's PES: that unit is dropped, the
/// smaller ones still reach the trunk, so the route keeps working.
#[tokio::test]
async fn a_ceiling_below_the_large_pes_drops_that_unit_only() {
    let cap = NonZeroUsize::new(CEILING_BELOW_THE_LARGE_SAMPLE).expect("non-zero ceiling");
    let lengths = timed_sample_lengths(Some(cap)).await;
    assert!(!lengths.is_empty(), "the smaller access units must pass");
    assert!(
        lengths
            .iter()
            .all(|&len| len <= CEILING_BELOW_THE_LARGE_SAMPLE),
        "no access unit above the ceiling may reach the trunk: {lengths:?}"
    );
}

/// The same bytes with `None` (the demux's unlimited default) pass through
/// whole, the large access unit included.
#[tokio::test]
async fn no_ceiling_passes_the_same_input_whole() {
    let lengths = timed_sample_lengths(None).await;
    assert!(!lengths.is_empty(), "the stream must reach the trunk");
    assert!(
        lengths.iter().any(|&len| len >= LARGE_SAMPLE_BYTES),
        "the large access unit must arrive whole with no ceiling: {lengths:?}"
    );
}

/// The configured ceiling reaches the route through `Config`: an origin built
/// with a one-byte ceiling never announces a program (every access unit
/// overflows and is dropped), so its media playlist is never served, while the
/// same origin and fixture with no ceiling does serve `#EXTINF:` segments.
///
/// The loopback harness is `crate::support::udp_origin::spawn_udp_ts_origin`
/// (a real UDP route fed the committed `ts/h264-aac.ts` capture) and the route
/// config `dispatch_ingest`'s own end-to-end tests use.
#[cfg(feature = "test-seams")]
#[tokio::test]
async fn the_config_ceiling_reaches_the_ts_udp_route() {
    use crate::support::shared_it::{fixture_path, nz, poll_until_extinf};
    use crate::support::udp_origin::{UdpTsOrigin, spawn_udp_ts_origin};
    use std::num::NonZeroUsize;
    use std::sync::atomic::Ordering;

    const OBSERVATION: Duration = Duration::from_secs(3);

    async fn origin_with(ceiling: Option<NonZeroUsize>) -> (UdpTsOrigin, String) {
        let origin = spawn_udp_ts_origin(
            |bind, input| {
                crate::dispatch_ingest::base_config(bind, input)
                    .with_max_ingest_pes_unit_bytes(ceiling)
                    .with_hold_cold_playlist(false)
            },
            &fixture_path("ts/h264-aac.ts"),
            Duration::from_millis(5),
        )
        .await;
        let playlist_url = format!("http://{}/cam/media.m3u8", origin.bind_addr);
        (origin, playlist_url)
    }

    fn stop(origin: UdpTsOrigin) {
        origin.stop.store(true, Ordering::Relaxed);
        origin.sender.abort();
        origin.server.abort();
    }

    let (bounded, bounded_url) = origin_with(Some(nz(1))).await;
    let client = crate::support::http::client();
    let deadline = tokio::time::Instant::now() + OBSERVATION;
    let mut tick = tokio::time::interval(Duration::from_millis(50));
    let mut answered = false;
    while tokio::time::Instant::now() < deadline {
        tick.tick().await;
        if let Ok(resp) = client.get(&bounded_url).send().await {
            answered = true;
            if let Ok(body) = resp.text().await {
                assert!(
                    !body.contains("#EXTINF:"),
                    "a one-byte PES ceiling must drop every access unit, yet a segment closed: {body}"
                );
            }
        }
    }
    stop(bounded);
    assert!(answered, "the origin must answer the playlist request");

    let (unlimited, unlimited_url) = origin_with(None).await;
    let playlist = poll_until_extinf(&client, &unlimited_url).await;
    stop(unlimited);
    assert!(
        playlist.contains("#EXTINF:"),
        "with no ceiling the same route must close and serve a segment"
    );
}
