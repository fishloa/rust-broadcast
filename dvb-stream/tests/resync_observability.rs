//! Resync observability tests for SectionStream.
//!
//! Verifies that mid-stream desync recovery and resync counters work correctly.
//! Uses in-memory Cursor sources with crafted byte streams.

use std::pin::Pin;
use std::time::Duration;

use dvb_stream::SectionStream;
use futures_core::stream::Stream;

// ── helpers ───────────────────────────────────────────────────────────────────

/// Build a minimal SI-carrying TS packet on `pid` with a valid PAT-like
/// section payload so the demux actually produces an event.
fn make_si_ts_packet(pid: u16, _payload_byte: u8) -> [u8; 188] {
    let mut pkt = [0xFFu8; 188];
    pkt[0] = 0x47; // sync
    pkt[1] = 0x40 | (((pid >> 8) & 0x1F) as u8); // PUSI + PID hi
    pkt[2] = (pid & 0xFF) as u8;
    pkt[3] = 0x10; // payload only

    // Minimal PAT section: table_id=0x00, section_syntax=1, section_length=13.
    let section: [u8; 15] = [
        0x00, // table_id = PAT
        0xB0, 0x0D, // section_syntax=1, section_length=13
        0x00, 0x01, // transport_stream_id
        0xC1, // version=0, section_number=0
        0x00, // last_section_number=0
        0x00, 0x00, // program_number=0 (NIT)
        0xE0, 0x10, // network_pid = 0x0010
        0x00, 0x00, 0x00, 0x00, // CRC-32 placeholder
    ];
    pkt[4] = 0x00; // pointer_field
    pkt[5..5 + section.len()].copy_from_slice(&section);
    pkt
}

/// Drain a SectionStream with a 5-second timeout.
async fn drain_section_stream<R: tokio::io::AsyncRead + Unpin>(stream: &mut SectionStream<R>) {
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            let item = std::future::poll_fn(|cx| Pin::new(&mut *stream).poll_next(cx)).await;
            if item.is_none() {
                break;
            }
        }
    })
    .await
    .expect("SectionStream stalled — timeout after 5 s");
}

// ── test 1: clean stream ──────────────────────────────────────────────────────
//
// A perfectly aligned stream with no leading junk.  The first feed_buf call
// is unsynced, so resync confirms the sync at offset 0 → resyncs=1,
// bytes_discarded=0.

#[tokio::test]
async fn section_stream_clean_resync_stats() {
    let pid = 0x0000u16;
    let mut data = Vec::new();
    for i in 0..3 {
        let pkt = make_si_ts_packet(pid, i);
        data.extend_from_slice(&pkt);
    }

    let cursor = std::io::Cursor::new(data);
    let mut stream = SectionStream::new(cursor);
    drain_section_stream(&mut stream).await;

    let stats = stream.resync_stats();
    assert_eq!(stats.resyncs, 1, "expected 1 resync, got {}", stats.resyncs);
    assert_eq!(stats.bytes_discarded, 0, "expected 0 bytes discarded");
    assert_eq!(stats.desyncs, 0, "expected 0 desyncs");
}

// ── test 2: leading non-0x47 junk ─────────────────────────────────────────────

#[tokio::test]
async fn section_stream_leading_junk_increments_resync_counters() {
    let pid = 0x0000u16;
    let mut data = Vec::new();
    data.extend_from_slice(&[0x00u8; 42]); // 42 bytes leading junk

    for i in 0..3 {
        let pkt = make_si_ts_packet(pid, i);
        data.extend_from_slice(&pkt);
    }

    let cursor = std::io::Cursor::new(data);
    let mut stream = SectionStream::new(cursor);
    drain_section_stream(&mut stream).await;

    let stats = stream.resync_stats();
    assert_eq!(stats.resyncs, 1, "expected 1 resync");
    assert_eq!(stats.bytes_discarded, 42, "expected 42 bytes discarded");
    assert_eq!(stats.desyncs, 0, "expected 0 desyncs");
}

// ── test 3: junk-only stream — all bytes discarded ────────────────────────────

#[tokio::test]
async fn section_stream_junk_only_discards_all_bytes() {
    let data = vec![0x00u8; 500]; // No 0x47 at all.
    let cursor = std::io::Cursor::new(data);
    let mut stream = SectionStream::new(cursor);
    drain_section_stream(&mut stream).await;

    let stats = stream.resync_stats();
    assert_eq!(stats.resyncs, 0, "expected 0 resyncs");
    assert_eq!(stats.bytes_discarded, 500, "expected 500 bytes discarded");
    assert_eq!(stats.desyncs, 0, "expected 0 desyncs");
}

// ── test 4: mid-stream corruption → desync detected ──────────────────────────
//
// Build 5 clean aligned packets, corrupt the 3rd packet's sync byte, and
// feed as a single chunk.  The desync fires on the corrupted packet, the
// rest of the chunk is dropped, and counters reflect it.

#[tokio::test]
async fn section_stream_mid_stream_corruption_detected() {
    let pid = 0x0000u16;

    let mut data = Vec::new();
    for i in 0..2 {
        data.extend_from_slice(&make_si_ts_packet(pid, i));
    }
    let mut corrupt = make_si_ts_packet(pid, 2);
    corrupt[0] = 0x00;
    data.extend_from_slice(&corrupt);
    for i in 3..5 {
        data.extend_from_slice(&make_si_ts_packet(pid, i));
    }

    // data = 5 * 188 = 940 bytes
    let cursor = std::io::Cursor::new(data);
    let mut stream = SectionStream::new(cursor);
    drain_section_stream(&mut stream).await;

    let stats = stream.resync_stats();
    assert_eq!(stats.resyncs, 1, "expected 1 resync (initial only)");
    // Desync discards rest of chunk from corrupted pkt offset (2*188=376).
    // 940 - 376 = 564 bytes discarded.
    assert_eq!(
        stats.bytes_discarded, 564,
        "expected 564 bytes discarded, got {}",
        stats.bytes_discarded
    );
    assert_eq!(stats.desyncs, 1, "expected 1 desync, got {}", stats.desyncs);
}

// ── test 5: mid-stream corruption with recovery via byte-at-a-time reader ────
//
// A custom AsyncRead that delivers data in small increments (18 bytes at a
// time).  This forces the stream to perform many poll_read cycles, naturally
// creating separate feed_buf calls.  After the corrupted byte triggers
// desync, the next poll_read cycle re-resyncs and delivers events from
// trailing clean packets.

use tokio::io::AsyncRead;

/// An [`AsyncRead`] that delivers data 18 bytes at a time (small enough to
/// force multiple read cycles).
struct SlowReader {
    data: Vec<u8>,
    pos: usize,
}

impl AsyncRead for SlowReader {
    fn poll_read(
        mut self: Pin<&mut Self>,
        _cx: &mut std::task::Context<'_>,
        buf: &mut tokio::io::ReadBuf<'_>,
    ) -> std::task::Poll<std::io::Result<()>> {
        if self.pos >= self.data.len() {
            return std::task::Poll::Ready(Ok(())); // EOF
        }
        let chunk_size = 18usize.min(self.data.len() - self.pos);
        buf.put_slice(&self.data[self.pos..self.pos + chunk_size]);
        self.pos += chunk_size;
        std::task::Poll::Ready(Ok(()))
    }
}

#[tokio::test]
async fn section_stream_slow_reader_corruption_recovers() {
    let pid = 0x0000u16;

    // 5 clean packets + 1 corrupted in the middle, then 2 trailing clean.
    let mut data = Vec::new();
    for i in 0..2 {
        data.extend_from_slice(&make_si_ts_packet(pid, i));
    }
    let mut corrupt = make_si_ts_packet(pid, 2);
    corrupt[0] = 0x00; // corrupt sync byte
    data.extend_from_slice(&corrupt);
    for i in 3..7 {
        data.extend_from_slice(&make_si_ts_packet(pid, i));
    }
    // data = 7 * 188 = 1316 bytes

    let reader = SlowReader { data, pos: 0 };
    let mut stream = SectionStream::new(reader);
    drain_section_stream(&mut stream).await;

    let stats = stream.resync_stats();
    // The slow reader feeds many small chunks.  The initial resync finds
    // 0x47 at offset 0.  The corrupted packet triggers desync in whatever
    // chunk it lands in.  After desync, the next chunk re-resyncs.
    assert_eq!(
        stats.resyncs, 2,
        "expected 2 resyncs (initial + post-desync), got {}",
        stats.resyncs
    );
    assert!(
        stats.desyncs >= 1,
        "expected at least 1 desync, got {}",
        stats.desyncs
    );
    // bytes_discarded should cover the corrupted packet + its trailing data
    // in the same chunk, plus any junk.  At least 188 bytes.
    assert!(
        stats.bytes_discarded >= 188,
        "expected at least 188 bytes discarded, got {}",
        stats.bytes_discarded
    );
}
