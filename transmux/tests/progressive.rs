//! Progressive (single-file, non-fragmented) MP4 output gate — issue #463.
//!
//! Pipeline under test: `TsDemux::unpackage(h264_aac.ts)` → [`Media`], then
//! `ProgressiveMux { faststart: true }.package(&media)` → a complete `.mp4`.
//!
//! Oracle: `fixtures/ts/demux-oracle/h264_aac.ref.mp4` is
//! `ffmpeg -movflags +faststart -c copy` of `fixtures/ts/h264_aac.ts`
//! (H.264 video + AAC audio, 75 video samples). Its box order (moov before
//! mdat) and its video sample bytes / `avcC` are the oracle.
//!
//! Each test re-parses the output with a minimal, offset-free ISOBMFF walker
//! (below) — no hardcoded offsets — so the tables must be internally consistent
//! with the real mdat layout for the assertions to hold.

use std::path::PathBuf;

use broadcast_common::{Package, Parse, Serialize, Unpackage};
use transmux::media::Media;
use transmux::pipeline::CodecConfig;
use transmux::progressive::ProgressiveMux;
use transmux::ts_demux::TsDemux;

// ---------------------------------------------------------------------------
// Minimal ISOBMFF walker (test-local; no dependency on crate box parsers so
// the tests genuinely bite the serialized bytes).
// ---------------------------------------------------------------------------

fn be32(b: &[u8], o: usize) -> u32 {
    u32::from_be_bytes([b[o], b[o + 1], b[o + 2], b[o + 3]])
}
fn be64(b: &[u8], o: usize) -> u64 {
    u64::from_be_bytes([
        b[o],
        b[o + 1],
        b[o + 2],
        b[o + 3],
        b[o + 4],
        b[o + 5],
        b[o + 6],
        b[o + 7],
    ])
}

/// A located box: absolute start, header length, total size, and 4-CC.
#[derive(Clone, Copy, Debug)]
struct Loc {
    start: usize,
    hdr: usize,
    size: usize,
    typ: [u8; 4],
}

impl Loc {
    fn body<'a>(&self, file: &'a [u8]) -> &'a [u8] {
        &file[self.start + self.hdr..self.start + self.size]
    }
}

/// Walk the immediate child boxes within `region` (absolute-offset aware:
/// `region_start` is the file offset of `region[0]`).
fn children(region_start: usize, region: &[u8]) -> Vec<Loc> {
    let mut out = Vec::new();
    let mut o = 0usize;
    while o + 8 <= region.len() {
        let mut size = be32(region, o) as usize;
        let mut hdr = 8;
        if size == 1 {
            size = be64(region, o + 8) as usize;
            hdr = 16;
        } else if size == 0 {
            size = region.len() - o;
        }
        if size < hdr || o + size > region.len() {
            break;
        }
        let mut typ = [0u8; 4];
        typ.copy_from_slice(&region[o + 4..o + 8]);
        out.push(Loc {
            start: region_start + o,
            hdr,
            size,
            typ,
        });
        o += size;
    }
    out
}

fn top_boxes(file: &[u8]) -> Vec<Loc> {
    children(0, file)
}

fn find<'a>(locs: &'a [Loc], typ: &[u8; 4]) -> Option<&'a Loc> {
    locs.iter().find(|l| &l.typ == typ)
}

// ---- stbl table parsers (test-local) --------------------------------------

/// Return the ordered list of trak boxes under moov.
fn traks(file: &[u8]) -> Vec<Loc> {
    let moov = *find(&top_boxes(file), b"moov").expect("moov");
    children(moov.start + moov.hdr, moov.body(file))
        .into_iter()
        .filter(|l| &l.typ == b"trak")
        .collect()
}

/// Locate the stbl children map for a given trak Loc.
fn stbl_children(file: &[u8], trak: Loc) -> Vec<Loc> {
    // trak → mdia → minf → stbl
    let mdia = *find(&children(trak.start + trak.hdr, trak.body(file)), b"mdia").unwrap();
    let minf = *find(&children(mdia.start + mdia.hdr, mdia.body(file)), b"minf").unwrap();
    let stbl = *find(&children(minf.start + minf.hdr, minf.body(file)), b"stbl").unwrap();
    children(stbl.start + stbl.hdr, stbl.body(file))
}

/// Is this trak a video (avc1) trak? (Check the stsd first entry 4-CC.)
fn is_video_trak(file: &[u8], trak: Loc) -> bool {
    let sc = stbl_children(file, trak);
    let stsd = *find(&sc, b"stsd").unwrap();
    let body = stsd.body(file); // version+flags(4) + count(4) then entries
    if body.len() < 16 {
        return false;
    }
    &body[12..16] == b"avc1"
}

fn video_trak(file: &[u8]) -> Loc {
    traks(file)
        .into_iter()
        .find(|&t| is_video_trak(file, t))
        .expect("video trak")
}

/// Parse stsz per-sample sizes (returns Vec of sizes; panics if uniform).
fn parse_stsz(file: &[u8], sc: &[Loc]) -> Vec<u32> {
    let stsz = *find(sc, b"stsz").expect("stsz");
    let b = stsz.body(file);
    let sample_size = be32(b, 4);
    let count = be32(b, 8) as usize;
    if sample_size != 0 {
        return vec![sample_size; count];
    }
    (0..count).map(|i| be32(b, 12 + i * 4)).collect()
}

/// Parse stts total sample count (sum of run counts).
fn parse_stts_total(file: &[u8], sc: &[Loc]) -> u32 {
    let stts = *find(sc, b"stts").expect("stts");
    let b = stts.body(file);
    let n = be32(b, 4) as usize;
    (0..n).map(|i| be32(b, 8 + i * 8)).sum()
}

/// Parse stss sync-sample indices (1-based), if present.
fn parse_stss(file: &[u8], sc: &[Loc]) -> Option<Vec<u32>> {
    let stss = find(sc, b"stss")?;
    let b = stss.body(file);
    let n = be32(b, 4) as usize;
    Some((0..n).map(|i| be32(b, 8 + i * 4)).collect())
}

/// Parse stsc entries: (first_chunk, samples_per_chunk, sdi).
fn parse_stsc(file: &[u8], sc: &[Loc]) -> Vec<(u32, u32, u32)> {
    let stsc = *find(sc, b"stsc").expect("stsc");
    let b = stsc.body(file);
    let n = be32(b, 4) as usize;
    (0..n)
        .map(|i| {
            let o = 8 + i * 12;
            (be32(b, o), be32(b, o + 4), be32(b, o + 8))
        })
        .collect()
}

/// Parse chunk offsets from stco (32-bit) or co64 (64-bit).
fn parse_chunk_offsets(file: &[u8], sc: &[Loc]) -> Vec<u64> {
    if let Some(stco) = find(sc, b"stco") {
        let b = stco.body(file);
        let n = be32(b, 4) as usize;
        (0..n).map(|i| be32(b, 8 + i * 4) as u64).collect()
    } else {
        let co64 = *find(sc, b"co64").expect("stco or co64");
        let b = co64.body(file);
        let n = be32(b, 4) as usize;
        (0..n).map(|i| be64(b, 8 + i * 8)).collect()
    }
}

/// Resolve every sample's byte range using stsz + stsc + chunk offsets, and
/// return the sample byte slices in decode order. This is the canonical
/// ISOBMFF sample-resolution algorithm (§8.7.4).
fn resolve_samples(file: &[u8], trak: Loc) -> Vec<Vec<u8>> {
    let sc = stbl_children(file, trak);
    let sizes = parse_stsz(file, &sc);
    let stsc = parse_stsc(file, &sc);
    let chunk_offsets = parse_chunk_offsets(file, &sc);

    // Expand stsc into a per-chunk samples-per-chunk list.
    let num_chunks = chunk_offsets.len();
    let mut spc = vec![0u32; num_chunks];
    for (i, &(first_chunk, samples_per_chunk, _sdi)) in stsc.iter().enumerate() {
        let last_chunk = if i + 1 < stsc.len() {
            stsc[i + 1].0
        } else {
            num_chunks as u32 + 1
        };
        for c in first_chunk..last_chunk {
            if (c as usize) >= 1 && (c as usize) <= num_chunks {
                spc[(c - 1) as usize] = samples_per_chunk;
            }
        }
    }

    let mut samples = Vec::new();
    let mut sample_idx = 0usize;
    for (c, &chunk_off) in chunk_offsets.iter().enumerate() {
        let mut pos = chunk_off as usize;
        for _ in 0..spc[c] {
            let sz = sizes[sample_idx] as usize;
            assert!(
                pos + sz <= file.len(),
                "sample {} of chunk {} exceeds file ({}+{} > {})",
                sample_idx,
                c,
                pos,
                sz,
                file.len()
            );
            samples.push(file[pos..pos + sz].to_vec());
            pos += sz;
            sample_idx += 1;
        }
    }
    assert_eq!(sample_idx, sizes.len(), "resolved sample count mismatch");
    samples
}

// ---------------------------------------------------------------------------
// Fixtures + pipeline helpers
// ---------------------------------------------------------------------------

fn fixture(rel: &[&str]) -> Vec<u8> {
    let mut p: PathBuf = [env!("CARGO_MANIFEST_DIR"), ".."].iter().collect();
    for seg in rel {
        p.push(seg);
    }
    std::fs::read(&p).unwrap_or_else(|e| panic!("read {}: {e}", p.display()))
}

fn demux_media() -> Media {
    let ts = fixture(&["fixtures", "ts", "h264_aac.ts"]);
    TsDemux::new().unpackage(&ts).expect("demux ts")
}

fn ref_mp4() -> Vec<u8> {
    fixture(&["fixtures", "ts", "demux-oracle", "h264_aac.ref.mp4"])
}

fn package(faststart: bool) -> Vec<u8> {
    let media = demux_media();
    ProgressiveMux { faststart }
        .package(&media)
        .expect("package progressive")
}

const EXPECTED_VIDEO_SAMPLES: u32 = 75;

// ---------------------------------------------------------------------------
// Test 1: faststart box order — moov before mdat.
// ---------------------------------------------------------------------------

#[test]
fn faststart_moov_precedes_mdat() {
    let out = package(true);
    let tops = top_boxes(&out);
    let moov = find(&tops, b"moov").expect("moov present");
    let mdat = find(&tops, b"mdat").expect("mdat present");
    assert!(
        moov.start < mdat.start,
        "faststart: moov (@{}) must precede mdat (@{})",
        moov.start,
        mdat.start
    );
    // Sanity: ftyp is the very first box.
    assert_eq!(&tops[0].typ, b"ftyp", "ftyp must be first");
}

// ---------------------------------------------------------------------------
// Test 2: sample-table internal consistency for the video trak.
// ---------------------------------------------------------------------------

#[test]
fn video_sample_tables_consistent() {
    let out = package(true);
    let vtrak = video_trak(&out);
    let sc = stbl_children(&out, vtrak);

    // stsz sample_count == 75.
    let sizes = parse_stsz(&out, &sc);
    assert_eq!(sizes.len() as u32, EXPECTED_VIDEO_SAMPLES, "stsz count");

    // stts entries sum to 75 samples.
    assert_eq!(
        parse_stts_total(&out, &sc),
        EXPECTED_VIDEO_SAMPLES,
        "stts total"
    );

    // The mdat box bounds.
    let tops = top_boxes(&out);
    let mdat = *find(&tops, b"mdat").expect("mdat");
    let mdat_payload_start = mdat.start + mdat.hdr;
    let mdat_payload_end = mdat.start + mdat.size;

    // Every chunk offset + running sample sizes stays within the file AND lands
    // inside the mdat payload.
    let stsc = parse_stsc(&out, &sc);
    let chunk_offsets = parse_chunk_offsets(&out, &sc);
    let num_chunks = chunk_offsets.len();
    let mut spc = vec![0u32; num_chunks];
    for (i, &(first_chunk, samples_per_chunk, _)) in stsc.iter().enumerate() {
        let last_chunk = if i + 1 < stsc.len() {
            stsc[i + 1].0
        } else {
            num_chunks as u32 + 1
        };
        for c in first_chunk..last_chunk {
            if (c as usize) >= 1 && (c as usize) <= num_chunks {
                spc[(c - 1) as usize] = samples_per_chunk;
            }
        }
    }

    let mut sample_idx = 0usize;
    let mut total_track_bytes = 0u64;
    for (c, &chunk_off) in chunk_offsets.iter().enumerate() {
        let mut pos = chunk_off as usize;
        assert!(
            pos >= mdat_payload_start && pos < mdat_payload_end,
            "chunk {c} offset {pos} outside mdat payload [{mdat_payload_start},{mdat_payload_end})"
        );
        for _ in 0..spc[c] {
            let sz = sizes[sample_idx] as usize;
            pos += sz;
            total_track_bytes += sz as u64;
            assert!(
                pos <= mdat_payload_end,
                "sample {sample_idx} overruns mdat payload"
            );
            assert!(pos <= out.len(), "sample {sample_idx} overruns file");
            sample_idx += 1;
        }
    }
    assert_eq!(sample_idx as u32, EXPECTED_VIDEO_SAMPLES);

    // Sum of parsed sizes == total mdat bytes consumed by this track's chunks.
    let sum_sizes: u64 = sizes.iter().map(|&s| s as u64).sum();
    assert_eq!(
        sum_sizes, total_track_bytes,
        "stsz sum vs chunk-consumed bytes"
    );

    // stss (if present) lists exactly the keyframe indices we produced.
    // Recompute expected keyframes from the IR.
    let media = demux_media();
    let vid = media
        .tracks
        .iter()
        .find(|t| matches!(t.spec.config, CodecConfig::Avc { .. }))
        .expect("ir video track");
    let all_sync = vid.samples.iter().all(|s| s.flags.is_sync);
    let stss = parse_stss(&out, &sc);
    if all_sync {
        assert!(stss.is_none(), "stss must be omitted when all samples sync");
    } else {
        let expected: Vec<u32> = vid
            .samples
            .iter()
            .enumerate()
            .filter_map(|(i, s)| {
                if s.flags.is_sync {
                    Some(i as u32 + 1)
                } else {
                    None
                }
            })
            .collect();
        assert_eq!(stss.expect("stss present"), expected, "stss keyframe list");
    }
}

// ---------------------------------------------------------------------------
// Test 3: video sample fidelity vs the ffmpeg reference mp4.
// ---------------------------------------------------------------------------

#[test]
fn video_samples_byte_identical_to_ref() {
    let out = package(true);
    let refmp4 = ref_mp4();

    let out_samples = resolve_samples(&out, video_trak(&out));
    let ref_samples = resolve_samples(&refmp4, video_trak(&refmp4));

    assert_eq!(
        out_samples.len(),
        EXPECTED_VIDEO_SAMPLES as usize,
        "our video sample count"
    );
    assert_eq!(
        ref_samples.len(),
        out_samples.len(),
        "ref vs ours video sample count"
    );
    for (i, (a, b)) in out_samples.iter().zip(ref_samples.iter()).enumerate() {
        assert_eq!(
            a,
            b,
            "video sample {i} differs from ref (len {} vs {})",
            a.len(),
            b.len()
        );
    }
}

// ---------------------------------------------------------------------------
// Test 4: avcC config reuse — byte-identical to the reference's avcC body.
// ---------------------------------------------------------------------------

fn avcc_body(file: &[u8]) -> Vec<u8> {
    let vtrak = video_trak(file);
    let sc = stbl_children(file, vtrak);
    let stsd = *find(&sc, b"stsd").expect("stsd");
    // stsd body: version+flags(4) + entry_count(4) then the first sample entry.
    let entry_start_in_body = 8usize;
    // The sample entry is a box; find avcC among its child boxes. The avc1
    // sample entry has a fixed prefix before its child boxes:
    //   size(4)+type(4) + 6 reserved + 2 data_ref_idx + 16 predefined/reserved
    //   + 2 width + 2 height + 4 horizres + 4 vertres + 4 reserved + 2 frame_count
    //   + 32 compressorname + 2 depth + 2 predefined = 86 bytes total box prefix.
    let entry_abs = stsd.start + stsd.hdr + entry_start_in_body;
    let entry_size = be32(file, entry_abs) as usize;
    let entry = &file[entry_abs..entry_abs + entry_size];
    // Walk child boxes starting after the 86-byte visual-sample-entry prefix.
    const VISUAL_PREFIX: usize = 86;
    let mut o = VISUAL_PREFIX;
    while o + 8 <= entry.len() {
        let sz = be32(entry, o) as usize;
        let typ = &entry[o + 4..o + 8];
        if typ == b"avcC" {
            // Return the box body (after size+type).
            return entry[o + 8..o + sz].to_vec();
        }
        if sz < 8 {
            break;
        }
        o += sz;
    }
    panic!("avcC not found in avc1 sample entry");
}

#[test]
fn avcc_matches_ref() {
    let out = package(true);
    let refmp4 = ref_mp4();
    let ours = avcc_body(&out);
    let theirs = avcc_body(&refmp4);
    assert!(!ours.is_empty(), "our avcC body non-empty");
    assert_eq!(ours, theirs, "avcC body must match the reference");
}

// ---------------------------------------------------------------------------
// Test 5: faststart:false yields identical sample bytes (only box order differs).
// ---------------------------------------------------------------------------

#[test]
fn faststart_false_same_samples_different_order() {
    let fast = package(true);
    let slow = package(false);

    // Box order differs: slow has mdat before moov.
    let slow_tops = top_boxes(&slow);
    let moov = find(&slow_tops, b"moov").expect("moov");
    let mdat = find(&slow_tops, b"mdat").expect("mdat");
    assert!(
        mdat.start < moov.start,
        "faststart:false: mdat (@{}) must precede moov (@{})",
        mdat.start,
        moov.start
    );

    // But the resolved video samples are byte-identical between the two.
    let fast_samples = resolve_samples(&fast, video_trak(&fast));
    let slow_samples = resolve_samples(&slow, video_trak(&slow));
    assert_eq!(
        fast_samples, slow_samples,
        "sample bytes must be identical regardless of faststart"
    );
    assert_eq!(fast_samples.len(), EXPECTED_VIDEO_SAMPLES as usize);
}

// ---------------------------------------------------------------------------
// r05-W24: a duration past u32::MAX promotes its header box to version 1
// ---------------------------------------------------------------------------

/// A movie timescale large enough (2 GHz, as in
/// `tests/fixtures/mp4/cenc_boxes/v1_mvhd.mp4`) that a few minutes of media
/// overflows `u32::MAX` ticks — the real-world trigger for the version-1
/// `mvhd`/`tkhd`/`mdhd` layouts.
const HUGE_TIMESCALE: u32 = 2_000_000_000;

fn huge_timescale_media() -> Media {
    let mut media = demux_media();
    // Give the video track and the movie a huge timescale so `mdhd.duration`,
    // the *presentation* duration derived from it, and therefore `mvhd.duration`
    // all overflow 32 bits: the sample durations are ~3000 ticks each, and
    // 1000 × 2 000 000 000 / 90 000 ticks per sample is well past `u32::MAX`.
    // The same sample durations in the *huge* track timescale are the large
    // values the v0 serializer used to truncate.
    for track in &mut media.tracks {
        let old_ts = u64::from(track.spec.timescale);
        let scale = u64::from(HUGE_TIMESCALE) / old_ts;
        track.spec.timescale = HUGE_TIMESCALE;
        track.start_decode_time = track.start_decode_time.saturating_mul(scale);
        for s in &mut track.samples {
            s.dts = s.dts.map(|v| v.saturating_mul(scale as i64));
            s.pts = s.pts.map(|v| v.saturating_mul(scale as i64));
            s.duration = s.duration.map(|d| d.saturating_mul(scale as u32));
        }
    }
    media.movie_timescale = HUGE_TIMESCALE;
    media
}

/// A version-0 header truncates its 64-bit duration with `as u32`, so a
/// duration past `u32::MAX` used to wrap (a 10 MHz Smooth-sourced track wrapped
/// after 7.2 minutes; 90 kHz after 13.2 h) and players reported a wrong length.
/// Every header must come out version 1 with the full duration.
#[test]
fn oversized_durations_promote_headers_to_version_1() {
    let media = huge_timescale_media();
    let out = ProgressiveMux { faststart: true }
        .package(&media)
        .expect("package progressive");

    let moov = *find(&top_boxes(&out), b"moov").expect("moov");
    let moov_children = children(moov.start + moov.hdr, moov.body(&out));
    let mvhd = *find(&moov_children, b"mvhd").expect("mvhd");
    assert_eq!(out[mvhd.start + 8], 1, "mvhd must be version 1");
    assert_eq!(mvhd.size, 120, "version-1 mvhd is 120 bytes");
    // v1 layout: version/flags(4) creation(8) modification(8) timescale(4)
    // then duration(8) at body offset 24.
    let mvhd_duration = be64(out.as_slice(), mvhd.start + 8 + 24);
    assert!(
        mvhd_duration > u64::from(u32::MAX),
        "mvhd.duration {mvhd_duration} must survive at 64 bits"
    );

    let trak_list = traks(&out);
    assert_eq!(trak_list.len(), 2);
    for trak in trak_list {
        let trak_children = children(trak.start + trak.hdr, trak.body(&out));
        let tkhd = *find(&trak_children, b"tkhd").expect("tkhd");
        assert_eq!(
            out[tkhd.start + 8],
            1,
            "tkhd must be version 1 (size {})",
            tkhd.size
        );
        assert_eq!(tkhd.size, 104, "version-1 tkhd is 104 bytes");
        let mdia = *find(&trak_children, b"mdia").expect("mdia");
        let mdia_children = children(mdia.start + mdia.hdr, mdia.body(&out));
        let mdhd = *find(&mdia_children, b"mdhd").expect("mdhd");
        assert_eq!(out[mdhd.start + 8], 1, "mdhd must be version 1");
        assert_eq!(mdhd.size, 44, "version-1 mdhd is 44 bytes");
    }
}

/// The boundary: a duration that fits `u32::MAX` stays version 0.
#[test]
fn small_durations_stay_version_0() {
    let out = package(true);
    let moov = *find(&top_boxes(&out), b"moov").expect("moov");
    let mvhd = *find(&children(moov.start + moov.hdr, moov.body(&out)), b"mvhd").expect("mvhd");
    assert_eq!(out[mvhd.start + 8], 0, "mvhd stays version 0");
    assert_eq!(mvhd.size, 108, "version-0 mvhd is 108 bytes");
}

/// Independent oracle: ffmpeg must read the promoted file with its two
/// streams. Skips cleanly when `ffprobe` is absent.
#[test]
fn version_1_headers_are_ffprobe_readable() {
    if std::process::Command::new("ffprobe")
        .arg("-version")
        .output()
        .is_err()
    {
        eprintln!("SKIP version_1_headers_are_ffprobe_readable: ffprobe not on PATH");
        return;
    }
    let media = huge_timescale_media();
    let out = ProgressiveMux { faststart: true }
        .package(&media)
        .expect("package progressive");
    let path = std::env::temp_dir().join(format!("transmux-w24-{}.mp4", std::process::id()));
    std::fs::write(&path, &out).expect("write");
    let probe = std::process::Command::new("ffprobe")
        .args([
            "-v",
            "error",
            "-show_entries",
            "stream=index,codec_name",
            "-of",
            "csv",
            path.to_str().expect("utf-8 path"),
        ])
        .output()
        .expect("run ffprobe");
    let stdout = String::from_utf8_lossy(&probe.stdout).into_owned();
    let stderr = String::from_utf8_lossy(&probe.stderr).into_owned();
    let _ = std::fs::remove_file(&path);
    assert!(
        stderr.is_empty(),
        "ffprobe -v error must be silent, got: {stderr}"
    );
    assert_eq!(stdout.lines().count(), 2, "two streams read: {stdout}");
}

// ---------------------------------------------------------------------------
// r05-W25: the mdat is interleaved, not one chunk per track
// ---------------------------------------------------------------------------

/// With one chunk per track, starting playback needs the first video *and*
/// first audio sample, which sit roughly a whole video track apart — so
/// `faststart` bought nothing for a forward-only client. The tracks' chunks
/// must now interleave in time (audit r05-W25).
#[test]
fn mdat_is_interleaved_across_tracks() {
    let out = package(true);
    let trak_list = traks(&out);
    assert_eq!(trak_list.len(), 2, "video + audio");

    let video = video_trak(&out);
    let audio = *trak_list
        .iter()
        .find(|t| !is_video_trak(&out, **t))
        .expect("audio trak");

    let video_offsets = parse_chunk_offsets(&out, &stbl_children(&out, video));
    let audio_offsets = parse_chunk_offsets(&out, &stbl_children(&out, audio));
    assert!(
        video_offsets.len() > 1 && audio_offsets.len() > 1,
        "each track needs more than one chunk: video {} audio {}",
        video_offsets.len(),
        audio_offsets.len()
    );

    // Every chunk offset must be distinct — a shared offset would mean two
    // chunks claim the same bytes.
    let mut all: Vec<u64> = video_offsets
        .iter()
        .chain(&audio_offsets)
        .copied()
        .collect();
    all.sort_unstable();
    let unique = all.len();
    all.dedup();
    assert_eq!(all.len(), unique, "chunk offsets must be distinct");

    // The audio track's first chunk must land before the video track's *last*
    // one, i.e. the two tracks are genuinely interleaved.
    let video_last = *video_offsets.last().unwrap();
    assert!(
        audio_offsets[0] < video_last,
        "audio's first chunk (@{}) must precede video's last (@{video_last}):          the mdat is not interleaved",
        audio_offsets[0]
    );

    // And the samples-per-chunk runs must describe the whole video track.
    // `stsc` runs are keyed by first chunk, so expand them back to one count
    // per chunk and check the track is covered exactly once.
    let runs = parse_stsc(&out, &stbl_children(&out, video));
    assert!(
        runs.iter().all(|&(_, _, sdi)| sdi == 1),
        "every chunk uses sample description 1"
    );
    let mut per_chunk = vec![0u32; video_offsets.len()];
    for (i, &(first_chunk, count, _)) in runs.iter().enumerate() {
        assert!(first_chunk >= 1, "chunk numbers are 1-based");
        let last = runs
            .get(i + 1)
            .map(|r| r.0)
            .unwrap_or(u32::try_from(video_offsets.len() + 1).unwrap());
        let mut c = first_chunk as usize - 1;
        while c < last as usize - 1 {
            per_chunk[c] = count;
            c += 1;
        }
    }
    let video_total: u32 = per_chunk.iter().sum();
    assert_eq!(
        video_total, EXPECTED_VIDEO_SAMPLES,
        "stsc must cover every sample exactly once: {per_chunk:?}"
    );
}

/// Independent oracle: ffmpeg must decode the interleaved file and report the
/// same sample count as the one-chunk layout did. Skips without `ffprobe`.
#[test]
fn interleaved_output_is_ffprobe_readable() {
    if std::process::Command::new("ffprobe")
        .arg("-version")
        .output()
        .is_err()
    {
        eprintln!("SKIP interleaved_output_is_ffprobe_readable: ffprobe not on PATH");
        return;
    }
    let out = package(true);
    let path = std::env::temp_dir().join(format!("transmux-w25-{}.mp4", std::process::id()));
    std::fs::write(&path, &out).expect("write");
    let probe = std::process::Command::new("ffprobe")
        .args([
            "-v",
            "error",
            "-count_packets",
            "-show_entries",
            "stream=index,codec_name,nb_read_packets",
            "-of",
            "csv",
            path.to_str().expect("utf-8 path"),
        ])
        .output()
        .expect("run ffprobe");
    let stdout = String::from_utf8_lossy(&probe.stdout).into_owned();
    let stderr = String::from_utf8_lossy(&probe.stderr).into_owned();
    let _ = std::fs::remove_file(&path);
    assert!(
        stderr.is_empty(),
        "ffprobe -v error must be silent, got: {stderr}"
    );
    assert!(
        stdout.contains(&EXPECTED_VIDEO_SAMPLES.to_string()),
        "ffprobe must see all {EXPECTED_VIDEO_SAMPLES} video packets: {stdout}"
    );
}

// ---------------------------------------------------------------------------
// Item 8: content oracle, duration oracle, and the version/stco boundaries
// ---------------------------------------------------------------------------

/// `-show_packets -show_data_hash sha256` on the interleaved output must equal
/// the same on `ffmpeg -c copy` of the source: a wrong `stco`/`co64` still
/// passes a packet *count*, so the packet payloads are the real oracle for the
/// chunk layout. Skips cleanly when ffmpeg/ffprobe are absent.
#[test]
fn interleaved_chunk_contents_match_the_reference() {
    if std::process::Command::new("ffprobe")
        .arg("-version")
        .output()
        .is_err()
        || std::process::Command::new("ffmpeg")
            .arg("-version")
            .output()
            .is_err()
    {
        eprintln!(
            "SKIP interleaved_chunk_contents_match_the_reference: ffprobe/ffmpeg not on PATH"
        );
        return;
    }
    let dir = std::env::temp_dir().join(format!("transmux-h8-{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("temp dir");

    // Our interleaved output.
    let ours = package(true);
    let ours_path = dir.join("ours.mp4");
    std::fs::write(&ours_path, &ours).expect("write");

    // The reference: the same TS, remuxed by ffmpeg (one chunk per track is
    // fine — the packet contents are what is compared).
    let ts = fixture(&["fixtures", "ts", "h264_aac.ts"]);
    let ts_path = dir.join("src.ts");
    std::fs::write(&ts_path, &ts).expect("write ts");
    let ref_path = dir.join("ref.mp4");
    let status = std::process::Command::new("ffmpeg")
        .args(["-v", "error", "-y", "-i"])
        .arg(&ts_path)
        .args(["-c", "copy", "-map", "0:v:0"])
        .arg(&ref_path)
        .status()
        .expect("spawn ffmpeg");
    assert!(status.success(), "ffmpeg reference mux failed");

    let hashes = |path: &std::path::Path| -> Vec<String> {
        let out = std::process::Command::new("ffprobe")
            .args([
                "-v",
                "error",
                "-select_streams",
                "v:0",
                "-show_packets",
                "-show_data_hash",
                "sha256",
                "-show_entries",
                "packet=data_hash",
                "-of",
                "csv=p=0",
            ])
            .arg(path)
            .output()
            .expect("run ffprobe");
        String::from_utf8_lossy(&out.stdout)
            .lines()
            .filter(|l| !l.trim().is_empty())
            .map(|l| l.trim().to_string())
            .collect()
    };
    let our_hashes = hashes(&ours_path);
    let ref_hashes = hashes(&ref_path);
    let _ = std::fs::remove_dir_all(&dir);

    assert_eq!(
        our_hashes.len(),
        EXPECTED_VIDEO_SAMPLES as usize,
        "ffprobe must see every video packet in our output"
    );
    assert_eq!(
        our_hashes, ref_hashes,
        "every video packet's SHA-256 must match the independent remux — a \
         wrong chunk offset would shift or truncate the payloads"
    );
}

/// `format=duration` on the version-1-promoted file must equal the duration
/// ffmpeg derives from the sample count, i.e. the 64-bit `mvhd.duration` is
/// really read (a truncated one would come back as a wrapped, tiny value).
#[test]
fn promoted_v1_duration_is_read_by_ffprobe() {
    if std::process::Command::new("ffprobe")
        .arg("-version")
        .output()
        .is_err()
    {
        eprintln!("SKIP promoted_v1_duration_is_read_by_ffprobe: ffprobe not on PATH");
        return;
    }
    let media = huge_timescale_media();
    let out = ProgressiveMux { faststart: true }
        .package(&media)
        .expect("package progressive");
    let path = std::env::temp_dir().join(format!("transmux-h8b-{}.mp4", std::process::id()));
    std::fs::write(&path, &out).expect("write");
    let probe = std::process::Command::new("ffprobe")
        .args([
            "-v",
            "error",
            "-select_streams",
            "v:0",
            "-show_entries",
            "stream=duration",
            "-of",
            "csv=p=0",
            path.to_str().expect("utf-8 path"),
        ])
        .output()
        .expect("run ffprobe");
    let _ = std::fs::remove_file(&path);
    let secs: f64 = String::from_utf8_lossy(&probe.stdout)
        .trim()
        .parse()
        .unwrap_or(f64::NAN);
    // The fixture is ~3 s of media; a wrapped 32-bit duration would report a
    // fraction of a second (or a huge value), not this.
    assert!(
        (2.0..=5.0).contains(&secs),
        "ffprobe must report the real duration, got {secs}"
    );
}

/// The `duration == u32::MAX` boundary: exactly at the limit stays version 0,
/// one tick past it promotes to version 1.
#[test]
fn duration_version_boundary_is_exact() {
    use transmux::MovieHeaderBox;

    let at_limit = MovieHeaderBox {
        version: 0,
        flags: 0,
        creation_time: 0,
        modification_time: 0,
        timescale: 1000,
        duration: u64::from(u32::MAX),
        rate: 0x0001_0000,
        volume: 0x0100,
        matrix: [0x0001_0000, 0, 0, 0, 0x0001_0000, 0, 0, 0, 0x4000_0000],
        next_track_id: 2,
    };
    let b = at_limit.try_to_bytes().expect("serialize at the limit");
    assert_eq!(b.len(), 108, "u32::MAX fits version 0");
    let p = MovieHeaderBox::parse(&b).expect("parse");
    assert_eq!(p.duration, u64::from(u32::MAX));
    assert_eq!(p.version, 0);

    let over = MovieHeaderBox {
        duration: u64::from(u32::MAX) + 1,
        version: 1,
        ..at_limit
    };
    let b = over.try_to_bytes().expect("serialize past the limit");
    assert_eq!(b.len(), 120, "past the limit is version 1");
    let p = MovieHeaderBox::parse(&b).expect("parse");
    assert_eq!(p.duration, u64::from(u32::MAX) + 1, "the full value");
    assert_eq!(p.version, 1);
}

/// The `stco` → `co64` switch: a chunk offset at exactly `u32::MAX` still fits
/// `stco`, one past it must select `co64`. The threshold itself is unit-tested
/// (`progressive::tests::co64_chosen_once_an_offset_exceeds_u32`); this pins
/// the *wiring* — a file small enough to fit `stco` must not use `co64` (a
/// needless 8-byte-per-chunk cost, and a `co64`-vs-`stco` mismatch is exactly
/// the shape that makes a chunk offset 4 bytes early).
#[test]
fn a_small_file_uses_stco_not_co64() {
    for faststart in [true, false] {
        let out = package(faststart);
        let sc = stbl_children(&out, video_trak(&out));
        assert!(
            find(&sc, b"stco").is_some(),
            "faststart={faststart}: a sub-4 GiB file must use stco"
        );
        assert!(
            find(&sc, b"co64").is_none(),
            "faststart={faststart}: co64 is only for offsets past u32::MAX"
        );
        // And every stco entry must resolve inside the file.
        for off in parse_chunk_offsets(&out, &sc) {
            assert!(
                (off as usize) < out.len(),
                "faststart={faststart}: chunk offset {off} is past the file"
            );
        }
    }
}

// ---------------------------------------------------------------------------
// Round-3 item 5: a non-monotonic decode time must not fail packaging
// ---------------------------------------------------------------------------

/// A source whose `dts` steps *backwards* partway (a TS discontinuity, a
/// spliced `Media`) is legal input. The chunk merge keyed on
/// `(start tick, track, sample)` walked such a track's chunks out of their own
/// sample order and returned `InvalidInput`; ticks are now only the
/// cross-track interleave criterion (audit item 5).
#[test]
fn non_monotonic_dts_still_packages() {
    let mut media = demux_media();
    let video = media
        .tracks
        .iter()
        .position(|t| matches!(t.spec.config, CodecConfig::Avc { .. }))
        .expect("a video track");
    let ts = i64::from(media.tracks[video].spec.timescale.max(1));
    // Sample 0 sits at t=0 and the rest advance 40 ms each, except that a run
    // jumps *forward* 60 s and the samples after it resume roughly where they
    // were — so one chunk start lands far ahead and a later chunk start lands
    // *behind* it. That is the shape that makes a merge keyed on ticks walk a
    // track's chunks out of its own sample order (audit item 5, round 3).
    let samples = &mut media.tracks[video].samples;
    let step = ts / 25; // 40 ms
    // A sample advance that never lets a chunk span more than one sample: the
    // interleave window is 0.5 s, so a 1 s step makes every sample its own
    // chunk and each chunk start its own sample tick.
    // Chunk starts land on samples 0,1,2,4,5,7,8,10,… (a dip sample merges
    // into the chunk before it), so the tick of *those* samples is what the
    // merge sees. Make the chips: sample 5 and 8 open chunks *behind* the
    // chunk at sample 4.
    for (i, s) in samples.iter_mut().enumerate() {
        let base = ts * i as i64;
        // Samples 5, 6 and 10 open chunks with *equal or lower* ticks than the
        // chunk before them: the old key `(tick, track, first)` then sorted a
        // track's own chunks out of sample order and rejected the input. (A
        // `Media` whose `dts` steps back is legal — a TS discontinuity or a
        // spliced stream does exactly this.)
        let t = match i {
            4 | 7 => 0,
            _ => base,
        };
        s.dts = Some(t);
        s.pts = Some(t);
        let _ = step;
    }

    let out = ProgressiveMux { faststart: true }
        .package(&media)
        .expect("a backwards dts step must still package");

    // Every sample must still be present and resolvable.
    let locs = top_boxes(&out);
    let mdat = find(&locs, b"mdat").expect("mdat");
    let mdat_payload = mdat.start + mdat.hdr;
    let mdat_end = mdat.start + mdat.size;
    let trak = video_trak(&out);
    let sc = stbl_children(&out, trak);
    let sizes = parse_stsz(&out, &sc);
    let offsets = parse_chunk_offsets(&out, &sc);
    let runs = parse_stsc(&out, &sc);
    assert_eq!(sizes.len(), EXPECTED_VIDEO_SAMPLES as usize, "all samples");

    // Expand the `stsc` runs back to one count per chunk, then check every
    // declared range lands inside the `mdat`.
    let mut per_chunk = vec![0u32; offsets.len()];
    for (i, &(first_chunk, count, sdi)) in runs.iter().enumerate() {
        assert_eq!(sdi, 1, "every chunk uses sample description 1");
        let last = runs
            .get(i + 1)
            .map(|r| r.0)
            .unwrap_or(u32::try_from(offsets.len() + 1).unwrap());
        let mut c = first_chunk as usize - 1;
        while c < last as usize - 1 && c < per_chunk.len() {
            per_chunk[c] = count;
            c += 1;
        }
    }
    let mut sample = 0usize;
    for (chunk, &count) in per_chunk.iter().enumerate() {
        let start = offsets[chunk] as usize;
        let count = count as usize;
        assert!(
            sample + count <= sizes.len(),
            "chunk {chunk} runs past the sample table"
        );
        let len: usize = sizes[sample..sample + count]
            .iter()
            .map(|&s| s as usize)
            .sum();
        assert!(
            start >= mdat_payload && start + len <= mdat_end,
            "chunk {chunk} range {start}..{} outside the mdat {mdat_payload}..{mdat_end}",
            start + len
        );
        sample += count;
    }
    assert_eq!(sample, EXPECTED_VIDEO_SAMPLES as usize, "payloads covered");
}
