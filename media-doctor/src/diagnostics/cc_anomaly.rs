//! `CcAnomalyCheck` — flags continuity_counter anomalies per PID.
//!
//! ITU-T H.222.0 §2.4.3.3: the 4-bit continuity_counter increments by 1 (mod 16)
//! for each payload-bearing packet (adaptation_field_control `01` or `11`) of the
//! same PID. An anomaly is flagged when a packet's CC is not the expected +1 AND
//! it is **not** a legal duplicate (same CC + byte-identical payload except
//! re-encoded PCR) AND **not** a signalled discontinuity
//! (discontinuity_indicator == 1).
//!
//! Non-payload-bearing packets (AFC `00` or `10`) do not advance the CC.

use alloc::collections::btree_map::BTreeMap;
use alloc::vec::Vec;

use crate::Diagnostic;
use crate::Report;
use crate::report::{Finding, Location, Severity};
use mpeg_ts::ts::{TS_PACKET_SIZE, TsPacket};

/// Per-PID continuity counter state.
#[derive(Debug, Clone)]
struct CcState {
    /// Whether we've seen the first payload-bearing packet for this PID.
    initialized: bool,
    /// Last continuity counter value on this PID.
    last_cc: u8,
    /// Payload bytes of the last packet (for duplicate detection).
    last_payload: Vec<u8>,
}

/// Checks each PID's continuity_counter for anomalies per §2.4.3.3.
///
/// Flags an Error finding when the CC sequence is broken, excluding:
/// - Legal duplicates (same CC + identical payload, except re-encoded PCR)
/// - Signalled discontinuities (discontinuity_indicator == 1)
#[derive(Debug, Clone, Copy)]
pub struct CcAnomalyCheck;

impl Diagnostic for CcAnomalyCheck {
    fn run(&self, ts: &[u8], report: &mut Report) {
        let n_packets = ts.len() / TS_PACKET_SIZE;
        let mut pid_states: BTreeMap<u16, CcState> = BTreeMap::new();

        for i in 0..n_packets {
            let offset = i * TS_PACKET_SIZE;
            let raw = &ts[offset..offset + TS_PACKET_SIZE];

            let Ok(pkt) = TsPacket::parse(raw) else {
                continue;
            };

            let hdr = &pkt.header;
            let pid = hdr.pid;

            // Skip null packets — CC is undefined (§2.4.3.3).
            if pid == 0x1FFF {
                continue;
            }

            // Only payload-bearing packets (AFC 01 or 11) interact with CC.
            if !hdr.has_payload {
                continue;
            }

            let cc = hdr.continuity_counter;
            let state = pid_states.entry(pid).or_insert(CcState {
                initialized: false,
                last_cc: 0,
                last_payload: Vec::new(),
            });

            if !state.initialized {
                // First payload-bearing packet for this PID — just record state.
                state.initialized = true;
                state.last_cc = cc;
                state.last_payload = pkt.payload.unwrap_or(&[]).to_vec();
                continue;
            }

            let expected = (state.last_cc + 1) & 0x0F;

            // Check discontinuity indicator.
            let has_discontinuity = if hdr.has_adaptation {
                let af_len = raw[4] as usize;
                af_len > 0 && (raw[5] & 0x80) != 0
            } else {
                false
            };

            if cc != expected && !has_discontinuity {
                // CC doesn't match — check if it's a legal duplicate.
                let cur_payload = pkt.payload.unwrap_or(&[]);
                let is_dup = cc == state.last_cc
                    && cur_payload.len() == state.last_payload.len()
                    && cur_payload == state.last_payload.as_slice();

                if !is_dup {
                    report.push(Finding::new(
                        Severity::Error,
                        Location::new(i, pid),
                        "cc-anomaly",
                        alloc::format!(
                            "PID 0x{pid:04X}: expected CC={expected}, got CC={cc} \
                             (not a legal duplicate or signalled discontinuity)"
                        ),
                    ));
                }
            }

            // Update state for this PID.
            state.last_cc = cc;
            state.last_payload = pkt.payload.unwrap_or(&[]).to_vec();
        }
    }
}
