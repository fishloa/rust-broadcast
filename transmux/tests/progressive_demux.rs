//! Real-fixture tests for [`ProgressiveDemux`] (issue #561).
//!
//! Ground truth for `h264_aac_prog.mp4` was cross-checked two independent
//! ways: (1) a standalone Python parser walking the file's own
//! `moov→trak→mdia→minf→stbl` boxes (`stts`/`ctts`/`stss`/`stsz`/`stsc`/
//! `stco`) directly, and (2) `ffprobe -show_packets -ignore_editlist 1`
//! (which reports pts/dts/flags *without* applying the trak's `elst` —
//! [`ProgressiveDemux`] never reads `elst` either, matching every other
//! demuxer in this crate, so `-ignore_editlist 1` is the correct comparison
//! mode). Both agree exactly; the literals below are those values.
//!
//! The default (edit-list-applied) `ffprobe -show_packets` reports a uniform
//! -1024-tick shift on every video/audio pts/dts (the trak's `elst`
//! `media_time` is 1024 in each track's own timescale) — that is a
//! presentation-timeline concern the mux/consumer applies, not something a
//! demuxer's decode-order [`Track`]/[`Sample`] IR carries.

use broadcast_common::{Package, Unpackage};
use transmux::{CmafMux, CodecConfig, Fmp4Demux, ProgressiveDemux, SegmentIndexBox};

fn prog_fixture() -> Vec<u8> {
    let path = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../fixtures/transmux/h264_aac_prog.mp4"
    );
    std::fs::read(path).expect("h264_aac_prog.mp4 fixture must exist")
}

fn sidx_fixture() -> Vec<u8> {
    let path = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../fixtures/transmux/h264_sidx.mp4"
    );
    std::fs::read(path).expect("h264_sidx.mp4 fixture must exist")
}

/// Running (dts, pts) pairs for a track, derived the same way the IR encodes
/// them: dts is the running sum of preceding durations, pts = dts +
/// composition_offset.
fn dts_pts(samples: &[transmux::Sample]) -> Vec<(u64, i64)> {
    let mut dts: u64 = 0;
    let mut out = Vec::with_capacity(samples.len());
    for s in samples {
        let pts = dts as i64 + s.composition_offset() as i64;
        out.push((dts, pts));
        dts += s.duration.unwrap_or(0) as u64;
    }
    out
}

// ---------------------------------------------------------------------------
// 1. Demux h264_aac_prog.mp4 — counts + ffprobe/box-table ground truth
// ---------------------------------------------------------------------------

#[test]
fn demux_h264_aac_prog_track_counts_and_codec() {
    let data = prog_fixture();
    let media = ProgressiveDemux::new(1024 * 1024)
        .unwrap()
        .unpackage(&data)
        .unwrap();

    assert_eq!(media.tracks.len(), 2, "exactly one video + one audio track");

    let video = media
        .tracks
        .iter()
        .find(|t| matches!(t.spec.config, CodecConfig::Avc { .. }))
        .expect("video (Avc) track");
    let audio = media
        .tracks
        .iter()
        .find(|t| matches!(t.spec.config, CodecConfig::Aac { .. }))
        .expect("audio (Aac) track");

    // Sample counts — ffprobe -show_packets -ignore_editlist 1: 50 video, 88 audio.
    assert_eq!(video.samples.len(), 50, "video sample count");
    assert_eq!(audio.samples.len(), 88, "audio sample count");

    // Video CodecConfig::Avc — ffprobe: profile High (profile_idc 100), level
    // 1.3 (level_idc 13), 320x240.
    match &video.spec.config {
        CodecConfig::Avc {
            config,
            width,
            height,
        } => {
            assert_eq!(config.config.profile_indication, 100, "AVC profile High");
            assert_eq!(config.config.level_indication, 13, "AVC level 1.3");
            assert_eq!(*width, 320, "coded width");
            assert_eq!(*height, 240, "coded height");
        }
        other => panic!("expected CodecConfig::Avc, got {other:?}"),
    }
    // Not TS-sourced.
    assert_eq!(video.spec.source_pid, None);
    assert_eq!(audio.spec.source_pid, None);
}

#[test]
fn demux_h264_aac_prog_video_sample_timing_and_sync() {
    let data = prog_fixture();
    let media = ProgressiveDemux::new(1024 * 1024)
        .unwrap()
        .unpackage(&data)
        .unwrap();
    let video = media
        .tracks
        .iter()
        .find(|t| matches!(t.spec.config, CodecConfig::Avc { .. }))
        .unwrap();

    let timing = dts_pts(&video.samples);
    assert_eq!(timing.len(), 50);

    // First 5 samples: (dts, pts) — from stts (delta=512 throughout) + ctts
    // (v0, unsigned, entries [(1,1024),(1,2048),(2,512),(1,2048),(2,512),...]).
    let expected_first = [
        (0u64, 1024i64),
        (512, 512 + 2048),
        (1024, 1024 + 512),
        (1536, 1536 + 512),
        (2048, 2048 + 2048),
    ];
    assert_eq!(&timing[..5], &expected_first, "first 5 video (dts,pts)");

    // Last 5 samples.
    let expected_last = [
        (23040u64, 23040i64 + 512),
        (23552, 23552 + 2048),
        (24064, 24064 + 512),
        (24576, 24576 + 512),
        (25088, 25088 + 1024),
    ];
    assert_eq!(&timing[45..50], &expected_last, "last 5 video (dts,pts)");

    // Total decode-timeline duration: 50 * 512 = 25600.
    let total_duration: u64 = video
        .samples
        .iter()
        .map(|s| s.duration.unwrap_or(0) as u64)
        .sum();
    assert_eq!(total_duration, 25600);

    // stss lists only sample #1 (1-based) as a sync sample — every other
    // sample is non-sync (ffprobe: only the first packet carries the `K` flag).
    for (i, s) in video.samples.iter().enumerate() {
        assert_eq!(s.flags.is_sync, i == 0, "video sample {i} sync flag");
    }
}

#[test]
fn demux_h264_aac_prog_audio_sample_timing_and_sync() {
    let data = prog_fixture();
    let media = ProgressiveDemux::new(1024 * 1024)
        .unwrap()
        .unpackage(&data)
        .unwrap();
    let audio = media
        .tracks
        .iter()
        .find(|t| matches!(t.spec.config, CodecConfig::Aac { .. }))
        .unwrap();

    let timing = dts_pts(&audio.samples);
    assert_eq!(timing.len(), 88);

    // stts: 87 samples of delta=1024, then 1 final sample of delta=136.
    // No ctts on the audio track ⇒ pts == dts for every sample.
    let expected_first = [(0u64, 0i64), (1024, 1024), (2048, 2048), (3072, 3072)];
    assert_eq!(&timing[..4], &expected_first, "first 4 audio (dts,pts)");

    let expected_last = [
        (86016u64, 86016i64),
        (87040, 87040),
        (88064, 88064),
        (89088, 89088),
    ];
    assert_eq!(&timing[84..88], &expected_last, "last 4 audio (dts,pts)");

    assert_eq!(
        audio.samples.last().unwrap().duration,
        Some(136),
        "final audio sample duration (short close-out frame)"
    );

    // No stss on the audio track ⇒ every sample is a sync sample.
    assert!(
        audio.samples.iter().all(|s| s.flags.is_sync),
        "every AAC sample is a sync sample (no stss box)"
    );

    // Sample byte sizes from stsz — spot-check first/last (from the box table).
    assert_eq!(audio.samples[0].data.len(), 314);
    assert_eq!(audio.samples[87].data.len(), 5);
}

// ---------------------------------------------------------------------------
// 2. Round-trip: progressive → IR → fMP4 (CmafMux) → Fmp4Demux
// ---------------------------------------------------------------------------

#[test]
fn round_trip_progressive_to_fmp4_preserves_samples_and_codec() {
    let data = prog_fixture();
    let original = ProgressiveDemux::new(1024 * 1024)
        .unwrap()
        .unpackage(&data)
        .unwrap();

    let cmaf = CmafMux::new(1)
        .package(&original)
        .expect("CmafMux::package");
    let reloaded = Fmp4Demux::new()
        .unpackage(&cmaf)
        .expect("Fmp4Demux::unpackage");

    assert_eq!(
        reloaded.tracks.len(),
        original.tracks.len(),
        "same track count after round-trip"
    );

    for orig_track in &original.tracks {
        let reloaded_track = reloaded
            .tracks
            .iter()
            .find(|t| t.spec.track_id == orig_track.spec.track_id)
            .unwrap_or_else(|| {
                panic!(
                    "track {} missing after round-trip",
                    orig_track.spec.track_id
                )
            });

        assert_eq!(
            reloaded_track.samples.len(),
            orig_track.samples.len(),
            "track {} sample count preserved",
            orig_track.spec.track_id
        );
        for (i, (a, b)) in orig_track
            .samples
            .iter()
            .zip(reloaded_track.samples.iter())
            .enumerate()
        {
            assert_eq!(
                a.data, b.data,
                "track {} sample {i} bytes byte-identical",
                orig_track.spec.track_id
            );
            assert_eq!(
                a.duration, b.duration,
                "track {} sample {i} duration preserved",
                orig_track.spec.track_id
            );
            assert_eq!(
                a.composition_offset(),
                b.composition_offset(),
                "track {} sample {i} composition_offset preserved",
                orig_track.spec.track_id
            );
            assert_eq!(
                a.flags.is_sync, b.flags.is_sync,
                "track {} sample {i} sync flag preserved",
                orig_track.spec.track_id
            );
        }

        // Codec config preserved (compare the discriminant + key fields —
        // CodecConfig has no PartialEq).
        match (&orig_track.spec.config, &reloaded_track.spec.config) {
            (
                CodecConfig::Avc {
                    width: w1,
                    height: h1,
                    config: c1,
                },
                CodecConfig::Avc {
                    width: w2,
                    height: h2,
                    config: c2,
                },
            ) => {
                assert_eq!(w1, w2);
                assert_eq!(h1, h2);
                assert_eq!(c1.config.profile_indication, c2.config.profile_indication);
                assert_eq!(c1.config.level_indication, c2.config.level_indication);
            }
            (CodecConfig::Aac { .. }, CodecConfig::Aac { .. }) => {}
            (a, b) => panic!("codec config mismatch after round-trip: {a:?} vs {b:?}"),
        }
    }
}

// ---------------------------------------------------------------------------
// 3. sidx byte-exact round-trip (h264_sidx.mp4)
// ---------------------------------------------------------------------------

#[test]
fn sidx_byte_exact_round_trip_and_mutation_bites() {
    use broadcast_common::{Parse, Serialize};

    let data = sidx_fixture();
    // The fixture is ftyp (28 bytes) + sidx (54 bytes) — locate sidx by
    // walking top-level boxes rather than hardcoding the offset.
    let mut offset = 0usize;
    let mut sidx_box: Option<&[u8]> = None;
    while offset + 8 <= data.len() {
        let size = u32::from_be_bytes(data[offset..offset + 4].try_into().unwrap()) as usize;
        let ty = &data[offset + 4..offset + 8];
        if ty == b"sidx" {
            sidx_box = Some(&data[offset..offset + size]);
            break;
        }
        if size < 8 {
            break;
        }
        offset += size;
    }
    let sidx_bytes = sidx_box.expect("sidx box must be present in h264_sidx.mp4");

    let parsed = SegmentIndexBox::parse(sidx_bytes).expect("parse sidx");
    assert!(
        parsed.references.len() >= 2,
        "fixture carries >= 2 sidx references"
    );

    // Byte-exact round-trip.
    let mut buf = vec![0u8; parsed.serialized_len()];
    let n = parsed.serialize_into(&mut buf).unwrap();
    assert_eq!(
        &buf[..n],
        sidx_bytes,
        "sidx parse->serialize byte-identical"
    );

    // Mutating a reference's subsegment_duration must change the serialized bytes.
    let mut mutated = parsed.clone();
    mutated.references[0].subsegment_duration += 1;
    let mut mbuf = vec![0u8; mutated.serialized_len()];
    let mn = mutated.serialize_into(&mut mbuf).unwrap();
    assert_ne!(
        &mbuf[..mn],
        &buf[..n],
        "mutating subsegment_duration must change serialized bytes"
    );
}

// ── Stage-adapter poisoning after a cap rejection ───────────────────────────

/// A `Stage::feed` rejected for exceeding `max_bytes` **discards that chunk**,
/// so the accumulated buffer has a hole in it. Because a progressive MP4
/// resolves sample payloads through `stbl`'s *file-absolute* chunk offsets
/// (ISO/IEC 14496-12:2015 §8.7.5), parsing a holed buffer does not fail
/// cleanly — it either reports a misleading `UnexpectedBox` or, far worse,
/// succeeds and hands back a `Media` whose samples carry the wrong bytes.
///
/// So the rejection is terminal: a following smaller chunk is refused too,
/// `demand()` stays saturated, and `finish()` re-reports the original cap
/// error instead of parsing. Before this, the smaller chunk was accepted and
/// `finish()` parsed the corrupt buffer.
#[test]
fn progressive_demux_stays_poisoned_after_a_cap_rejection() {
    use broadcast_common::{Demand, Stage, Timestamp};

    const MAX_BYTES: usize = 4096;
    let mut demux = ProgressiveDemux::new(MAX_BYTES).expect("non-zero cap");

    // A real prefix of a real file, so `finish()` could plausibly parse
    // something if the poison were missing.
    let file = prog_fixture();
    Stage::feed(&mut demux, &file[..1024], Timestamp::ZERO).expect("first chunk fits");

    // Overshoot the cap: this chunk is dropped on the floor.
    let big = vec![0u8; MAX_BYTES];
    let first_err = Stage::feed(&mut demux, &big, Timestamp::ZERO)
        .expect_err("a chunk past the cap must be rejected");
    assert!(
        matches!(first_err, transmux::Error::BufferCapExceeded { cap, .. } if cap == MAX_BYTES),
        "expected BufferCapExceeded naming the bound, got {first_err:?}"
    );

    let Demand { saturated, .. } = Stage::demand(&demux);
    assert!(saturated, "a poisoned demuxer must report saturated");

    // A smaller chunk that *would* fit must still be refused — accepting it
    // is what used to build the holed buffer.
    let follow_up = Stage::feed(&mut demux, &file[1024..1088], Timestamp::ZERO)
        .expect_err("a poisoned demuxer must refuse every further feed");
    assert!(
        matches!(follow_up, transmux::Error::BufferCapExceeded { .. }),
        "the poison must keep reporting the original cap error, got {follow_up:?}"
    );

    let finish_err =
        Stage::finish(&mut demux).expect_err("finish must not parse a buffer with a hole");
    assert!(
        matches!(finish_err, transmux::Error::BufferCapExceeded { cap, .. } if cap == MAX_BYTES),
        "finish must re-report the original cap error, got {finish_err:?}"
    );
    assert!(
        Stage::poll(&mut demux).is_none(),
        "no Media may ever be produced from a poisoned demuxer"
    );
}

/// `feed` after `finish` used to append bytes that nothing would ever parse,
/// while `demand()` still advertised headroom inviting more of them.
#[test]
fn progressive_demux_refuses_feed_after_finish() {
    use broadcast_common::{Stage, Timestamp};

    let file = prog_fixture();
    let mut demux = ProgressiveDemux::new(file.len()).expect("non-zero cap");
    Stage::feed(&mut demux, &file, Timestamp::ZERO).expect("whole file fits");
    Stage::finish(&mut demux).expect("parse");
    assert!(
        Stage::poll(&mut demux).is_some(),
        "finish produces the Media"
    );

    assert!(
        Stage::demand(&demux).saturated,
        "a finished demuxer must not advertise headroom"
    );
    let err = Stage::feed(&mut demux, &file[..64], Timestamp::ZERO)
        .expect_err("feed after finish must be refused, not silently appended");
    assert!(
        matches!(err, transmux::Error::InvalidInput(_)),
        "expected a named InvalidInput, got {err:?}"
    );
}

/// `ProgressiveDemux::new(0)` is a useless `Stage` configuration. It must fail
/// loudly at construction rather than wedge: a zero cap can never accept a
/// single byte, so `new` itself now rejects it instead of handing back a
/// `Stage` that would be saturated and poisoned before it ever saw data.
#[test]
fn progressive_demux_new_zero_fails_loudly_instead_of_wedging() {
    let err = ProgressiveDemux::new(0)
        .expect_err("a zero cap must be rejected at construction, not accepted and left to wedge");
    assert!(
        matches!(err, transmux::Error::InvalidInput(_)),
        "expected a named InvalidInput, got {err:?}"
    );
}
