//! `CodecSignallingCheck` — PMT `stream_type` vs. the actual elementary
//! codec (issue #567).
//!
//! ISO/IEC 13818-1 Table 2-34 lets a PMT declare a `stream_type` for each
//! elementary PID (e.g. `0x1B` = H.264, `0x24` = HEVC, `0x0F` = AAC ADTS).
//! This check walks the raw elementary-stream bytes of every PMT-declared
//! `0x1B`/`0x24`/`0x0F` PID (via [`transmux::iter_annexb_nals`] for video,
//! [`transmux::parse_adts_header`] for audio — reused, not duplicated) and
//! flags a PID whose bytes never once look like the declared codec's framing
//! at all: zero Annex B NAL units anywhere on a declared H.264/HEVC PID, or
//! zero valid ADTS sync anywhere on a declared AAC-ADTS PID.
//!
//! # Why "any framing at all", not a full decode or a `TsDemux` track
//!
//! Two stronger designs were tried and rejected against this crate's own
//! committed real fixtures (the `PtsCheck`-lesson hard gate: a check that
//! fires on real clean content is rejected, no exceptions):
//!
//! - Requiring a fully **decodable SPS** on the PID: real short/trimmed
//!   broadcast captures (e.g. `fixtures/ts/france-pcr-discontinuity.ts`'s
//!   secondary-program video PIDs, `0x00DC`/`0x026C`/`0x02D0`) carry many
//!   genuine H.264 access units — slice/SEI/AUD NALs, in wire order — with
//!   **no parameter-set refresh anywhere inside the captured window**, since
//!   SPS/PPS repeat on their own cadence independent of clip length. Full
//!   decode is not a matter of the ES having framing, only of how the clip
//!   happened to be cut.
//! - Requiring [`transmux::TsDemux`] to resolve a track with a matching
//!   `source_pid`: `TsDemux`'s incremental multi-track engine has its own
//!   preconditions (PCR/continuity resync, per-AU duration resolution,
//!   PMT-declaration-order promotion) that a real capture can fail to
//!   satisfy for reasons that have nothing to do with codec signalling
//!   correctness — verified false positives on exactly the fixtures above,
//!   plus `fixtures/ts/m6-*.ts` (a PMT-declared video PID with zero packets
//!   at all in a trimmed capture).
//!
//! "Any NAL/ADTS framing at all" is the weakest bar that still catches the
//! issue's literal example — "stream_type says AVC but the ES isn't NAL"
//! (raw bytes, a different transport syntax entirely, or no ES data shaped
//! like a bitstream) — without turning a short real capture's parameter-set
//! cadence, or a demuxer's unrelated multi-track scheduling, into a false
//! signalling-mismatch finding.
//!
//! A PID that never carries **any** access unit at all (also seen in this
//! crate's own fixtures — a PMT entry for a PID absent from a trimmed
//! capture) is likewise not flagged: there is no bitstream to judge, so
//! there is nothing to disagree with the PMT's claim.

use alloc::collections::btree_map::BTreeMap;
use alloc::vec::Vec;

use dvb_si::tables::pmt::StreamType;
use transmux::{iter_annexb_nals, parse_adts_header};

use crate::Diagnostic;
use crate::Report;
use crate::diagnostics::codec_common::{
    collect_pmt_streams, for_each_access_unit, pids_with_stream_type,
};
use crate::report::{Finding, Location, Severity};

/// Per-PID tracking: has any access unit been observed at all, and did any
/// of them look like the declared codec's framing.
#[derive(Debug, Default, Clone, Copy)]
struct Seen {
    any_au: bool,
    structured: bool,
}

/// Cross-validates PMT `stream_type` (`0x1B`/`0x24`/`0x0F`) against the
/// actual elementary-stream framing.
#[derive(Debug, Clone, Copy)]
pub struct CodecSignallingCheck;

impl Diagnostic for CodecSignallingCheck {
    fn run(&self, ts: &[u8], report: &mut Report) {
        let declared = collect_pmt_streams(ts);
        let video_pids: Vec<u16> = pids_with_stream_type(&declared, StreamType::H264)
            .into_iter()
            .chain(pids_with_stream_type(&declared, StreamType::Hevc))
            .collect();
        let audio_pids = pids_with_stream_type(&declared, StreamType::AacAdts);
        if video_pids.is_empty() && audio_pids.is_empty() {
            return;
        }

        let mut video_seen: BTreeMap<u16, Seen> =
            video_pids.iter().map(|&p| (p, Seen::default())).collect();
        let mut audio_seen: BTreeMap<u16, Seen> =
            audio_pids.iter().map(|&p| (p, Seen::default())).collect();

        for_each_access_unit(
            ts,
            |pid| video_pids.contains(&pid) || audio_pids.contains(&pid),
            |payload, _packet_index, pid| {
                if let Some(seen) = video_seen.get_mut(&pid) {
                    seen.any_au = true;
                    if iter_annexb_nals(payload).next().is_some() {
                        seen.structured = true;
                    }
                } else if let Some(seen) = audio_seen.get_mut(&pid) {
                    seen.any_au = true;
                    if has_adts_sync(payload) {
                        seen.structured = true;
                    }
                }
            },
        );

        for (&pid, seen) in &video_seen {
            if seen.any_au && !seen.structured {
                report.push(Finding::new(
                    Severity::Error,
                    Location::new(0, pid),
                    "codec-signalling-mismatch",
                    alloc::format!(
                        "PMT declares a NAL video codec on PID 0x{pid:04X} but its elementary \
                         stream never contains a single Annex B start code — the stream_type \
                         claim and the bitstream disagree (ISO/IEC 13818-1 Table 2-34)",
                    ),
                ));
            }
        }
        for (&pid, seen) in &audio_seen {
            if seen.any_au && !seen.structured {
                report.push(Finding::new(
                    Severity::Error,
                    Location::new(0, pid),
                    "codec-signalling-mismatch",
                    alloc::format!(
                        "PMT declares stream_type AAC-ADTS (0x0F) on PID 0x{pid:04X} but its \
                         elementary stream never contains a valid ADTS sync — the stream_type \
                         claim and the bitstream disagree (ISO/IEC 13818-1 Table 2-34)",
                    ),
                ));
            }
        }
    }
}

/// Scan `payload` for a byte offset at which a well-formed ADTS header
/// parses (ISO/IEC 13818-7 §6.2, via [`transmux::parse_adts_header`]) — not
/// assumed to sit at offset 0 so this tolerates leading PES stuffing.
fn has_adts_sync(payload: &[u8]) -> bool {
    const ADTS_MIN: usize = 7;
    if payload.len() < ADTS_MIN {
        return false;
    }
    (0..=payload.len() - ADTS_MIN).any(|off| {
        payload[off] == 0xFF
            && (payload[off + 1] & 0xF0) == 0xF0
            && parse_adts_header(&payload[off..]).is_ok()
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::diagnostics::codec_common::tests::{build_pat_pmt_ts, build_pes, make_pes_packet};
    use crate::report::Report;

    /// A PMT declaring H.264 on a PID that never carries any access unit at
    /// all (absent from the capture, e.g. a trimmed real fixture) must NOT be
    /// flagged — there is no bitstream to disagree with the PMT.
    #[test]
    fn declared_pid_with_no_es_at_all_not_flagged() {
        let ts = build_pat_pmt_ts(&[(0x101, StreamType::H264)]);
        let mut report = Report::new();
        CodecSignallingCheck.run(&ts, &mut report);
        assert!(
            report
                .findings()
                .iter()
                .all(|f| f.rule_id != "codec-signalling-mismatch"),
            "a declared PID with zero access units must not be flagged, got {:?}",
            report.findings()
        );
    }

    /// PMT declares H.264 on a PID that DOES carry PES traffic, but the
    /// payload has no Annex-B start codes at all (not NAL-structured) — the
    /// literal "PMT says AVC but the ES isn't NAL" case from the issue.
    #[test]
    fn declared_h264_with_non_nal_payload_is_flagged() {
        const VIDEO_PID: u16 = 0x101;
        let garbage: &[u8] = &[0xAA; 32]; // no 00 00 01 start codes anywhere
        let mut ts = build_pat_pmt_ts(&[(VIDEO_PID, StreamType::H264)]);
        ts.extend_from_slice(&make_pes_packet(VIDEO_PID, 0, &build_pes(0xE0, garbage)));

        let mut report = Report::new();
        CodecSignallingCheck.run(&ts, &mut report);
        assert!(
            report
                .findings()
                .iter()
                .any(|f| f.rule_id == "codec-signalling-mismatch" && f.location.pid == VIDEO_PID),
            "expected codec-signalling-mismatch for a non-NAL H.264 PID, got {:?}",
            report.findings()
        );
    }

    /// PMT declares H.264 on a PID carrying a real-shaped NAL access unit
    /// (SPS+PPS+slice) — must not be flagged.
    #[test]
    fn declared_h264_with_real_nal_payload_not_flagged() {
        const VIDEO_PID: u16 = 0x101;
        let sps: &[u8] = &[
            0x67, 0x64, 0x00, 0x0D, 0xAD, 0xC8, 0xBF, 0xFE, 0x03, 0xC1, 0x41, 0xF9,
        ];
        let pps: &[u8] = &[0x68, 0xCE, 0x38, 0x80];
        let slice: &[u8] = &[0x65, 0x88, 0x84, 0x00];

        let mut au = Vec::new();
        for nal in [sps, pps, slice] {
            au.extend_from_slice(&[0x00, 0x00, 0x01]);
            au.extend_from_slice(nal);
        }
        let mut ts = build_pat_pmt_ts(&[(VIDEO_PID, StreamType::H264)]);
        ts.extend_from_slice(&make_pes_packet(VIDEO_PID, 0, &build_pes(0xE0, &au)));

        let mut report = Report::new();
        CodecSignallingCheck.run(&ts, &mut report);
        assert!(
            report
                .findings()
                .iter()
                .all(|f| f.rule_id != "codec-signalling-mismatch"),
            "a real NAL-structured AVC access unit must not be flagged, got {:?}",
            report.findings()
        );
    }

    /// PMT declares AAC-ADTS on a PID whose payload never contains a valid
    /// ADTS sync — flagged.
    #[test]
    fn declared_aac_with_non_adts_payload_is_flagged() {
        const AUDIO_PID: u16 = 0x102;
        let garbage: &[u8] = &[0x00; 32];
        let mut ts = build_pat_pmt_ts(&[(AUDIO_PID, StreamType::AacAdts)]);
        ts.extend_from_slice(&make_pes_packet(AUDIO_PID, 0, &build_pes(0xC0, garbage)));

        let mut report = Report::new();
        CodecSignallingCheck.run(&ts, &mut report);
        assert!(
            report
                .findings()
                .iter()
                .any(|f| f.rule_id == "codec-signalling-mismatch" && f.location.pid == AUDIO_PID),
            "expected codec-signalling-mismatch for a non-ADTS AAC PID, got {:?}",
            report.findings()
        );
    }

    /// PMT declares AAC-ADTS on a PID carrying a real ADTS-framed frame —
    /// must not be flagged.
    #[test]
    fn declared_aac_with_real_adts_payload_not_flagged() {
        const AUDIO_PID: u16 = 0x102;
        // A minimal valid ADTS header (AAC-LC, 44.1kHz, stereo), built via
        // transmux's own builder rather than hand-encoded.
        let header = transmux::build_adts_header(1, 4, 2, 7); // frame_len=7 (header only)
        let mut ts = build_pat_pmt_ts(&[(AUDIO_PID, StreamType::AacAdts)]);
        ts.extend_from_slice(&make_pes_packet(AUDIO_PID, 0, &build_pes(0xC0, &header)));

        let mut report = Report::new();
        CodecSignallingCheck.run(&ts, &mut report);
        assert!(
            report
                .findings()
                .iter()
                .all(|f| f.rule_id != "codec-signalling-mismatch"),
            "a real ADTS-framed AAC PID must not be flagged, got {:?}",
            report.findings()
        );
    }

    /// Real committed captures must never be flagged — the ultimate
    /// clean-negative gate (mirrors the crate-wide hard gate this issue
    /// requires; see also `media-doctor/tests/codec_v2_real_captures.rs`).
    #[test]
    fn real_captures_not_flagged() {
        for rel in [
            "ts/h264/baseline.ts",
            "ts/hevc/main.ts",
            "ts/h264_aac.ts",
            "ts/france-pcr-discontinuity.ts",
            "ts/m6-single.ts",
        ] {
            let path = alloc::format!("{}/../fixtures/{rel}", env!("CARGO_MANIFEST_DIR"));
            let ts = std::fs::read(&path).unwrap_or_else(|e| panic!("read fixture {path}: {e}"));
            let mut report = Report::new();
            CodecSignallingCheck.run(&ts, &mut report);
            assert!(
                report
                    .findings()
                    .iter()
                    .all(|f| f.rule_id != "codec-signalling-mismatch"),
                "real capture {rel} must not be flagged, got {:?}",
                report.findings()
            );
        }
    }
}
