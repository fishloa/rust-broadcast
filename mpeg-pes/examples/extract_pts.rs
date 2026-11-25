//! Advanced: depacketize a real MPEG-TS capture, reassemble PES on one PID,
//! and report the PTS timeline.
//!
//! Run with: `cargo run -p mpeg-pes --example extract_pts`
//!
//! Reads the committed `m6-single.ts` fixture from the workspace-shared
//! `fixtures/` tree at runtime.

use std::fs::File;
use std::io::{self, Read};

use mpeg_pes::{PesAssembler, PesPacket};

const PKT: usize = 188;

/// Read granularity for streaming the capture.
const READ_CHUNK_LEN: usize = 1 << 20;

/// Fill `packet` from `reader`; `false` at a clean end of stream.
fn read_packet(reader: &mut impl Read, packet: &mut [u8; PKT]) -> io::Result<bool> {
    let mut filled = 0usize;
    while filled < packet.len() {
        let read = reader.read(&mut packet[filled..])?;
        if read == 0 {
            return Ok(false);
        }
        filled += read;
    }
    Ok(true)
}
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
    let file = File::open(&path).unwrap_or_else(|e| {
        panic!(
            "committed fixture {} could not be read: {e}",
            path.display()
        )
    });

    let mut asm = PesAssembler::new();
    let mut tally = Tally::default();
    let mut reader = io::BufReader::with_capacity(READ_CHUNK_LEN, file);
    let mut packet = [0u8; PKT];
    while read_packet(&mut reader, &mut packet)
        .unwrap_or_else(|e| panic!("fixture read failed: {e}"))
    {
        if let Some((pid, pusi, payload)) = ts_payload(&packet)
            && pid == PES_PID
            && let Some(v) = asm.feed(pusi, payload)
        {
            tally.unit(&v);
        }
    }
    if let Some(v) = asm.flush() {
        tally.unit(&v);
    }

    println!(
        "PID {PES_PID:#06X}: {} PES packets, {} with PTS, {} truncated",
        tally.pes_packets, tally.with_pts, tally.short_units
    );
    if let (Some(min), Some(max)) = (tally.min_pts, tally.max_pts) {
        let span = (max - min) as f64 / 90_000.0;
        println!("PTS range      : {min}..={max} ticks (~{span:.3}s span @ 90 kHz)");
    }
}

/// Running tallies over the PES units of the watched PID.
#[derive(Default)]
struct Tally {
    /// PES units seen.
    pes_packets: usize,
    /// Units carrying a PTS.
    with_pts: usize,
    /// Units whose declared length overran the bytes that arrived.
    short_units: usize,
    /// Smallest PTS seen.
    min_pts: Option<u64>,
    /// Largest PTS seen.
    max_pts: Option<u64>,
}

impl Tally {
    /// Examine one assembled PES unit.
    fn unit(&mut self, raw: &[u8]) {
        self.pes_packets += 1;
        // Real captures declare a `PES_packet_length` longer than the bytes
        // that arrive before the next unit starts, so read each unit with the
        // lenient `parse_unit` and report any shortfall rather than rejecting
        // the stream.
        let (pkt, truncation) = PesPacket::parse_unit(raw).expect("captured PES must parse");
        if let Some(t) = truncation {
            self.short_units += 1;
            println!(
                "truncated unit : declared {} bytes, {} available",
                t.declared, t.available
            );
        }
        if let Some(pts) = pkt.header.as_ref().and_then(|h| h.pts) {
            let ticks = pts.ticks();
            assert!(ticks < (1 << 33), "PTS must be 33-bit");
            self.with_pts += 1;
            self.min_pts = Some(self.min_pts.map_or(ticks, |min| min.min(ticks)));
            self.max_pts = Some(self.max_pts.map_or(ticks, |max| max.max(ticks)));
        }
    }
}
