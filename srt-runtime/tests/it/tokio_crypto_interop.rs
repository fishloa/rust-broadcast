#![cfg(all(feature = "tokio", feature = "crypto"))]

use std::net::SocketAddr;
use std::time::Duration;
use test_fixtures::process::KillOnDrop;

use srt_runtime::Error;
use srt_runtime::handshake_sm::{CryptoConfig, HandshakeConfig};
use srt_runtime::io::{IoConfig, SrtListener, SrtSocket};
use srt_runtime::packet::EncryptionField;
use tokio::net::UdpSocket as TokioUdpSocket;

include!("support/skip.rs");
include!("support/libsrt_spawn.rs");

const TEST_TIMEOUT: Duration = Duration::from_secs(60);
const PAYLOAD_LEN: usize = 1316;
const LOADED_HOST_LATENCY_MS: u16 = 5_000;
const PASSPHRASE: &str = "0123456789abcdef";
const SALT: [u8; 16] = [0x11u8; 16];

fn pattern(n: usize, seed: u8) -> Vec<u8> {
    (0..n).map(|i| seed.wrapping_add(i as u8)).collect()
}

fn crypto_config(passphrase: &str, key_len: usize) -> HandshakeConfig {
    let encryption_field = match key_len {
        16 => EncryptionField::Aes128,
        24 => EncryptionField::Aes192,
        32 => EncryptionField::Aes256,
        _ => EncryptionField::Aes128,
    };
    HandshakeConfig {
        encryption_field,
        crypto: Some(CryptoConfig {
            passphrase: passphrase.as_bytes().to_vec(),
            salt: SALT,
            sek: vec![0x22u8; key_len],
        }),
        ..HandshakeConfig::default()
    }
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

fn spawn_libsrt_listener(
    discard_port: u16,
    passphrase: &str,
    pbkeylen: usize,
) -> (KillOnDrop, u16) {
    let (child, port) = spawn_libsrt_ready(&[LIBSRT_BOUND_LINE], |port| {
        vec![
            format!("srt://:{port}?passphrase={passphrase}&pbkeylen={pbkeylen}"),
            format!("udp://127.0.0.1:{discard_port}"),
        ]
    });
    (KillOnDrop(child), port)
}

#[tokio::test]
async fn tokio_caller_to_libsrt_listener_aes128() {
    tokio::time::timeout(TEST_TIMEOUT, async {
        skip_unless_tools!("srt-live-transmit" => "-version");
        let discard = TokioUdpSocket::bind("127.0.0.1:0").await.unwrap();
        let discard_port = discard.local_addr().unwrap().port();
        let (_lib, listen_port) = spawn_libsrt_listener(discard_port, PASSPHRASE, 16);
        let mut caller = tokio::time::timeout(
            Duration::from_secs(10),
            SrtSocket::connect(
                format!("127.0.0.1:{listen_port}")
                    .parse::<SocketAddr>()
                    .unwrap(),
                crypto_config(PASSPHRASE, 16),
            ),
        )
        .await
        .unwrap()
        .unwrap();
        let burst = pattern(PAYLOAD_LEN * 10, 1);
        send_burst(&mut caller, &burst).await;
        let received = recv_at_least(&discard, burst.len(), Duration::from_secs(10)).await;
        assert_eq!(received, burst);
        drop(caller);
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn tokio_caller_to_libsrt_listener_aes192() {
    tokio::time::timeout(TEST_TIMEOUT, async {
        skip_unless_tools!("srt-live-transmit" => "-version");
        let discard = TokioUdpSocket::bind("127.0.0.1:0").await.unwrap();
        let discard_port = discard.local_addr().unwrap().port();
        let (_lib, listen_port) = spawn_libsrt_listener(discard_port, PASSPHRASE, 24);
        let mut caller = tokio::time::timeout(
            Duration::from_secs(10),
            SrtSocket::connect(
                format!("127.0.0.1:{listen_port}")
                    .parse::<SocketAddr>()
                    .unwrap(),
                crypto_config(PASSPHRASE, 24),
            ),
        )
        .await
        .unwrap()
        .unwrap();
        let burst = pattern(PAYLOAD_LEN * 10, 2);
        send_burst(&mut caller, &burst).await;
        let received = recv_at_least(&discard, burst.len(), Duration::from_secs(10)).await;
        assert_eq!(received, burst);
        drop(caller);
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn tokio_caller_to_libsrt_listener_aes256() {
    tokio::time::timeout(TEST_TIMEOUT, async {
        skip_unless_tools!("srt-live-transmit" => "-version");
        let discard = TokioUdpSocket::bind("127.0.0.1:0").await.unwrap();
        let discard_port = discard.local_addr().unwrap().port();
        let (_lib, listen_port) = spawn_libsrt_listener(discard_port, PASSPHRASE, 32);
        let mut caller = tokio::time::timeout(
            Duration::from_secs(10),
            SrtSocket::connect(
                format!("127.0.0.1:{listen_port}")
                    .parse::<SocketAddr>()
                    .unwrap(),
                crypto_config(PASSPHRASE, 32),
            ),
        )
        .await
        .unwrap()
        .unwrap();
        let burst = pattern(PAYLOAD_LEN * 10, 3);
        send_burst(&mut caller, &burst).await;
        let received = recv_at_least(&discard, burst.len(), Duration::from_secs(10)).await;
        assert_eq!(received, burst);
        drop(caller);
    })
    .await
    .unwrap();
}

async fn libsrt_caller_to_tokio_listener(key_len: usize) {
    tokio::time::timeout(TEST_TIMEOUT, async {
        skip_unless_tools!("srt-live-transmit" => "-version");
        let mut listener = SrtListener::bind("127.0.0.1:0", crypto_config(PASSPHRASE, key_len))
            .await
            .unwrap();
        let bound = listener.local_addr().unwrap();
        let accept_jh = tokio::spawn(async move { listener.accept().await.unwrap() });
        let (caller, src_port) = spawn_libsrt_ready(&[LIBSRT_UDP_SOURCE_BOUND_LINE], |src_port| {
            vec![
                format!("-chunk:{PAYLOAD_LEN}"),
                format!("udp://:{src_port}"),
                format!(
                    "srt://127.0.0.1:{}?passphrase={}&pbkeylen={}",
                    bound.port(),
                    PASSPHRASE,
                    key_len
                ),
            ]
        });
        let _caller = KillOnDrop(caller);
        let mut receiver = tokio::time::timeout(Duration::from_secs(10), accept_jh)
            .await
            .unwrap()
            .unwrap();
        // feed 10 chunks
        let feed = TokioUdpSocket::bind("127.0.0.1:0").await.unwrap();
        let mut received = 0usize;
        let deadline = tokio::time::Instant::now() + Duration::from_secs(20);
        while received < 10 {
            if tokio::time::Instant::now() > deadline {
                panic!("timeout");
            }
            // feed
            let _ = feed
                .send_to(&pattern(PAYLOAD_LEN, 7), ("127.0.0.1", src_port))
                .await;
            // try recv
            if let Ok(Ok(Some(p))) =
                tokio::time::timeout(Duration::from_millis(50), receiver.recv()).await
            {
                assert_eq!(p.len(), PAYLOAD_LEN);
                received += 1;
            }
        }
        assert_eq!(received, 10);
        drop(receiver);
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn libsrt_caller_to_tokio_listener_aes128() {
    libsrt_caller_to_tokio_listener(16).await;
}

#[tokio::test]
async fn libsrt_caller_to_tokio_listener_aes256() {
    libsrt_caller_to_tokio_listener(32).await;
}

#[tokio::test]
async fn tokio_wrong_passphrase_is_refused() {
    tokio::time::timeout(TEST_TIMEOUT, async {
        skip_unless_tools!("srt-live-transmit" => "-version");
        // direction 1: our caller wrong passphrase to libsrt listener
        let discard = TokioUdpSocket::bind("127.0.0.1:0").await.unwrap();
        let discard_port = discard.local_addr().unwrap().port();
        let (_lib, listen_port) = spawn_libsrt_listener(discard_port, PASSPHRASE, 16);
        let err = tokio::time::timeout(
            Duration::from_secs(10),
            SrtSocket::connect(
                format!("127.0.0.1:{listen_port}")
                    .parse::<SocketAddr>()
                    .unwrap(),
                crypto_config("wrong-passphrase-xxxxxxxx", 16),
            ),
        )
        .await
        .unwrap()
        .unwrap_err();
        assert!(
            matches!(err, Error::Rejected(_)),
            "expected rejection, got {err:?}"
        );
        // direction 2: libsrt caller wrong passphrase to our listener
        let mut listener = SrtListener::bind("127.0.0.1:0", crypto_config(PASSPHRASE, 16))
            .await
            .unwrap();
        let bound = listener.local_addr().unwrap();
        let accept_jh = tokio::spawn(async move { listener.accept().await });
        let (caller, _src_port) = spawn_libsrt_ready(&[LIBSRT_UDP_SOURCE_BOUND_LINE], |src_port| {
            vec![
                format!("-chunk:{PAYLOAD_LEN}"),
                format!("udp://:{src_port}"),
                format!(
                    "srt://127.0.0.1:{}?passphrase=wrong-passphrase-xxxxxxxx&pbkeylen=16",
                    bound.port()
                ),
            ]
        });
        let _caller = KillOnDrop(caller);
        // rejected connection never reaches accept
        assert!(
            tokio::time::timeout(Duration::from_secs(5), accept_jh)
                .await
                .is_err()
        );
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn tokio_key_refresh_keeps_delivering() {
    tokio::time::timeout(TEST_TIMEOUT, async {
        use srt_runtime::km_refresh::KmRefreshThresholds;
        let thresholds = KmRefreshThresholds {
            refresh_period: 50,
            pre_announcement_period: 5,
        };
        let loaded_host_config = || HandshakeConfig {
            latency_ms: LOADED_HOST_LATENCY_MS,
            ..crypto_config(PASSPHRASE, 16)
        };
        let io = IoConfig::default().with_km_refresh_thresholds(thresholds);
        let mut listener = SrtListener::bind_with("127.0.0.1:0", loaded_host_config(), io)
            .await
            .unwrap();
        let bound = listener.local_addr().unwrap();
        let accept_jh = tokio::spawn(async move { listener.accept().await.unwrap() });
        let mut caller = SrtSocket::connect_with(
            format!("127.0.0.1:{}", bound.port())
                .parse::<SocketAddr>()
                .unwrap(),
            loaded_host_config(),
            io,
        )
        .await
        .unwrap();
        let mut receiver = tokio::time::timeout(Duration::from_secs(10), accept_jh)
            .await
            .unwrap()
            .unwrap();

        let mut sent_chunks = Vec::new();
        let mut received: Vec<Vec<u8>> = Vec::new();
        for i in 0..80u32 {
            let mut chunk = pattern(PAYLOAD_LEN, 7);
            chunk[..4].copy_from_slice(&i.to_be_bytes());
            sent_chunks.push(chunk.clone());
            caller.send(&chunk).await.unwrap();
            // drain
            while let Ok(Ok(Some(p))) =
                tokio::time::timeout(Duration::from_millis(5), receiver.recv()).await
            {
                received.push(p.to_vec());
            }
        }
        // drain remaining
        let drain_deadline = tokio::time::Instant::now() + Duration::from_secs(10);
        while received.len() < 80 && tokio::time::Instant::now() < drain_deadline {
            if let Ok(Ok(Some(p))) =
                tokio::time::timeout(Duration::from_millis(50), receiver.recv()).await
            {
                received.push(p.to_vec());
            }
        }
        let have: std::collections::BTreeSet<u32> = received
            .iter()
            .map(|p| u32::from_be_bytes([p[0], p[1], p[2], p[3]]))
            .collect();
        let missing: Vec<u32> = (0..80).filter(|i| !have.contains(i)).collect();
        assert_eq!(
            received.len(),
            80,
            "expected 80 packets, got {}; missing {missing:?}",
            received.len()
        );
        for (i, p) in received.iter().enumerate() {
            assert_eq!(p, &sent_chunks[i], "packet {i} mismatch");
        }
        drop(caller);
        drop(receiver);
    })
    .await
    .unwrap();
}

// ---------- Rendezvous + crypto (issue #1658) ----------

fn libsrt_rdv_crypto_uri(
    peer_port: u16,
    local_port: u16,
    passphrase: &str,
    pbkeylen: usize,
) -> String {
    format!(
        "srt://127.0.0.1:{peer_port}?mode=rendezvous&port={local_port}&latency=120&passphrase={passphrase}&pbkeylen={pbkeylen}"
    )
}

fn crypto_patient_config(passphrase: &str, key_len: usize) -> HandshakeConfig {
    let mut c = crypto_config(passphrase, key_len);
    c.max_retries = 60;
    c
}

async fn rendezvous_with_libsrt(key_len: usize, seed: u8) {
    tokio::time::timeout(TEST_TIMEOUT, async {
        skip_unless_tools!("srt-live-transmit" => "-version");
        let our_sock = TokioUdpSocket::bind("127.0.0.1:0").await.unwrap();
        let our_port = our_sock.local_addr().unwrap().port();
        let discard = TokioUdpSocket::bind("127.0.0.1:0").await.unwrap();
        let discard_port = discard.local_addr().unwrap().port();
        let (libsrt, lib_port) = spawn_libsrt_ready(&[LIBSRT_BOUND_LINE], |lib_port| {
            vec![
                libsrt_rdv_crypto_uri(our_port, lib_port, PASSPHRASE, key_len),
                format!("udp://127.0.0.1:{discard_port}"),
            ]
        });
        let _libsrt = KillOnDrop(libsrt);

        let mut ours = tokio::time::timeout(
            Duration::from_secs(25),
            SrtSocket::rendezvous_from_socket(
                our_sock,
                format!("127.0.0.1:{lib_port}")
                    .parse::<SocketAddr>()
                    .unwrap(),
                crypto_patient_config(PASSPHRASE, key_len),
                IoConfig::default().with_handshake(Duration::from_secs(20)),
            ),
        )
        .await
        .unwrap_or_else(|_| panic!("rendezvous never completed"))
        .unwrap_or_else(|e| panic!("rendezvous with libsrt: {e}"));

        let burst = pattern(PAYLOAD_LEN * 8, seed);
        for chunk in burst.chunks(PAYLOAD_LEN) {
            ours.send(chunk).await.expect("send");
        }
        let got = recv_at_least(&discard, burst.len(), Duration::from_secs(10)).await;
        assert_eq!(got, burst, "burst must be forwarded byte-identical");
        drop(ours);
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn rendezvous_with_libsrt_aes128() {
    rendezvous_with_libsrt(16, 11).await;
}

#[tokio::test]
async fn rendezvous_with_libsrt_aes256() {
    rendezvous_with_libsrt(32, 22).await;
}

#[tokio::test]
async fn rendezvous_wrong_passphrase_is_refused() {
    tokio::time::timeout(TEST_TIMEOUT, async {
        skip_unless_tools!("srt-live-transmit" => "-version");
        let our_sock = TokioUdpSocket::bind("127.0.0.1:0").await.unwrap();
        let our_port = our_sock.local_addr().unwrap().port();
        let _discard = TokioUdpSocket::bind("127.0.0.1:0").await.unwrap();
        let discard_port = _discard.local_addr().unwrap().port();
        let (libsrt, lib_port) = spawn_libsrt_ready(&[LIBSRT_BOUND_LINE], |lib_port| {
            vec![
                libsrt_rdv_crypto_uri(our_port, lib_port, PASSPHRASE, 16),
                format!("udp://127.0.0.1:{discard_port}"),
            ]
        });
        let _libsrt = KillOnDrop(libsrt);

        let err = tokio::time::timeout(
            Duration::from_secs(15),
            SrtSocket::rendezvous_from_socket(
                our_sock,
                format!("127.0.0.1:{lib_port}")
                    .parse::<SocketAddr>()
                    .unwrap(),
                crypto_patient_config("wrong-passphrase-xxxxxxxx", 16),
                IoConfig::default().with_handshake(Duration::from_secs(20)),
            ),
        )
        .await
        .unwrap()
        .unwrap_err();
        assert!(
            matches!(err, Error::Rejected(_)),
            "expected rejection, got {err:?}"
        );
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn tokio_rendezvous_pair_encrypted_round_trip() {
    tokio::time::timeout(TEST_TIMEOUT, async {
        let sock_a = TokioUdpSocket::bind("127.0.0.1:0").await.unwrap();
        let port_a = sock_a.local_addr().unwrap().port();
        let sock_b = TokioUdpSocket::bind("127.0.0.1:0").await.unwrap();
        let port_b = sock_b.local_addr().unwrap().port();
        let cfg = crypto_config(PASSPHRASE, 16);
        let io = IoConfig::default().with_handshake(Duration::from_secs(20));
        let cfg_a = cfg.clone();
        let cfg_b = cfg.clone();
        let io_a = io;
        let io_b = io;

        let ha = tokio::spawn(async move {
            SrtSocket::rendezvous_from_socket(
                sock_a,
                format!("127.0.0.1:{port_b}").parse::<SocketAddr>().unwrap(),
                cfg_a,
                io_a,
            )
            .await
        });
        let hb = tokio::spawn(async move {
            SrtSocket::rendezvous_from_socket(
                sock_b,
                format!("127.0.0.1:{port_a}").parse::<SocketAddr>().unwrap(),
                cfg_b,
                io_b,
            )
            .await
        });

        let mut a = ha.await.unwrap().unwrap();
        let mut b = hb.await.unwrap().unwrap();

        // A -> B
        let burst_ab = pattern(PAYLOAD_LEN * 6, 41);
        for chunk in burst_ab.chunks(PAYLOAD_LEN) {
            a.send(chunk).await.expect("send ab");
        }
        let mut received_ab = Vec::new();
        while received_ab.len() < burst_ab.len() {
            let p = b.recv().await.expect("recv").expect("conn ended");
            received_ab.extend_from_slice(&p);
        }
        assert_eq!(received_ab, burst_ab);

        // B -> A
        let burst_ba = pattern(PAYLOAD_LEN * 6, 99);
        for chunk in burst_ba.chunks(PAYLOAD_LEN) {
            b.send(chunk).await.expect("send ba");
        }
        let mut received_ba = Vec::new();
        while received_ba.len() < burst_ba.len() {
            let p = a.recv().await.expect("recv").expect("conn ended");
            received_ba.extend_from_slice(&p);
        }
        assert_eq!(received_ba, burst_ba);

        drop(a);
        drop(b);
    })
    .await
    .unwrap();
}
