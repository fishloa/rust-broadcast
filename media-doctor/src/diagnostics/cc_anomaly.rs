//! `CcAnomalyCheck` — flags continuity_counter anomalies per PID.
//!
//! ITU-T H.222.0 §2.4.3.3 (cited via `mpeg-ts/docs/README.md`): the 4-bit
//! continuity_counter increments by 1 (mod 16) for each payload-bearing
//! packet (adaptation_field_control `01` or `11`) of the same PID. A repeat
//! of the same CC is legal only as "two, and only two" consecutive
//! packets, byte-identical to the original except for a re-encoded PCR
//! field — a third consecutive repeat is itself a continuity fault, not a
//! legal duplicate. An anomaly is flagged when a packet's CC is not the
//! expected +1 AND it is **not** a legal duplicate (or is a second,
//! illegal, consecutive repeat) AND **not** a signalled discontinuity
//! (discontinuity_indicator == 1).
//!
//! The legal-duplicate byte-identity and cardinality rules are owned by
//! [`broadcast_common::ts_dup`] — shared with `dvb-conformance` and
//! `ts-fix`, which independently implement the same §2.4.3.3 clause.
//!
//! Non-payload-bearing packets (AFC `00` or `10`) do not advance the CC.

use alloc::collections::btree_map::BTreeMap;
use alloc::vec::Vec;

use broadcast_common::ts_dup::{DuplicateVerdict, check_duplicate};

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
    /// Raw bytes of the last payload-bearing packet seen on this PID. Used
    /// with [`broadcast_common::ts_dup::check_duplicate`] for the
    /// §2.4.3.3 legal-duplicate byte-identity rule (that function owns
    /// locating and exempting the PCR field).
    last_packet: Vec<u8>,
    /// Whether the one legal duplicate repeat of `last_packet` has already
    /// been consumed (§2.4.3.3 "two, and only two").
    dup_used: bool,
}

/// Checks each PID's continuity_counter for anomalies per §2.4.3.3.
///
/// Flags an Error finding when the CC sequence is broken, excluding:
/// - Legal duplicates (same CC + byte-identical packet, PCR excepted) — but
///   only the first repeat; a second consecutive repeat is itself flagged.
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
                last_packet: Vec::new(),
                dup_used: false,
            });

            if !state.initialized {
                // First payload-bearing packet for this PID — just record state.
                state.initialized = true;
                state.last_cc = cc;
                state.last_packet = raw.to_vec();
                continue;
            }

            // Check discontinuity indicator.
            let has_discontinuity = if hdr.has_adaptation {
                let af_len = raw[4] as usize;
                af_len > 0 && (raw[5] & 0x80) != 0
            } else {
                false
            };

            if has_discontinuity {
                // §2.4.3.5: discontinuity_indicator == 1 legalises any CC
                // value here; reset the duplicate-run tracking too.
                state.last_cc = cc;
                state.last_packet = raw.to_vec();
                state.dup_used = false;
                continue;
            }

            match check_duplicate(&state.last_packet, raw, state.dup_used) {
                DuplicateVerdict::Legal => {
                    // Byte-identical (PCR excepted) to the predecessor —
                    // leave last_cc/last_packet in place, just record that
                    // the one legal repeat has now been used.
                    state.dup_used = true;
                }
                DuplicateVerdict::IllegalThirdRepeat => {
                    report.push(Finding::new(
                        Severity::Error,
                        Location::new(i, pid),
                        "cc-anomaly",
                        alloc::format!(
                            "PID 0x{pid:04X}: third consecutive repeat of CC={cc} \
                             (§2.4.3.3 permits two, and only two)"
                        ),
                    ));
                    state.dup_used = true;
                }
                DuplicateVerdict::NotDuplicate => {
                    let expected = (state.last_cc + 1) & 0x0F;
                    if cc != expected {
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
                    state.dup_used = false;
                    state.last_cc = cc;
                    state.last_packet = raw.to_vec();
                }
                // `DuplicateVerdict` is `#[non_exhaustive]`; there is no
                // fourth variant today, but a future addition must not
                // silently fall through as "not a duplicate".
                _ => unreachable!("unhandled DuplicateVerdict variant"),
            }
        }
    }
}
