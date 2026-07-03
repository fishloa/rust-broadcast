//! Gate tests for issues #556 (per-sample source timing / AC-3 & E-AC-3
//! syncframe splitting) and #557 (opaque PES data tracks + PCR timeline).
//!
//! Every assertion below is checked against an oracle *independently*
//! derived in this file (a fresh walk of the raw TS bytes: PES reassembly,
//! PMT section parsing, or an adaptation-field scan) — never against the
//! `TsDemux` internals under test.

use std::collections::HashMap;
use std::path::PathBuf;

use broadcast_common::{Package, Unpackage};
use mpeg_pes::PesAssembler;
use mpeg_ts::ts::{SectionReassembler, TsPacket};

use transmux::media::CmafMux;
use transmux::pipeline::{CodecConfig, Sample, SourceTiming, TrackSpec};
use transmux::{Ec3SyncframeInfo, Error, Fmp4Demux, Media, PcrSample, Track, TsDemux};

// ── Fixture loading ─────────────────────────────────────────────────────────

fn fixtures_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../fixtures/ts")
}

fn read_fixture(rel: &str) -> Vec<u8> {
    let path = fixtures_dir().join(rel);
    let data = std::fs::read(&path).unwrap_or_else(|e| panic!("{rel}: {e}"));
    assert_eq!(
        data.len() % 188,
        0,
        "{rel}: must be whole 188-byte TS packets"
    );
    data
}

fn demux(data: &[u8]) -> Media {
    TsDemux::new()
        .unpackage(data)
        .expect("TS demux must succeed")
}

// ── Independent PES-reassembly oracle ────────────────────────────────────────

/// One PES access unit collected by an independent walk of the raw TS bytes
/// (separate from `TsDemux`'s own PES reassembly).
struct PesAu {
    payload: Vec<u8>,
    pts: Option<u64>,
    dts: Option<u64>,
}

/// Reassemble every PES access unit on `target_pid`, in stream order.
fn collect_pes(data: &[u8], target_pid: u16) -> Vec<PesAu> {
    let mut assembler = PesAssembler::new();
    let mut out = Vec::new();
    for chunk in data.chunks_exact(188) {
        let Ok(pkt) = TsPacket::parse(chunk) else {
            continue;
        };
        if pkt.header.pid != target_pid {
            continue;
        }
        let Some(payload) = pkt.payload else {
            continue;
        };
        if let Some(completed) = assembler.feed(pkt.header.pusi, payload) {
            push_pes(&mut out, &completed);
        }
    }
    if let Some(completed) = assembler.flush() {
        push_pes(&mut out, &completed);
    }
    out
}

fn push_pes(out: &mut Vec<PesAu>, bytes: &[u8]) {
    let Ok(pes) = mpeg_pes::PesPacket::parse(bytes) else {
        return;
    };
    if pes.payload.is_empty() {
        return;
    }
    let pts = pes.header.as_ref().and_then(|h| h.pts.map(|p| p.0));
    let dts = pes.header.as_ref().and_then(|h| h.dts.map(|d| d.0));
    out.push(PesAu {
        payload: pes.payload.to_vec(),
        pts,
        dts,
    });
}

/// Cumulative byte offsets: `starts[i]` is the sum of `lens[0..i]`.
fn cumulative_starts(lens: &[usize]) -> Vec<usize> {
    let mut acc = 0usize;
    let mut out = Vec::with_capacity(lens.len());
    for &l in lens {
        out.push(acc);
        acc += l;
    }
    out
}

/// Assert that splitting a track's PES access units into `samples` lost no
/// bytes (concatenation is byte-identical to the concatenated PES payloads)
/// and that every sample landing exactly on a PES boundary carries that PES's
/// PTS in `source_timing`, exactly (0 ticks of drift).
fn assert_pes_boundary_timing(samples: &[Sample], pes: &[PesAu]) {
    let sample_lens: Vec<usize> = samples.iter().map(|s| s.data.len()).collect();
    let sample_starts = cumulative_starts(&sample_lens);
    let pes_lens: Vec<usize> = pes.iter().map(|p| p.payload.len()).collect();
    let pes_starts = cumulative_starts(&pes_lens);

    let concat_samples: Vec<u8> = samples.iter().flat_map(|s| s.data.clone()).collect();
    let concat_pes: Vec<u8> = pes.iter().flat_map(|p| p.payload.clone()).collect();
    assert_eq!(
        concat_samples, concat_pes,
        "splitting must be byte-identical / lossless"
    );

    for (&pes_start, pes_au) in pes_starts.iter().zip(pes.iter()) {
        let idx = sample_starts
            .iter()
            .position(|&s| s == pes_start)
            .expect("every PES boundary must align exactly with a sample boundary");
        let st = samples[idx]
            .source_timing
            .expect("a PES-boundary sample must carry source_timing");
        if let Some(pts) = pes_au.pts {
            assert_eq!(
                st.pts, pts,
                "PES-boundary sample PTS must equal the PES PTS exactly (0 ticks)"
            );
        }
    }
}

// ── #556: AC-3 / E-AC-3 syncframe splitting + exact PES-boundary PTS ────────

const DOLBY_AUDIO_PID: u16 = 0x0100;

#[test]
fn ac3_syncframe_splitting_exact_pts_and_duration() {
    let data = read_fixture("dolby/ac3.ts");
    let pes = collect_pes(&data, DOLBY_AUDIO_PID);
    assert!(!pes.is_empty(), "fixture must carry AC-3 PES packets");

    let media = demux(&data);
    let track = media
        .tracks
        .iter()
        .find(|t| matches!(t.spec.config, CodecConfig::Ac3 { .. }))
        .expect("AC-3 track");

    assert!(
        track.samples.len() >= pes.len(),
        "at least one syncframe per PES access unit"
    );
    assert_pes_boundary_timing(&track.samples, &pes);
    for s in &track.samples {
        assert_eq!(
            s.duration,
            transmux::ac3::AC3_SAMPLES_PER_SYNCFRAME,
            "every AC-3 sample duration must be 1536 (6 blocks x 256 samples)"
        );
    }
}

#[test]
fn eac3_syncframe_splitting_exact_pts_and_duration() {
    let data = read_fixture("dolby/eac3.ts");
    let pes = collect_pes(&data, DOLBY_AUDIO_PID);
    assert!(!pes.is_empty(), "fixture must carry E-AC-3 PES packets");

    // Independent `numblks` oracle: parse the BSI of the very first syncframe
    // (the library's own BSI parser, exactly as the pre-existing `dolby.rs`
    // gate test does — the byte-exactness of the *split* / *timing*, not the
    // BSI bit-parsing itself, is what this test exercises).
    let info = Ec3SyncframeInfo::from_es(&pes[0].payload).expect("E-AC-3 BSI");
    let expected_duration = info.numblks as u32 * 256;

    let media = demux(&data);
    let track = media
        .tracks
        .iter()
        .find(|t| matches!(t.spec.config, CodecConfig::Eac3 { .. }))
        .expect("E-AC-3 track");

    assert!(
        track.samples.len() >= pes.len(),
        "at least one access unit per PES access unit"
    );
    assert_pes_boundary_timing(&track.samples, &pes);
    for s in &track.samples {
        assert_eq!(
            s.duration, expected_duration,
            "every E-AC-3 sample duration must be numblks * 256"
        );
    }
}

// ── #556: AAC exact PES-boundary PTS + video source_timing ──────────────────

const H264_AAC_VIDEO_PID: u16 = 0x0100;
const H264_AAC_AUDIO_PID: u16 = 0x0101;

#[test]
fn aac_exact_pes_boundary_pts_and_video_source_timing() {
    let data = read_fixture("h264_aac.ts");
    let video_pes = collect_pes(&data, H264_AAC_VIDEO_PID);
    let audio_pes = collect_pes(&data, H264_AAC_AUDIO_PID);
    assert!(!video_pes.is_empty());
    assert!(!audio_pes.is_empty());

    let media = demux(&data);
    let video_track = media
        .tracks
        .iter()
        .find(|t| matches!(t.spec.config, CodecConfig::Avc { .. }))
        .expect("AVC video track");
    let audio_track = media
        .tracks
        .iter()
        .find(|t| matches!(t.spec.config, CodecConfig::Aac { .. }))
        .expect("AAC audio track");

    // Video: every sample now carries source_timing, and the FIRST video
    // sample's pts/dts match the first video PES exactly.
    assert!(
        video_track
            .samples
            .iter()
            .all(|s| s.source_timing.is_some()),
        "every H.264 sample must carry source_timing (issue #556)"
    );
    let first_video_pes = &video_pes[0];
    let expected_pts = first_video_pes.pts.expect("first video PES has a PTS");
    let expected_dts = first_video_pes.dts.unwrap_or(expected_pts);
    // The very first access unit in decode order should be the very first
    // access unit in wire order for this fixture (no B-frame reordering at
    // the stream head), so comparing the track's first sample is valid.
    let first_sample_ts = video_track.samples[0]
        .source_timing
        .expect("first video sample has source_timing");
    assert_eq!(first_sample_ts.pts, expected_pts);
    assert_eq!(first_sample_ts.dts, expected_dts);

    // AAC: exact PES-boundary PTS. Unlike AC-3/E-AC-3 (which keep the raw
    // syncframe bytes verbatim), each AAC `Sample` strips the 7-byte ADTS
    // header, so byte-length correlation doesn't apply here — instead,
    // independently count ADTS frames per PES access unit (a from-scratch
    // ADTS frame-length walk) to locate each PES's first decoded sample.
    let mut pes_boundary_indices = Vec::with_capacity(audio_pes.len());
    let mut idx = 0usize;
    for pes_au in &audio_pes {
        pes_boundary_indices.push(idx);
        idx += count_adts_frames(&pes_au.payload);
    }
    assert_eq!(
        idx,
        audio_track.samples.len(),
        "total decoded AAC samples must match an independent ADTS frame count"
    );
    for (pes_au, &boundary_idx) in audio_pes.iter().zip(pes_boundary_indices.iter()) {
        let st = audio_track.samples[boundary_idx]
            .source_timing
            .expect("a PES-boundary AAC sample must carry source_timing");
        if let Some(pts) = pes_au.pts {
            assert_eq!(
                st.pts, pts,
                "PES-boundary AAC sample PTS must equal the PES PTS exactly"
            );
        }
    }
}

/// Independent ADTS frame-length walk (ISO/IEC 13818-7 Annex A / 14496-3
/// Annex 1), used only to locate PES-boundary sample indices for the AAC
/// timing assertion above — written from scratch, not calling into
/// `transmux`'s own ADTS parser.
fn count_adts_frames(payload: &[u8]) -> usize {
    let mut n = 0usize;
    let mut off = 0usize;
    while off + 7 <= payload.len() {
        if payload[off] != 0xFF || (payload[off + 1] & 0xF0) != 0xF0 {
            break;
        }
        let frame_len = (((payload[off + 3] & 0x03) as usize) << 11)
            | ((payload[off + 4] as usize) << 3)
            | ((payload[off + 5] as usize) >> 5);
        if frame_len < 7 || off + frame_len > payload.len() {
            break;
        }
        n += 1;
        off += frame_len;
    }
    n
}

// ── #557: opaque PES data track (real DVB mux, m6-single.ts) ────────────────

const M6_PMT_PID: u16 = 0x0064;
const STREAM_TYPE_PES_PRIVATE: u8 = 0x06;
/// `descriptor_tag` for the DVB subtitling_descriptor (ETSI EN 300 468 §6.2.41).
const SUBTITLING_DESCRIPTOR_TAG: u8 = 0x59;

/// Independently reassemble & parse every PMT section on `pmt_pid`, returning
/// `(stream_type, elementary_pid, ES_info descriptor bytes)` for every ES
/// loop entry across every distinct section seen. Written from scratch here
/// (ISO/IEC 13818-1 §2.4.4.8) rather than reusing `TsDemux`'s private parser.
fn collect_pmt_es(data: &[u8], pmt_pid: u16) -> Vec<(u8, u16, Vec<u8>)> {
    const TABLE_ID_PMT: u8 = 0x02;
    let mut reasm = SectionReassembler::default();
    let mut out = Vec::new();
    for chunk in data.chunks_exact(188) {
        let Ok(pkt) = TsPacket::parse(chunk) else {
            continue;
        };
        if pkt.header.pid != pmt_pid {
            continue;
        }
        let Some(payload) = pkt.payload else {
            continue;
        };
        reasm.feed(payload, pkt.header.pusi);
        while let Some(section) = reasm.pop_section() {
            if section.first().copied() != Some(TABLE_ID_PMT) {
                continue;
            }
            if section.len() < 12 {
                continue;
            }
            let section_length = (((section[1] & 0x0F) as usize) << 8) | section[2] as usize;
            let end = (3 + section_length).min(section.len());
            if end < 12 {
                continue;
            }
            let body = &section[8..end - 4];
            if body.len() < 4 {
                continue;
            }
            let program_info_length = (((body[2] & 0x0F) as usize) << 8) | body[3] as usize;
            let mut off = 4 + program_info_length;
            while off + 5 <= body.len() {
                let stream_type = body[off];
                let pid = (((body[off + 1] & 0x1F) as u16) << 8) | body[off + 2] as u16;
                let es_info_length =
                    (((body[off + 3] & 0x0F) as usize) << 8) | body[off + 4] as usize;
                let ds = off + 5;
                let de = (ds + es_info_length).min(body.len());
                out.push((stream_type, pid, body[ds..de].to_vec()));
                off += 5 + es_info_length;
            }
        }
    }
    out
}

/// Whether a descriptor with `tag` appears anywhere in a raw ES_info
/// descriptor loop (`tag(1) + length(1) + data(length)`, repeated).
fn descriptor_loop_has_tag(descriptors: &[u8], tag: u8) -> bool {
    let mut off = 0usize;
    while off + 2 <= descriptors.len() {
        let this_tag = descriptors[off];
        let len = descriptors[off + 1] as usize;
        if this_tag == tag {
            return true;
        }
        off += 2 + len;
    }
    false
}

#[test]
fn data_track_subtitle_pes_passthrough() {
    let data = read_fixture("m6-single.ts");

    let media = demux(&data);
    assert!(
        media
            .tracks
            .iter()
            .any(|t| matches!(t.spec.config, CodecConfig::Data { stream_type, .. } if stream_type == STREAM_TYPE_PES_PRIVATE)),
        "at least one CodecConfig::Data track with stream_type 0x06 must exist"
    );

    // m6-single.ts's PMT is re-sent with different content over the capture
    // (subtitle PIDs get renumbered / gain a STREAM_IDENTIFIER descriptor
    // partway through); not every PMT-listed PID has actual PES payload
    // within this short excerpt. Pick, independently of the demuxer, a
    // stream_type 0x06 PID that (a) carries a DVB subtitling_descriptor
    // anywhere in its ES_info descriptor loop and (b) actually has PES data
    // in this fixture.
    let pmt_es = collect_pmt_es(&data, M6_PMT_PID);
    let (target_pid, descriptors) = pmt_es
        .iter()
        .filter(|(stream_type, _, descriptors)| {
            *stream_type == STREAM_TYPE_PES_PRIVATE
                && descriptor_loop_has_tag(descriptors, SUBTITLING_DESCRIPTOR_TAG)
        })
        .find_map(|(_, pid, descriptors)| {
            let pes = collect_pes(&data, *pid);
            (!pes.is_empty()).then(|| (*pid, descriptors.clone()))
        })
        .expect("at least one DVB subtitle PID with real PES data in this fixture");

    let data_track = media
        .tracks
        .iter()
        .find(|t| match &t.spec.config {
            CodecConfig::Data { descriptors: d, .. } => d.as_slice() == descriptors.as_slice(),
            _ => false,
        })
        .unwrap_or_else(|| {
            panic!("a Data track with PID {target_pid:#06x}'s exact descriptor bytes must exist")
        });
    match &data_track.spec.config {
        CodecConfig::Data { stream_type, .. } => assert_eq!(*stream_type, STREAM_TYPE_PES_PRIVATE),
        _ => unreachable!(),
    }

    let pes = collect_pes(&data, target_pid);
    assert!(!pes.is_empty(), "the chosen PID must carry PES packets");
    assert_eq!(
        data_track.samples.len(),
        pes.len(),
        "one Data sample per PES access unit, no splitting"
    );

    let concat_samples: Vec<u8> = data_track
        .samples
        .iter()
        .flat_map(|s| s.data.clone())
        .collect();
    let concat_pes: Vec<u8> = pes.iter().flat_map(|p| p.payload.clone()).collect();
    assert_eq!(
        concat_samples, concat_pes,
        "Data samples must be byte-identical to the PES payloads"
    );

    for (sample, pes_au) in data_track.samples.iter().zip(pes.iter()) {
        let st = sample
            .source_timing
            .expect("every Data sample must carry source_timing");
        if let Some(pts) = pes_au.pts {
            assert_eq!(st.pts, pts, "Data sample PTS must equal its PES PTS");
        }
    }
}

// ── #557: PCR timeline ───────────────────────────────────────────────────────

/// Independently collect every PCR observation from `data`'s TS adaptation
/// fields (ISO/IEC 13818-1 §2.4.3.4/§2.4.3.5), in packet order.
fn collect_pcr_oracle(data: &[u8]) -> Vec<PcrSample> {
    let mut out = Vec::new();
    for (idx, chunk) in data.chunks_exact(188).enumerate() {
        let Ok(pkt) = TsPacket::parse(chunk) else {
            continue;
        };
        let Some(Ok(af)) = pkt.adaptation_field() else {
            continue;
        };
        let Some(pcr) = af.pcr else {
            continue;
        };
        out.push(PcrSample {
            pcr_27mhz: pcr.as_27mhz(),
            pid: pkt.header.pid,
            packet_index: idx as u64,
            discontinuity: af.discontinuity_indicator,
        });
    }
    out
}

/// 27 MHz ticks in one second — the threshold above which a same-PID PCR
/// delta is treated as a discontinuity rather than ordinary jitter.
const ONE_SECOND_27MHZ: i128 = 27_000_000;

/// Find the first same-PID consecutive pair whose PCR delta is negative
/// (backwards) or exceeds one second, returning `(index_before, index_after)`
/// into `samples`.
fn find_discontinuity(samples: &[PcrSample]) -> Option<(usize, usize)> {
    let mut last: HashMap<u16, (usize, u64)> = HashMap::new();
    for (i, s) in samples.iter().enumerate() {
        if let Some(&(prev_idx, prev_val)) = last.get(&s.pid) {
            let delta = s.pcr_27mhz as i128 - prev_val as i128;
            if !(0..=ONE_SECOND_27MHZ).contains(&delta) {
                return Some((prev_idx, i));
            }
        }
        last.insert(s.pid, (i, s.pcr_27mhz));
    }
    None
}

#[test]
fn pcr_timeline_france_discontinuity() {
    let data = read_fixture("france-pcr-discontinuity.ts");
    let oracle = collect_pcr_oracle(&data);
    assert!(!oracle.is_empty(), "fixture must carry PCR");

    let media = demux(&data);
    assert!(!media.pcr.is_empty());
    assert_eq!(
        media.pcr, oracle,
        "TsDemux's PCR timeline must match an independent adaptation-field walk exactly"
    );

    let (oracle_before, oracle_after) =
        find_discontinuity(&oracle).expect("fixture must contain a PCR discontinuity");
    let (lib_before, lib_after) =
        find_discontinuity(&media.pcr).expect("library PCR must show the same discontinuity");
    assert_eq!(
        oracle_before, lib_before,
        "discontinuity position must be stable between the oracle and the library"
    );
    assert_eq!(
        oracle_after, lib_after,
        "discontinuity position must be stable between the oracle and the library"
    );
    assert!(
        oracle[oracle_after].discontinuity,
        "the anomalous PCR sample should carry discontinuity_indicator=true"
    );
}

#[test]
fn pcr_timeline_clean_stream_monotonic() {
    // h264_aac.ts carries real PCR (39 observations on PID 0x100, verified
    // independently); a clean stream's PCR timeline must be non-empty,
    // match the oracle, and be monotonic (no discontinuities).
    let data = read_fixture("h264_aac.ts");
    let media = demux(&data);
    let oracle = collect_pcr_oracle(&data);
    assert!(!oracle.is_empty(), "h264_aac.ts must carry PCR");
    assert_eq!(media.pcr, oracle);
    assert!(
        find_discontinuity(&media.pcr).is_none(),
        "h264_aac.ts PCR must be monotonic throughout"
    );

    // m6-single.ts is PID-filtered such that no adaptation field carries PCR
    // (verified independently): the timeline must then be empty, not invented.
    let m6 = read_fixture("m6-single.ts");
    let m6_media = demux(&m6);
    assert!(
        m6_media.pcr.is_empty(),
        "m6-single.ts carries no PCR; none may be invented"
    );
}

// ── #556/#557: CMAF mux interaction ──────────────────────────────────────────

#[test]
fn cmaf_mux_ac3_durations_no_longer_zero() {
    let ts = read_fixture("dolby/ac3.ts");
    let media = demux(&ts);
    let fmp4 = CmafMux::default()
        .package(&media)
        .expect("package TS-sourced AC-3 to CMAF");

    let round: Media = Fmp4Demux::new()
        .unpackage(&fmp4)
        .expect("re-parse our own CMAF output");
    let ac3_track = round
        .tracks
        .iter()
        .find(|t| matches!(t.spec.config, CodecConfig::Ac3 { .. }))
        .expect("AC-3 track survives the fMP4 round trip");
    assert!(!ac3_track.samples.is_empty());
    for s in &ac3_track.samples {
        assert_ne!(
            s.duration, 0,
            "AC-3 trun durations must be non-zero after issue #556 (pre-#556 they were all 0)"
        );
    }
}

#[test]
fn data_track_excluded_from_cmaf_like_vp8_vorbis() {
    // Synthetic media: a lone CodecConfig::Data track vs. the pre-existing
    // CodecConfig::Vp8 precedent — both must fail identically.
    let data_media = Media::new(
        vec![Track::new(
            TrackSpec {
                track_id: 1,
                timescale: 90_000,
                config: CodecConfig::Data {
                    stream_type: 0x06,
                    descriptors: vec![],
                },
            },
            vec![Sample::from_raw(vec![1, 2, 3], 0)],
        )],
        90_000,
    );
    let vp8_media = Media::new(
        vec![Track::new(
            TrackSpec {
                track_id: 1,
                timescale: 90_000,
                config: CodecConfig::Vp8 {
                    width: 640,
                    height: 480,
                },
            },
            vec![Sample::from_raw(vec![1, 2, 3], 0)],
        )],
        90_000,
    );

    let data_err = CmafMux::default().package(&data_media).unwrap_err();
    let vp8_err = CmafMux::default().package(&vp8_media).unwrap_err();
    match (data_err, vp8_err) {
        (Error::UnsupportedCodec { codec: dc }, Error::UnsupportedCodec { codec: vc }) => {
            assert_eq!(dc, "Data");
            assert_eq!(vc, "VP8");
        }
        other => panic!("expected UnsupportedCodec for both Data and Vp8, got {other:?}"),
    }

    // And confirm the same behaviour on a real demuxed Data track.
    let ts = read_fixture("m6-single.ts");
    let media = demux(&ts);
    assert!(
        media
            .tracks
            .iter()
            .any(|t| matches!(t.spec.config, CodecConfig::Data { .. })),
        "m6-single.ts must produce at least one Data track"
    );
    let err = CmafMux::default().package(&media).unwrap_err();
    assert!(
        matches!(err, Error::UnsupportedCodec { codec: "Data" }),
        "packaging a Media with a Data track must fail exactly like Vp8/Vorbis, got {err:?}"
    );
}

// ── SourceTiming::with_source_timing sanity ─────────────────────────────────

#[test]
fn source_timing_builder_round_trips() {
    let s = Sample::from_raw(vec![0u8; 4], 100).with_source_timing(SourceTiming {
        pts: 12345,
        dts: 12000,
    });
    let st = s
        .source_timing
        .expect("with_source_timing must set the field");
    assert_eq!(st.pts, 12345);
    assert_eq!(st.dts, 12000);
}
