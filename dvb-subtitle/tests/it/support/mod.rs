//! Shared test support: the real subtitle `PES_data_field`s of the committed
//! M6 capture.

use std::fs;

use mpeg_pes::{PesAssembler, StreamId};

const PKT: usize = 188;
const SUBTITLE_PID: u16 = 0x008C;
const PRIVATE_STREAM_1: StreamId = StreamId::new(0xBD);
const DATA_IDENTIFIER_DVB_SUBTITLE: u8 = 0x20;

/// The `private_stream_1` payload of a reassembled PES unit, or `None` for
/// another `stream_id`: the 3 fixed header bytes at offset 6,
/// `PES_header_data_length` at offset 8, then the payload (ISO/IEC 13818-1
/// Table 2-21), read directly because this capture's units are truncated.
fn subtitle_payload(raw: &[u8]) -> Option<&[u8]> {
    if raw.get(3) != Some(&PRIVATE_STREAM_1.get()) {
        return None;
    }
    let hdl = usize::from(*raw.get(8)?);
    raw.get(9 + hdl..)
}

pub fn data_fields(max_seeds: usize) -> Vec<Vec<u8>> {
    let ts = fs::read(test_fixtures::path("ts/m6-single.ts"))
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
        // The subtitle data field is the bytes after the PES header, read
        // directly: this capture joins mid-PES so its units are truncated and
        // `PesPacket::parse` reports that (#2746).
        let Some(payload) = subtitle_payload(raw) else {
            continue;
        };
        if payload.first() == Some(&DATA_IDENTIFIER_DVB_SUBTITLE)
            && !out.iter().any(|o| o == payload)
        {
            out.push(payload.to_vec());
        }
        if out.len() >= max_seeds {
            break;
        }
    }
    out
}
