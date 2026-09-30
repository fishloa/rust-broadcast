//! `TsMux` gate — hub `Media` IR → MPEG-2 TS, verified by a full round-trip
//! through the independent, ffmpeg-oracle-gated `TsDemux` (issue #460).
//!
//! Oracle: `TsDemux` is byte-oracle-gated against ffmpeg in `tests/ts_demux.rs`,
//! so a `TsDemux(TsMux(TsDemux(fixture)))` round-trip that preserves tracks,
//! codec configs, coded NAL payloads, frame counts, and per-sample timing proves
//! the mux is a faithful inverse — none of it can be faked (a raw-passthrough
//! serialize would not parse back as valid TS).
//!
//! Pipeline: `ir = TsDemux(h264_aac.ts)` → `ts2 = TsMux(ir)` →
//! `ir2 = TsDemux(ts2)`.

use broadcast_common::{Package, Serialize, Unpackage};
use transmux::media::{CmafMux, Media};
use transmux::pipeline::CodecConfig;
use transmux::{TsDemux, TsMux};

// ── Fixture + pipeline ───────────────────────────────────────────────────────

fn load_ts() -> Vec<u8> {
    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../fixtures/ts/h264_aac.ts");
    std::fs::read(path).expect("h264_aac.ts fixture must exist")
}

/// `ir` = the fixture demuxed; `ir2` = re-demux of `TsMux(ir)`; `ts2` = the mux
/// output bytes.
fn pipeline() -> (Media, Vec<u8>, Media) {
    let ts = load_ts();
    let ir = TsDemux::new().unpackage(&ts).expect("demux fixture");
    let ts2 = TsMux::new().package(&ir).expect("mux IR back to TS");
    let ir2 = TsDemux::new().unpackage(&ts2).expect("re-demux mux output");
    (ir, ts2, ir2)
}

// ── Minimal TS packet + PSI walking (byte-level, no crate internals) ──────────

const TS: usize = 188;

/// Read the 13-bit PID from a 188-byte packet.
fn pid_of(pkt: &[u8]) -> u16 {
    (((pkt[1] & 0x1F) as u16) << 8) | pkt[2] as u16
}

/// Payload-unit-start-indicator.
fn pusi_of(pkt: &[u8]) -> bool {
    pkt[1] & 0x40 != 0
}

/// Payload offset in a packet (skips the 4-byte header + any adaptation field).
fn payload_offset(pkt: &[u8]) -> usize {
    let afc = (pkt[3] >> 4) & 0x3;
    let has_af = afc & 0b10 != 0;
    let has_payload = afc & 0b01 != 0;
    if !has_payload {
        return TS; // no payload
    }
    if has_af { 4 + 1 + pkt[4] as usize } else { 4 }
}

/// Reassemble the first complete PSI section carried on `pid` (single-packet
/// sections, which is all this muxer emits). Returns the section bytes without
/// the pointer_field, trimmed to `section_length`.
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
        // First payload byte is the pointer_field; the section starts after it.
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

/// Parse a PAT section → list of (program_number, program_map_PID).
fn parse_pat(sec: &[u8]) -> Vec<(u16, u16)> {
    // header: table_id(1) + flags/len(2) + tsid(2) + ver(1) + secno(1) + last(1)
    let body = &sec[8..sec.len() - 4]; // strip 8-byte header + 4-byte CRC
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

/// Parse a PMT section → list of (stream_type, elementary_PID).
fn parse_pmt(sec: &[u8]) -> Vec<(u8, u16)> {
    let body = &sec[8..sec.len() - 4];
    // reserved/PCR_PID(2) + reserved/program_info_length(2) + program descriptors
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

/// Extract the demuxed avcC record body bytes for a track (serialized).
fn avcc_body(track: &transmux::media::Track) -> Vec<u8> {
    match &track.spec.config {
        CodecConfig::Avc { config, .. } => {
            let r = &config.config;
            let mut buf = vec![0u8; r.serialized_len()];
            r.serialize_into(&mut buf).unwrap();
            buf
        }
        other => panic!("expected AVC track, got {other:?}"),
    }
}

/// Split length-prefixed (4-byte) NAL data into its coded NAL payloads.
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

// ── Test 1 — well-formed TS: whole packets, PAT → PMT → 2 ES (0x1B, 0x0F) ─────

#[test]
fn output_is_well_formed_ts_with_pat_pmt_two_streams() {
    let (_ir, ts2, _ir2) = pipeline();

    assert_eq!(ts2.len() % TS, 0, "output must be whole 188-byte packets");
    assert!(!ts2.is_empty(), "output must not be empty");
    // Every packet parses as a TS packet (sync byte 0x47).
    for pkt in ts2.chunks_exact(TS) {
        assert_eq!(pkt[0], 0x47, "each packet must start with the TS sync byte");
    }

    // PAT on PID 0 resolves to a PMT PID.
    let pat = first_section(&ts2, 0x0000).expect("PAT must be present on PID 0");
    assert_eq!(pat[0], 0x00, "PAT table_id");
    let programs = parse_pat(&pat);
    assert_eq!(programs.len(), 1, "one program");
    let pmt_pid = programs[0].1;

    // PMT lists exactly the 2 elementary streams with the expected stream_types.
    let pmt = first_section(&ts2, pmt_pid).expect("PMT must resolve from PAT");
    assert_eq!(pmt[0], 0x02, "PMT table_id");
    let streams = parse_pmt(&pmt);
    assert_eq!(streams.len(), 2, "PMT must list 2 elementary streams");
    let types: Vec<u8> = streams.iter().map(|s| s.0).collect();
    assert!(types.contains(&0x1B), "must carry H.264 (stream_type 0x1B)");
    assert!(types.contains(&0x0F), "must carry AAC (stream_type 0x0F)");
    // Video is listed first (mirrors track order).
    assert_eq!(streams[0].0, 0x1B, "first ES is H.264");
    assert_eq!(streams[1].0, 0x0F, "second ES is AAC");
}

// ── Test 2 — track/codec preservation + avcC byte-identity ────────────────────

#[test]
fn tracks_and_avcc_preserved() {
    let (ir, _ts2, ir2) = pipeline();

    assert_eq!(ir2.tracks.len(), 2, "round-trip must recover 2 tracks");
    assert!(
        matches!(ir2.tracks[0].spec.config, CodecConfig::Avc { .. }),
        "track 0 must be AVC, got {:?}",
        ir2.tracks[0].spec.config
    );
    assert!(
        matches!(ir2.tracks[1].spec.config, CodecConfig::Aac { .. }),
        "track 1 must be AAC, got {:?}",
        ir2.tracks[1].spec.config
    );

    // The demuxed avcC from ir2 equals the avcC from ir, byte-identical.
    assert_eq!(
        avcc_body(&ir2.tracks[0]),
        avcc_body(&ir.tracks[0]),
        "round-tripped avcC must be byte-identical to the original"
    );
}

// ── Test 3 — sample fidelity: video NAL payloads + audio frames byte-identical ─

#[test]
fn sample_payloads_round_trip_byte_identical() {
    let (ir, _ts2, ir2) = pipeline();

    // Video: compare coded NAL payloads sample-for-sample.
    let v0 = &ir.tracks[0];
    let v2 = &ir2.tracks[0];
    assert_eq!(
        v2.samples.len(),
        v0.samples.len(),
        "video sample count preserved"
    );
    assert_eq!(v0.samples.len(), 75, "expected 75 video samples");
    for (i, (a, b)) in v0.samples.iter().zip(&v2.samples).enumerate() {
        let na = split_lp(&a.data);
        let nb = split_lp(&b.data);
        assert_eq!(
            na, nb,
            "video sample {i}: coded NAL payloads must be byte-identical"
        );
    }

    // Audio: frame count preserved (131) and each raw AAC sample byte-identical.
    let a0 = &ir.tracks[1];
    let a2 = &ir2.tracks[1];
    assert_eq!(a0.samples.len(), 131, "expected 131 audio frames");
    assert_eq!(
        a2.samples.len(),
        131,
        "audio frame count preserved through round-trip"
    );
    for (i, (a, b)) in a0.samples.iter().zip(&a2.samples).enumerate() {
        assert_eq!(
            a.data, b.data,
            "audio sample {i}: raw AAC frame bytes must be byte-identical"
        );
    }
}

// ── Test 4 — timing preserved: video DTS deltas + composition offsets ─────────

#[test]
fn timing_preserved_for_video() {
    let (ir, _ts2, ir2) = pipeline();

    let v0 = &ir.tracks[0];
    let v2 = &ir2.tracks[0];
    assert_eq!(v0.samples.len(), 75, "75 video samples");
    assert_eq!(v2.samples.len(), 75, "75 video samples after round-trip");

    // Per-sample DTS delta (== duration) and composition offset (PTS − DTS).
    for (i, (a, b)) in v0.samples.iter().zip(&v2.samples).enumerate() {
        assert_eq!(
            b.duration, a.duration,
            "video sample {i}: DTS delta (duration) must be preserved"
        );
        assert_eq!(
            b.composition_offset(),
            a.composition_offset(),
            "video sample {i}: composition offset (PTS − DTS) must be preserved"
        );
    }
}

// ── Test 5 — end-to-end: CMAF(ir2) video mdat NALs == CMAF(ir) ────────────────

#[test]
fn cmaf_from_round_tripped_ir_matches_cmaf_from_original() {
    let (ir, _ts2, ir2) = pipeline();

    let cmaf_orig = CmafMux::default().package(&ir).expect("CMAF from ir");
    let cmaf_round = CmafMux::default().package(&ir2).expect("CMAF from ir2");

    // Re-parse both CMAF outputs and compare the video track's length-prefixed
    // sample NAL payloads (the mdat coded data), sample-for-sample.
    let m_orig: Media = transmux::Fmp4Demux::new()
        .unpackage(&cmaf_orig)
        .expect("parse orig CMAF");
    let m_round: Media = transmux::Fmp4Demux::new()
        .unpackage(&cmaf_round)
        .expect("parse round CMAF");

    let vo = &m_orig.tracks[0];
    let vr = &m_round.tracks[0];
    assert_eq!(
        vo.samples.len(),
        vr.samples.len(),
        "same video sample count"
    );
    for (i, (a, b)) in vo.samples.iter().zip(&vr.samples).enumerate() {
        assert_eq!(
            split_lp(&a.data),
            split_lp(&b.data),
            "CMAF video sample {i} mdat NAL payloads must match (TS→IR→TS→IR→CMAF == TS→IR→CMAF)"
        );
    }
}

// ── Test 6 — AVC/HEVC access units get a leading AUD (H.222.0 §2.14.1/§2.17.1) ─

/// Reassemble the PES payload bytes of every packet on `pid` (strip the 4-byte
/// TS header + adaptation field), one entry per PUSI-delimited PES packet.
fn pes_payloads(ts: &[u8], pid: u16) -> Vec<Vec<u8>> {
    let mut out = Vec::new();
    let mut cur: Vec<u8> = Vec::new();
    for pkt in ts.chunks_exact(TS) {
        if pid_of(pkt) != pid {
            continue;
        }
        let off = payload_offset(pkt);
        if off >= TS {
            continue;
        }
        if pusi_of(pkt) && !cur.is_empty() {
            out.push(core::mem::take(&mut cur));
        }
        cur.extend_from_slice(&pkt[off..]);
    }
    if !cur.is_empty() {
        out.push(cur);
    }
    out
}

/// The elementary-stream payload of a PES packet (skips its optional header).
fn pes_es(pes: &[u8]) -> &[u8] {
    assert_eq!(&pes[..3], &[0, 0, 1], "PES start code prefix");
    let header_data_len = pes[8] as usize;
    &pes[9 + header_data_len..]
}

/// Annex B start-code length at the front of `au` (4, 3, or 0 when absent).
fn start_code_len(au: &[u8]) -> usize {
    if au.starts_with(&[0, 0, 0, 1]) {
        4
    } else if au.starts_with(&[0, 0, 1]) {
        3
    } else {
        0
    }
}

/// Demux an AUD-less MP4 into the IR, mux it to TS, and return the video PES
/// payloads plus the video track's PID from the PMT.
fn mux_noaud_fixture(fixture: &str) -> (Vec<Vec<u8>>, u16) {
    let path = alloc_path(fixture);
    let mp4 = std::fs::read(path).expect("fixture must exist");
    let ir = transmux::Fmp4Demux::new()
        .unpackage(&mp4)
        .expect("demux the AUD-less MP4");
    let ts = TsMux::new().package(&ir).expect("mux to TS");
    let pmt = first_section(&ts, 0x1000).expect("PMT present");
    let streams = parse_pmt(&pmt);
    let video_pid = streams[0].1;
    (pes_payloads(&ts, video_pid), video_pid)
}

fn alloc_path(fixture: &str) -> std::path::PathBuf {
    std::path::PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/../fixtures/mp4/")).join(fixture)
}

/// Every AVC access unit begins with the canonical `00 00 00 01 09 F0` AUD.
#[test]
fn avc_access_units_get_a_leading_aud() {
    let (pes, _) = mux_noaud_fixture("noaud/h264_noaud.mp4");
    assert_eq!(pes.len(), 15, "one PES per AVC sample");
    for (i, p) in pes.iter().enumerate() {
        let es = pes_es(p);
        assert_eq!(
            start_code_len(es),
            4,
            "sample {i}: AU must open with a 4-byte Annex B start code"
        );
        assert_eq!(
            es[4] & H264_NAL_TYPE_MASK_IN_TEST,
            9,
            "sample {i}: AU must begin with an AVC AUD (nal_unit_type 9) \
             (H.222.0 §2.14.1), got nal_unit_type {}",
            es[4] & H264_NAL_TYPE_MASK_IN_TEST
        );
        assert_eq!(
            es[5], 0xF0,
            "sample {i}: AUD primary_pic_type = 7 (H.264 Table 7-3)"
        );
        // The AUD must carry no payload beyond its 2 bytes: the next bytes are
        // the following NAL's start code.
        assert_eq!(
            &es[6..10],
            &[0, 0, 0, 1],
            "sample {i}: the following NAL must start right after the AUD"
        );
    }
}

/// Every HEVC access unit begins with the canonical `00 00 00 01 46 01 50` AUD.
#[test]
fn hevc_access_units_get_a_leading_aud() {
    let (pes, _) = mux_noaud_fixture("noaud/hevc_noaud.mp4");
    assert_eq!(pes.len(), 15, "one PES per HEVC sample");
    for (i, p) in pes.iter().enumerate() {
        let es = pes_es(p);
        assert_eq!(start_code_len(es), 4, "sample {i}: 4-byte start code");
        assert_eq!(
            (es[4] >> 1) & HEVC_NAL_TYPE_MASK_IN_TEST,
            35,
            "sample {i}: AU must begin with an HEVC AUD (AUD_NUT 35) \
             (H.222.0 §2.17.1), got nal_unit_type {}",
            (es[4] >> 1) & HEVC_NAL_TYPE_MASK_IN_TEST
        );
        assert_eq!(
            &es[5..7],
            &[0x01, 0x50],
            "sample {i}: AUD nuh_layer_id 0 / nuh_temporal_id_plus1 1 + pic_type 2 \
             (H.265 Table 7-4)"
        );
        assert_eq!(
            &es[7..11],
            &[0, 0, 0, 1],
            "sample {i}: next NAL right after AUD"
        );
    }
}

/// AUD insertion must not change the coded picture data: the NALs after the
/// inserted (or already-present) AUD are exactly the source's NALs.
#[test]
fn aud_insertion_preserves_coded_nals() {
    let path = alloc_path("noaud/h264_noaud.mp4");
    let mp4 = std::fs::read(path).unwrap();
    let ir = transmux::Fmp4Demux::new().unpackage(&mp4).unwrap();
    let ts = TsMux::new().package(&ir).unwrap();
    let pmt = first_section(&ts, 0x1000).unwrap();
    let streams = parse_pmt(&pmt);
    let pes = pes_payloads(&ts, streams[0].1);

    for (i, (p, s)) in pes.iter().zip(&ir.tracks[0].samples).enumerate() {
        let es = pes_es(p);
        let after_aud = &es[6..];
        // Re-frame the Annex B tail back into the NAL byte sequences.
        let mut reframed: Vec<Vec<u8>> = Vec::new();
        let mut pos = 0usize;
        while pos + 4 <= after_aud.len() {
            assert_eq!(
                &after_aud[pos..pos + 4],
                &[0, 0, 0, 1],
                "sample {i}: start code"
            );
            let rest = &after_aud[pos + 4..];
            let end = rest
                .windows(4)
                .position(|w| w == [0, 0, 0, 1])
                .unwrap_or(rest.len());
            reframed.push(rest[..end].to_vec());
            pos += 4 + end;
        }

        let mut src: Vec<Vec<u8>> = Vec::new();
        let mut off = 0usize;
        while off + 4 <= s.data.len() {
            let n = u32::from_be_bytes([
                s.data[off],
                s.data[off + 1],
                s.data[off + 2],
                s.data[off + 3],
            ]) as usize;
            off += 4;
            src.push(s.data[off..off + n].to_vec());
            off += n;
        }

        // The mux may prepend SPS/PPS to a keyframe (§2.14.1 self-decodability),
        // so the source NALs must appear in order as a suffix of the re-framed AU.
        assert!(
            reframed.len() >= src.len(),
            "sample {i}: AU lost NALs ({} < {})",
            reframed.len(),
            src.len()
        );
        assert_eq!(
            &reframed[reframed.len() - src.len()..],
            &src[..],
            "sample {i}: the coded NALs after the AUD/parameter sets must be byte-identical \
             to the source sample"
        );
    }
}

const H264_NAL_TYPE_MASK_IN_TEST: u8 = 0x1F;
const HEVC_NAL_TYPE_MASK_IN_TEST: u8 = 0x3F;
// ── Test 7 — an over-long audio PES is rejected, not clamped (r05-W19) ────────

/// A single audio sample whose PES framing would exceed 65535 bytes must make
/// the mux fail rather than emit a `PES_packet_length = 0xFFFF` header over a
/// longer payload (§2.4.3.7 reserves the unbounded `0` form for video).
#[test]
fn oversized_audio_sample_is_rejected_not_clamped() {
    use transmux::ir::{Sample, TrackSpec};
    use transmux::pipeline::CodecConfig;

    // A PES-carried Data stream: this muxer re-emits its samples verbatim in a
    // PES, so the sample length is exactly the PES payload length.
    let spec = TrackSpec::new(
        1,
        48_000,
        CodecConfig::Data {
            stream_type: 0x06,
            descriptors: Vec::new(),
            carriage: transmux::ir::DataCarriage::Pes,
        },
    );
    let sample = Sample::new(vec![0u8; 70_000], Some(0), Some(0), Some(1536), true);
    let track = transmux::media::Track::new(spec, vec![sample]);
    let media = Media::new(vec![track], 48_000);

    let err = TsMux::new().package(&media).unwrap_err();
    match err {
        transmux::Error::BufferCapExceeded { what, cap } => {
            assert_eq!(what, "PES_packet_length");
            assert_eq!(cap, 65_535);
        }
        other => panic!("expected BufferCapExceeded for PES_packet_length, got {other:?}"),
    }
}

// ── Test 8 — PCR PID selection + PCR-only fill (H.222.0 §2.4.2.2, r05-W20) ─────

/// Read the 13-bit `PCR_PID` from a PMT section (4 bytes into the body:
/// `reserved`(3) + `PCR_PID`(13)).
fn pmt_pcr_pid(sec: &[u8]) -> u16 {
    let body = &sec[8..sec.len() - 4];
    (((body[0] & 0x1F) as u16) << 8) | body[1] as u16
}

/// A PCR reading (90 kHz base) from a packet whose adaptation field carries one.
fn packet_pcr_90k(pkt: &[u8]) -> Option<u64> {
    let afc = (pkt[3] >> 4) & 0x3;
    if afc & 0b10 == 0 || pkt[4] < 7 || pkt[5] & 0x10 == 0 {
        return None;
    }
    let b = &pkt[6..12];
    Some(
        ((b[0] as u64) << 25)
            | ((b[1] as u64) << 17)
            | ((b[2] as u64) << 9)
            | ((b[3] as u64) << 1)
            | ((b[4] as u64) >> 7),
    )
}

/// Demux the real AC-3 fixture into a single-track `Media`, with each sample's
/// decode time stretched `spacing_ticks` apart so the emitted PCR cadence is
/// driven by the test rather than the fixture's own frame rate.
fn ac3_media(spacing_ticks: i64) -> Media {
    let ts = std::fs::read(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../fixtures/ts/dolby/ac3.ts"
    ))
    .expect("ac3 fixture must exist");
    let mut ir = TsDemux::new().unpackage(&ts).expect("demux AC-3 fixture");
    let t = ir.tracks.get_mut(0).expect("one AC-3 track");
    let scale = t.spec.timescale.max(1) as i64;
    // 4 samples, `spacing_ticks` apart (the first at 0).
    t.samples.truncate(4);
    for (i, s) in t.samples.iter_mut().enumerate() {
        let dts = (i as i64) * spacing_ticks;
        s.dts = Some(dts);
        s.pts = Some(dts);
        s.duration = Some(u32::try_from(scale).unwrap_or(u32::MAX));
    }
    let _ = scale;
    ir
}

/// The PCR PID must be the audio ES even when a PES-carried `Data` track is
/// listed first — a sparse subtitle/teletext PID cannot anchor the clock.
#[test]
fn pcr_pid_is_audio_not_a_sparse_data_pid() {
    use transmux::ir::{DataCarriage, Sample, TrackSpec};
    use transmux::pipeline::CodecConfig;

    let mut media = ac3_media(88_200);
    let audio_pid = media.tracks[0].spec.track_id;
    // A PES-carried Data track listed *before* the audio track.
    media.tracks.insert(
        0,
        transmux::media::Track::new(
            TrackSpec::new(
                99,
                90_000,
                CodecConfig::Data {
                    stream_type: 0x06,
                    descriptors: Vec::new(),
                    carriage: DataCarriage::Pes,
                },
            ),
            vec![Sample::new(
                vec![0xAAu8; 32],
                Some(0),
                Some(0),
                Some(9000),
                false,
            )],
        ),
    );
    let ts = TsMux::new().package(&media).unwrap();

    // PMT: first ES is the Data track (stream_type 0x06), second is AC-3 (0x81).
    let pmt = first_section(&ts, 0x1000).expect("PMT present");
    let streams = parse_pmt(&pmt);
    assert_eq!(streams[0].0, 0x06, "Data ES listed first");
    assert_eq!(streams[1].0, 0x81, "AC-3 ES listed second");
    assert_eq!(
        pmt_pcr_pid(&pmt),
        streams[1].1,
        "PCR PID must be the audio ES, not the sparse Data ES ({:#x})",
        streams[0].1
    );
    let _ = audio_pid;
}

/// With only a sparse audio ES, PCR-only packets must hold the interval within
/// the §2.4.2.2 / TR 101 290 2.3 bound (40 ms).
#[test]
fn sparse_audio_gets_pcr_only_packets() {
    // 1 s between audio samples (44100 ticks) => a 1 s PCR gap without filler.
    let media = ac3_media(44_100);
    let ts = TsMux::new().package(&media).unwrap();

    let pcr_pid = pmt_pcr_pid(&first_section(&ts, 0x1000).unwrap());
    let mut pcrs = Vec::new();
    for pkt in ts.chunks_exact(TS) {
        if pid_of(pkt) == pcr_pid
            && let Some(b) = packet_pcr_90k(pkt)
        {
            pcrs.push(b);
        }
    }
    assert!(pcrs.len() > 4, "PCR-only packets must have been inserted");
    for w in pcrs.windows(2) {
        assert!(
            w[1].saturating_sub(w[0]) <= 3600,
            "PCR interval {} ticks exceeds the 40 ms bound (§2.4.2.2 / TR 101 290 2.3)",
            w[1] - w[0]
        );
    }
}

/// A PCR-only packet carries no payload, so it must not advance the PCR PID's
/// continuity counter (§2.4.3.3).
#[test]
fn pcr_only_packets_do_not_advance_cc() {
    let media = ac3_media(44_100);
    let ts = TsMux::new().package(&media).unwrap();
    let pcr_pid = pmt_pcr_pid(&first_section(&ts, 0x1000).unwrap());

    let mut expected: Option<u8> = None;
    let mut inserted = 0usize;
    for pkt in ts.chunks_exact(TS) {
        if pid_of(pkt) != pcr_pid {
            continue;
        }
        let has_payload = (pkt[3] >> 4) & 0b01 != 0;
        let cc = pkt[3] & 0x0F;
        if !has_payload {
            inserted += 1;
            assert_eq!(
                expected,
                Some(cc),
                "a payload-less packet must repeat the preceding payload packet's CC                  (§2.4.3.3), not advance it"
            );
            continue;
        }
        let want = expected.map_or(0, |c| (c + 1) & 0x0F);
        assert_eq!(cc, want, "payload packet CC sequence");
        expected = Some(cc);
    }
    assert!(
        inserted > 0,
        "the sparse PCR PID must have had PCR-only packets inserted"
    );
}

// ── Test 9 — stream_id family overflow is refused (Table 2-22, r05-W22) ────────

/// A 17th video ES would take `stream_id` `0xF0` (`ECM_stream`), and a 33rd
/// audio ES `0xE0` (inside the video range) — the muxer must refuse rather than
/// mislabel the stream.
#[test]
fn too_many_elementary_streams_is_refused() {
    use transmux::media::Track;

    let fixture = std::fs::read(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../fixtures/ts/h264_aac.ts"
    ))
    .expect("h264_aac.ts fixture must exist");
    let ir = TsDemux::new().unpackage(&fixture).expect("demux fixture");
    let video = ir
        .tracks
        .iter()
        .find(|t| matches!(t.spec.config, CodecConfig::Avc { .. }))
        .expect("AVC track");
    let audio = ir
        .tracks
        .iter()
        .find(|t| matches!(t.spec.config, CodecConfig::Aac { .. }))
        .expect("AAC track");

    let clone_n = |t: &Track, n: usize| -> Vec<Track> {
        (0..n)
            .map(|i| {
                let mut c = t.clone();
                c.spec.track_id = u32::try_from(i + 1).unwrap();
                c
            })
            .collect()
    };

    // 16 video is legal; 17 is not.
    let ok = Media::new(clone_n(video, 16), 90_000);
    let ts = TsMux::new()
        .package(&ok)
        .expect("16 video streams must fit");
    assert_eq!(ts.len() % TS, 0);
    let err = TsMux::new()
        .package(&Media::new(clone_n(video, 17), 90_000))
        .unwrap_err();
    match err {
        transmux::Error::TooManyElementaryStreams { family, max } => {
            assert_eq!(family, "video");
            assert_eq!(max, 16);
        }
        other => panic!("expected TooManyElementaryStreams(video), got {other:?}"),
    }

    // 32 audio is legal; 33 is not.
    let ok = Media::new(clone_n(audio, 32), 48_000);
    TsMux::new()
        .package(&ok)
        .expect("32 audio streams must fit");
    let err = TsMux::new()
        .package(&Media::new(clone_n(audio, 33), 48_000))
        .unwrap_err();
    match err {
        transmux::Error::TooManyElementaryStreams { family, max } => {
            assert_eq!(family, "audio");
            assert_eq!(max, 32);
        }
        other => panic!("expected TooManyElementaryStreams(audio), got {other:?}"),
    }
}

// ── Test 10 — independent oracle on the AUD-bearing output (audit fix wave 2) ──

fn have_tool(tool: &str, arg: &str) -> bool {
    std::process::Command::new(tool)
        .arg(arg)
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

/// Run `ffprobe -count_frames` and return the video frame count.
fn ffprobe_video_frames(path: &std::path::Path) -> u64 {
    let out = std::process::Command::new("ffprobe")
        .args([
            "-v",
            "error",
            "-count_frames",
            "-select_streams",
            "v",
            "-show_entries",
            "stream=nb_read_frames",
            "-of",
            "csv=p=0",
        ])
        .arg(path)
        .output()
        .expect("spawn ffprobe");
    assert!(out.status.success(), "ffprobe rejected {}", path.display());
    String::from_utf8_lossy(&out.stdout)
        .lines()
        .find_map(|l| l.trim().parse().ok())
        .unwrap_or(0)
}

/// Count the AVC access-unit delimiters ffprobe reports (`-show_frames`).
fn ffprobe_aud_frames(path: &std::path::Path) -> usize {
    let out = std::process::Command::new("ffprobe")
        .args(["-v", "error", "-select_streams", "v", "-show_frames"])
        .arg(path)
        .output()
        .expect("spawn ffprobe");
    // ffprobe does not label AUDs, so count the frames whose pict_type is set —
    // every access unit ffprobe can split is one the AUD delimited.
    String::from_utf8_lossy(&out.stdout)
        .lines()
        .filter(|l| l.trim_start().starts_with("pict_type="))
        .count()
}

/// The AUD-bearing TS must decode to exactly the source's frame count, and every
/// access unit must be delimited. The source is a deliberately AUD-less MP4, so
/// this only passes because the muxer inserts the delimiters; ffprobe is the
/// independent judge of both the frame count and the split.
#[test]
fn aud_output_matches_source_frame_count_and_delimits_every_au() {
    if !have_tool("ffprobe", "-version") {
        eprintln!("SKIP ts_mux AUD oracle: ffprobe not on PATH");
        return;
    }
    let mp4 = std::fs::read(alloc_path("noaud/h264_noaud.mp4")).expect("noaud fixture");
    let ir = transmux::Fmp4Demux::new()
        .unpackage(&mp4)
        .expect("demux the AUD-less MP4");
    let ts2 = TsMux::new().package(&ir).expect("mux to TS");
    let src_frames = u64::try_from(ir.tracks[0].samples.len()).unwrap();

    let dir = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../target/ts-mux-oracle");
    std::fs::create_dir_all(&dir).unwrap();
    let out = dir.join("muxed-noaud.ts");
    std::fs::write(&out, &ts2).unwrap();

    assert_eq!(
        ffprobe_video_frames(&out),
        src_frames,
        "the muxed TS must decode to the input's frame count"
    );

    // Our own NAL parse: one AUD per access unit, and the AUD leads it.
    let pmt = first_section(&ts2, 0x1000).expect("PMT present");
    let streams = parse_pmt(&pmt);
    let pes = pes_payloads(&ts2, streams[0].1);
    assert_eq!(
        pes.len() as u64,
        src_frames,
        "one PES per access unit, matching the input frame count"
    );
    let auds: usize = pes
        .iter()
        .map(|p| {
            let es = pes_es(p);
            let mut count = 0usize;
            let mut i = 0usize;
            while i + 5 <= es.len() {
                if es[i..i + 4] == [0, 0, 0, 1] {
                    if es[i + 4] & 0x1F == 9 {
                        count += 1;
                    }
                    i += 4;
                } else {
                    i += 1;
                }
            }
            count
        })
        .sum();
    assert_eq!(
        auds, src_frames as usize,
        "exactly one AUD per access unit (our own parse)"
    );
    assert_eq!(
        ffprobe_aud_frames(&out) as u64,
        src_frames,
        "ffprobe splits the same number of frames out of our TS"
    );
}

/// The "already present" case: re-muxing our own AUD-bearing TS must not add a
/// second delimiter (TS → TS).
#[test]
fn re_muxing_an_aud_bearing_ts_does_not_duplicate_the_aud() {
    let (_ir, _ts2, ir2) = pipeline();
    let ts3 = TsMux::new()
        .package(&ir2)
        .expect("re-mux the AUD-bearing IR");
    let pmt = first_section(&ts3, 0x1000).expect("PMT present");
    let streams = parse_pmt(&pmt);
    let pes = pes_payloads(&ts3, streams[0].1);
    for (i, p) in pes.iter().enumerate() {
        let es = pes_es(p);
        // Count AUD NALs in this access unit.
        let mut auds = 0usize;
        let mut j = 0usize;
        while j + 5 <= es.len() {
            if es[j..j + 4] == [0, 0, 0, 1] {
                if es[j + 4] & 0x1F == 9 {
                    auds += 1;
                }
                j += 4;
            } else {
                j += 1;
            }
        }
        assert_eq!(
            auds, 1,
            "sample {i}: exactly one AUD after a second mux pass"
        );
    }
}

/// W20, independent oracle: TSDuck's PCR analysis (when installed) must report
/// no PCR repetition/discontinuity errors on the muxed output.
#[test]
fn muxed_ts_pcr_passes_tsanalyze() {
    if !have_tool("tsanalyze", "--version") {
        eprintln!("SKIP ts_mux PCR oracle: tsanalyze not on PATH");
        return;
    }
    let (_ir, ts2, _ir2) = pipeline();
    let dir = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../target/ts-mux-oracle");
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("pcr.ts");
    std::fs::write(&path, &ts2).unwrap();

    let out = std::process::Command::new("tsanalyze")
        .arg(&path)
        .output()
        .expect("spawn tsanalyze");
    let report = format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    // A PCR interval error shows as a non-zero "Errors:"/"Unexpect:" or the
    // "PCR repetition" line; the per-PID tables print "Unexpect: ......... N".
    for line in report.lines().filter(|l| l.contains("Unexpect:")) {
        let n: u64 = line
            .split("Unexpect:")
            .nth(1)
            .and_then(|s| s.split_whitespace().next())
            .and_then(|s| s.trim_start_matches('.').parse().ok())
            .unwrap_or(0);
        assert_eq!(n, 0, "tsanalyze reports PCR/continuity errors: {line}");
    }
    assert!(
        !report.contains("PCR repetition error"),
        "tsanalyze flags a PCR repetition error:\n{report}"
    );
}
