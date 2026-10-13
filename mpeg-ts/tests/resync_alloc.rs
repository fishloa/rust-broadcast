//! Allocation bound for `TsResync::feed` / `feed_into` (audit r01-O4): output
//! capacity is reserved once, and a reused output `Vec` costs no allocation
//! per steady-state call.

use mpeg_ts::resync::TsResync;
use mpeg_ts::ts::{TS_PACKET_SIZE, TS_SYNC_BYTE};
use test_alloc::ThreadCounting;

#[global_allocator]
static A: ThreadCounting = ThreadCounting::new();

fn stream(packets: usize) -> Vec<u8> {
    let mut v = vec![0u8; packets * TS_PACKET_SIZE];
    for p in v.chunks_mut(TS_PACKET_SIZE) {
        p[0] = TS_SYNC_BYTE;
    }
    v
}

#[test]
fn feed_reserves_output_once() {
    let data = stream(2000);
    let mut r = TsResync::new();
    let (out, snap) = ThreadCounting::measure(|| r.feed(&data));
    assert_eq!(out.len(), 2000);
    // Input buffer (1) + output (1) + slack; doubling growth of the output
    // alone would take >= 11 reallocations for 2000 packets.
    assert!(snap.allocs <= 4, "allocs = {}", snap.allocs);
}

#[test]
fn feed_into_reused_vec_is_alloc_free() {
    let data = stream(64);
    let mut r = TsResync::new();
    let mut out = Vec::new();
    r.feed_into(&data, &mut out); // warm-up sizes buf + out
    out.clear();
    let (n, snap) = ThreadCounting::measure(|| r.feed_into(&data, &mut out));
    assert_eq!(n, 64);
    assert_eq!(snap.allocs, 0, "steady-state feed_into allocated");
}
