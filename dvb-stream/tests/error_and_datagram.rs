//! Regression tests for the warning sweep (issue #1099):
//! - W-DS-1: an I/O error from the reader must be distinguishable from a
//!   clean EOF, not silently turned into `Poll::Ready(None)`.
//! - W-DS-2: a `bind_multicast`-backed stream must not silently truncate an
//!   oversized UDP datagram, and must not stitch a trailing partial packet
//!   from one datagram onto the next, unrelated one.

#[cfg(feature = "udp")]
use std::net::{Ipv4Addr, SocketAddrV4};
use std::pin::Pin;
use std::task::{Context, Poll};
#[cfg(feature = "udp")]
use std::time::Duration;

#[cfg(feature = "udp")]
use dvb_stream::UdpSectionStream;
#[cfg(feature = "udp")]
use dvb_stream::udp::MulticastConfig;
use dvb_stream::{SectionStream, T2miEventStream};
use futures_core::Stream;
use tokio::io::{AsyncRead, ReadBuf};

/// A reader that yields one `ErrorKind::ConnectionReset` error and nothing
/// else — models a dropped TCP connection or a device error mid-stream.
struct FailingReader;

impl AsyncRead for FailingReader {
    fn poll_read(
        self: Pin<&mut Self>,
        _cx: &mut Context<'_>,
        _buf: &mut ReadBuf<'_>,
    ) -> Poll<std::io::Result<()>> {
        Poll::Ready(Err(std::io::Error::new(
            std::io::ErrorKind::ConnectionReset,
            "connection reset by peer",
        )))
    }
}

#[tokio::test]
async fn section_stream_distinguishes_io_error_from_eof() {
    let mut stream = SectionStream::new(FailingReader);

    let item = std::future::poll_fn(|cx| Pin::new(&mut stream).poll_next(cx)).await;
    assert!(item.is_none(), "stream must end on a reader error");

    let err = stream
        .take_io_error()
        .expect("W-DS-1: the ConnectionReset error must be retrievable, not silently discarded");
    assert_eq!(err.kind(), std::io::ErrorKind::ConnectionReset);

    // A second take returns None — the error was consumed, not duplicated.
    assert!(stream.take_io_error().is_none());
}

/// Drift pin (#1141): `T2miEventStream` and `SectionStream` now share one
/// `TsFramer`, so the W-DS-1 error/EOF distinction must hold through the
/// T2-MI call site too (it was a separately-maintained copy).
#[tokio::test]
async fn t2mi_stream_distinguishes_io_error_from_eof() {
    let mut stream = T2miEventStream::new(FailingReader, 0x0006);

    let item = std::future::poll_fn(|cx| Pin::new(&mut stream).poll_next(cx)).await;
    assert!(item.is_none(), "stream must end on a reader error");

    let err = stream
        .take_io_error()
        .expect("the ConnectionReset error must be retrievable");
    assert_eq!(err.kind(), std::io::ErrorKind::ConnectionReset);
    assert!(stream.take_io_error().is_none());
}

/// Builds one syntactically-valid TS packet (stuffing table_id on PID 0),
/// distinct per `cc` so consecutive packets are not byte-identical.
#[cfg(feature = "udp")]
fn make_ts_packet(cc: u8) -> [u8; 188] {
    let mut pkt = [0xFFu8; 188];
    pkt[0] = 0x47;
    pkt[1] = 0x40; // PUSI, PID hi = 0
    pkt[2] = 0x00; // PID lo = 0 (PAT PID; contents are stuffing, not a real PAT)
    pkt[3] = 0x10 | (cc & 0x0F); // payload only, continuity_counter
    pkt[4] = 0x00; // pointer_field = 0
    pkt[5] = 0xFF; // table_id 0xFF: stuffing, completes no section
    pkt
}

/// A valid PAT section (real CRC-32/MPEG-2) in one TS packet on PID 0 (the demux emits one event).
#[cfg(feature = "udp")]
fn pat_packet() -> [u8; 188] {
    let mut section = vec![
        0x00, 0xB0, 0x0D, 0x00, 0x01, 0xC1, 0x00, 0x00, 0x00, 0x01, 0xE0, 0x20,
    ];
    let crc = broadcast_common::crc32_mpeg2::compute(&section);
    section.extend_from_slice(&crc.to_be_bytes());
    let mut pkt = [0xFFu8; 188];
    pkt[..5].copy_from_slice(&[0x47, 0x40, 0x00, 0x10, 0x00]);
    pkt[5..5 + section.len()].copy_from_slice(&section);
    pkt
}

/// Hang guard (issue #807 pattern): bounds how long we wait for a UDP
/// multicast datagram to be delivered loopback-locally, which is normally
/// near-instant.
#[cfg(feature = "udp")]
const UDP_TEST_TIMEOUT: Duration = Duration::from_secs(10);

/// Multicast variant of the W-DS-2 regression: `MulticastConfig` on an OS-assigned port (no fixed
/// port shared between parallel test runs), the port read back from the bound socket.
/// Skips cleanly where the sandbox has no multicast bind/join/loopback.
#[cfg(feature = "udp")]
#[tokio::test]
async fn udp_multicast_stream_does_not_truncate_oversized_datagram() {
    // W-DS-2: the pre-fix 1316-byte (7x188) read buffer silently truncates
    // any UDP datagram larger than 7 TS packets. Send 10 packets (1880 bytes)
    // in one datagram and check all 10 are counted, not just 7.
    let group = Ipv4Addr::new(239, 209, 7, 1);
    let config = MulticastConfig::new(SocketAddrV4::new(Ipv4Addr::UNSPECIFIED, 0), group);
    let std_socket = match config.bind() {
        Ok(s) => s,
        Err(e) => {
            eprintln!(
                "skipping udp_multicast_stream_does_not_truncate_oversized_datagram: \
                 multicast bind/join unavailable in this environment: {e}"
            );
            return;
        }
    };
    let port = std_socket.local_addr().expect("local addr").port();
    let mut stream = UdpSectionStream::from_socket(
        tokio::net::UdpSocket::from_std(std_socket).expect("tokio socket"),
    );

    let sender = tokio::net::UdpSocket::bind(SocketAddrV4::new(Ipv4Addr::UNSPECIFIED, 0))
        .await
        .expect("bind sender socket");
    sender
        .set_multicast_loop_v4(true)
        .expect("enable multicast loop");

    let mut datagram = Vec::with_capacity(188 * 10);
    for cc in 0..9u8 {
        datagram.extend_from_slice(&make_ts_packet(cc));
    }
    datagram.extend_from_slice(&pat_packet()); // 10th packet: yields the event we await
    assert_eq!(datagram.len(), 1880, "10 packets = 1880 bytes, > 7x188");
    // Genuinely-unavailable multicast is detected UP FRONT (no bind/join above, no route to the
    // group here), never inferred from a timeout.
    if let Err(e) = sender
        .send_to(&datagram, SocketAddrV4::new(group, port))
        .await
    {
        eprintln!(
            "skipping udp_multicast_stream_does_not_truncate_oversized_datagram: \
             no multicast route in this environment: {e}"
        );
        return;
    }

    // Hang guard (issue #807 pattern): the PAT (10th packet) produces a SectionEvent, so awaiting
    // the next event, not a fixed delay, proves all 10 packets were demuxed. Multicast was
    // verified available above, so a timeout here is a FAILURE (the 7-packet truncation
    // regression this test exists for would show up exactly as a missing PAT).
    tokio::time::timeout(
        UDP_TEST_TIMEOUT,
        std::future::poll_fn(|cx| Pin::new(&mut stream).poll_next(cx)),
    )
    .await
    .expect("W-DS-2: the 10th packet (PAT) of the 1880-byte datagram never arrived: truncated?");
    assert_eq!(
        stream.stats().packets,
        10,
        "W-DS-2: all 10 packets in the 1880-byte datagram must be seen"
    );
}
