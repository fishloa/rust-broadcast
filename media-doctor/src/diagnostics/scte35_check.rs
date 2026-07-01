//! `Scte35Check` — SCTE-35 splice insertion consistency diagnostic.
//!
//! Reassembles `splice_info_section` sections (table_id 0xFC) from the TS byte
//! stream and reports container-level splice consistency violations:
//!
//! - **Unbalanced `splice_insert`**: a `splice_insert` with
//!   `out_of_network_indicator == true` (an "out"/break start) that has no
//!   matching `out_of_network_indicator == false` (the "in"/return) with the
//!   same `splice_event_id` by end of stream → Warning.
//! - **Duplicate open out**: two "out" `splice_insert`s with the same
//!   `splice_event_id` and no intervening "in" → Warning.
//!
//! Events with `splice_event_cancel_indicator == true` are ignored (they cancel
//! the named event and neither open nor close a splice). Well-formed, balanced
//! out→in pairs produce no findings.

use alloc::collections::btree_map::{BTreeMap, Entry};

use broadcast_common::Parse;

use crate::report::{Finding, Location, Severity};
use crate::Diagnostic;
use crate::Report;
use mpeg_ts::ts::SectionReassembler;
use mpeg_ts::ts::{TsPacket, TS_PACKET_SIZE};

/// `table_id` of a SCTE-35 `splice_info_section` (ANSI/SCTE 35 §9.6.1).
const SCTE35_TABLE_ID: u8 = 0xFC;

/// Tracks the open/close state of a `splice_insert` event.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SpliceInsertState {
    /// An "out" (out_of_network_indicator == true) has been seen for this
    /// event_id, and no matching "in" has arrived yet.
    Open,
    /// A balanced out→in pair has been completed.
    Closed,
}

/// Per-PID tracking for SCTE-35 splice events.
#[derive(Default)]
struct Scte35PidState {
    /// Section reassembler for this PID.
    reassembler: SectionReassembler,
    /// Tracks the current state per splice_event_id.
    events: BTreeMap<u32, SpliceInsertState>,
}

/// Checks SCTE-35 splice insertion consistency across the stream.
///
/// Flags findings when:
/// - An "out" splice_insert has no matching "in" by stream end (Warning).
/// - A duplicate "out" splice_insert arrives without an intervening "in"
///   (Warning).
///
/// Balanced out→in pairs with the same splice_event_id produce no findings.
/// Cancelled events (`splice_event_cancel_indicator == true`) are ignored.
#[derive(Debug, Clone, Copy)]
pub struct Scte35Check;

/// PID on which SCTE-35 splice_info_section messages are typically carried.
const SCTE35_PID: u16 = 0x01F0;

impl Diagnostic for Scte35Check {
    fn run(&self, ts: &[u8], report: &mut Report) {
        let n_packets = ts.len() / TS_PACKET_SIZE;
        let mut pid_states: BTreeMap<u16, Scte35PidState> = BTreeMap::new();

        for i in 0..n_packets {
            let offset = i * TS_PACKET_SIZE;
            let raw = &ts[offset..offset + TS_PACKET_SIZE];

            let Ok(pkt) = TsPacket::parse(raw) else {
                continue;
            };

            let pid = pkt.header.pid;

            // Only watch the SCTE-35 PID.
            if pid != SCTE35_PID {
                continue;
            }

            let payload = match pkt.payload {
                Some(pl) => pl,
                None => continue,
            };

            let pusi = pkt.header.pusi;

            let state = pid_states.entry(pid).or_default();
            state.reassembler.feed(payload, pusi);

            // Drain completed sections.
            while let Some(section) = state.reassembler.pop_section() {
                let section_data = &section[..];

                // table_id must be 0xFC (splice_info_section).
                if section_data.is_empty() || section_data[0] != SCTE35_TABLE_ID {
                    continue;
                }

                // Parse with scte35_splice.
                let Ok(sis) = scte35_splice::SpliceInfoSection::parse(section_data) else {
                    continue;
                };

                let Some(ref clear) = sis.clear else {
                    continue;
                };

                let command = &clear.command;
                let scte35_splice::commands::AnyCommand::SpliceInsert(si) = command else {
                    continue;
                };

                // Ignore cancelled events.
                if si.splice_event_cancel_indicator {
                    continue;
                }

                let eid = si.splice_event_id;
                let oon = si.out_of_network_indicator;

                match state.events.entry(eid) {
                    Entry::Vacant(entry) => {
                        if oon {
                            // First encounter is an "out" → mark open.
                            entry.insert(SpliceInsertState::Open);
                        } else {
                            // First encounter is an "in" without a preceding
                            // "out" → this is a valid standalone return
                            // (already closed).
                            entry.insert(SpliceInsertState::Closed);
                        }
                    }
                    Entry::Occupied(mut entry) => {
                        match *entry.get() {
                            SpliceInsertState::Open => {
                                if oon {
                                    // Duplicate open "out" with no intervening
                                    // "in" — Warning.
                                    report.push(Finding::new(
                                        Severity::Warning,
                                        Location::new(i, pid),
                                        "scte35-dup-out",
                                        alloc::format!(
                                            "duplicate open splice_insert: out event_id {} \
                                             with no intervening in",
                                            eid,
                                        ),
                                    ));
                                } else {
                                    // Matching "in" — close the event.
                                    entry.insert(SpliceInsertState::Closed);
                                }
                            }
                            SpliceInsertState::Closed => {
                                if oon {
                                    // New "out" after a completed pair — reopen.
                                    entry.insert(SpliceInsertState::Open);
                                }
                                // Duplicate "in" after closed → ignore (no harm).
                            }
                        }
                    }
                }
            }
        }

        // End of stream: report any remaining open events.
        for (&pid, state) in pid_states.iter() {
            for (&eid, &status) in state.events.iter() {
                if status == SpliceInsertState::Open {
                    report.push(Finding::new(
                        Severity::Warning,
                        Location::new(n_packets.saturating_sub(1), pid),
                        "scte35-unbalanced",
                        alloc::format!(
                            "unbalanced splice_insert: out event_id {} with no matching in",
                            eid,
                        ),
                    ));
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::report::Report;

    /// Build a TS packet with SCTE-35 sections on the given PID.
    /// PUSI is set in byte 1, payload starts at offset 4.
    fn make_packet(payload: &[u8], pid: u16, cc: u8) -> Vec<u8> {
        let mut pkt = vec![0x47u8; 188];
        pkt[1] = 0x40 | (((pid >> 8) as u8) & 0x1F); // PUSI=1, PID high
        pkt[2] = (pid & 0xFF) as u8;
        pkt[3] = 0x10 | (cc & 0x0F); // AFC=01 payload-only, CC
        let write_end = 4 + payload.len().min(184);
        pkt[4..write_end].copy_from_slice(payload);
        pkt
    }

    /// Build a minimal valid SCTE-35 splice_info_section bytes containing a
    /// splice_insert command. Returns the section body (with full MPEG section
    /// header and CRC32).
    fn make_splice_insert_section(event_id: u32, out_of_network: bool, cancel: bool) -> Vec<u8> {
        use broadcast_common::Serialize;
        use scte35_splice::commands::SpliceInsert;
        use scte35_splice::SpliceInfoSection;

        let si = SpliceInsert {
            splice_event_id: event_id,
            splice_event_cancel_indicator: cancel,
            out_of_network_indicator: out_of_network,
            program_splice_flag: true,
            splice_immediate_flag: true,
            ..SpliceInsert::default()
        };

        let command = scte35_splice::commands::AnyCommand::SpliceInsert(si);
        let sis = SpliceInfoSection::new_clear(command, &[]);
        let mut buf = vec![0u8; sis.serialized_len()];
        sis.serialize_into(&mut buf).unwrap();
        buf
    }

    /// A clean PID with no SCTE-35 sections should produce zero findings.
    #[test]
    fn empty_pid_no_findings() {
        let mut ts = Vec::new();
        for _ in 0..3 {
            let mut pkt = vec![0x47u8; 188];
            pkt[1] = 0x01;
            pkt[2] = 0xF0; // PID 0x01F0
            pkt[3] = 0x10; // AFC=01, CC=0
            ts.extend_from_slice(&pkt);
        }
        let mut report = Report::new();
        Scte35Check.run(&ts, &mut report);
        assert!(
            report.is_empty(),
            "expected no findings, got {:?}",
            report.findings()
        );
    }

    /// A balanced out→in pair should produce zero findings.
    #[test]
    fn balanced_pair_no_findings() {
        let pid = 0x01F0u16;
        let out_bytes = make_splice_insert_section(100, true, false);
        let in_bytes = make_splice_insert_section(100, false, false);

        let mut payload = Vec::new();
        payload.push(0x00); // pointer_field
        payload.extend_from_slice(&out_bytes);
        payload.extend_from_slice(&in_bytes);

        let ts = make_packet(&payload, pid, 0);
        let mut report = Report::new();
        Scte35Check.run(&ts, &mut report);
        assert!(
            report.is_empty(),
            "balanced pair should have no findings, got {:?}",
            report.findings()
        );
    }

    /// A single "out" with no matching "in" should produce an unbalanced
    /// finding.
    #[test]
    fn unbalanced_out_produces_finding() {
        let pid = 0x01F0u16;
        let out_bytes = make_splice_insert_section(42, true, false);

        let mut payload = Vec::new();
        payload.push(0x00);
        payload.extend_from_slice(&out_bytes);

        let ts = make_packet(&payload, pid, 0);
        let mut report = Report::new();
        Scte35Check.run(&ts, &mut report);
        let unbal: Vec<_> = report
            .findings()
            .iter()
            .filter(|f| f.rule_id == "scte35-unbalanced")
            .collect();
        assert_eq!(
            unbal.len(),
            1,
            "expected 1 unbalanced finding for event_id 42, got {:?}",
            report.findings()
        );
        assert!(
            unbal[0].message.contains("42"),
            "message should reference event_id 42: {}",
            unbal[0].message
        );
    }

    /// Duplicate open "out" (same event_id, no intervening "in") should
    /// produce a duplicate-out finding.
    #[test]
    fn duplicate_out_produces_finding() {
        let pid = 0x01F0u16;
        let out1 = make_splice_insert_section(7, true, false);
        let out2 = make_splice_insert_section(7, true, false);

        let mut payload = Vec::new();
        payload.push(0x00);
        payload.extend_from_slice(&out1);
        payload.extend_from_slice(&out2);

        let ts = make_packet(&payload, pid, 0);
        let mut report = Report::new();
        Scte35Check.run(&ts, &mut report);
        let dup: Vec<_> = report
            .findings()
            .iter()
            .filter(|f| f.rule_id == "scte35-dup-out")
            .collect();
        assert_eq!(
            dup.len(),
            1,
            "expected 1 duplicate-out finding for event_id 7, got {:?}",
            report.findings()
        );
        assert!(
            dup[0].message.contains("7"),
            "message should reference event_id 7: {}",
            dup[0].message
        );
    }

    /// Cancelled events must not be tracked.
    #[test]
    fn cancelled_event_ignored() {
        let pid = 0x01F0u16;
        let cancel = make_splice_insert_section(99, true, true);

        let mut payload = Vec::new();
        payload.push(0x00);
        payload.extend_from_slice(&cancel);

        let ts = make_packet(&payload, pid, 0);
        let mut report = Report::new();
        Scte35Check.run(&ts, &mut report);
        assert!(
            report.is_empty(),
            "cancelled event should produce no findings, got {:?}",
            report.findings()
        );
    }
}
