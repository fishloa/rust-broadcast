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

    // Detect the T2-MI PID from the pump stats — we try 0x0006 (common default).
    const T2MI_PID: u16 = 0x0006;

    let oracle = t2mi_sync_oracle(&data, T2MI_PID);

    let cursor = std::io::Cursor::new(data.clone());
    let stream = T2miEventStream::new(cursor, T2MI_PID);

    let async_events = tokio::time::timeout(Duration::from_secs(5), async {
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
    .expect("T2miEventStream stalled — timeout after 5 s");

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

// ── test 2: in-memory cursor with constructed T2-MI packets ──────────────────

/// Build a minimal syntactically-valid T2-MI TS packet (BBFrame type 0x00).
fn make_t2mi_ts_packet(pid: u16) -> [u8; 188] {
    use broadcast_common::crc32_mpeg2;

    // Build the T2-MI packet: header(6) + payload(3) + CRC(4).
    let payload = [0x01u8, 0x02, 0x00]; // minimal BBFrame payload
    let payload_len_bits = (payload.len() * 8) as u16;
    let mut t2mi: Vec<u8> = Vec::with_capacity(6 + payload.len() + 4);
    t2mi.push(0x00); // packet_type: BBFrame
    t2mi.push(0x01); // packet_count
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
    assert!(stats.ts_packets >= 1, "expected at least 1 ts_packets");
    assert_eq!(stats.crc_failures, 0);
}
