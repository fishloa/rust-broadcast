//! The transport-layer helper the real-capture tests share: one file's PES
//! payloads per PID, assembled through `mpeg-ts` / `mpeg-pes`.

use mpeg_pes::{PesAssembler, PesPacket};
use mpeg_ts::ts::TsPacket;
use std::collections::BTreeMap;

/// One TS packet is 188 bytes (ISO/IEC 13818-1 §2.4.3.2).
const TS_PACKET_SIZE: usize = 188;

/// Every PID's assembled PES payloads, in file order.
pub fn pes_payloads(file: &[u8]) -> BTreeMap<u16, Vec<u8>> {
    let mut assemblers: BTreeMap<u16, PesAssembler> = BTreeMap::new();
    let mut out: BTreeMap<u16, Vec<u8>> = BTreeMap::new();
    for packet in file.chunks_exact(TS_PACKET_SIZE) {
        let Ok(ts) = TsPacket::parse(packet) else {
            continue;
        };
        let Some(payload) = ts.payload else {
            continue;
        };
        let pid = ts.header.pid.value();
        let assembler = assemblers.entry(pid).or_default();
        let Some(pes) = assembler.feed(ts.header.pusi, payload) else {
            continue;
        };
        let Ok(packet) = PesPacket::parse(&pes) else {
            continue;
        };
        out.entry(pid)
            .or_default()
            .extend_from_slice(packet.payload);
    }
    for (pid, mut assembler) in assemblers {
        let Some(pes) = assembler.flush() else {
            continue;
        };
        let Ok(packet) = PesPacket::parse(&pes) else {
            continue;
        };
        out.entry(pid)
            .or_default()
            .extend_from_slice(packet.payload);
    }
    out
}
