//! `SyncByteCheck` — flags TS packets whose first byte is not the sync byte
//! (ITU-T H.222.0 §2.4.3.2).

use crate::report::{Finding, Location, Severity};
use crate::Diagnostic;
use crate::Report;

/// Checks that every 188-byte TS packet starts with `0x47` (the MPEG-TS sync byte).
///
/// ITU-T H.222.0 §2.4.3.2: each TS packet begins with a `sync_byte` of `0x47`.
/// Any deviation indicates corruption, stream misalignment, or a non-TS stream.
#[derive(Debug, Clone, Copy)]
pub struct SyncByteCheck;

impl Diagnostic for SyncByteCheck {
    fn run(&self, ts: &[u8], report: &mut Report) {
        const SYNC: u8 = 0x47;
        const PACKET_SIZE: usize = 188;

        let n_packets = ts.len() / PACKET_SIZE;
        for i in 0..n_packets {
            let offset = i * PACKET_SIZE;
            if ts[offset] != SYNC {
                report.push(Finding::new(
                    Severity::Error,
                    Location::new(
                        i,
                        u16::from_be_bytes([ts[offset + 1], ts[offset + 2]]) & 0x1FFF,
                    ),
                    "sync-byte",
                    alloc::format!(
                        "Expected sync byte 0x47 at start of packet {}, found 0x{:02X}",
                        i,
                        ts[offset]
                    ),
                ));
            }
        }
    }
}
