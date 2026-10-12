// SPDX-License-Identifier: MIT OR Apache-2.0
//! Issue #1161 — H.264 PAFF (field pictures, one PES per FIELD) must demux to
//! ONE sample per complementary field pair (ITU-T H.264 §3.35-3.37; ISO/IEC
//! 14496-15 samples are whole access units), PTS/DTS from the first field.
//!
//! The streams are spec-built by `tests/support/paff_gen.rs` (I_PCM, losslessly
//! decodable, every field different), 12 frames of 64x64, TFF and BFF; two are
//! also committed under `tests/fixtures/paff/` and pinned against the generator.

#[path = "support/paff_gen.rs"]
mod paff_gen;

use broadcast_common::{Package, Unpackage};
use transmux::pipeline::CodecConfig;
use transmux::slice_header::{AvcSliceHeader, decode_avc_slice_pps, parse_avc_slice_header};
use transmux::sps::decode_avc_slice_sps;
use transmux::{
    CmafMux, DemuxEvent, Fmp4Demux, Media, StreamingTsDemux, TsDemux, TsMux, iter_annexb_nals,
    iter_length_prefixed_nals,
};

const FRAMES: usize = 12;
const FRAME_PERIOD: i64 = 3600;
const BASE_PTS: i64 = 90_000;
const NAL_SPS: u8 = 7;
const NAL_PPS: u8 = 8;

struct S {
    data: Vec<u8>,
    dts: Option<i64>,
    pts: Option<i64>,
    duration: Option<u32>,
    sync: bool,
}

fn avc_samples(m: &Media) -> Vec<S> {
    let t = m
        .tracks
        .iter()
        .find(|t| matches!(t.spec.config, CodecConfig::Avc { .. }))
        .expect("AVC track");
    t.samples
        .iter()
        .map(|s| S {
            data: s.data.to_vec(),
            dts: s.dts,
            pts: s.pts,
            duration: s.duration,
            sync: s.flags.is_sync,
        })
        .collect()
}

fn batch(ts: &[u8]) -> Vec<S> {
    avc_samples(&TsDemux::new().unpackage(ts).expect("batch demux"))
}

fn streaming(ts: &[u8], chunk: usize) -> Vec<S> {
    // Only the AVC track (a capture may carry audio too).
    let video_id = TsDemux::new()
        .unpackage(ts)
        .expect("batch demux")
        .tracks
        .iter()
        .find(|t| matches!(t.spec.config, CodecConfig::Avc { .. }))
        .expect("AVC track")
        .spec
        .track_id;
    let mut d = StreamingTsDemux::new();
    let mut out = Vec::new();
    let mut drain = |d: &mut StreamingTsDemux| {
        while let Some(e) = d.poll_event() {
            if let DemuxEvent::Sample {
                track_id, sample, ..
            } = e
                && track_id == video_id
            {
                out.push(S {
                    data: sample.data.to_vec(),
                    dts: sample.dts,
                    pts: sample.pts,
                    duration: sample.duration,
                    sync: sample.flags.is_sync,
                });
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

/// Parsed slice headers of every VCL NAL in a length-prefixed sample, using the
/// SPS/PPS of the generated elementary stream.
fn slices(es: &[u8], sample: &[u8]) -> Vec<AvcSliceHeader> {
    let mut sps = None;
    let mut pps = None;
    for n in iter_annexb_nals(es) {
        match n[0] & 0x1F {
            NAL_SPS => sps = Some(decode_avc_slice_sps(n).unwrap()),
            NAL_PPS => pps = Some(decode_avc_slice_pps(n).unwrap()),
            _ => {}
        }
    }
    let (sps, pps) = (sps.unwrap(), pps.unwrap());
    iter_length_prefixed_nals(sample)
        .expect("well-formed length-prefixed sample")
        .into_iter()
        .filter(|n| matches!(n[0] & 0x1F, 1 | 5))
        .map(|n| parse_avc_slice_header(n, &sps, &pps).unwrap())
        .collect()
}

fn aud_count(sample: &[u8]) -> usize {
    iter_length_prefixed_nals(sample)
        .unwrap()
        .iter()
        .filter(|n| n[0] & 0x1F == 9)
        .count()
}

fn assert_paired(g: &paff_gen::Generated, samples: &[S], what: &str) {
    assert_paired_from(g, samples, what, BASE_PTS)
}

/// [`assert_paired`] for a stream whose clock was re-based (muxers add an offset).
fn assert_paired_from(g: &paff_gen::Generated, samples: &[S], what: &str, base: i64) {
    assert_eq!(
        samples.len(),
        FRAMES,
        "{what}: 12 frames -> 12 samples (was 24)"
    );
    for (k, s) in samples.iter().enumerate() {
        let hs = slices(&g.es, &s.data);
        assert_eq!(
            hs.len(),
            2,
            "{what}: sample {k} holds exactly two slice NALs"
        );
        assert!(hs.iter().all(|h| h.field_pic_flag), "{what}: field slices");
        assert_ne!(
            hs[0].bottom_field_flag, hs[1].bottom_field_flag,
            "{what}: sample {k} fields have opposite parity"
        );
        assert_eq!(hs[0].frame_num, hs[1].frame_num, "{what}: same frame_num");
        assert_eq!(aud_count(&s.data), 1, "{what}: one AUD per access unit");
        let want = base + FRAME_PERIOD * k as i64;
        assert_eq!(s.pts, Some(want), "{what}: PTS is the first field's");
        assert_eq!(s.dts, Some(want), "{what}: DTS is the first field's");
        assert_eq!(
            s.duration,
            Some(FRAME_PERIOD as u32),
            "{what}: frame duration"
        );
        assert_eq!(
            s.sync,
            k == 0,
            "{what}: only the IDR frame is a sync sample"
        );
    }
}

fn gen_tff() -> paff_gen::Generated {
    paff_gen::generate(FRAMES, 64, 64, false)
}
fn gen_bff() -> paff_gen::Generated {
    paff_gen::generate(FRAMES, 64, 64, true)
}

#[test]
fn tff_and_bff_pair_into_one_sample_per_frame() {
    for (name, g, first_bottom) in [("tff", gen_tff(), false), ("bff", gen_bff(), true)] {
        let s = batch(&g.ts);
        assert_paired(&g, &s, name);
        // The first field in decode order is the first slice of each sample.
        assert!(
            s.iter()
                .all(|x| slices(&g.es, &x.data)[0].bottom_field_flag == first_bottom),
            "{name}: first slice is the first field"
        );
        // Byte-level: the sample is exactly field1 ++ field2 (AUD of the second dropped),
        // so nothing was lost or reordered. Reconstruct from the generated field AUs.
        for (k, x) in s.iter().enumerate() {
            let mut want = Vec::new();
            for (i, (_, au)) in g.aus[2 * k..2 * k + 2].iter().enumerate() {
                for n in iter_annexb_nals(au) {
                    if i == 1 && n[0] & 0x1F == 9 {
                        continue;
                    }
                    want.extend((n.len() as u32).to_be_bytes());
                    want.extend_from_slice(n);
                }
            }
            assert_eq!(x.data, want, "{name}: sample {k} bytes");
        }
    }
}

#[test]
fn streaming_equals_batch_at_every_chunk_size() {
    for g in [gen_tff(), gen_bff()] {
        let b = batch(&g.ts);
        assert_paired(&g, &b, "batch");
        let sizes = (1..=64usize).chain([100, 187, 188, 189, 200, 376, 377, 500, 751, 997, 1000]);
        for chunk in sizes {
            let s = streaming(&g.ts, chunk);
            assert_eq!(s.len(), b.len(), "chunk {chunk}: sample count");
            for (i, (x, y)) in s.iter().zip(&b).enumerate() {
                assert!(
                    x.data == y.data
                        && x.dts == y.dts
                        && x.pts == y.pts
                        && x.duration == y.duration
                        && x.sync == y.sync,
                    "chunk {chunk}: sample {i} differs from batch"
                );
            }
        }
    }
}

#[test]
fn remux_through_ts_and_fmp4_stays_one_sample_per_frame() {
    for g in [gen_tff(), gen_bff()] {
        let ir = TsDemux::new().unpackage(&g.ts).unwrap();
        // TS -> IR -> TS -> IR
        let ts2 = TsMux::new().package(&ir).expect("remux TS");
        let r = batch(&ts2);
        let base = r[0].pts.unwrap();
        assert_paired_from(&g, &r, "TS remux", base);
        // IR -> fMP4 -> IR
        let mp4 = CmafMux::default().package(&ir).expect("mux fMP4");
        let back = Fmp4Demux::new().unpackage(&mp4).expect("demux fMP4");
        let n = avc_samples(&back).len();
        assert_eq!(n, FRAMES, "fMP4 round trip keeps 12 samples");
        for s in avc_samples(&back) {
            assert_eq!(slices(&g.es, &s.data).len(), 2);
        }
    }
}

#[test]
fn committed_fixtures_match_the_generator_and_pair() {
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/paff");
    for (name, g) in [
        ("paff_tff_12f_64x64", gen_tff()),
        ("paff_bff_12f_64x64", gen_bff()),
    ] {
        let ts = std::fs::read(dir.join(format!("{name}.ts"))).expect("committed fixture");
        assert_eq!(ts, g.ts, "{name}.ts is exactly what paff_gen produces");
        let yuv = std::fs::read(dir.join(format!("{name}.yuv"))).expect("committed yuv");
        assert_eq!(yuv, g.yuv, "{name}.yuv");
        assert_paired(&g, &batch(&ts), name);
    }
}

#[test]
fn unpaired_fields_are_emitted_alone_not_held_or_merged() {
    let g = gen_tff();
    // Lose the second field of frame 5 (mid-stream) and of the last frame (tail).
    let mut aus = g.aus.clone();
    aus.remove(2 * 11 + 1);
    aus.remove(2 * 5 + 1);
    let ts = paff_gen::ts_from_aus(&aus);
    for s in [batch(&ts), streaming(&ts, 188), streaming(&ts, 7)] {
        assert_eq!(s.len(), FRAMES, "12 frames still 12 samples");
        for (k, x) in s.iter().enumerate() {
            let n = slices(&g.es, &x.data).len();
            let lone = k == 5 || k == 11;
            assert_eq!(n, if lone { 1 } else { 2 }, "sample {k} slice count");
            assert_eq!(
                x.pts,
                Some(BASE_PTS + FRAME_PERIOD * k as i64),
                "sample {k} pts"
            );
        }
    }
}

#[test]
fn second_field_after_a_timestamp_jump_is_not_merged() {
    let g = gen_tff();
    let mut aus = g.aus.clone();
    // Second field of frame 5 arrives 10 s late (discontinuity-like jump); every
    // later AU follows it on the new clock.
    for (pts, _) in aus.iter_mut().skip(2 * 5 + 1) {
        *pts += 900_000;
    }
    let ts = paff_gen::ts_from_aus(&aus);
    let s = batch(&ts);
    let counts: Vec<usize> = s.iter().map(|x| slices(&g.es, &x.data).len()).collect();
    assert_eq!(counts.iter().sum::<usize>(), 2 * FRAMES, "no field lost");
    assert_eq!(
        counts[5], 1,
        "frame 5's first field stays alone across the jump"
    );
    assert_eq!(counts[..5], [2; 5], "pairs before the jump are intact");
    assert_eq!(
        counts.len(),
        FRAMES + 1,
        "the orphan second field is its own sample"
    );
}

#[test]
fn ffmpeg_decodes_the_generated_and_demuxed_streams_to_the_expected_frames() {
    // Oracle: the generator's stream is valid (decodes to the woven yuv), and the
    // paired samples re-muxed to TS still decode to exactly the same frames.
    let Ok(ver) = std::process::Command::new("ffmpeg")
        .arg("-version")
        .output()
    else {
        eprintln!("ffmpeg not on PATH: SKIPPED");
        return;
    };
    assert!(ver.status.success());
    let g = gen_tff();
    let dir = std::env::temp_dir().join(format!("paff_oracle_{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let decode = |name: &str, bytes: &[u8]| -> Vec<u8> {
        let input = dir.join(name);
        std::fs::write(&input, bytes).unwrap();
        let out = std::process::Command::new("ffmpeg")
            .args(["-v", "error", "-i"])
            .arg(&input)
            .args([
                "-fps_mode",
                "passthrough",
                "-f",
                "rawvideo",
                "-pix_fmt",
                "yuv420p",
                "-",
            ])
            .output()
            .unwrap();
        assert!(
            out.status.success(),
            "ffmpeg: {}",
            String::from_utf8_lossy(&out.stderr)
        );
        out.stdout
    };
    assert!(
        decode("gen.ts", &g.ts) == g.yuv,
        "generator stream decodes to the expected yuv"
    );
    let ir = TsDemux::new().unpackage(&g.ts).unwrap();
    let remuxed = TsMux::new().package(&ir).unwrap();
    assert!(
        decode("remux.ts", &remuxed) == g.yuv,
        "demuxed+remuxed (paired) stream decodes to the same 12 frames"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn progressive_and_non_field_streams_are_unaffected() {
    // Every committed H.264 TS fixture that is not PAFF must keep one sample per
    // access unit: classification never reports a field for them.
    use transmux::slice_header::{AvcParamSets, AvcPicture};
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../fixtures/ts/h264");
    let Ok(rd) = std::fs::read_dir(&root) else {
        eprintln!("{root:?} absent: SKIPPED");
        return;
    };
    let mut checked = 0;
    for e in rd
        .flatten()
        .filter(|e| e.path().extension().is_some_and(|x| x == "ts"))
    {
        let bytes = std::fs::read(e.path()).unwrap();
        let media = TsDemux::new().unpackage(&bytes).unwrap();
        let Some(t) = media
            .tracks
            .iter()
            .find(|t| matches!(t.spec.config, CodecConfig::Avc { .. }))
        else {
            continue;
        };
        let mut ps = AvcParamSets::new();
        let mut frames = 0;
        for s in &t.samples {
            let mut annexb = Vec::new();
            for n in iter_length_prefixed_nals(&s.data).unwrap() {
                annexb.extend([0, 0, 0, 1]);
                annexb.extend_from_slice(n);
            }
            let pic = ps.classify_au(&annexb);
            assert!(
                !matches!(pic, AvcPicture::Field(_)),
                "{:?}: non-PAFF fixture classified as field pictures",
                e.path()
            );
            frames += usize::from(pic == AvcPicture::Frame);
        }
        // Guard against a vacuous pass (every AU `Unknown`): the slice headers parse.
        assert!(
            frames > 0,
            "{:?}: no AU classified as a coded frame",
            e.path()
        );
        checked += 1;
    }
    assert!(
        checked >= 3,
        "expected the committed H.264 fixtures, got {checked}"
    );
}

/// Regenerates the committed fixtures: `cargo test -p transmux --test
/// paff_pairing -- --ignored regenerate_fixtures`.
#[test]
#[ignore]
fn regenerate_fixtures() {
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/paff");
    for (name, g) in [
        ("paff_tff_12f_64x64", gen_tff()),
        ("paff_bff_12f_64x64", gen_bff()),
    ] {
        std::fs::write(dir.join(format!("{name}.ts")), &g.ts).unwrap();
        std::fs::write(dir.join(format!("{name}.yuv")), &g.yuv).unwrap();
    }
}

/// The committed real broadcast capture `fixtures/ts/gulli-opengop.ts` is field-coded
/// (PAFF, one PES per field): 293 fields must demux to 147 samples (146 pairs plus
/// the odd tail field), each pair two slice NALs of opposite parity.
#[test]
fn real_broadcast_paff_capture_pairs() {
    use transmux::slice_header::{AvcParamSets, AvcPicture};
    let path =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../fixtures/ts/gulli-opengop.ts");
    let ts = std::fs::read(path).expect("gulli-opengop.ts fixture");
    let samples = batch(&ts);
    assert_eq!(samples.len(), 147, "293 fields -> 146 pairs + 1 lone field");
    let mut ps = AvcParamSets::new();
    let (mut pairs, mut lone) = (0, 0);
    for s in &samples {
        let mut annexb = Vec::new();
        for n in iter_length_prefixed_nals(&s.data).unwrap() {
            annexb.extend([0, 0, 0, 1]);
            annexb.extend_from_slice(n);
        }
        // Two parities in one AU classify as a Frame; a lone field as Field.
        match ps.classify_au(&annexb) {
            AvcPicture::Frame => pairs += 1,
            AvcPicture::Field(_) => lone += 1,
            other => panic!("unexpected classification {other:?}"),
        }
        assert_eq!(aud_count(&s.data), 1, "one AUD per access unit");
    }
    assert_eq!((pairs, lone), (146, 1));
    for chunk in [1usize, 7, 188, 999] {
        assert_eq!(streaming(&ts, chunk).len(), 147, "streaming chunk {chunk}");
    }
}
