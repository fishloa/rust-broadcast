//! `TsMux`/`TsHlsPackager` HEVC **carriage** gate (issues #627/#628).
//!
//! #627: `ts_mux::EsKind::from_config` only mapped AVC/AAC/AC-3/E-AC-3/DTS/
//! MPEG-H/Data — every other codec (HEVC included) fell through to `None` and
//! was silently skipped by `plan_elementary_streams`, so a HEVC track never
//! appeared in TS/TS-HLS output at all. #628: `choose_anchor`/`Segmenter::new`
//! only recognised `CodecConfig::Avc` as video, so an HEVC-only program picked
//! the wrong (or a nonexistent) anchor track for segment cuts.
//!
//! Test 1 proves #627 end to end on a real capture: `TsDemux(hevc/main.ts)` →
//! `TsMux` → PMT must declare `stream_type = 0x24`, and re-demuxing the muxed
//! bytes must recover the same HEVC track (dims + VPS/SPS/PPS). This cannot be
//! faked by a raw passthrough — the intermediate bytes are real TS packets
//! independently re-parsed by `TsDemux`.
//!
//! Test 2 proves the HEVC access-unit parameter-set-prepend path (the HEVC
//! sibling of `build_annexb_au`'s AVC guarantee): a synthetic HEVC track whose
//! second keyframe access unit does **not** carry its own VPS/SPS/PPS is
//! segmented by `TsHlsPackager` so that keyframe opens segment 2; re-demuxing
//! that segment alone must show the parameter sets inserted, in AU order,
//! before the first slice NAL.
//!
//! Test 3 proves #628: a two-track program with audio at track index 0 and
//! HEVC video at index 1 must pick the HEVC track as the segmentation anchor
//! — observed through the number of segments `TsHlsPackager` actually cuts
//! (audio's own keyframe cadence would produce a different count than
//! HEVC's).

use std::path::PathBuf;

use broadcast_common::{Package, Unpackage};
use transmux::pipeline::CodecConfig;
use transmux::{
    Ac3SpecificBox, HEVCConfigurationBox, HEVCDecoderConfigurationRecord, Media, Sample, Track,
    TrackSpec, TsDemux, TsHlsPackager, TsMux,
};

// ── Fixture loading (mirrors tests/ts_hevc.rs) ──────────────────────────────

fn fixtures_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../fixtures/ts/hevc")
}

fn load_ts(name: &str) -> Vec<u8> {
    let path = fixtures_dir().join(name);
    let data = std::fs::read(&path).unwrap_or_else(|_| panic!("{name} fixture must exist"));
    assert_eq!(
        data.len() % 188,
        0,
        "TS file must be whole 188-byte packets"
    );
    data
}

fn hevc_track(media: &Media) -> &Track {
    let hevc: Vec<&Track> = media
        .tracks
        .iter()
        .filter(|t| matches!(t.spec.config, CodecConfig::Hevc { .. }))
        .collect();
    assert_eq!(hevc.len(), 1, "must carry exactly one HEVC video track");
    hevc[0]
}

/// HEVC `nal_unit_type` from a 2-byte NAL header: `(byte0 >> 1) & 0x3F`
/// (ITU-T H.265 Table 7-1).
fn hevc_nal_type(nal: &[u8]) -> Option<u8> {
    nal.first().map(|b| (b >> 1) & 0x3F)
}

/// Split 4-byte length-prefixed NAL data into its coded NAL payloads.
fn split_lp(lp: &[u8]) -> Vec<Vec<u8>> {
    let mut out = Vec::new();
    let mut off = 0;
    while off + 4 <= lp.len() {
        let n = u32::from_be_bytes([lp[off], lp[off + 1], lp[off + 2], lp[off + 3]]) as usize;
        off += 4;
        if off + n > lp.len() {
            break;
        }
        out.push(lp[off..off + n].to_vec());
        off += n;
    }
    out
}

// ── Minimal TS/PSI walking (byte-level, mirrors tests/ts_mux.rs) ────────────

const TS: usize = 188;

fn pid_of(pkt: &[u8]) -> u16 {
    (((pkt[1] & 0x1F) as u16) << 8) | pkt[2] as u16
}

fn pusi_of(pkt: &[u8]) -> bool {
    pkt[1] & 0x40 != 0
}

fn payload_offset(pkt: &[u8]) -> usize {
    let afc = (pkt[3] >> 4) & 0x3;
    let has_af = afc & 0b10 != 0;
    let has_payload = afc & 0b01 != 0;
    if !has_payload {
        return TS;
    }
    if has_af { 4 + 1 + pkt[4] as usize } else { 4 }
}

fn first_section(ts: &[u8], pid: u16) -> Option<Vec<u8>> {
    for pkt in ts.chunks_exact(TS) {
        if pid_of(pkt) != pid || !pusi_of(pkt) {
            continue;
        }
        let off = payload_offset(pkt);
        if off >= TS {
            continue;
        }
        let payload = &pkt[off..];
        let ptr = payload[0] as usize;
        let sec_start = 1 + ptr;
        if sec_start + 3 > payload.len() {
            continue;
        }
        let sec = &payload[sec_start..];
        let section_length = (((sec[1] & 0x0F) as usize) << 8) | sec[2] as usize;
        let total = 3 + section_length;
        if total > sec.len() {
            continue;
        }
        return Some(sec[..total].to_vec());
    }
    None
}

/// PAT → list of (program_number, program_map_PID).
fn parse_pat(sec: &[u8]) -> Vec<(u16, u16)> {
    let body = &sec[8..sec.len() - 4];
    let mut out = Vec::new();
    let mut i = 0;
    while i + 4 <= body.len() {
        let prog = u16::from_be_bytes([body[i], body[i + 1]]);
        let pmt_pid = (((body[i + 2] & 0x1F) as u16) << 8) | body[i + 3] as u16;
        out.push((prog, pmt_pid));
        i += 4;
    }
    out
}

/// PMT → list of (stream_type, elementary_PID).
fn parse_pmt(sec: &[u8]) -> Vec<(u8, u16)> {
    let body = &sec[8..sec.len() - 4];
    let program_info_length = (((body[2] & 0x0F) as usize) << 8) | body[3] as usize;
    let mut i = 4 + program_info_length;
    let mut out = Vec::new();
    while i + 5 <= body.len() {
        let stream_type = body[i];
        let es_pid = (((body[i + 1] & 0x1F) as u16) << 8) | body[i + 2] as u16;
        let es_info_len = (((body[i + 3] & 0x0F) as usize) << 8) | body[i + 4] as usize;
        out.push((stream_type, es_pid));
        i += 5 + es_info_len;
    }
    out
}

/// Every VPS(32)/SPS(33)/PPS(34) NAL byte payload carried in an `hvcC` record,
/// as `(nal_unit_type, bytes)` pairs, array order then in-array order (the
/// same order `ts_mux::hevc_parameter_sets` emits them in).
fn parameter_sets(record: &HEVCDecoderConfigurationRecord) -> Vec<(u8, Vec<u8>)> {
    let mut out = Vec::new();
    for arr in &record.arrays {
        if matches!(arr.nal_unit_type, 32..=34) {
            for nalu in &arr.nalus {
                out.push((arr.nal_unit_type, nalu.0.clone()));
            }
        }
    }
    out
}

// ── Test 1 — real fixture: HEVC survives a TS→TS round-trip (issue #627) ────

#[test]
fn hevc_track_survives_ts_mux_round_trip() {
    let ts = load_ts("main.ts");
    let ir = TsDemux::new().unpackage(&ts).expect("demux fixture");
    let before = hevc_track(&ir);
    assert!(
        !before.samples.is_empty(),
        "fixture must carry HEVC samples"
    );

    let ts2 = TsMux::new().package(&ir).expect("mux IR back to TS");
    assert_eq!(ts2.len() % TS, 0, "output must be whole 188-byte packets");
    assert!(!ts2.is_empty(), "output must not be empty");

    // PMT must declare stream_type 0x24 (HEVC — ISO/IEC 13818-1 Table 2-34)
    // for the HEVC PID. Before the #627 fix this stream is dropped entirely
    // and this section either has no such entry or mux_tracks errors outright.
    let pat = first_section(&ts2, 0x0000).expect("PAT must be present on PID 0");
    let programs = parse_pat(&pat);
    let pmt_pid = programs[0].1;
    let pmt = first_section(&ts2, pmt_pid).expect("PMT must resolve from PAT");
    let streams = parse_pmt(&pmt);
    let types: Vec<u8> = streams.iter().map(|s| s.0).collect();
    assert!(
        types.contains(&0x24),
        "PMT must carry HEVC stream_type 0x24, got {types:02X?}"
    );

    // Re-demux the muxed bytes: the HEVC track must come back with the same
    // sample count and byte-identical VPS/SPS/PPS.
    let ir2 = TsDemux::new().unpackage(&ts2).expect("re-demux mux output");
    let after = hevc_track(&ir2);
    assert!(!after.samples.is_empty(), "HEVC samples must survive");
    assert_eq!(
        after.samples.len(),
        before.samples.len(),
        "HEVC sample count must be preserved"
    );

    let (record_before, w0, h0) = match &before.spec.config {
        CodecConfig::Hevc {
            config,
            width,
            height,
        } => (&config.config, *width, *height),
        other => panic!("expected HEVC, got {other:?}"),
    };
    let (record_after, w1, h1) = match &after.spec.config {
        CodecConfig::Hevc {
            config,
            width,
            height,
        } => (&config.config, *width, *height),
        other => panic!("expected HEVC, got {other:?}"),
    };
    assert_eq!((w0, h0), (w1, h1), "dimensions must be preserved");
    assert_eq!(
        parameter_sets(record_before),
        parameter_sets(record_after),
        "VPS/SPS/PPS must round-trip byte-identically through the TS mux"
    );

    // Sample NAL payloads themselves must also survive byte-identically.
    for (i, (a, b)) in before.samples.iter().zip(&after.samples).enumerate() {
        assert_eq!(
            split_lp(&a.data),
            split_lp(&b.data),
            "HEVC sample {i}: coded NAL payloads must be byte-identical"
        );
    }
}

// ── Test 2 — synthetic timing / real NAL content: HEVC keyframe AU missing ──
// ── VPS/SPS/PPS gets them prepended at a mid-stream segment cut (issue ──────
// ── #627's AU-framing half) ─────────────────────────────────────────────────

fn lp_nal(nal: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(4 + nal.len());
    out.extend_from_slice(&(nal.len() as u32).to_be_bytes());
    out.extend_from_slice(nal);
    out
}

const NAL_AUD: u8 = 35;
const NAL_VPS: u8 = 32;
const NAL_SPS: u8 = 33;
const NAL_PPS: u8 = 34;

/// The real `hvcC` config plus the real AUD/VPS/SPS/PPS/slice NAL bytes from
/// `hevc/main.ts`'s only IRAP access unit — used to build synthetic samples
/// with genuinely decodable parameter sets (a fabricated SPS would not decode,
/// so `TsDemux` could never resolve a `CodecConfig::Hevc` from it; see
/// `hevc_config.rs`/`sps.rs` decode-or-keep-probing behaviour).
struct RealHevcParts {
    config: HEVCConfigurationBox,
    width: u16,
    height: u16,
    aud: Vec<u8>,
    vps: Vec<u8>,
    sps: Vec<u8>,
    pps: Vec<u8>,
    slice: Vec<u8>,
}

fn real_hevc_parts() -> RealHevcParts {
    let ts = load_ts("main.ts");
    let ir = TsDemux::new().unpackage(&ts).expect("demux fixture");
    let track = hevc_track(&ir);
    let (config, width, height) = match &track.spec.config {
        CodecConfig::Hevc {
            config,
            width,
            height,
        } => (config.clone(), *width, *height),
        other => panic!("expected HEVC, got {other:?}"),
    };
    let first = &track.samples[0];
    assert!(first.is_sync, "fixture's first AU must be the IRAP");
    let (mut aud, mut vps, mut sps, mut pps, mut slice) = (None, None, None, None, None);
    for nal in split_lp(&first.data) {
        match hevc_nal_type(&nal) {
            Some(NAL_AUD) if aud.is_none() => aud = Some(nal),
            Some(NAL_VPS) if vps.is_none() => vps = Some(nal),
            Some(NAL_SPS) if sps.is_none() => sps = Some(nal),
            Some(NAL_PPS) if pps.is_none() => pps = Some(nal),
            Some(t) if (16..=23).contains(&t) && slice.is_none() => slice = Some(nal),
            _ => {}
        }
    }
    RealHevcParts {
        config,
        width,
        height,
        aud: aud.expect("fixture AU must carry an AUD"),
        vps: vps.expect("fixture AU must carry a VPS"),
        sps: sps.expect("fixture AU must carry an SPS"),
        pps: pps.expect("fixture AU must carry a PPS"),
        slice: slice.expect("fixture AU must carry an IRAP slice"),
    }
}

#[test]
fn mid_stream_segment_prepends_missing_hevc_parameter_sets() {
    let parts = real_hevc_parts();

    let spec = TrackSpec::new(
        1,
        90_000,
        CodecConfig::Hevc {
            config: parts.config.clone(),
            width: parts.width,
            height: parts.height,
        },
    );

    // Sample 0: full IRAP AU carrying its own VPS/SPS/PPS (first GOP, as a
    // real encoder emits it) — one second at 90 kHz.
    let sample0_data: Vec<u8> = [&parts.aud, &parts.vps, &parts.sps, &parts.pps, &parts.slice]
        .into_iter()
        .flat_map(|n| lp_nal(n))
        .collect();
    let sample0 = Sample::new(sample0_data, 90_000, true, 0);

    // Sample 1: a second IRAP AU *without* its own VPS/SPS/PPS — the case
    // `build_hevc_annexb_au` must repair so every emitted TS AU stays
    // independently decodable (mirrors the AVC guarantee).
    let sample1_data: Vec<u8> = [&parts.aud, &parts.slice]
        .into_iter()
        .flat_map(|n| lp_nal(n))
        .collect();
    let sample1 = Sample::new(sample1_data, 90_000, true, 0);

    let track = Track::new(spec, vec![sample0, sample1]);
    let media = Media::new(vec![track], 90_000);

    // 1-second target at 90 kHz: sample 0 alone reaches the target, so sample
    // 1 (an IRAP) opens segment 2.
    let out = TsHlsPackager::new(1)
        .package(&media)
        .expect("package TS-HLS");
    assert_eq!(
        out.segments.len(),
        2,
        "buffered duration + sample 1's keyframe must cut a second segment"
    );

    // Re-demux segment 2 alone (every TS-HLS segment is self-contained
    // PAT/PMT/PES) and inspect its first (only) HEVC access unit.
    let seg2: Media = TsDemux::new()
        .unpackage(&out.segments[1])
        .expect("re-demux segment 2");
    let seg2_track = hevc_track(&seg2);
    assert_eq!(
        seg2_track.samples.len(),
        1,
        "segment 2 must carry exactly the one HEVC AU"
    );
    let nals = split_lp(&seg2_track.samples[0].data);
    let types: Vec<Option<u8>> = nals.iter().map(|n| hevc_nal_type(n)).collect();

    let vps_pos = types
        .iter()
        .position(|t| *t == Some(NAL_VPS))
        .expect("segment 2's AU must carry a prepended VPS");
    let sps_pos = types
        .iter()
        .position(|t| *t == Some(NAL_SPS))
        .expect("segment 2's AU must carry a prepended SPS");
    let pps_pos = types
        .iter()
        .position(|t| *t == Some(NAL_PPS))
        .expect("segment 2's AU must carry a prepended PPS");
    let real_slice_type = hevc_nal_type(&parts.slice).expect("real slice NAL must be non-empty");
    let slice_pos = types
        .iter()
        .position(|t| *t == Some(real_slice_type))
        .expect("segment 2's AU must still carry its slice NAL");

    assert!(
        vps_pos < sps_pos && sps_pos < pps_pos && pps_pos < slice_pos,
        "parameter sets must precede the slice, in VPS/SPS/PPS AU order: {types:?}"
    );
}

// ── Test 3 — anchor selection: HEVC (not audio) is the segmentation anchor ──
// ── (issue #628) ─────────────────────────────────────────────────────────

fn dummy_ac3_config() -> Ac3SpecificBox {
    Ac3SpecificBox {
        fscod: 0, // 48 kHz
        bsid: 8,
        bsmod: 0,
        acmod: 2, // stereo
        lfeon: false,
        bit_rate_code: 0,
    }
}

#[test]
fn hevc_track_is_chosen_as_anchor_over_audio() {
    // Track 0: audio (AC-3), plenty of small always-sync frames — if this were
    // mistakenly chosen as the anchor (pre-#628 behaviour: `unwrap_or(0)`
    // defaults to track 0 since neither track matches `CodecConfig::Avc`), its
    // short total duration never reaches the 1-second target, so the whole
    // stream would land in a single segment.
    let audio_spec = TrackSpec::new(
        1,
        48_000,
        CodecConfig::Ac3 {
            config: dummy_ac3_config(),
            channel_count: 2,
            sample_rate: 48_000,
            sample_size: 16,
        },
    );
    let audio_samples: Vec<Sample> = (0..20)
        .map(|_| Sample::new(vec![0xAAu8; 8], 1_024, true, 0))
        .collect();
    let audio_track = Track::new(audio_spec, audio_samples);

    // Track 1: HEVC, 4 one-second (@ 90 kHz) samples, IRAP at indices 0 and 2.
    // Correctly anchored on this track, a 1-second target must cut exactly 2
    // segments (buffered reaches the target at sample 1, sample 2 is the next
    // IRAP → cut before it).
    let parts = real_hevc_parts();
    let full_au: Vec<u8> = [&parts.vps, &parts.sps, &parts.pps, &parts.slice]
        .into_iter()
        .flat_map(|n| lp_nal(n))
        .collect();
    let video_spec = TrackSpec::new(
        2,
        90_000,
        CodecConfig::Hevc {
            config: parts.config,
            width: parts.width,
            height: parts.height,
        },
    );
    let video_samples = vec![
        Sample::new(full_au.clone(), 90_000, true, 0),
        Sample::new(full_au.clone(), 90_000, false, 0),
        Sample::new(full_au.clone(), 90_000, true, 0),
        Sample::new(full_au, 90_000, false, 0),
    ];
    let video_track = Track::new(video_spec, video_samples);

    let media = Media::new(vec![audio_track, video_track], 90_000);
    let out = TsHlsPackager::new(1)
        .package(&media)
        .expect("package TS-HLS");

    assert_eq!(
        out.segments.len(),
        2,
        "anchoring on the HEVC track's keyframes must cut 2 segments; \
         anchoring on audio (pre-#628 bug) would cut only 1 (audio never \
         reaches the 1s target across its 20 short frames)"
    );
}
