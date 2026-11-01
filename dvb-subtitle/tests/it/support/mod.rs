//! Shared test support: the real subtitle `PES_data_field`s of the committed
//! M6 capture.
#![allow(dead_code)]

use std::fs;

use mpeg_pes::{PesAssembler, PesPacket, StreamId};

const PKT: usize = 188;
const SUBTITLE_PID: u16 = 0x008C;
const PRIVATE_STREAM_1: StreamId = StreamId(0xBD);
const DATA_IDENTIFIER_DVB_SUBTITLE: u8 = 0x20;

pub fn data_fields(max_seeds: usize) -> Vec<Vec<u8>> {
    let ts = fs::read(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../fixtures/ts/m6-single.ts"
    ))
    .expect("fixture m6-single.ts must be present");
    let mut asm = PesAssembler::new();
    let mut pes = Vec::new();
    for p in ts.chunks(PKT) {
        if p.len() != PKT || p[0] != 0x47 {
            continue;
        }
        let pid = (u16::from(p[1] & 0x1F) << 8) | u16::from(p[2]);
        if pid != SUBTITLE_PID {
            continue;
        }
        let start = match (p[3] >> 4) & 0x03 {
            1 => 4,
            3 => 5 + usize::from(p[4]),
            _ => continue,
        };
        if start >= PKT {
            continue;
        }
        if let Some(v) = asm.feed(p[1] & 0x40 != 0, &p[start..]) {
            pes.push(v);
        }
    }
    if let Some(v) = asm.flush() {
        pes.push(v);
    }
    let mut out: Vec<Vec<u8>> = Vec::new();
    for raw in &pes {
        let pkt = PesPacket::parse(raw).expect("subtitle PES parses");
        if pkt.stream_id == PRIVATE_STREAM_1
            && pkt.payload.first() == Some(&DATA_IDENTIFIER_DVB_SUBTITLE)
            && !out.iter().any(|o| o == pkt.payload)
        {
            out.push(pkt.payload.to_vec());
        }
        if out.len() >= max_seeds {
            break;
        }
    }
    out
}
