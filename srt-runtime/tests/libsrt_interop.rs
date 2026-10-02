//! Real `libsrt` interop over loopback UDP, both directions (libsrt interop).
//!
//! `tests/libsrt_fixtures.rs` already proves real libsrt KEEPALIVE/ACKACK
//! bytes parse correctly (issue #1060). This file proves the *handshake and
//! data path* actually completes end-to-end against a genuine libsrt peer, and
//! that the connection-lifetime behaviour (idle survival, SHUTDOWN in both
//! directions, a lost CONCLUSION response, a refused connect) matches what
//! libsrt does — the independent oracle for the adapter, since every in-crate
//! test of it is self-vs-self (issue #1131). The §6 key exchange has its own
//! oracle file, `tests/libsrt_crypto_interop.rs`.
//!
//! Skips itself LOUDLY (prints why, `--nocapture`) when `srt-live-transmit` is
//! not on `PATH`; set `SRT_REQUIRE_LIBSRT=1` to make the skip a failure.
//!
//! No test sleeps to wait for a process to come up: the Caller's handshake
//! retransmits every 250 ms until libsrt answers, a libsrt Caller retries by
//! itself, and the UDP source libsrt reads is bound before it connects, so
//! feeding it after the connection is accepted cannot race. The one deliberate
//! wait is the idle window, whose elapsed wall-clock time *is* the thing under
//! test.

#![cfg(feature = "tokio")]

use std::io::ErrorKind;
use std::net::{SocketAddr, UdpSocket};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use srt_runtime::Error;
use srt_runtime::handshake_sm::{HandshakeConfig, RejectionReason};
use srt_runtime::io::{SrtListener, SrtSocket};
use srt_runtime::packet::{ControlPacket, HandshakeType, SrtPacket};
use tokio::net::UdpSocket as TokioUdpSocket;

include!("support/skip.rs");

const TEST_TIMEOUT: Duration = Duration::from_secs(60);
const PAYLOAD_LEN: usize = 1316;
/// How long the idle window between the two bursts lasts: several libsrt
/// Keep-Alive periods (§3.2.3, one second each), during which real
/// Keep-Alive/ACK traffic crosses the wire in both directions.
const IDLE_WINDOW: Duration = Duration::from_secs(4);

fn free_udp_port() -> u16 {
    UdpSocket::bind("127.0.0.1:0")
        .expect("bind free port")
        .local_addr()
        .expect("local addr")
        .port()
}

fn pattern(n: usize, seed: u8) -> Vec<u8> {
    (0..n).map(|i| seed.wrapping_add(i as u8)).collect()
}

/// Kills the wrapped child on drop (panic, early `?`, or a `tokio::time::timeout`
/// dropping the future all count) — a plain `.kill()` at the end of the async
/// block is skipped by any of those paths, leaking a real `srt-live-transmit`
/// process that then spins retrying forever.
struct KillOnDrop(Child);

impl Drop for KillOnDrop {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

fn spawn(cmd: &mut Command) -> KillOnDrop {
    KillOnDrop(
        cmd.stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .expect("spawn srt-live-transmit"),
    )
}

/// A libsrt listener forwarding to a UDP discard socket this test reads.
fn spawn_libsrt_listener(
    listen_port: u16,
    discard_port: u16,
    options: &str,
    extra_args: &[&str],
) -> KillOnDrop {
    let mut cmd = Command::new("srt-live-transmit");
    cmd.arg("-loglevel:error");
    cmd.args(extra_args);
    cmd.arg(format!("srt://:{listen_port}?{options}"));
    cmd.arg(format!("udp://127.0.0.1:{discard_port}"));
    spawn(&mut cmd)
}

async fn recv_at_least(discard: &TokioUdpSocket, want_len: usize, timeout: Duration) -> Vec<u8> {
    let mut out = Vec::new();
    let mut buf = vec![0u8; 65536];
    let deadline = tokio::time::Instant::now() + timeout;
    while out.len() < want_len {
        let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
        if remaining.is_zero() {
            break;
        }
        match tokio::time::timeout(remaining, discard.recv(&mut buf)).await {
            Ok(Ok(n)) => out.extend_from_slice(&buf[..n]),
            _ => break,
        }
    }
    out
}

async fn send_burst(socket: &mut SrtSocket, burst: &[u8]) {
    for chunk in burst.chunks(PAYLOAD_LEN) {
        socket.send(chunk).await.expect("send burst chunk");
    }
}

/// Numbered, self-describing chunk `index` of `PAYLOAD_LEN` bytes.
fn numbered_chunk(index: u32) -> Vec<u8> {
    let mut c = pattern(PAYLOAD_LEN, 7);
    c[..4].copy_from_slice(&index.to_be_bytes());
    c
}

/// Feed numbered chunks to a libsrt `udp://` source every 50 ms until `wanted`
/// of them have come out of `receiver`, and return the indexes received.
///
/// `srt-live-transmit` discards what its `udp://` source delivers while its
/// SRT target is still connecting, and there is no signal for "connected" on
/// this side. So the feed repeats: the chunks sent while libsrt was still
/// connecting vanish, everything after must arrive uncorrupted, complete and
/// in order (checked by the caller via the returned indexes).
async fn feed_until_received(src_port: u16, receiver: &mut SrtSocket, wanted: usize) -> Vec<u32> {
    let feed = TokioUdpSocket::bind("127.0.0.1:0").await.unwrap();
    let mut next_to_feed = 0u32;
    let mut received: Vec<u32> = Vec::new();
    let mut feeder = tokio::time::interval(Duration::from_millis(50));
    let deadline = tokio::time::Instant::now() + Duration::from_secs(20);
    while received.len() < wanted {
        tokio::select! {
            _ = feeder.tick() => {
                feed.send_to(&numbered_chunk(next_to_feed), ("127.0.0.1", src_port))
                    .await
                    .unwrap();
                next_to_feed += 1;
            }
            r = receiver.recv() => {
                let payload = r.expect("recv").expect("connection ended");
                assert_eq!(payload.len(), PAYLOAD_LEN);
                let index = u32::from_be_bytes(payload[..4].try_into().unwrap());
                assert_eq!(payload, numbered_chunk(index), "chunk {index} corrupted");
                received.push(index);
            }
            _ = tokio::time::sleep_until(deadline) => {
                panic!(
                    "no data: the libsrt caller never delivered \
                     (got {received:?} after feeding {next_to_feed} chunks)"
                );
            }
        }
    }
    assert!(
        received.windows(2).all(|w| w[1] == w[0] + 1),
        "chunks must arrive in order without gaps, got {received:?}"
    );
    received
}

/// Our [`SrtSocket`] (caller) connects to a genuine `srt-live-transmit`
/// listener, sends a burst, stays idle for several Keep-Alive periods, then
/// sends a second burst; both must arrive byte-identical through libsrt's UDP
/// target. (libsrt gives up on a silent peer only after its exponential
/// expiry timer has counted sixteen expirations — tens of seconds — so this
/// checks that an idle connection stays usable, not the 5 s timeout itself;
/// that, and the Keep-Alive timer, are covered by the adapter's own
/// paused-clock unit tests.) Pre-fix of the original
/// interop bug the handshake never got past the caller's CONCLUSION, which a
/// real Listener silently dropped.
#[tokio::test]
async fn our_caller_completes_real_handshake_and_data_with_libsrt_listener() {
    skip_unless_tools!("srt-live-transmit" => "-version");

    tokio::time::timeout(TEST_TIMEOUT, async {
        let listen_port = free_udp_port();
        let discard = TokioUdpSocket::bind("127.0.0.1:0")
            .await
            .expect("bind discard socket");
        let discard_port = discard.local_addr().unwrap().port();
        let _listener_proc = spawn_libsrt_listener(listen_port, discard_port, "latency=120", &[]);

        // No wait for the process to bind: the handshake retransmits every
        // 250 ms until libsrt answers.
        let mut caller = tokio::time::timeout(
            Duration::from_secs(10),
            SrtSocket::connect(
                format!("127.0.0.1:{listen_port}")
                    .parse::<SocketAddr>()
                    .unwrap(),
                HandshakeConfig::default(),
            ),
        )
        .await
        .expect("caller connect timed out — handshake never completed")
        .expect("caller connect to real libsrt listener");

        let first_burst = pattern(PAYLOAD_LEN * 10, 0);
        send_burst(&mut caller, &first_burst).await;
        let received_first =
            recv_at_least(&discard, first_burst.len(), Duration::from_secs(10)).await;
        assert_eq!(
            received_first, first_burst,
            "first burst must be forwarded byte-identical by the real libsrt listener"
        );

        // The deliberate wait of this test: real elapsed time with no
        // application data in either direction.
        tokio::time::sleep(IDLE_WINDOW).await;

        let second_burst = pattern(PAYLOAD_LEN * 5, 100);
        send_burst(&mut caller, &second_burst).await;
        let received_second =
            recv_at_least(&discard, second_burst.len(), Duration::from_secs(10)).await;
        assert_eq!(
            received_second, second_burst,
            "second burst (after the idle window) must still arrive byte-identical"
        );

        eprintln!("PASS: our_caller_completes_real_handshake_and_data_with_libsrt_listener");
        drop(caller);
    })
    .await
    .expect("test timed out — real libsrt listener/session stalled");
}

/// A genuine `srt-live-transmit` caller connects to our [`SrtListener`],
/// completing the real HSv5 handshake and delivering a real DATA payload
/// through the `ingress()` path.
///
/// The source is `udp://:PORT`, not `file://con` (stdin): independently
/// reproduced against two genuine `srt-live-transmit` processes with no code
/// from this crate involved at all, `file://con` forwards zero bytes (a real
/// libsrt/macOS `ConsoleSource` defect, see `tests/libsrt_fixtures.rs`'s
/// module doc), while plain `udp://` sourcing forwards the full burst.
#[tokio::test]
async fn real_libsrt_caller_completes_handshake_and_data_with_our_listener() {
    skip_unless_tools!("srt-live-transmit" => "-version");

    tokio::time::timeout(TEST_TIMEOUT, async {
        let mut listener = SrtListener::bind(
            "127.0.0.1:0".parse::<SocketAddr>().unwrap(),
            HandshakeConfig::default(),
        )
        .await
        .expect("listener bind");
        let bound_addr = listener.local_addr().expect("listener local addr");
        let accept_jh =
            tokio::spawn(async move { listener.accept().await.expect("listener accept") });

        let src_port = free_udp_port();
        let _caller = spawn(
            Command::new("srt-live-transmit")
                .arg("-loglevel:error")
                .arg(format!("-chunk:{PAYLOAD_LEN}"))
                .arg(format!("udp://:{src_port}"))
                .arg(format!("srt://127.0.0.1:{}", bound_addr.port())),
        );

        // libsrt binds its `udp://` source before it connects, so once the
        // connection is accepted, feeding that source cannot race its startup.
        let mut receiver = tokio::time::timeout(Duration::from_secs(10), accept_jh)
            .await
            .expect("accept timed out — real libsrt caller handshake never completed")
            .expect("join listener accept");

        let received = feed_until_received(src_port, &mut receiver, 10).await;
        eprintln!("received chunks {received:?}");

        eprintln!("PASS: real_libsrt_caller_completes_handshake_and_data_with_our_listener");
        drop(receiver);
    })
    .await
    .expect("test timed out — real libsrt handshake/session stalled");
}

/// r08-SRT-W12: against a libsrt listener that requires a passphrase, our
/// (unencrypted) caller is refused — and the refusal is a typed
/// [`Error::Rejected`] carrying libsrt's own reason, not a generic error.
#[tokio::test]
async fn libsrt_refusal_reaches_the_caller_as_a_typed_rejection() {
    skip_unless_tools!("srt-live-transmit" => "-version");

    tokio::time::timeout(TEST_TIMEOUT, async {
        let listen_port = free_udp_port();
        let discard = TokioUdpSocket::bind("127.0.0.1:0").await.unwrap();
        let discard_port = discard.local_addr().unwrap().port();
        let _listener_proc = spawn_libsrt_listener(
            listen_port,
            discard_port,
            "passphrase=0123456789abcdef&pbkeylen=16",
            &[],
        );
        let err = tokio::time::timeout(
            Duration::from_secs(10),
            SrtSocket::connect(
                format!("127.0.0.1:{listen_port}")
                    .parse::<SocketAddr>()
                    .unwrap(),
                HandshakeConfig::default(),
            ),
        )
        .await
        .expect("connect never finished")
        .expect_err("a passphrase-protected libsrt listener must refuse an unencrypted caller");
        assert!(
            matches!(
                err,
                Error::Rejected(RejectionReason::Unsecure | RejectionReason::BadSecret)
            ),
            "expected libsrt's own rejection reason, got {err:?}"
        );
        eprintln!("libsrt refused with {err}");
    })
    .await
    .expect("test timed out");
}

/// r08-SRT-W6: our dropped caller tells libsrt it is leaving (SHUTDOWN, §3.2.7).
/// The libsrt listener is started with auto-reconnect off, so it exits when —
/// and only when — the connection is closed; hearing our SHUTDOWN, it must
/// exit within three seconds (without it libsrt would hold the connection for
/// its much longer expiry timeout).
#[tokio::test]
async fn libsrt_closes_promptly_when_our_socket_is_dropped() {
    skip_unless_tools!("srt-live-transmit" => "-version");

    tokio::time::timeout(TEST_TIMEOUT, async {
        let listen_port = free_udp_port();
        let discard = TokioUdpSocket::bind("127.0.0.1:0").await.unwrap();
        let discard_port = discard.local_addr().unwrap().port();
        let mut libsrt =
            spawn_libsrt_listener(listen_port, discard_port, "latency=120", &["-autoreconnect:no"]);
        let mut caller = tokio::time::timeout(
            Duration::from_secs(10),
            SrtSocket::connect(
                format!("127.0.0.1:{listen_port}")
                    .parse::<SocketAddr>()
                    .unwrap(),
                HandshakeConfig::default(),
            ),
        )
        .await
        .expect("connect timed out")
        .expect("connect");
        // Prove the connection is live, so libsrt is not simply idle.
        caller.send(&pattern(PAYLOAD_LEN, 3)).await.unwrap();
        let got = recv_at_least(&discard, PAYLOAD_LEN, Duration::from_secs(10)).await;
        assert_eq!(got.len(), PAYLOAD_LEN);

        drop(caller);

        // Bounded poll of an external process: it has 3 s to notice.
        let deadline = Instant::now() + Duration::from_secs(3);
        loop {
            if libsrt.0.try_wait().expect("try_wait").is_some() {
                break;
            }
            assert!(
                Instant::now() < deadline,
                "libsrt was still connected 3 s after our socket was dropped: no SHUTDOWN reached it"
            );
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .expect("test timed out");
}

/// A libsrt caller that exits cleanly sends SHUTDOWN; our accepted socket must
/// see the close at once (its `recv` ends), not after our own five-second idle
/// timeout.
#[tokio::test]
async fn our_listener_sees_libsrts_shutdown() {
    skip_unless_tools!("srt-live-transmit" => "-version");

    tokio::time::timeout(TEST_TIMEOUT, async {
        let mut listener = SrtListener::bind(
            "127.0.0.1:0".parse::<SocketAddr>().unwrap(),
            HandshakeConfig::default(),
        )
        .await
        .expect("listener bind");
        let bound_addr = listener.local_addr().expect("local addr");
        let accept_jh =
            tokio::spawn(async move { listener.accept().await.expect("listener accept") });
        let src_port = free_udp_port();
        // `-timeout:2` makes the libsrt process close the connection and exit
        // by itself two seconds after it starts.
        let _caller = spawn(
            Command::new("srt-live-transmit")
                .arg("-loglevel:error")
                .arg("-timeout:2")
                .arg(format!("udp://:{src_port}"))
                .arg(format!("srt://127.0.0.1:{}", bound_addr.port())),
        );
        let mut accepted = tokio::time::timeout(Duration::from_secs(10), accept_jh)
            .await
            .expect("accept timed out")
            .expect("join");
        let accepted_at = Instant::now();
        let ended = tokio::time::timeout(Duration::from_secs(4), accepted.recv())
            .await
            .expect("the close was not noticed within the idle timeout")
            .expect("recv");
        assert_eq!(ended, None, "no data was sent, the close ends the stream");
        assert!(
            accepted_at.elapsed() < Duration::from_millis(4_500),
            "noticed only after {:?}",
            accepted_at.elapsed()
        );
    })
    .await
    .expect("test timed out");
}

/// A UDP relay between a libsrt caller and our listener that drops the first
/// CONCLUSION *response* travelling back (everything else is forwarded), so
/// the libsrt caller has to repeat its CONCLUSION.
struct DropFirstConclusionResponse {
    front_port: u16,
    dropped: Arc<AtomicUsize>,
    stop: Arc<AtomicBool>,
    threads: Vec<std::thread::JoinHandle<()>>,
}

impl DropFirstConclusionResponse {
    fn spawn(listener_addr: SocketAddr) -> Self {
        let front = Arc::new(UdpSocket::bind("127.0.0.1:0").expect("bind front"));
        let back = Arc::new(UdpSocket::bind("127.0.0.1:0").expect("bind back"));
        back.connect(listener_addr).expect("connect back");
        // The reads wake up periodically only to notice `stop`.
        let poll = Some(Duration::from_millis(100));
        front.set_read_timeout(poll).unwrap();
        back.set_read_timeout(poll).unwrap();
        let front_port = front.local_addr().unwrap().port();
        let caller_addr: Arc<Mutex<Option<SocketAddr>>> = Arc::new(Mutex::new(None));
        let dropped = Arc::new(AtomicUsize::new(0));
        let stop = Arc::new(AtomicBool::new(false));

        let forward = {
            let (front, back, caller_addr, stop) = (
                Arc::clone(&front),
                Arc::clone(&back),
                Arc::clone(&caller_addr),
                Arc::clone(&stop),
            );
            std::thread::spawn(move || {
                let mut buf = [0u8; 2048];
                while !stop.load(Ordering::Relaxed) {
                    match front.recv_from(&mut buf) {
                        Ok((n, src)) => {
                            *caller_addr.lock().unwrap() = Some(src);
                            let _ = back.send(&buf[..n]);
                        }
                        Err(e)
                            if matches!(e.kind(), ErrorKind::WouldBlock | ErrorKind::TimedOut) => {}
                        Err(_) => break,
                    }
                }
            })
        };
        let backward = {
            let (front, back, caller_addr, stop, dropped) = (
                Arc::clone(&front),
                Arc::clone(&back),
                Arc::clone(&caller_addr),
                Arc::clone(&stop),
                Arc::clone(&dropped),
            );
            std::thread::spawn(move || {
                let mut buf = [0u8; 2048];
                while !stop.load(Ordering::Relaxed) {
                    match back.recv(&mut buf) {
                        Ok(n) => {
                            let is_conclusion = matches!(
                                SrtPacket::parse(&buf[..n]),
                                Ok(SrtPacket::Control(ControlPacket::Handshake(hp)))
                                    if hp.handshake_type == HandshakeType::Conclusion
                            );
                            if is_conclusion && dropped.load(Ordering::Relaxed) == 0 {
                                dropped.fetch_add(1, Ordering::Relaxed);
                                continue;
                            }
                            if let Some(to) = *caller_addr.lock().unwrap() {
                                let _ = front.send_to(&buf[..n], to);
                            }
                        }
                        Err(e)
                            if matches!(e.kind(), ErrorKind::WouldBlock | ErrorKind::TimedOut) => {}
                        Err(_) => break,
                    }
                }
            })
        };
        Self {
            front_port,
            dropped,
            stop,
            threads: vec![forward, backward],
        }
    }
}

impl Drop for DropFirstConclusionResponse {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        for t in self.threads.drain(..) {
            let _ = t.join();
        }
    }
}

/// r08-SRT-W5: the first CONCLUSION response is lost on the way to a real
/// libsrt caller. libsrt repeats its CONCLUSION; our listener — whose
/// application already has the accepted socket — must answer the repeat so the
/// libsrt side connects, and data must then flow through that same accepted
/// socket. (Before the fix the repeat went unanswered: libsrt timed out while
/// the application held a connection that was never real.)
#[tokio::test]
async fn libsrt_caller_survives_a_lost_conclusion_response() {
    skip_unless_tools!("srt-live-transmit" => "-version");

    tokio::time::timeout(TEST_TIMEOUT, async {
        let mut listener = SrtListener::bind(
            "127.0.0.1:0".parse::<SocketAddr>().unwrap(),
            HandshakeConfig::default(),
        )
        .await
        .expect("listener bind");
        let listener_addr = listener.local_addr().expect("local addr");
        let relay = DropFirstConclusionResponse::spawn(listener_addr);
        // A server loop: keep accepting (the repeated CONCLUSION arrives here).
        let (accepted_tx, mut accepted_rx) = tokio::sync::mpsc::unbounded_channel();
        let accept_loop = tokio::spawn(async move {
            while let Ok(socket) = listener.accept().await {
                if accepted_tx.send(socket).is_err() {
                    break;
                }
            }
        });

        let src_port = free_udp_port();
        let _caller = spawn(
            Command::new("srt-live-transmit")
                .arg("-loglevel:error")
                .arg(format!("-chunk:{PAYLOAD_LEN}"))
                .arg(format!("udp://:{src_port}"))
                .arg(format!("srt://127.0.0.1:{}", relay.front_port)),
        );

        let mut receiver = tokio::time::timeout(Duration::from_secs(10), accepted_rx.recv())
            .await
            .expect("accept timed out")
            .expect("accept loop ended");

        // With the response lost libsrt connects ~250 ms later than usual,
        // which is exactly when `feed_until_received`'s early chunks vanish.
        feed_until_received(src_port, &mut receiver, 10).await;
        assert_eq!(
            relay.dropped.load(Ordering::Relaxed),
            1,
            "the relay must really have dropped one CONCLUSION response"
        );
        assert!(
            accepted_rx.try_recv().is_err(),
            "the repeated CONCLUSION must not create a second connection"
        );
        accept_loop.abort();
    })
    .await
    .expect("test timed out");
}
