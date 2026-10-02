//! Loss recovery against a genuine libsrt, over a UDP relay that drops chosen
//! datagrams (r08-SRT-W1, W2, W4; issue #1131's independent oracle for the
//! NAK path — `tests/io_loss_recovery.rs` only ever exchanges NAKs between two
//! halves of this crate).
//!
//! * `libsrt_listener_naks_are_served_by_our_sender`: our caller sends through
//!   a relay that drops a run and some isolated first-time DATA packets;
//!   libsrt's receiver reports them in NAKs (a range and singles, encoded by
//!   libsrt) and our sender must find and retransmit exactly those.
//! * `libsrt_sender_recovers_losses_reported_in_our_naks`: a libsrt caller
//!   sends through a relay that drops the same pattern; *our* receiver's NAKs
//!   (coalesced ranges and singles) must be understood by libsrt's sender.
//! * `libsrt_sender_recovers_a_hundreds_of_losses_list_from_our_periodic_nak`:
//!   450 isolated losses, with every small NAK the relay sees swallowed, so
//!   recovery rests entirely on our *periodic* NAK — whose loss list (about
//!   1.8 kB) must be split to fit the MTU or libsrt, which reads at most one
//!   MTU per datagram, never sees it.
//!
//! Skips LOUDLY when `srt-live-transmit` is missing (`SRT_REQUIRE_LIBSRT=1`
//! turns that into a failure). Waits are bounded; the relay's reads wake only
//! to notice its stop flag.

#![cfg(feature = "tokio")]

use std::io::ErrorKind;
use std::net::{SocketAddr, UdpSocket};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use srt_runtime::handshake_sm::HandshakeConfig;
use srt_runtime::io::{SrtListener, SrtSocket};
use srt_runtime::packet::{ControlPacket, SrtPacket};
use tokio::net::UdpSocket as TokioUdpSocket;

include!("support/skip.rs");

const TEST_TIMEOUT: Duration = Duration::from_secs(90);
/// Small payloads, so that a thousand of them fit a kernel UDP buffer in one
/// go (the periodic-NAK test needs hundreds of losses outstanding at once).
const PAYLOAD_LEN: usize = 256;
/// First byte of a synchronisation datagram (never the first byte of a
/// numbered chunk, whose first four bytes are a big-endian index < 2^24).
const SYNC_MARKER: u8 = 0xFF;

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

fn free_udp_port() -> u16 {
    UdpSocket::bind("127.0.0.1:0")
        .expect("bind free port")
        .local_addr()
        .expect("local addr")
        .port()
}

fn chunk(index: u32) -> Vec<u8> {
    let mut c: Vec<u8> = (0..PAYLOAD_LEN)
        .map(|i| u8::try_from(i % 251).unwrap().wrapping_add(index as u8))
        .collect();
    c[..4].copy_from_slice(&index.to_be_bytes());
    c
}

/// The chunk index a first-time DATA payload carries, if it is one of this
/// test's numbered chunks (not a sync datagram or anything else).
fn chunk_index(payload: &[u8]) -> Option<u32> {
    (payload.len() == PAYLOAD_LEN && payload.first() != Some(&SYNC_MARKER))
        .then(|| u32::from_be_bytes([payload[0], payload[1], payload[2], payload[3]]))
}

/// The path MTU the relay enforces: a datagram whose IP packet (payload + 20 B
/// IPv4 + 8 B UDP) exceeds it is dropped, as a real 1500-byte path would drop
/// or refuse it. Loopback itself would carry far larger datagrams, which would
/// hide an oversize NAK.
const PATH_MTU: usize = 1500;
const IP_UDP_HEADERS: usize = 28;

/// A UDP relay between a client and a server (an optional `blackhole` chunk is
/// dropped on every transmission, retransmissions included). A first-time DATA datagram from
/// the client carrying numbered chunk `i` is dropped when `forward_drop(i)`
/// says so; a datagram from the server is dropped when `backward_drop` says so
/// for the parsed packet. Everything else is forwarded unchanged. (Dropping by
/// chunk index, not by arrival ordinal, keeps the pattern independent of how
/// many sync datagrams libsrt happened to pass first.)
struct Relay {
    front_port: u16,
    forward_dropped: Arc<AtomicUsize>,
    backward_dropped: Arc<AtomicUsize>,
    /// Datagrams dropped for exceeding [`PATH_MTU`], in either direction.
    oversize_dropped: Arc<AtomicUsize>,
    stop: Arc<AtomicBool>,
    threads: Vec<std::thread::JoinHandle<()>>,
}

impl Relay {
    fn spawn(
        server: SocketAddr,
        forward_drop: impl Fn(u32) -> bool + Send + 'static,
        backward_drop: impl Fn(&SrtPacket<'_>) -> bool + Send + 'static,
        blackhole: Option<u32>,
    ) -> Self {
        let front = Arc::new(UdpSocket::bind("127.0.0.1:0").expect("bind front"));
        let back = Arc::new(UdpSocket::bind("127.0.0.1:0").expect("bind back"));
        back.connect(server).expect("connect back");
        let poll = Some(Duration::from_millis(100));
        front.set_read_timeout(poll).unwrap();
        back.set_read_timeout(poll).unwrap();
        let front_port = front.local_addr().unwrap().port();
        let client: Arc<Mutex<Option<SocketAddr>>> = Arc::new(Mutex::new(None));
        let forward_dropped = Arc::new(AtomicUsize::new(0));
        let backward_dropped = Arc::new(AtomicUsize::new(0));
        let oversize_dropped = Arc::new(AtomicUsize::new(0));
        let stop = Arc::new(AtomicBool::new(false));
        // A read that merely woke up to check `stop`, or reported the ICMP
        // "port unreachable" of an earlier datagram (the server process may
        // not have bound its port yet), is not the end of the relay.
        let transient = |e: &std::io::Error| {
            matches!(
                e.kind(),
                ErrorKind::WouldBlock | ErrorKind::TimedOut | ErrorKind::ConnectionRefused
            )
        };

        let forward = {
            let (front, back, client, stop, dropped, oversize) = (
                Arc::clone(&front),
                Arc::clone(&back),
                Arc::clone(&client),
                Arc::clone(&stop),
                Arc::clone(&forward_dropped),
                Arc::clone(&oversize_dropped),
            );
            std::thread::spawn(move || {
                let mut buf = [0u8; 2048];
                while !stop.load(Ordering::Relaxed) {
                    match front.recv_from(&mut buf) {
                        Ok((n, src)) => {
                            *client.lock().unwrap_or_else(PoisonError::into_inner) = Some(src);
                            if n + IP_UDP_HEADERS > PATH_MTU {
                                oversize.fetch_add(1, Ordering::Relaxed);
                                continue;
                            }
                            if let Ok(SrtPacket::Data(d)) = SrtPacket::parse(&buf[..n])
                                && let Some(index) = chunk_index(d.data)
                                && ((!d.retransmitted && forward_drop(index))
                                    // A blackholed chunk never gets through,
                                    // retransmissions included.
                                    || blackhole == Some(index))
                            {
                                dropped.fetch_add(1, Ordering::Relaxed);
                                continue;
                            }
                            let _ = back.send(&buf[..n]);
                        }
                        Err(e) if transient(&e) => {}
                        Err(_) => break,
                    }
                }
            })
        };
        let backward = {
            let (front, back, client, stop, dropped, oversize) = (
                Arc::clone(&front),
                Arc::clone(&back),
                Arc::clone(&client),
                Arc::clone(&stop),
                Arc::clone(&backward_dropped),
                Arc::clone(&oversize_dropped),
            );
            std::thread::spawn(move || {
                let mut buf = [0u8; 2048];
                while !stop.load(Ordering::Relaxed) {
                    match back.recv(&mut buf) {
                        Ok(n) => {
                            if n + IP_UDP_HEADERS > PATH_MTU {
                                oversize.fetch_add(1, Ordering::Relaxed);
                                continue;
                            }
                            if SrtPacket::parse(&buf[..n]).is_ok_and(|p| backward_drop(&p)) {
                                dropped.fetch_add(1, Ordering::Relaxed);
                                continue;
                            }
                            let to = *client.lock().unwrap_or_else(PoisonError::into_inner);
                            if let Some(to) = to {
                                let _ = front.send_to(&buf[..n], to);
                            }
                        }
                        Err(e) if transient(&e) => {}
                        Err(_) => break,
                    }
                }
            })
        };
        Self {
            front_port,
            forward_dropped,
            backward_dropped,
            oversize_dropped,
            stop,
            threads: vec![forward, backward],
        }
    }
}

impl Drop for Relay {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        for t in self.threads.drain(..) {
            let _ = t.join();
        }
    }
}

/// Chunks the relay drops: a run (one NAK range) and some isolated packets.
/// The last chunk of each 20-chunk window (19, 39, 59) is never among them: a
/// lost *last* packet reveals no gap, and the next window is only sent once
/// this one has been delivered.
fn drop_pattern(index: u32) -> bool {
    matches!(index, 3..=7 | 20 | 31 | 32 | 45)
}
const DROP_PATTERN_LEN: usize = 5 + 1 + 2 + 1;

/// Our caller → relay → libsrt listener → UDP.
#[tokio::test]
async fn libsrt_listener_naks_are_served_by_our_sender() {
    skip_unless_tools!("srt-live-transmit" => "-version");
    tokio::time::timeout(TEST_TIMEOUT, async {
        const CHUNKS: u32 = 60;
        let listen_port = free_udp_port();
        let discard = TokioUdpSocket::bind("127.0.0.1:0").await.unwrap();
        let discard_port = discard.local_addr().unwrap().port();
        let _libsrt = spawn(
            Command::new("srt-live-transmit")
                .arg("-loglevel:error")
                .arg(format!("srt://:{listen_port}"))
                .arg(format!("udp://127.0.0.1:{discard_port}")),
        );
        let relay = Relay::spawn(
            format!("127.0.0.1:{listen_port}").parse().unwrap(),
            drop_pattern,
            |_| false,
            None,
        );
        let mut caller = SrtSocket::connect(
            format!("127.0.0.1:{}", relay.front_port)
                .parse::<SocketAddr>()
                .unwrap(),
            HandshakeConfig::default(),
        )
        .await
        .expect("connect through the relay");

        for index in 0..CHUNKS {
            caller.send(&chunk(index)).await.expect("send");
        }
        let mut buf = vec![0u8; 65536];
        for index in 0..CHUNKS {
            let n = tokio::time::timeout(Duration::from_secs(15), discard.recv(&mut buf))
                .await
                .unwrap_or_else(|_| panic!("chunk {index} never recovered after the relay's loss"))
                .expect("recv");
            assert_eq!(&buf[..n], chunk(index).as_slice(), "chunk {index}");
        }
        assert_eq!(
            relay.forward_dropped.load(Ordering::Relaxed),
            DROP_PATTERN_LEN,
            "the relay must really have dropped the pattern"
        );
    })
    .await
    .expect("test timed out");
}

/// Feed numbered chunks to a libsrt caller's `udp://` source and collect them
/// from `receiver`, windowed (a window is only followed by the next once it has
/// been delivered), after a sync phase that waits for libsrt to be connected.
async fn feed_and_collect(src_port: u16, receiver: &mut SrtSocket, chunks: u32, window: u32) {
    let feed = TokioUdpSocket::bind("127.0.0.1:0").await.unwrap();
    let target = ("127.0.0.1", src_port);
    // libsrt drops what its UDP source sees until its SRT side has connected.
    let mut ticker = tokio::time::interval(Duration::from_millis(50));
    loop {
        tokio::select! {
            _ = ticker.tick() => {
                feed.send_to(&[SYNC_MARKER; 64], target).await.unwrap();
            }
            r = receiver.recv() => {
                if r.expect("recv").expect("connection ended").first() == Some(&SYNC_MARKER) {
                    break;
                }
            }
        }
    }
    let mut next = 0u32;
    while next < chunks {
        let end = (next + window).min(chunks);
        for index in next..end {
            feed.send_to(&chunk(index), target).await.unwrap();
        }
        let mut expected = next;
        while expected < end {
            let payload = tokio::time::timeout(Duration::from_secs(20), receiver.recv())
                .await
                .unwrap_or_else(|_| panic!("chunk {expected} was never delivered"))
                .expect("recv")
                .expect("connection ended");
            if payload.first() == Some(&SYNC_MARKER) {
                continue; // a late sync datagram
            }
            assert_eq!(payload, chunk(expected), "chunk {expected}");
            expected += 1;
        }
        next = end;
    }
}

async fn our_listener_behind_a_relay(
    config: HandshakeConfig,
    libsrt_options: &str,
    blackhole: Option<u32>,
    forward_drop: impl Fn(u32) -> bool + Send + 'static,
    backward_drop: impl Fn(&SrtPacket<'_>) -> bool + Send + 'static,
) -> (SrtSocket, Relay, KillOnDrop, u16) {
    let mut listener = SrtListener::bind("127.0.0.1:0".parse::<SocketAddr>().unwrap(), config)
        .await
        .expect("bind");
    let bound = listener.local_addr().unwrap();
    let accept = tokio::spawn(async move { listener.accept().await.expect("accept") });
    let relay = Relay::spawn(bound, forward_drop, backward_drop, blackhole);
    let src_port = free_udp_port();
    let libsrt = spawn(
        Command::new("srt-live-transmit")
            .arg("-loglevel:error")
            .arg(format!("-chunk:{PAYLOAD_LEN}"))
            .arg(format!("udp://:{src_port}"))
            .arg(format!(
                "srt://127.0.0.1:{}?{libsrt_options}",
                relay.front_port
            )),
    );
    let receiver = tokio::time::timeout(Duration::from_secs(10), accept)
        .await
        .expect("accept timed out")
        .expect("join");
    (receiver, relay, libsrt, src_port)
}

/// libsrt caller → relay → our listener: our NAKs reach libsrt's sender
/// intact (ranges and singles) and it retransmits.
#[tokio::test]
async fn libsrt_sender_recovers_losses_reported_in_our_naks() {
    skip_unless_tools!("srt-live-transmit" => "-version");
    tokio::time::timeout(TEST_TIMEOUT, async {
        let (mut receiver, relay, _libsrt, src_port) = our_listener_behind_a_relay(
            HandshakeConfig {
                latency_ms: 500,
                ..HandshakeConfig::default()
            },
            "latency=120",
            None,
            drop_pattern,
            |_| false,
        )
        .await;
        feed_and_collect(src_port, &mut receiver, 60, 20).await;
        assert!(
            relay.forward_dropped.load(Ordering::Relaxed) > 0,
            "the relay must really have dropped data"
        );
    })
    .await
    .expect("test timed out");
}

/// 450 isolated losses; every NAK the relay can see that fits one range entry
/// is swallowed, so what recovers the stream is our *periodic* NAK, whose
/// loss list is far larger than one MTU. The relay models a 1500-byte path: it
/// drops (and counts) any datagram that would not fit, which is what the
/// unsplit NAK would hit on a real network — loopback itself would carry it.
#[tokio::test]
async fn libsrt_sender_recovers_a_hundreds_of_losses_list_from_our_periodic_nak() {
    skip_unless_tools!("srt-live-transmit" => "-version");
    tokio::time::timeout(TEST_TIMEOUT, async {
        const CHUNKS: u32 = 1_000;
        const FIRST_LOSS: u32 = 20;
        const LAST_LOSS: u32 = 920;
        let (mut receiver, relay, _libsrt, src_port) = our_listener_behind_a_relay(
            HandshakeConfig {
                // Room for the periodic NAK to do the recovering before
                // libsrt's sender would give up on a packet.
                latency_ms: 2_000,
                ..HandshakeConfig::default()
            },
            "latency=120",
            None,
            // Drop every other first-time chunk in the range (isolated
            // losses: one single-entry NAK each). Even indices only, so the
            // last chunk (999) is kept.
            |index| (FIRST_LOSS..LAST_LOSS).contains(&index) && index % 2 == 0,
            |packet| {
                matches!(
                    packet,
                    SrtPacket::Control(ControlPacket::Nak(nak)) if nak.raw_loss_list.len() <= 8
                )
            },
        )
        .await;
        feed_and_collect(src_port, &mut receiver, CHUNKS, CHUNKS).await;
        assert_eq!(
            relay.forward_dropped.load(Ordering::Relaxed),
            usize::try_from((LAST_LOSS - FIRST_LOSS) / 2).unwrap(),
            "the relay must have dropped the 450 isolated packets"
        );
        assert!(
            relay.backward_dropped.load(Ordering::Relaxed) > 0,
            "the relay must have swallowed small NAKs, forcing the periodic one"
        );
        assert_eq!(
            relay.oversize_dropped.load(Ordering::Relaxed),
            0,
            "a datagram exceeded the {PATH_MTU}-byte path MTU: the periodic NAK was not split"
        );
    })
    .await
    .expect("test timed out");
}

/// The negotiated TLPKTDROP flag decides whether a gap may be skipped. With a
/// libsrt caller that disabled it (`tlpktdrop=0`) the flag is not negotiated,
/// and a chunk lost for good (every transmission of it is dropped, libsrt
/// keeps retransmitting) must stall delivery behind it — our receiver must not
/// give up on it after the latency and acknowledge a packet it never got. The
/// chunks before it still arrive; none after it does, however long we wait.
#[tokio::test]
async fn without_negotiated_tlpktdrop_a_lost_chunk_stalls_delivery_behind_it() {
    skip_unless_tools!("srt-live-transmit" => "-version");
    tokio::time::timeout(TEST_TIMEOUT, async {
        const LOST: u32 = 5;
        let (mut receiver, relay, _libsrt, src_port) = our_listener_behind_a_relay(
            HandshakeConfig::default(), // 120 ms latency
            "tlpktdrop=0&latency=120",
            Some(LOST),
            |_| false,
            |_| false,
        )
        .await;
        // Sync (libsrt drops its source's data until connected), then 20 chunks.
        let feed = TokioUdpSocket::bind("127.0.0.1:0").await.unwrap();
        let target = ("127.0.0.1", src_port);
        let mut ticker = tokio::time::interval(Duration::from_millis(50));
        loop {
            tokio::select! {
                _ = ticker.tick() => {
                    feed.send_to(&[SYNC_MARKER; 64], target).await.unwrap();
                }
                r = receiver.recv() => {
                    if r.expect("recv").expect("ended").first() == Some(&SYNC_MARKER) {
                        break;
                    }
                }
            }
        }
        for index in 0..20u32 {
            feed.send_to(&chunk(index), target).await.unwrap();
        }
        // 0..LOST arrive in order.
        let mut next = 0u32;
        while next < LOST {
            let payload = tokio::time::timeout(Duration::from_secs(10), receiver.recv())
                .await
                .expect("the chunks before the loss must arrive")
                .expect("recv")
                .expect("ended");
            if payload.first() == Some(&SYNC_MARKER) {
                continue;
            }
            assert_eq!(payload, chunk(next));
            next += 1;
        }
        // Nothing may follow, for several multiples of the 120 ms latency and
        // its 150 ms too-late threshold.
        let after = tokio::time::timeout(Duration::from_secs(2), async {
            loop {
                let p = receiver.recv().await.expect("recv")?;
                if p.first() != Some(&SYNC_MARKER) {
                    break Some(p);
                }
            }
        })
        .await;
        assert!(
            after.is_err(),
            "delivery went past the lost chunk even though TLPKTDROP was not negotiated: {:?}",
            after.ok().flatten().map(|p| chunk_index(&p))
        );
        assert!(
            relay.forward_dropped.load(Ordering::Relaxed) >= 1,
            "the relay must have dropped the blackholed chunk"
        );
    })
    .await
    .expect("test timed out");
}
