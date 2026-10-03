//! Differential tests for T2miEventStream vs T2miPump sync oracle.

use std::pin::Pin;
use std::time::Duration;

use dvb_stream::T2miEventStream;
use dvb_t2mi::pump::T2miPump;
use futures_core::stream::Stream;

// ── oracle ────────────────────────────────────────────────────────────────────

/// Drive T2miPump synchronously on 188-byte-aligned data and collect all events.
fn t2mi_sync_oracle(data: &[u8], pid: u16) -> Vec<dvb_t2mi::pump::T2miEvent> {
    let mut pump = T2miPump::new(pid);
    let mut events = Vec::new();
    for pkt in data.chunks_exact(188) {
        for ev in pump.feed_ts(pkt) {
            events.push(ev);
        }
    }
    events
}

fn t2mi_fixture_path() -> &'static str {
    concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../fixtures/dvb-t2mi/colombia-capital-t2mi.ts"
    )
}

// ── test 1: differential against T2miPump on the T2-MI fixture ───────────────

#[tokio::test]
async fn t2mi_stream_matches_sync_oracle() {
    let path = t2mi_fixture_path();
    let data = std::fs::read(path).expect("colombia-capital-t2mi.ts fixture not found");

    // The T2-MI PID for this fixture (see dvb-t2mi/tests/real_capture.rs,
    // which uses the same file and PID).
    const T2MI_PID: u16 = 0x0040;

    let oracle = t2mi_sync_oracle(&data, T2MI_PID);
    assert!(
        !oracle.is_empty(),
        "oracle produced no events — wrong PID or empty fixture?"
    );

    let cursor = std::io::Cursor::new(data.clone());
    let stream = T2miEventStream::new(cursor, T2MI_PID);

    // HANG GUARD (issue #807): in-memory Cursor over a small fixture, no real
    // I/O wait — normally completes well under a second. Generous on
    // purpose: only fails a stalled/deadlocked stream, not a timing claim.
    let async_events = tokio::time::timeout(Duration::from_secs(60), async {
        let mut events = Vec::new();
        let mut stream = stream;
        loop {
            let item = std::future::poll_fn(|cx| Pin::new(&mut stream).poll_next(cx)).await;
            match item {
                Some(ev) => events.push(ev),
                None => break,
            }
        }
        events
    })
    .await
    .expect("T2miEventStream stalled — hang guard (issue #807) fired after 60 s");

    assert_eq!(
        async_events.len(),
        oracle.len(),
        "T2-MI event count: async={} oracle={}",
        async_events.len(),
        oracle.len()
    );

    for (i, (got, want)) in async_events.iter().zip(oracle.iter()).enumerate() {
        assert_eq!(
            got.packet_type(),
            want.packet_type(),
            "event[{i}] packet_type mismatch"
        );
        assert_eq!(got.bytes(), want.bytes(), "event[{i}] bytes mismatch");
    }
}

// ── test 1b: one-byte-at-a-time stress test (issue #1036) ────────────────────
//
// `std::io::Cursor`'s `poll_read` always fills as much of the destination
// buffer as is available, and `T2miEventStream`'s read buffer
// (`TS_PACKET_SIZE * 7` = 1316 bytes) is itself an exact multiple of 188 —
// so every Cursor-backed read in test 1 above lands exactly on a packet
// boundary and never exercises a trailing partial packet at all. A reader
// that returns short reads of an unrelated size is required to hit that
// path; mirrors `differential.rs`'s `OneByteAtATime` for `SectionStream`,
// which already carries partial packets correctly.
struct OneByteAtATime {
    data: Vec<u8>,
    pos: usize,
}

impl tokio::io::AsyncRead for OneByteAtATime {
    fn poll_read(
        mut self: Pin<&mut Self>,
        _cx: &mut std::task::Context<'_>,
        buf: &mut tokio::io::ReadBuf<'_>,
    ) -> std::task::Poll<std::io::Result<()>> {
        if self.pos >= self.data.len() {
            return std::task::Poll::Ready(Ok(())); // EOF
        }
        buf.put_slice(&self.data[self.pos..self.pos + 1]);
        self.pos += 1;
        std::task::Poll::Ready(Ok(()))
    }
}

#[tokio::test]
async fn t2mi_stream_one_byte_at_a_time_matches_oracle() {
    let path = t2mi_fixture_path();
    let data = std::fs::read(path).expect("colombia-capital-t2mi.ts fixture not found");
    const T2MI_PID: u16 = 0x0040;
    let oracle = t2mi_sync_oracle(&data, T2MI_PID);
    assert!(
        !oracle.is_empty(),
        "oracle produced no events — fixture empty?"
    );

    let reader = OneByteAtATime {
        data: data.clone(),
        pos: 0,
    };
    let stream = T2miEventStream::new(reader, T2MI_PID);

    // HANG GUARD (issue #807): in-memory, one byte at a time is slower than
    // the bulk cursor case above but still normally sub-second even for a
    // whole T2-MI fixture; generous rather than a timing claim.
    let async_events = tokio::time::timeout(Duration::from_secs(60), async {
        let mut events = Vec::new();
        let mut stream = stream;
        loop {
            let item = std::future::poll_fn(|cx| Pin::new(&mut stream).poll_next(cx)).await;
            match item {
                Some(ev) => events.push(ev),
                None => break,
            }
        }
        events
    })
    .await
    .expect("hang guard (issue #807) fired: one-byte-at-a-time stream never completed within 60s");

    assert_eq!(
        async_events.len(),
        oracle.len(),
        "one-byte reader: T2-MI event count mismatch async={} oracle={}",
        async_events.len(),
        oracle.len()
    );
    for (i, (got, want)) in async_events.iter().zip(oracle.iter()).enumerate() {
        assert_eq!(
            got.packet_type(),
            want.packet_type(),
            "event[{i}] packet_type"
        );
        assert_eq!(got.bytes(), want.bytes(), "event[{i}] bytes");
    }
}

// ── test 2: in-memory cursor with constructed T2-MI packets ──────────────────

/// Build a minimal syntactically-valid T2-MI TS packet (BBFrame type 0x00).
fn make_t2mi_ts_packet(pid: u16) -> [u8; 188] {
    make_t2mi_ts_packet_n(pid, 0x01)
}

/// As [`make_t2mi_ts_packet`] with a chosen T2-MI `packet_count`, so
/// consecutive packets are distinguishable.
fn make_t2mi_ts_packet_n(pid: u16, packet_count: u8) -> [u8; 188] {
    use broadcast_common::crc32_mpeg2;

    // Build the T2-MI packet: header(6) + payload(3) + CRC(4).
    let payload = [0x01u8, 0x02, 0x00]; // minimal BBFrame payload
    let payload_len_bits = (payload.len() * 8) as u16;
    let mut t2mi: Vec<u8> = Vec::with_capacity(6 + payload.len() + 4);
    t2mi.push(0x00); // packet_type: BBFrame
    t2mi.push(packet_count); // packet_count
    t2mi.push(0x00); // superframe_idx + rfu + t2mi_stream_id
    t2mi.push(0x00); // rfu
    t2mi.extend_from_slice(&payload_len_bits.to_be_bytes());
    t2mi.extend_from_slice(&payload);
    let crc = crc32_mpeg2::compute(&t2mi);
    t2mi.extend_from_slice(&crc.to_be_bytes());

    // Wrap in a TS packet.
    let mut pkt = [0xFFu8; 188];
    pkt[0] = 0x47; // sync
    pkt[1] = 0x40 | (((pid >> 8) & 0x1F) as u8); // PUSI + PID hi
    pkt[2] = (pid & 0xFF) as u8;
    pkt[3] = 0x10; // payload only
    pkt[4] = 0x00; // pointer_field = 0
    let start = 5;
    pkt[start..start + t2mi.len()].copy_from_slice(&t2mi);
    pkt
}

#[tokio::test]
async fn t2mi_stream_in_memory_constructed_packet() {
    const PID: u16 = 0x0006;
    let pkt = make_t2mi_ts_packet(PID);

    let cursor = std::io::Cursor::new(pkt.to_vec());
    let stream = T2miEventStream::new(cursor, PID);

    // HANG GUARD (issue #807): single 188-byte in-memory packet — normally
    // instant. Generous on purpose, not a timing claim.
    let events = tokio::time::timeout(Duration::from_secs(60), async {
        let mut events = Vec::new();
        let mut stream = stream;
        loop {
            let item = std::future::poll_fn(|cx| Pin::new(&mut stream).poll_next(cx)).await;
            match item {
                Some(ev) => events.push(ev),
                None => break,
            }
        }
        events
    })
    .await
    .expect("hang guard (issue #807) fired: stream never completed within 60s");

    assert_eq!(events.len(), 1, "expected exactly one T2-MI event");
    assert_eq!(
        events[0].packet_type(),
        0x00,
        "expected BBFrame packet_type"
    );
}

// ── test 3: stats accessible after T2miEventStream completion ────────────────

#[tokio::test]
async fn t2mi_stream_stats_after_completion() {
    const PID: u16 = 0x0006;
    let pkt = make_t2mi_ts_packet(PID);

    let cursor = std::io::Cursor::new(pkt.to_vec());
    let mut stream = T2miEventStream::new(cursor, PID);

    // HANG GUARD (issue #807): single 188-byte in-memory packet — normally
    // instant. Generous on purpose, not a timing claim.
    tokio::time::timeout(Duration::from_secs(60), async {
        loop {
            let item = std::future::poll_fn(|cx| Pin::new(&mut stream).poll_next(cx)).await;
            if item.is_none() {
                break;
            }
        }
    })
    .await
    .expect("hang guard (issue #807) fired: stream never completed within 60s");

    let stats = stream.stats();
    assert!(stats.ts_packets >= 1, "expected at least 1 ts_packets");
    assert_eq!(stats.crc_failures, 0);
}

// ── test 4: a TS packet split across two reads (partial-packet carry-over,
// issue #1036) ─────────────────────────────────────────────────────────────

/// Hands out its data in the given chunk sizes, one chunk per `poll_read`.
struct ChunkedReader {
    data: Vec<u8>,
    chunks: Vec<usize>,
    pos: usize,
    next: usize,
}

impl tokio::io::AsyncRead for ChunkedReader {
    fn poll_read(
        mut self: Pin<&mut Self>,
        _cx: &mut std::task::Context<'_>,
        buf: &mut tokio::io::ReadBuf<'_>,
    ) -> std::task::Poll<std::io::Result<()>> {
        let Some(&n) = self.chunks.get(self.next) else {
            return std::task::Poll::Ready(Ok(())); // EOF
        };
        let end = (self.pos + n).min(self.data.len());
        buf.put_slice(&self.data[self.pos..end]);
        self.pos = end;
        self.next += 1;
        std::task::Poll::Ready(Ok(()))
    }
}

/// Three distinct T2-MI packets delivered as reads of 100 + 150 + 314 bytes:
/// the first read ends mid-packet-0, the second ends mid-packet-1. The
/// trailing partial packet must be carried into the next read (shared
/// `TsFramer`, #1036/#1141) — all three events arrive intact, byte-equal to
/// the synchronous oracle.
#[tokio::test]
async fn t2mi_stream_partial_packet_split_across_two_reads_is_carried_over() {
    const PID: u16 = 0x0006;
    let mut data = Vec::new();
    for count in 1..=3u8 {
        data.extend_from_slice(&make_t2mi_ts_packet_n(PID, count));
    }
    assert_eq!(data.len(), 3 * 188);
    let oracle = t2mi_sync_oracle(&data, PID);
    assert_eq!(oracle.len(), 3, "oracle sees all three packets");

    let reader = ChunkedReader {
        data: data.clone(),
        chunks: vec![100, 150, 314],
        pos: 0,
        next: 0,
    };
    let mut stream = T2miEventStream::new(reader, PID);
    let got = tokio::time::timeout(Duration::from_secs(60), async {
        let mut events = Vec::new();
        while let Some(ev) = std::future::poll_fn(|cx| Pin::new(&mut stream).poll_next(cx)).await {
            events.push(ev);
        }
        events
    })
    .await
    .expect("hang guard (issue #807) fired");

    assert_eq!(got.len(), 3, "all packets delivered");
    for (i, (g, w)) in got.iter().zip(oracle.iter()).enumerate() {
        assert_eq!(g.bytes(), w.bytes(), "event[{i}] bytes intact");
    }
    assert_eq!(stream.resync_stats().bytes_discarded, 0, "nothing dropped");
    assert_eq!(stream.resync_stats().desyncs, 0);
}
