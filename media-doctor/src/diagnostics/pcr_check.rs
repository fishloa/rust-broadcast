//! `PcrCheck` — TR 101 290 v1.4.1 Table 5.0b indicator 2.3b, evaluated on
//! the PCR **value** stream of a file.
//!
//! 2.3b (`PCR_discontinuity_indicator_error`) flags a difference between two
//! consecutive PCR values outside `0..100 ms` when the packet did not set the
//! `discontinuity_indicator` — the transcription at
//! `private/specs-verified/dvb/dvb-conformance/tr_101_290.md`. A backward step
//! is also outside the range, so it is 2.3b too.
//!
//! Indicator 2.3a (`PCR_repetition_error`) is **not emitted in file mode**:
//! every forward step over 100 ms without the indicator is already reported as
//! 2.3b, so a separate 2.3a finding would only duplicate it. The timing
//! question 2.3a asks is answered where a real arrival clock exists — the live
//! `watch` path, fed wall-clock `Duration`s, and `dvb-conformance`'s
//! caller-supplied timestamps.
//!
//! PCR values on 33-bit base x 300 + 9-bit extension use modular arithmetic
//! (modulo 2^33 x 300) to handle 33-bit wrap, matching the dvb-conformance
//! crate's method.

use alloc::collections::btree_map::BTreeMap;

use crate::Diagnostic;
use crate::Report;
use crate::report::{Finding, Location, Severity};
use mpeg_ts::ts::{TS_PACKET_SIZE, TsPacket};

use broadcast_common::clock33::pcr_forward_distance;
/// 27 MHz clock rate (ticks per second) — ISO/IEC 13818-1 §2.4.2.
use broadcast_common::clock33::{PCR_HZ as CLOCK_27MHZ, PCR_WRAP_27MHZ as PCR_MODULUS_27MHZ};

/// Largest forward PCR distance that is unambiguous: beyond half the wrap the
/// nearer direction is backwards.
const PCR_27MHZ_MAX_FORWARD: u64 = PCR_MODULUS_27MHZ / 2;

/// 27 MHz ticks in one millisecond — every PCR interval below is rendered in
/// ms by dividing by this.
const TICKS_27MHZ_PER_MS: u64 = CLOCK_27MHZ / 1000;

// `PCR_MODULUS_27MHZ` (2³³ × 300, ISO/IEC 13818-1 §2.4.3.5) is
// `broadcast_common::clock33::PCR_WRAP_27MHZ`, imported above.

/// 2.3b's range is "0...100 ms" (TR 101 290 v1.4.1 Table 5.0b), inclusive of
/// 100 ms, so a step is outside it only when strictly greater.
const PCR_DISCONTINUITY_LIMIT_MS: u64 = 100;

/// Per-PID PCR tracking state.
#[derive(Debug, Clone)]
struct PcrState {
    /// Previous PCR value on this PID (27 MHz ticks).
    last_pcr: u64,
    /// Whether we have an initialised baseline.
    initialised: bool,
}

/// Checks TR 101 290 indicator 2.3b per PCR PID.
///
/// Flags a finding when the difference between two consecutive PCR values is
/// outside `0..100 ms` and the packet did not set `discontinuity_indicator`
/// (2.3b, Error — includes a backward step). 2.3a is not emitted in file mode
/// (see the module docs).
///
/// Legitimate system-time-base changes signalled via
/// `discontinuity_indicator == 1` (§2.4.3.5) are NOT flagged.
#[derive(Debug, Clone, Copy)]
pub struct PcrCheck;

impl Diagnostic for PcrCheck {
    fn run(&self, ts: &[u8], report: &mut Report) {
        let n_packets = ts.len() / TS_PACKET_SIZE;
        let mut pcr_states: BTreeMap<u16, PcrState> = BTreeMap::new();

        for i in 0..n_packets {
            let offset = i * TS_PACKET_SIZE;
            let raw = &ts[offset..offset + TS_PACKET_SIZE];

            let Ok(pkt) = TsPacket::parse(raw) else {
                continue;
            };

            let pid = pkt.header.pid;

            // Only packets with an adaptation field can carry PCR.
            if !pkt.header.has_adaptation() {
                continue;
            }

            let af = match pkt.adaptation_field() {
                Some(Ok(a)) => a,
                _ => continue,
            };

            let pcr = match af.pcr {
                Some(p) => p.as_27mhz(),
                None => continue,
            };

            let state = pcr_states.entry(pid.value()).or_insert(PcrState {
                last_pcr: 0,
                initialised: false,
            });

            if !state.initialised {
                state.last_pcr = pcr;
                state.initialised = true;
                continue;
            }

            let last_pcr = state.last_pcr;

            // A signalled discontinuity (§2.4.3.5) re-anchors the clock.
            // The PCR value on this packet samples a new time base — do not
            // compare against the previous baseline.
            if af.discontinuity_indicator {
                state.last_pcr = pcr;
                continue;
            }

            // PCR delta (modular, handling 33-bit wrap).
            let delta = pcr_forward_distance(last_pcr, pcr);
            let delta_ms = delta / TICKS_27MHZ_PER_MS;

            // 2.3b is defined on the signed difference: a modular delta past
            // half the modulus is a backward step. See the module docs and
            // `private/specs-verified/dvb/dvb-conformance/tr_101_290.md`.
            let signed_step = delta; // 2.3b is defined on the signed difference.
            let backward = signed_step > PCR_27MHZ_MAX_FORWARD;
            let forward_ms = delta_ms;
            // `backward` guarantees `signed_step > modulus/2`, so the
            // subtraction cannot underflow; `saturating_sub` states that
            // rather than resting on it.
            let backward_ms = (PCR_MODULUS_27MHZ.saturating_sub(signed_step)) / TICKS_27MHZ_PER_MS;

            // ── 2.3b: PCR_discontinuity_indicator_error ─────────────────
            // "outside 0...100 ms without the discontinuity_indicator set".
            // Decidable from the values alone, so this is the Error.
            let outside_range = backward || forward_ms > PCR_DISCONTINUITY_LIMIT_MS;
            let flagged_discontinuity = outside_range && !af.discontinuity_indicator;
            if flagged_discontinuity {
                let what = if backward {
                    alloc::format!("PCR goes backwards by {backward_ms} ms")
                } else {
                    alloc::format!("PCR delta {forward_ms} ms")
                };
                report.push(Finding::new(
                    Severity::Error,
                    Location::at_pid(i, pid),
                    "pcr-discontinuity",
                    alloc::format!(
                        "{what} on PID 0x{pid:04X}, outside the 0..{PCR_DISCONTINUITY_LIMIT_MS} ms range and no discontinuity_indicator — TR 101 290 indicator 2.3b",
                    ),
                ));
            }

            // 2.3a is not emitted in file mode: every forward step over
            // 100 ms without the discontinuity_indicator is already reported
            // above as 2.3b.

            state.last_pcr = pcr;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::report::{Report, Severity};
    use alloc::{vec, vec::Vec};
    use mpeg_ts::Pcr;
    use mpeg_ts::pid::Pid;

    /// Encode a PCR value into its 6-byte adaptation-field representation.
    fn encode_pcr_27mhz(ticks: u64) -> [u8; 6] {
        Pcr::from_27mhz(ticks)
            .expect("within the PCR range")
            .to_field_bytes()
    }

    /// Build a TS packet (188 bytes) with a PCR-bearing adaptation field on
    /// the given PID, with optional discontinuity_indicator.
    fn make_pcr_packet(pid: Pid, pcr_27mhz: u64, cc: u8, discontinuity: bool) -> Vec<u8> {
        let mut pkt = vec![0x47u8; 188];
        pkt[1] = ((pid.value() >> 8) as u8) & 0x1F;
        pkt[2] = (pid.value() & 0xFF) as u8;
        // AFC=11 (adaptation + payload), CC=cc.
        pkt[3] = 0x30 | (cc & 0x0F);

        let pcr_bytes = encode_pcr_27mhz(pcr_27mhz);
        // adaptation_field_length: 1 (flags) + 6 (PCR) = 7, plus discontinuity
        // adds nothing extra (it's in the flags byte).
        let af_len = 1 + 6; // flags + PCR
        pkt[4] = af_len as u8;
        let flags = if discontinuity {
            0x80 | 0x10 // discontinuity + PCR flag
        } else {
            0x10 // PCR flag only
        };
        pkt[5] = flags;
        pkt[6..6 + 6].copy_from_slice(&pcr_bytes);
        pkt
    }

    /// A single PCR packet on a clean PID should produce zero findings.
    #[test]
    fn single_pcr_no_findings() {
        let pid = 0x0100u16;
        let ts = make_pcr_packet(Pid::from_wire_masked(pid), 0, 0, false);
        let mut report = Report::new();
        PcrCheck.run(&ts, &mut report);
        assert!(report.is_empty(), "unexpected findings: {report:?}");
    }

    /// Two PCRs within 40 ms should produce no findings.
    #[test]
    fn two_pcrs_within_limit_no_findings() {
        let pid = 0x0100u16;
        let mut ts = Vec::new();
        ts.extend_from_slice(&make_pcr_packet(Pid::from_wire_masked(pid), 0, 0, false));
        // 30 ms later in PCR ticks (within 40 ms warning threshold).
        ts.extend_from_slice(&make_pcr_packet(
            Pid::from_wire_masked(pid),
            CLOCK_27MHZ * 30 / 1000,
            1,
            false,
        ));
        let mut report = Report::new();
        PcrCheck.run(&ts, &mut report);
        assert!(
            report.is_empty(),
            "expected no findings, got {:?}",
            report.findings()
        );
    }

    /// A forward value step over 100 ms is exactly one 2.3b pcr-discontinuity
    /// and no other pcr rule (2.3a is not emitted in file mode).
    #[test]
    fn pcr_step_over_100ms_is_2_3b_not_2_3a() {
        let pid = 0x0100u16;
        let mut ts = Vec::new();
        ts.extend_from_slice(&make_pcr_packet(Pid::from_wire_masked(pid), 0, 0, false));
        // 150 ms later in PCR ticks, no discontinuity_indicator.
        ts.extend_from_slice(&make_pcr_packet(
            Pid::from_wire_masked(pid),
            CLOCK_27MHZ * 150 / 1000,
            1,
            false,
        ));
        let mut report = Report::new();
        PcrCheck.run(&ts, &mut report);

        let disc: Vec<_> = report
            .findings()
            .iter()
            .filter(|f| f.rule_id == "pcr-discontinuity")
            .collect();
        assert_eq!(
            disc.len(),
            1,
            "a 150 ms step outside 0..100 ms with no indicator is one 2.3b              error; got {:?}",
            report.findings(),
        );
        assert_eq!(disc[0].severity, Severity::Error);
        assert!(
            disc[0].message.contains("2.3b"),
            "the finding must name indicator 2.3b; got {:?}",
            disc[0].message,
        );
        assert_eq!(
            report.findings().len(),
            1,
            "a 150 ms forward step is exactly one pcr-discontinuity and no other pcr rule; got {:?}",
            report.findings(),
        );
        assert_eq!(report.findings()[0].rule_id, "pcr-discontinuity");
    }

    /// A value step at or below the 100 ms limit is not reported at all.
    #[test]
    fn pcr_value_step_within_100ms_is_silent() {
        let pid = 0x0100u16;
        let mut ts = Vec::new();
        ts.extend_from_slice(&make_pcr_packet(Pid::from_wire_masked(pid), 0, 0, false));
        ts.extend_from_slice(&make_pcr_packet(
            Pid::from_wire_masked(pid),
            CLOCK_27MHZ * 80 / 1000,
            1,
            false,
        ));
        let mut report = Report::new();
        PcrCheck.run(&ts, &mut report);
        assert!(
            report.is_empty(),
            "an 80 ms step is legal spacing and must produce no findings; got {:?}",
            report.findings(),
        );
    }

    /// A *long run of packets with no PCR at all* is not decidable from a
    /// file: the value step across it is what a file can honestly measure,
    /// and here that step is normal. This is the false-positive mode the old
    /// byte-position-derived clock produced on real captures (issue #1112
    /// review).
    #[test]
    fn packet_gap_alone_is_not_flagged() {
        let pid = 0x0100u16;
        let mut ts = Vec::new();
        ts.extend_from_slice(&make_pcr_packet(Pid::from_wire_masked(pid), 0, 0, false));
        // 3000 non-PCR packets on the PID, then the next PCR only 40 ms of
        // PCR time later — a null-stripped / remuxed capture's shape.
        for _ in 0..3000 {
            let mut filler = vec![0x47u8; 188];
            filler[1] = ((pid >> 8) as u8) & 0x1F;
            filler[2] = (pid & 0xFF) as u8;
            filler[3] = 0x10; // payload only => no adaptation field => no PCR
            ts.extend_from_slice(&filler);
        }
        ts.extend_from_slice(&make_pcr_packet(
            Pid::from_wire_masked(pid),
            CLOCK_27MHZ * 40 / 1000,
            2,
            false,
        ));

        let mut report = Report::new();
        PcrCheck.run(&ts, &mut report);
        assert!(
            report.is_empty(),
            "a long packet gap with a normal PCR value step must produce no findings; got {:?}",
            report.findings(),
        );
    }

    /// A value jump on a PID that did NOT signal a discontinuity is 2.3b: one
    /// Error, exactly one kind of rule.
    #[test]
    fn value_jump_is_discontinuity_not_repetition() {
        let pid = 0x0100u16;
        let mut ts = Vec::new();
        // A normal 40 ms step, then a +200 ms jump with no discontinuity flag.
        ts.extend_from_slice(&make_pcr_packet(Pid::from_wire_masked(pid), 0, 0, false));
        ts.extend_from_slice(&make_pcr_packet(
            Pid::from_wire_masked(pid),
            CLOCK_27MHZ * 40 / 1000,
            1,
            false,
        ));
        ts.extend_from_slice(&make_pcr_packet(
            Pid::from_wire_masked(pid),
            CLOCK_27MHZ * 240 / 1000,
            2,
            false,
        ));

        let mut report = Report::new();
        PcrCheck.run(&ts, &mut report);
        let disc: Vec<_> = report
            .findings()
            .iter()
            .filter(|f| f.rule_id == "pcr-discontinuity")
            .collect();
        assert_eq!(
            disc.len(),
            1,
            "the +200 ms value step must be reported once as pcr-discontinuity (Error); got {:?}",
            report.findings(),
        );
        assert_eq!(disc[0].severity, Severity::Error);
        assert_eq!(
            report.findings().len(),
            1,
            "one step, one finding and no other pcr rule; got {:?}",
            report.findings(),
        );
    }

    /// A large PCR jump without discontinuity_indicator should flag a
    /// pcr-discontinuity error.
    #[test]
    fn pcr_jump_without_discontinuity_flags_error() {
        let pid = 0x0100u16;
        let mut ts = Vec::new();
        ts.extend_from_slice(&make_pcr_packet(Pid::from_wire_masked(pid), 0, 0, false));
        // +10 second jump, no discontinuity_indicator.
        ts.extend_from_slice(&make_pcr_packet(
            Pid::from_wire_masked(pid),
            CLOCK_27MHZ * 10,
            1,
            false,
        ));
        let mut report = Report::new();
        PcrCheck.run(&ts, &mut report);
        let disc: Vec<_> = report
            .findings()
            .iter()
            .filter(|f| f.rule_id == "pcr-discontinuity")
            .collect();
        assert_eq!(disc.len(), 1);
        assert_eq!(disc[0].severity, Severity::Error);
        assert!(
            disc[0].message.contains("2.3b"),
            "the finding must name indicator 2.3b; got {:?}",
            disc[0].message,
        );
    }

    /// A large PCR jump WITH discontinuity_indicator must NOT be flagged.
    #[test]
    fn pcr_jump_with_discontinuity_not_flagged() {
        let pid = 0x0100u16;
        let mut ts = Vec::new();
        ts.extend_from_slice(&make_pcr_packet(Pid::from_wire_masked(pid), 0, 0, false));
        // +10 second jump WITH discontinuity_indicator set.
        ts.extend_from_slice(&make_pcr_packet(
            Pid::from_wire_masked(pid),
            CLOCK_27MHZ * 10,
            1,
            true,
        ));
        let mut report = Report::new();
        PcrCheck.run(&ts, &mut report);
        let disc: Vec<_> = report
            .findings()
            .iter()
            .filter(|f| f.rule_id == "pcr-discontinuity")
            .collect();
        assert!(
            disc.is_empty(),
            "signalled discontinuity should not be flagged: {disc:?}"
        );
    }

    /// PCR wrap-around (modular arithmetic) should not be flagged.
    #[test]
    fn pcr_wrap_around_not_flagged() {
        let pid = 0x0100u16;
        let mut ts = Vec::new();
        // Start near the wrap point — 30 ms before wrap.
        let start = PCR_MODULUS_27MHZ - CLOCK_27MHZ * 30 / 1000;
        ts.extend_from_slice(&make_pcr_packet(
            Pid::from_wire_masked(pid),
            start,
            0,
            false,
        ));
        // After wrap — 5 ms worth of ticks after wrap.
        let after = CLOCK_27MHZ * 5 / 1000;
        ts.extend_from_slice(&make_pcr_packet(
            Pid::from_wire_masked(pid),
            after,
            1,
            false,
        ));
        let mut report = Report::new();
        PcrCheck.run(&ts, &mut report);
        assert!(
            report.is_empty(),
            "PCR wrap should not be flagged: {:?}",
            report.findings()
        );
    }

    /// The first PCR on a PID after a discontinuity_indicator resets the
    /// baseline; subsequent normal PCRs should not be trigged by the old
    /// pre-jump value.
    #[test]
    fn discontinuity_resets_baseline() {
        let pid = 0x0100u16;
        let mut ts = Vec::new();
        // First PCR at t=0.
        ts.extend_from_slice(&make_pcr_packet(Pid::from_wire_masked(pid), 0, 0, false));
        // Discontinuity PCR at t=+10s (with indicator).
        ts.extend_from_slice(&make_pcr_packet(
            Pid::from_wire_masked(pid),
            CLOCK_27MHZ * 10,
            1,
            true,
        ));
        // Next PCR at 30 ms after the new baseline (should be clean).
        ts.extend_from_slice(&make_pcr_packet(
            Pid::from_wire_masked(pid),
            CLOCK_27MHZ * 10 + CLOCK_27MHZ * 30 / 1000,
            2,
            false,
        ));
        let mut report = Report::new();
        PcrCheck.run(&ts, &mut report);
        assert!(
            report.is_empty(),
            "post-discontinuity PCRs must produce no findings: {:?}",
            report.findings()
        );
    }

    /// A backward step reports the size of the step in milliseconds.
    #[test]
    fn backward_step_message_names_the_size() {
        let pid = 0x0100u16;
        let mut ts = Vec::new();
        // 10 s -> 9.5 s with no discontinuity_indicator: a 500 ms backward
        // step.
        ts.extend_from_slice(&make_pcr_packet(
            Pid::from_wire_masked(pid),
            CLOCK_27MHZ * 10,
            0,
            false,
        ));
        ts.extend_from_slice(&make_pcr_packet(
            Pid::from_wire_masked(pid),
            CLOCK_27MHZ * 10 - CLOCK_27MHZ * 500 / 1000,
            1,
            false,
        ));
        let mut report = Report::new();
        PcrCheck.run(&ts, &mut report);
        let disc: Vec<_> = report
            .findings()
            .iter()
            .filter(|f| f.rule_id == "pcr-discontinuity")
            .collect();
        assert_eq!(disc.len(), 1, "one backward step: {:?}", report.findings());
        assert!(
            disc[0].message.contains("PCR goes backwards by 500 ms"),
            "the message must name the 500 ms backward step: {:?}",
            disc[0].message
        );
    }

    /// A forward step of exactly half the PCR range is still forward; one
    /// tick more is backward. Both are outside `0..100 ms`, so both are 2.3b
    /// — the boundary decides which *direction* the finding describes.
    #[test]
    fn half_range_pcr_step_is_forward_not_backward() {
        let half = PCR_MODULUS_27MHZ / 2;

        fn messages_for(second: u64) -> alloc::vec::Vec<alloc::string::String> {
            let pid = 0x0100u16;
            let mut ts = Vec::new();
            ts.extend_from_slice(&make_pcr_packet(Pid::from_wire_masked(pid), 0, 0, false));
            ts.extend_from_slice(&make_pcr_packet(
                Pid::from_wire_masked(pid),
                second,
                1,
                false,
            ));
            let mut report = Report::new();
            PcrCheck.run(&ts, &mut report);
            report
                .findings()
                .iter()
                .filter(|f| f.rule_id == "pcr-discontinuity")
                .map(|f| f.message.clone())
                .collect()
        }

        let forward = messages_for(half);
        assert_eq!(
            forward.len(),
            1,
            "a half-range step is outside 0..100 ms and is one finding; got {forward:?}",
        );
        assert!(
            forward[0].contains("PCR delta") && !forward[0].contains("backwards"),
            "exactly half the range is a forward step; got {:?}",
            forward[0],
        );

        let backward = messages_for(half + 1);
        assert_eq!(
            backward.len(),
            1,
            "one tick past half the range is one finding; got {backward:?}",
        );
        assert!(
            backward[0].contains("backwards"),
            "one tick past half the range is backward; got {:?}",
            backward[0],
        );
    }
}
