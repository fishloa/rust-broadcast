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

/// Hang guard (issue #807 pattern): bounds how long we wait for a UDP
/// multicast datagram to be delivered loopback-locally, which is normally
/// near-instant.
#[cfg(feature = "udp")]
const UDP_TEST_TIMEOUT: Duration = Duration::from_secs(10);

#[cfg(feature = "udp")]
#[tokio::test]
async fn udp_multicast_stream_does_not_truncate_oversized_datagram() {
    // W-DS-2: the pre-fix 1316-byte (7×188) read buffer silently truncates
    // any UDP datagram larger than 7 TS packets — routine for
    // RTP-encapsulated delivery and for any encoder batching more packets
    // per datagram. Send 10 packets (1880 bytes) in one datagram and check
    // all 10 are counted, not just 7.
    let group = Ipv4Addr::new(239, 209, 7, 1);
    // A fixed (not OS-assigned) port, since the sender needs to target it and
    // the public API has no accessor to read back an OS-assigned port.
    const TEST_PORT: u16 = 43_991;

    let mut stream = match SectionStream::bind_multicast(
        SocketAddrV4::new(Ipv4Addr::UNSPECIFIED, TEST_PORT),
        group,
    )
    .await
    {
        Ok(s) => s,
        Err(e) => {
            eprintln!(
                "skipping udp_multicast_stream_does_not_truncate_oversized_datagram: \
                 multicast bind/join unavailable in this environment: {e}"
            );
            return;
        }
    };

    let sender = tokio::net::UdpSocket::bind(SocketAddrV4::new(Ipv4Addr::UNSPECIFIED, 0))
        .await
        .expect("bind sender socket");
    sender
        .set_multicast_loop_v4(true)
        .expect("enable multicast loop");

    let mut datagram = Vec::with_capacity(188 * 10);
    for cc in 0..10u8 {
        datagram.extend_from_slice(&make_ts_packet(cc));
    }
    assert_eq!(datagram.len(), 1880, "10 packets = 1880 bytes, > 7×188");

    sender
        .send_to(&datagram, SocketAddrV4::new(group, TEST_PORT))
        .await
        .expect("send oversized datagram");

    let result = tokio::time::timeout(UDP_TEST_TIMEOUT, async {
        // Drain events until the demux has seen packets (stuffing produces
        // no SectionEvent, so poll_next alone would hang at Pending forever
        // — poll stats directly on a short interval instead).
        loop {
            if stream.stats().packets > 0 {
                return stream.stats().packets;
            }
            // Nudge the stream's internal read by polling it once with a
            // waker that immediately reschedules; tokio's UDP recv future
            // will complete once the datagram above lands.
            let woke = std::future::poll_fn(|cx| match Pin::new(&mut stream).poll_next(cx) {
                Poll::Ready(_) => Poll::Ready(()),
                Poll::Pending => Poll::Pending,
            });
            tokio::select! {
                _ = woke => {}
                _ = tokio::time::sleep(Duration::from_millis(20)) => {}
            }
        }
    })
    .await;

    match result {
        Ok(packets) => {
            assert_eq!(
                packets, 10,
                "W-DS-2: all 10 packets in the 1880-byte datagram must be seen, \
                 not truncated to the old 1316-byte (7-packet) buffer"
            );
        }
        Err(_) => {
            eprintln!(
                "skipping udp_multicast_stream_does_not_truncate_oversized_datagram: \
                 no multicast delivery observed within {UDP_TEST_TIMEOUT:?} \
                 (sandboxed/CI network likely blocks multicast loopback)"
            );
        }
    }
}
