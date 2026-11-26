//! `FpsCadenceCheck` positive/negative streams: the VUI-declared frame rate
//! against the measured PES-timestamp cadence (issue #567).
//!
//! Every SPS is recovered from a committed real capture by the same demuxer
//! the check uses, never hand-built bits, so `declared_fps` is a genuine VUI
//! timing value (25 fps for the AVC and HEVC fixtures). Only the PES cadence
//! is synthesised, which is what the check compares against.

use broadcast_core::Serialize;
use dvb_si::descriptors::any::DescriptorLoop;
use dvb_si::tables::pat::{PatEntry, PatSection};
use dvb_si::tables::pmt::{PmtSection, PmtStream, StreamType};
use media_doctor::{Diagnostic, FpsCadenceCheck, Report, Severity};
use mpeg_ts::mux::SectionPacketiser;
use mpeg_ts::pid::Pid;
use transmux::CodecConfig;

const TS_PACKET_SIZE: usize = 188;
const PMT_PID: u16 = 0x0100;
const VIDEO_PID: u16 = 0x0101;

/// H.264 IDR slice NAL (type 5): a keyframe so the demuxer keeps the track.
const AVC_IDR: &[u8] = &[0x65, 0x88, 0x84, 0x00];
/// H.265 IDR_W_RADL NAL (type 19): the HEVC keyframe.
const HEVC_IRAP: &[u8] = &[0x26, 0x01, 0xAF, 0x00];

/// Parameter-set NAL bytes plus the PMT `stream_type` they belong to.
struct ParamSets {
    nals: Vec<Vec<u8>>,
    stream_type: StreamType,
}

fn avc_param_sets(rel: &str) -> ParamSets {
    let ts = test_fixtures::read(rel);
    let media = test_harness::collect::media_from_ts(&ts).expect("fixture demuxes");
    for track in &media.tracks {
        if let CodecConfig::Avc { config, .. } = &track.spec.config {
            let mut nals: Vec<Vec<u8>> = config.config.sps.iter().map(|s| s.0.clone()).collect();
            nals.extend(config.config.pps.iter().map(|p| p.0.clone()));
            return ParamSets {
                nals,
                stream_type: StreamType::H264,
            };
        }
    }
    panic!("{rel}: no AVC track");
}

fn hevc_param_sets(rel: &str) -> ParamSets {
    let ts = test_fixtures::read(rel);
    let media = test_harness::collect::media_from_ts(&ts).expect("fixture demuxes");
    for track in &media.tracks {
        if let CodecConfig::Hevc { config, .. } = &track.spec.config {
            let nals: Vec<Vec<u8>> = config
                .config
                .arrays
                .iter()
                .filter(|a| matches!(a.nal_unit_type, 32..=34))
                .flat_map(|a| a.nalus.iter().map(|n| n.0.clone()))
                .collect();
            return ParamSets {
                nals,
                stream_type: StreamType::Hevc,
            };
        }
    }
    panic!("{rel}: no HEVC track");
}

fn push_section(pid: Pid, section: &[u8], out: &mut Vec<u8>) {
    for pkt in SectionPacketiser::new(pid).packetise(&[section]) {
        out.extend_from_slice(&pkt);
    }
}

/// PAT + PMT declaring one video stream of `stream_type` on `VIDEO_PID`.
fn pat_pmt(stream_type: StreamType) -> Vec<u8> {
    let pat = PatSection::new(
        1,
        0,
        true,
        0,
        0,
        vec![PatEntry::new(1, Pid::from_wire_masked(PMT_PID))],
    );
    let mut pat_bytes = vec![0u8; pat.serialized_len()];
    let n = pat.serialize_into(&mut pat_bytes).expect("serialize PAT");
    pat_bytes.truncate(n);

    let pmt = PmtSection::new(
        1,
        dvb_si::tables::SectionVersion::new(0, true, 0, 0),
        Pid::from_wire_masked(VIDEO_PID),
        DescriptorLoop::new(&[]),
        vec![PmtStream::new(
            stream_type,
            Pid::from_wire_masked(VIDEO_PID),
            DescriptorLoop::new(&[]),
        )],
    );
    let mut pmt_bytes = vec![0u8; pmt.serialized_len()];
    let n = pmt.serialize_into(&mut pmt_bytes).expect("serialize PMT");
    pmt_bytes.truncate(n);

    let mut out = Vec::new();
    push_section(dvb_si::tables::pat::PID, &pat_bytes, &mut out);
    push_section(Pid::from_wire_masked(PMT_PID), &pmt_bytes, &mut out);
    out
}

/// One PES with both PTS and DTS (`PTS_DTS_flags = 11`), as `TsDemux`'s video
/// duration rule reads the DTS delta between access units.
fn pes_with_dts(au: &[u8], dts: u64) -> Vec<u8> {
    let mut out = Vec::new();
    out.extend_from_slice(&[0x00, 0x00, 0x01, 0xE0]);
    let header_data_len = 10u8;
    let payload_len = au.len() + 3 + header_data_len as usize;
    out.extend_from_slice(&(payload_len as u16).to_be_bytes());
    out.push(0x80); // marker + no scrambling
    out.push(0xC0); // PTS_DTS_flags = 11
    out.push(header_data_len);
    out.extend_from_slice(&broadcast_common::clock33::pack_marked(dts, 0b0011));
    out.extend_from_slice(&broadcast_common::clock33::pack_marked(dts, 0b0001));
    out.extend_from_slice(au);
    out
}

fn annexb_au(params: &[Vec<u8>], idr: &[u8]) -> Vec<u8> {
    let mut au = Vec::new();
    for nal in params {
        au.extend_from_slice(&[0x00, 0x00, 0x00, 0x01]);
        au.extend_from_slice(nal);
    }
    au.extend_from_slice(&[0x00, 0x00, 0x00, 0x01]);
    au.extend_from_slice(idr);
    au
}

fn push_pes(pid: u16, cc: &mut u8, pes: &[u8], out: &mut Vec<u8>) {
    let body_cap = TS_PACKET_SIZE - 4;
    assert!(
        pes.len() < body_cap,
        "the synthetic PES must fit one packet"
    );
    let mut pkt = Vec::with_capacity(TS_PACKET_SIZE);
    pkt.push(0x47);
    pkt.push(0x40 | ((pid >> 8) as u8 & 0x1F)); // PUSI
    pkt.push((pid & 0xFF) as u8);
    let af_len = body_cap - 1 - pes.len();
    pkt.push(0x30 | (*cc & 0x0F)); // adaptation + payload
    pkt.push(af_len as u8);
    if af_len > 0 {
        pkt.push(0x00); // adaptation flags
        pkt.extend(core::iter::repeat_n(0xFF, af_len - 1));
    }
    pkt.extend_from_slice(pes);
    assert_eq!(pkt.len(), TS_PACKET_SIZE, "TS packet must be 188 bytes");
    *cc = cc.wrapping_add(1);
    out.extend_from_slice(&pkt);
}

/// Synthesize a single-video-program TS: the first access unit carries the
/// real parameter sets, then one AU per entry in `steps`, where `steps[i]` is
/// the DTS advance to the next AU.
fn synth_ts(params: &ParamSets, idr: &[u8], steps: &[u32]) -> Vec<u8> {
    let mut out = pat_pmt(params.stream_type);
    let mut cc = 0u8;
    let mut dts = 0u64;
    push_pes(
        VIDEO_PID,
        &mut cc,
        &pes_with_dts(&annexb_au(&params.nals, idr), dts),
        &mut out,
    );
    for &step in steps {
        dts += u64::from(step);
        push_pes(
            VIDEO_PID,
            &mut cc,
            &pes_with_dts(&annexb_au(&[], idr), dts),
            &mut out,
        );
    }
    out
}

fn check(ts: &[u8]) -> Vec<media_doctor::Finding> {
    let mut report = Report::new();
    FpsCadenceCheck.run(ts, &mut report);
    report
        .findings()
        .iter()
        .filter(|f| f.rule_id == "fps-cadence-mismatch")
        .cloned()
        .collect()
}

/// A 25 fps VUI with 40 ms (3600-tick) spacing measures 25 fps: clean.
#[test]
fn avc_25fps_cadence_is_clean() {
    let params = avc_param_sets("ts/h264/baseline.ts");
    let hits = check(&synth_ts(&params, AVC_IDR, &[3600; 7]));
    assert!(hits.is_empty(), "25 fps cadence must be clean: {hits:?}");
}

/// The same 25 fps VUI with 20 ms (1800-tick) spacing measures 50 fps: one
/// Warning naming both rates.
#[test]
fn avc_50fps_cadence_is_flagged() {
    let params = avc_param_sets("ts/h264/baseline.ts");
    let hits = check(&synth_ts(&params, AVC_IDR, &[1800; 7]));
    assert_eq!(hits.len(), 1, "{hits:?}");
    assert_eq!(hits[0].severity, Severity::Warning);
    assert!(
        hits[0].message.contains("25.000") && hits[0].message.contains("50.000"),
        "the message must name declared 25 and measured 50 fps: {:?}",
        hits[0].message
    );
}

/// Three access units are below `MIN_SAMPLES_FOR_CADENCE`: no finding even
/// though the cadence is 50 fps.
#[test]
fn avc_three_samples_are_below_the_minimum() {
    let params = avc_param_sets("ts/h264/baseline.ts");
    let hits = check(&synth_ts(&params, AVC_IDR, &[1800, 1800]));
    assert!(hits.is_empty(), "3 samples must be skipped: {hits:?}");
}

/// Exactly `MIN_SAMPLES_FOR_CADENCE` (4) samples are checked.
#[test]
fn avc_four_samples_are_checked() {
    let params = avc_param_sets("ts/h264/baseline.ts");
    let hits = check(&synth_ts(&params, AVC_IDR, &[1800, 1800, 1800]));
    assert_eq!(hits.len(), 1, "4 samples must be checked: {hits:?}");
}

/// An interlaced SPS (`frame_mbs_only_flag == 0`) is skipped even at 50 fps.
#[test]
fn avc_interlaced_sps_is_skipped() {
    let params = avc_param_sets("ts/h264/interlaced.ts");
    let hits = check(&synth_ts(&params, AVC_IDR, &[1800; 7]));
    assert!(hits.is_empty(), "interlaced coding is skipped: {hits:?}");
}

/// All AUs sharing one DTS means zero sample durations: no finding.
#[test]
fn avc_zero_durations_are_skipped() {
    let params = avc_param_sets("ts/h264/baseline.ts");
    let hits = check(&synth_ts(&params, AVC_IDR, &[0; 7]));
    assert!(hits.is_empty(), "zero durations are skipped: {hits:?}");
}

/// The tolerance boundary is strict: measured 27.44 fps (9.8% over) is inside,
/// 27.61 fps (10.4% over) is outside.
#[test]
fn avc_tolerance_boundary_is_strict() {
    let params = avc_param_sets("ts/h264/baseline.ts");
    // 90000 / 3280 = 27.439 fps -> 9.76% over: inside.
    let inside = check(&synth_ts(&params, AVC_IDR, &[3280; 7]));
    assert!(inside.is_empty(), "just inside tolerance: {inside:?}");
    // 90000 / 3260 = 27.607 fps -> 10.43% over: outside.
    let outside = check(&synth_ts(&params, AVC_IDR, &[3260; 7]));
    assert_eq!(outside.len(), 1, "just outside tolerance: {outside:?}");
}

/// The HEVC arm behaves identically: clean at 25 fps, flagged at 50 fps.
#[test]
fn hevc_cadence_matches_the_avc_behaviour() {
    let params = hevc_param_sets("ts/hevc/main.ts");
    let clean = check(&synth_ts(&params, HEVC_IRAP, &[3600; 7]));
    assert!(clean.is_empty(), "HEVC 25 fps cadence: {clean:?}");
    let fast = check(&synth_ts(&params, HEVC_IRAP, &[1800; 7]));
    assert_eq!(fast.len(), 1, "HEVC 50 fps cadence: {fast:?}");
}
