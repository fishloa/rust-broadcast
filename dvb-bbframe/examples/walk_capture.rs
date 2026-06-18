//! Advanced: walk a real DVB-S2 capture, reassemble BBFrames from TS private
//! sections, and parse every BBHEADER.
//!
//! Run with: `cargo run -p dvb-bbframe --example walk_capture`
//!
//! Reads the committed `tnt-5w-12732v-bbframe.ts` fixture at runtime, so the
//! example compiles even when the fixture is absent.

use dvb_bbframe::crc::crc8;
use dvb_bbframe::header::{Bbheader, Mode};

const BBFRAME_PID: u16 = 0x010E;
const NEW_FRAME_COUNT: u8 = 0xB8; // section count byte marking a new BBFrame

/// Reassemble complete BBFrames carried in TS private sections on `pid`.
fn extract_bbframes(data: &[u8], pid: u16) -> Vec<Vec<u8>> {
    let mut frames = Vec::new();
    let mut current = Vec::with_capacity(8192);
    let mut started = false;

    for pkt in data.chunks(188) {
        if pkt.len() < 188 || pkt[0] != 0x47 {
            continue;
        }
        let ts_pid = ((u16::from(pkt[1]) & 0x1F) << 8) | u16::from(pkt[2]);
        if ts_pid != pid || pkt[3] & 0x30 != 0x10 {
            continue;
        }
        if pkt[4] != 0x00 || pkt[5] != 0x80 || pkt[6] != 0x00 {
            continue; // section header: 00 80 00 [slen] [count]
        }
        let slen = usize::from(pkt[7]);
        if slen == 0 || slen > 0xB4 {
            continue;
        }
        let data_end = 9 + (slen - 1);
        if data_end > 188 {
            continue;
        }
        if pkt[8] == NEW_FRAME_COUNT {
            if started && !current.is_empty() {
                frames.push(core::mem::take(&mut current));
            }
            started = true;
            current.clear();
        }
        if started {
            current.extend_from_slice(&pkt[9..data_end]);
        }
    }
    frames
}

fn main() {
    let path = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/fixtures/tnt-5w-12732v-bbframe.ts"
    );
    let data = match std::fs::read(path) {
        Ok(b) => b,
        Err(e) => {
            eprintln!("fixture not available ({e}); nothing to do");
            return;
        }
    };

    let frames = extract_bbframes(&data, BBFRAME_PID);
    println!(
        "PID {BBFRAME_PID:#06X}: {} BBFrames reassembled",
        frames.len()
    );

    let mut normal = 0;
    let mut crc_ok = 0;
    for frame in &frames {
        if frame.len() < 10 {
            continue;
        }
        let hdr = Bbheader::parse(frame).expect("BBHEADER parses");
        if hdr.mode == Mode::Normal {
            normal += 1;
        }
        // NM integrity: stored CRC-8 equals computed over the first 9 bytes.
        if crc8(&frame[..9]) == frame[9] {
            crc_ok += 1;
        }
    }
    println!("Normal-Mode headers : {normal}");
    println!("CRC-8 intact        : {crc_ok}/{}", frames.len());

    if let Some(first) = frames.first() {
        let hdr = Bbheader::parse(first).unwrap();
        println!(
            "first frame         : UPL={} bits, DFL={} bits, SYNC={:#04X}",
            hdr.upl, hdr.dfl, hdr.sync
        );
    }
}
