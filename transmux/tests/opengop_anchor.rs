//! Open-GOP AVC random-access anchor gate — issue #595.
//!
//! Broadcast H.264 is frequently **open-GOP**: it never codes an IDR (NAL
//! type 5) at all. Each GOP instead opens with SPS(7)/PPS(8) + a non-IDR
//! I-slice, usually announced by a `recovery_point` SEI message (ITU-T H.264
//! Annex D.1.7 syntax / D.2.7 semantics). Before #595, `TsDemux` /
//! `StreamingTsDemux` set `Sample.is_sync` only for an IDR NAL, so an
//! open-GOP AU never anchored — `Segmenter` buffered the *entire* stream into
//! one giant segment.
//!
//! Fixtures:
//! - `fixtures/ts/gulli-opengop.ts` — real open-GOP Gulli DVB-T capture
//!   (H.264 PID `0x100`): **zero IDR**; 5 SPS-led GOP starts, each carrying a
//!   `recovery_point` SEI (see `fixtures/ts/gulli-opengop-PROVENANCE.md`).
//! - `fixtures/ts/h264/main.ts` — closed-GOP IDR fixture (H.264 PID `0x100`):
//!   proves IDR-based anchoring is unchanged.
//!
//! Test 1 re-derives the open-GOP RAP count with a NAL walker written
//! independently of `transmux::nal` (own PES/AU reassembly, own start-code
//! scan, own `sei_message()` `payloadType` decode) so the "matches" assertion
//! actually cross-checks the implementation rather than exercising the same
//! code path twice.

use std::path::PathBuf;

use transmux::{CodecConfig, MovieFragmentBox, Segmenter, TsDemux};

fn fixtures_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../fixtures/ts")
}

/// AVC video elementary-stream PID in both committed fixtures.
const VIDEO_PID: u16 = 0x0100;

// ── Independent (test-owned) TS → AU → NAL-type walk ────────────────────────
//
// Deliberately does NOT call anything in `transmux::nal` or `transmux::annexb`
// — this is a from-scratch re-implementation so Test 1's "is_sync count
// matches the independent count" assertion is a real cross-check.

/// Reassemble one PID's access units from raw TS packets (PUSI marks a new
/// PES packet = one AU). Returns each AU as its full PES packet bytes
/// (header not yet stripped).
fn reassemble_pes_packets(ts: &[u8], pid: u16) -> Vec<Vec<u8>> {
    const TS_PACKET: usize = 188;
    let mut packets: Vec<Vec<u8>> = Vec::new();
    let mut cur: Option<Vec<u8>> = None;
    for pkt in ts.chunks_exact(TS_PACKET) {
        if pkt[0] != 0x47 {
            continue;
        }
        let pkt_pid = (((pkt[1] & 0x1F) as u16) << 8) | pkt[2] as u16;
        if pkt_pid != pid {
            continue;
        }
        let pusi = pkt[1] & 0x40 != 0;
        let afc = (pkt[3] >> 4) & 0x3;
        let mut off = 4usize;
        if afc & 0x2 != 0 {
            off += 1 + pkt[4] as usize; // adaptation_field_length + the field
        }
        if afc & 0x1 == 0 || off >= TS_PACKET {
            continue;
        }
        let payload = &pkt[off..];
        if pusi {
            if let Some(prev) = cur.take() {
                packets.push(prev);
            }
            cur = Some(payload.to_vec());
        } else if let Some(c) = cur.as_mut() {
            c.extend_from_slice(payload);
        }
    }
    if let Some(c) = cur {
        packets.push(c);
    }
    packets
}

/// Strip a PES packet header (ITU-T H.222.0 §2.4.3.6), returning the
/// elementary-stream Annex B payload.
fn pes_es_payload(pes: &[u8]) -> &[u8] {
    assert_eq!(&pes[0..3], &[0x00, 0x00, 0x01], "PES start-code prefix");
    let header_data_len = pes[8] as usize;
    &pes[9 + header_data_len..]
}

/// Split an Annex B byte stream into its NAL units (start code `00 00 01`
/// stripped, trailing `zero_byte` padding stripped).
fn split_annexb_nals(annexb: &[u8]) -> Vec<&[u8]> {
    let n = annexb.len();
    let mut positions = Vec::new();
    let mut i = 0usize;
    while i + 3 <= n {
        if annexb[i] == 0 && annexb[i + 1] == 0 && annexb[i + 2] == 1 {
            positions.push(i);
            i += 3;
        } else {
            i += 1;
        }
    }
    let mut nals = Vec::new();
    for (idx, &pos) in positions.iter().enumerate() {
        let start = pos + 3;
        let end = positions.get(idx + 1).copied().unwrap_or(n);
        if start >= end {
            continue;
        }
        let mut nal = &annexb[start..end];
        while let Some((&0, rest)) = nal.split_last() {
            nal = rest;
        }
        if !nal.is_empty() {
            nals.push(nal);
        }
    }
    nals
}

/// Remove H.264 `emulation_prevention_three_byte` (`00 00 03` → `00 00`).
fn unescape_ebsp(nal: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(nal.len());
    let mut i = 0usize;
    while i < nal.len() {
        if i + 2 < nal.len() && nal[i] == 0 && nal[i + 1] == 0 && nal[i + 2] == 3 {
            out.push(0);
            out.push(0);
            i += 3;
        } else {
            out.push(nal[i]);
            i += 1;
        }
    }
    out
}

/// Whether a type-6 SEI NAL carries a `recovery_point` message (payloadType
/// 6) — ITU-T H.264 §7.3.2.3.1 `sei_payload()` varint coding, walked
/// independently of `transmux::nal::recovery_point_sei`.
fn independent_sei_has_recovery_point(nal: &[u8]) -> bool {
    if nal.is_empty() || nal[0] & 0x1F != 6 {
        return false;
    }
    let rbsp = unescape_ebsp(&nal[1..]);
    let mut i = 0usize;
    while i < rbsp.len() {
        let mut payload_type: u32 = 0;
        loop {
            if i >= rbsp.len() {
                return false;
            }
            let b = rbsp[i];
            i += 1;
            payload_type += u32::from(b);
            if b != 0xFF {
                break;
            }
        }
        let mut payload_size: u32 = 0;
        loop {
            if i >= rbsp.len() {
                return false;
            }
            let b = rbsp[i];
            i += 1;
            payload_size += u32::from(b);
            if b != 0xFF {
                break;
            }
        }
        if payload_type == 6 {
            return true;
        }
        i += payload_size as usize;
        if i >= rbsp.len() {
            return false;
        }
    }
    false
}

/// Whether an Annex B AVC access unit is an open-GOP RAP by the independent
/// walk: IDR (type 5), SPS (type 7), or a `recovery_point` SEI (type 6).
fn independent_au_is_rap(annexb: &[u8]) -> bool {
    for nal in split_annexb_nals(annexb) {
        let nal_type = nal[0] & 0x1F;
        if nal_type == 5 || nal_type == 7 {
            return true;
        }
        if nal_type == 6 && independent_sei_has_recovery_point(nal) {
            return true;
        }
    }
    false
}

/// Reassemble Annex B access units for `pid` from a raw TS byte slice.
fn access_units(ts: &[u8], pid: u16) -> Vec<Vec<u8>> {
    reassemble_pes_packets(ts, pid)
        .iter()
        .map(|pes| pes_es_payload(pes).to_vec())
        .collect()
}

// ── moof inspection (reused pattern from tests/segmenter.rs) ────────────────

/// Find the first top-level box of `fourcc` in a serialized segment and return
/// its **body** (bytes after the 8-byte box header).
fn find_box_body<'a>(data: &'a [u8], fourcc: &[u8; 4]) -> Option<&'a [u8]> {
    let mut off = 0usize;
    while off + 8 <= data.len() {
        let size = u32::from_be_bytes(data[off..off + 4].try_into().unwrap()) as usize;
        let ty = &data[off + 4..off + 8];
        if size < 8 || off + size > data.len() {
            return None;
        }
        if ty == fourcc {
            return Some(&data[off + 8..off + size]);
        }
        off += size;
    }
    None
}

/// Total sample count across every `traf`/`trun` in one media segment's `moof`.
fn segment_sample_count(segment: &[u8]) -> usize {
    let moof = find_box_body(segment, b"moof").expect("segment has moof");
    let mf = MovieFragmentBox::parse_body(moof).expect("moof parses");
    mf.traf
        .iter()
        .map(|traf| traf.trun.iter().map(|r| r.samples.len()).sum::<usize>())
        .sum()
}

// ── Test 1: open-GOP fixture anchors + segments (the headline case) ─────────

#[test]
fn open_gop_gulli_fixture_anchors_and_segments() {
    let ts = std::fs::read(fixtures_dir().join("gulli-opengop.ts"))
        .expect("gulli-opengop.ts fixture must exist");

    // Independent cross-check: walk the raw TS ourselves and count AUs that
    // carry an open-GOP RAP signal (SPS or recovery-point SEI; no IDR exists
    // in this fixture per the provenance note).
    let aus = access_units(&ts, VIDEO_PID);
    assert!(
        aus.len() > 100,
        "fixture must carry many AUs (got {})",
        aus.len()
    );
    let independent_rap_indices: Vec<usize> = aus
        .iter()
        .enumerate()
        .filter(|(_, au)| independent_au_is_rap(au))
        .map(|(i, _)| i)
        .collect();
    assert!(
        independent_rap_indices.len() >= 2,
        "independent walk must find >=2 open-GOP RAPs (got {})",
        independent_rap_indices.len()
    );
    // No IDR anywhere in this fixture (confirms it truly is open-GOP, not
    // something that happens to also carry IDR).
    assert!(
        aus.iter()
            .all(|au| !split_annexb_nals(au).iter().any(|nal| nal[0] & 0x1F == 5)),
        "fixture must contain zero IDR NALs (open-GOP)"
    );

    // Now demux through the real pipeline and check `is_sync`.
    let mut demux = TsDemux::new();
    let media = demux.demux(&ts).expect("demux gulli-opengop.ts");
    let video = media
        .tracks
        .iter()
        .find(|t| matches!(t.spec.config, CodecConfig::Avc { .. }))
        .expect("AVC video track present");
    assert_eq!(
        video.samples.len(),
        aus.len(),
        "demuxed sample count must equal our own AU count"
    );

    let is_sync_indices: Vec<usize> = video
        .samples
        .iter()
        .enumerate()
        .filter(|(_, s)| s.is_sync)
        .map(|(i, _)| i)
        .collect();

    // Pre-fix this would be empty (0 is_sync AUs) — the headline assertion.
    assert!(
        is_sync_indices.len() >= 2,
        "expected >=2 is_sync access units on an open-GOP fixture, got {}",
        is_sync_indices.len()
    );
    assert_eq!(
        is_sync_indices, independent_rap_indices,
        "demuxer is_sync positions must match the independently-walked open-GOP RAP positions"
    );

    // Segment the demuxed track: a target duration far below the real ~2.3 s
    // GOP spacing forces a cut on every subsequent RAP.
    let mut seg = Segmenter::new(vec![video.spec.clone()], video.spec.timescale, 0.05)
        .expect("segmenter construction");
    let init = seg.init_segment().expect("init segment");
    assert!(!init.is_empty());

    let mut segments: Vec<Vec<u8>> = Vec::new();
    for sample in &video.samples {
        seg.push(video.spec.track_id, sample.clone())
            .expect("segmenter push");
        segments.extend(seg.take_ready());
    }
    seg.flush().expect("segmenter flush");
    segments.extend(seg.take_ready());

    // Pre-fix, this fixture yields exactly one giant segment. Post-fix it
    // must cut at every GOP start.
    assert!(
        segments.len() > 1,
        "open-GOP stream must cut into multiple segments, got {}",
        segments.len()
    );

    // Sane per-segment sample counts: every segment carries at least one
    // sample, no segment carries the *entire* stream, and the segments
    // together are lossless (sum == total pushed samples).
    let mut total = 0usize;
    for s in &segments {
        let n = segment_sample_count(s);
        assert!(n > 0, "every emitted segment must carry >=1 sample");
        assert!(
            n < video.samples.len(),
            "no single segment may carry the whole stream (got {n} of {})",
            video.samples.len()
        );
        total += n;
    }
    assert_eq!(
        total,
        video.samples.len(),
        "segments must losslessly cover every pushed sample"
    );
}

// ── Test 2: closed-GOP IDR fixture is unchanged ──────────────────────────────

#[test]
fn closed_gop_h264_main_ts_idr_anchoring_unchanged() {
    let ts = std::fs::read(fixtures_dir().join("h264/main.ts")).expect("h264/main.ts fixture");

    let aus = access_units(&ts, VIDEO_PID);
    assert!(aus.len() > 1, "need multiple AUs to bite");

    // Independent IDR-only count (this fixture is closed-GOP: SPS is sent
    // only once, alongside the sole IDR — no separate open-GOP signal).
    let independent_idr_indices: Vec<usize> = aus
        .iter()
        .enumerate()
        .filter(|(_, au)| split_annexb_nals(au).iter().any(|nal| nal[0] & 0x1F == 5))
        .map(|(i, _)| i)
        .collect();
    assert!(
        !independent_idr_indices.is_empty(),
        "closed-GOP fixture must contain at least one IDR"
    );

    let mut demux = TsDemux::new();
    let media = demux.demux(&ts).expect("demux h264/main.ts");
    let video = media
        .tracks
        .iter()
        .find(|t| matches!(t.spec.config, CodecConfig::Avc { .. }))
        .expect("AVC video track present");
    assert_eq!(video.samples.len(), aus.len());

    let is_sync_indices: Vec<usize> = video
        .samples
        .iter()
        .enumerate()
        .filter(|(_, s)| s.is_sync)
        .map(|(i, _)| i)
        .collect();

    assert_eq!(
        is_sync_indices, independent_idr_indices,
        "closed-GOP is_sync positions must match the IDR positions exactly (unchanged behaviour)"
    );
    // Bite: not every AU is a keyframe, otherwise the comparison is trivial.
    assert!(is_sync_indices.len() < video.samples.len());
}
