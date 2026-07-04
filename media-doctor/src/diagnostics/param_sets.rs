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
//! probe resolved. NAL classification is `transmux::nal`/`transmux::annexb`
//! throughout — no duplicated NAL/SPS parsing.

use alloc::collections::btree_map::BTreeMap;
use alloc::format;

use dvb_si::tables::pmt::StreamType;
use transmux::annexb::iter_annexb_nals;
use transmux::nal::{NalCodec, is_keyframe_nal};

use crate::Diagnostic;
use crate::Report;
use crate::diagnostics::codec_common::{
    collect_pmt_streams, for_each_access_unit, pids_with_stream_type,
};
use crate::report::{Finding, Location, Severity};

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
    fn run(&self, ts: &[u8], report: &mut Report) {
        let declared = collect_pmt_streams(ts);
        let avc_pids = pids_with_stream_type(&declared, StreamType::H264);
        let hevc_pids = pids_with_stream_type(&declared, StreamType::Hevc);
        if avc_pids.is_empty() && hevc_pids.is_empty() {
            return;
        }

        let mut avc_state: BTreeMap<u16, ParamState> = avc_pids
            .iter()
            .map(|&p| (p, ParamState::default()))
            .collect();
        let mut hevc_state: BTreeMap<u16, ParamState> = hevc_pids
            .iter()
            .map(|&p| (p, ParamState::default()))
            .collect();

        for_each_access_unit(
            ts,
            |pid| avc_pids.contains(&pid) || hevc_pids.contains(&pid),
            |payload, packet_index, pid| {
                if let Some(state) = avc_state.get_mut(&pid) {
                    walk_avc(payload, packet_index, pid, state, report);
                } else if let Some(state) = hevc_state.get_mut(&pid) {
                    walk_hevc(payload, packet_index, pid, state, report);
                }
            },
        );
    }
}

fn walk_avc(
    payload: &[u8],
    packet_index: usize,
    pid: u16,
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
                        Location::new(packet_index, pid),
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
    pid: u16,
    state: &mut ParamState,
    report: &mut Report,
) {
    for nal in iter_annexb_nals(payload) {
        if nal.is_empty() || state.reported {
            continue;
        }
        match transmux::nal::nal_unit_type(NalCodec::Hevc, nal) {
            Some(HEVC_NAL_SPS) => state.seen_sps = true,
            Some(HEVC_NAL_PPS) => state.seen_pps = true,
            _ => {
                if is_keyframe_nal(NalCodec::Hevc, nal) && !(state.seen_sps && state.seen_pps) {
                    report.push(Finding::new(
                        Severity::Error,
                        Location::new(packet_index, pid),
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

    const VIDEO_PID: u16 = 0x101;

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

        let mut ts = build_pat_pmt_ts(&[(VIDEO_PID, StreamType::H264)]);
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

        let mut ts = build_pat_pmt_ts(&[(VIDEO_PID, StreamType::H264)]);
        ts.extend_from_slice(&make_pes_packet(VIDEO_PID, 0, &build_pes(0xE0, &au)));

        let mut report = Report::new();
        ParamSetsCheck.run(&ts, &mut report);
        assert!(
            report
                .findings()
                .iter()
                .any(|f| f.rule_id == "missing-parameter-sets" && f.location.pid == VIDEO_PID),
            "expected missing-parameter-sets, got {:?}",
            report.findings()
        );
    }
}
