//! Real-capture regression for issue #1035: a conformant multiplex must not
//! raise `Buffer_error` (TR 101 290 indicator 3.3).
//!
//! Uses the real French TNT UHF-32 off-air multiplex from the private
//! fixtures repo: `private/fixtures/ts/france-tnt-uhf32-video-w30MB-10MB.ts`, a
//! 10 MiB video-intact window of the capture opening at a PAT packet (the test
//! skips loudly when `private/` is not checked out). Window choice is measured:
//! with the #1035 fix reverted the first `Buffer_error` appears 38-39 MiB into
//! the capture, and only windows opening early enough to warm the buffer
//! model (30 MiB+10 MiB reproduces it; 34+8 and 36+6 do not, nor do prefixes
//! under 39 MiB) fail, so this window keeps the test biting at 10 MiB instead
//! of the former 60 MB read. Video is intact because TBn buffers of the video
//! PIDs are part of the model. Packet arrival times are PCR-derived (a
//! constant rate from the first-to-last PCR of the first PCR PID — ~24
//! Mbit/s), not invented. Before the fix, the EIT PID (0x0012), which is DVB
//! SI and not PSI, was fed into TBsys; its back-to-back packets overflowed
//! the 512-byte / 1 Mbit/s buffer seven times in the first 20 s.

use core::time::Duration;
use dvb_conformance::{ConformanceMonitor, Indicator};
use std::io::Read;

const TS_PACKET: usize = 188;
/// ~20 s of the capture at its ~24 Mbit/s rate.
const READ_BYTES: usize = 60_000_000;

/// 27 MHz PCR of a packet whose adaptation field carries one, with its PID.
fn pcr_of(p: &[u8]) -> Option<(u16, u64)> {
    if p[3] & 0x20 == 0 || p[4] < 7 || p[5] & 0x10 == 0 {
        return None;
    }
    let pid = (u16::from(p[1] & 0x1F) << 8) | u16::from(p[2]);
    let base = (u64::from(p[6]) << 25)
        | (u64::from(p[7]) << 17)
        | (u64::from(p[8]) << 9)
        | (u64::from(p[9]) << 1)
        | (u64::from(p[10]) >> 7);
    let ext = (u64::from(p[10] & 1) << 8) | u64::from(p[11]);
    Some((pid, base * 300 + ext))
}

#[test]
fn real_multiplex_raises_no_buffer_error() {
    let Some(path) = test_fixtures::require(
        "ts/france-tnt-uhf32-video-w30MB-10MB.ts",
        "tbsys_real_capture",
    ) else {
        return;
    };
    let f = std::fs::File::open(&path).expect("open capture");
    let mut data = Vec::new();
    f.take(READ_BYTES as u64)
        .read_to_end(&mut data)
        .expect("read capture prefix");
    data.truncate(data.len() / TS_PACKET * TS_PACKET);
    let n = data.len() / TS_PACKET;

    let mut first: Option<(usize, u16, u64)> = None;
    let mut last: Option<(usize, u64)> = None;
    for i in 0..n {
        if let Some((pid, pcr)) = pcr_of(&data[i * TS_PACKET..(i + 1) * TS_PACKET]) {
            match first {
                None => first = Some((i, pid, pcr)),
                Some((_, fp, _)) if fp == pid => last = Some((i, pcr)),
                _ => {}
            }
        }
    }
    let (fi, _, fpcr) = first.expect("capture carries PCR");
    let (li, lpcr) = last.expect("PCR PID repeats");
    let per_packet = (lpcr - fpcr) as f64 / 27e6 / (li - fi) as f64;

    let mut monitor = ConformanceMonitor::new();
    let mut buffer_errors = Vec::new();
    for i in 0..n {
        let t = Duration::from_secs_f64(i as f64 * per_packet);
        for e in monitor.feed(&data[i * TS_PACKET..(i + 1) * TS_PACKET], t) {
            if e.indicator == Indicator::BufferError {
                buffer_errors.push((e.pid, e.detail.clone()));
            }
        }
    }
    assert!(
        buffer_errors.is_empty(),
        "a conformant multiplex raised Buffer_error: {buffer_errors:?}"
    );
}
