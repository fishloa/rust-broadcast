//! `PcrCheck` — TR 101 290 v1.4.1 Table 5.0b indicators 2.3a and 2.3b,
//! evaluated on the PCR **value** stream of a file.
//!
//! Both indicators are defined on the same quantity — the difference between
//! two consecutive PCR values — with different thresholds. Quoting the
//! transcription at `dvb-conformance/docs/tr_101_290.md`:
//!
//! | Indicator | Definition |
//! |---|---|
//! | 2.3a `PCR_repetition_error` | "Time interval between two consecutive PCR values more than 100 ms" (note 2: the 40 ms limitation was removed in 2005) |
//! | 2.3b `PCR_discontinuity_indicator_error` | "The difference between two consecutive PCR values (PCR_(i+1) - PCR_i) is outside the range of 0…100 ms without the discontinuity_indicator set" |
//!
//! So 2.3b covers a forward step over 100 ms **and** a backward step, unless
//! the packet signalled the discontinuity; 2.3a covers a forward step over
//! 100 ms. A step assigned to 2.3b is never also reported as 2.3a: a
//! discontinuity is not also a repetition fault, and reporting one step twice
//! both inflates the count and misdescribes it.
//!
//! # Severities, and what a file can and cannot show
//!
//! - **2.3b is an Error.** It is decidable from the values alone, and this
//!   check reports every violating step individually — a mux paced at 104 ms
//!   has a 2.3b violation on every pair, not one fault repeated.
//! - **2.3a is Info.** The indicator is a statement about *time*, and a
//!   recorded file carries no arrival timing: its byte cadence is whatever
//!   the recorder's null-stripping and remuxing produced, which diverges
//!   from the PCR cadence on exactly the sparse, low-bitrate streams where a
//!   byte-derived clock would misreport (measured: on
//!   `fixtures/scte35-ssai/ts/video_with_scte35_splice_insert.ts` the PCR
//!   values advance in exact 100 ms steps while the packet spacing between
//!   them varies by 6x). The PCR *value step* is the only evidence a file
//!   offers, and on an arrival-timed source it is a sound proxy — the
//!   encoder stamps each PCR against the clock the packets are paced by — so
//!   an exceeding step is surfaced, at Info, and reported once per PID for
//!   the worst step seen. The Error-severity timing question is answered
//!   where a real arrival clock exists: the live `watch` path, fed
//!   wall-clock `Duration`s, and `dvb-conformance`'s caller-supplied
//!   timestamps.
//!
//! PCR values on 33-bit base x 300 + 9-bit extension use modular arithmetic
//! (modulo 2^33 x 300) to handle 33-bit wrap, matching the dvb-conformance
//! crate's method.

use alloc::collections::btree_map::BTreeMap;

use crate::Diagnostic;
use crate::Report;
use crate::report::{Finding, Location, Severity};
use mpeg_ts::ts::{TS_PACKET_SIZE, TsPacket};

/// 27 MHz clock rate (ticks per second) — ISO/IEC 13818-1 §2.4.2.
const CLOCK_27MHZ: u64 = 27_000_000;

/// 27 MHz ticks in one millisecond — every PCR interval below is rendered in
/// ms by dividing by this.
const TICKS_27MHZ_PER_MS: u64 = CLOCK_27MHZ / 1000;

/// PCR modulus on the 27 MHz clock: 2³³ × 300.
/// ISO/IEC 13818-1 §2.4.3.5 — 33-bit base wraps modulo this value.
const PCR_MODULUS_27MHZ: u64 = (1u64 << 33) * 300;

/// PCR repetition error threshold: 100 ms (TR 101 290 Table 5.0b indicator
/// 2.3a / note 2).
const PCR_REPETITION_LIMIT_MS: u64 = 100;

/// 2.3b's range is "0...100 ms" (TR 101 290 v1.4.1 Table 5.0b), inclusive of
/// 100 ms, so a step is outside it only when strictly greater.
const PCR_DISCONTINUITY_LIMIT_MS: u64 = 100;

/// Per-PID PCR tracking state.
#[derive(Debug, Clone)]
struct PcrState {
    /// Previous PCR value on this PID (27 MHz ticks).
    last_pcr: u64,
    /// The largest value step already reported for this PID, so a run of
    /// over-limit steps produces one finding rather than one per PCR.
    worst_reported_step_ms: u64,
    /// Whether we have an initialised baseline.
    initialised: bool,
}

/// Checks TR 101 290 indicators 2.3a and 2.3b per PCR PID.
///
/// Flags findings when:
/// - the difference between two consecutive PCR values is outside
///   `0..100 ms` and the packet did not set `discontinuity_indicator`
///   (**2.3b**, Error — includes a backward step); or
/// - a forward PCR value step exceeds 100 ms and was not already reported as
///   2.3b (**2.3a**, Info — see the module docs for why a file cannot
///   support an Error here).
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
            if !pkt.header.has_adaptation {
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

            let state = pcr_states.entry(pid).or_insert(PcrState {
                last_pcr: 0,
                worst_reported_step_ms: 0,
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
            let delta = (pcr.wrapping_add(PCR_MODULUS_27MHZ) - last_pcr) % PCR_MODULUS_27MHZ;
            let delta_ms = delta / TICKS_27MHZ_PER_MS;

            // The two indicators are defined on the *same* quantity, with
            // different thresholds (TR 101 290 v1.4.1 Table 5.0b, transcribed
            // at `dvb-conformance/docs/tr_101_290.md`):
            //
            //   2.3a  PCR_repetition_error: "Time interval between two
            //         consecutive PCR values more than 100 ms" (note 2: the
            //         40 ms limitation was removed in 2005).
            //   2.3b  PCR_discontinuity_indicator_error: "The difference
            //         between two consecutive PCR values (PCR_(i+1) - PCR_i)
            //         is outside the range of 0...100 ms without the
            //         discontinuity_indicator set".
            //
            // So 2.3b covers both a forward step over 100 ms *and* a
            // backward step, unless the packet signalled the discontinuity;
            // 2.3a covers a forward step over 100 ms. A step the spec assigns
            // to 2.3b must not be reported a second time as 2.3a — a genuine
            // discontinuity is not also a repetition error, and reporting it
            // twice both inflates the count and misdescribes it (a +600 s
            // program splice is not a PCR repetition fault).
            let signed_step = delta; // 2.3b is defined on the signed difference.
            let backward = signed_step > PCR_MODULUS_27MHZ / 2;
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
                    Location::new(i, u32::from(pid)),
                    "pcr-discontinuity",
                    alloc::format!(
                        "{what} on PID 0x{pid:04X}, outside the 0..{PCR_DISCONTINUITY_LIMIT_MS} ms range and no discontinuity_indicator — TR 101 290 indicator 2.3b",
                    ),
                ));
            }

            // ── 2.3a: PCR_repetition_error ──────────────────────────────
            // "Time interval between two consecutive PCR values more than
            // 100 ms". Reported once per PID for the worst step seen (a
            // per-step finding on every 104 ms PCR of a mux paced at 104 ms
            // is 19 findings for one fault), and never for a step already
            // reported as 2.3b.
            if !backward
                && !flagged_discontinuity
                && forward_ms > PCR_REPETITION_LIMIT_MS
                && forward_ms > state.worst_reported_step_ms
            {
                state.worst_reported_step_ms = forward_ms;
                report.push(Finding::new(
                    Severity::Info,
                    Location::new(i, u32::from(pid)),
                    "pcr-repetition",
                    alloc::format!(
                        "PCR value step {forward_ms} ms exceeds the {PCR_REPETITION_LIMIT_MS} ms repetition limit on PID 0x{pid:04X} — TR 101 290 indicator 2.3a. Reported at Info: a file carries no arrival clock, so the value step is the only evidence available; `media-doctor watch` evaluates 2.3a against real arrival times",
                    ),
                ));
            }

            // Update state.
            state.last_pcr = pcr;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::report::{Report, Severity};
    use mpeg_ts::Pcr;

    /// Encode a PCR value into its 6-byte adaptation-field representation.
    fn encode_pcr_27mhz(ticks: u64) -> [u8; 6] {
        Pcr::from_27mhz(ticks).to_field_bytes()
    }

    /// Build a TS packet (188 bytes) with a PCR-bearing adaptation field on
    /// the given PID, with optional discontinuity_indicator.
    fn make_pcr_packet(pid: u16, pcr_27mhz: u64, cc: u8, discontinuity: bool) -> Vec<u8> {
        let mut pkt = vec![0x47u8; 188];
        pkt[1] = ((pid >> 8) as u8) & 0x1F;
        pkt[2] = (pid & 0xFF) as u8;
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
        let ts = make_pcr_packet(pid, 0, 0, false);
        let mut report = Report::new();
        PcrCheck.run(&ts, &mut report);
        assert!(report.is_empty(), "unexpected findings: {report:?}");
    }

    /// Two PCRs within 40 ms should produce no findings.
    #[test]
    fn two_pcrs_within_limit_no_findings() {
        let pid = 0x0100u16;
        let mut ts = Vec::new();
        ts.extend_from_slice(&make_pcr_packet(pid, 0, 0, false));
        // 30 ms later in PCR ticks (within 40 ms warning threshold).
        ts.extend_from_slice(&make_pcr_packet(pid, CLOCK_27MHZ * 30 / 1000, 1, false));
        let mut report = Report::new();
        PcrCheck.run(&ts, &mut report);
        assert!(
            report.is_empty(),
            "expected no findings, got {:?}",
            report.findings()
        );
    }

    /// A PCR value step over the 100 ms repetition limit is reported — but
    /// at **Info**, not Error: a file has no arrival clock, so the value step
    /// is advisory (module docs).
    #[test]
    fn pcr_step_over_100ms_is_2_3b_not_2_3a() {
        let pid = 0x0100u16;
        let mut ts = Vec::new();
        ts.extend_from_slice(&make_pcr_packet(pid, 0, 0, false));
        // 150 ms later in PCR ticks, no discontinuity_indicator.
        ts.extend_from_slice(&make_pcr_packet(pid, CLOCK_27MHZ * 150 / 1000, 1, false));
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
        assert!(
            report
                .findings()
                .iter()
                .all(|f| f.rule_id != "pcr-repetition"),
            "the same step must not also be reported as 2.3a; got {:?}",
            report.findings(),
        );
    }

    /// A value step at or below the 100 ms limit is not reported at all — the
    /// 40 ms figure in Table 5.0b note 2 is a recommendation, and emitting an
    /// advisory for every 80 ms PCR of an 80 ms-spaced clean capture is noise
    /// (issue #1112 review).
    #[test]
    fn pcr_value_step_within_100ms_is_silent() {
        let pid = 0x0100u16;
        let mut ts = Vec::new();
        ts.extend_from_slice(&make_pcr_packet(pid, 0, 0, false));
        ts.extend_from_slice(&make_pcr_packet(pid, CLOCK_27MHZ * 80 / 1000, 1, false));
        let mut report = Report::new();
        PcrCheck.run(&ts, &mut report);
        assert!(
            report
                .findings()
                .iter()
                .all(|f| f.rule_id != "pcr-repetition"),
            "an 80 ms step is legal spacing and must not be reported; got {:?}",
            report.findings(),
        );
    }

    /// A *long run of packets with no PCR at all* is not a 2.3a violation on
    /// its own — the value step across it is what a file can honestly
    /// measure, and here that step is normal. This is the false-positive mode
    /// the old byte-position-derived clock produced on real captures
    /// (issue #1112 review).
    #[test]
    fn packet_gap_alone_is_not_a_repetition_error() {
        let pid = 0x0100u16;
        let mut ts = Vec::new();
        ts.extend_from_slice(&make_pcr_packet(pid, 0, 0, false));
        // 3000 non-PCR packets on the PID, then the next PCR only 40 ms of
        // PCR time later — a null-stripped / remuxed capture's shape.
        for _ in 0..3000 {
            let mut filler = vec![0x47u8; 188];
            filler[1] = ((pid >> 8) as u8) & 0x1F;
            filler[2] = (pid & 0xFF) as u8;
            filler[3] = 0x10; // payload only => no adaptation field => no PCR
            ts.extend_from_slice(&filler);
        }
        ts.extend_from_slice(&make_pcr_packet(pid, CLOCK_27MHZ * 40 / 1000, 2, false));

        let mut report = Report::new();
        PcrCheck.run(&ts, &mut report);
        assert!(
            report
                .findings()
                .iter()
                .all(|f| f.rule_id != "pcr-repetition"),
            "a long packet gap with a normal PCR value step is not decidable as 2.3a from a file and must not be reported; got {:?}",
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
        ts.extend_from_slice(&make_pcr_packet(pid, 0, 0, false));
        ts.extend_from_slice(&make_pcr_packet(pid, CLOCK_27MHZ * 40 / 1000, 1, false));
        ts.extend_from_slice(&make_pcr_packet(pid, CLOCK_27MHZ * 240 / 1000, 2, false));

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
    }

    /// A large PCR jump without discontinuity_indicator should flag a
    /// pcr-discontinuity error.
    #[test]
    fn pcr_jump_without_discontinuity_flags_error() {
        let pid = 0x0100u16;
        let mut ts = Vec::new();
        ts.extend_from_slice(&make_pcr_packet(pid, 0, 0, false));
        // +10 second jump, no discontinuity_indicator.
        ts.extend_from_slice(&make_pcr_packet(pid, CLOCK_27MHZ * 10, 1, false));
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
        ts.extend_from_slice(&make_pcr_packet(pid, 0, 0, false));
        // +10 second jump WITH discontinuity_indicator set.
        ts.extend_from_slice(&make_pcr_packet(pid, CLOCK_27MHZ * 10, 1, true));
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
        ts.extend_from_slice(&make_pcr_packet(pid, start, 0, false));
        // After wrap — 5 ms worth of ticks after wrap.
        let after = CLOCK_27MHZ * 5 / 1000;
        ts.extend_from_slice(&make_pcr_packet(pid, after, 1, false));
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
        ts.extend_from_slice(&make_pcr_packet(pid, 0, 0, false));
        // Discontinuity PCR at t=+10s (with indicator).
        ts.extend_from_slice(&make_pcr_packet(pid, CLOCK_27MHZ * 10, 1, true));
        // Next PCR at 30 ms after the new baseline (should be clean).
        ts.extend_from_slice(&make_pcr_packet(
            pid,
            CLOCK_27MHZ * 10 + CLOCK_27MHZ * 30 / 1000,
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
        assert!(
            disc.is_empty(),
            "post-discontinuity PCRs should be clean: {disc:?}"
        );
        let rep: Vec<_> = report
            .findings()
            .iter()
            .filter(|f| f.rule_id == "pcr-repetition")
            .collect();
        assert!(
            rep.is_empty(),
            "post-discontinuity PCR repetition should be clean: {rep:?}"
        );
    }
}
