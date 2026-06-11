//! Real-capture round-trip for the section→TS packetizer (`dvb_si::mux`).
//!
//! Fixture: `tests/fixtures/m6-single.ts` (French TNT). Drives the full output
//! half against the real input half: demux the capture to its current SI
//! sections, re-packetize each PID's sections with `SectionPacketizer`, then
//! feed the regenerated TS packets back through a fresh `SiDemux` and assert
//! the **same set** of (PID, section-bytes) emerges with no malformed/dropped
//! sections. This verifies the packetizer interoperates with the real demux
//! (PID filtering, PAT-following, CRC validation, version gating) — not just
//! the lower-level `SectionReassembler`.
#![cfg(feature = "ts")]

use dvb_si::demux::SiDemux;
use dvb_si::mux::SectionPacketizer;
use dvb_si::ts::{TS_PACKET_SIZE, TS_SYNC_BYTE};

fn read_fixture(name: &str) -> Vec<u8> {
    let path = format!("{}/tests/fixtures/{}", env!("CARGO_MANIFEST_DIR"), name);
    std::fs::read(&path).unwrap_or_else(|e| panic!("read fixture {path}: {e}"))
}

/// Demux a capture into the unique current sections it carries, as
/// `(pid, section_bytes)` in discovery order.
fn collect_sections(data: &[u8]) -> Vec<(u16, Vec<u8>)> {
    let mut demux = SiDemux::builder().build();
    let mut out = Vec::new();
    for chunk in data.chunks(TS_PACKET_SIZE) {
        if chunk.len() != TS_PACKET_SIZE || chunk[0] != TS_SYNC_BYTE {
            continue;
        }
        for ev in demux.feed(chunk) {
            out.push((u16::from(ev.pid()), ev.bytes().to_vec()));
        }
    }
    out
}

#[test]
fn packetizer_round_trips_real_capture_through_si_demux() {
    let data = read_fixture("m6-single.ts");
    let original = collect_sections(&data);
    assert!(
        original.len() >= 3,
        "fixture must yield several SI sections, got {}",
        original.len()
    );

    // Re-packetize each PID's sections. PAT (0x0000) must go first so the
    // second demux follows it and accepts the PMT PID's packets.
    let mut pids: Vec<u16> = Vec::new();
    for (pid, _) in &original {
        if !pids.contains(pid) {
            pids.push(*pid);
        }
    }
    pids.sort_by_key(|&p| if p == 0x0000 { 0 } else { 1 + p as u32 });

    let mut regenerated: Vec<[u8; TS_PACKET_SIZE]> = Vec::new();
    for &pid in &pids {
        let sections: Vec<&[u8]> = original
            .iter()
            .filter(|(p, _)| *p == pid)
            .map(|(_, b)| b.as_slice())
            .collect();
        let mut packetizer = SectionPacketizer::new(pid);
        regenerated.extend(packetizer.packetize(&sections));
    }

    // Feed the regenerated stream through a fresh demux.
    let mut demux = SiDemux::builder().build();
    let mut round = Vec::new();
    for pkt in &regenerated {
        for ev in demux.feed(pkt) {
            round.push((u16::from(ev.pid()), ev.bytes().to_vec()));
        }
    }

    // Same multiset of (pid, bytes) — order-independent.
    let mut a = original.clone();
    let mut b = round.clone();
    a.sort();
    b.sort();
    assert_eq!(
        b,
        a,
        "round-tripped sections must match the originals (got {} vs {})",
        round.len(),
        original.len()
    );
}
