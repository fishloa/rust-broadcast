//! Advanced: depacketize a real MPEG-TS capture, reassemble PES on one PID,
//! and report the PTS timeline.
//!
//! Run with: `cargo run -p mpeg-pes --example extract_pts`
//!
//! Reads the committed `m6-single.ts` fixture from the workspace-shared
//! `fixtures/` tree at runtime.

use mpeg_pes::{PesAssembler, PesPacket};

const PKT: usize = 188;
const PES_PID: u16 = 0x0082; // an ES PID carrying PES with PTS in this capture

/// (pid, payload_unit_start, payload) for a TS packet that carries payload.
fn ts_payload(p: &[u8]) -> Option<(u16, bool, &[u8])> {
    if p.len() != PKT || p[0] != 0x47 {
        return None;
    }
    let pid = (u16::from(p[1] & 0x1F) << 8) | u16::from(p[2]);
    let pusi = p[1] & 0x40 != 0;
    let start = match (p[3] >> 4) & 0x03 {
        1 => 4,                     // payload only
        3 => 5 + usize::from(p[4]), // adaptation field, then payload
        _ => return None,           // 0 reserved / 2 adaptation only
    };
    (start < PKT).then(|| (pid, pusi, &p[start..]))
}

fn main() {
    // Fixtures live in the workspace-shared `fixtures/` tree, not under a
    // sibling crate. A committed fixture that cannot be read is a bug, not a
    // reason to skip — so a missing/unreadable fixture is a hard failure.
    let path = test_fixtures::path("ts/m6-single.ts");
    let ts = std::fs::read(&path).unwrap_or_else(|e| {
        panic!(
            "committed fixture {} could not be read: {e}",
            path.display()
        )
    });

    let mut asm = PesAssembler::new();
    let mut pes = Vec::new();
    for pkt in ts.chunks(PKT) {
        if let Some((pid, pusi, payload)) = ts_payload(pkt)
            && pid == PES_PID
            && let Some(v) = asm.feed(pusi, payload)
        {
            pes.push(v);
        }
    }
    if let Some(v) = asm.flush() {
        pes.push(v);
    }

    // Real captures declare a `PES_packet_length` longer than the bytes that
    // arrive before the next unit starts, so read each unit with the lenient
    // `parse_unit` and report any shortfall rather than rejecting the stream.
    let mut pts_ticks = Vec::new();
    let mut short_units = 0usize;
    for raw in &pes {
        let (pkt, truncation) = PesPacket::parse_unit(raw).expect("captured PES must parse");
        if let Some(t) = truncation {
            short_units += 1;
            println!(
                "truncated unit : declared {} bytes, {} available",
                t.declared, t.available
            );
        }
        if let Some(pts) = pkt.header.as_ref().and_then(|h| h.pts) {
            assert!(pts.ticks() < (1 << 33), "PTS must be 33-bit");
            pts_ticks.push(pts.ticks());
        }
    }

    println!(
        "PID {PES_PID:#06X}: {} PES packets, {} with PTS, {} truncated",
        pes.len(),
        pts_ticks.len(),
        short_units
    );
    if let (Some(&min), Some(&max)) = (pts_ticks.iter().min(), pts_ticks.iter().max()) {
        let span = (max - min) as f64 / 90_000.0;
        println!("PTS range      : {min}..={max} ticks (~{span:.3}s span @ 90 kHz)");
    }
}
