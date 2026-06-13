//! Differential tests: async adapter vs. sync oracle.
//!
//! For each fixture, the SYNC oracle drives [`SiDemux`] directly on the raw bytes,
//! collecting all `SectionEvent`s. The ASYNC path feeds the same bytes through
//! [`SectionStream`] (in-memory `Cursor`) and collects the same events.
//! The two lists must be identical (same table_ids, same PIDs, same section bytes,
//! same order).

use std::time::Duration;

use dvb_si::demux::SiDemux;
use dvb_stream::SectionStream;
use futures_core::stream::Stream;
use std::pin::Pin;
use std::task::{Context, Poll};
use tokio::io::AsyncRead;

// ── helpers ───────────────────────────────────────────────────────────────────

/// Collect all `SectionEvent`s from `stream` with a hard 5-second timeout.
/// Panics if the timeout fires (catches a stalled stream instead of hanging).
async fn collect_section_stream<R: AsyncRead + Unpin>(
    mut stream: SectionStream<R>,
) -> Vec<dvb_si::demux::SectionEvent> {
    tokio::time::timeout(Duration::from_secs(5), async {
        let mut events = Vec::new();
        while let Some(ev) = futures_util_poll_once(&mut stream).await {
            events.push(ev);
        }
        events
    })
    .await
    .expect("SectionStream stalled — timeout after 5 s")
}

/// Poll a `Stream` once, returning `Some(item)` or `None` on termination.
/// This is a minimal stand-in for `StreamExt::next` without pulling in futures.
async fn futures_util_poll_once<S: Stream + Unpin>(stream: &mut S) -> Option<S::Item> {
    std::future::poll_fn(|cx| Pin::new(&mut *stream).poll_next(cx)).await
}

// ── oracle helper ─────────────────────────────────────────────────────────────

/// Drive `SiDemux` synchronously on `data` (raw 188-byte-aligned bytes), using
/// a fresh default demux, and return all emitted `SectionEvent`s.
fn sync_oracle(data: &[u8]) -> Vec<dvb_si::demux::SectionEvent> {
    let mut demux = SiDemux::builder().build();
    let mut events = Vec::new();
    for pkt in data.chunks_exact(188) {
        for ev in demux.feed(pkt) {
            events.push(ev);
        }
    }
    events
}

// ── fixture path helper ───────────────────────────────────────────────────────

fn m6_fixture_path() -> &'static str {
    concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../dvb-si/tests/fixtures/m6-single.ts"
    )
}

// ── test 1: differential against SiDemux on m6-single.ts ─────────────────────

#[tokio::test]
async fn section_stream_matches_sync_oracle_m6() {
    let path = m6_fixture_path();
    let data = std::fs::read(path).expect("m6-single.ts fixture not found");

    // Sync oracle.
    let oracle = sync_oracle(&data);
    assert!(
        !oracle.is_empty(),
        "oracle produced no events — fixture empty?"
    );

    // Async path: feed via in-memory Cursor.
    let cursor = tokio::io::BufReader::new(std::io::Cursor::new(data));
    let stream = SectionStream::new(cursor);
    let async_events = collect_section_stream(stream).await;

    // Compare counts first for a cleaner failure message.
    assert_eq!(
        async_events.len(),
        oracle.len(),
        "event count mismatch: async={} oracle={}",
        async_events.len(),
        oracle.len()
    );

    // Compare each event: pid, table_id, section bytes.
    for (i, (got, want)) in async_events.iter().zip(oracle.iter()).enumerate() {
        assert_eq!(
            got.pid(),
            want.pid(),
            "event[{i}] pid mismatch: got={:?} want={:?}",
            got.pid(),
            want.pid()
        );
        assert_eq!(
            got.table_id(),
            want.table_id(),
            "event[{i}] table_id mismatch"
        );
        assert_eq!(got.bytes(), want.bytes(), "event[{i}] bytes mismatch");
    }
}

// ── test 2: in-memory cursor smoke test ──────────────────────────────────────

#[tokio::test]
async fn section_stream_in_memory_cursor_produces_events() {
    let path = m6_fixture_path();
    let data = std::fs::read(path).expect("m6-single.ts fixture not found");

    let cursor = std::io::Cursor::new(data);
    let stream = SectionStream::new(cursor);

    let events = tokio::time::timeout(Duration::from_secs(5), async {
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
    .expect("timed out");

    assert!(
        !events.is_empty(),
        "expected at least one section event from m6-single.ts"
    );
}

// ── test 3: tiny-chunk stress test ───────────────────────────────────────────
//
// Feed the fixture one byte at a time through a custom AsyncRead adapter to
// stress the resync + partial-packet carry-over logic.

struct OneByteAtATime {
    data: Vec<u8>,
    pos: usize,
}

impl AsyncRead for OneByteAtATime {
    fn poll_read(
        mut self: Pin<&mut Self>,
        _cx: &mut Context<'_>,
        buf: &mut tokio::io::ReadBuf<'_>,
    ) -> Poll<std::io::Result<()>> {
        if self.pos >= self.data.len() {
            return Poll::Ready(Ok(())); // EOF
        }
        buf.put_slice(&self.data[self.pos..self.pos + 1]);
        self.pos += 1;
        Poll::Ready(Ok(()))
    }
}

#[tokio::test]
async fn section_stream_one_byte_at_a_time_matches_oracle() {
    let path = m6_fixture_path();
    let data = std::fs::read(path).expect("m6-single.ts fixture not found");
    let oracle = sync_oracle(&data);

    let reader = OneByteAtATime {
        data: data.clone(),
        pos: 0,
    };
    let stream = SectionStream::new(reader);
    let async_events = tokio::time::timeout(Duration::from_secs(10), async {
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
    .expect("one-byte-at-a-time test timed out");

    assert_eq!(
        async_events.len(),
        oracle.len(),
        "one-byte reader: event count mismatch async={} oracle={}",
        async_events.len(),
        oracle.len()
    );
    for (i, (got, want)) in async_events.iter().zip(oracle.iter()).enumerate() {
        assert_eq!(got.pid(), want.pid(), "event[{i}] pid");
        assert_eq!(got.table_id(), want.table_id(), "event[{i}] table_id");
        assert_eq!(got.bytes(), want.bytes(), "event[{i}] bytes");
    }
}

// ── test 4: stats() accessible after stream completion ───────────────────────

#[tokio::test]
async fn section_stream_stats_after_completion() {
    let path = m6_fixture_path();
    let data = std::fs::read(path).expect("m6-single.ts fixture not found");
    let data_len = data.len();

    let cursor = std::io::Cursor::new(data);
    let mut stream = SectionStream::new(cursor);

    // Drain the stream.
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            let item = std::future::poll_fn(|cx| Pin::new(&mut stream).poll_next(cx)).await;
            if item.is_none() {
                break;
            }
        }
    })
    .await
    .expect("timeout");

    let stats = stream.stats();
    // We should have processed at least data_len / 188 packets.
    assert!(
        stats.packets >= (data_len / 188) as u64,
        "expected at least {} packets, got {}",
        data_len / 188,
        stats.packets
    );
    assert!(stats.emitted > 0, "expected emitted > 0");
    assert_eq!(stats.crc_failures, 0, "expected zero CRC failures");
}
