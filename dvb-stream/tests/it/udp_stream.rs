//! UDP input through `UdpFramed` + `socket2` binds (SP1.1, SP1.6). Every port is
//! OS-assigned (port 0); no fixed ports, no sleeps.
#![cfg(feature = "udp")]

use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr, SocketAddrV4};
use std::pin::Pin;
use std::time::Duration;

use broadcast_common::net::{MulticastInterface, MulticastJoin, UdpOptions};
use dvb_stream::{UdpSectionStream, UdpT2miStream};
use futures_core::Stream;
use mpeg_ts::pid::Pid;
use test_fixtures::psi::pat_packet;

fn pkt(cc: u8) -> [u8; 188] {
    let mut p = [0xFFu8; 188];
    p[0] = 0x47;
    p[1] = 0x1F; // null PID 0x1FFF stuffing: counted by the demux, no section event
    p[2] = 0xFF;
    p[3] = 0x10 | (cc & 0x0F);
    p
}

/// Hang guard (issue #807 pattern): waits for the next `SectionEvent`, which the PAT packet
/// produces once the demux has been fed every packet before it.
async fn next_event(stream: &mut UdpSectionStream) {
    tokio::time::timeout(
        Duration::from_secs(20),
        std::future::poll_fn(|cx| Pin::new(&mut *stream).poll_next(cx)),
    )
    .await
    .expect("event must arrive")
    .expect("stream must not end");
}

/// One non-blocking poll: the stream must be idle (nothing more to deliver) afterwards.
async fn poll_once_is_pending(stream: &mut UdpSectionStream) -> bool {
    std::future::poll_fn(|cx| {
        std::task::Poll::Ready(Pin::new(&mut *stream).poll_next(cx).is_pending())
    })
    .await
}

/// 9 stuffing packets + a PAT packet (1880 B, larger than 7x188) plus a 5-byte tail in ONE
/// datagram, over plain unicast loopback on an ephemeral port (`from_socket`), so it needs no
/// multicast support.
#[tokio::test]
async fn an_oversized_datagram_is_fully_seen_and_its_stray_tail_is_dropped() {
    let rx = tokio::net::UdpSocket::bind((Ipv4Addr::LOCALHOST, 0))
        .await
        .unwrap();
    let addr = rx.local_addr().unwrap();
    let mut stream = UdpSectionStream::from_socket(rx);
    let tx = tokio::net::UdpSocket::bind((Ipv4Addr::LOCALHOST, 0))
        .await
        .unwrap();
    let mut datagram = Vec::new();
    for cc in 0..9u8 {
        datagram.extend_from_slice(&pkt(cc));
    }
    datagram.extend_from_slice(&pat_packet());
    datagram.extend_from_slice(&[0x47, 1, 2, 3, 4]);
    tx.send_to(&datagram, addr).await.unwrap();
    next_event(&mut stream).await; // the PAT is the 10th packet
    assert_eq!(stream.stats().packets, 10);
    // Polling on lets the framer reach the end of the datagram, where the 5-byte tail is dropped.
    assert!(poll_once_is_pending(&mut stream).await);
    assert_eq!(stream.resync_stats().bytes_discarded, 5);
}

/// A junk-only datagram between two good ones does not poison either neighbour.
#[tokio::test]
async fn a_junk_datagram_between_good_ones_is_isolated() {
    let rx = tokio::net::UdpSocket::bind((Ipv4Addr::LOCALHOST, 0))
        .await
        .unwrap();
    let addr = rx.local_addr().unwrap();
    let mut stream = UdpSectionStream::from_socket(rx);
    let tx = tokio::net::UdpSocket::bind((Ipv4Addr::LOCALHOST, 0))
        .await
        .unwrap();
    tx.send_to(&pkt(0), addr).await.unwrap();
    tx.send_to(&[0u8; 700], addr).await.unwrap();
    tx.send_to(&pat_packet(), addr).await.unwrap();
    next_event(&mut stream).await;
    assert_eq!(stream.stats().packets, 2);
    assert_eq!(stream.resync_stats().bytes_discarded, 700);
}

/// `UdpT2miStream` must frame the T2-MI TS packets inside a datagram, yield
/// their events, and report the counters of what it actually did — an
/// always-`Pending` or zeroed-stats stream would leave every UDP T2-MI
/// consumer silently empty (audit a23cp #3434).
#[tokio::test]
async fn udp_t2mi_stream_frames_datagrams_and_reports_stats() {
    const T2MI_PID: Pid = Pid::from_const(0x0040);
    const JUNK: usize = 5;
    const DATAGRAM_PACKETS: usize = 40;
    const DATAGRAMS: usize = 20;
    const MAX_POLLS: usize = 4000;

    let fixture = test_fixtures::read("t2mi/colombia-capital-t2mi.ts");
    let sent_packets = DATAGRAM_PACKETS * DATAGRAMS;
    assert!(
        fixture.len() >= sent_packets * 188,
        "fixture must hold the packets sent"
    );

    let rx = tokio::net::UdpSocket::bind((Ipv4Addr::LOCALHOST, 0))
        .await
        .unwrap();
    let addr = rx.local_addr().unwrap();
    let mut stream = UdpT2miStream::from_socket(rx, T2MI_PID);
    let tx = tokio::net::UdpSocket::bind((Ipv4Addr::LOCALHOST, 0))
        .await
        .unwrap();
    for index in 0..DATAGRAMS {
        let mut datagram = Vec::new();
        if index == 0 {
            datagram.extend_from_slice(&[0u8; JUNK]);
        }
        datagram.extend_from_slice(
            &fixture[index * DATAGRAM_PACKETS * 188..(index + 1) * DATAGRAM_PACKETS * 188],
        );
        tx.send_to(&datagram, addr).await.unwrap();
    }

    // Wait for the first T2-MI packet to reassemble (the first datagrams also
    // carry the PAT/PMT, so it is not in the very first packets).
    let mut events = 0usize;
    tokio::time::timeout(
        Duration::from_secs(20),
        std::future::poll_fn(|cx| Pin::new(&mut stream).poll_next(cx)),
    )
    .await
    .expect("a T2-MI event must arrive")
    .expect("stream must not end");
    events += 1;

    // Drain the rest (bounded): the pump feeds one TS packet per frame poll,
    // so the counters are only complete once every queued datagram is framed.
    for _ in 0..MAX_POLLS {
        let polled =
            std::future::poll_fn(|cx| std::task::Poll::Ready(Pin::new(&mut stream).poll_next(cx)))
                .await;
        match polled {
            std::task::Poll::Ready(Some(_)) => events += 1,
            std::task::Poll::Ready(None) | std::task::Poll::Pending => break,
        }
    }

    assert!(
        events >= 1,
        "the datagrams carry T2-MI packets, got {events}; pump {:?} resync {:?}",
        stream.stats(),
        stream.resync_stats()
    );
    assert_eq!(
        stream.stats().ts_packets,
        sent_packets as u64,
        "every framed TS packet reaches the pump"
    );
    assert_eq!(
        stream.resync_stats().bytes_discarded,
        JUNK as u64,
        "the junk prefix is the only thing discarded"
    );
}

/// `check_udp` refuses a cross-family join and a join its own options reject
/// before any socket is opened, and accepts a matching-family multicast join
/// (audit a23cp #3437).
#[test]
fn check_udp_accepts_a_matching_join_and_refuses_the_rest() {
    let bind_addr = SocketAddr::V4(SocketAddrV4::new(Ipv4Addr::UNSPECIFIED, 0));

    let v4_join = MulticastJoin::new(IpAddr::V4(Ipv4Addr::new(239, 255, 42, 1)))
        .with_interface(MulticastInterface::V4(Ipv4Addr::UNSPECIFIED));
    assert!(
        dvb_stream::udp::check_udp(bind_addr, &UdpOptions::new().with_join(v4_join)).is_ok(),
        "an IPv4 bind joining an IPv4 multicast group is accepted"
    );

    let v6_join = MulticastJoin::new(IpAddr::V6(Ipv6Addr::new(0xff02, 0, 0, 0, 0, 0, 0, 1)));
    let error = dvb_stream::udp::check_udp(bind_addr, &UdpOptions::new().with_join(v6_join))
        .expect_err("an IPv6 group cannot be joined from an IPv4 bind");
    assert_eq!(error.kind(), std::io::ErrorKind::InvalidInput);

    let unicast_join = MulticastJoin::new(IpAddr::V4(Ipv4Addr::new(192, 0, 2, 1)))
        .with_interface(MulticastInterface::V4(Ipv4Addr::UNSPECIFIED));
    let error = dvb_stream::udp::check_udp(bind_addr, &UdpOptions::new().with_join(unicast_join))
        .expect_err("a join group that is not multicast is refused");
    assert_eq!(error.kind(), std::io::ErrorKind::InvalidInput);
}

#[test]
fn udp_options_apply_the_socket_options() {
    // 64 KiB is below the default `SO_RCVBUF` of both macOS (~768 KiB) and Linux (~208 KiB),
    // so the assertion discriminates "option applied" from "OS default" on either.
    const REQUESTED: usize = 64 * 1024;
    let requested = std::num::NonZeroU32::new(u32::try_from(REQUESTED).unwrap()).unwrap();
    let bind_addr = SocketAddrV4::new(Ipv4Addr::UNSPECIFIED, 0);
    let join = MulticastJoin::new(IpAddr::V4(Ipv4Addr::new(239, 255, 42, 1)))
        .with_interface(MulticastInterface::V4(*bind_addr.ip()));
    let cfg = UdpOptions::new().with_join(join);
    assert!(
        !cfg.reuse_address && !cfg.reuse_port,
        "default matches the plain UdpSocket::bind it replaces (no address reuse)"
    );
    let cfg = cfg
        .with_recv_buffer_bytes(requested)
        .with_reuse_address(true);
    // Join may be refused in a sandbox with no multicast route: that is an environment limit,
    // not a failure of the option wiring, so only the Ok path asserts.
    match dvb_stream::udp::bind_udp(SocketAddr::V4(bind_addr), &cfg) {
        Ok(sock) => {
            let s = socket2::SockRef::from(&sock);
            let applied = s.recv_buffer_size().unwrap();
            let plain = std::net::UdpSocket::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
            let default = socket2::SockRef::from(&plain).recv_buffer_size().unwrap();
            assert!(
                applied >= REQUESTED / 2 && applied < default,
                "SO_RCVBUF applied: requested {REQUESTED}, got {applied}, OS default {default}"
            );
            assert!(
                s.reuse_address().unwrap(),
                "SO_REUSEADDR applied when requested"
            );
            assert!(sock.local_addr().unwrap().as_socket().unwrap().port() != 0);
        }
        Err(e) => eprintln!("skipping udp_options_apply_the_socket_options: {e}"),
    }
}
