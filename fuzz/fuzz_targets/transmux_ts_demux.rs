#![no_main]

use libfuzzer_sys::fuzz_target;

// Fuzz `StreamingTsDemux` (transmux/src/ts_demux.rs) on raw arbitrary bytes
// (T5, test-integrity wave: +1361 lines of new TS parsing — `parse_pat`,
// `parse_pmt_section_header`, the PMT-diff state machine, the new CRC path —
// landed on this branch with zero fuzz coverage). `StreamingTsDemux::feed`
// takes untrusted transport-layer input (a live capture, a hostile stream)
// and must never panic on any byte sequence, however truncated, misaligned,
// or adversarial. Exercises the raw resync path: most chunks here will NOT
// start on a `0x47` sync byte, so this mostly drives the packet-boundary
// resync logic; see `transmux_ts_demux_packets.rs` for a variant that forces
// sync-byte alignment to reach the PAT/PMT/PES parsing paths behind it.
fuzz_target!(|data: &[u8]| {
    let mut d = transmux::StreamingTsDemux::new();
    d.feed(data);
    while d.poll_event().is_some() {}
    d.finish();
    while d.poll_event().is_some() {}
});
