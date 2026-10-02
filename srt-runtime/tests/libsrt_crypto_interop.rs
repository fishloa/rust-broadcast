//! Real `libsrt` oracle for the §6 key exchange and payload encryption, over
//! loopback UDP (r08-SRT-W7; issue #1131's "SRT tests are self-vs-self").
//!
//! The in-crate encryption tests only ever exchange bytes between two halves
//! of this crate, so an error both halves share (a wrong `SE` value in the
//! Key Material, a wrong KEK derivation, a wrong AES-CTR IV) passes them all.
//! Here the other half is a genuine `srt-live-transmit` (libsrt):
//!
//! * our [`CallerHandshake`] offers a Key Material to a libsrt listener
//!   (`srt://:PORT?passphrase=..`). libsrt only accepts a KMREQ whose `SE`
//!   equals its own (`hcryptCtx_Rx_ParseKM`, `km_msg[SE] == crypto->se`, MPEG-TS/SRT
//!   = 2): with the old `SE = 0` it answered with a rejection. After the handshake
//!   we encrypt UDP-sized payloads with the negotiated SEK ourselves and libsrt
//!   must decrypt them and forward the exact plaintext.
//! * a libsrt caller (`srt://127.0.0.1:PORT?passphrase=..`) connects to our
//!   [`ListenerHandshake`]; we must unwrap its SEK (proving KEK derivation
//!   matches libsrt's) and decrypt the DATA libsrt encrypts.
//! * a wrong passphrase is refused by libsrt with `REJ_BADSECRET`.
//!
//! The adapter ([`srt_runtime::io`]) refuses encryption, so the sans-IO
//! engines are driven over plain `std::net::UdpSocket`s here with bounded
//! read timeouts (no sleeps used as synchronisation: the handshake retransmits
//! until the libsrt process answers).
//!
//! Skips itself LOUDLY when `srt-live-transmit` is not on `PATH`; set
//! `SRT_REQUIRE_LIBSRT=1` to turn the skip into a failure (CI on a host that
//! must have it).

#![cfg(feature = "crypto")]

use std::io::ErrorKind;
use std::net::{SocketAddr, UdpSocket};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

use srt_runtime::caller::CallerHandshake;
use srt_runtime::crypto::{SALT_LEN, aes_ctr_apply};
use srt_runtime::handshake_sm::{
    CryptoConfig, HandshakeConfig, HandshakeOutput, NegotiatedParams, RejectionReason,
};
use srt_runtime::listener::ListenerHandshake;
use srt_runtime::packet::{
    ControlPacket, DataPacket, EncryptionField, EncryptionKeyField, PacketPosition, SrtPacket,
};

include!("support/skip.rs");

/// libsrt requires a passphrase of 10..=79 characters.
const PASSPHRASE: &str = "0123456789abcdef";
const SEK: [u8; 16] = [
    0x10, 0x21, 0x32, 0x43, 0x54, 0x65, 0x76, 0x87, 0x98, 0xA9, 0xBA, 0xCB, 0xDC, 0xED, 0xFE, 0x0F,
];
const SALT: [u8; SALT_LEN] = [
    0xA0, 0xA1, 0xA2, 0xA3, 0xA4, 0xA5, 0xA6, 0xA7, 0xB0, 0xB1, 0xB2, 0xB3, 0xB4, 0xB5, 0xB6, 0xB7,
];
const PAYLOAD_LEN: usize = 1316;
const PACKETS: u32 = 5;
const HANDSHAKE_DEADLINE: Duration = Duration::from_secs(15);
const RECV_DEADLINE: Duration = Duration::from_secs(15);
/// The handshake retransmit period libsrt itself uses (and this test's read timeout).
const POLL: Duration = Duration::from_millis(250);

/// Kills the wrapped child on drop (panic or early return included).
struct KillOnDrop(Child);

impl Drop for KillOnDrop {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

fn free_udp_port() -> u16 {
    UdpSocket::bind("127.0.0.1:0")
        .expect("bind free port")
        .local_addr()
        .expect("local addr")
        .port()
}

fn payload(index: u32) -> Vec<u8> {
    (0..PAYLOAD_LEN)
        .map(|i| (i as u32).wrapping_mul(7).wrapping_add(index * 31) as u8)
        .collect()
}

fn is_timeout(e: &std::io::Error) -> bool {
    matches!(e.kind(), ErrorKind::WouldBlock | ErrorKind::TimedOut)
}

fn crypto_config(passphrase: &str) -> HandshakeConfig {
    HandshakeConfig {
        encryption_field: EncryptionField::Aes128,
        // One handshake tick per read timeout: retransmit every `POLL`.
        retransmit_after_ticks: 1,
        max_retries: 1_000,
        crypto: Some(CryptoConfig {
            passphrase: passphrase.as_bytes().to_vec(),
            salt: SALT,
            sek: SEK.to_vec(),
        }),
        ..HandshakeConfig::default()
    }
}

enum CallerResult {
    Connected(NegotiatedParams),
    Rejected(RejectionReason),
}

/// Drive `hs` over `sock` to a terminal state, retransmitting on each read
/// timeout, ignoring datagrams from anyone but `peer`.
fn drive_caller(hs: &mut CallerHandshake, sock: &UdpSocket, peer: SocketAddr) -> CallerResult {
    sock.set_read_timeout(Some(POLL)).expect("read timeout");
    let first = hs.start().expect("start");
    sock.send_to(&first, peer).expect("send induction");
    let deadline = Instant::now() + HANDSHAKE_DEADLINE;
    let mut buf = [0u8; 1500];
    while Instant::now() < deadline {
        let outputs = match sock.recv_from(&mut buf) {
            Ok((n, src)) if src == peer => match hs.feed_bytes(&buf[..n]) {
                Ok(o) => o,
                Err(_) => continue,
            },
            Ok(_) => continue,
            Err(e) if is_timeout(&e) => hs.tick(),
            Err(e) => panic!("recv: {e}"),
        };
        for o in outputs {
            match o {
                HandshakeOutput::Send(b) => {
                    sock.send_to(&b, peer).expect("send");
                }
                HandshakeOutput::Connected(p) => return CallerResult::Connected(p),
                HandshakeOutput::Rejected(r) => return CallerResult::Rejected(r),
                HandshakeOutput::TimedOut => panic!("handshake retry budget exhausted"),
                other => panic!("unexpected {other:?}"),
            }
        }
    }
    panic!("handshake with libsrt did not finish within {HANDSHAKE_DEADLINE:?}");
}

fn spawn_libsrt_listener(listen_port: u16, discard_port: u16, passphrase: &str) -> KillOnDrop {
    KillOnDrop(
        Command::new("srt-live-transmit")
            .arg("-loglevel:error")
            .arg(format!(
                "srt://:{listen_port}?passphrase={passphrase}&pbkeylen=16"
            ))
            .arg(format!("udp://127.0.0.1:{discard_port}"))
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .expect("spawn srt-live-transmit listener"),
    )
}

/// Our caller offers a KM; libsrt must accept it (SE = MPEG-TS/SRT) and then
/// decrypt DATA we encrypt with the negotiated SEK.
#[test]
fn libsrt_listener_accepts_our_key_material_and_decrypts_our_data() {
    skip_unless_tools!("srt-live-transmit" => "-version");

    let listen_port = free_udp_port();
    let discard = UdpSocket::bind("127.0.0.1:0").expect("bind discard");
    let discard_port = discard.local_addr().unwrap().port();
    discard.set_read_timeout(Some(RECV_DEADLINE)).unwrap();
    let _libsrt = spawn_libsrt_listener(listen_port, discard_port, PASSPHRASE);

    let sock = UdpSocket::bind("127.0.0.1:0").expect("bind caller socket");
    let peer: SocketAddr = format!("127.0.0.1:{listen_port}").parse().unwrap();
    let config = crypto_config(PASSPHRASE);
    let isn = 0x0123_4567 & 0x7FFF_FFFF;
    let mut hs = CallerHandshake::new(
        0x0BAD_CAFE,
        HandshakeConfig {
            initial_seq_number: isn,
            ..config
        },
    );
    let negotiated = match drive_caller(&mut hs, &sock, peer) {
        CallerResult::Connected(p) => p,
        CallerResult::Rejected(r) => panic!(
            "libsrt rejected our Key Material: {r} (a KMREQ with SE != MPEG-TS/SRT is refused)"
        ),
    };
    assert_eq!(negotiated.sek.as_deref(), Some(SEK.as_slice()));

    let started = Instant::now();
    for index in 0..PACKETS {
        let seq = isn + index;
        let mut data = payload(index);
        aes_ctr_apply(&SEK, &SALT, seq, &mut data).expect("encrypt");
        let pkt = DataPacket {
            seq_number: seq,
            position: PacketPosition::Solo,
            in_order: true,
            key_flag: EncryptionKeyField::Even,
            retransmitted: false,
            message_number: index + 1,
            timestamp: u32::try_from(started.elapsed().as_micros()).unwrap(),
            dest_socket_id: negotiated.peer_socket_id,
            data: &data,
        };
        let mut wire = vec![0u8; pkt.serialized_len()];
        pkt.serialize_into(&mut wire).unwrap();
        sock.send_to(&wire, peer).expect("send data");
    }

    let mut buf = [0u8; 2048];
    for index in 0..PACKETS {
        let n = discard.recv(&mut buf).unwrap_or_else(|e| {
            panic!("libsrt forwarded only {index} of {PACKETS} payloads before {e}")
        });
        assert_eq!(
            &buf[..n],
            payload(index).as_slice(),
            "payload {index}: libsrt did not decrypt to our plaintext"
        );
    }
}

/// A wrong passphrase on our side: libsrt answers with `REJ_BADSECRET`.
#[test]
fn libsrt_listener_rejects_a_wrong_passphrase_with_badsecret() {
    skip_unless_tools!("srt-live-transmit" => "-version");

    let listen_port = free_udp_port();
    let discard = UdpSocket::bind("127.0.0.1:0").expect("bind discard");
    let discard_port = discard.local_addr().unwrap().port();
    let _libsrt = spawn_libsrt_listener(listen_port, discard_port, PASSPHRASE);

    let sock = UdpSocket::bind("127.0.0.1:0").expect("bind caller socket");
    let peer: SocketAddr = format!("127.0.0.1:{listen_port}").parse().unwrap();
    let mut hs = CallerHandshake::new(0x0BAD_CAFE, crypto_config("a-different-passphrase"));
    match drive_caller(&mut hs, &sock, peer) {
        CallerResult::Rejected(reason) => assert_eq!(reason, RejectionReason::BadSecret),
        CallerResult::Connected(_) => panic!("libsrt accepted a Key Material of the wrong secret"),
    }
}

/// A libsrt caller connects to our listener engine: we recover its SEK from
/// its KMREQ (KEK derivation agrees with libsrt's) and decrypt the DATA it
/// encrypts.
#[test]
fn our_listener_recovers_libsrts_sek_and_decrypts_its_data() {
    skip_unless_tools!("srt-live-transmit" => "-version");

    let sock = UdpSocket::bind("127.0.0.1:0").expect("bind listener socket");
    let port = sock.local_addr().unwrap().port();
    sock.set_read_timeout(Some(POLL)).unwrap();

    let src_port = free_udp_port();
    let _libsrt = KillOnDrop(
        Command::new("srt-live-transmit")
            .arg("-loglevel:error")
            .arg(format!("-chunk:{PAYLOAD_LEN}"))
            .arg(format!("udp://:{src_port}"))
            .arg(format!(
                "srt://127.0.0.1:{port}?passphrase={PASSPHRASE}&pbkeylen=16"
            ))
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .expect("spawn srt-live-transmit caller"),
    );

    let mut hs = ListenerHandshake::new(
        0x0C0F_FEE0,
        0x1357_9BDF,
        HandshakeConfig {
            crypto: Some(CryptoConfig {
                passphrase: PASSPHRASE.as_bytes().to_vec(),
                salt: [0; SALT_LEN],
                sek: Vec::new(),
            }),
            encryption_field: EncryptionField::Aes128,
            ..HandshakeConfig::default()
        },
    );

    let deadline = Instant::now() + HANDSHAKE_DEADLINE;
    let mut buf = [0u8; 2048];
    let mut negotiated: Option<NegotiatedParams> = None;
    let mut libsrt_addr: Option<SocketAddr> = None;
    let feeder = UdpSocket::bind("127.0.0.1:0").expect("bind feeder");
    // Indexes of the payloads libsrt delivered (decrypted). `srt-live-transmit`
    // discards what its UDP source sees until its SRT side has connected, so
    // numbered payloads are fed one per poll interval until enough have come
    // out; those fed too early simply never arrive.
    let mut fed = 0u32;
    let mut received: Vec<u32> = Vec::new();

    while Instant::now() < deadline && received.len() < PACKETS as usize {
        let (n, src) = match sock.recv_from(&mut buf) {
            Ok(v) => v,
            Err(e) if is_timeout(&e) => {
                if negotiated.is_some() {
                    feeder
                        .send_to(&payload(fed), ("127.0.0.1", src_port))
                        .expect("feed libsrt");
                    fed += 1;
                }
                continue;
            }
            Err(e) => panic!("recv: {e}"),
        };
        if libsrt_addr.is_some_and(|a| a != src) {
            continue;
        }
        match SrtPacket::parse(&buf[..n]) {
            Ok(SrtPacket::Control(ControlPacket::Handshake(_))) if negotiated.is_none() => {
                libsrt_addr = Some(src);
                let outputs = match hs.feed_bytes(&buf[..n]) {
                    Ok(o) => o,
                    Err(_) => continue,
                };
                for o in outputs {
                    match o {
                        HandshakeOutput::Send(b) => {
                            sock.send_to(&b, src).expect("send");
                        }
                        HandshakeOutput::Connected(p) => {
                            negotiated = Some(p);
                        }
                        HandshakeOutput::Rejected(r) => panic!("our listener rejected libsrt: {r}"),
                        other => panic!("unexpected {other:?}"),
                    }
                }
            }
            Ok(SrtPacket::Data(d)) => {
                let p = negotiated
                    .as_ref()
                    .expect("DATA before the handshake finished");
                assert_ne!(
                    d.key_flag,
                    EncryptionKeyField::NotEncrypted,
                    "libsrt must encrypt once a passphrase is set"
                );
                let sek = p.sek.as_deref().expect("negotiated SEK");
                let salt = p.salt.expect("negotiated salt");
                let mut data = d.data.to_vec();
                aes_ctr_apply(sek, &salt, d.seq_number, &mut data).expect("decrypt");
                let index = (0..fed)
                    .find(|&i| payload(i) == data)
                    .unwrap_or_else(|| panic!("decrypted DATA matches no payload we fed"));
                received.push(index);
            }
            _ => {}
        }
    }

    assert!(
        negotiated.is_some(),
        "handshake with libsrt never completed"
    );
    assert_eq!(received.len(), PACKETS as usize, "DATA from libsrt");
    assert!(
        received.windows(2).all(|w| w[1] == w[0] + 1),
        "payloads decrypted with the negotiated SEK must arrive in order, got {received:?}"
    );
}
