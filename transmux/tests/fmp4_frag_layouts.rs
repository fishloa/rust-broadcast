//! Oracle consumer: `Fmp4Demux` fragment addressing (audit r05-W7).
//!
//! Consumes the CLEAR variants of `tests/fixtures/cenc_frag_layouts/` — the
//! three addressing forms a real muxer writes (`clear_ffmpeg_{default,
//! base_moof,omit}.mp4`) plus the two hand-split multi-trun files
//! (`clear_multitrun_{base_moof,omit}_implicit.mp4`; no tool emits multiple
//! truns per traf — see the fixture README). Every file must demux to the same
//! per-sample plaintext as `clear.mp4` (whose per-sample hashes are pinned in
//! `clear_samples.txt`), for **all** tracks.
//!
//! `Fmp4Demux` shares its §8.8.7/§8.8.8 resolver with `CencDecryptor`
//! (`frag_offsets`), so this proves the demuxer's side of the same rule set.
#![cfg(feature = "std")]

use broadcast_common::Unpackage;
use transmux::{CodecConfig, Fmp4Demux, Media};

const DIR: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/tests/fixtures/cenc_frag_layouts"
);

fn read(name: &str) -> Vec<u8> {
    let p = format!("{DIR}/{name}");
    std::fs::read(&p).unwrap_or_else(|e| panic!("read {p}: {e}"))
}

fn demux(name: &str) -> Media {
    let bytes = read(name);
    let mut d = Fmp4Demux::new();
    d.unpackage(bytes.as_slice())
        .unwrap_or_else(|e| panic!("{name}: demux: {e}"))
}

/// `(track_id, per-sample bytes)` for every track, in moov order.
fn by_track(m: &Media) -> Vec<(u32, Vec<Vec<u8>>)> {
    m.tracks
        .iter()
        .map(|t| {
            (
                t.spec.track_id,
                t.samples.iter().map(|s| s.data.to_vec()).collect(),
            )
        })
        .collect()
}

/// Every clear layout must demux to the same samples (all tracks) as
/// `clear.mp4`.
#[test]
fn all_clear_layouts_demux_identically() {
    let reference = by_track(&demux("clear.mp4"));
    assert_eq!(reference.len(), 2, "clear.mp4 has video + audio");
    assert_eq!(reference[0].1.len(), 20, "video samples");
    assert_eq!(reference[1].1.len(), 95, "audio samples");
    assert!(
        reference
            .iter()
            .any(|(_, s)| s.iter().any(|b| matches!(b.len(), 3001 | 322))),
        "the reference must carry real video sample bytes"
    );

    // The clear variants: the three real-muxer forms plus the hand-split
    // multi-trun files (no tool emits multiple truns per traf).
    let files = [
        "clear_ffmpeg_default.mp4",
        "clear_ffmpeg_base_moof.mp4",
        "clear_ffmpeg_omit.mp4",
        "clear_multitrun_base_moof_implicit.mp4",
        "clear_multitrun_omit_implicit.mp4",
    ];
    for f in files {
        let got = by_track(&demux(f));
        assert_eq!(got.len(), reference.len(), "{f}: track count");
        for ((rt, rs), (gt, gs)) in reference.iter().zip(got.iter()) {
            assert_eq!(rt, gt, "{f}: track ids must match");
            assert_eq!(rs.len(), gs.len(), "{f}: track {gt} sample count");
            for (i, (a, b)) in rs.iter().zip(gs.iter()).enumerate() {
                assert_eq!(
                    a, b,
                    "{f}: track {gt} sample {i} must match clear.mp4 byte-for-byte"
                );
            }
        }
    }
}

/// The fixtures really are the layouts they claim: `clear.mp4` and
/// `clear_ffmpeg_base_moof.mp4` are the same bytes (per the fixture README),
/// and all clear variants decode to the same video sample count.
#[test]
fn clear_variants_carry_the_same_codec_config() {
    for f in [
        "clear.mp4",
        "clear_ffmpeg_default.mp4",
        "clear_ffmpeg_omit.mp4",
        "clear_multitrun_omit_implicit.mp4",
    ] {
        let m = demux(f);
        assert_eq!(m.tracks.len(), 2, "{f}: two tracks");
        assert!(
            matches!(m.tracks[0].spec.config, CodecConfig::Avc { .. }),
            "{f}: track 1 is AVC"
        );
        assert_eq!(m.tracks[0].samples.len(), 20, "{f}: 20 video samples");
        assert_eq!(m.tracks[1].samples.len(), 95, "{f}: 95 audio samples");
    }
}

// ---------------------------------------------------------------------------
// r05-W26: a truncated tail, and several `moof`s before one `mdat`
// ---------------------------------------------------------------------------

/// Find every top-level box `(four_cc, start, size)` in `data`.
fn top_boxes(data: &[u8]) -> Vec<([u8; 4], usize, usize)> {
    let mut out = Vec::new();
    let mut off = 0usize;
    while off + 8 <= data.len() {
        let size =
            u32::from_be_bytes([data[off], data[off + 1], data[off + 2], data[off + 3]]) as usize;
        if size < 8 || off + size > data.len() {
            break;
        }
        let mut ty = [0u8; 4];
        ty.copy_from_slice(&data[off + 4..off + 8]);
        out.push((ty, off, size));
        off += size;
    }
    out
}

/// A live capture or an interrupted recording ends mid-box. `parse_box` on the
/// cut-short tail used to fail the *whole* demux, so a mostly-complete
/// recording could not be read at all (audit r05-W26).
#[test]
fn truncated_final_box_is_tolerated() {
    let full = read("clear.mp4");
    let media = Fmp4Demux::new()
        .unpackage(full.as_slice())
        .expect("the whole file demuxes");
    let full_samples: usize = media.tracks.iter().map(|t| t.samples.len()).sum();
    assert!(full_samples > 0);

    // Cut the file mid-box: half of the last top-level box's body is gone.
    let boxes = top_boxes(&full);
    let (_, last_start, last_size) = *boxes.last().expect("last box");
    let cut = last_start + last_size / 2;
    let truncated = full[..cut].to_vec();
    // Make sure the cut lands inside the body, not exactly on the boundary.
    assert!(truncated.len() > last_start + 8, "cut inside the last box");

    let partial = Fmp4Demux::new()
        .unpackage(truncated.as_slice())
        .expect("a truncated tail must not fail the demux");
    let partial_samples: usize = partial.tracks.iter().map(|t| t.samples.len()).sum();
    assert!(
        partial_samples > 0,
        "the fragments before the cut must still be demuxed"
    );
    assert!(
        partial_samples <= full_samples,
        "a truncated file cannot yield more samples than the whole one \
         ({partial_samples} > {full_samples})"
    );
}

/// `moof`, `moof`, `mdat` is legal (ISO/IEC 14496-12:2015 §8.8.4 allows any
/// number of `moof`s, and a multi-track CMF2 segment writes one per track).
/// A single pending-`moof` slot overwrote the first and silently dropped its
/// fragment (audit r05-W26).
///
/// The two synthetic fragments use an explicit `tfhd.base_data_offset` so
/// their addressing is an absolute file offset — independent of where the
/// `moof` itself sits, which is what lets the second one be queued ahead of
/// the `mdat` at all (§8.8.7.1; `default-base-is-moof` is relative to the
/// `moof`, so it cannot share an `mdat` this way).
#[test]
fn two_moofs_before_one_mdat_keeps_both_fragments() {
    use broadcast_common::Serialize;
    use transmux::movie_fragment::TFHD_BASE_DATA_OFFSET_PRESENT;
    use transmux::{
        MovieFragmentBox, MovieFragmentHeaderBox, TrackFragmentBaseMediaDecodeTimeBox,
        TrackFragmentBox, TrackFragmentHeaderBox, TrackFragmentRunBox, TrunSample,
    };

    // The init movie comes from the real fixture, so the track this fragment
    // addresses actually exists.
    let full = read("clear.mp4");
    let boxes = top_boxes(&full);
    let init_end = boxes
        .iter()
        .find(|(ty, ..)| *ty == *b"moof")
        .expect("a moof")
        .1;
    let init = full[..init_end].to_vec();

    // One sample per fragment, sized to the fixture track's own first sample
    // (`frag_offsets` resolves the run's byte range from the trun's declared
    // sizes, so a mismatch here is a range error, not a sample-content one).
    let sample_len = track1_first_sample_len(&full) as usize;
    let sample_a = vec![0xAAu8; sample_len];
    let sample_b = vec![0xBBu8; sample_len];
    let mut moofs: Vec<Vec<u8>> = Vec::new();
    for (i, sample) in [&sample_a, &sample_b].iter().enumerate() {
        let trun = TrackFragmentRunBox {
            version: 0,
            tr_flags: transmux::movie_fragment::TRUN_SAMPLE_SIZE_PRESENT,
            data_offset: None,
            first_sample_flags: None,
            samples: vec![TrunSample {
                sample_duration: None,
                sample_size: Some(sample.len() as u32),
                sample_flags: None,
                sample_composition_time_offset: None,
            }],
        };
        let mut moof = MovieFragmentBox::new(
            MovieFragmentHeaderBox::new(i as u32 + 1),
            vec![TrackFragmentBox::new(
                TrackFragmentHeaderBox {
                    // An explicit base pointing at the single `mdat` payload.
                    flags: TFHD_BASE_DATA_OFFSET_PRESENT,
                    track_id: 1,
                    base_data_offset: Some(0),
                    sample_description_index: None,
                    default_sample_duration: None,
                    default_sample_size: None,
                    default_sample_flags: None,
                },
                Some(TrackFragmentBaseMediaDecodeTimeBox::new_v0(i as u32 * 1024)),
                vec![trun],
            )],
        );
        // `serialized_len` depends on the base_data_offset flag only, so a
        // first pass fixes the moof's own size; the base then points at the
        // mdat payload, which starts after both moofs.
        let _ = &mut moof;
        moofs.push(moof.to_bytes());
    }

    // `mdat`: 8-byte header + both samples, and both moofs come first (after
    // the init segment).
    let payload_len = sample_len * 2;
    let mdat_start = init.len() + moofs.iter().map(Vec::len).sum::<usize>();
    let mut mdat = Vec::new();
    mdat.extend_from_slice(&((8 + payload_len) as u32).to_be_bytes());
    mdat.extend_from_slice(b"mdat");
    mdat.extend_from_slice(&sample_a);
    mdat.extend_from_slice(&sample_b);

    // Patch each moof's base and data offset to the `mdat` payload.
    for (i, moof) in moofs.iter_mut().enumerate() {
        let base = (mdat_start + 8 + i * sample_len) as u64;
        patch_tfhd_base_and_offset(moof, base);
    }

    // Control: init + the first moof + the mdat, with the first moof's base
    // (which is what its `tfhd` was patched to) unchanged.
    let mut control = init.clone();
    control.extend_from_slice(&moofs[0]);
    control.extend_from_slice(&mdat);

    let control_samples: usize = Fmp4Demux::new()
        .unpackage(control.as_slice())
        .expect("moof + mdat")
        .tracks
        .iter()
        .map(|t| t.samples.len())
        .sum();
    assert_eq!(
        control_samples, 1,
        "the control fragment carries one sample"
    );

    let mut two = init;
    two.extend_from_slice(&moofs[0]);
    two.extend_from_slice(&moofs[1]);
    two.extend_from_slice(&mdat);
    let two_media = Fmp4Demux::new()
        .unpackage(two.as_slice())
        .expect("moof, moof, mdat");
    let two_samples: usize = two_media.tracks.iter().map(|t| t.samples.len()).sum();
    assert_eq!(
        two_samples, 2,
        "both queued `moof`s must resolve; the first one used to be dropped"
    );
}

/// The byte length of track 1's first sample, taken from the fixture's own
/// `trun` (so the synthetic fragment's addressing matches the real track).
fn track1_first_sample_len(file: &[u8]) -> u64 {
    let boxes = top_boxes(file);
    let (_, off, size) = *boxes
        .iter()
        .find(|(ty, ..)| *ty == *b"moof")
        .expect("a moof");
    let moof = transmux::MovieFragmentBox::parse_body(&file[off + 8..off + size])
        .expect("parse moof body");
    let trun = &moof.traf[0].trun[0];
    u64::from(trun.samples[0].sample_size.expect("sample_size present"))
}

/// Patch a serialized `moof` so its `tfhd` carries `base_data_offset = base`
/// and its sole `trun` starts there (relative to the traf base, §8.8.8).
///
/// The boxes were built with `TFHD_BASE_DATA_OFFSET_PRESENT` and no
/// `TRUN_DATA_OFFSET_PRESENT`, so only the 8-byte base field and the box
/// sizes need rewriting — `trun.data_offset` is absent, meaning "continue
/// after the previous run", which for the first run means the traf base.
fn patch_tfhd_base_and_offset(moof: &mut [u8], base: u64) {
    // moof(8) + mfhd(16) + traf(8) + tfhd(8 + 4 + 4 + 8)
    let base_at = 8 + 16 + 8 + 8 + 4 + 4;
    moof[base_at..base_at + 8].copy_from_slice(&base.to_be_bytes());
}

/// Independent oracle: the truncated file must still be readable by ffprobe
/// (which also tolerates a short tail), so `Fmp4Demux` is not accepting
/// something no real reader accepts. Skips without `ffprobe`.
#[test]
fn truncated_tail_matches_ffprobe_success() {
    if std::process::Command::new("ffprobe")
        .arg("-version")
        .output()
        .is_err()
    {
        eprintln!("SKIP truncated_tail_matches_ffprobe_success: ffprobe not on PATH");
        return;
    }
    let full = read("clear.mp4");
    let boxes = top_boxes(&full);
    let (_, last_start, last_size) = *boxes.last().expect("last box");
    let truncated = full[..last_start + last_size / 2].to_vec();
    let path = std::env::temp_dir().join(format!("transmux-w26-{}.mp4", std::process::id()));
    std::fs::write(&path, &truncated).expect("write");
    let probe = std::process::Command::new("ffprobe")
        .args([
            "-v",
            "error",
            "-show_entries",
            "stream=index",
            "-of",
            "csv",
            path.to_str().expect("utf-8 path"),
        ])
        .output()
        .expect("run ffprobe");
    let stdout = String::from_utf8_lossy(&probe.stdout).into_owned();
    let _ = std::fs::remove_file(&path);
    assert!(
        stdout.lines().count() >= 2,
        "ffprobe reads both streams from the truncated file: {stdout}"
    );
}

/// A mid-file box whose declared size is a *size underflow* (2..7, impossible
/// for a box) is a framing error: every box after it would be read from a
/// bogus offset. The `Ok(..) else break` walk treated it as end-of-data and
/// silently truncated the file at the first corrupt box (audit item 4).
#[test]
fn mid_file_size_underflow_box_is_an_error() {
    let full = read("clear.mp4");
    let boxes = top_boxes(&full);
    let (_, moof_at, moof_len) = *boxes
        .iter()
        .find(|(ty, ..)| *ty == *b"moof")
        .expect("a moof");

    // Replace the first `moof`'s size with 3 (below the 8-byte minimum, so no
    // header can be read from it).
    let mut corrupt = full.clone();
    corrupt[moof_at..moof_at + 4].copy_from_slice(&3u32.to_be_bytes());
    // Keep the rest of the file intact so a lenient walk would still find the
    // later fragments.
    assert!(moof_at + moof_len <= corrupt.len());

    let err = Fmp4Demux::new()
        .unpackage(corrupt.as_slice())
        .expect_err("a mid-file size underflow must be rejected");
    assert!(
        matches!(err, transmux::Error::BoxSizeUnderflow { .. }),
        "a size of 3 is a size underflow, not end-of-data; got {err:?}"
    );
}

/// A file cut *inside* the 24-byte header of a trailing `uuid` box is
/// truncated at the tail, exactly like a cut inside a normal box body: the
/// fragments before it must still demux (audit item 9). `parse_box` reports
/// this as `UuidBufferTooShort`, a distinct variant from the body case, which
/// the tail check has to cover too.
#[test]
fn a_tail_cut_inside_a_uuid_header_is_tolerated() {
    let full = read("clear.mp4");
    let boxes = top_boxes(&full);
    let init_end = boxes
        .iter()
        .find(|(ty, ..)| *ty == *b"moof")
        .expect("a moof")
        .1;
    let clean = &full[..init_end];
    let reference: usize = Fmp4Demux::new()
        .unpackage(clean)
        .expect("the init-only prefix demuxes")
        .tracks
        .iter()
        .map(|t| t.samples.len())
        .sum();

    // Append a `uuid` box header that claims an 8-byte body but is cut off
    // inside its 16-byte usertype.
    for cut in [10usize, 18, 22] {
        let mut truncated = clean.to_vec();
        let mut header = Vec::new();
        header.extend_from_slice(&24u32.to_be_bytes());
        header.extend_from_slice(b"uuid");
        truncated.extend_from_slice(&header);
        truncated.extend_from_slice(&vec![0xAB; cut - 8]);

        let media = Fmp4Demux::new()
            .unpackage(truncated.as_slice())
            .expect("a uuid header cut at the tail must not fail the demux");
        let got: usize = media.tracks.iter().map(|t| t.samples.len()).sum();
        assert_eq!(
            got, reference,
            "cut at {cut}: every fragment before the cut survives"
        );
    }
}
