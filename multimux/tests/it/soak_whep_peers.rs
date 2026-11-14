#![cfg(all(feature = "whep", feature = "test-hooks"))]
//! Load soak with real WHEP peers (#2565): `PEERS` viewers, each a genuine ICE + DTLS-SRTP peer
//! built on `webrtc_runtime::media::MediaTransport`, negotiate with one WHEP route over its
//! signalling endpoint and then receive `SOAK_SECONDS` of H.264 samples over SRTP through the
//! route's shared media socket.
//!
//! Gated by `RUST_BROADCAST_SOAK=1` (the nightly soak env); without it the test prints why it
//! skipped and passes. `SOAK_WHEP_PEERS=<n>` overrides the peer count.
//!
//! Measured ceiling (24-core Apple-silicon macOS laptop, `--profile ci`, 8 worker threads shared
//! by the route AND the in-process viewers, 30 frames per second of 1000-byte NALs, 20 s):
//! 200 peers: 100% delivery, p99 12 ms (debug profile). 500 peers: 100%, p99 277 ms with one
//! route socket. 1000 peers with one route socket: slowest viewer 54%, p99 7.9 s (the kernel
//! `sendto` on that one socket was the serial point, `sample` showed about two busy threads of
//! eight); with 8 route sockets per IP: 100%, p99 61 ms. 2000 peers saturate the shared worker
//! threads (the viewers' own decrypt included): about half the sessions fail.
//! Linux (aarch64 Docker container, 24 CPUs, 8 GB, rust:1.97, `--profile ci`, 8 sockets per IP): 1000 peers
//! connected in 221 ms, 0 failures, 100% delivery, latency p50 9 ms, p99 13 ms, 2010 alive tasks.
//! The default of 200
//! leaves a wide margin for a shared CI runner; `SOAK_WHEP_PEERS` raises it.
//!
//! Asserts zero failed sessions, that every viewer receives at least `MIN_DELIVERY_PERCENT` of
//! the frames published after the warm-up, a p99 publish-to-receive ceiling over all viewers and
//! a bound on the runtime's alive tasks (a few per viewer on each side, not a growing set).

use std::collections::BTreeSet;
use std::num::NonZeroUsize;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::time::{Duration, Instant};

use media_plane::trunk::TrunkConfig;
use media_plane::{RetentionClass, Trunk};
use multimux::output::whep::{WhepProbes, run_whep_on_probed};
use rtc_dtls::crypto::Certificate;
use rtc_dtls::crypto_provider::default_provider;
use tokio::net::UdpSocket;
use tokio_util::sync::CancellationToken;
use tokio_util::task::TaskTracker;
use transmux::pipeline::{CodecConfig, Sample, TrackSpec};
use transmux::{TrackId, avc_config_from_sprop};
use webrtc_runtime::media::{
    MediaEvent, MediaTransport, MediaTransportConfig, SetupRole, certificate_fingerprint,
};

use crate::support::raw_http::http;

const SPROP: &str = "Z0IAKeKQFAe2AtwEBAaQeJEV,aM48gA==";
const DEFAULT_PEERS: usize = 200;
const SOAK_SECONDS: u64 = 20;
const FPS: u64 = 30;
const FRAME_INTERVAL: Duration = Duration::from_millis(1000 / FPS);
const WARMUP_FRAMES: u32 = 30;
const NAL_BYTES: usize = 1000;
const NAL_TYPE_IDR: u8 = 0x65;
const TIMESTAMP_AT: usize = 1;
const FRAME_NUMBER_AT: usize = 9;
const SETUP_CONCURRENCY: usize = 50;
const CONNECT_BUDGET: Duration = Duration::from_secs(60);
const DRAIN: Duration = Duration::from_millis(500);
const TRUNK_TIMED_RING: usize = 1024;
const NANOS_PER_MILLI: u64 = 1_000_000;

const MIN_DELIVERY_PERCENT: u64 = 95;
const P99_CEILING: Duration = Duration::from_millis(1000);
const MAX_TASKS_PER_PEER: usize = 8;

fn percentile(sorted: &[u64], p: f64) -> u64 {
    if sorted.is_empty() {
        return 0;
    }
    sorted[((sorted.len() as f64 - 1.0) * p).round() as usize]
}

struct Answer {
    ufrag: String,
    pwd: String,
    fingerprint: String,
    candidate: String,
    setup: String,
}

fn attribute(sdp: &str, name: &str) -> String {
    sdp.lines()
        .find_map(|l| l.strip_prefix(name))
        .unwrap_or_else(|| panic!("answer has no {name}: {sdp}"))
        .trim_end_matches('\r')
        .to_string()
}

fn parse_answer(sdp: &str) -> Answer {
    let candidate = attribute(sdp, "a=candidate:");
    Answer {
        ufrag: attribute(sdp, "a=ice-ufrag:"),
        pwd: attribute(sdp, "a=ice-pwd:"),
        fingerprint: attribute(sdp, "a=fingerprint:"),
        candidate,
        setup: attribute(sdp, "a=setup:"),
    }
}

#[derive(Default)]
struct ViewerResult {
    connected: bool,
    error: Option<String>,
    frames: BTreeSet<u32>,
    latencies_ns: Vec<u64>,
}

struct Shared {
    base: Instant,
    connected: AtomicUsize,
    stop: AtomicBool,
}

async fn viewer(
    addr: std::net::SocketAddr,
    index: usize,
    shared: Arc<Shared>,
    setup_slots: Arc<tokio::sync::Semaphore>,
) -> ViewerResult {
    let mut result = ViewerResult::default();
    let slot = setup_slots.acquire_owned().await.expect("semaphore");
    let provider = default_provider().expect("crypto provider");
    let cert = Certificate::generate_self_signed(vec!["localhost".to_string()], provider.crypto())
        .expect("client certificate");
    let fingerprint = certificate_fingerprint(&cert);
    let socket = UdpSocket::bind("127.0.0.1:0").await.expect("client socket");
    let local_addr = socket.local_addr().expect("client address");
    let ufrag = format!("v{index:07}");
    let pwd = format!("pwd{index:07}0123456789abcdef");
    let offer = format!(
        "v=0\r\no=- 0 0 IN IP4 127.0.0.1\r\ns=-\r\nt=0 0\r\n\
         m=video 9 UDP/TLS/RTP/SAVPF 96\r\nc=IN IP4 0.0.0.0\r\n\
         a=ice-ufrag:{ufrag}\r\na=ice-pwd:{pwd}\r\na=fingerprint:sha-256 {fingerprint}\r\n\
         a=setup:actpass\r\na=mid:0\r\na=rtcp-mux\r\na=recvonly\r\na=rtpmap:96 H264/90000\r\n\
         a=fmtp:96 level-asymmetry-allowed=1;packetization-mode=1;profile-level-id=42e01f\r\n\
         a=candidate:1 1 udp 2130706431 127.0.0.1 {} typ host\r\n",
        local_addr.port()
    );
    let response = http(
        addr,
        "POST",
        "/whep",
        &[("Content-Type", "application/sdp")],
        &offer,
    )
    .await;
    if response.status != 201 {
        result.error = Some(format!("signalling {}: {}", response.status, response.body));
        return result;
    }
    let answer = parse_answer(&response.body);
    assert_eq!(
        answer.setup, "passive",
        "the route is the DTLS server for an actpass offer"
    );
    let mut media = match MediaTransport::with_certificate_for_test(
        MediaTransportConfig::new(
            local_addr,
            ufrag,
            pwd,
            answer.ufrag,
            answer.pwd,
            SetupRole::Active,
            answer.fingerprint,
        )
        .with_is_controlling(true),
        cert,
        Instant::now(),
    ) {
        Ok(media) => media,
        Err(e) => {
            result.error = Some(format!("transport: {e}"));
            return result;
        }
    };
    if let Err(e) = media.add_remote_candidate(&answer.candidate) {
        result.error = Some(format!("candidate: {e}"));
        return result;
    }
    let mut buf = vec![0u8; 2048];
    let mut slot = Some(slot);
    let handshake_deadline = tokio::time::Instant::now() + CONNECT_BUDGET;
    while !shared.stop.load(Ordering::Relaxed) {
        while let Some(datagram) = media.poll_transmit() {
            if let Err(e) = socket.send_to(&datagram.bytes, datagram.peer).await {
                result.error = Some(format!("send: {e}"));
                return result;
            }
        }
        let wake = media
            .poll_timeout()
            .map_or_else(
                || tokio::time::Instant::now() + Duration::from_millis(50),
                tokio::time::Instant::from_std,
            )
            .min(tokio::time::Instant::now() + Duration::from_millis(100));
        let events = tokio::select! {
            received = socket.recv_from(&mut buf) => match received {
                Ok((n, peer)) => match media.handle_datagram(Instant::now(), peer, &buf[..n]) {
                    Ok(events) => events,
                    Err(e) => {
                        result.error = Some(format!("datagram: {e}"));
                        return result;
                    }
                },
                Err(e) => {
                    result.error = Some(format!("recv: {e}"));
                    return result;
                }
            },
            () = tokio::time::sleep_until(wake) => media.handle_timeout(Instant::now()),
        };
        for event in events {
            match event {
                MediaEvent::DtlsHandshakeComplete => {
                    result.connected = true;
                    shared.connected.fetch_add(1, Ordering::SeqCst);
                    slot = None;
                }
                MediaEvent::Rtp(rtp) => {
                    let received = shared.base.elapsed().as_nanos() as u64;
                    if rtp.payload.len() > FRAME_NUMBER_AT + 4 {
                        let sent = u64::from_be_bytes(
                            rtp.payload[TIMESTAMP_AT..TIMESTAMP_AT + 8]
                                .try_into()
                                .expect("timestamp bytes"),
                        );
                        let frame = u32::from_be_bytes(
                            rtp.payload[FRAME_NUMBER_AT..FRAME_NUMBER_AT + 4]
                                .try_into()
                                .expect("frame bytes"),
                        );
                        if result.frames.insert(frame) {
                            result.latencies_ns.push(received.saturating_sub(sent));
                        }
                    }
                }
                MediaEvent::TimerError(e) | MediaEvent::AuthFailure { reason: e, .. } => {
                    result.error = Some(e.to_string());
                    return result;
                }
                MediaEvent::DtlsClosed => {
                    result.error = Some("dtls closed by the route".into());
                    return result;
                }
                _ => {}
            }
        }
        if !result.connected && tokio::time::Instant::now() >= handshake_deadline {
            result.error = Some("handshake did not complete in time".into());
            return result;
        }
    }
    drop(slot);
    result
}

fn frame_sample(frame: u32, sent_ns: u64) -> Sample {
    let mut nal = vec![0u8; NAL_BYTES];
    nal[0] = NAL_TYPE_IDR;
    nal[TIMESTAMP_AT..TIMESTAMP_AT + 8].copy_from_slice(&sent_ns.to_be_bytes());
    nal[FRAME_NUMBER_AT..FRAME_NUMBER_AT + 4].copy_from_slice(&frame.to_be_bytes());
    let mut data = (nal.len() as u32).to_be_bytes().to_vec();
    data.extend_from_slice(&nal);
    let pts = i64::from(frame) * i64::from(transmux::VIDEO_CLOCK_RATE) / FPS as i64;
    Sample::new(data, Some(pts), Some(pts), None, true)
}

#[tokio::test(flavor = "multi_thread", worker_threads = 8)]
async fn soak_real_whep_peers_receive_the_route() {
    if std::env::var_os("RUST_BROADCAST_SOAK").is_none() {
        eprintln!("SKIPPED soak_whep_peers: set RUST_BROADCAST_SOAK=1 to run the load soak");
        return;
    }
    let peers: usize = std::env::var("SOAK_WHEP_PEERS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(DEFAULT_PEERS);

    let ring = |n: usize| NonZeroUsize::new(n).expect("non-zero ring");
    let trunk = Trunk::new(TrunkConfig::default().with_timed_capacity(ring(TRUNK_TIMED_RING)));
    let writer = trunk.writer().expect("trunk writer");
    writer.set_tracks(vec![TrackSpec::new(
        TrackId::new(1),
        transmux::VIDEO_CLOCK_RATE,
        CodecConfig::Avc {
            config: avc_config_from_sprop(SPROP).expect("valid sprop"),
            width: 0,
            height: 0,
        },
    )]);

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind signalling");
    let addr = listener.local_addr().expect("signalling address");
    let cancel = CancellationToken::new();
    let probes = WhepProbes::default();
    let route = {
        let (trunk, cancel, probes) = (Arc::clone(&trunk), cancel.clone(), probes.clone());
        tokio::spawn(async move {
            run_whep_on_probed(
                listener,
                trunk,
                cancel,
                None,
                peers + 1,
                TaskTracker::new(),
                probes,
            )
            .await;
        })
    };

    let shared = Arc::new(Shared {
        base: Instant::now(),
        connected: AtomicUsize::new(0),
        stop: AtomicBool::new(false),
    });
    let setup_slots = Arc::new(tokio::sync::Semaphore::new(SETUP_CONCURRENCY));
    let mut viewers = tokio::task::JoinSet::new();
    let connect_started = Instant::now();
    for index in 0..peers {
        viewers.spawn(viewer(
            addr,
            index,
            Arc::clone(&shared),
            Arc::clone(&setup_slots),
        ));
    }
    let connect_deadline = tokio::time::Instant::now() + CONNECT_BUDGET;
    while shared.connected.load(Ordering::SeqCst) < peers
        && tokio::time::Instant::now() < connect_deadline
        && viewers.len() == peers
    {
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    let connect_time = connect_started.elapsed();
    let connected = shared.connected.load(Ordering::SeqCst);

    let metrics = tokio::runtime::Handle::current().metrics();
    let mut max_tasks = 0usize;
    let total_frames = WARMUP_FRAMES + (SOAK_SECONDS * FPS) as u32;
    let mut ticker = tokio::time::interval(FRAME_INTERVAL);
    for frame in 0..total_frames {
        ticker.tick().await;
        let sent_ns = shared.base.elapsed().as_nanos() as u64;
        writer.publish(
            TrackId::new(1),
            RetentionClass::Timed,
            frame_sample(frame, sent_ns),
        );
        if frame % FPS as u32 == 0 {
            max_tasks = max_tasks.max(metrics.num_alive_tasks());
        }
    }
    tokio::time::sleep(DRAIN).await;
    shared.stop.store(true, Ordering::Relaxed);
    drop(writer);

    let mut failures = Vec::new();
    let mut latencies = Vec::new();
    let mut delivery_percent = Vec::new();
    let counted = u64::from(total_frames - WARMUP_FRAMES);
    while let Some(done) = viewers.join_next().await {
        let result = done.expect("viewer task");
        if let Some(error) = result.error {
            failures.push(error);
            continue;
        }
        if !result.connected {
            failures.push("never connected".into());
            continue;
        }
        let received: u64 = result
            .frames
            .iter()
            .filter(|f| **f >= WARMUP_FRAMES)
            .count() as u64;
        delivery_percent.push(received * 100 / counted);
        latencies.extend(
            result
                .frames
                .iter()
                .zip(&result.latencies_ns)
                .filter(|(f, _)| **f >= WARMUP_FRAMES)
                .map(|(_, l)| *l),
        );
    }
    cancel.cancel();
    let _ = route.await;
    latencies.sort_unstable();
    delivery_percent.sort_unstable();
    let p99 = Duration::from_nanos(percentile(&latencies, 0.99));
    let min_delivery = delivery_percent.first().copied().unwrap_or(0);
    eprintln!(
        "soak_whep_peers: peers {peers}, connected {connected} in {connect_time:?}, failures {}, \
         min delivery {min_delivery}% (median {}%), latency p50 {} ms p99 {} ms, \
         max alive tasks {max_tasks}",
        failures.len(),
        delivery_percent
            .get(delivery_percent.len() / 2)
            .copied()
            .unwrap_or(0),
        percentile(&latencies, 0.5) / NANOS_PER_MILLI,
        p99.as_millis(),
    );

    assert!(
        failures.is_empty(),
        "{} of {peers} viewer sessions failed, first: {:?}",
        failures.len(),
        failures.first()
    );
    assert!(
        min_delivery >= MIN_DELIVERY_PERCENT,
        "the slowest viewer received {min_delivery}% of the frames"
    );
    assert!(p99 <= P99_CEILING, "publish-to-receive p99 {p99:?}");
    assert!(
        max_tasks <= peers * MAX_TASKS_PER_PEER,
        "{max_tasks} alive tasks for {peers} viewers"
    );
}
