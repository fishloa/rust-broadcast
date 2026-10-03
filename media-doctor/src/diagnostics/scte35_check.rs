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

use alloc::collections::btree_map::BTreeMap;

use dvb_si::tables::pmt::StreamType;

use crate::Diagnostic;
use crate::Report;
use crate::diagnostics::codec_common::{collect_pmt_streams, pids_with_stream_type};
use crate::diagnostics::scte35_track::SpliceTracker;
use crate::report::{Finding, Location, Severity};
use mpeg_ts::ts::{TS_PACKET_SIZE, TsPacket};

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

/// Conventional PID SCTE-35 is *commonly* carried on — but never guaranteed:
/// ANSI/SCTE 35 assigns no fixed PID, the PMT declares the real one with
/// `stream_type 0x86` (or a `registration_descriptor` for `"CUEI"`). Used
/// only as a fallback (see [`Diagnostic::run`]) when the stream carries no
/// PSI at all to discover a PID from (issue #1046 / audit MD-C1).
const SCTE35_PID: u16 = 0x01F0;

impl Diagnostic for Scte35Check {
    fn run(&self, ts: &[u8], report: &mut Report) {
        let n_packets = ts.len() / TS_PACKET_SIZE;
        let mut pid_states: BTreeMap<u16, SpliceTracker> = BTreeMap::new();

        // Discover the real SCTE-35 PID(s) from the PMT (`stream_type
        // 0x86`, ANSI/SCTE 35 §8.1), the same source `watch.rs` uses —
        // instead of the old hard-coded `SCTE35_PID`, which missed every
        // real capture that (correctly) carries its cue elsewhere, and would
        // misparse unrelated PES on `0x01F0` as SCTE-35 garbage on streams
        // that happen to use that PID for something else.
        let declared = collect_pmt_streams(ts);
        let mut scte35_pids = pids_with_stream_type(&declared, StreamType::Scte35);
        if declared.is_empty() {
            // No PSI at all to discover a PID from (e.g. a minimal
            // synthetic fixture with no PAT/PMT) — fall back to the
            // conventional PID rather than watching nothing.
            scte35_pids.push(SCTE35_PID);
        }

        for i in 0..n_packets {
            let offset = i * TS_PACKET_SIZE;
            let raw = &ts[offset..offset + TS_PACKET_SIZE];

            let Ok(pkt) = TsPacket::parse(raw) else {
                continue;
            };

            let pid = pkt.header.pid;

            // Only watch PID(s) the PMT declares as SCTE-35 (or the
            // fallback above).
            if !scte35_pids.contains(&pid) {
                continue;
            }

            let payload = match pkt.payload {
                Some(pl) => pl,
                None => continue,
            };

            let state = pid_states.entry(pid).or_default();
            state.feed(payload, pkt.header.pusi, |event| {
                if event.duplicate_open {
                    // Duplicate open "out" with no intervening "in".
                    report.push(Finding::new(
                        Severity::Warning,
                        Location::new(i, u32::from(pid)),
                        "scte35-dup-out",
                        alloc::format!(
                            "duplicate open splice_insert: out event_id {} \
                             with no intervening in",
                            event.event_id,
                        ),
                    ));
                }
            });
        }

        // End of stream: report any remaining open events.
        for (&pid, state) in pid_states.iter() {
            for eid in state.open_events() {
                report.push(Finding::new(
                    Severity::Warning,
                    Location::new(n_packets.saturating_sub(1), u32::from(pid)),
                    "scte35-unbalanced",
                    alloc::format!(
                        "unbalanced splice_insert: out event_id {eid} with no matching in",
                    ),
                ));
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
        make_splice_insert_section_with_break(event_id, out_of_network, cancel, None)
    }

    /// As [`make_splice_insert_section`], optionally carrying a
    /// `break_duration()` — `Some((auto_return, ticks))` — the form a real
    /// SSAI break uses (ANSI/SCTE 35 §9.7.3, §9.8.2).
    fn make_splice_insert_section_with_break(
        event_id: u32,
        out_of_network: bool,
        cancel: bool,
        break_duration: Option<(bool, u64)>,
    ) -> Vec<u8> {
        use broadcast_common::Serialize;
        use scte35_splice::SpliceInfoSection;
        use scte35_splice::commands::SpliceInsert;
        use scte35_splice::time::BreakDuration;

        let si = SpliceInsert {
            splice_event_id: event_id,
            splice_event_cancel_indicator: cancel,
            out_of_network_indicator: out_of_network,
            program_splice_flag: true,
            splice_immediate_flag: true,
            break_duration: break_duration.map(|(auto_return, duration)| BreakDuration {
                auto_return,
                duration,
            }),
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

    /// An "out" with `break_duration.auto_return = true` has no return cue by
    /// design: the splicer returns after `duration` (ANSI/SCTE 35 §9.9.2.2).
    /// It must NOT be reported as an unbalanced break (audit MD-W3).
    #[test]
    fn auto_return_out_is_not_unbalanced() {
        let pid = 0x01F0u16;
        // 30 s break (30 * 90_000 ticks), auto_return set.
        let out = make_splice_insert_section_with_break(7, true, false, Some((true, 30 * 90_000)));

        let mut payload = Vec::new();
        payload.push(0x00);
        payload.extend_from_slice(&out);

        let ts = make_packet(&payload, pid, 0);
        let mut report = Report::new();
        Scte35Check.run(&ts, &mut report);
        assert!(
            report.is_empty(),
            "an auto-return out closes at its own duration and must produce no findings, got {:?}",
            report.findings()
        );
    }

    /// The same break followed by its own "in" (an explicit return, some
    /// head-ends still emit one) must also be clean — the auto-return out is
    /// already closed, so the "in" is a no-op rather than a duplicate.
    #[test]
    fn auto_return_out_followed_by_in_is_clean() {
        let pid = 0x01F0u16;
        let out = make_splice_insert_section_with_break(7, true, false, Some((true, 90_000)));
        let back = make_splice_insert_section(7, false, false);

        let mut payload = Vec::new();
        payload.push(0x00);
        payload.extend_from_slice(&out);
        payload.extend_from_slice(&back);

        let ts = make_packet(&payload, pid, 0);
        let mut report = Report::new();
        Scte35Check.run(&ts, &mut report);
        assert!(
            report.is_empty(),
            "auto-return out followed by its own return cue must be clean, got {:?}",
            report.findings()
        );
    }

    /// A break with `auto_return = false` still expects a separate return cue
    /// — that stays unbalanced, and the auto-return exemption must not
    /// swallow it.
    #[test]
    fn non_auto_return_out_still_unbalanced() {
        let pid = 0x01F0u16;
        let out = make_splice_insert_section_with_break(7, true, false, Some((false, 90_000)));

        let mut payload = Vec::new();
        payload.push(0x00);
        payload.extend_from_slice(&out);

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
            "a break_duration with auto_return = false needs an explicit return cue, got {:?}",
            report.findings()
        );
    }
}
