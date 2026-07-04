//! DVB player track-picker provenance — issue #582.
//!
//! `TrackSpec::source_pid` + `TrackSpec::es_info_descriptors` must be
//! populated for EVERY TS-demuxed track (codec and opaque `Data` alike), so a
//! player can select/label tracks without running its own PAT/PMT parser in
//! parallel.
//!
//! Fixture: `fixtures/ts/france2.ts` — one PMT (PID `0x6E`), 6 elementary
//! streams: H.264 video (`0x78`), 3× audio with ISO-639 + E-AC-3 descriptors
//! (`0x82`/`0x83`/`0x84`, languages `fre`/`qad`/`qaa`), 2× DVB-subtitled
//! streams (`0x8C`/`0x8E`). See `fixtures/ts/france2-PROVENANCE.md`.
//!
//! This test's PAT/PMT walk is entirely independent of `transmux::ts_demux`'s
//! internal `parse_pmt` — it re-implements the byte-level walk from scratch so
//! the comparison is a real oracle, not a tautology.

use std::collections::BTreeMap;

use broadcast_common::{Package, Unpackage};
use transmux::pipeline::CodecConfig;
use transmux::{CmafMux, DemuxEvent, Fmp4Demux, Media, StreamingTsDemux, TsDemux};

const TS_PACKET_LEN: usize = 188;
const SYNC_BYTE: u8 = 0x47;
const TABLE_ID_PAT: u8 = 0x00;
const TABLE_ID_PMT: u8 = 0x02;
const DESC_TAG_ISO_639_LANGUAGE: u8 = 0x0A;
const DESC_TAG_DVB_SUBTITLING: u8 = 0x59;

const EXPECTED_VIDEO_PID: u16 = 0x78;
const EXPECTED_AUDIO_PIDS: [u16; 3] = [0x82, 0x83, 0x84];
const EXPECTED_SUBTITLE_PIDS: [u16; 2] = [0x8C, 0x8E];

fn fixture_bytes() -> Vec<u8> {
    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../fixtures/ts/france2.ts");
    std::fs::read(path).expect("fixtures/ts/france2.ts must exist (issue #582 prep fixture)")
}

// ---------------------------------------------------------------------------
// Independent, from-scratch PAT/PMT walk (byte-level, no crate internals).
// ---------------------------------------------------------------------------

fn pid_of(pkt: &[u8]) -> u16 {
    (((pkt[1] & 0x1F) as u16) << 8) | pkt[2] as u16
}

fn pusi_of(pkt: &[u8]) -> bool {
    pkt[1] & 0x40 != 0
}

/// Payload offset in a packet (skips the 4-byte header + any adaptation
/// field). Returns `TS_PACKET_LEN` (i.e. "no payload") for AF-only packets.
fn payload_offset(pkt: &[u8]) -> usize {
    let afc = (pkt[3] >> 4) & 0x3;
    let has_af = afc & 0b10 != 0;
    let has_payload = afc & 0b01 != 0;
    if !has_payload {
        return TS_PACKET_LEN;
    }
    if has_af { 4 + 1 + pkt[4] as usize } else { 4 }
}

/// Reassemble a PSI section carried on `pid` whose first byte is `table_id`.
/// Handles sections that continue across multiple TS packets on the same PID
/// (payload accumulated in packet order, from the `pusi` packet's
/// `pointer_field` up to `3 + section_length` bytes).
fn reassemble_section(ts: &[u8], pid: u16, table_id: u8) -> Vec<u8> {
    let mut buf: Vec<u8> = Vec::new();
    let mut needed: Option<usize> = None;
    for pkt in ts.chunks_exact(TS_PACKET_LEN) {
        if pkt[0] != SYNC_BYTE || pid_of(pkt) != pid {
            continue;
        }
        let off = payload_offset(pkt);
        if off >= TS_PACKET_LEN {
            continue;
        }
        let payload = &pkt[off..];
        if pusi_of(pkt) {
            let ptr = payload[0] as usize;
            if 1 + ptr >= payload.len() {
                continue;
            }
            let sec = &payload[1 + ptr..];
            if (sec.is_empty() || sec[0] != table_id) && needed.is_none() {
                continue; // stuffing / a different table before ours
            }
            buf.clear();
            buf.extend_from_slice(sec);
            let section_length = (((sec[1] & 0x0F) as usize) << 8) | sec[2] as usize;
            needed = Some(3 + section_length);
        } else if needed.is_some() {
            buf.extend_from_slice(payload);
        } else {
            continue;
        }
        if let Some(n) = needed {
            if buf.len() >= n {
                buf.truncate(n);
                return buf;
            }
        }
    }
    panic!("PSI section (table_id {table_id:#04x}) on PID {pid:#06x} never fully reassembled");
}

/// PAT → list of `(program_number, program_map_PID)`.
fn parse_pat(sec: &[u8]) -> Vec<(u16, u16)> {
    assert_eq!(sec[0], TABLE_ID_PAT, "PAT table_id");
    let body = &sec[8..sec.len() - 4]; // strip 8-byte section header + 4-byte CRC_32
    let mut out = Vec::new();
    let mut i = 0;
    while i + 4 <= body.len() {
        let program_number = u16::from_be_bytes([body[i], body[i + 1]]);
        let pid = (((body[i + 2] & 0x1F) as u16) << 8) | body[i + 3] as u16;
        out.push((program_number, pid));
        i += 4;
    }
    out
}

/// PMT → `(elementary_PID, stream_type, ES_info descriptor-loop bytes)` for
/// every elementary stream (ISO/IEC 13818-1 §2.4.4.8).
fn parse_pmt(sec: &[u8]) -> Vec<(u16, u8, Vec<u8>)> {
    assert_eq!(sec[0], TABLE_ID_PMT, "PMT table_id");
    let body = &sec[8..sec.len() - 4]; // strip 8-byte section header + 4-byte CRC_32
    // reserved(3)+PCR_PID(13) [2 bytes], then reserved(4)+program_info_length(12) [2 bytes].
    let program_info_length = (((body[2] & 0x0F) as usize) << 8) | body[3] as usize;
    let mut i = 4 + program_info_length;
    let mut out = Vec::new();
    while i + 5 <= body.len() {
        let stream_type = body[i];
        let es_pid = (((body[i + 1] & 0x1F) as u16) << 8) | body[i + 2] as u16;
        let es_info_length = (((body[i + 3] & 0x0F) as usize) << 8) | body[i + 4] as usize;
        let desc_start = i + 5;
        let desc_end = (desc_start + es_info_length).min(body.len());
        out.push((es_pid, stream_type, body[desc_start..desc_end].to_vec()));
        i += 5 + es_info_length;
    }
    out
}

/// Walk PAT → (single program's) PMT and return `PID -> (stream_type,
/// ES_info bytes)` for every elementary stream, entirely independent of
/// `transmux`'s own PMT parser.
fn independent_pmt_walk(ts: &[u8]) -> BTreeMap<u16, (u8, Vec<u8>)> {
    let pat = reassemble_section(ts, 0x0000, TABLE_ID_PAT);
    let programs = parse_pat(&pat);
    let (_, pmt_pid) = *programs
        .iter()
        .find(|(program_number, _)| *program_number != 0)
        .expect("PAT must list at least one program");
    let pmt = reassemble_section(ts, pmt_pid, TABLE_ID_PMT);
    parse_pmt(&pmt)
        .into_iter()
        .map(|(pid, stream_type, descriptors)| (pid, (stream_type, descriptors)))
        .collect()
}

/// Walk a descriptor loop's tag/length TLVs; return the first descriptor body
/// matching `tag` (ISO/IEC 13818-1 §2.6, descriptor()).
fn find_descriptor(desc_loop: &[u8], tag: u8) -> Option<&[u8]> {
    let mut i = 0;
    while i + 2 <= desc_loop.len() {
        let t = desc_loop[i];
        let len = desc_loop[i + 1] as usize;
        let start = i + 2;
        let end = (start + len).min(desc_loop.len());
        if t == tag {
            return Some(&desc_loop[start..end]);
        }
        i = end;
    }
    None
}

// ---------------------------------------------------------------------------
// Test 1 — source_pid + es_info_descriptors on every batch-demuxed track.
// ---------------------------------------------------------------------------

#[test]
fn every_track_carries_its_pmt_pid_and_es_info() {
    let ts = fixture_bytes();
    let expected = independent_pmt_walk(&ts);

    let expected_pids: Vec<u16> = {
        let mut v: Vec<u16> = expected.keys().copied().collect();
        v.sort_unstable();
        v
    };
    let mut want_pids = Vec::new();
    want_pids.push(EXPECTED_VIDEO_PID);
    want_pids.extend_from_slice(&EXPECTED_AUDIO_PIDS);
    want_pids.extend_from_slice(&EXPECTED_SUBTITLE_PIDS);
    want_pids.sort_unstable();
    assert_eq!(
        expected_pids, want_pids,
        "independent PAT/PMT walk must find exactly the 6 documented elementary streams"
    );

    let media = TsDemux::new().unpackage(&ts).expect("demux france2.ts");
    assert_eq!(
        media.tracks.len(),
        6,
        "every PMT-listed ES becomes a track (issue #576), 6 expected"
    );

    let mut seen_pids: Vec<u16> = Vec::new();
    for track in &media.tracks {
        let pid = track
            .spec
            .source_pid
            .expect("every TS-demuxed track must carry its source PID (issue #582)");
        let (_, expected_descriptors) = expected
            .get(&pid)
            .unwrap_or_else(|| panic!("track pid {pid:#06x} not among the PMT's ES entries"));
        assert_eq!(
            &track.spec.es_info_descriptors, expected_descriptors,
            "es_info_descriptors for PID {pid:#06x} must equal the PMT ES_info bytes verbatim"
        );
        seen_pids.push(pid);
    }
    seen_pids.sort_unstable();
    assert_eq!(
        seen_pids, want_pids,
        "all 6 PIDs must appear as some track's source_pid"
    );
}

// ---------------------------------------------------------------------------
// Test 2 — labels recoverable by walking the descriptor bytes inline.
// ---------------------------------------------------------------------------

#[test]
fn audio_language_and_subtitle_descriptors_recoverable() {
    let ts = fixture_bytes();
    let media = TsDemux::new().unpackage(&ts).expect("demux france2.ts");

    let track_for_pid = |pid: u16| {
        media
            .tracks
            .iter()
            .find(|t| t.spec.source_pid == Some(pid))
            .unwrap_or_else(|| panic!("no track for PID {pid:#06x}"))
    };

    let mut languages = std::collections::BTreeSet::new();
    for &pid in &EXPECTED_AUDIO_PIDS {
        let track = track_for_pid(pid);
        let lang_desc = find_descriptor(&track.spec.es_info_descriptors, DESC_TAG_ISO_639_LANGUAGE)
            .unwrap_or_else(|| {
                panic!("PID {pid:#06x} es_info_descriptors must carry an ISO-639 language descriptor (tag 0x0A)")
            });
        assert!(
            lang_desc.len() >= 3,
            "ISO_639_language_descriptor body must carry at least a 3-byte language code"
        );
        let lang = std::str::from_utf8(&lang_desc[..3])
            .expect("language code must be ASCII")
            .to_string();
        languages.insert(lang);
    }
    let expected_languages: std::collections::BTreeSet<String> = ["fre", "qad", "qaa"]
        .iter()
        .map(|s| s.to_string())
        .collect();
    assert_eq!(
        languages, expected_languages,
        "the 3 audio tracks' ISO-639 languages must be fre/qad/qaa"
    );

    for &pid in &EXPECTED_SUBTITLE_PIDS {
        let track = track_for_pid(pid);
        assert!(
            find_descriptor(&track.spec.es_info_descriptors, DESC_TAG_DVB_SUBTITLING).is_some(),
            "PID {pid:#06x} es_info_descriptors must carry a DVB subtitling descriptor (tag 0x59)"
        );
    }
}

// ---------------------------------------------------------------------------
// Test 3 — streaming parity: StreamingTsDemux::TrackAdded carries the same
// source_pid/es_info_descriptors as the batch TsDemux path.
// ---------------------------------------------------------------------------

#[test]
fn streaming_demux_track_added_matches_batch_provenance() {
    let ts = fixture_bytes();

    let mut demux = StreamingTsDemux::new();
    demux.feed(&ts);
    demux.finish();

    let mut streamed: BTreeMap<u32, (Option<u16>, Vec<u8>)> = BTreeMap::new();
    while let Some(event) = demux.poll_event() {
        if let DemuxEvent::TrackAdded(track) = event {
            streamed.insert(
                track.spec.track_id,
                (
                    track.spec.source_pid,
                    track.spec.es_info_descriptors.clone(),
                ),
            );
        }
    }
    assert_eq!(streamed.len(), 6, "6 TrackAdded events expected");

    let batch = TsDemux::new().unpackage(&ts).expect("demux france2.ts");
    assert_eq!(batch.tracks.len(), streamed.len());

    for track in &batch.tracks {
        let (streamed_pid, streamed_descriptors) =
            streamed.get(&track.spec.track_id).unwrap_or_else(|| {
                panic!(
                    "no streamed TrackAdded for track_id {}",
                    track.spec.track_id
                )
            });
        assert_eq!(
            *streamed_pid, track.spec.source_pid,
            "streaming source_pid must match the batch path for track_id {}",
            track.spec.track_id
        );
        assert_eq!(
            streamed_descriptors, &track.spec.es_info_descriptors,
            "streaming es_info_descriptors must match the batch path for track_id {}",
            track.spec.track_id
        );
    }
}

// ---------------------------------------------------------------------------
// Test 4 — non-TS source (fMP4 round-trip) leaves source_pid = None and
// es_info_descriptors empty.
// ---------------------------------------------------------------------------

#[test]
fn fmp4_round_trip_has_no_ts_provenance() {
    let ts = fixture_bytes();
    let demuxed = TsDemux::new().unpackage(&ts).expect("demux france2.ts");

    let video_track = demuxed
        .tracks
        .iter()
        .find(|t| matches!(t.spec.config, CodecConfig::Avc { .. }))
        .expect("an AVC video track");
    assert_eq!(
        video_track.spec.source_pid,
        Some(EXPECTED_VIDEO_PID),
        "sanity: TS-demuxed video track carries its source PID"
    );

    let video_only = Media::new(vec![video_track.clone()], video_track.spec.timescale);
    let mut mux = CmafMux::new(1);
    let fmp4 = mux
        .package(&video_only)
        .expect("mux video-only track to CMAF");

    let redemuxed = Fmp4Demux::new()
        .unpackage(&fmp4)
        .expect("re-demux the fMP4 output");
    assert_eq!(redemuxed.tracks.len(), 1, "one video track in the fMP4");
    let fmp4_track = &redemuxed.tracks[0];
    assert_eq!(
        fmp4_track.spec.source_pid, None,
        "fMP4 has no PMT — source_pid must be None for a non-TS source"
    );
    assert!(
        fmp4_track.spec.es_info_descriptors.is_empty(),
        "fMP4 has no PMT — es_info_descriptors must be empty for a non-TS source"
    );
}
