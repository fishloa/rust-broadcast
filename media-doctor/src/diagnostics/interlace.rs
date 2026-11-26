//! `InterlaceCheck` — surfaces AVC interlaced coding tools (issue #567).
//!
//! Decodes the first SPS seen on every PMT-declared H.264 PID
//! (`stream_type` `0x1B`) via [`broadcast_bitstream::sps::decode_avc_sps`] (reused, not
//! duplicated) and flags `frame_mbs_only_flag == 0` (ITU-T H.264
//! §7.3.2.1.1) — interlaced coding tools (PicAFF/field pictures) are
//! enabled.
//!
//! The issue's scope frames this as "interlaced SPS vs a progressive
//! container claim", but neither MPEG-2 TS PMT signalling (ISO/IEC
//! 13818-1) nor this crate's ISOBMFF IR carries an explicit
//! progressive/interlace *container* flag to compare against — there is no
//! declared claim to cross-validate here. This check instead surfaces the
//! bitstream fact plainly (`Info` severity, not a mismatch `Error`/`Warning`)
//! so a caller can judge it against whatever downstream progressive
//! assumption applies to their pipeline.
//!
//! HEVC is out of scope: `broadcast_bitstream::sps::decode_hevc_sps` does not decode
//! the VUI `field_seq_flag`/`frame_field_info_present_flag` interlace
//! signalling (ITU-T H.265 §E.2.1), so there is no decoded field to check
//! without adding new decode surface to `transmux` — out of bounds for this
//! issue (`media-doctor` consumes `transmux`, it does not extend it).

use alloc::boxed::Box;
use alloc::collections::btree_map::BTreeMap;
use alloc::format;
use alloc::vec::Vec;
use mpeg_ts::pid::Pid;

use broadcast_bitstream::annexb::iter_annexb_nals;
use broadcast_bitstream::sps::decode_avc_sps;
use dvb_si::tables::pmt::StreamType;

use crate::diagnostics::codec_common::{AccessUnitScanner, pids_with_stream_type};
use crate::report::{Finding, Location, Severity};
use crate::{Diagnostic, DiagnosticScan, PmtStreamIndex, Report};

/// H.264 `nal_unit_type` mask (ITU-T H.264 §7.3.1, bits `[4:0]`).
const H264_NAL_TYPE_MASK: u8 = 0x1F;
/// H.264 SPS `nal_unit_type` (ITU-T H.264 Table 7-1).
const H264_NAL_SPS: u8 = 7;

/// Surfaces AVC `frame_mbs_only_flag == 0` (interlaced coding tools enabled)
/// on every PMT-declared H.264 PID's first decodable SPS.
#[derive(Debug, Clone, Copy)]
pub struct InterlaceCheck;

impl Diagnostic for InterlaceCheck {
    fn needs_pmt_streams(&self) -> bool {
        true
    }

    fn scan(&self, pmt: &PmtStreamIndex) -> Box<dyn DiagnosticScan> {
        let avc_pids = pids_with_stream_type(pmt.streams(), StreamType::H264);
        // `false` = not yet decoded; `true` = already reported, stop looking
        // on this PID.
        let reported = avc_pids.iter().map(|&p| (p, false)).collect();
        Box::new(InterlaceScan {
            avc_pids,
            reported,
            assembler: AccessUnitScanner::new(),
        })
    }
}

/// One streaming pass of [`InterlaceCheck`].
struct InterlaceScan {
    avc_pids: Vec<Pid>,
    reported: BTreeMap<Pid, bool>,
    assembler: AccessUnitScanner,
}

impl DiagnosticScan for InterlaceScan {
    fn feed(&mut self, packet: &[u8], report: &mut Report) {
        if self.avc_pids.is_empty() {
            return;
        }
        let Self {
            avc_pids,
            reported,
            assembler,
        } = self;
        let mut wanted = |pid: Pid| avc_pids.contains(&pid);
        assembler.feed(packet, &mut wanted, &mut |payload, packet_index, pid| {
            scan_payload(reported, payload, packet_index, pid, report)
        });
    }

    fn finish(&mut self, report: &mut Report) {
        let Self {
            reported,
            assembler,
            ..
        } = self;
        assembler.finish(&mut |payload, packet_index, pid| {
            scan_payload(reported, payload, packet_index, pid, report)
        });
    }
}

/// Check one access unit for the first decodable AVC SPS on `pid`.
fn scan_payload(
    reported: &mut BTreeMap<Pid, bool>,
    payload: &[u8],
    packet_index: usize,
    pid: Pid,
    report: &mut Report,
) {
    let Some(done) = reported.get(&pid).copied() else {
        return;
    };
    if done {
        return;
    }
    for nal in iter_annexb_nals(payload) {
        if nal.is_empty() || nal[0] & H264_NAL_TYPE_MASK != H264_NAL_SPS {
            continue;
        }
        let Ok(info) = decode_avc_sps(nal) else {
            continue;
        };
        reported.insert(pid, true);
        if !info.frame_mbs_only() {
            report.push(Finding::new(
                Severity::Info,
                Location::at_pid(packet_index, pid),
                "avc-interlaced-content",
                format!(
                    "AVC SPS on PID 0x{pid:04X} has frame_mbs_only_flag=0 \
                     (interlaced coding tools enabled) — ITU-T H.264 §7.3.2.1.1"
                ),
            ));
        }
        break;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::diagnostics::codec_common::tests::{build_pat_pmt_ts, build_pes, make_pes_packet};

    const VIDEO_PID: Pid = Pid::from_const(0x101);

    /// A progressive SPS (`frame_mbs_only_flag=1`) must not be flagged.
    /// Real SPS extracted for `decode_high_sps_with_scaling_lists` in
    /// `broadcast_bitstream::sps` tests (320x240 High profile, progressive).
    #[test]
    fn progressive_sps_not_flagged() {
        let sps: &[u8] = &[
            0x67, 0x64, 0x00, 0x0D, 0xAD, 0xC8, 0xBF, 0xFE, 0x03, 0xC1, 0x41, 0xF9,
        ];
        let mut au = alloc::vec::Vec::new();
        au.extend_from_slice(&[0x00, 0x00, 0x01]);
        au.extend_from_slice(sps);

        let mut ts = build_pat_pmt_ts(&[(VIDEO_PID.value(), StreamType::H264)]);
        ts.extend_from_slice(&make_pes_packet(VIDEO_PID, 0, &build_pes(0xE0, &au)));

        let mut report = Report::new();
        InterlaceCheck.run(&ts, &mut report);
        assert!(
            report
                .findings()
                .iter()
                .all(|f| f.rule_id != "avc-interlaced-content"),
            "progressive SPS must not be flagged, got {:?}",
            report.findings()
        );
    }

    /// A minimal hand-encoded Baseline-profile SPS with `frame_mbs_only_flag`
    /// explicitly cleared. Bit layout (after the 4-byte
    /// header/profile_idc/constraint_flags/level_idc, decoded per ITU-T
    /// H.264 §7.3.2.1.1 — profile 66 skips the high-profile branch):
    ///
    /// ```text
    /// seq_parameter_set_id           ue(0)  = 1
    /// log2_max_frame_num_minus4      ue(0)  = 1
    /// pic_order_cnt_type             ue(2)  = 011   (type 2: no extra fields)
    /// max_num_ref_frames             ue(1)  = 010
    /// gaps_in_frame_num_allowed_flag u(1)   = 0
    /// pic_width_in_mbs_minus1        ue(0)  = 1
    /// pic_height_in_map_units_minus1 ue(0)  = 1
    /// frame_mbs_only_flag            u(1)   = 0     <- interlaced
    /// mb_adaptive_frame_field_flag   u(1)   = 0     (present: frame_mbs_only=0)
    /// direct_8x8_inference_flag      u(1)   = 1
    /// frame_cropping_flag            u(1)   = 0
    /// vui_parameters_present_flag    u(1)   = 0
    /// ```
    /// packed MSB-first into 3 bytes: `0xDA 0x64 0x80` (decoding stops at
    /// `vui_parameters_present_flag`, so the trailing padding bits are never
    /// read). Verified by the `baseline_interlaced_sps_decodes_as_expected`
    /// regression test below.
    const INTERLACED_SPS: &[u8] = &[0x67, 0x42, 0x00, 0x0A, 0xDA, 0x64, 0x80];

    /// Regression guard for [`INTERLACED_SPS`]'s hand-computed bit packing:
    /// pins the exact decoded fields so a future edit that breaks the
    /// encoding fails here first, not in `interlaced_sps_is_flagged`.
    #[test]
    fn baseline_interlaced_sps_decodes_as_expected() {
        let info = decode_avc_sps(INTERLACED_SPS).expect("hand-built SPS must decode");
        assert_eq!(info.profile_idc(), 66);
        assert!(!info.frame_mbs_only(), "SPS must decode as interlaced");
    }

    #[test]
    fn interlaced_sps_is_flagged() {
        let mut au = alloc::vec::Vec::new();
        au.extend_from_slice(&[0x00, 0x00, 0x01]);
        au.extend_from_slice(INTERLACED_SPS);

        let mut ts = build_pat_pmt_ts(&[(VIDEO_PID.value(), StreamType::H264)]);
        ts.extend_from_slice(&make_pes_packet(VIDEO_PID, 0, &build_pes(0xE0, &au)));

        let mut report = Report::new();
        InterlaceCheck.run(&ts, &mut report);
        assert!(
            report
                .findings()
                .iter()
                .any(|f| f.rule_id == "avc-interlaced-content"
                    && f.location.stream == crate::StreamLocator::Ts(VIDEO_PID)),
            "expected avc-interlaced-content, got {:?}",
            report.findings()
        );
    }
}
