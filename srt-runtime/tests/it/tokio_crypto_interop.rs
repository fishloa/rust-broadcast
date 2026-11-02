#![cfg(all(feature = "tokio", feature = "crypto"))]

use std::net::{SocketAddr, UdpSocket};
use std::process::{Child, Command, Stdio};
use std::time::Duration;

use srt_runtime::Error;
use srt_runtime::handshake_sm::{CryptoConfig, HandshakeConfig};
use srt_runtime::io::{IoConfig, SrtListener, SrtSocket};
use srt_runtime::packet::EncryptionField;
use tokio::net::UdpSocket as TokioUdpSocket;

include!("support/skip.rs");

const TEST_TIMEOUT: Duration = Duration::from_secs(60);
const PAYLOAD_LEN: usize = 1316;
const PASSPHRASE: &str = "0123456789abcdef";
const SALT: [u8; 16] = [0x11u8; 16];

fn free_udp_port() -> u16 {
    UdpSocket::bind("127.0.0.1:0")
        .expect("bind free port")
        .local_addr()
        .unwrap()
        .port()
}

fn pattern(n: usize, seed: u8) -> Vec<u8> {
    (0..n).map(|i| seed.wrapping_add(i as u8)).collect()
}

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
            .expect("spawn"),
    )
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
    listen_port: u16,
    discard_port: u16,
    passphrase: &str,
    pbkeylen: usize,
) -> KillOnDrop {
    let mut cmd = Command::new("srt-live-transmit");
    cmd.arg("-loglevel:error");
    cmd.arg(format!(
        "srt://:{listen_port}?passphrase={passphrase}&pbkeylen={pbkeylen}"
    ));
    cmd.arg(format!("udp://127.0.0.1:{discard_port}"));
    spawn(&mut cmd)
}

#[tokio::test]
async fn tokio_caller_to_libsrt_listener_aes128() {
    tokio::time::timeout(TEST_TIMEOUT, async {
        skip_unless_tools!("srt-live-transmit" => "-version");
        let listen_port = free_udp_port();
        let discard = TokioUdpSocket::bind("127.0.0.1:0").await.unwrap();
        let discard_port = discard.local_addr().unwrap().port();
        let _lib = spawn_libsrt_listener(listen_port, discard_port, PASSPHRASE, 16);
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
        let listen_port = free_udp_port();
        let discard = TokioUdpSocket::bind("127.0.0.1:0").await.unwrap();
        let discard_port = discard.local_addr().unwrap().port();
        let _lib = spawn_libsrt_listener(listen_port, discard_port, PASSPHRASE, 24);
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
        let listen_port = free_udp_port();
        let discard = TokioUdpSocket::bind("127.0.0.1:0").await.unwrap();
        let discard_port = discard.local_addr().unwrap().port();
        let _lib = spawn_libsrt_listener(listen_port, discard_port, PASSPHRASE, 32);
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
        let src_port = free_udp_port();
        let _caller = spawn(
            Command::new("srt-live-transmit")
                .arg("-loglevel:error")
                .arg(format!("-chunk:{PAYLOAD_LEN}"))
                .arg(format!("udp://:{src_port}"))
                .arg(format!(
                    "srt://127.0.0.1:{}?passphrase={}&pbkeylen={}",
                    bound.port(),
                    PASSPHRASE,
                    key_len
                )),
        );
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
        let listen_port = free_udp_port();
        let discard = TokioUdpSocket::bind("127.0.0.1:0").await.unwrap();
        let discard_port = discard.local_addr().unwrap().port();
        let _lib = spawn_libsrt_listener(listen_port, discard_port, PASSPHRASE, 16);
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
        let src_port = free_udp_port();
        let _caller = spawn(
            Command::new("srt-live-transmit")
                .arg("-loglevel:error")
                .arg(format!("-chunk:{PAYLOAD_LEN}"))
                .arg(format!("udp://:{src_port}"))
                .arg(format!(
                    "srt://127.0.0.1:{}?passphrase=wrong-passphrase-xxxxxxxx&pbkeylen=16",
                    bound.port()
                )),
        );
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
        let io = IoConfig::default().with_km_refresh_thresholds(thresholds);
        let mut listener = SrtListener::bind_with("127.0.0.1:0", crypto_config(PASSPHRASE, 16), io)
            .await
            .unwrap();
        let bound = listener.local_addr().unwrap();
        let accept_jh = tokio::spawn(async move { listener.accept().await.unwrap() });
        let mut caller = SrtSocket::connect_with(
            format!("127.0.0.1:{}", bound.port())
                .parse::<SocketAddr>()
                .unwrap(),
            crypto_config(PASSPHRASE, 16),
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
        assert_eq!(
            received.len(),
            80,
            "expected 80 packets, got {}",
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
