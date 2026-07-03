//! DTS Transport-Stream spoke gate — TS→IR demux + IR→TS mux (issue #560).
//!
//! Fixture: `fixtures/ts/dts/dts_core.ts` — a real ffmpeg `dca` encode (DTS
//! core substream, sync `0x7FFE8001`), stream_type `0x82`, 48 kHz stereo, 1170
//! whole 188-byte TS packets (see the fixture's `GENERATE.md`).
//!
//! Gate 2 below is an **independent oracle**: it reassembles the DTS PID's PES
//! payloads and counts `0x7FFE8001` sync words itself (byte-level, no crate
//! internals), rather than calling `transmux::split_dts_core_frames` — so it
//! actually bites a splitter that misses or duplicates frames.

use broadcast_common::{Package, Parse, Serialize, Unpackage};
use transmux::media::{CmafMux, Media};
use transmux::pipeline::CodecConfig;
use transmux::{DtsSpecificBox, Fmp4Demux, TsDemux, TsMux};

const TS: usize = 188;

fn load_fixture() -> Vec<u8> {
    let path = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../fixtures/ts/dts/dts_core.ts"
    );
    std::fs::read(path).expect("dts_core.ts fixture must exist")
}

fn demux_fixture() -> Media {
    let data = load_fixture();
    TsDemux::new().unpackage(&data).expect("demux dts_core.ts")
}

fn dts_track(media: &Media) -> &transmux::media::Track {
    media
        .tracks
        .iter()
        .find(|t| matches!(t.config(), CodecConfig::Dts { .. }))
        .expect("a DTS track in the demuxed media")
}

// ---------------------------------------------------------------------------
// Minimal TS/PSI/PES walking (byte-level, independent of crate internals) —
// mirrors tests/ts_mux.rs / tests/dolby.rs.
// ---------------------------------------------------------------------------

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

/// Reassemble the first complete single-packet PSI section on `pid`.
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

/// (stream_type, elementary_PID) pairs from a PMT section.
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

/// Find the DTS elementary PID (stream_type 0x82/0x85/0x8A) via PAT → PMT.
fn find_dts_pid(ts: &[u8]) -> (u16, u8) {
    let pat = first_section(ts, 0x0000).expect("PAT must be present");
    let programs = parse_pat(&pat);
    let pmt_pid = programs
        .iter()
        .find(|(prog, _)| *prog != 0)
        .map(|(_, pid)| *pid)
        .expect("PAT must list a program");
    let pmt = first_section(ts, pmt_pid).expect("PMT must resolve from PAT");
    let streams = parse_pmt(&pmt);
    streams
        .into_iter()
        .find(|(st, _)| matches!(*st, 0x82 | 0x85 | 0x8A))
        .map(|(st, pid)| (pid, st))
        .expect("PMT must carry a DTS elementary stream")
}

/// One reassembled PES packet's payload plus its PTS (90 kHz, wrapped as read).
struct OraclePes {
    payload: Vec<u8>,
    pts: u64,
}

/// Reassemble every PES packet on `target_pid`, decoding each PES header's
/// PTS. Adaptation fields / PUSI pointer fields are stripped by hand — no
/// crate internals used.
fn reassemble_pes(ts: &[u8], target_pid: u16) -> Vec<OraclePes> {
    let mut out: Vec<OraclePes> = Vec::new();
    for pkt in ts.chunks_exact(TS) {
        if pkt[0] != 0x47 || pid_of(pkt) != target_pid {
            continue;
        }
        let off = payload_offset(pkt);
        if off >= TS {
            continue;
        }
        let mut payload = &pkt[off..];
        if pusi_of(pkt) {
            assert_eq!(&payload[0..3], &[0x00, 0x00, 0x01], "PES start code");
            let flags2 = payload[7];
            let hdr_len = payload[8] as usize;
            let pts_dts_flags = (flags2 >> 6) & 0x3;
            let mut pts = 0u64;
            if pts_dts_flags != 0 {
                let b = &payload[9..14];
                pts = (((b[0] as u64 >> 1) & 0x7) << 30)
                    | ((b[1] as u64) << 22)
                    | (((b[2] as u64 >> 1) & 0x7F) << 15)
                    | ((b[3] as u64) << 7)
                    | ((b[4] as u64 >> 1) & 0x7F);
            }
            out.push(OraclePes {
                payload: Vec::new(),
                pts,
            });
            payload = &payload[9 + hdr_len..];
        }
        if let Some(last) = out.last_mut() {
            last.payload.extend_from_slice(payload);
        }
    }
    out
}

/// Count `0x7FFE8001` sync words in a concatenated ES buffer — the
/// independent oracle (does NOT use `transmux::split_dts_core_frames`).
fn count_dts_syncs(es: &[u8]) -> usize {
    let mut n = 0;
    let mut i = 0;
    while i + 4 <= es.len() {
        if es[i] == 0x7F && es[i + 1] == 0xFE && es[i + 2] == 0x80 && es[i + 3] == 0x01 {
            n += 1;
        }
        i += 1;
    }
    n
}

// ---------------------------------------------------------------------------
// Gate 1: demux enumerates one DTS track with sane codec-config fields.
// ---------------------------------------------------------------------------

#[test]
fn demux_enumerates_dts_track_with_correct_config() {
    let media = demux_fixture();
    let audio_tracks: Vec<_> = media
        .tracks
        .iter()
        .filter(|t| matches!(t.config(), CodecConfig::Dts { .. }))
        .collect();
    assert_eq!(audio_tracks.len(), 1, "exactly one DTS track expected");

    let track = audio_tracks[0];
    let CodecConfig::Dts {
        config,
        codec_fourcc,
        channel_count,
        sample_rate,
        sample_size,
    } = track.config()
    else {
        unreachable!()
    };

    assert_eq!(*sample_rate, 48_000);
    assert_eq!(*channel_count, 2);
    assert_eq!(codec_fourcc, b"dtsc");
    assert_eq!(*sample_size, 16);

    assert_eq!(config.dts_sampling_frequency, 48_000);
    assert!(!config.core_lfe_present, "fixture is stereo, no LFE");
    assert_eq!(config.core_layout, 2, "AMODE=stereo -> CoreLayout 2");
    assert_eq!(
        config.stream_construction, 1,
        "core-only -> StreamConstruction 1"
    );
    assert_eq!(
        config.pcm_sample_depth, 16,
        "core substream pcmSampleDepth is always 16"
    );

    // core_size must equal the fixture's actual per-frame byte length.
    let data = load_fixture();
    let (_dts_pid, stream_type) = find_dts_pid(&data);
    assert_eq!(stream_type, 0x82, "fixture stream_type must be 0x82");
    assert_eq!(
        config.core_size as usize,
        track.samples[0].data.len(),
        "core_size must equal the byte length of a demuxed DTS frame"
    );

    assert!(
        track.samples.len() > 100,
        "expected > 100 DTS core frames (~188 @ 512 samples/frame / 48 kHz), got {}",
        track.samples.len()
    );
}

// ---------------------------------------------------------------------------
// Gate 2: independent oracle — sync-word count matches the demuxed sample
// count exactly (proves the splitter neither drops nor duplicates frames).
// ---------------------------------------------------------------------------

#[test]
fn independent_sync_word_oracle_matches_demuxed_sample_count() {
    let data = load_fixture();
    let (dts_pid, _stream_type) = find_dts_pid(&data);
    let pes = reassemble_pes(&data, dts_pid);
    let es: Vec<u8> = pes.iter().flat_map(|p| p.payload.iter().copied()).collect();
    let oracle_count = count_dts_syncs(&es);
    assert!(oracle_count > 100, "sanity: fixture must carry >100 frames");

    let media = demux_fixture();
    let track = dts_track(&media);
    assert_eq!(
        track.samples.len(),
        oracle_count,
        "demuxed sample count must equal the independently-counted sync words"
    );
}

// ---------------------------------------------------------------------------
// Gate 3: timing — PTS strictly monotonic, durations consistent, first
// sample PTS equals the first PES packet's PTS.
// ---------------------------------------------------------------------------

#[test]
fn timing_is_monotonic_and_matches_first_pes_pts() {
    let data = load_fixture();
    let (dts_pid, _st) = find_dts_pid(&data);
    let pes = reassemble_pes(&data, dts_pid);
    let first_pes_pts = pes.first().expect("at least one PES packet").pts;

    let media = demux_fixture();
    let track = dts_track(&media);

    assert!(!track.samples.is_empty());
    let first = &track.samples[0];
    let timing0 = first
        .source_timing
        .expect("DTS samples must carry SourceTiming");
    assert_eq!(
        timing0.pts, first_pes_pts,
        "first sample PTS must equal the first PES packet's PTS"
    );

    let expected_duration = first.duration;
    assert_eq!(expected_duration, 512, "512 samples/frame (NBLKS=15)");

    let mut prev_pts: Option<u64> = None;
    for (i, s) in track.samples.iter().enumerate() {
        let t = s
            .source_timing
            .unwrap_or_else(|| panic!("sample {i} must carry SourceTiming"));
        if let Some(p) = prev_pts {
            assert!(
                t.pts > p,
                "sample {i}: PTS must be strictly monotonic ({} > {p})",
                t.pts
            );
        }
        prev_pts = Some(t.pts);
        assert_eq!(
            s.duration, expected_duration,
            "sample {i}: duration must be consistent (constant NBLKS across the stream)"
        );
    }
}

// ---------------------------------------------------------------------------
// Gate 4: round-trip TS -> IR -> fMP4 -> IR: ddts + sample bytes byte-exact.
// ---------------------------------------------------------------------------

#[test]
fn fmp4_round_trip_preserves_ddts_and_sample_bytes() {
    let media = demux_fixture();
    let track1 = dts_track(&media);
    let CodecConfig::Dts { config: ddts1, .. } = track1.config() else {
        unreachable!()
    };
    let mut ddts1_bytes = vec![0u8; ddts1.serialized_len()];
    ddts1.serialize_into(&mut ddts1_bytes).unwrap();

    let mut mux = CmafMux::new(1);
    let fmp4 = mux.package(&media).expect("mux to fMP4");

    let media2 = Fmp4Demux::new()
        .unpackage(&fmp4)
        .expect("demux round-tripped fMP4");
    let track2 = dts_track(&media2);
    let CodecConfig::Dts { config: ddts2, .. } = track2.config() else {
        unreachable!()
    };
    let mut ddts2_bytes = vec![0u8; ddts2.serialized_len()];
    ddts2.serialize_into(&mut ddts2_bytes).unwrap();

    assert_eq!(
        ddts1_bytes, ddts2_bytes,
        "ddts must round-trip byte-identical through fMP4"
    );

    assert_eq!(track2.samples.len(), track1.samples.len());
    for (i, (a, b)) in track1.samples.iter().zip(&track2.samples).enumerate() {
        assert_eq!(
            a.data, b.data,
            "sample {i}: coded DTS frame bytes must be byte-identical after fMP4 round-trip"
        );
    }

    // Cross-check DtsSpecificBox::parse also agrees (parse/serialize symmetry).
    let reparsed = DtsSpecificBox::parse(&ddts2_bytes).unwrap();
    assert_eq!(&reparsed, ddts2);
}

// ---------------------------------------------------------------------------
// Gate 5: TS mux round-trip — sample bytes byte-identical, PMT carries 0x82.
// ---------------------------------------------------------------------------

#[test]
fn ts_mux_round_trip_preserves_samples_and_stream_type() {
    let media = demux_fixture();
    let track1 = dts_track(&media);

    let ts2 = TsMux::new().package(&media).expect("mux IR back to TS");

    // PMT must carry a DTS elementary stream with stream_type 0x82.
    let (_dts_pid2, stream_type2) = find_dts_pid(&ts2);
    assert_eq!(
        stream_type2, 0x82,
        "muxed PMT must carry DTS stream_type 0x82"
    );

    let media2 = TsDemux::new()
        .unpackage(&ts2)
        .expect("re-demux muxed output");
    let track2 = dts_track(&media2);

    assert_eq!(track2.samples.len(), track1.samples.len());
    for (i, (a, b)) in track1.samples.iter().zip(&track2.samples).enumerate() {
        assert_eq!(
            a.data, b.data,
            "sample {i}: DTS frame bytes must be byte-identical after TS mux round-trip"
        );
    }

    let CodecConfig::Dts {
        sample_rate: sr1,
        channel_count: cc1,
        ..
    } = track1.config()
    else {
        unreachable!()
    };
    let CodecConfig::Dts {
        sample_rate: sr2,
        channel_count: cc2,
        ..
    } = track2.config()
    else {
        unreachable!()
    };
    assert_eq!(sr1, sr2);
    assert_eq!(cc1, cc2);
}
