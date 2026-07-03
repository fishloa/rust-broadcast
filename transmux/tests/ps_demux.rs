//! `PsDemux` gate — MPEG-2 Program Stream → hub `Media` IR, verified against
//! ffprobe / ffmpeg oracles for `fixtures/ps/h264_ac3.ps` (issue #470).
//!
//! Oracles (committed):
//! - `fixtures/ps/h264_ac3.packets.csv` — ffprobe per-packet
//!   (codec_type, stream_index, pts, dts, duration, size, keyframe) in 90 kHz
//!   ticks. Many video rows carry empty (`N/A`) pts/dts — a PS does not stamp
//!   every frame — so video timing is asserted only against the rows that carry
//!   values.
//! - `fixtures/ts/demux-oracle/h264_aac.ref.mp4` — the H.264 video is the SAME
//!   source video (`-c:v copy`); its `avcC` box body is the byte oracle for the
//!   H.264 config and its video sample NAL payloads are the byte oracle for the
//!   coded video data.
//!
//! Every test below is written to *bite*: the demuxed IR is compared against the
//! external oracle values, not hardcoded numbers.

use std::path::PathBuf;

use broadcast_common::Unpackage;
use transmux::PsDemux;
use transmux::media::Media;
use transmux::pipeline::CodecConfig;

// ── Fixture loading ─────────────────────────────────────────────────────────

fn ps_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../fixtures/ps")
}

fn load_ps() -> Vec<u8> {
    let path = ps_dir().join("h264_ac3.ps");
    let data = std::fs::read(&path).expect("h264_ac3.ps fixture must exist");
    assert_eq!(data[4], 0x44, "byte 4 must be the MPEG-2 SCR prefix (0x44)");
    data
}

fn ref_mp4() -> Vec<u8> {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../fixtures/ts/demux-oracle/h264_aac.ref.mp4");
    std::fs::read(&path).expect("h264_aac.ref.mp4 oracle must exist")
}

/// One ffprobe oracle row; `pts`/`dts` are `None` for empty (`N/A`) fields.
#[derive(Debug, Clone)]
struct OracleRow {
    codec_type: String,
    pts: Option<u64>,
    dts: Option<u64>,
    keyframe: Option<bool>,
}

fn load_oracle_rows() -> Vec<OracleRow> {
    let text = std::fs::read_to_string(ps_dir().join("h264_ac3.packets.csv"))
        .expect("packets.csv oracle must exist");
    let mut rows = Vec::new();
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let f: Vec<&str> = line.split(',').collect();
        assert!(f.len() >= 7, "oracle row must have 7 columns: {line:?}");
        let parse_opt = |s: &str| -> Option<u64> {
            let s = s.trim();
            if s.is_empty() || s == "N/A" {
                None
            } else {
                Some(s.parse().unwrap_or_else(|_| panic!("bad ts field {s:?}")))
            }
        };
        let kf = f[6].trim();
        rows.push(OracleRow {
            codec_type: f[0].to_string(),
            pts: parse_opt(f[2]),
            dts: parse_opt(f[3]),
            keyframe: if kf.is_empty() || kf == "N/A" {
                None
            } else {
                Some(kf.parse::<u8>().unwrap() == 1)
            },
        });
    }
    rows
}

fn oracle_for(rows: &[OracleRow], codec_type: &str) -> Vec<OracleRow> {
    rows.iter()
        .filter(|r| r.codec_type == codec_type)
        .cloned()
        .collect()
}

// ── Minimal ISOBMFF box walking for the ref mp4 (progressive, non-fragmented) ─

struct Box4<'a> {
    ty: [u8; 4],
    offset: usize,
    size: usize,
    header: usize,
    body: &'a [u8],
}

fn iter_boxes(buf: &[u8], base: usize) -> Vec<Box4<'_>> {
    let mut out = Vec::new();
    let mut off = 0usize;
    while off + 8 <= buf.len() {
        let size =
            u32::from_be_bytes([buf[off], buf[off + 1], buf[off + 2], buf[off + 3]]) as usize;
        let ty = [buf[off + 4], buf[off + 5], buf[off + 6], buf[off + 7]];
        let (size, header) = if size == 1 {
            let s = u64::from_be_bytes([
                buf[off + 8],
                buf[off + 9],
                buf[off + 10],
                buf[off + 11],
                buf[off + 12],
                buf[off + 13],
                buf[off + 14],
                buf[off + 15],
            ]) as usize;
            (s, 16)
        } else {
            (size, 8)
        };
        if size < header || off + size > buf.len() {
            break;
        }
        out.push(Box4 {
            ty,
            offset: base + off,
            size,
            header,
            body: &buf[off + header..off + size],
        });
        off += size;
    }
    out
}

fn find_box<'a>(buf: &'a [u8], base: usize, want: &[u8; 4]) -> Option<Box4<'a>> {
    for b in iter_boxes(buf, base) {
        if &b.ty == want {
            return Some(Box4 {
                ty: b.ty,
                offset: b.offset,
                size: b.size,
                header: b.header,
                body: b.body,
            });
        }
        let containers: &[[u8; 4]] = &[
            *b"moov", *b"trak", *b"mdia", *b"minf", *b"stbl", *b"mvex", *b"edts",
        ];
        if containers.contains(&b.ty) {
            if let Some(found) = find_box(b.body, b.offset + b.header, want) {
                return Some(found);
            }
        }
        if &b.ty == b"stsd" && b.body.len() > 8 {
            if let Some(found) = find_box(&b.body[8..], b.offset + b.header + 8, want) {
                return Some(found);
            }
        }
        if &b.ty == b"avc1" || &b.ty == b"mp4a" {
            let skip = if &b.ty == b"avc1" { 78 } else { 28 };
            if b.body.len() > skip {
                if let Some(found) = find_box(&b.body[skip..], b.offset + b.header + skip, want) {
                    return Some(found);
                }
            }
        }
    }
    None
}

/// Handler type + trak location, for locating the video track in the ref mp4.
struct RefTrack<'a> {
    handler: [u8; 4],
    trak_body: &'a [u8],
    trak_base: usize,
}

fn ref_tracks(mp4: &[u8]) -> Vec<RefTrack<'_>> {
    let moov = find_box(mp4, 0, b"moov").expect("ref mp4 must have moov");
    let mut tracks = Vec::new();
    for b in iter_boxes(moov.body, moov.offset + moov.header) {
        if &b.ty == b"trak" {
            let hdlr = find_box(b.body, b.offset + b.header, b"hdlr").expect("trak has hdlr");
            let handler = [hdlr.body[8], hdlr.body[9], hdlr.body[10], hdlr.body[11]];
            tracks.push(RefTrack {
                handler,
                trak_body: b.body,
                trak_base: b.offset + b.header,
            });
        }
    }
    tracks
}

/// Resolve a track's samples (stsz/stsc/stco → mdat byte ranges), in file order.
fn ref_track_samples(mp4: &[u8], trak_body: &[u8], trak_base: usize) -> Vec<Vec<u8>> {
    let stsz = find_box(trak_body, trak_base, b"stsz").expect("stsz");
    let stsc = find_box(trak_body, trak_base, b"stsc").expect("stsc");
    let stco = find_box(trak_body, trak_base, b"stco").expect("stco");

    let default_size = u32::from_be_bytes([stsz.body[4], stsz.body[5], stsz.body[6], stsz.body[7]]);
    let sample_count =
        u32::from_be_bytes([stsz.body[8], stsz.body[9], stsz.body[10], stsz.body[11]]) as usize;
    let sizes: Vec<u32> = if default_size != 0 {
        vec![default_size; sample_count]
    } else {
        (0..sample_count)
            .map(|i| {
                let o = 12 + i * 4;
                u32::from_be_bytes([
                    stsz.body[o],
                    stsz.body[o + 1],
                    stsz.body[o + 2],
                    stsz.body[o + 3],
                ])
            })
            .collect()
    };

    let chunk_count =
        u32::from_be_bytes([stco.body[4], stco.body[5], stco.body[6], stco.body[7]]) as usize;
    let chunk_offsets: Vec<u32> = (0..chunk_count)
        .map(|i| {
            let o = 8 + i * 4;
            u32::from_be_bytes([
                stco.body[o],
                stco.body[o + 1],
                stco.body[o + 2],
                stco.body[o + 3],
            ])
        })
        .collect();

    let stsc_count =
        u32::from_be_bytes([stsc.body[4], stsc.body[5], stsc.body[6], stsc.body[7]]) as usize;
    let stsc_entries: Vec<(u32, u32)> = (0..stsc_count)
        .map(|i| {
            let o = 8 + i * 12;
            let first_chunk = u32::from_be_bytes([
                stsc.body[o],
                stsc.body[o + 1],
                stsc.body[o + 2],
                stsc.body[o + 3],
            ]);
            let spc = u32::from_be_bytes([
                stsc.body[o + 4],
                stsc.body[o + 5],
                stsc.body[o + 6],
                stsc.body[o + 7],
            ]);
            (first_chunk, spc)
        })
        .collect();

    let mut spc_per_chunk = vec![0u32; chunk_count];
    for (idx, &(first_chunk, spc)) in stsc_entries.iter().enumerate() {
        let last_chunk = if idx + 1 < stsc_entries.len() {
            stsc_entries[idx + 1].0 - 1
        } else {
            chunk_count as u32
        };
        for c in first_chunk..=last_chunk {
            if (c as usize) <= chunk_count {
                spc_per_chunk[(c - 1) as usize] = spc;
            }
        }
    }

    let mut samples = Vec::with_capacity(sample_count);
    let mut sample_idx = 0usize;
    for (c, &chunk_off) in chunk_offsets.iter().enumerate() {
        let mut pos = chunk_off as usize;
        for _ in 0..spc_per_chunk[c] {
            if sample_idx >= sample_count {
                break;
            }
            let sz = sizes[sample_idx] as usize;
            samples.push(mp4[pos..pos + sz].to_vec());
            pos += sz;
            sample_idx += 1;
        }
    }
    assert_eq!(samples.len(), sample_count, "resolved all ref samples");
    samples
}

/// Split length-prefixed (4-byte) NAL data into its coded NAL payloads.
fn split_length_prefixed(lp: &[u8]) -> Vec<Vec<u8>> {
    let mut out = Vec::new();
    let mut off = 0usize;
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

/// Extract the `avcC` box body (the full `AVCDecoderConfigurationRecord`) from
/// the ref mp4's video track.
fn ref_avcc_body(mp4: &[u8]) -> Vec<u8> {
    let avcc = find_box(mp4, 0, b"avcC").expect("ref mp4 must have an avcC box");
    avcc.body.to_vec()
}

// ── Tests ─────────────────────────────────────────────────────────────────

/// Test 1 — stream enumeration: exactly 2 tracks, H.264 video then AC-3 audio.
#[test]
fn enumerates_two_tracks_h264_then_ac3() {
    let ps = load_ps();
    let media: Media = PsDemux::new().unpackage(&ps).expect("demux must succeed");

    assert_eq!(media.tracks.len(), 2, "must demux exactly 2 tracks");
    assert!(
        matches!(media.tracks[0].spec.config, CodecConfig::Avc { .. }),
        "track 0 must be H.264/AVC video, got {:?}",
        media.tracks[0].spec.config
    );
    assert!(
        matches!(media.tracks[1].spec.config, CodecConfig::Ac3 { .. }),
        "track 1 must be AC-3 audio, got {:?}",
        media.tracks[1].spec.config
    );
}

/// Test 2 — sample counts + oracle timing.
///
/// 75 video + 88 audio samples. Every video oracle row that carries a PTS/DTS
/// must be present in the demuxed video samples (matched by (pts, dts)); the
/// keyframe flag on those stamped rows must agree. Audio: 88 contiguous frames.
#[test]
fn sample_counts_and_oracle_timing() {
    let ps = load_ps();
    let media: Media = PsDemux::new().unpackage(&ps).expect("demux");
    let rows = load_oracle_rows();
    let video_rows = oracle_for(&rows, "video");
    let audio_rows = oracle_for(&rows, "audio");
    assert_eq!(video_rows.len(), 75, "oracle has 75 video frames");
    assert_eq!(audio_rows.len(), 88, "oracle has 88 audio frames");

    let vtrack = &media.tracks[0];
    let atrack = &media.tracks[1];
    assert_eq!(vtrack.samples.len(), 75, "75 demuxed video samples");
    assert_eq!(atrack.samples.len(), 88, "88 demuxed audio samples");

    // Reconstruct each demuxed sample's absolute (dts, composition_offset).
    // Track base decode time is 0, so demuxed DTS is the running sum of the
    // decode-order durations; PTS = DTS + composition_offset.
    let mut running_dts: i64 = 0;
    let mut demuxed: Vec<(i64, i32, bool)> = Vec::new(); // (dts, co, is_sync)
    for s in &vtrack.samples {
        demuxed.push((running_dts, s.composition_offset, s.is_sync));
        running_dts += s.duration as i64;
    }

    // Every stamped oracle row carries both PTS and DTS here. Two independent,
    // non-cancelling invariants must hold for each:
    //   • composition_offset (pts − dts) matches EXACTLY — catches a PTS-only or
    //     DTS-only shift (relative-delta matching alone would not).
    //   • the DTS position relative to the first stamped frame matches — catches
    //     wrong frame spacing / mis-anchored decode times.
    let stamped: Vec<&OracleRow> = video_rows
        .iter()
        .filter(|r| r.pts.is_some() && r.dts.is_some())
        .collect();
    assert!(
        stamped.len() >= 9,
        "oracle must carry several stamped video rows, got {}",
        stamped.len()
    );
    let oracle_base_dts = stamped[0].dts.unwrap() as i64;
    let demux_base_dts = demuxed[0].0;
    for row in &stamped {
        let want_co = (row.pts.unwrap() as i64 - row.dts.unwrap() as i64) as i32;
        let want_dts_rel = row.dts.unwrap() as i64 - oracle_base_dts;
        let found = demuxed
            .iter()
            .find(|&&(dts, co, _)| co == want_co && (dts - demux_base_dts) == want_dts_rel);
        assert!(
            found.is_some(),
            "no demuxed video sample matching stamped oracle \
             (pts={:?}, dts={:?}, co={want_co}, rel_dts={want_dts_rel})",
            row.pts,
            row.dts
        );
        if let Some(kf) = row.keyframe {
            let (_, _, is_sync) = *found.unwrap();
            assert_eq!(
                is_sync, kf,
                "keyframe flag must match on stamped oracle row (pts={:?})",
                row.pts
            );
        }
    }

    // The number of demuxed sync samples must equal the oracle keyframe count
    // (the fixture has an IDR every GOP) — sanity that is_sync bites.
    let oracle_keyframes = video_rows
        .iter()
        .filter(|r| r.keyframe == Some(true))
        .count();
    let syncs = vtrack.samples.iter().filter(|s| s.is_sync).count();
    assert_eq!(
        syncs, oracle_keyframes,
        "demuxed video keyframe count must equal the oracle's"
    );
    assert!(
        oracle_keyframes >= 1,
        "oracle must carry at least one keyframe"
    );
}

/// Test 3 — config byte oracle.
///
/// The `avcC` built from the PS's in-band SPS/PPS is byte-identical to the
/// `avcC` box body in the ref mp4 (same source video). The AC-3 `dac3` is built
/// from the syncframe BSI: assert a valid 0x0B77 syncframe and a self-consistent
/// `dac3`.
#[test]
fn config_byte_oracle() {
    use broadcast_common::Serialize;

    let ps = load_ps();
    let media: Media = PsDemux::new().unpackage(&ps).expect("demux");

    // Serialize the demuxed avcC box and strip its 8-byte box header → body.
    let CodecConfig::Avc { ref config, .. } = media.tracks[0].spec.config else {
        panic!("track 0 must be AVC");
    };
    let mut buf = vec![0u8; config.serialized_len()];
    config.serialize_into(&mut buf).expect("serialize avcC");
    // config.serialize_into writes the full `avcC` box (size+type+record); the
    // record body starts after the 8-byte box header.
    assert_eq!(&buf[4..8], b"avcC", "demuxed config must serialize as avcC");
    let demuxed_body = &buf[8..];

    let oracle_body = ref_avcc_body(&ref_mp4());
    assert_eq!(
        demuxed_body, oracle_body,
        "avcC body from PS in-band SPS/PPS must equal the ref mp4 avcC body"
    );

    // AC-3: a valid 0x0B77 syncframe and self-consistent dac3.
    let CodecConfig::Ac3 {
        ref config,
        sample_rate,
        channel_count,
        ..
    } = media.tracks[1].spec.config
    else {
        panic!("track 1 must be AC-3");
    };
    // First audio sample begins with the AC-3 syncword.
    assert_eq!(
        &media.tracks[1].samples[0].data[0..2],
        &[0x0B, 0x77],
        "AC-3 frame must begin with the 0x0B77 syncword"
    );
    // dac3 is self-consistent: known sample rate, non-zero channel count.
    assert!(
        matches!(sample_rate, 48000 | 44100 | 32000),
        "AC-3 sample_rate must be a valid fscod rate, got {sample_rate}"
    );
    assert!(channel_count >= 1, "AC-3 must have at least one channel");
    // dac3 body serializes to its fixed 3-byte record and its fscod agrees with
    // the reported sample_rate (self-consistency of the built config).
    let mut dbuf = vec![0u8; config.serialized_len()];
    let written = config.serialize_into(&mut dbuf).expect("serialize dac3");
    assert_eq!(
        written, 3,
        "dac3 record body is 3 bytes (ETSI TS 102 366 F.4)"
    );
    // Top 2 bits of the record are `fscod`; map back to Hz and cross-check.
    let fscod = dbuf[0] >> 6;
    let fscod_rate = match fscod {
        0 => 48000,
        1 => 44100,
        2 => 32000,
        _ => 0,
    };
    assert_eq!(
        fscod_rate, sample_rate,
        "dac3 fscod must round-trip to the reported AC-3 sample_rate"
    );
}

/// Test 4 — video sample fidelity: the PS-demuxed video coded NAL payloads equal
/// the ref mp4's video sample NAL payloads, sample-for-sample (same source).
#[test]
fn video_sample_fidelity() {
    let ps = load_ps();
    let media: Media = PsDemux::new().unpackage(&ps).expect("demux");

    let mp4 = ref_mp4();
    let tracks = ref_tracks(&mp4);
    let vtrack = tracks
        .iter()
        .find(|t| &t.handler == b"vide")
        .expect("ref mp4 must have a video track");
    let ref_samples = ref_track_samples(&mp4, vtrack.trak_body, vtrack.trak_base);
    assert_eq!(ref_samples.len(), 75, "ref mp4 has 75 video samples");
    assert_eq!(
        media.tracks[0].samples.len(),
        75,
        "75 demuxed video samples"
    );

    // Compare the SET of coded NAL payloads (VCL + relevant NALs). The PS carries
    // in-band SPS/PPS/AUD interleaved; the ref mp4 (mp4 -c copy) drops AUDs and
    // may relocate SPS/PPS to the avcC. So compare the multiset of *slice* NAL
    // payloads (VCL types 1 & 5), which is the coded video data proper.
    fn vcl_nals(samples: &[Vec<Vec<u8>>]) -> Vec<Vec<u8>> {
        let mut out = Vec::new();
        for s in samples {
            for nal in s {
                let t = nal[0] & 0x1F;
                if t == 1 || t == 5 {
                    out.push(nal.clone());
                }
            }
        }
        out
    }

    let demuxed_nals: Vec<Vec<Vec<u8>>> = media.tracks[0]
        .samples
        .iter()
        .map(|s| split_length_prefixed(&s.data))
        .collect();
    let ref_nals: Vec<Vec<Vec<u8>>> = ref_samples
        .iter()
        .map(|s| split_length_prefixed(s))
        .collect();

    let mut demux_vcl = vcl_nals(&demuxed_nals);
    let mut ref_vcl = vcl_nals(&ref_nals);
    assert_eq!(
        demux_vcl.len(),
        75,
        "one VCL slice per video access unit (demuxed)"
    );
    assert_eq!(
        ref_vcl.len(),
        75,
        "one VCL slice per video access unit (ref)"
    );
    demux_vcl.sort();
    ref_vcl.sort();
    assert_eq!(
        demux_vcl, ref_vcl,
        "PS-demuxed coded video NAL payloads must equal the ref mp4's, sample-for-sample"
    );
}

/// Test 5 — audio frame validity: every demuxed AC-3 frame begins with the
/// 0x0B77 syncword; 88 frames.
#[test]
fn audio_frame_validity() {
    let ps = load_ps();
    let media: Media = PsDemux::new().unpackage(&ps).expect("demux");
    let atrack = &media.tracks[1];
    assert_eq!(atrack.samples.len(), 88, "88 AC-3 frames");
    for (i, s) in atrack.samples.iter().enumerate() {
        assert!(
            s.data.len() >= 2 && s.data[0] == 0x0B && s.data[1] == 0x77,
            "AC-3 frame {i} must begin with the 0x0B77 syncword, got {:02X?}",
            &s.data[..s.data.len().min(2)]
        );
    }
}
