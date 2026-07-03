//! `TsHlsPackager` gate — classic HLS (MPEG-2 TS `.ts` segments + media
//! playlist), verified by round-tripping the outputs through the independent,
//! ffmpeg-oracle-gated `TsDemux` (issue #472).
//!
//! Oracle: `TsDemux` is byte-oracle-gated against ffmpeg in `tests/ts_demux.rs`.
//! So demuxing the concatenated `.ts` segments (or the first segment alone) and
//! comparing tracks / coded NAL payloads / audio frames / timing against the
//! original IR proves segmentation is lossless and each segment is decodable —
//! none of it can be faked (a raw-passthrough serialize would not parse back as
//! valid TS with in-segment PAT/PMT + keyframe starts).
//!
//! Pipeline: `ir = TsDemux(h264_aac.ts)` → `out = TsHlsPackager(ir)` →
//! demux `out.segments` (concat and per-segment) → compare.

use broadcast_common::{Package, Serialize, Unpackage};
use transmux::media::{Media, Track};
use transmux::pipeline::CodecConfig;
use transmux::{TsDemux, TsHlsOutput, TsHlsPackager};

// ── Fixture + pipeline ───────────────────────────────────────────────────────

fn load_ts() -> Vec<u8> {
    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../fixtures/ts/h264_aac.ts");
    std::fs::read(path).expect("h264_aac.ts fixture must exist")
}

fn demux(ts: &[u8]) -> Media {
    TsDemux::new().unpackage(ts).expect("demux TS")
}

/// A small target (1s) forces multiple segments on the fixture.
fn package(ir: &Media, target_secs: u32) -> TsHlsOutput {
    TsHlsPackager::new(target_secs)
        .package(ir)
        .expect("package classic HLS")
}

// ── Minimal TS packet + PSI walking (byte-level, no crate internals) ──────────

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

/// Reassemble the first complete single-packet PSI section carried on `pid`.
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

/// The packet index (in units of 188 bytes) of the first media PES packet — the
/// first packet whose PID is neither 0 (PAT) nor the PMT PID.
fn first_media_packet_index(ts: &[u8], pmt_pid: u16) -> Option<usize> {
    for (i, pkt) in ts.chunks_exact(TS).enumerate() {
        let pid = pid_of(pkt);
        if pid != 0x0000 && pid != pmt_pid {
            return Some(i);
        }
    }
    None
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

/// H.264 `nal_unit_type` for a coded slice of an IDR picture (Table 7-1).
const H264_NAL_IDR: u8 = 5;
const H264_NAL_TYPE_MASK: u8 = 0x1F;

/// True if any NAL in a length-prefixed video sample is an IDR slice.
fn has_idr(sample_data: &[u8]) -> bool {
    split_lp(sample_data)
        .iter()
        .any(|n| !n.is_empty() && (n[0] & H264_NAL_TYPE_MASK) == H264_NAL_IDR)
}

fn avcc_body(track: &Track) -> Vec<u8> {
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

/// Index of the (first) AVC video track in an IR.
fn video_idx(m: &Media) -> usize {
    m.tracks
        .iter()
        .position(|t| matches!(t.spec.config, CodecConfig::Avc { .. }))
        .expect("a video track")
}
fn audio_idx(m: &Media) -> usize {
    m.tracks
        .iter()
        .position(|t| matches!(t.spec.config, CodecConfig::Aac { .. }))
        .expect("an audio track")
}

// ── Test 1 — segment structure: whole packets, in-segment PAT+PMT, count ──────

#[test]
fn segments_are_whole_packets_with_pat_pmt_and_expected_count() {
    let ir = demux(&load_ts());
    let out = package(&ir, 1);

    // Every segment is a whole number of 188-byte packets, and non-empty.
    for (i, seg) in out.segments.iter().enumerate() {
        assert!(!seg.is_empty(), "segment {i} must not be empty");
        assert_eq!(seg.len() % TS, 0, "segment {i} must be whole TS packets");
    }

    // Each segment opens with a PAT (PID 0) and a PMT, both before the first
    // media PES packet (parse them; do not byte-match).
    for (i, seg) in out.segments.iter().enumerate() {
        let pat = first_section(seg, 0x0000).unwrap_or_else(|| panic!("segment {i} PAT"));
        assert_eq!(pat[0], 0x00, "segment {i} PAT table_id");
        // The PAT points at a PMT PID; that PMT must resolve and list the ES.
        let body = &pat[8..pat.len() - 4];
        let pmt_pid = (((body[2] & 0x1F) as u16) << 8) | body[3] as u16;
        let pmt = first_section(seg, pmt_pid).unwrap_or_else(|| panic!("segment {i} PMT"));
        assert_eq!(pmt[0], 0x02, "segment {i} PMT table_id");
        let streams = parse_pmt(&pmt);
        assert!(
            streams.iter().any(|s| s.0 == 0x1B),
            "segment {i} PMT lists H.264"
        );

        // PAT and PMT precede the first media PES packet.
        let pat_pkt = seg
            .chunks_exact(TS)
            .position(|p| pid_of(p) == 0x0000 && pusi_of(p))
            .unwrap();
        let pmt_pkt = seg
            .chunks_exact(TS)
            .position(|p| pid_of(p) == pmt_pid && pusi_of(p))
            .unwrap();
        let media_pkt = first_media_packet_index(seg, pmt_pid).unwrap();
        assert!(pat_pkt < media_pkt, "segment {i}: PAT before media");
        assert!(pmt_pkt < media_pkt, "segment {i}: PMT before media");
    }

    // Segment count == ceil(total_video_duration / target) computed from the IR.
    let v = video_idx(&ir);
    let total_ticks: u64 = ir.tracks[v].samples.iter().map(|s| s.duration as u64).sum();
    let scale = ir.tracks[v].spec.timescale.max(1) as u64;
    let target = 1u64; // seconds
    let expected = total_ticks.div_ceil(target * scale).max(1) as usize;
    assert_eq!(
        out.segments.len(),
        expected,
        "segment count must equal ceil(total_video_dur / target) = {expected}"
    );
    assert!(out.segments.len() > 1, "small target must yield >1 segment");
}

// ── Test 2 — keyframe-aligned starts: each segment's first video AU is an IDR ──

#[test]
fn each_segment_starts_on_a_keyframe() {
    let ir = demux(&load_ts());
    let out = package(&ir, 1);

    for (i, seg) in out.segments.iter().enumerate() {
        let m = demux(seg);
        let v = video_idx(&m);
        let first = &m.tracks[v].samples[0];
        assert!(
            first.is_sync,
            "segment {i}: first video sample must be a sync sample"
        );
        assert!(
            has_idr(&first.data),
            "segment {i}: first video AU must contain an IDR NAL (type 5)"
        );
    }
}

// ── Test 3 — playlist consistency ─────────────────────────────────────────────

#[test]
fn playlist_is_consistent_with_segments() {
    let ir = demux(&load_ts());
    let out = package(&ir, 1);
    let pl = &out.playlist;

    assert!(
        pl.starts_with("#EXTM3U\n"),
        "playlist must start with #EXTM3U"
    );
    assert!(pl.contains("#EXT-X-VERSION:"), "has #EXT-X-VERSION");
    assert!(pl.contains("#EXT-X-MEDIA-SEQUENCE:0"), "has media sequence");
    assert!(pl.trim_end().ends_with("#EXT-X-ENDLIST"), "VOD endlist");

    // Parse #EXTINF durations + URIs.
    let target: f64 = pl
        .lines()
        .find_map(|l| l.strip_prefix("#EXT-X-TARGETDURATION:"))
        .expect("target duration")
        .trim()
        .parse()
        .unwrap();

    let mut extinfs: Vec<f64> = Vec::new();
    let mut uris: Vec<&str> = Vec::new();
    let mut lines = pl.lines().peekable();
    while let Some(line) = lines.next() {
        if let Some(rest) = line.strip_prefix("#EXTINF:") {
            let dur: f64 = rest.trim_end_matches(',').parse().unwrap();
            extinfs.push(dur);
            let uri = lines.next().expect("URI after #EXTINF");
            assert!(!uri.starts_with('#'), "URI line must not be a tag");
            assert!(uri.ends_with(".ts"), "classic HLS uses .ts URIs");
            uris.push(uri);
        }
    }

    // #EXTINF/URI pair count == segment count.
    assert_eq!(extinfs.len(), out.segments.len(), "one #EXTINF per segment");
    assert_eq!(uris.len(), out.segments.len(), "one URI per segment");

    // No #EXT-X-MAP (that is a CMAF-only concept).
    assert!(!pl.contains("#EXT-X-MAP"), "no init segment for TS media");

    // #EXT-X-TARGETDURATION >= every #EXTINF.
    for (i, &d) in extinfs.iter().enumerate() {
        assert!(target >= d, "target {target} must be >= EXTINF[{i}] {d}");
    }

    // Sum of #EXTINF ≈ total video duration (within rounding).
    let v = video_idx(&ir);
    let total_ticks: u64 = ir.tracks[v].samples.iter().map(|s| s.duration as u64).sum();
    let scale = ir.tracks[v].spec.timescale.max(1) as f64;
    let total_secs = total_ticks as f64 / scale;
    let sum: f64 = extinfs.iter().sum();
    assert!(
        (sum - total_secs).abs() < 0.5,
        "sum of #EXTINF {sum} ≈ total video duration {total_secs}"
    );
}

// ── Test 4 — lossless concat round-trip ───────────────────────────────────────

#[test]
fn concatenated_segments_round_trip_losslessly() {
    let ir = demux(&load_ts());
    let out = package(&ir, 1);

    // Concatenate all .ts segments and demux the result.
    let mut concat = Vec::new();
    for seg in &out.segments {
        concat.extend_from_slice(seg);
    }
    let ir2 = demux(&concat);

    assert_eq!(ir2.tracks.len(), 2, "2 tracks recovered");

    // Video: coded NAL payloads byte-identical, sample-for-sample, count 75.
    let v0 = &ir.tracks[video_idx(&ir)];
    let v2 = &ir2.tracks[video_idx(&ir2)];
    assert_eq!(
        v0.samples.len(),
        75,
        "expected 75 video samples in original"
    );
    assert_eq!(
        v2.samples.len(),
        v0.samples.len(),
        "video sample count preserved through segmentation"
    );
    for (i, (a, b)) in v0.samples.iter().zip(&v2.samples).enumerate() {
        assert_eq!(
            split_lp(&a.data),
            split_lp(&b.data),
            "video sample {i}: coded NAL payloads must be byte-identical"
        );
        // Timing preserved (DTS delta = duration, and composition offset).
        assert_eq!(
            b.duration, a.duration,
            "video sample {i}: duration preserved"
        );
        assert_eq!(
            b.composition_offset, a.composition_offset,
            "video sample {i}: composition offset preserved"
        );
    }

    // Audio: 131 raw AAC frames byte-identical.
    let a0 = &ir.tracks[audio_idx(&ir)];
    let a2 = &ir2.tracks[audio_idx(&ir2)];
    assert_eq!(
        a0.samples.len(),
        131,
        "expected 131 audio frames in original"
    );
    assert_eq!(
        a2.samples.len(),
        131,
        "audio frame count preserved through segmentation"
    );
    for (i, (a, b)) in a0.samples.iter().zip(&a2.samples).enumerate() {
        assert_eq!(
            a.data, b.data,
            "audio sample {i}: raw AAC bytes byte-identical"
        );
    }
}

// ── Test 5 — independent-segment decodability ─────────────────────────────────

#[test]
fn first_segment_is_independently_decodable() {
    let ir = demux(&load_ts());
    let out = package(&ir, 1);

    // Demux the FIRST segment alone.
    let m = demux(&out.segments[0]);
    let v = video_idx(&m);

    // Its first video sample is a sync sample.
    assert!(
        m.tracks[v].samples[0].is_sync,
        "first segment's first video sample must be a sync sample"
    );

    // avcC / config is recoverable in-segment (PAT/PMT + SPS/PPS present), and
    // byte-identical to the original stream's avcC.
    let orig_avcc = avcc_body(&ir.tracks[video_idx(&ir)]);
    let seg_avcc = avcc_body(&m.tracks[v]);
    assert_eq!(
        seg_avcc, orig_avcc,
        "first segment's recovered avcC must match the original (SPS/PPS in-segment)"
    );

    // And the first AU carries the IDR.
    assert!(
        has_idr(&m.tracks[v].samples[0].data),
        "first segment's first AU must contain an IDR"
    );

    // The LAST segment is likewise independently decodable — its avcC recovers
    // (SPS/PPS present in-segment) even though it is not the stream's first.
    let last = demux(out.segments.last().unwrap());
    let lv = video_idx(&last);
    assert!(
        last.tracks[lv].samples[0].is_sync,
        "last segment's first video sample must be a sync sample"
    );
    assert_eq!(
        avcc_body(&last.tracks[lv]),
        orig_avcc,
        "last segment's recovered avcC must match the original (SPS/PPS in-segment)"
    );
}
