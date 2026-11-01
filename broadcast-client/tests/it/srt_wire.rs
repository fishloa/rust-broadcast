//! The SRT caller against real peers over loopback UDP: `srt-runtime`'s own tokio listener, and
//! libsrt's `srt-live-transmit` (clear and passphrase-encrypted). The libsrt tests skip loudly when
//! the binary is not on `PATH`; `SRT_REQUIRE_LIBSRT=1` makes the skip a failure.

use std::collections::BTreeMap;
use std::net::UdpSocket;
use std::process::{Child, Command, Stdio};
use std::time::Duration;

use crate::support::replay::{SampleRec, replay_paced};
use crate::support::srt_sim::config_with_seed;
use crate::support::udp_host::{TS_PACKET, ts_datagrams};
use crate::support::wire_host::WireHost;
use broadcast_client::{Config, Event, SrtEncryption, SrtKeyLength};
use srt_runtime::handshake_sm::HandshakeConfig;
use srt_runtime::io::SrtListener;

const SMALL: &str = "ts/h264_aac.ts";
const LIVE_CAPTURE: &str = "zenith-live/tf-1-8MB.ts";
const LIVE_PREFIX_BYTES: usize = 1_500_000;
const FAST_BPS: u64 = 100_000_000;
const SEND_INTERVAL: Duration = Duration::from_millis(2);
const DRAIN: Duration = Duration::from_millis(1_500);
const RUN_DEADLINE: Duration = Duration::from_secs(40);
const PASSPHRASE: &str = "0123456789abcdef";
const PAYLOAD_PACKETS: usize = 7;
const LOCAL_SRT_LATENCY_MS: u16 = 120;

struct KillOnDrop(Child);

impl Drop for KillOnDrop {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

fn libsrt_available(test: &str) -> bool {
    let found = std::env::var_os("PATH").is_some_and(|paths| {
        std::env::split_paths(&paths).any(|dir| dir.join("srt-live-transmit").is_file())
    });
    if !found {
        assert!(
            std::env::var_os("SRT_REQUIRE_LIBSRT").is_none(),
            "{test}: srt-live-transmit is required (SRT_REQUIRE_LIBSRT) but not on PATH"
        );
        eprintln!("SKIPPED {test}: srt-live-transmit (libsrt) is not on PATH");
    }
    found
}

fn free_udp_port() -> u16 {
    UdpSocket::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port()
}

fn baseline(ts: &[u8]) -> Vec<SampleRec> {
    replay_paced(ts, Config::default(), PAYLOAD_PACKETS * TS_PACKET, FAST_BPS).samples()
}

#[test]
fn the_caller_plays_a_srt_runtime_listener_stream_exactly() {
    let ts = test_fixtures::read(SMALL);
    let want = baseline(&ts);
    let datagrams = ts_datagrams(&ts, PAYLOAD_PACKETS);
    let (tx, rx) = std::sync::mpsc::channel();
    let server = std::thread::spawn(move || {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        rt.block_on(async move {
            let mut listener = SrtListener::bind("127.0.0.1:0", HandshakeConfig::default())
                .await
                .unwrap();
            tx.send(listener.local_addr().unwrap().port()).unwrap();
            let mut socket = listener.accept().await.unwrap();
            for d in &datagrams {
                socket.send(d).await.unwrap();
                tokio::time::sleep(SEND_INTERVAL).await;
            }
            tokio::time::sleep(Duration::from_millis(600)).await;
        });
    });
    let port = rx.recv().unwrap();
    let mut host = WireHost::open(
        &format!("srt://127.0.0.1:{port}"),
        config_with_seed(Config::default()),
    );
    let finished = host.run_until(RUN_DEADLINE, |h| {
        h.closes >= 1 && h.count(|e| matches!(e, Event::Sample { .. })) > 0
    });
    server.join().unwrap();
    assert!(finished, "the stream never ended");
    assert_eq!(host.replay().samples(), want);
    assert_eq!(host.session.datagram_stats().lost, 0);
}

fn libsrt_stream_is_intact(passphrase: Option<&str>, key_length: SrtKeyLength, test: &str) {
    if !libsrt_available(test) {
        return;
    }
    let Some(ts) = test_fixtures::require_bytes(LIVE_CAPTURE, test) else {
        return;
    };
    let ts = &ts[..LIVE_PREFIX_BYTES / TS_PACKET * TS_PACKET];
    let want: BTreeMap<(String, Option<i64>), u64> = baseline(ts)
        .into_iter()
        .map(|s| ((s.track, s.dts), s.hash))
        .collect();
    let srt_port = free_udp_port();
    let src = UdpSocket::bind("127.0.0.1:0").unwrap();
    let src_port = free_udp_port();
    let mut cmd = Command::new("srt-live-transmit");
    let mut listen = format!("srt://:{srt_port}?latency={LOCAL_SRT_LATENCY_MS}");
    if let Some(p) = passphrase {
        listen.push_str(&format!(
            "&passphrase={p}&pbkeylen={}",
            key_length_bytes(key_length)
        ));
    }
    let _libsrt = KillOnDrop(
        cmd.arg("-loglevel:error")
            .arg(format!("-chunk:{}", PAYLOAD_PACKETS * TS_PACKET))
            .arg(format!("udp://:{src_port}"))
            .arg(listen)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .expect("spawn srt-live-transmit"),
    );
    let mut config = config_with_seed(Config::default());
    if let Some(p) = passphrase {
        config.srt.encryption = Some(SrtEncryption::new(p, key_length));
    }
    let mut host = WireHost::open(&format!("srt://127.0.0.1:{srt_port}"), config);
    let datagrams = ts_datagrams(ts, PAYLOAD_PACKETS);
    let feeder = std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(400));
        for d in &datagrams {
            let _ = src.send_to(d, ("127.0.0.1", src_port));
            std::thread::sleep(SEND_INTERVAL);
        }
    });
    host.run_until(RUN_DEADLINE, |_| feeder.is_finished());
    host.run_until(DRAIN, |_| false);
    feeder.join().unwrap();
    let samples = host.replay().samples();
    assert!(
        samples.len() * 5 >= want.len(),
        "{test}: only {} of {} samples arrived",
        samples.len(),
        want.len()
    );
    let mut wrong = 0;
    for s in &samples {
        match want.get(&(s.track.clone(), s.dts)) {
            Some(h) if *h == s.hash => {}
            _ => wrong += 1,
        }
    }
    assert_eq!(wrong, 0, "{test}: {wrong} samples differ from the source");
    assert_eq!(host.session.datagram_stats().undecryptable, 0);
}

fn key_length_bytes(k: SrtKeyLength) -> usize {
    match k {
        SrtKeyLength::Aes128 => 16,
        SrtKeyLength::Aes192 => 24,
        _ => 32,
    }
}

#[test]
fn the_caller_plays_a_libsrt_listener_clear() {
    libsrt_stream_is_intact(None, SrtKeyLength::Aes128, "libsrt clear");
}

#[test]
fn the_caller_decrypts_a_libsrt_listener_stream_for_every_key_length() {
    for k in [
        SrtKeyLength::Aes128,
        SrtKeyLength::Aes192,
        SrtKeyLength::Aes256,
    ] {
        libsrt_stream_is_intact(Some(PASSPHRASE), k, &format!("libsrt {k}"));
    }
}

#[test]
fn a_wrong_passphrase_is_refused_by_libsrt() {
    if !libsrt_available("libsrt wrong passphrase") {
        return;
    }
    let srt_port = free_udp_port();
    let src_port = free_udp_port();
    let _libsrt = KillOnDrop(
        Command::new("srt-live-transmit")
            .arg("-loglevel:error")
            .arg(format!("udp://:{src_port}"))
            .arg(format!(
                "srt://:{srt_port}?passphrase={PASSPHRASE}&pbkeylen=16"
            ))
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .expect("spawn srt-live-transmit"),
    );
    let mut config = config_with_seed(Config::default());
    config.srt.encryption = Some(SrtEncryption::new(
        "another-passphrase-xyz",
        SrtKeyLength::Aes128,
    ));
    let mut host = WireHost::open(&format!("srt://127.0.0.1:{srt_port}"), config);
    let refused = host.run_until(Duration::from_secs(10), |h| {
        h.count(|e| {
            matches!(
                e,
                Event::Error {
                    kind: broadcast_client::ErrorKind::SrtRejected { .. },
                    ..
                }
            )
        }) > 0
    });
    assert!(refused, "libsrt did not refuse the wrong passphrase");
}

const DROP_EVERY: u64 = 25;
const PROXY_POLL: Duration = Duration::from_millis(20);

struct LossyProxy {
    stop: std::sync::Arc<std::sync::atomic::AtomicBool>,
    threads: Vec<std::thread::JoinHandle<()>>,
    pub port: u16,
    pub dropped: std::sync::Arc<std::sync::atomic::AtomicU64>,
}

impl LossyProxy {
    fn start(upstream_port: u16) -> Self {
        use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
        use std::sync::{Arc, Mutex};
        let front = UdpSocket::bind("127.0.0.1:0").unwrap();
        let port = front.local_addr().unwrap().port();
        let back = UdpSocket::bind("127.0.0.1:0").unwrap();
        back.connect(("127.0.0.1", upstream_port)).unwrap();
        front.set_read_timeout(Some(PROXY_POLL)).unwrap();
        back.set_read_timeout(Some(PROXY_POLL)).unwrap();
        let client = Arc::new(Mutex::new(None));
        let stop = Arc::new(AtomicBool::new(false));
        let dropped = Arc::new(AtomicU64::new(0));
        let (front_b, back_b) = (front.try_clone().unwrap(), back.try_clone().unwrap());
        let (c1, s1) = (client.clone(), stop.clone());
        let up = std::thread::spawn(move || {
            let mut buf = [0u8; 2048];
            while !s1.load(Ordering::Relaxed) {
                if let Ok((n, from)) = front.recv_from(&mut buf) {
                    *c1.lock().unwrap() = Some(from);
                    let _ = back.send(&buf[..n]);
                }
            }
        });
        let (c2, s2, d2) = (client, stop.clone(), dropped.clone());
        let down = std::thread::spawn(move || {
            let mut buf = [0u8; 2048];
            let mut data_seen = 0u64;
            while !s2.load(Ordering::Relaxed) {
                if let Ok(n) = back_b.recv(&mut buf) {
                    let is_data = buf[0] & 0x80 == 0;
                    if is_data {
                        data_seen += 1;
                        if data_seen % DROP_EVERY == 7 {
                            d2.fetch_add(1, Ordering::Relaxed);
                            continue;
                        }
                    }
                    if let Some(to) = *c2.lock().unwrap() {
                        let _ = front_b.send_to(&buf[..n], to);
                    }
                }
            }
        });
        Self {
            stop,
            threads: vec![up, down],
            port,
            dropped,
        }
    }
}

impl Drop for LossyProxy {
    fn drop(&mut self) {
        self.stop.store(true, std::sync::atomic::Ordering::Relaxed);
        for t in self.threads.drain(..) {
            let _ = t.join();
        }
    }
}

#[test]
fn libsrt_retransmissions_repair_a_lossy_link() {
    let test = "libsrt lossy link";
    if !libsrt_available(test) {
        return;
    }
    let Some(ts) = test_fixtures::require_bytes(LIVE_CAPTURE, test) else {
        return;
    };
    let ts = &ts[..LIVE_PREFIX_BYTES / TS_PACKET * TS_PACKET];
    let want: BTreeMap<(String, Option<i64>), u64> = baseline(ts)
        .into_iter()
        .map(|s| ((s.track, s.dts), s.hash))
        .collect();
    let srt_port = free_udp_port();
    let src = UdpSocket::bind("127.0.0.1:0").unwrap();
    let src_port = free_udp_port();
    let _libsrt = KillOnDrop(
        Command::new("srt-live-transmit")
            .arg("-loglevel:error")
            .arg(format!("-chunk:{}", PAYLOAD_PACKETS * TS_PACKET))
            .arg(format!("udp://:{src_port}"))
            .arg(format!("srt://:{srt_port}?latency=400"))
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .expect("spawn srt-live-transmit"),
    );
    let proxy = LossyProxy::start(srt_port);
    let mut config = config_with_seed(Config::default());
    config.srt.latency_ms = 400;
    let mut host = WireHost::open(&format!("srt://127.0.0.1:{}", proxy.port), config);
    let datagrams = ts_datagrams(ts, PAYLOAD_PACKETS);
    let feeder = std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(400));
        for d in &datagrams {
            let _ = src.send_to(d, ("127.0.0.1", src_port));
            std::thread::sleep(SEND_INTERVAL);
        }
    });
    host.run_until(RUN_DEADLINE, |_| feeder.is_finished());
    host.run_until(DRAIN, |_| false);
    feeder.join().unwrap();
    let dropped = proxy.dropped.load(std::sync::atomic::Ordering::Relaxed);
    let samples = host.replay().samples();
    assert!(dropped >= 10, "the proxy dropped only {dropped} packets");
    assert!(
        samples.len() * 5 >= want.len() * 4,
        "{} of {}",
        samples.len(),
        want.len()
    );
    assert!(
        samples
            .iter()
            .all(|s| want.get(&(s.track.clone(), s.dts)) == Some(&s.hash)),
        "a repaired stream must be byte-identical to the source"
    );
    let stats = host.session.datagram_stats();
    assert!(
        stats.lost * 4 <= dropped,
        "libsrt's retransmissions must recover most of the {dropped} dropped packets: {stats:?}"
    );
}
