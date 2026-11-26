//! `ParamSetsCheck` — flags a keyframe access unit that appears before any
//! parameter sets have been observed on its PID (issue #567).
//!
//! ITU-T H.264 §7.4.1.2.3 / ITU-T H.265 §7.4.2.4.3 require an IDR/IRAP
//! access unit's referenced SPS (+PPS) to be active before it can be
//! decoded — a decoder that has not yet seen a parameter set cannot decode
//! the picture. This walks the raw elementary-stream NAL sequence (PMT
//! `stream_type` 0x1B/0x24 PIDs only) in wire order and flags the first IDR
//! (AVC) / IRAP (HEVC) VCL NAL unit seen before an SPS+PPS pair has been
//! observed on that PID.
//!
//! This is intentionally a lower-level, wire-order walk than
//! [`crate::CodecSignallingCheck`]'s [`transmux::TsDemux`]-based presence
//! check: `TsDemux` cannot produce this fact at all — its per-track config
//! probe only *emits a track once SPS+PPS are found*, so a `Media`-level
//! inspection can never observe an ordering fault that happened before the
//! probe resolved. NAL classification is `broadcast_bitstream::nal`/
//! `broadcast_bitstream::annexb` throughout — no duplicated NAL/SPS parsing.

use alloc::boxed::Box;
use alloc::collections::btree_map::BTreeMap;
use alloc::format;
use alloc::vec::Vec;
use mpeg_ts::pid::Pid;

use broadcast_bitstream::annexb::iter_annexb_nals;
use broadcast_bitstream::nal::{NalCodec, is_keyframe_nal};
use dvb_si::tables::pmt::StreamType;

use crate::diagnostics::codec_common::{AccessUnitScanner, pids_with_stream_type};
use crate::report::{Finding, Location, Severity};
use crate::{Diagnostic, DiagnosticScan, PmtStreamIndex, Report};

/// H.264 `nal_unit_type` mask (ITU-T H.264 §7.3.1, bits `[4:0]`).
const H264_NAL_TYPE_MASK: u8 = 0x1F;
/// H.264 SPS `nal_unit_type` (ITU-T H.264 Table 7-1).
const H264_NAL_SPS: u8 = 7;
/// H.264 PPS `nal_unit_type` (ITU-T H.264 Table 7-1).
const H264_NAL_PPS: u8 = 8;
/// HEVC SPS `nal_unit_type` (ITU-T H.265 Table 7-1).
const HEVC_NAL_SPS: u8 = 33;
/// HEVC PPS `nal_unit_type` (ITU-T H.265 Table 7-1).
const HEVC_NAL_PPS: u8 = 34;

#[derive(Default)]
struct ParamState {
    seen_sps: bool,
    seen_pps: bool,
    reported: bool,
}

/// Flags an IDR (AVC) / IRAP (HEVC) access unit that appears on the wire
/// before its PID's SPS+PPS have been observed.
#[derive(Debug, Clone, Copy)]
pub struct ParamSetsCheck;

impl Diagnostic for ParamSetsCheck {
    fn needs_pmt_streams(&self) -> bool {
        true
    }

    fn scan(&self, pmt: &PmtStreamIndex) -> Box<dyn DiagnosticScan> {
        let avc_pids = pids_with_stream_type(pmt.streams(), StreamType::H264);
        let hevc_pids = pids_with_stream_type(pmt.streams(), StreamType::Hevc);
        let avc_state = avc_pids
            .iter()
            .map(|&p| (p, ParamState::default()))
            .collect();
        let hevc_state = hevc_pids
            .iter()
            .map(|&p| (p, ParamState::default()))
            .collect();
        Box::new(ParamSetsScan {
            avc_pids,
            hevc_pids,
            avc_state,
            hevc_state,
            assembler: AccessUnitScanner::new(),
        })
    }
}

/// One streaming pass of [`ParamSetsCheck`].
struct ParamSetsScan {
    avc_pids: Vec<Pid>,
    hevc_pids: Vec<Pid>,
    avc_state: BTreeMap<Pid, ParamState>,
    hevc_state: BTreeMap<Pid, ParamState>,
    assembler: AccessUnitScanner,
}

fn dispatch(
    avc_state: &mut BTreeMap<Pid, ParamState>,
    hevc_state: &mut BTreeMap<Pid, ParamState>,
    payload: &[u8],
    packet_index: usize,
    pid: Pid,
    report: &mut Report,
) {
    if let Some(state) = avc_state.get_mut(&pid) {
        walk_avc(payload, packet_index, pid, state, report);
    } else if let Some(state) = hevc_state.get_mut(&pid) {
        walk_hevc(payload, packet_index, pid, state, report);
    }
}

impl DiagnosticScan for ParamSetsScan {
    fn feed(&mut self, packet: &[u8], report: &mut Report) {
        if self.avc_pids.is_empty() && self.hevc_pids.is_empty() {
            return;
        }
        let Self {
            avc_pids,
            hevc_pids,
            avc_state,
            hevc_state,
            assembler,
        } = self;
        let mut wanted = |pid: Pid| avc_pids.contains(&pid) || hevc_pids.contains(&pid);
        assembler.feed(packet, &mut wanted, &mut |payload, index, pid| {
            dispatch(avc_state, hevc_state, payload, index, pid, report)
        });
    }

    fn finish(&mut self, report: &mut Report) {
        let Self {
            avc_state,
            hevc_state,
            assembler,
            ..
        } = self;
        assembler.finish(&mut |payload, index, pid| {
            dispatch(avc_state, hevc_state, payload, index, pid, report)
        });
    }
}

fn walk_avc(
    payload: &[u8],
    packet_index: usize,
    pid: Pid,
    state: &mut ParamState,
    report: &mut Report,
) {
    for nal in iter_annexb_nals(payload) {
        if nal.is_empty() || state.reported {
            continue;
        }
        match nal[0] & H264_NAL_TYPE_MASK {
            H264_NAL_SPS => state.seen_sps = true,
            H264_NAL_PPS => state.seen_pps = true,
            _ => {
                if is_keyframe_nal(NalCodec::Avc, nal) && !(state.seen_sps && state.seen_pps) {
                    report.push(Finding::new(
                        Severity::Error,
                        Location::at_pid(packet_index, pid),
                        "missing-parameter-sets",
                        format!(
                            "IDR access unit on PID 0x{pid:04X} appears before SPS+PPS were \
                             observed on the wire — ITU-T H.264 §7.4.1.2.3 requires an active \
                             SPS+PPS to decode it"
                        ),
                    ));
                    state.reported = true;
                }
            }
        }
    }
}

fn walk_hevc(
    payload: &[u8],
    packet_index: usize,
    pid: Pid,
    state: &mut ParamState,
    report: &mut Report,
) {
    for nal in iter_annexb_nals(payload) {
        if nal.is_empty() || state.reported {
            continue;
        }
        match broadcast_bitstream::nal::nal_unit_type(NalCodec::Hevc, nal) {
            Some(HEVC_NAL_SPS) => state.seen_sps = true,
            Some(HEVC_NAL_PPS) => state.seen_pps = true,
            _ => {
                if is_keyframe_nal(NalCodec::Hevc, nal) && !(state.seen_sps && state.seen_pps) {
                    report.push(Finding::new(
                        Severity::Error,
                        Location::at_pid(packet_index, pid),
                        "missing-parameter-sets",
                        format!(
                            "IRAP access unit on PID 0x{pid:04X} appears before SPS+PPS were \
                             observed on the wire — ITU-T H.265 §7.4.2.4.3 requires an active \
                             SPS+PPS to decode it"
                        ),
                    ));
                    state.reported = true;
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::diagnostics::codec_common::tests::{build_pat_pmt_ts, build_pes, make_pes_packet};

    const VIDEO_PID: Pid = Pid::from_const(0x101);

    /// A real-shaped SPS/PPS/IDR ordering (params before the IDR) must not be
    /// flagged.
    #[test]
    fn params_before_idr_not_flagged() {
        let sps: &[u8] = &[0x67, 0x42, 0x00, 0x0A]; // minimal, decode failure tolerated: only NAL type matters here
        let pps: &[u8] = &[0x68, 0xCE, 0x38, 0x80];
        let idr: &[u8] = &[0x65, 0x88, 0x84, 0x00];

        let mut au = alloc::vec::Vec::new();
        au.extend_from_slice(&[0x00, 0x00, 0x01]);
        au.extend_from_slice(sps);
        au.extend_from_slice(&[0x00, 0x00, 0x01]);
        au.extend_from_slice(pps);
        au.extend_from_slice(&[0x00, 0x00, 0x01]);
        au.extend_from_slice(idr);

        let mut ts = build_pat_pmt_ts(&[(VIDEO_PID.value(), StreamType::H264)]);
        ts.extend_from_slice(&make_pes_packet(VIDEO_PID, 0, &build_pes(0xE0, &au)));

        let mut report = Report::new();
        ParamSetsCheck.run(&ts, &mut report);
        assert!(
            report
                .findings()
                .iter()
                .all(|f| f.rule_id != "missing-parameter-sets"),
            "params-before-IDR must not be flagged, got {:?}",
            report.findings()
        );
    }

    /// An IDR with NO preceding SPS/PPS on the PID must be flagged.
    #[test]
    fn idr_without_params_is_flagged() {
        let idr: &[u8] = &[0x65, 0x88, 0x84, 0x00];
        let mut au = alloc::vec::Vec::new();
        au.extend_from_slice(&[0x00, 0x00, 0x01]);
        au.extend_from_slice(idr);

        let mut ts = build_pat_pmt_ts(&[(VIDEO_PID.value(), StreamType::H264)]);
        ts.extend_from_slice(&make_pes_packet(VIDEO_PID, 0, &build_pes(0xE0, &au)));

        let mut report = Report::new();
        ParamSetsCheck.run(&ts, &mut report);
        assert!(
            report
                .findings()
                .iter()
                .any(|f| f.rule_id == "missing-parameter-sets"
                    && f.location.stream == crate::StreamLocator::Ts(VIDEO_PID)),
            "expected missing-parameter-sets, got {:?}",
            report.findings()
        );
    }

    /// Annex B access unit from NAL units (start-code prefixed).
    fn annexb_au(nals: &[&[u8]]) -> alloc::vec::Vec<u8> {
        let mut au = alloc::vec::Vec::new();
        for nal in nals {
            au.extend_from_slice(&[0x00, 0x00, 0x01]);
            au.extend_from_slice(nal);
        }
        au
    }

    fn count_missing_params(report: &Report) -> usize {
        report
            .findings()
            .iter()
            .filter(|f| f.rule_id == "missing-parameter-sets")
            .count()
    }

    // H.264 NAL headers: SPS 0x67 (type 7), PPS 0x68 (type 8), IDR 0x65
    // (type 5), non-IDR slice 0x61 (type 1) — ITU-T H.264 Table 7-1.
    const AVC_SPS: &[u8] = &[0x67, 0x42, 0x00, 0x0A];
    const AVC_PPS: &[u8] = &[0x68, 0xCE, 0x38, 0x80];
    const AVC_IDR: &[u8] = &[0x65, 0x88, 0x84, 0x00];
    const AVC_NON_IDR: &[u8] = &[0x61, 0x88, 0x84, 0x00];

    // H.265 NAL headers: SPS type 33 (0x42 0x01), PPS type 34 (0x44 0x01),
    // IDR_W_RADL type 19 (0x26 0x01) — ITU-T H.265 Table 7-1.
    const HEVC_SPS: &[u8] = &[0x42, 0x01, 0x01, 0x60];
    const HEVC_PPS: &[u8] = &[0x44, 0x01, 0xC0, 0xF1];
    const HEVC_IRAP: &[u8] = &[0x26, 0x01, 0xAF, 0x00];

    fn run_avc_au(au: &[u8]) -> Report {
        let mut ts = build_pat_pmt_ts(&[(VIDEO_PID.value(), StreamType::H264)]);
        ts.extend_from_slice(&make_pes_packet(VIDEO_PID, 0, &build_pes(0xE0, au)));
        let mut report = Report::new();
        ParamSetsCheck.run(&ts, &mut report);
        report
    }

    fn run_hevc_au(au: &[u8]) -> Report {
        let mut ts = build_pat_pmt_ts(&[(VIDEO_PID.value(), StreamType::Hevc)]);
        ts.extend_from_slice(&make_pes_packet(VIDEO_PID, 0, &build_pes(0xE0, au)));
        let mut report = Report::new();
        ParamSetsCheck.run(&ts, &mut report);
        report
    }

    /// SPS without PPS is still missing a parameter set.
    #[test]
    fn avc_sps_then_idr_without_pps_is_flagged() {
        let report = run_avc_au(&annexb_au(&[AVC_SPS, AVC_IDR]));
        assert_eq!(
            count_missing_params(&report),
            1,
            "SPS+PPS are both required, got {:?}",
            report.findings()
        );
    }

    /// PPS without SPS is still missing a parameter set.
    #[test]
    fn avc_pps_then_idr_without_sps_is_flagged() {
        let report = run_avc_au(&annexb_au(&[AVC_PPS, AVC_IDR]));
        assert_eq!(
            count_missing_params(&report),
            1,
            "SPS+PPS are both required, got {:?}",
            report.findings()
        );
    }

    /// A non-IDR slice is not a keyframe and must not be flagged before the
    /// parameter sets arrive.
    #[test]
    fn avc_non_idr_slice_before_params_is_not_flagged() {
        let report = run_avc_au(&annexb_au(&[AVC_SPS, AVC_NON_IDR]));
        assert_eq!(
            count_missing_params(&report),
            0,
            "only an IDR access unit is flagged, got {:?}",
            report.findings()
        );
    }

    /// Two IDR access units before any parameter sets produce exactly one
    /// finding (the fault is reported once per PID).
    #[test]
    fn avc_repeated_idr_without_params_is_reported_once() {
        let au = annexb_au(&[AVC_IDR]);
        let mut ts = build_pat_pmt_ts(&[(VIDEO_PID.value(), StreamType::H264)]);
        ts.extend_from_slice(&make_pes_packet(VIDEO_PID, 0, &build_pes(0xE0, &au)));
        ts.extend_from_slice(&make_pes_packet(VIDEO_PID, 1, &build_pes(0xE0, &au)));
        let mut report = Report::new();
        ParamSetsCheck.run(&ts, &mut report);
        assert_eq!(
            count_missing_params(&report),
            1,
            "the fault is reported once per PID, got {:?}",
            report.findings()
        );
    }

    /// An HEVC IRAP before any VPS/SPS/PPS is flagged.
    #[test]
    fn hevc_irap_without_params_is_flagged() {
        let report = run_hevc_au(&annexb_au(&[HEVC_IRAP]));
        assert_eq!(
            count_missing_params(&report),
            1,
            "an IRAP before VPS/SPS/PPS is flagged, got {:?}",
            report.findings()
        );
    }

    /// HEVC SPS+PPS before the IRAP is clean.
    #[test]
    fn hevc_params_before_irap_not_flagged() {
        let report = run_hevc_au(&annexb_au(&[HEVC_SPS, HEVC_PPS, HEVC_IRAP]));
        assert_eq!(
            count_missing_params(&report),
            0,
            "SPS+PPS before the IRAP must not be flagged, got {:?}",
            report.findings()
        );
    }

    /// HEVC SPS without PPS before the IRAP is still missing a parameter set.
    #[test]
    fn hevc_sps_only_then_irap_is_flagged() {
        let report = run_hevc_au(&annexb_au(&[HEVC_SPS, HEVC_IRAP]));
        assert_eq!(
            count_missing_params(&report),
            1,
            "SPS+PPS are both required, got {:?}",
            report.findings()
        );
    }

    /// Two HEVC IRAP access units before any parameter sets produce exactly
    /// one finding.
    #[test]
    fn hevc_repeated_irap_without_params_is_reported_once() {
        let au = annexb_au(&[HEVC_IRAP]);
        let mut ts = build_pat_pmt_ts(&[(VIDEO_PID.value(), StreamType::Hevc)]);
        ts.extend_from_slice(&make_pes_packet(VIDEO_PID, 0, &build_pes(0xE0, &au)));
        ts.extend_from_slice(&make_pes_packet(VIDEO_PID, 1, &build_pes(0xE0, &au)));
        let mut report = Report::new();
        ParamSetsCheck.run(&ts, &mut report);
        assert_eq!(
            count_missing_params(&report),
            1,
            "the fault is reported once per PID, got {:?}",
            report.findings()
        );
    }
}
