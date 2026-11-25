//! `SyncByteCheck` — flags TS packets whose first byte is not the sync byte
//! (ITU-T H.222.0 §2.4.3.3).

use crate::report::{Finding, Location, Severity};
use crate::{Diagnostic, DiagnosticScan, PmtStreamIndex, Report};
use alloc::boxed::Box;

/// Offset of the PID's high byte within a TS packet header
/// (ISO/IEC 13818-1 §2.4.3.2).
const TS_PID_HIGH_OFFSET: usize = 1;
/// Offset of the PID's low byte.
const TS_PID_LOW_OFFSET: usize = 2;

/// Checks that every 188-byte TS packet starts with `0x47` (the MPEG-TS sync byte).
///
/// ITU-T H.222.0 §2.4.3.3: each TS packet begins with a `sync_byte` of `0x47`.
/// Any deviation indicates corruption, stream misalignment, or a non-TS stream.
#[derive(Debug, Clone, Copy)]
pub struct SyncByteCheck;

impl Diagnostic for SyncByteCheck {
    fn scan(&self, _pmt: &PmtStreamIndex) -> Box<dyn DiagnosticScan> {
        Box::new(SyncByteScan { index: 0 })
    }
}

/// One streaming pass of [`SyncByteCheck`].
struct SyncByteScan {
    index: usize,
}

impl DiagnosticScan for SyncByteScan {
    fn feed(&mut self, packet: &[u8], report: &mut Report) {
        use mpeg_ts::pid::Pid;
        use mpeg_ts::ts::TS_SYNC_BYTE as SYNC;

        let i = self.index;
        self.index += 1;
        if packet[0] != SYNC {
            report.push(Finding::new(
                Severity::Error,
                Location::at_pid(
                    i,
                    Pid::from_wire_masked(u16::from_be_bytes([
                        packet[TS_PID_HIGH_OFFSET],
                        packet[TS_PID_LOW_OFFSET],
                    ])),
                ),
                "sync-byte",
                alloc::format!(
                    "Expected sync byte 0x47 at start of packet {}, found 0x{:02X}",
                    i,
                    packet[0]
                ),
            ));
        }
    }
}
