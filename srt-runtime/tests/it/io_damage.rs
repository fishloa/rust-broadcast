//! Damage reporting through the tokio adapter: a run of packets the peer never
//! delivers is skipped by TSBPD (TLPKTDROP), the application learns what it
//! lost, and the caller's policy decides whether the connection survives it.
//!
//! Real loopback UDP through a relay that drops every attempt at a chosen
//! sequence run (the original and every retransmission), so TSBPD has no
//! choice but to skip the gap; every test is wrapped in a timeout so a
//! regression fails fast instead of hanging.

#![cfg(feature = "tokio")]

use core::time::Duration;
use std::net::SocketAddr;
use std::sync::{Arc, Mutex};

use broadcast_core::damage::{DamageKind, DamagePolicy, Severity};
use srt_runtime::HandshakeConfig;
use srt_runtime::io::{SrtEvent, SrtListener, SrtSocket, SrtSocketConfig};
use srt_runtime::packet::SrtPacket;

const NUM_PAYLOADS: usize = 40;
/// The run of sequence offsets, relative to the first DATA packet, that the
/// relay never forwards.
const DROPPED_RUN: core::ops::Range<usize> = 10..14;
const TEST_TIMEOUT: Duration = Duration::from_secs(30);
/// TSBPD latency: long enough that a retransmission could have made it, short
/// enough that skipping the gap does not make the test slow.
const LATENCY_MS: u64 = 120;

/// A relay that drops the whole `DROPPED_RUN` (first transmissions and every
/// retransmission) and forwards everything else both ways.
async fn run_relay(
    client_side: tokio::net::UdpSocket,
    server_side: tokio::net::UdpSocket,
    listener_addr: SocketAddr,
    base_seq: Arc<Mutex<Option<u32>>>,
) {
    let mut caller_addr: Option<SocketAddr> = None;
    let mut from_caller = [0u8; 2048];
    let mut from_listener = [0u8; 2048];
    loop {
        tokio::select! {
            res = client_side.recv_from(&mut from_caller) => {
                let Ok((len, src)) = res else { break };
                caller_addr = Some(src);
                let bytes = &from_caller[..len];
                let drop = match SrtPacket::parse(bytes) {
                    Ok(SrtPacket::Data(d)) => {
                        let mut base = base_seq.lock().expect("base seq");
                        let base = *base.get_or_insert(d.seq_number);
                        let idx = d.seq_number.wrapping_sub(base) as usize;
                        DROPPED_RUN.contains(&idx)
                    }
                    _ => false,
                };
                if drop {
                    continue;
                }
                let _ = server_side.send_to(bytes, listener_addr).await;
            }
            res = server_side.recv_from(&mut from_listener) => {
                let Ok((len, _src)) = res else { break };
                if let Some(caller) = caller_addr {
                    let _ = client_side.send_to(&from_listener[..len], caller).await;
                }
            }
        }
    }
}

struct Rig {
    caller: SrtSocket,
    receiver: SrtSocket,
    relay: tokio::task::JoinHandle<()>,
}

async fn connect(policy: DamagePolicy) -> Rig {
    let listener_bind: SocketAddr = "127.0.0.1:0".parse().expect("addr");
    let listener_config = SrtSocketConfig::new(
        HandshakeConfig::default().with_latency(Duration::from_millis(LATENCY_MS)),
    )
    .with_damage_policy(policy);
    let mut listener = SrtListener::bind(listener_bind, listener_config)
        .await
        .expect("listener bind");
    let listener_addr = listener.local_addr().expect("listener addr");

    let client_side = tokio::net::UdpSocket::bind("127.0.0.1:0")
        .await
        .expect("relay client bind");
    let relay_addr = client_side.local_addr().expect("relay addr");
    let server_side = tokio::net::UdpSocket::bind("127.0.0.1:0")
        .await
        .expect("relay server bind");
    let base_seq = Arc::new(Mutex::new(None));
    let relay = tokio::spawn(run_relay(
        client_side,
        server_side,
        listener_addr,
        Arc::clone(&base_seq),
    ));

    let accept = tokio::spawn(async move { listener.accept().await.expect("accept") });
    let caller_config = SrtSocketConfig::new(
        HandshakeConfig::default().with_latency(Duration::from_millis(LATENCY_MS)),
    );
    let caller = SrtSocket::connect(relay_addr, caller_config)
        .await
        .expect("caller connect");
    let receiver = accept.await.expect("join accept");
    Rig {
        caller,
        receiver,
        relay,
    }
}

fn payload(i: usize) -> Vec<u8> {
    (0..40).map(|j| (i as u8).wrapping_add(j as u8)).collect()
}

#[tokio::test]
async fn a_skipped_packet_run_reaches_the_player_as_one_report_and_playback_continues() {
    tokio::time::timeout(TEST_TIMEOUT, async {
        let mut rig = connect(DamagePolicy::new()).await;
        let send = tokio::spawn(async move {
            for i in 0..NUM_PAYLOADS {
                rig.caller.send(&payload(i)).await.expect("send");
                tokio::task::yield_now().await;
            }
            rig
        });
        // The receiver keeps reading while the sender runs, exactly as a
        // player does.
        let mut rig = send.await.expect("join sender");
        let mut damages = Vec::new();
        let mut payloads = 0usize;
        while payloads < NUM_PAYLOADS - DROPPED_RUN.len() {
            match rig.receiver.recv_event().await.expect("recv") {
                Some(SrtEvent::Payload(_)) => payloads = payloads.saturating_add(1),
                Some(SrtEvent::Damage(damage)) => damages.push(damage),
                Some(other) => panic!("unexpected event: {other:?}"),
                None => break,
            }
        }
        rig.relay.abort();

        assert_eq!(
            damages.len(),
            1,
            "the dropped run is one report, not one per packet: {damages:?}"
        );
        let damage = &damages[0];
        assert_eq!(damage.kind(), DamageKind::MediaUnitDropped);
        assert_eq!(damage.severity(), Severity::Degraded);
        assert!(
            damage.location().ordinal().is_some(),
            "the report names the run"
        );
        assert!(
            payloads > DROPPED_RUN.len(),
            "playback continued past the skipped run ({payloads} payloads)"
        );
    })
    .await
    .expect("the lossy session must finish within the timeout");
}

#[tokio::test]
async fn the_fail_policy_ends_the_same_session_with_the_same_report() {
    tokio::time::timeout(TEST_TIMEOUT, async {
        let rig = connect(DamagePolicy::new().fail_on(Severity::Degraded)).await;
        let mut caller = rig.caller;
        let mut receiver = rig.receiver;
        let relay = rig.relay;
        let stop = tokio::spawn(async move {
            for i in 0..NUM_PAYLOADS {
                if caller.send(&payload(i)).await.is_err() {
                    break;
                }
                tokio::task::yield_now().await;
            }
            caller
        });
        let error = loop {
            match receiver.recv_event().await {
                Ok(Some(_)) => {}
                Ok(None) => panic!("the connection ended without reporting the loss"),
                Err(error) => break error,
            }
        };
        relay.abort();
        let _caller = stop.await.expect("join sender");
        let srt_runtime::Error::Damage(damage_error) = error else {
            panic!("expected the policy stop, got {error}");
        };
        assert_eq!(damage_error.damage().kind(), DamageKind::MediaUnitDropped);
        assert_eq!(damage_error.damage().severity(), Severity::Degraded);
    })
    .await
    .expect("the failing session must finish within the timeout");
}

/// The relay never drops here: a clean run reports nothing.
#[tokio::test]
async fn a_clean_session_reports_nothing() {
    tokio::time::timeout(TEST_TIMEOUT, async {
        let mut clean = connect_clean().await;
        for i in 0..NUM_PAYLOADS {
            clean.caller.send(&payload(i)).await.expect("send");
            tokio::task::yield_now().await;
        }
        let mut got = 0usize;
        while got < NUM_PAYLOADS {
            match clean.receiver.recv_event().await.expect("recv") {
                Some(SrtEvent::Payload(_)) => got = got.saturating_add(1),
                Some(SrtEvent::Damage(damage)) => {
                    panic!("a clean session reported damage: {damage}")
                }
                Some(other) => panic!("unexpected event: {other:?}"),
                None => break,
            }
        }
        clean.relay.abort();
        assert_eq!(got, NUM_PAYLOADS);
    })
    .await
    .expect("the clean session must finish within the timeout");
}

/// A rig whose relay forwards every datagram.
async fn connect_clean() -> Rig {
    let listener_bind: SocketAddr = "127.0.0.1:0".parse().expect("addr");
    let listener_config = SrtSocketConfig::new(
        HandshakeConfig::default().with_latency(Duration::from_millis(LATENCY_MS)),
    );
    let mut listener = SrtListener::bind(listener_bind, listener_config)
        .await
        .expect("listener bind");
    let listener_addr = listener.local_addr().expect("listener addr");
    let client_side = tokio::net::UdpSocket::bind("127.0.0.1:0")
        .await
        .expect("relay client bind");
    let relay_addr = client_side.local_addr().expect("relay addr");
    let server_side = tokio::net::UdpSocket::bind("127.0.0.1:0")
        .await
        .expect("relay server bind");
    let relay = tokio::spawn(async move {
        let mut caller_addr: Option<SocketAddr> = None;
        let mut a = [0u8; 2048];
        let mut b = [0u8; 2048];
        loop {
            tokio::select! {
                r = client_side.recv_from(&mut a) => {
                    let Ok((len, src)) = r else { break };
                    caller_addr = Some(src);
                    let _ = server_side.send_to(&a[..len], listener_addr).await;
                }
                r = server_side.recv_from(&mut b) => {
                    let Ok((len, _)) = r else { break };
                    if let Some(c) = caller_addr {
                        let _ = client_side.send_to(&b[..len], c).await;
                    }
                }
            }
        }
    });
    let accept = tokio::spawn(async move { listener.accept().await.expect("accept") });
    let caller = SrtSocket::connect(
        relay_addr,
        SrtSocketConfig::new(
            HandshakeConfig::default().with_latency(Duration::from_millis(LATENCY_MS)),
        ),
    )
    .await
    .expect("caller connect");
    let receiver = accept.await.expect("join accept");
    Rig {
        caller,
        receiver,
        relay,
    }
}
