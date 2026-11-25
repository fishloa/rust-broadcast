//! Advanced: walk a real DVB-S2 capture, reassemble BBFrames from TS private
//! sections, and parse every BBHEADER.
//!
//! Run with: `cargo run -p dvb-bbframe --example walk_capture`
//!
//! Reads the committed `tnt-5w-12732v-bbframe.ts` fixture at runtime, one TS
//! packet at a time, so the capture is never held whole.

use std::fs::File;
use std::io::{self, Read};

use dvb_bbframe::crc::crc8;
use dvb_bbframe::header::{Bbheader, Mode};

const BBFRAME_PID: u16 = 0x010E;

/// Length of one MPEG-2 TS packet (ISO/IEC 13818-1 §2.4.3.2).
const TS_PACKET_LEN: usize = 188;

/// Read granularity for streaming the capture.
const READ_CHUNK_LEN: usize = 1 << 20;

/// Fill `packet` from `reader`; `false` at a clean end of stream.
fn read_packet(reader: &mut impl Read, packet: &mut [u8; TS_PACKET_LEN]) -> io::Result<bool> {
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
const NEW_FRAME_COUNT: u8 = 0xB8; // section count byte marking a new BBFrame

/// Reassembles BBFrames carried in TS private sections on one PID.
struct BbFrameAssembler {
    /// The section payload of the frame under assembly.
    current: Vec<u8>,
    /// Whether a first section has been seen.
    started: bool,
}

impl BbFrameAssembler {
    fn new() -> Self {
        Self {
            current: Vec::with_capacity(8192),
            started: false,
        }
    }

    /// Feed one 188-byte TS packet, returning a frame if this packet completed
    /// one.
    fn feed(&mut self, pkt: &[u8]) -> Option<Vec<u8>> {
        if pkt.len() < 188 || pkt[0] != 0x47 {
            return None;
        }
        let ts_pid = ((u16::from(pkt[1]) & 0x1F) << 8) | u16::from(pkt[2]);
        if ts_pid != BBFRAME_PID || pkt[3] & 0x30 != 0x10 {
            return None;
        }
        if pkt[4] != 0x00 || pkt[5] != 0x80 || pkt[6] != 0x00 {
            return None; // section header: 00 80 00 [slen] [count]
        }
        let slen = usize::from(pkt[7]);
        if slen == 0 || slen > 0xB4 {
            return None;
        }
        let data_end = 9 + (slen - 1);
        if data_end > 188 {
            return None;
        }
        let mut completed = None;
        if pkt[8] == NEW_FRAME_COUNT {
            if self.started && !self.current.is_empty() {
                completed = Some(core::mem::take(&mut self.current));
            }
            self.started = true;
            self.current.clear();
        }
        if self.started {
            self.current.extend_from_slice(&pkt[9..data_end]);
        }
        completed
    }
}

fn main() {
    // Fixtures live in the workspace-shared `fixtures/` tree, not under the
    // crate. A committed fixture that cannot be read is a bug, not a reason
    // to skip — so a missing/unreadable fixture is a hard failure.
    let path = test_fixtures::path("bbframe/tnt-5w-12732v-bbframe.ts");
    let file = File::open(&path).unwrap_or_else(|e| {
        panic!(
            "committed fixture {} could not be read: {e}",
            path.display()
        )
    });

    let mut assembler = BbFrameAssembler::new();
    let mut frames = 0usize;
    let mut normal = 0usize;
    let mut crc_ok = 0usize;
    let mut first_header: Option<(u16, u16, u8)> = None;

    let mut reader = io::BufReader::with_capacity(READ_CHUNK_LEN, file);
    let mut packet = [0u8; TS_PACKET_LEN];
    while read_packet(&mut reader, &mut packet)
        .unwrap_or_else(|e| panic!("fixture read failed: {e}"))
    {
        let Some(frame) = assembler.feed(&packet) else {
            continue;
        };
        frames += 1;
        if frame.len() < 10 {
            continue;
        }
        let hdr = Bbheader::parse(&frame).expect("BBHEADER parses");
        if hdr.mode == Mode::Normal {
            normal += 1;
        }
        // NM integrity: stored CRC-8 equals computed over the first 9 bytes.
        if crc8(&frame[..9]) == frame[9] {
            crc_ok += 1;
        }
        first_header.get_or_insert((hdr.upl, hdr.dfl, hdr.sync));
    }

    println!("PID {BBFRAME_PID:#06X}: {frames} BBFrames reassembled");
    println!("Normal-Mode headers : {normal}");
    println!("CRC-8 intact        : {crc_ok}/{frames}");

    if let Some((upl, dfl, sync)) = first_header {
        println!("first frame         : UPL={upl} bits, DFL={dfl} bits, SYNC={sync:#04X}");
    }
}
