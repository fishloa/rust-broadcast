//! No-panic hostility sweep (audit S3): real subtitle `PES_data_field`s from
//! the committed M6 capture are truncated at every length and mutated at every
//! byte position, then parsed (and re-serialized when they still parse). The
//! contract is "`Ok` or `Err`, never a panic": subtitle segments carry
//! attacker-controlled lengths and pixel-code strings, which is where a
//! panicking slice/index on a parsed length would hide.

use std::fs;

use broadcast_common::{Parse, Serialize};
use dvb_subtitle::PesDataField;
use mpeg_pes::{PesAssembler, PesPacket, StreamId};

const PKT: usize = 188;
const SUBTITLE_PID: u16 = 0x008C;
const PRIVATE_STREAM_1: StreamId = StreamId(0xBD);
const DATA_IDENTIFIER_DVB_SUBTITLE: u8 = 0x20;
/// Distinct real fields swept (bounded so the test stays fast).
const MAX_SEEDS: usize = 16;

fn data_fields() -> Vec<Vec<u8>> {
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
        if out.len() >= MAX_SEEDS {
            break;
        }
    }
    out
}

fn exercise(bytes: &[u8]) -> bool {
    match PesDataField::parse(bytes) {
        Ok(f) => {
            let _ = f.try_to_bytes();
            true
        }
        Err(_) => false,
    }
}

#[test]
fn truncation_and_mutation_of_real_data_fields_never_panic() {
    let seeds = data_fields();
    assert!(!seeds.is_empty(), "no real subtitle data fields found");
    let mut parsed_after_mutation = 0usize;
    for seed in &seeds {
        assert!(exercise(seed), "seed must parse");
        for n in 0..=seed.len() {
            exercise(&seed[..n]);
        }
        for pos in 0..seed.len() {
            for v in [0x00u8, 0x01, 0x0F, 0x7F, 0x80, 0xFF, seed[pos] ^ 0x55] {
                let mut m = seed.clone();
                m[pos] = v;
                if exercise(&m) {
                    parsed_after_mutation += 1;
                }
            }
        }
    }
    assert!(parsed_after_mutation > 100, "{parsed_after_mutation}");
}
