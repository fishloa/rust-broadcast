#![no_main]

use libfuzzer_sys::fuzz_target;

const TS_PACKET_SIZE: usize = 188;
const SYNC_BYTE: u8 = 0x47;

// Fuzz `StreamingTsDemux` with well-formed-ish 188-byte packets (T5): each
// chunk's sync byte is forced to `0x47` so the packet-boundary resync in
// `mpeg_ts::resync` locks immediately, letting the fuzzer spend its budget on
// what's actually new on this branch — `parse_pat`, `parse_pmt_section_header`,
// the PMT-diff state machine (add/remove/reclassify a stream across PMT
// versions), and the new CRC path — rather than mostly re-discovering "find
// the next 0x47". The remaining 187 bytes of each packet (PID, flags,
// continuity counter, adaptation field, PSI/PES payload) stay fully
// arbitrary, so PAT/PMT section bytes, CRC words, and PES headers are all
// fuzzer-controlled.
fuzz_target!(|data: &[u8]| {
    let mut d = transmux::StreamingTsDemux::new();
    let mut packet = [0xFFu8; TS_PACKET_SIZE];
    for chunk in data.chunks(TS_PACKET_SIZE - 1) {
        packet[0] = SYNC_BYTE;
        packet[1..1 + chunk.len()].copy_from_slice(chunk);
        // Any bytes beyond a short final chunk stay `0xFF` stuffing from the
        // previous iteration's leftovers being overwritten in place; zero the
        // tail explicitly so no fuzz iteration ever cross-contaminates the
        // next with left-over data from a longer prior chunk.
        for b in packet[1 + chunk.len()..].iter_mut() {
            *b = 0xFF;
        }
        d.feed(&packet);
        while d.poll_event().is_some() {}
    }
    d.finish();
    while d.poll_event().is_some() {}
});
