//! RTSP push: every awaited IO is bounded (SP6.1, defect 4).
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use multimux::config::{PushFormat, ReconnectPolicy};
use multimux::push::{PushTransport, RtspTransport, RtspTransportConfig, drive_push};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};

/// Real-time guard: a bounded path returns long before this; an unbounded
/// one hangs and trips it.
const GUARD: Duration = Duration::from_secs(5);

/// Tight bounds so a bounded path returns in ~300 ms while an unbounded one
/// hits `GUARD`.
fn config_with_bounds() -> RtspTransportConfig {
    RtspTransportConfig::default().with_timeouts(
        rtsp_runtime::RtspTimeouts::default()
            .with_read_idle(Duration::from_millis(300))
            .with_write(Duration::from_millis(300)),
    )
}

/// How the scripted peer treats the connection after RECORD.
#[derive(Clone, Copy)]
enum AfterRecord {
    /// Hold the socket open and never read again (a live peer that stopped
    /// consuming).
    StopReading,
    /// Keep reading and discarding (a healthy peer).
    Drain,
}

/// Read one RTSP request (headers + `Content-Length` body) off `sock`;
/// returns `(method, cseq)`.
async fn read_request(sock: &mut TcpStream, buf: &mut Vec<u8>) -> Option<(String, String)> {
    loop {
        if let Some(end) = buf.windows(4).position(|w| w == b"\r\n\r\n") {
            let head = String::from_utf8_lossy(&buf[..end]).into_owned();
            let body_len = head
                .lines()
                .find_map(|l| {
                    l.to_ascii_lowercase()
                        .strip_prefix("content-length:")
                        .map(|v| v.trim().parse::<usize>().unwrap_or(0))
                })
                .unwrap_or(0);
            if buf.len() >= end + 4 + body_len {
                buf.drain(..end + 4 + body_len);
                let method = head.split_whitespace().next()?.to_string();
                let cseq = head
                    .lines()
                    .find_map(|l| l.strip_prefix("CSeq:").map(|v| v.trim().to_string()))?;
                return Some((method, cseq));
            }
        }
        let mut chunk = [0u8; 2048];
        let n = sock.read(&mut chunk).await.ok()?;
        if n == 0 {
            return None;
        }
        buf.extend_from_slice(&chunk[..n]);
    }
}

/// An RTSP record-mode peer: answers OPTIONS/ANNOUNCE/SETUP/RECORD with 200,
/// then behaves per `after`. `accepted` counts connections.
async fn scripted_record_peer(
    listener: TcpListener,
    accepted: Arc<AtomicUsize>,
    after: AfterRecord,
) {
    loop {
        let Ok((mut sock, _)) = listener.accept().await else {
            return;
        };
        accepted.fetch_add(1, Ordering::SeqCst);
        tokio::spawn(async move {
            let mut buf = Vec::new();
            loop {
                let Some((method, cseq)) = read_request(&mut sock, &mut buf).await else {
                    return;
                };
                let extra = if method == "SETUP" {
                    "Transport: RTP/AVP/TCP;unicast;interleaved=0-1;mode=record\r\nSession: 1\r\n"
                } else {
                    "Session: 1\r\n"
                };
                let resp = format!("RTSP/1.0 200 OK\r\nCSeq: {cseq}\r\n{extra}\r\n");
                if sock.write_all(resp.as_bytes()).await.is_err() {
                    return;
                }
                if method == "RECORD" {
                    break;
                }
            }
            match after {
                AfterRecord::StopReading => std::future::pending::<()>().await,
                AfterRecord::Drain => {
                    let mut sink = [0u8; 65536];
                    while matches!(sock.read(&mut sink).await, Ok(n) if n > 0) {}
                }
            }
        });
    }
}

async fn peer(after: AfterRecord) -> (String, Arc<AtomicUsize>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("rtsp://{}/live/key", listener.local_addr().unwrap());
    let accepted = Arc::new(AtomicUsize::new(0));
    tokio::spawn(scripted_record_peer(listener, Arc::clone(&accepted), after));
    (url, accepted)
}

/// Whole TS packets, a multiple of the 7-packet RTP payload.
const CHUNK: usize = 188 * 7 * 50;

/// CONTROL: the scripted peer is a faithful record-mode server — a healthy
/// (draining) peer accepts connect/setup/send.
#[tokio::test]
async fn control_a_healthy_peer_accepts_connect_setup_and_send() {
    let (url, _) = peer(AfterRecord::Drain).await;
    let mut t = tokio::time::timeout(GUARD, RtspTransport::connect(&url, &config_with_bounds()))
        .await
        .expect("connect must not hang")
        .expect("connect");
    tokio::time::timeout(GUARD, t.setup(&[]))
        .await
        .expect("setup must not hang")
        .expect("setup");
    tokio::time::timeout(GUARD, t.send(&vec![0u8; CHUNK]))
        .await
        .expect("send must not hang")
        .expect("send");
}

/// A peer that accepts TCP and never answers OPTIONS fails `connect` at the
/// response bound.
#[tokio::test]
async fn a_peer_that_never_answers_options_fails_connect_at_the_bound() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("rtsp://{}/live/key", listener.local_addr().unwrap());
    let _held = tokio::spawn(async move {
        let (sock, _) = listener.accept().await.unwrap();
        std::future::pending::<()>().await;
        drop(sock);
    });
    let r = tokio::time::timeout(GUARD, RtspTransport::connect(&url, &config_with_bounds()))
        .await
        .expect("connect must fail at its bound, not hang on a peer that never answers OPTIONS");
    assert!(r.is_err(), "no answer is a failure");
}

/// A peer that completes RECORD then stops reading fails `send` at the write
/// bound instead of blocking forever.
#[tokio::test]
async fn a_stalled_interleaved_write_fails_send_at_the_write_bound() {
    let (url, _) = peer(AfterRecord::StopReading).await;
    let mut t = RtspTransport::connect(&url, &config_with_bounds())
        .await
        .expect("connect");
    t.setup(&[]).await.expect("setup");
    let chunk = vec![0u8; CHUNK];
    let sent = tokio::time::timeout(GUARD, async {
        for _ in 0..4096 {
            // 4096 * CHUNK (~330 KB) = ~1.3 GB: far beyond any loopback buffer.
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
        sent,
        "a peer that stopped reading must eventually fail a send"
    );
}

fn avc_spec(track_id: u32) -> transmux::ir::TrackSpec {
    transmux::ir::TrackSpec::new(
        track_id,
        90_000,
        transmux::CodecConfig::Avc {
            config: transmux::AVCConfigurationBox::new(transmux::AVCDecoderConfigurationRecord {
                configuration_version: 1,
                profile_indication: 0x42,
                profile_compatibility: 0,
                level_indication: 0x1f,
                length_size_minus_one: 3,
                sps: Vec::new(),
                pps: Vec::new(),
                chroma_format: None,
                bit_depth_luma_minus8: None,
                bit_depth_chroma_minus8: None,
                sps_ext: Vec::new(),
            }),
            width: 0,
            height: 0,
        },
    )
}

fn big_sample() -> transmux::ir::Sample {
    // AVCC framing (`length_size_minus_one: 3`): a 4-byte big-endian length
    // prefix, then exactly that many bytes of NAL — one 256 KiB IDR slice.
    let nal_len = 256 * 1024;
    let mut data = Vec::with_capacity(4 + nal_len);
    data.extend_from_slice(&(nal_len as u32).to_be_bytes());
    data.push(0x65); // IDR-slice NAL header (ISO/IEC 14496-10 Table 7-1).
    data.resize(4 + nal_len, 0xAA);
    transmux::ir::Sample::new(
        bytes::Bytes::from(data),
        Some(0),
        Some(0),
        Some(3_000),
        true,
    )
}

/// END TO END through `drive_push`: a trunk WITH a video track and a steady
/// stream of large samples, pushed at a peer that stops reading after RECORD.
/// The transport's write bound (300 ms) must trip and `drive_push` must
/// reconnect long before `drive_push`'s own 10 s flush bound would.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn drive_push_reconnects_at_the_transport_write_bound_not_the_ten_second_flush_bound() {
    use media_plane::trunk::{RetentionClass, Trunk, TrunkConfig};
    let nz = |n| std::num::NonZeroUsize::new(n).unwrap();
    let trunk = Trunk::new(TrunkConfig::new(nz(64), nz(64), nz(64), nz(64), nz(64)));
    let writer = trunk.writer().expect("writer is free");
    writer.set_tracks(vec![avc_spec(1)]);

    let (url, accepted) = peer(AfterRecord::StopReading).await;
    let cancel = tokio_util::sync::CancellationToken::new();
    let task = tokio::spawn(drive_push::<RtspTransport>(
        Arc::clone(&trunk),
        url,
        config_with_bounds(),
        PushFormat::Ts,
        ReconnectPolicy {
            initial_backoff_ms: 0,
            max_backoff_ms: 0,
            max_attempts: None,
        },
        cancel.clone(),
    ));
    // Prime the trunk with enough large samples to overflow any loopback
    // socket buffer, then just wait for the reconnect (a condition wait, not
    // a fixed-interval ticker): once the socket is full the transport's own
    // write bound will trip without further publishing.
    for _ in 0..512 {
        writer.publish(1, RetentionClass::Timed, big_sample());
    }
    let reconnected = tokio::time::timeout(GUARD, async {
        while accepted.load(Ordering::SeqCst) < 2 {
            tokio::task::yield_now().await;
        }
    })
    .await;
    cancel.cancel();
    let _ = tokio::time::timeout(Duration::from_secs(2), task).await;
    assert!(
        reconnected.is_ok(),
        "the stalled write must be failed by the transport's own bound (reconnect within {GUARD:?}); \
         connections seen: {}",
        accepted.load(Ordering::SeqCst)
    );
}

/// A peer answering the record-mode handshake that then keeps reading and
/// reports (over `seen`) the method of every request it receives after RECORD —
/// so a test can prove the transport sent `TEARDOWN` on `close()`.
async fn teardown_recording_peer(
    listener: TcpListener,
    seen: tokio::sync::mpsc::UnboundedSender<String>,
) {
    let Ok((mut sock, _)) = listener.accept().await else {
        return;
    };
    let mut buf = Vec::new();
    loop {
        let Some((method, cseq)) = read_request(&mut sock, &mut buf).await else {
            return;
        };
        let extra = if method == "SETUP" {
            "Transport: RTP/AVP/TCP;unicast;interleaved=0-1;mode=record\r\nSession: 1\r\n"
        } else {
            "Session: 1\r\n"
        };
        let resp = format!("RTSP/1.0 200 OK\r\nCSeq: {cseq}\r\n{extra}\r\n");
        if sock.write_all(resp.as_bytes()).await.is_err() {
            return;
        }
        let _ = seen.send(method.clone());
        if method == "TEARDOWN" {
            return;
        }
        // No media is sent in this test, so just loop for the next request.
    }
}

/// I5: `close()` sends a bounded best-effort `TEARDOWN` (RFC 2326 §10.10), so
/// the server frees the publisher slot instead of waiting for its own timeout.
///
/// Revert-check: restore `fn close(&mut self) { self.client = None; }` and no
/// TEARDOWN is received — the `seen` assertion times out.
#[tokio::test]
async fn close_sends_a_teardown() {
    use tokio::sync::mpsc;
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("rtsp://{}/live/key", listener.local_addr().unwrap());
    let (tx, mut rx) = mpsc::unbounded_channel();
    tokio::spawn(teardown_recording_peer(listener, tx));

    let mut t = tokio::time::timeout(GUARD, RtspTransport::connect(&url, &config_with_bounds()))
        .await
        .expect("connect")
        .expect("connect");
    t.setup(&[]).await.expect("setup");
    t.close();
    // Drop so the transport's ownership of the teardown task ends cleanly.
    drop(t);

    let mut saw_teardown = false;
    let deadline = tokio::time::Instant::now() + GUARD;
    while tokio::time::Instant::now() < deadline {
        match tokio::time::timeout_at(deadline, rx.recv()).await {
            Ok(Some(m)) if m == "TEARDOWN" => {
                saw_teardown = true;
                break;
            }
            Ok(Some(_)) => {}
            Ok(None) | Err(_) => break,
        }
    }
    assert!(saw_teardown, "close() must send a TEARDOWN");
}

/// I-D(2): calling `close()` twice sends exactly ONE TEARDOWN (the second is a
/// no-op because the client is already taken).
#[tokio::test]
async fn close_twice_sends_exactly_one_teardown() {
    use tokio::sync::mpsc;
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("rtsp://{}/live/key", listener.local_addr().unwrap());
    let (tx, mut rx) = mpsc::unbounded_channel();
    tokio::spawn(teardown_recording_peer(listener, tx));

    let mut t = tokio::time::timeout(GUARD, RtspTransport::connect(&url, &config_with_bounds()))
        .await
        .expect("connect")
        .expect("connect");
    t.setup(&[]).await.expect("setup");
    t.close();
    t.close(); // no-op: exactly one TEARDOWN
    drop(t);

    let mut teardowns = 0usize;
    let deadline = tokio::time::Instant::now() + GUARD;
    loop {
        match tokio::time::timeout_at(deadline, rx.recv()).await {
            Ok(Some(m)) => {
                if m == "TEARDOWN" {
                    teardowns += 1;
                }
            }
            Ok(None) | Err(_) => break,
        }
    }
    assert_eq!(teardowns, 1, "close() twice must send exactly one TEARDOWN");
}
