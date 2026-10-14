//! Audio frames that straddle PES boundaries (issue #1160): MPEG audio, ADTS,
//! AC-3, E-AC-3 and DTS core.
//!
//! A muxer that does not align audio PES to frames (Rai's does not) cuts a
//! frame across two PES packets; ISO/IEC 13818-1 §2.4.3.7 stamps a PES with the
//! first access unit that *commences* in it. The demuxer must carry the partial
//! tail into the next PES of the same PID, for every audio family.
//!
//! PROVENANCE. Every input is a committed fixture, none is hand-made bytes:
//!
//! * `fixtures/audio_pes_straddle/sintel-ac3-eac3-aligned.ts` — the AC-3 5.1 and
//!   E-AC-3 5.1 tracks of a 10 s excerpt of the Sintel soundtrack
//!   (CC BY 3.0, see `ATTRIBUTION.txt` beside it), encoded and muxed with
//!   ffmpeg, trimmed to ~3 s of each (94 AC-3 PES of one syncframe, 47 E-AC-3 PES of two). Frame-aligned PES.
//! * `fixtures/ts/h264_aac.ts`, `fixtures/ts/legacy/mpeg2_mp2.ts`,
//!   `fixtures/ts/dts/dts_core.ts` — the workspace's existing ffmpeg-generated
//!   AAC, MP2 and DTS-core streams.
//!
//! The straddling variant of each is produced **in this test** by
//! [`straddle_audio_pes`], a port of zenith-tv's `ts-misalign-audio-pes.py`:
//! every audio PES gives its last `tail` bytes to the next PES on the same PID
//! (PTS values kept: the moved bytes are a continuation, not a frame start).
//! The oracle is the aligned stream itself plus an independent byte-level walk
//! of its reassembled elementary stream.

#![cfg(feature = "std")]

use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;
use transmux::TrackId;

use broadcast_common::Unpackage;
use transmux::{DemuxEvent, StreamingTsDemux, TsDemux};

const TS: usize = 188;
/// Bytes each audio PES hands to the next one (zenith-tv's default).
const TAIL: usize = 100;

fn fixture(rel: &str) -> Vec<u8> {
    test_fixtures::read(rel)
}

fn local_fixture(rel: &str) -> Vec<u8> {
    let p = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/audio_pes_straddle")
        .join(rel);
    std::fs::read(&p).unwrap_or_else(|e| panic!("read fixture {}: {e}", p.display()))
}

// ── TS / PES plumbing (independent of the crate under test) ─────────────────

fn pid_of(p: &[u8]) -> u16 {
    (u16::from(p[1] & 0x1F) << 8) | u16::from(p[2])
}

fn payload_of(p: &[u8]) -> &[u8] {
    match (p[3] >> 4) & 3 {
        0 | 2 => &[],
        3 => &p[(5 + p[4] as usize).min(TS)..],
        _ => &p[4..],
    }
}

/// PES `stream_id`s of audio: MPEG audio `110x xxxx` and `private_stream_1`
/// (AC-3/E-AC-3/DTS in DVB) — ISO/IEC 13818-1 Table 2-22.
fn is_audio_stream_id(sid: u8) -> bool {
    (0xC0..0xE0).contains(&sid) || sid == 0xBD
}

/// `(header, payload)` of one whole PES packet.
fn split_pes(pes: &[u8]) -> (&[u8], &[u8]) {
    let hlen = 9 + pes[8] as usize;
    pes.split_at(hlen)
}

/// Packetise one PES onto `pid`, stuffing the last packet with an adaptation
/// field (never trailing payload bytes).
fn ts_packets(pid: u16, pes: &[u8], cc: &mut u8) -> Vec<[u8; TS]> {
    let mut out = Vec::new();
    let mut off = 0usize;
    let mut first = true;
    while off < pes.len() {
        let take = (pes.len() - off).min(184);
        let pad = 184 - take;
        let mut p = [0u8; TS];
        p[0] = 0x47;
        p[1] = (if first { 0x40 } else { 0 }) | (pid >> 8) as u8;
        p[2] = (pid & 0xFF) as u8;
        let mut at = 4;
        if pad == 0 {
            p[3] = 0x10 | *cc;
        } else {
            p[3] = 0x30 | *cc;
            if pad == 1 {
                p[4] = 0; // adaptation_field_length = 0
                at = 5;
            } else {
                p[4] = (pad - 1) as u8;
                p[5] = 0;
                p[6..4 + pad].fill(0xFF);
                at = 4 + pad;
            }
        }
        p[at..at + take].copy_from_slice(&pes[off..off + take]);
        *cc = (*cc + 1) & 0xF;
        out.push(p);
        off += take;
        first = false;
    }
    out
}

/// Re-packetise every audio PES of `ts` so each gives its last `tail` bytes to
/// the next PES of the same PID (port of zenith-tv `ts-misalign-audio-pes.py`).
fn straddle_audio_pes(ts: &[u8], tail: usize) -> Vec<u8> {
    enum Item {
        Pkt(Vec<u8>),
        Pes(u16, usize),
    }
    let mut order: Vec<Item> = Vec::new();
    let mut pes: BTreeMap<u16, Vec<Vec<u8>>> = BTreeMap::new();
    let mut audio: BTreeSet<u16> = BTreeSet::new();
    let mut collecting: BTreeSet<u16> = BTreeSet::new();
    for p in ts.chunks_exact(TS).filter(|p| p[0] == 0x47) {
        let pid = pid_of(p);
        let pusi = p[1] & 0x40 != 0;
        let pl = payload_of(p);
        if pusi && pl.starts_with(&[0, 0, 1]) {
            if is_audio_stream_id(pl[3]) {
                audio.insert(pid);
            }
            if audio.contains(&pid) {
                let list = pes.entry(pid).or_default();
                list.push(pl.to_vec());
                order.push(Item::Pes(pid, list.len() - 1));
                collecting.insert(pid);
                continue;
            }
        }
        if audio.contains(&pid) && collecting.contains(&pid) {
            pes.get_mut(&pid)
                .unwrap()
                .last_mut()
                .unwrap()
                .extend_from_slice(pl);
            continue;
        }
        order.push(Item::Pkt(p.to_vec()));
    }
    for list in pes.values_mut() {
        for i in 0..list.len().saturating_sub(1) {
            let (h, body) = {
                let (h, b) = split_pes(&list[i]);
                (h.to_vec(), b.to_vec())
            };
            let (nh, nbody) = {
                let (h, b) = split_pes(&list[i + 1]);
                (h.to_vec(), b.to_vec())
            };
            let k = tail.min(body.len() - 1);
            let keep = body.len() - k;
            list[i] = [h.as_slice(), &body[..keep]].concat();
            list[i + 1] = [nh.as_slice(), &body[keep..], &nbody].concat();
        }
        for b in list.iter_mut() {
            let plen = b.len() - 6;
            let plen = if plen <= 0xFFFF { plen } else { 0 };
            b[4..6].copy_from_slice(&(plen as u16).to_be_bytes());
        }
    }
    let mut cc: BTreeMap<u16, u8> = pes.keys().map(|&k| (k, 0)).collect();
    let mut out = Vec::new();
    for item in order {
        match item {
            Item::Pkt(p) => out.extend_from_slice(&p),
            Item::Pes(pid, i) => {
                for p in ts_packets(pid, &pes[&pid][i], cc.get_mut(&pid).unwrap()) {
                    out.extend_from_slice(&p);
                }
            }
        }
    }
    out
}

/// The elementary stream of `pid`: its PES payloads concatenated.
fn es_of(ts: &[u8], pid: u16) -> Vec<u8> {
    let mut pes: Vec<Vec<u8>> = Vec::new();
    for p in ts.chunks_exact(TS).filter(|p| pid_of(p) == pid) {
        let pl = payload_of(p);
        if p[1] & 0x40 != 0 {
            pes.push(pl.to_vec());
        } else if let Some(l) = pes.last_mut() {
            l.extend_from_slice(pl);
        }
    }
    pes.iter().flat_map(|b| split_pes(b).1.to_vec()).collect()
}

// ── Observation ──────────────────────────────────────────────────────────────

type Sample = (Vec<u8>, Option<i64>, Option<i64>, Option<u32>);

/// Samples per track id, driving [`StreamingTsDemux`] in `chunk`-byte feeds.
fn observe(ts: &[u8], chunk: usize) -> BTreeMap<TrackId, Vec<Sample>> {
    let mut d = StreamingTsDemux::new();
    let mut out: BTreeMap<TrackId, Vec<Sample>> = BTreeMap::new();
    let mut drain = |d: &mut StreamingTsDemux| {
        while let Some(e) = d.poll_event() {
            if let DemuxEvent::Sample {
                track_id, sample, ..
            } = e
            {
                out.entry(TrackId::new(track_id.value()))
                    .or_default()
                    .push((
                        sample.data.to_vec(),
                        sample.dts,
                        sample.pts,
                        sample.duration,
                    ));
            }
        }
    };
    for c in ts.chunks(chunk) {
        d.feed(c);
        drain(&mut d);
    }
    d.finish();
    drain(&mut d);
    out
}

/// Batch demux, as the same per-track sample lists.
fn batch(ts: &[u8]) -> BTreeMap<TrackId, Vec<Sample>> {
    let media = TsDemux::new().unpackage(ts).expect("batch demux");
    media
        .tracks
        .iter()
        .map(|t| {
            (
                t.spec.track_id,
                t.samples
                    .iter()
                    .map(|s| (s.data.to_vec(), s.dts, s.pts, s.duration))
                    .collect(),
            )
        })
        .collect()
}

/// Feed sizes swept for the streaming-vs-batch identity.
const CHUNKS: &[usize] = &[
    1, 2, 3, 5, 7, 11, 13, 31, 64, 100, 187, 188, 189, 376, 500, 997, 1000,
];

/// Aligned and straddling streams must demux to the *same* samples per track —
/// bytes, timing and count — through the batch demuxer and the streaming one
/// at every feed size.
fn assert_straddle_invariant(label: &str, aligned: &[u8]) -> BTreeMap<TrackId, Vec<Sample>> {
    let split = straddle_audio_pes(aligned, TAIL);
    assert_ne!(aligned, split.as_slice(), "{label}: the split must differ");
    let want = batch(aligned);
    assert!(!want.is_empty(), "{label}: aligned stream has tracks");
    let got = batch(&split);
    assert_eq!(
        got.keys().collect::<Vec<_>>(),
        want.keys().collect::<Vec<_>>(),
        "{label}: same tracks resolved"
    );
    for (id, w) in &want {
        let g = &got[id];
        assert_eq!(g.len(), w.len(), "{label} track {id}: sample count");
        for (n, (gs, ws)) in g.iter().zip(w).enumerate() {
            assert_eq!(gs.0, ws.0, "{label} track {id}: frame {n} bytes");
        }
        // Timing: the PTS of the PES a frame *started* in is unchanged by
        // moving continuation bytes, so timing is identical too.
        assert_eq!(g, w, "{label} track {id}: samples + timing");
    }
    for &c in CHUNKS {
        assert_eq!(observe(&split, c), got, "{label}: streaming feed {c}");
    }
    got
}

// ── Tests ────────────────────────────────────────────────────────────────────

/// AC-3 5.1 + E-AC-3 5.1 (Sintel, CC BY 3.0), straddled: every syncframe of
/// both tracks recovered, identical to the aligned demux and to a byte-level
/// walk of the reassembled elementary stream.
#[test]
fn sintel_ac3_and_eac3_straddling_pes_recover_every_syncframe() {
    const AC3_PID: u16 = 258;
    const EAC3_PID: u16 = 259;
    let aligned = local_fixture("sintel-ac3-eac3-aligned.ts");
    let tracks = assert_straddle_invariant("sintel", &aligned);
    assert_eq!(tracks.len(), 2, "AC-3 + E-AC-3 tracks");

    // Oracle 1: the reassembled ES, walked by the crate's own syncframe
    // splitter over ONE contiguous buffer (no PES boundary to cross).
    let ac3_es = es_of(&aligned, AC3_PID);
    let eac3_es = es_of(&aligned, EAC3_PID);
    let ac3_frames: Vec<Vec<u8>> = transmux::ac3::split_ac3_syncframes(&ac3_es)
        .into_iter()
        .map(<[u8]>::to_vec)
        .collect();
    let eac3_frames: Vec<Vec<u8>> = transmux::ac3::split_eac3_syncframes(&eac3_es)
        .into_iter()
        .map(|f| f.data)
        .collect();
    // Oracle 2 (independent of the splitter): the ES is exactly those frames,
    // and ~3 s (94 frames) of each track were kept by the trim.
    assert_eq!(ac3_frames.iter().map(Vec::len).sum::<usize>(), ac3_es.len());
    assert_eq!(
        eac3_frames.iter().map(Vec::len).sum::<usize>(),
        eac3_es.len()
    );
    const TRIMMED_FRAMES: usize = 94; // 10 s = 313 frames; trim ~3 s
    assert_eq!(ac3_frames.len(), TRIMMED_FRAMES);
    assert_eq!(eac3_frames.len(), TRIMMED_FRAMES);

    let mut by_len: Vec<(usize, &Vec<Sample>)> = tracks.values().map(|v| (v.len(), v)).collect();
    by_len.sort_by_key(|(n, _)| *n);
    for (n, samples) in by_len {
        assert_eq!(n, TRIMMED_FRAMES, "every syncframe of both tracks");
        let es_frames = if samples[0].0 == ac3_frames[0] {
            &ac3_frames
        } else {
            &eac3_frames
        };
        for (i, s) in samples.iter().enumerate() {
            assert_eq!(&s.0, &es_frames[i], "frame {i} byte-exact vs the ES walk");
        }
    }
}

/// The straddling invariant over every other audio family the workspace has a
/// real fixture for: AAC (ADTS), MPEG-1 Layer II, DTS core.
#[test]
fn aac_mp2_and_dts_straddling_pes_match_the_aligned_demux() {
    for rel in [
        "ts/h264_aac.ts",
        "ts/legacy/mpeg2_mp2.ts",
        "ts/dts/dts_core.ts",
        "ts/aac-5_1-640k.ts",
    ] {
        let aligned = fixture(rel);
        let tracks = assert_straddle_invariant(rel, &aligned);
        assert!(
            tracks.values().any(|v| v.len() >= 30),
            "{rel}: a real audio track with plenty of frames"
        );
    }
}

/// Real capture (`m6-single.ts`, a French broadcast): its three E-AC-3 tracks
/// carry syncframes that straddle PES boundaries on the wire. Before #1160 only
/// 4/5/1 survived; every complete frame is now recovered, and each emitted
/// sample is a whole syncframe (`0x0B77`, length from its own `frmsiz`).
#[test]
fn real_capture_eac3_recovers_straddling_syncframes() {
    let ts = fixture("ts/m6-single.ts");
    let media = TsDemux::new().unpackage(ts.as_slice()).expect("demux");
    let mut counts: Vec<usize> = Vec::new();
    for t in media
        .tracks
        .iter()
        .filter(|t| matches!(t.config(), transmux::pipeline::CodecConfig::Eac3 { .. }))
    {
        counts.push(t.samples.len());
        for s in &t.samples {
            assert_eq!(s.data[..2], [0x0B, 0x77], "starts on the syncword");
            // frmsiz: bits 21..32 of the frame = 11 bits after strmtyp(2)+substreamid(3).
            let frmsiz = (usize::from(s.data[2] & 0x07) << 8) | usize::from(s.data[3]);
            assert_eq!(s.data.len(), (frmsiz + 1) * 2, "a whole syncframe");
        }
    }
    counts.sort_unstable();
    assert_eq!(counts, [3, 6, 7], "was [1, 4, 5] before the carry (#1160)");
}

/// Drop the first PES of `pid` (from its first start packet up to, but not
/// including, the next start packet), so the stream's first audio PES on that
/// PID begins mid-frame — a capture that starts in the middle of a stream.
fn drop_first_pes(ts: &[u8], pid: u16) -> Vec<u8> {
    let mut out = Vec::new();
    let mut state = 0u8; // 0 = before it, 1 = inside it, 2 = after it
    for p in ts.chunks_exact(TS) {
        if pid_of(p) == pid {
            let pusi = p[1] & 0x40 != 0;
            match (state, pusi) {
                (0, true) => {
                    state = 1;
                    continue;
                }
                (1, false) => continue,
                (1, true) => state = 2,
                _ => {}
            }
        }
        out.extend_from_slice(p);
    }
    out
}

/// Config recovery when the stream's FIRST audio PES starts mid-frame (a capture
/// joined mid-stream): the track must still resolve, by resyncing on the first
/// valid syncframe, and every frame from there on is recovered byte-exactly.
#[test]
fn first_pes_starting_mid_syncframe_still_resolves_ac3_and_eac3() {
    let aligned = local_fixture("sintel-ac3-eac3-aligned.ts");
    let want = batch(&aligned);
    let mut split = straddle_audio_pes(&aligned, TAIL);
    split = drop_first_pes(&split, 258);
    split = drop_first_pes(&split, 259);
    // The dropped first PES commenced one AC-3 syncframe / two E-AC-3
    // syncframes (the Sintel mux packs 1 / 2 per PES); the tail bytes of the
    // last of them sit at the front of the next PES, which therefore starts
    // mid-frame.
    let got = batch(&split);
    // Track ids follow PMT order: 258 (AC-3) then 259 (E-AC-3).
    let ids: Vec<TrackId> = want.keys().copied().collect();
    assert_eq!(ids.len(), 2);
    for (id, lost_frames) in [(ids[0], 1usize), (ids[1], 2usize)] {
        let g: Vec<&[u8]> = got[&id].iter().map(|s| s.0.as_slice()).collect();
        let w: Vec<&[u8]> = want[&id][lost_frames..]
            .iter()
            .map(|s| s.0.as_slice())
            .collect();
        assert_eq!(
            g, w,
            "track {id}: every frame after the lost PES, byte-exact"
        );
    }
    for &c in &[1usize, 7, 188, 1000] {
        assert_eq!(observe(&split, c), got, "streaming feed {c}");
    }
}
