//! Advanced: pump a real T2-MI stream out of an MPEG-TS capture and tally the
//! payload types (BBFrames, L1 signalling, timestamps).
//!
//! Run with: `cargo run -p dvb-t2mi --example pump_capture` (needs the default
//! `ts` feature). Reads the committed `colombia-capital-t2mi.ts` fixture at
//! runtime.

use dvb_t2mi::payload::AnyPayload;
use dvb_t2mi::pump::T2miPump;

const T2MI_PID: u16 = 0x0040;
const PKT: usize = 188;

fn main() {
    // Fixtures live in the workspace-shared `fixtures/` tree, not under the
    // crate. A committed fixture that cannot be read is a bug, not a reason
    // to skip — so a missing/unreadable fixture is a hard failure.
    let path = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../fixtures/dvb-t2mi/colombia-capital-t2mi.ts"
    );
    let data = std::fs::read(path)
        .unwrap_or_else(|e| panic!("committed fixture {path} could not be read: {e}"));

    let mut pump = T2miPump::new(T2MI_PID);
    let (mut bbframes, mut l1, mut timestamps, mut other) = (0u32, 0u32, 0u32, 0u32);

    for pkt in data.chunks(PKT) {
        if pkt.len() < PKT {
            break;
        }
        for event in pump.feed_ts(pkt) {
            match event.payload() {
                Ok(AnyPayload::Bbframe(_)) => bbframes += 1,
                Ok(AnyPayload::L1Current(_)) => l1 += 1,
                Ok(AnyPayload::Timestamp(_)) => timestamps += 1,
                Ok(_) => other += 1,
                Err(_) => {}
            }
        }
    }

    println!("T2-MI on PID {T2MI_PID:#06X}:");
    println!("  BBFrame payloads   : {bbframes}");
    println!("  L1-current packets : {l1}");
    println!("  timestamp packets  : {timestamps}");
    println!("  other payloads     : {other}");
    println!("  CRC-32 failures    : {}", pump.stats().crc_failures);
}
