//! Advanced: depacketize a real MPEG-TS capture, reassemble PES on one PID,
//! and report the PTS timeline.
//!
//! Run with: `cargo run -p mpeg-pes --example extract_pts`
//!
//! Reads the committed `m6-single.ts` DVB fixture from the sibling `dvb-si`
//! crate at runtime (so the example compiles even when the fixture is absent).

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
    let path = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../dvb-si/tests/fixtures/m6-single.ts"
    );
    let ts = match std::fs::read(path) {
        Ok(b) => b,
        Err(e) => {
            eprintln!("fixture not available ({e}); nothing to do");
            return;
        }
    };

    let mut asm = PesAssembler::new();
    let mut pes = Vec::new();
    for pkt in ts.chunks(PKT) {
        if let Some((pid, pusi, payload)) = ts_payload(pkt) {
            if pid == PES_PID {
                if let Some(v) = asm.feed(pusi, payload) {
                    pes.push(v);
                }
            }
        }
    }
    if let Some(v) = asm.flush() {
        pes.push(v);
    }

    let mut pts_ticks = Vec::new();
    for raw in &pes {
        let pkt = PesPacket::parse(raw).expect("captured PES must parse");
        if let Some(pts) = pkt.header.as_ref().and_then(|h| h.pts) {
            assert!(pts.ticks() < (1 << 33), "PTS must be 33-bit");
            pts_ticks.push(pts.ticks());
        }
    }

    println!(
        "PID {PES_PID:#06X}: {} PES packets, {} with PTS",
        pes.len(),
        pts_ticks.len()
    );
    if let (Some(&min), Some(&max)) = (pts_ticks.iter().min(), pts_ticks.iter().max()) {
        let span = (max - min) as f64 / 90_000.0;
        println!("PTS range      : {min}..={max} ticks (~{span:.3}s span @ 90 kHz)");
    }
}
