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
            b.composition_offset, a.composition_offset,
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
