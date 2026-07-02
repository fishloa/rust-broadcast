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

use broadcast_common::{Package, Unpackage};
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
    let all_sync = vid.samples.iter().all(|s| s.is_sync);
    let stss = parse_stss(&out, &sc);
    if all_sync {
        assert!(stss.is_none(), "stss must be omitted when all samples sync");
    } else {
        let expected: Vec<u32> = vid
            .samples
            .iter()
            .enumerate()
            .filter_map(|(i, s)| if s.is_sync { Some(i as u32 + 1) } else { None })
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
