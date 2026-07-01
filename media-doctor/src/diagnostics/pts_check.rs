//! `PtsCheck` — flags non-monotonic PTS/DTS and forbidden `PTS_DTS_flags == 01`.
//!
//! ITU-T H.222.0 / ISO/IEC 13818-1 §2.4.3.7 specifies the PTS/DTS timing model:
//! PTS values must be monotonically increasing on each PID (33-bit wrap permitted).
//! The `PTS_DTS_flags` field is a 2-bit code (`00`=none, `10`=PTS, `11`=PTS+DTS);
//! the value `01` is reserved / forbidden by the spec and signals a malformed stream.
//!
//! # Checks
//!
//! - **pts-backward** (Error): a PTS that goes backward on a PID after 33-bit
//!   wrap-unrolling (non-monotonic). Also checks DTS if present.
//! - **pts-forbidden-flags** (Error): `PTS_DTS_flags == 0b01` in a PES header,
//!   which is forbidden by ITU-T H.222.0 §2.4.3.7 Table 2-21.
//!
//! A signalled TS-layer discontinuity (`discontinuity_indicator == 1`) resets
//! the per-PID PTS/DTS baseline so that a jump across the discontinuity is not
//! flagged as backward.

use alloc::collections::btree_map::BTreeMap;

use crate::report::{Finding, Location, Severity};
use crate::Diagnostic;
use crate::Report;
use mpeg_pes::PesPacket;
use mpeg_ts::ts::{TsPacket, TS_PACKET_SIZE};

/// 33-bit PTS/DTS modulus (2^33).
const PTS_MODULUS: u64 = 1u64 << 33;

/// Half of the 33-bit range — used as the threshold to distinguish a genuine
/// backward jump from a legal 33-bit wrap.
const PTS_HALF: u64 = 1u64 << 32;

/// Which timestamp kinds we track per PID.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct TsValue {
    raw: u64,
    initialised: bool,
}

/// Per-PID PES reassembly and PTS/DTS tracking state.
#[derive(Debug)]
struct PtsPidState {
    /// PES assembler for this PID.
    assembler: mpeg_pes::PesAssembler,
    /// Previous raw PTS value (33-bit), if any.
    prev_pts: TsValue,
    /// Previous raw DTS value (33-bit), if any.
    prev_dts: TsValue,
}

impl Default for PtsPidState {
    fn default() -> Self {
        Self {
            assembler: mpeg_pes::PesAssembler::new(),
            prev_pts: TsValue {
                raw: 0,
                initialised: false,
            },
            prev_dts: TsValue {
                raw: 0,
                initialised: false,
            },
        }
    }
}

/// Checks PTS/DTS monotonicity and forbidden `PTS_DTS_flags` per PID.
///
/// Flags findings when:
/// - PTS (or DTS) goes backward (non-monotonic) on the same PID.
/// - A PES header has `PTS_DTS_flags == 0b01`, which is forbidden.
///
/// A signalled TS-layer discontinuity (`discontinuity_indicator == 1`) resets
/// the baseline and does NOT produce backward-jump errors across the break.
#[derive(Debug, Clone, Copy)]
pub struct PtsCheck;

impl Diagnostic for PtsCheck {
    fn run(&self, ts: &[u8], report: &mut Report) {
        let n_packets = ts.len() / TS_PACKET_SIZE;
        let mut pid_states: BTreeMap<u16, PtsPidState> = BTreeMap::new();

        for i in 0..n_packets {
            let offset = i * TS_PACKET_SIZE;
            let raw = &ts[offset..offset + TS_PACKET_SIZE];

            let Ok(pkt) = TsPacket::parse(raw) else {
                continue;
            };

            let pid = pkt.header.pid;

            // Check for TS-layer discontinuity: if present, reset the entire
            // PES state (assembler + baseline) so the jump across the
            // discontinuity is not flagged.
            if pkt.header.has_adaptation {
                if let Some(Ok(af)) = pkt.adaptation_field() {
                    if af.discontinuity_indicator {
                        pid_states.remove(&pid);
                        // We removed the assembler, so skip to the next packet
                        // (the discontinuity packet's payload is the start of a
                        // new sequence — it will be freshly assembled below).
                        continue;
                    }
                }
            }

            // Get the TS payload for PES reassembly.
            let payload = match pkt.payload {
                Some(p) => p,
                None => continue,
            };

            if payload.is_empty() {
                continue;
            }

            let pus = pkt.header.pusi;

            // Feed the assembler and check if a complete PES was produced.
            let state = pid_states.entry(pid).or_default();
            if let Some(pes_bytes) = state.assembler.feed(pus, payload) {
                check_pes(
                    &pes_bytes,
                    i,
                    pid,
                    report,
                    &mut state.prev_pts,
                    &mut state.prev_dts,
                );
            }
        }

        // Flush any remaining PES packets at end of stream.
        for (&pid, state) in pid_states.iter_mut() {
            if let Some(pes_bytes) = state.assembler.flush() {
                check_pes(
                    &pes_bytes,
                    n_packets.saturating_sub(1),
                    pid,
                    report,
                    &mut state.prev_pts,
                    &mut state.prev_dts,
                );
            }
        }
    }
}

/// Check a single assembled PES packet for PTS/DTS issues.
fn check_pes(
    pes_bytes: &[u8],
    packet_index: usize,
    pid: u16,
    report: &mut Report,
    prev_pts: &mut TsValue,
    prev_dts: &mut TsValue,
) {
    // --- Forbidden PTS_DTS_flags == 01 check (raw bytes) ---
    //
    // We detect this at the raw byte level because `mpeg_pes::PesPacket::parse`
    // maps `01` to no-PTS (same as `00`), so we cannot see it via the typed
    // header. Inspect the PES optional-header flags byte directly.
    //
    // PES layout (for stream_ids with an optional header):
    //   [0..3]  packet_start_code_prefix (00 00 01)
    //   [3]     stream_id
    //   [4..5]  PES_packet_length
    //   [6]     flags1 (scrambling, priority, alignment, etc.)
    //   [7]     flags2 (PTS_DTS_flags at bits [7:6], ESCR_flag, ES_rate_flag, ...)
    //   [8]     PES_header_data_length
    if pes_bytes.len() >= 9 {
        let stream_id = pes_bytes[3];
        // Only check stream_ids that carry the optional PES header
        // (video 0xE0-0xEF, audio 0xC0-0xDF, private_stream_1 0xBD, etc.)
        let has_optional = !matches!(
            stream_id,
            0xBC | 0xBE | 0xBF | 0xF0 | 0xF1 | 0xF2 | 0xF8 | 0xFF
        );
        if has_optional {
            // pes_bytes[6] is the first flag byte (f1), [7] is the second (f2).
            let f2 = pes_bytes[7];
            let pts_dts_flags = (f2 >> 6) & 0x03;
            if pts_dts_flags == 0b01 {
                report.push(Finding::new(
                    Severity::Error,
                    Location::new(packet_index, pid),
                    "pts-forbidden-flags",
                    alloc::format!(
                        "Forbidden PTS_DTS_flags == 0b01 on PID 0x{:04X} \
                         (stream_id 0x{:02X}) — ITU-T H.222.0 §2.4.3.7",
                        pid,
                        stream_id,
                    ),
                ));
            }
        }
    }

    // --- Parse the PES to get typed PTS/DTS values ---
    let Ok(pes) = PesPacket::parse(pes_bytes) else {
        return;
    };

    let Some(ref header) = pes.header else {
        return;
    };

    // Check PTS monotonicity.
    if let Some(pts) = header.pts {
        let raw = pts.ticks();
        if prev_pts.initialised {
            let delta = raw.wrapping_sub(prev_pts.raw) & (PTS_MODULUS - 1);
            if delta != 0 && delta > PTS_HALF {
                report.push(Finding::new(
                    Severity::Error,
                    Location::new(packet_index, pid),
                    "pts-backward",
                    alloc::format!(
                        "Non-monotonic PTS on PID 0x{:04X}: raw {} → {} (backward delta {})",
                        pid,
                        prev_pts.raw,
                        raw,
                        PTS_MODULUS - delta,
                    ),
                ));
            }
        }
        prev_pts.raw = raw;
        prev_pts.initialised = true;
    }

    // Check DTS monotonicity.
    if let Some(dts) = header.dts {
        let raw = dts.ticks();
        if prev_dts.initialised {
            let delta = raw.wrapping_sub(prev_dts.raw) & (PTS_MODULUS - 1);
            if delta != 0 && delta > PTS_HALF {
                report.push(Finding::new(
                    Severity::Error,
                    Location::new(packet_index, pid),
                    "dts-backward",
                    alloc::format!(
                        "Non-monotonic DTS on PID 0x{:04X}: raw {} → {} (backward delta {})",
                        pid,
                        prev_dts.raw,
                        raw,
                        PTS_MODULUS - delta,
                    ),
                ));
            }
        }
        prev_dts.raw = raw;
        prev_dts.initialised = true;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::report::Report;

    /// Build a TS packet (188 bytes) containing a PES fragment with the given
    /// raw PES bytes at the start of its payload (payload_unit_start=true).
    fn make_pes_packet(pid: u16, cc: u8, pes_bytes: &[u8]) -> Vec<u8> {
        let mut pkt = vec![0x47u8; 188];
        pkt[1] = ((pid >> 8) as u8) & 0x1F;
        pkt[2] = (pid & 0xFF) as u8;
        // AFC=01 (payload only), PUSI=1, CC=cc.
        // Byte 1: PUSI=1 at bit 6, no adaptation
        pkt[1] |= 0x40; // payload_unit_start
        pkt[3] = 0x10 | (cc & 0x0F); // no adaptation, payload only

        let len = pes_bytes.len().min(184);
        pkt[4..4 + len].copy_from_slice(&pes_bytes[..len]);
        pkt
    }

    /// Build a complete PES packet with PTS (PTS_DTS_flags = 0b10).
    /// Returns the PES bytes (not wrapped in TS).
    fn build_pes_with_pts(stream_id: u8, pts_raw: u64, payload: &[u8]) -> Vec<u8> {
        let pts_bytes = mpeg_pes::Pts(pts_raw).to_field_bytes();
        let _hdr_data_len = 5 + payload.len(); // 5-byte PTS + ... well, the hdr data
                                               // Minimal PES: no header_stuffing, no other flags.
                                               // flags1 = 0x80 (marker bit, no scrambling, no flags)
                                               // flags2 = 0x80 (PTS_DTS_flags=10)
        let hdr_len = 5u8; // PTS only, no stuffing
        let pes_len = 9 + hdr_len as usize + payload.len();
        let mut pes = Vec::with_capacity(pes_len);
        pes.extend_from_slice(&[0x00, 0x00, 0x01, stream_id]);
        let length = if pes_len > 6 { (pes_len - 6) as u16 } else { 0 };
        pes.extend_from_slice(&length.to_be_bytes());
        pes.push(0x80); // flags1: marker + no special flags
        pes.push(0x80); // flags2: PTS_DTS_flags=10
        pes.push(hdr_len); // PES_header_data_length
        pes.extend_from_slice(&pts_bytes);
        pes.extend_from_slice(payload);
        pes
    }

    /// Build a complete PES packet with PTS_DTS_flags = 0b01 (forbidden).
    /// The typed parse will see no-PTS, but we detect it at the raw byte level.
    fn build_pes_forbidden_flags(stream_id: u8, payload: &[u8]) -> Vec<u8> {
        // flags2 with PTS_DTS_flags = 0b01
        let pes_len = 9 + payload.len();
        let mut pes = vec![0x00, 0x00, 0x01, stream_id];
        let length = if pes_len > 6 { (pes_len - 6) as u16 } else { 0 };
        pes.extend_from_slice(&length.to_be_bytes());
        pes.push(0x80); // flags1
        pes.push(0x40); // flags2: PTS_DTS_flags=01
        pes.push(0x00); // PES_header_data_length = 0 (no PTS data follows)
        pes.extend_from_slice(payload);
        pes
    }

    /// Helper: PTS half range constant for tests.
    const PTS_MOD: u64 = 1u64 << 33;

    #[test]
    fn single_pes_no_findings() {
        let pid = 0x0100;
        let pes = build_pes_with_pts(0xE0, 90_000, &[0xAA, 0xBB]);
        let ts = make_pes_packet(pid, 0, &pes);
        let mut report = Report::new();
        PtsCheck.run(&ts, &mut report);
        assert!(
            report.is_empty(),
            "single PES should produce no findings, got {:?}",
            report.findings()
        );
    }

    #[test]
    fn forward_pts_no_findings() {
        let pid = 0x0100;
        let mut ts = Vec::new();
        ts.extend_from_slice(&make_pes_packet(
            pid,
            0,
            &build_pes_with_pts(0xE0, 90_000, &[0xAA]),
        ));
        ts.extend_from_slice(&make_pes_packet(
            pid,
            1,
            &build_pes_with_pts(0xE0, 93_000, &[0xBB]),
        ));
        let mut report = Report::new();
        PtsCheck.run(&ts, &mut report);
        assert!(
            report.is_empty(),
            "forward PTS should produce no findings, got {:?}",
            report.findings()
        );
    }

    #[test]
    fn backward_pts_flags_error() {
        let pid = 0x0100;
        let mut ts = Vec::new();
        ts.extend_from_slice(&make_pes_packet(
            pid,
            0,
            &build_pes_with_pts(0xE0, 90_000, &[0xAA]),
        ));
        ts.extend_from_slice(&make_pes_packet(
            pid,
            1,
            &build_pes_with_pts(0xE0, 40_000, &[0xBB]),
        ));
        let mut report = Report::new();
        PtsCheck.run(&ts, &mut report);
        let bw: Vec<_> = report
            .findings()
            .iter()
            .filter(|f| f.rule_id == "pts-backward")
            .collect();
        assert_eq!(
            bw.len(),
            1,
            "expected one pts-backward finding, got {:?}",
            report.findings()
        );
        assert_eq!(bw[0].severity, Severity::Error);
    }

    #[test]
    fn legal_pts_wrap_not_flagged() {
        let pid = 0x0100;
        let mut ts = Vec::new();
        // Near the top of the 33-bit range.
        let near_wrap = PTS_MOD - 5000;
        ts.extend_from_slice(&make_pes_packet(
            pid,
            0,
            &build_pes_with_pts(0xE0, near_wrap, &[0xAA]),
        ));
        // Just past wrap.
        ts.extend_from_slice(&make_pes_packet(
            pid,
            1,
            &build_pes_with_pts(0xE0, 1000, &[0xBB]),
        ));
        let mut report = Report::new();
        PtsCheck.run(&ts, &mut report);
        let bw: Vec<_> = report
            .findings()
            .iter()
            .filter(|f| f.rule_id == "pts-backward")
            .collect();
        assert!(
            bw.is_empty(),
            "legal PTS wrap should not be flagged: {:?}",
            report.findings()
        );
    }

    #[test]
    fn forbidden_pts_dts_flags_detected() {
        let pid = 0x0100;
        let pes = build_pes_forbidden_flags(0xE0, &[0xAA, 0xBB]);
        let ts = make_pes_packet(pid, 0, &pes);
        let mut report = Report::new();
        PtsCheck.run(&ts, &mut report);
        let ff: Vec<_> = report
            .findings()
            .iter()
            .filter(|f| f.rule_id == "pts-forbidden-flags")
            .collect();
        assert_eq!(
            ff.len(),
            1,
            "expected one pts-forbidden-flags finding, got {:?}",
            report.findings()
        );
        assert_eq!(ff[0].severity, Severity::Error);
    }

    #[test]
    fn discontinuity_resets_baseline() {
        let pid = 0x0100;
        // Build helper for packets with adaptation field (carrying discontinuity
        // indicator) + PES payload.
        fn make_disc_pkt(pid: u16, cc: u8, pes_bytes: &[u8], discontinuity: bool) -> Vec<u8> {
            let mut pkt = vec![0x47u8; 188];
            pkt[1] = 0x40 | ((pid >> 8) as u8) & 0x1F;
            pkt[2] = (pid & 0xFF) as u8;
            pkt[3] = 0x30 | (cc & 0x0F); // AFC=11 (adaptation+payload)
            pkt[4] = 1; // adaptation_field_length = 1 (flags byte only)
            pkt[5] = if discontinuity { 0x80 } else { 0x00 };
            let len = pes_bytes.len().min(188 - 6);
            pkt[6..6 + len].copy_from_slice(&pes_bytes[..len]);
            pkt
        }

        // Packet 0: PES with PTS=90000, no discontinuity.
        let pes0 = build_pes_with_pts(0xE0, 90_000, &[0xAA]);
        // Packet 1: PES with PTS=40000, WITH discontinuity.
        // The discontinuity completely resets state (assembler + baseline),
        // so the 40000 starts fresh with no previous PTS.
        let pes1 = build_pes_with_pts(0xE0, 40_000, &[0xBB]);
        // Packet 2: PES with PTS=45000 (forward from 40000), no discontinuity.
        let pes2 = build_pes_with_pts(0xE0, 45_000, &[0xCC]);

        let mut ts = Vec::new();
        ts.extend_from_slice(&make_disc_pkt(pid, 0, &pes0, false));
        ts.extend_from_slice(&make_disc_pkt(pid, 1, &pes1, true));
        ts.extend_from_slice(&make_disc_pkt(pid, 2, &pes2, false));
        let mut report = Report::new();
        PtsCheck.run(&ts, &mut report);
        let bw: Vec<_> = report
            .findings()
            .iter()
            .filter(|f| f.rule_id == "pts-backward")
            .collect();
        assert!(
            bw.is_empty(),
            "discontinuity-reset baseline should not produce backward errors: {:?}",
            report.findings()
        );
    }
}
