//! Allocation bound for the section collectors (audit r02-O1): a carousel
//! re-sends identical sections every cycle, so the steady state is "push a
//! section already held". That must not copy the section into a fresh `Arc`
//! just to compare it against the retained one and drop it.
//!
//! Uses the real NIT sections of a committed capture
//! (`fixtures/dvb-si/tnt-5w-12732v-isi6-10s.ts`, PID 0x0010).
#![cfg(feature = "ts")]

use dvb_si::collect::SectionSetCollector;
use mpeg_ts::pid::Pid;
use mpeg_ts::ts::{SectionReassembler, TS_PACKET_SIZE, TsPacket};
use test_alloc::ThreadCounting;

#[global_allocator]
static A: ThreadCounting = ThreadCounting::new();

fn nit_sections() -> Vec<Vec<u8>> {
    let path = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../fixtures/dvb-si/tnt-5w-12732v-isi6-10s.ts"
    );
    let data = std::fs::read(path).expect("read fixture");
    let mut reassembler = SectionReassembler::default();
    let mut sections = Vec::new();
    for chunk in data.chunks(TS_PACKET_SIZE) {
        let Ok(pkt) = TsPacket::parse(chunk) else {
            continue;
        };
        if pkt.header.pid != Pid::new(0x0010) {
            continue;
        }
        if let Some(payload) = pkt.payload {
            reassembler.feed(payload, pkt.header.pusi);
            while let Some(sec) = reassembler.pop_section() {
                sections.push(sec.to_vec());
            }
        }
    }
    sections
}

#[test]
fn pushing_an_already_held_section_does_not_copy_it() {
    let sections = nit_sections();
    assert!(!sections.is_empty(), "need a real NIT section");
    let mut c = SectionSetCollector::new();
    for s in &sections {
        let _ = c.push_section(s).expect("real NIT section");
    }
    // Steady state: every section of the cycle arrives again, byte-identical.
    let (_, snap) = ThreadCounting::measure(|| {
        for s in &sections {
            let _ = c.push_section(s).expect("duplicate is not an error");
        }
    });
    // The retained copy already exists, so a duplicate allocates nothing.
    assert_eq!(
        snap.allocs, 0,
        "duplicate pushes allocated {} bytes",
        snap.bytes
    );
}
