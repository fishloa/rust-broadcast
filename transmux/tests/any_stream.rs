//! Gate for issue #576: lossless carriage of **any** MPEG-2 TS elementary
//! stream through demux → IR → mux — TS, classic TS-HLS, and the fMP4/CMAF
//! mux's graceful skip of what it cannot carry.
//!
//! Fixture: the real DVB capture `fixtures/ts/m6-single.ts` (PMT PID
//! `0x0064`). Its PMT lists (across the several distinct PMT section
//! contents the capture actually carries — see `tests/ir_timing.rs`'s module
//! docs for the same caveat, and note below: even the SAME PID's ES_info
//! descriptors flip between two byte-for-byte variants across repeats):
//! PES-carried `0x78` (H.264, `0x1B`) and `0x82`/`0x83`/`0x84` (audio,
//! `0x06`) and `0x8C`/`0x8D`/`0x96`/`0x97` (DVB subtitles, `0x06`);
//! section-carried `0xAA` (`0x05` private_sections), `0xAB` (`0x0B` DSM-CC),
//! `0xAC` (`0x0C` DSM-CC). Only 6 of those PIDs ever carry a *complete*
//! reassembled unit in this short excerpt — `0x82`, `0x83`, `0x84`, `0x8C`,
//! `0xAA`, `0xAB` — a PID needs at least one `PUSI=1` packet to start a PES
//! or section at all (ISO/IEC 13818-1 §2.4.3.2/§2.4.4), and the H.264 PID
//! (`0x78`, zero packets at all), the other subtitle PIDs, and `0xAC` (4
//! packets, but none `PUSI=1` — a truncated tail with no section ever
//! started) never get one in this excerpt (a pre-existing, already-
//! documented property of this fixture, see `tests/ir_timing.rs` /
//! `tests/streaming_demux.rs`), so **no video track is ever produced from
//! this fixture alone**; test 4 below combines it with a second real
//! fixture that does carry video.
//!
//! On the PMT-content-flips-per-repeat oddity: `0x82`/`0x83`/`0x84` carry
//! TWO distinct raw ES_info byte strings across the capture's repeated PMT
//! sections (with vs. without a leading `STREAM_IDENTIFIER` descriptor;
//! confirmed independently in this file, not assumed). Which one a given
//! demuxer instance latches onto depends on exactly which PMT repeat it
//! first has a resolved reassembler for (`TsDemux`'s PMT reassembler itself
//! only exists from the point the PAT's first section resolves — an earlier
//! PMT repeat that races the PAT is silently unattributed, same as any other
//! not-yet-classified PID). So tests 1/2 below accept **either** raw variant
//! actually observed on the wire for a PID as proof the descriptors are
//! genuine (never fabricated, never the crate's own value assumed correct);
//! tests 3/5 (round-trip) instead pin against whatever the direct demux
//! actually captured, which sidesteps the ambiguity entirely.
//!
//! Oracle: every PMT/section assertion is checked against an independent
//! walk of the raw TS bytes in this file ([`mpeg_ts::ts::SectionReassembler`]
//! plus a hand-rolled PMT-body walk) — never the crate's own PMT parser
//! under test (mirrors `tests/ir_timing.rs`'s `collect_pmt_es`).

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::path::PathBuf;

use broadcast_common::{Package, Unpackage};
use mpeg_ts::ts::{SectionReassembler, TsPacket};

use transmux::pipeline::{CodecConfig, DataCarriage};
use transmux::{CmafMux, Fmp4Demux, Media, Track, TsDemux, TsHlsPackager, TsMux};

const TS: usize = 188;

// ── Fixture loading ─────────────────────────────────────────────────────────

fn fixtures_ts_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../fixtures/ts")
}

fn read_fixture(name: &str) -> Vec<u8> {
    let path = fixtures_ts_dir().join(name);
    std::fs::read(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()))
}

fn demux(data: &[u8]) -> Media {
    TsDemux::new()
        .unpackage(data)
        .expect("TS demux must succeed")
}

// ── Independent oracle: PAT/PMT section walk (not the crate's own parser) ──

/// Every PID that carries at least one `PUSI=1` packet in `data` — the
/// independent, generalisable proxy for "this PID could ever complete a PES
/// packet or PSI/private section" (both require a `PUSI=1` packet to even
/// start, ISO/IEC 13818-1 §2.4.3.2/§2.4.4): a PID with packets but no
/// `PUSI=1` (e.g. this fixture's `0xAC`, a truncated tail) can never
/// reassemble anything, and a PID that never appears on the wire at all
/// (e.g. this fixture's H.264 PID) trivially has none either.
fn pusi_seen_pids(data: &[u8]) -> HashMap<u16, usize> {
    let mut counts = HashMap::new();
    for chunk in data.chunks_exact(TS) {
        let Ok(pkt) = TsPacket::parse(chunk) else {
            continue;
        };
        if pkt.header.pusi {
            *counts.entry(pkt.header.pid).or_insert(0usize) += 1;
        }
    }
    counts
}

/// The PMT PID for the (single) program listed in `data`'s PAT, found by an
/// independent PAT walk (never the crate's own `TsDemux`/`TsMux` internals).
fn find_pmt_pid(data: &[u8]) -> u16 {
    const PAT_PID: u16 = 0x0000;
    const TABLE_ID_PAT: u8 = 0x00;
    let mut reasm = SectionReassembler::default();
    for chunk in data.chunks_exact(TS) {
        let Ok(pkt) = TsPacket::parse(chunk) else {
            continue;
        };
        if pkt.header.pid != PAT_PID {
            continue;
        }
        let Some(payload) = pkt.payload else {
            continue;
        };
        reasm.feed(payload, pkt.header.pusi);
        while let Some(section) = reasm.pop_section() {
            if section.first().copied() != Some(TABLE_ID_PAT) || section.len() < 12 {
                continue;
            }
            let section_length = (((section[1] & 0x0F) as usize) << 8) | section[2] as usize;
            let end = (3 + section_length).min(section.len());
            if end < 12 {
                continue;
            }
            let body = &section[8..end - 4];
            let mut off = 0usize;
            while off + 4 <= body.len() {
                let program_number = u16::from_be_bytes([body[off], body[off + 1]]);
                let pid = (((body[off + 2] & 0x1F) as u16) << 8) | body[off + 3] as u16;
                if program_number != 0 {
                    return pid;
                }
                off += 4;
            }
        }
    }
    panic!("no program_map_PID found via an independent PAT walk");
}

/// `(stream_type, elementary_pid, ES_info descriptor bytes)` for every ES
/// loop entry across every distinct PMT section seen on `pmt_pid` in `data`
/// (ISO/IEC 13818-1 §2.4.4.8) — written from scratch here, not reusing the
/// crate's own PMT parser (mirrors `tests/ir_timing.rs`'s `collect_pmt_es`).
fn collect_pmt_es(data: &[u8], pmt_pid: u16) -> Vec<(u8, u16, Vec<u8>)> {
    const TABLE_ID_PMT: u8 = 0x02;
    let mut reasm = SectionReassembler::default();
    let mut out = Vec::new();
    for chunk in data.chunks_exact(TS) {
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
            if section.first().copied() != Some(TABLE_ID_PMT) || section.len() < 12 {
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

/// One PMT-listed elementary stream that actually starts a PES/section on
/// the wire, with every distinct raw ES_info byte-string observed for it
/// across every PMT repeat in the capture (see the module docs' note on the
/// PMT-content-flips-per-repeat oddity in `0x82`/`0x83`/`0x84`).
struct LiveEs {
    pid: u16,
    stream_type: u8,
    descriptor_variants: BTreeSet<Vec<u8>>,
}

/// Every PMT-listed elementary stream in `data` that ever carries at least
/// one `PUSI=1` packet (see [`pusi_seen_pids`]), each with the full set of
/// distinct ES_info byte-strings genuinely observed for it on the wire.
fn live_pmt_es(data: &[u8]) -> Vec<LiveEs> {
    let pmt_pid = find_pmt_pid(data);
    let pusi_counts = pusi_seen_pids(data);
    let mut by_pid: BTreeMap<u16, (u8, BTreeSet<Vec<u8>>)> = BTreeMap::new();
    for (stream_type, pid, descriptors) in collect_pmt_es(data, pmt_pid) {
        if pusi_counts.get(&pid).copied().unwrap_or(0) == 0 {
            continue; // PMT-listed but never starts a PES/section on the wire
        }
        let entry = by_pid.entry(pid).or_insert((stream_type, BTreeSet::new()));
        assert_eq!(
            entry.0, stream_type,
            "PID {pid:#06x} must not change stream_type across PMT repeats"
        );
        entry.1.insert(descriptors);
    }
    by_pid
        .into_iter()
        .map(|(pid, (stream_type, descriptor_variants))| LiveEs {
            pid,
            stream_type,
            descriptor_variants,
        })
        .collect()
}

/// Section-carried `stream_type`s (ISO/IEC 13818-1 Table 2-34) — mirrors
/// `transmux::ts_demux`'s private `data_carriage`, reimplemented from the
/// spec table rather than imported, so the test is a genuine second opinion.
fn expected_carriage(stream_type: u8) -> DataCarriage {
    match stream_type {
        0x05 | 0x0A | 0x0B | 0x0C | 0x0D | 0x14 | 0x86 => DataCarriage::Sections,
        _ => DataCarriage::Pes,
    }
}

/// True if `bytes` is a structurally valid long-form PSI/private section:
/// enough bytes for the 3-byte header, and `section_length` (bytes[1..3])
/// accounts for exactly the rest of `bytes` (ISO/IEC 13818-1 §2.4.4.1).
fn is_valid_long_form_section(bytes: &[u8]) -> bool {
    if bytes.len() < 3 {
        return false;
    }
    let section_length = (((bytes[1] & 0x0F) as usize) << 8) | bytes[2] as usize;
    3 + section_length == bytes.len()
}

/// Find the `CodecConfig::Data` track in `media` matching `es`'s
/// `stream_type` and ANY of its independently-observed `descriptor_variants`
/// (see [`LiveEs`] / the module docs' PMT-flips-per-repeat note) — proving
/// the track's descriptors are genuine PMT ES_info bytes, without assuming
/// which specific repeat the demuxer happened to latch onto.
fn find_data_track<'a>(media: &'a Media, es: &LiveEs) -> &'a Track {
    media
        .tracks
        .iter()
        .find(|t| match &t.spec.config {
            CodecConfig::Data {
                stream_type,
                descriptors,
                ..
            } => *stream_type == es.stream_type && es.descriptor_variants.contains(descriptors),
            _ => false,
        })
        .unwrap_or_else(|| {
            panic!(
                "PID {:#06x}: no Data track for stream_type {:#04X} matching any \
                 of the {} observed ES_info variants",
                es.pid,
                es.stream_type,
                es.descriptor_variants.len()
            )
        })
}

/// Find the `CodecConfig::Data` track in `media` with this EXACT
/// `(stream_type, descriptors)` pair — used for round-trip checks, where the
/// expected value is whatever a prior demux actually captured (no PMT-repeat
/// ambiguity to tolerate).
fn find_data_track_exact<'a>(media: &'a Media, stream_type: u8, descriptors: &[u8]) -> &'a Track {
    media
        .tracks
        .iter()
        .find(|t| match &t.spec.config {
            CodecConfig::Data {
                stream_type: st,
                descriptors: d,
                ..
            } => *st == stream_type && d.as_slice() == descriptors,
            _ => false,
        })
        .unwrap_or_else(|| {
            panic!("no Data track for stream_type {stream_type:#04X} with the exact descriptors")
        })
}

// ── Test 1 — demux completeness: every live PMT stream becomes a track ─────

#[test]
fn demux_completeness_every_live_pmt_stream_becomes_a_track() {
    let data = read_fixture("m6-single.ts");
    let media = demux(&data);
    let live = live_pmt_es(&data);

    // This fixture's own reality (see module docs): exactly 6 PMT-listed
    // PIDs ever start a PES/section, none of them the H.264 video PID —
    // pinned here so a change to the fixture or the demuxer's classification
    // is caught.
    assert_eq!(
        live.len(),
        6,
        "m6-single.ts must have exactly 6 live PMT-listed PIDs"
    );
    assert_eq!(
        media.tracks.len(),
        6,
        "every live PMT stream must become exactly one track, got {:?}",
        media
            .tracks
            .iter()
            .map(|t| &t.spec.config)
            .collect::<Vec<_>>()
    );

    let mut n_pes = 0usize;
    let mut n_sections = 0usize;
    for es in &live {
        let track = find_data_track(&media, es);
        let (carriage, descriptors) = match &track.spec.config {
            CodecConfig::Data {
                carriage,
                descriptors,
                ..
            } => (*carriage, descriptors),
            other => panic!("PID {:#06x} must be a Data track, got {other:?}", es.pid),
        };
        assert!(
            !descriptors.is_empty() || es.descriptor_variants.contains(&Vec::new()),
            "PID {:#06x}: descriptors must equal the (non-empty, in this fixture) \
             PMT ES_info bytes",
            es.pid
        );
        assert_eq!(
            carriage,
            expected_carriage(es.stream_type),
            "PID {:#06x} (stream_type {:#04X}) carriage classification",
            es.pid,
            es.stream_type
        );
        match carriage {
            DataCarriage::Pes => n_pes += 1,
            DataCarriage::Sections => n_sections += 1,
            _ => {}
        }
    }
    assert_eq!(
        n_pes, 4,
        "expected 4 PES-carried Data tracks (0x82/0x83/0x84 audio + 0x8C subtitle)"
    );
    assert_eq!(
        n_sections, 2,
        "expected 2 section-carried Data tracks (0xAA/0xAB — 0xAC never starts a \
         section in this excerpt, see module docs)"
    );
}

// ── Test 2 — section tracks actually carry reassembled sections ────────────

#[test]
fn section_tracks_carry_valid_reassembled_sections() {
    let data = read_fixture("m6-single.ts");
    let media = demux(&data);
    let live = live_pmt_es(&data);

    let mut checked = 0usize;
    for es in &live {
        if expected_carriage(es.stream_type) != DataCarriage::Sections {
            continue;
        }
        let track = find_data_track(&media, es);
        assert!(
            !track.samples.is_empty(),
            "section-carried stream_type {:#04X} must have >= 1 sample",
            es.stream_type
        );
        for (i, sample) in track.samples.iter().enumerate() {
            assert!(
                is_valid_long_form_section(&sample.data),
                "stream_type {:#04X} sample {i} is not a structurally valid \
                 long-form section (len {}), proving it was NOT reassembled",
                es.stream_type,
                sample.data.len()
            );
            assert!(
                sample.source_timing.is_none(),
                "a section sample must carry no PTS/DTS (source_timing: None)"
            );
        }
        checked += 1;
    }
    assert_eq!(checked, 2, "expected to check both section-carried tracks");
}

// ── Test 3 — TS -> IR -> TS payload round-trip (data + section tracks) ─────

#[test]
fn ts_ir_ts_round_trip_is_payload_lossless_for_data_and_sections() {
    let data = read_fixture("m6-single.ts");
    let media = demux(&data);

    // Re-mux the whole IR (every track here is `CodecConfig::Data`, PES or
    // section — issue #576 means the TS muxer can now carry ALL of them).
    let ts2 = TsMux::new()
        .package(&media)
        .expect("TsMux must carry every Data track (PES and section), not error");
    let media2 = demux(&ts2);
    assert_eq!(
        media2.tracks.len(),
        media.tracks.len(),
        "re-demux must recover the same number of tracks"
    );

    // The re-emitted PMT must carry each track's exact preserved stream_type
    // + descriptors (parsed independently here, not via the crate's PMT
    // parser) — the round-trip pins against what the IR actually holds, not
    // the raw fixture's PMT-repeat ambiguity (see module docs).
    let out_pmt_pid = find_pmt_pid(&ts2);
    let out_es = collect_pmt_es(&ts2, out_pmt_pid);
    for track in &media.tracks {
        let CodecConfig::Data {
            stream_type,
            descriptors,
            ..
        } = &track.spec.config
        else {
            panic!("m6-single.ts must produce only Data tracks in this excerpt");
        };
        assert!(
            out_es
                .iter()
                .any(|(st, _pid, d)| st == stream_type && d == descriptors),
            "re-emitted PMT must list stream_type {stream_type:#04X} with its \
             preserved ES_info descriptors"
        );

        // Every original track's sample payloads must survive byte-for-byte
        // (payload-lossless — NOT packet-identical: PIDs/PSI/PES framing
        // legitimately differ between the two TS byte streams).
        let round = find_data_track_exact(&media2, *stream_type, descriptors);
        let orig_payloads: Vec<&[u8]> = track.samples.iter().map(|s| s.data.as_slice()).collect();
        let round_payloads: Vec<&[u8]> = round.samples.iter().map(|s| s.data.as_slice()).collect();
        assert_eq!(
            orig_payloads, round_payloads,
            "stream_type {stream_type:#04X}: sample payloads must round-trip byte-for-byte"
        );
    }
}

// ── Test 4 — TS -> fMP4 succeeds, skipping Data, keeping real A/V ──────────

#[test]
fn ts_to_fmp4_skips_data_tracks_and_keeps_video_audio() {
    // `m6-single.ts` alone carries no video in this excerpt (see module
    // docs), so it cannot demonstrate "video survives" on its own. Combine
    // it with a second real, committed fixture (`h264_aac.ts`, a genuine
    // decoded H.264+AAC capture) that does — both halves are real captured
    // bytes, never hand-built/fabricated.
    let av_media = demux(&read_fixture("h264_aac.ts"));
    assert!(
        av_media
            .tracks
            .iter()
            .any(|t| matches!(t.spec.config, CodecConfig::Avc { .. }))
    );
    assert!(
        av_media
            .tracks
            .iter()
            .any(|t| matches!(t.spec.config, CodecConfig::Aac { .. }))
    );

    let data_source = read_fixture("m6-single.ts");
    let data_media = demux(&data_source);
    let live = live_pmt_es(&data_source);
    let section_es = live
        .iter()
        .find(|es| expected_carriage(es.stream_type) == DataCarriage::Sections)
        .expect("m6-single.ts must have a section-carried Data track");
    let mut data_track = find_data_track(&data_media, section_es).clone();

    let mut tracks = av_media.tracks.clone();
    data_track.spec.track_id = tracks.iter().map(|t| t.spec.track_id).max().unwrap_or(0) + 1;
    tracks.push(data_track);
    let mixed = Media::new(tracks, av_media.movie_timescale);
    assert_eq!(
        mixed.tracks.len(),
        3,
        "video + audio + one opaque Data track"
    );

    // The headline assertion: this must SUCCEED (no `UnsupportedCodec`),
    // silently omitting the Data track.
    let out = CmafMux::default()
        .package(&mixed)
        .expect("CmafMux must skip the opaque Data track rather than erroring");

    let reparsed: Media = Fmp4Demux::new()
        .unpackage(&out)
        .expect("re-parse the fMP4 output");
    assert_eq!(
        reparsed.tracks.len(),
        2,
        "the Data track must be omitted; only video+audio survive, got {:?}",
        reparsed
            .tracks
            .iter()
            .map(|t| &t.spec.config)
            .collect::<Vec<_>>()
    );
    assert!(
        reparsed
            .tracks
            .iter()
            .any(|t| matches!(t.spec.config, CodecConfig::Avc { .. })),
        "the video track must survive"
    );
    assert!(
        reparsed
            .tracks
            .iter()
            .any(|t| matches!(t.spec.config, CodecConfig::Aac { .. })),
        "the audio track must survive"
    );
    assert!(
        !reparsed
            .tracks
            .iter()
            .any(|t| matches!(t.spec.config, CodecConfig::Data { .. })),
        "no Data track may survive into the fMP4 output"
    );
}

// ── Test 5 (headline) — TS-HLS carries every data + section stream ─────────

#[test]
fn ts_hls_carries_every_data_and_section_track_in_every_segment_pmt() {
    let data = read_fixture("m6-single.ts");
    let media = demux(&data);
    let live = live_pmt_es(&data);
    assert_eq!(live.len(), 6, "sanity: 6 live PMT-listed PIDs (see test 1)");

    let out = TsHlsPackager::new(1)
        .package(&media)
        .expect("TS-HLS packaging must carry every Data/section track, not error");
    assert!(
        !out.segments.is_empty(),
        "must produce at least one segment"
    );
    assert!(out.playlist.starts_with("#EXTM3U"));

    // Pin the exact (stream_type, descriptors) each track's config carries in
    // the IR — the segment PMT check pins against what the IR actually
    // holds, not the raw fixture's PMT-repeat ambiguity (see module docs).
    let track_ids: Vec<(u8, Vec<u8>)> = media
        .tracks
        .iter()
        .map(|t| match &t.spec.config {
            CodecConfig::Data {
                stream_type,
                descriptors,
                ..
            } => (*stream_type, descriptors.clone()),
            other => panic!("m6-single.ts must produce only Data tracks, got {other:?}"),
        })
        .collect();

    // Every generated segment's PMT must list EVERY live elementary stream —
    // a receiver joining any segment must find every PID's PSI there, not
    // just the ones with samples in that particular segment (ISO/IEC
    // 13818-1 §2.4.4 PSI repetition).
    for (i, seg) in out.segments.iter().enumerate() {
        assert_eq!(seg.len() % TS, 0, "segment {i} must be whole TS packets");
        let seg_pmt_pid = find_pmt_pid(seg);
        let seg_es = collect_pmt_es(seg, seg_pmt_pid);
        for (stream_type, descriptors) in &track_ids {
            assert!(
                seg_es
                    .iter()
                    .any(|(st, _pid, d)| st == stream_type && d == descriptors),
                "segment {i}'s PMT must list stream_type {stream_type:#04X} \
                 with its preserved ES_info descriptors, got {seg_es:?}"
            );
        }
    }

    // Concatenating the segments and re-demuxing must reproduce every
    // data/section track's sample payloads byte-for-byte (payload-lossless
    // through segmentation), matching the direct single-shot demux.
    let mut concat = Vec::new();
    for seg in &out.segments {
        concat.extend_from_slice(seg);
    }
    let media2 = demux(&concat);

    for (stream_type, descriptors) in &track_ids {
        let orig = find_data_track_exact(&media, *stream_type, descriptors);
        let round = find_data_track_exact(&media2, *stream_type, descriptors);
        let orig_payloads: Vec<&[u8]> = orig.samples.iter().map(|s| s.data.as_slice()).collect();
        let round_payloads: Vec<&[u8]> = round.samples.iter().map(|s| s.data.as_slice()).collect();
        assert_eq!(
            orig_payloads, round_payloads,
            "stream_type {stream_type:#04X}: payload-lossless through TS-HLS segmentation"
        );
    }
}
