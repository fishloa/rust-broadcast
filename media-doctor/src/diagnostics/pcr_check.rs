//! `PcrCheck` — flags PCR repetition and discontinuity anomalies per PID.
//!
//! ITU-T H.222.0 / ISO/IEC 13818-1 §2.7.2 requires PCRs on each PCR_PID at
//! intervals ≤100 ms. TR 101 290 §5.2.1 Table 5.0b indicators 2.3a (PCR
//! repetition error) and 2.3b (PCR discontinuity indicator error) specify the
//! measurement methodology reused here.
//!
//! The check walks every TS packet. For each PCR-bearing adaptation field it
//! records the 27 MHz PCR value. Successive PCRs on the same PID are compared:
//!
//! - **PCR repetition** (§2.7.2, TR 101 290 2.3a): if the PCR-to-PCR value
//!   delta exceeds the repetition limit (100 ms), a finding is emitted
//!   (Warning above 40 ms, Error above 100 ms). This mirrors the mandatory
//!   interval since PCR values track the encoder's 27 MHz system-time-base.
//!
//! - **PCR discontinuity / jump** (TR 101 290 2.3b): a large PCR step
//!   (>100 ms delta) on a PID where the packet does **not** carry
//!   `discontinuity_indicator == 1` signals an anomaly (Error). When the
//!   indicator IS set (§2.4.3.5), the jump is a legitimate system-time-base
//!   change and is NOT flagged.
//!
//! PCR values on 33-bit base × 300 + 9-bit extension use modular arithmetic
//! (modulo 2³³×300) to handle 33-bit wrap, matching the dvb-conformance
//! crate's method.
//!
//! Number of 27 MHz ticks in 100 ms: 27_000_000 / 10 = 2_700_000

use alloc::collections::btree_map::BTreeMap;

use crate::report::{Finding, Location, Severity};
use crate::Diagnostic;
use crate::Report;
use mpeg_ts::ts::{TsPacket, TS_PACKET_SIZE};

/// 27 MHz clock rate (ticks per second) — ISO/IEC 13818-1 §2.4.2.
const CLOCK_27MHZ: u64 = 27_000_000;

/// PCR modulus on the 27 MHz clock: 2³³ × 300.
/// ISO/IEC 13818-1 §2.4.3.5 — 33-bit base wraps modulo this value.
const PCR_MODULUS_27MHZ: u64 = (1u64 << 33) * 300;

/// PCR repetition error threshold: 100 ms in 27 MHz ticks.
/// TR 101 290 Table 5.0b indicator 2.3a / note 2.
const PCR_REPETITION_LIMIT_MS: u64 = 100;
const PCR_REPETITION_LIMIT_TICKS: u64 = CLOCK_27MHZ * PCR_REPETITION_LIMIT_MS / 1000;

/// PCR repetition warning threshold: 40 ms (recommended maximum per note 2).
const PCR_WARNING_LIMIT_MS: u64 = 40;
const PCR_WARNING_LIMIT_TICKS: u64 = CLOCK_27MHZ * PCR_WARNING_LIMIT_MS / 1000;

/// PCR discontinuity/jump error threshold: 100 ms in 27 MHz ticks.
/// TR 101 290 Table 5.0b indicator 2.3b.
const PCR_JUMP_LIMIT_TICKS: u64 = PCR_REPETITION_LIMIT_TICKS;

/// Per-PID PCR tracking state.
#[derive(Debug, Clone)]
struct PcrState {
    /// Previous PCR value on this PID (27 MHz ticks).
    last_pcr: u64,
    /// Whether we have an initialised baseline.
    initialised: bool,
}

/// Checks PCR repetition and discontinuity per PID.
///
/// Flags findings when:
/// - PCR-to-PCR spacing exceeds 40 ms (Warning) or 100 ms (Error).
/// - A PCR jump >100 ms occurs without `discontinuity_indicator` (Error).
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

            let discontinuity = af.discontinuity_indicator;

            let state = pcr_states.entry(pid).or_insert(PcrState {
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
            if discontinuity {
                state.last_pcr = pcr;
                continue;
            }

            // PCR delta (modular, handling 33-bit wrap).
            let delta = (pcr.wrapping_add(PCR_MODULUS_27MHZ) - last_pcr) % PCR_MODULUS_27MHZ;
            let delta_ms = delta * 1000 / CLOCK_27MHZ;

            // 2.3a: PCR repetition check — interval between consecutive PCRs.
            if delta > PCR_REPETITION_LIMIT_TICKS {
                report.push(Finding::new(
                    Severity::Error,
                    Location::new(i, pid),
                    "pcr-repetition",
                    alloc::format!(
                        "PCR repetition interval {} ms exceeds limit {} ms on PID 0x{:04X}",
                        delta_ms,
                        PCR_REPETITION_LIMIT_MS,
                        pid,
                    ),
                ));
            } else if delta > PCR_WARNING_LIMIT_TICKS {
                report.push(Finding::new(
                    Severity::Warning,
                    Location::new(i, pid),
                    "pcr-repetition",
                    alloc::format!(
                        "PCR repetition interval {} ms exceeds recommended {} ms on PID 0x{:04X}",
                        delta_ms,
                        PCR_WARNING_LIMIT_MS,
                        pid,
                    ),
                ));
            }

            // 2.3b: PCR discontinuity/jump — large delta without signalled
            // discontinuity (already handled above — we only get here if
            // discontinuity is false, so this check is redundant but kept for
            // symmetry with dvb-conformance).
            if delta > PCR_JUMP_LIMIT_TICKS {
                report.push(Finding::new(
                    Severity::Error,
                    Location::new(i, pid),
                    "pcr-discontinuity",
                    alloc::format!(
                        "PCR delta {} ms exceeds limit {} ms on PID 0x{:04X} \
                         without discontinuity_indicator",
                        delta_ms,
                        PCR_REPETITION_LIMIT_MS,
                        pid,
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

    /// PCR repetition >100 ms should flag an Error.
    #[test]
    fn pcr_repetition_over_100ms_flags_error() {
        let pid = 0x0100u16;
        let mut ts = Vec::new();
        ts.extend_from_slice(&make_pcr_packet(pid, 0, 0, false));
        // 150 ms later in PCR ticks.
        ts.extend_from_slice(&make_pcr_packet(pid, CLOCK_27MHZ * 150 / 1000, 1, false));
        let mut report = Report::new();
        PcrCheck.run(&ts, &mut report);
        let rep: Vec<_> = report
            .findings()
            .iter()
            .filter(|f| f.rule_id == "pcr-repetition")
            .collect();
        assert_eq!(rep.len(), 1);
        assert_eq!(rep[0].severity, Severity::Error);
    }

    /// PCR repetition >40 ms but ≤100 ms should flag a Warning.
    #[test]
    fn pcr_repetition_over_40ms_flags_warning() {
        let pid = 0x0100u16;
        let mut ts = Vec::new();
        ts.extend_from_slice(&make_pcr_packet(pid, 0, 0, false));
        // 60 ms later in PCR ticks (above 40 ms warning, below 100 ms error).
        ts.extend_from_slice(&make_pcr_packet(pid, CLOCK_27MHZ * 60 / 1000, 1, false));
        let mut report = Report::new();
        PcrCheck.run(&ts, &mut report);
        let rep: Vec<_> = report
            .findings()
            .iter()
            .filter(|f| f.rule_id == "pcr-repetition")
            .collect();
        assert_eq!(rep.len(), 1);
        assert_eq!(rep[0].severity, Severity::Warning);
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
        assert!(disc[0].message.contains("without discontinuity_indicator"));
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
