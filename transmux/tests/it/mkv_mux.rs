//! Matroska (MKV) muxer round-trip integration tests (issue #915).
//!
//! Each fixture is demuxed with [`MatroskaDemux`](transmux::MatroskaDemux) (real ffmpeg captures — see
//! `fixtures/mkv/PROVENANCE.md`), packaged back to bytes with [`MkvMuxStage`], then
//! re-demuxed with the same stage. The two [`Media`] values (original
//! demux vs. demux-of-the-remux) are compared field-by-field: track count,
//! codec identity/config, sample count, PTS/DTS, keyframe flags, and the
//! coded sample bytes themselves. A muxer that dropped a sample, mis-scaled a
//! timestamp, or corrupted `CodecPrivate` fails one of these comparisons.

use test_harness::collect::Collected as Media;
use transmux::MkvMuxStage;
use transmux::pipeline::CodecConfig;

/// H.264 (`fixtures/ts/h264-aac.ts`, `-c copy`'d to Matroska) + AAC.
const H264_AAC: &str = "mkv/h264-aac.mkv";
/// HEVC (`fixtures/ts/hevc/main.ts`) video muxed with the same real AAC audio
/// as [`H264_AAC`] (`fixtures/ts/h264-aac.ts`) — two genuine captures, one file.
const HEVC_AAC: &str = "mkv/hevc-aac.mkv";
/// VP9 + Opus (`fixtures/mkv/vp9-opus.webm`, `-c copy`'d to Matroska).
const VP9_OPUS: &str = "mkv/vp9-opus.mkv";

fn demux(path: &str) -> Media {
    let bytes =
        std::fs::read(test_fixtures::path(path)).unwrap_or_else(|e| panic!("read {path}: {e}"));
    test_harness::collect::media_from_matroska(&bytes, test_harness::collect::CollectMode::Seekable)
        .unwrap_or_else(|e| panic!("demux {path}: {e}"))
}

/// Demux `path`, mux the result back with [`MkvMuxStage`], then demux that output
/// again. Returns (original demux, demux-of-the-remux).
fn round_trip(path: &str) -> (Media, Media) {
    let original = demux(path);
    let muxed = crate::mux_bytes::mux_bytes(
        &mut MkvMuxStage::new(std::time::Duration::from_millis(5_000)),
        &original,
    );
    let remuxed = test_harness::collect::media_from_matroska(
        &muxed,
        test_harness::collect::CollectMode::Seekable,
    )
    .unwrap_or_else(|e| panic!("re-demux muxed {path}: {e}"));
    (original, remuxed)
}

/// A short discriminant name for a [`CodecConfig`], for the codec-kind
/// assertion (the detailed field comparison happens separately per variant).
fn codec_kind(c: &CodecConfig) -> &'static str {
    match c {
        CodecConfig::Avc { .. } => "Avc",
        CodecConfig::Hevc { .. } => "Hevc",
        CodecConfig::Vp9 { .. } => "Vp9",
        CodecConfig::Vp8 { .. } => "Vp8",
        CodecConfig::Av1 { .. } => "Av1",
        CodecConfig::Aac { .. } => "Aac",
        CodecConfig::Opus { .. } => "Opus",
        CodecConfig::Vorbis { .. } => "Vorbis",
        _ => "other",
    }
}

/// Assert the two `CodecConfig`s carry byte-identical `CodecPrivate`-equivalent
/// config plus matching dimensions/channels/sample-rate.
fn assert_codec_config_equal(a: &CodecConfig, b: &CodecConfig) {
    match (a, b) {
        (
            CodecConfig::Avc {
                config: ca,
                width: wa,
                height: ha,
            },
            CodecConfig::Avc {
                config: cb,
                width: wb,
                height: hb,
            },
        ) => {
            use broadcast_core::Serialize;
            assert_eq!(
                ca.config.try_to_bytes().unwrap(),
                cb.config.try_to_bytes().unwrap(),
                "avcC bytes"
            );
            assert_eq!((wa, ha), (wb, hb), "AVC dimensions");
        }
        (
            CodecConfig::Hevc {
                config: ca,
                width: wa,
                height: ha,
            },
            CodecConfig::Hevc {
                config: cb,
                width: wb,
                height: hb,
            },
        ) => {
            use broadcast_core::Serialize;
            assert_eq!(
                ca.config.try_to_bytes().unwrap(),
                cb.config.try_to_bytes().unwrap(),
                "hvcC bytes"
            );
            assert_eq!((wa, ha), (wb, hb), "HEVC dimensions");
        }
        (
            CodecConfig::Vp9 {
                width: wa,
                height: ha,
                ..
            },
            CodecConfig::Vp9 {
                width: wb,
                height: hb,
                ..
            },
        ) => {
            assert_eq!((wa, ha), (wb, hb), "VP9 dimensions");
        }
        (
            CodecConfig::Aac {
                esds: ea,
                channel_count: cca,
                sample_rate: sra,
                ..
            },
            CodecConfig::Aac {
                esds: eb,
                channel_count: ccb,
                sample_rate: srb,
                ..
            },
        ) => {
            let asc_a = &ea
                .es_descriptor
                .decoder_config
                .as_ref()
                .unwrap()
                .decoder_specific_info
                .as_ref()
                .unwrap()
                .data;
            let asc_b = &eb
                .es_descriptor
                .decoder_config
                .as_ref()
                .unwrap()
                .decoder_specific_info
                .as_ref()
                .unwrap()
                .data;
            assert_eq!(asc_a, asc_b, "AAC AudioSpecificConfig bytes");
            assert_eq!((cca, sra), (ccb, srb), "AAC channels/rate");
        }
        (
            CodecConfig::Opus {
                channel_count: cca,
                sample_rate: sra,
                ..
            },
            CodecConfig::Opus {
                channel_count: ccb,
                sample_rate: srb,
                ..
            },
        ) => {
            assert_eq!((cca, sra), (ccb, srb), "Opus channels/rate");
        }
        (a, b) => panic!("codec kind mismatch: {a:?} vs {b:?}"),
    }
}

/// Assert two [`Media`] values are equivalent for round-trip purposes: same
/// track count/order, same codec identity/config per track, same sample
/// count/timing/keyframe-flags/coded-bytes per track.
fn assert_media_round_trips(original: &Media, remuxed: &Media) {
    assert_eq!(
        original.tracks.len(),
        remuxed.tracks.len(),
        "track count differs after mux/re-demux"
    );
    for (i, (ta, tb)) in original.tracks.iter().zip(&remuxed.tracks).enumerate() {
        assert_eq!(
            codec_kind(&ta.spec.config),
            codec_kind(&tb.spec.config),
            "track {i} codec kind"
        );
        assert_codec_config_equal(&ta.spec.config, &tb.spec.config);
        assert_eq!(ta.samples.len(), tb.samples.len(), "track {i} sample count");
        for (j, (sa, sb)) in ta.samples.iter().zip(&tb.samples).enumerate() {
            assert_eq!(
                sa.timing.and_then(|t| t.pts()),
                sb.timing.and_then(|t| t.pts()),
                "track {i} sample {j} pts"
            );
            assert_eq!(
                sa.timing.and_then(|t| t.dts()),
                sb.timing.and_then(|t| t.dts()),
                "track {i} sample {j} dts"
            );
            assert_eq!(
                sa.flags.is_sync, sb.flags.is_sync,
                "track {i} sample {j} keyframe flag"
            );
            assert_eq!(sa.data, sb.data, "track {i} sample {j} coded bytes");
        }
    }
}

#[test]
fn mkv_round_trip_h264_aac() {
    let (original, remuxed) = round_trip(H264_AAC);
    assert_eq!(original.tracks.len(), 2, "expected video + audio tracks");
    assert!(matches!(
        original.tracks[0].spec.config,
        CodecConfig::Avc { .. }
    ));
    assert!(matches!(
        original.tracks[1].spec.config,
        CodecConfig::Aac { .. }
    ));
    assert!(!original.tracks[0].samples.is_empty());
    assert!(!original.tracks[1].samples.is_empty());
    assert_media_round_trips(&original, &remuxed);
}

#[test]
fn mkv_round_trip_hevc_aac() {
    let (original, remuxed) = round_trip(HEVC_AAC);
    assert_eq!(original.tracks.len(), 2, "expected video + audio tracks");
    assert!(matches!(
        original.tracks[0].spec.config,
        CodecConfig::Hevc { .. }
    ));
    assert!(matches!(
        original.tracks[1].spec.config,
        CodecConfig::Aac { .. }
    ));
    assert!(!original.tracks[0].samples.is_empty());
    assert!(!original.tracks[1].samples.is_empty());
    assert_media_round_trips(&original, &remuxed);
}

#[test]
fn mkv_round_trip_vp9_opus() {
    let (original, remuxed) = round_trip(VP9_OPUS);
    assert_eq!(original.tracks.len(), 2, "expected video + audio tracks");
    assert!(matches!(
        original.tracks[0].spec.config,
        CodecConfig::Vp9 { .. }
    ));
    assert!(matches!(
        original.tracks[1].spec.config,
        CodecConfig::Opus { .. }
    ));
    assert!(!original.tracks[0].samples.is_empty());
    assert!(!original.tracks[1].samples.is_empty());
    assert_media_round_trips(&original, &remuxed);
}

/// A muxed file must itself be well-formed enough to survive a *second*
/// mux/re-demux cycle byte-identically (idempotence beyond one round trip).
#[test]
fn mkv_double_round_trip_is_stable() {
    let (_, remuxed_once) = round_trip(H264_AAC);
    let muxed_twice = crate::mux_bytes::mux_bytes(
        &mut MkvMuxStage::new(std::time::Duration::from_millis(5_000)),
        &remuxed_once,
    );
    let remuxed_twice = test_harness::collect::media_from_matroska(
        &muxed_twice,
        test_harness::collect::CollectMode::Seekable,
    )
    .expect("second re-demux");
    assert_media_round_trips(&remuxed_once, &remuxed_twice);
}

// ── Issue #4154: a timeline break cuts a Cluster and rebases a backward one ──

#[test]
fn a_forward_break_cuts_a_cluster_at_the_seam_and_stays_readable() {
    let two = crate::discontinuity::forward();
    let seam = crate::discontinuity::seam_matroska_ticks(&two);
    let outputs = crate::discontinuity::drive(
        &mut MkvMuxStage::new(std::time::Duration::from_millis(5_000)),
        &two,
        None,
    )
    .expect("a forward break is expressible");
    let bytes = crate::mux_bytes::apply(outputs);

    let clusters = crate::discontinuity::cluster_timestamps(&bytes);
    assert!(
        clusters.len() >= 2,
        "the file must carry more than one Cluster, got {clusters:?}"
    );
    assert!(
        clusters.contains(&seam),
        "a Cluster must start at the break (tick {seam}), got {clusters:?}"
    );

    let remuxed = test_harness::collect::media_from_matroska(
        &bytes,
        test_harness::collect::CollectMode::Seekable,
    )
    .expect("the muxed file re-demuxes");
    for track in &two.media.tracks {
        let got = remuxed
            .tracks
            .iter()
            .find(|candidate| candidate.spec.track_id == track.spec.track_id)
            .expect("every track survives the break");
        assert_eq!(
            track.samples.len(),
            got.samples.len(),
            "every sample of track {:?} survives the break",
            track.spec.track_id
        );
    }

    let path = crate::discontinuity::scratch_path(&format!("dc1-mkv-{}.mkv", std::process::id()));
    std::fs::write(&path, &bytes).expect("write the muxed file");
    let audio_index = two
        .media
        .tracks
        .iter()
        .position(|track| matches!(track.spec.config, CodecConfig::Aac { .. }))
        .expect("the fixture carries AAC audio");
    if let Some(audio) = crate::discontinuity::ffprobe_dts(&path, "a:0") {
        crate::discontinuity::assert_non_decreasing("audio", &audio);
        assert_eq!(
            audio.len(),
            two.media.tracks[audio_index].samples.len(),
            "ffprobe reads every audio packet"
        );
        let split = two.split[audio_index];
        let gap = audio[split] - audio[split - 1];
        assert!(
            gap > 1.0,
            "the break must appear as a real jump in the audio timeline, gap {gap}"
        );
    }
    let video_index = two
        .media
        .tracks
        .iter()
        .position(|track| matches!(track.spec.config, CodecConfig::Avc { .. }))
        .expect("the fixture carries AVC video");
    if let Some(video) = crate::discontinuity::ffprobe_pts(&path, "v:0") {
        assert_eq!(
            video.len(),
            two.media.tracks[video_index].samples.len(),
            "ffprobe reads every video packet"
        );
        // A reordered (B-frame) stream's packets are in decode order, so the
        // continuity the file must keep is that the whole second source sits
        // after the whole first one, with the break's gap intact.
        let split = two.split[video_index];
        let first_end = video[..split]
            .iter()
            .copied()
            .fold(f64::NEG_INFINITY, f64::max);
        let second_start = video[split..].iter().copied().fold(f64::INFINITY, f64::min);
        assert!(
            second_start - first_end > 1.0,
            "the second source must sit after the first with the break's gap: \
             first ends {first_end}, second starts {second_start}"
        );
    }
    let _ = std::fs::remove_file(&path);
}

/// A break that moves a Block timestamp **backwards** cannot be written as
/// such: a file's `Cues` and patched `Info.Duration` describe one
/// non-decreasing timeline (RFC 9559 §11, §12). It is expressed by rebasing
/// instead: each track's first Block after the break continues from its own
/// last timestamp plus its last duration, its later Blocks keep their spacing,
/// and the `Cues` written after the break use those rebased times. An
/// independent reader (the EBML walker for Clusters, `ffprobe` for packet
/// times, the crate's own demuxer for the samples) must see a monotonic,
/// complete, playable file.
#[test]
fn a_backward_break_rebases_onto_the_continuing_timeline() {
    let two = crate::discontinuity::backwards();
    let outputs = crate::discontinuity::drive(
        &mut MkvMuxStage::new(std::time::Duration::from_millis(5_000)),
        &two,
        None,
    )
    .expect("a backward break is expressed by rebasing");
    let bytes = crate::mux_bytes::apply(outputs);

    let clusters = crate::discontinuity::cluster_timestamps(&bytes);
    assert!(
        clusters.len() >= 2,
        "the file must carry more than one Cluster, got {clusters:?}"
    );
    for pair in clusters.windows(2) {
        assert!(
            pair[1] >= pair[0],
            "Cluster timestamps must not step backwards: {} then {}",
            pair[0],
            pair[1]
        );
    }

    let remuxed = test_harness::collect::media_from_matroska(
        &bytes,
        test_harness::collect::CollectMode::Seekable,
    )
    .expect("the muxed file re-demuxes");
    let mut seams = Vec::new();
    for (track, &split) in two.media.tracks.iter().zip(&two.split) {
        let got = remuxed
            .tracks
            .iter()
            .find(|candidate| candidate.spec.track_id == track.spec.track_id)
            .expect("every track survives the break");
        assert_eq!(
            track.samples.len(),
            got.samples.len(),
            "every sample of track {:?} survives the break",
            track.spec.track_id
        );
        let times: Vec<i64> = got
            .samples
            .iter()
            .filter_map(|sample| {
                sample
                    .timing
                    .and_then(|timing| timing.pts().or_else(|| timing.dts()))
            })
            .collect();
        if matches!(track.spec.config, CodecConfig::Aac { .. }) {
            // Audio has no reordering, so its presentation times are the
            // track's timeline and must not step back.
            crate::discontinuity::assert_non_decreasing(
                &format!("track {:?}", track.spec.track_id),
                &times.iter().map(|tick| *tick as f64).collect::<Vec<_>>(),
            );
        } else {
            // A reordered video track's Blocks are in decode order, so its
            // presentation times may interleave; what the rebase must keep is
            // that the whole second source sits at or after the first's end.
            let first_end = times[..split].iter().copied().fold(i64::MIN, i64::max);
            let second_start = times[split..].iter().copied().fold(i64::MAX, i64::min);
            assert!(
                second_start >= first_end,
                "the rebased second source must not overlap the first: \
                 first ends {first_end}, second starts {second_start}"
            );
        }
        if let Some(seam) = got.samples.get(split).and_then(|sample| {
            sample
                .timing
                .and_then(|timing| timing.pts().or_else(|| timing.dts()))
        }) {
            seams.push(
                i64::try_from(broadcast_core::timescale::rescale(
                    i128::from(seam),
                    got.spec.timescale,
                    broadcast_core::timescale::Timescale::try_new(1_000)
                        .expect("a non-zero timescale"),
                    broadcast_core::timescale::Rounding::Floor,
                ))
                .unwrap_or(i64::MAX),
            );
        }
    }
    // The Cluster the break opens starts at whichever track's first post-break
    // Block the writer interleaves first — the earliest rebased seam.
    let seam = *seams.iter().min().expect("every track has a seam Block");
    assert!(
        clusters.contains(&seam),
        "a Cluster must start at the rebased seam (tick {seam}), got {clusters:?}"
    );
    // Each track continues from its own last end, so a rebase by another
    // track's duration would be visible as two equal seams.
    assert!(
        seams.windows(2).any(|pair| pair[0] != pair[1]),
        "the tracks must end at different times for this test to bite: {seams:?}"
    );

    let path =
        crate::discontinuity::scratch_path(&format!("dc1-mkv-backward-{}.mkv", std::process::id()));
    std::fs::write(&path, &bytes).expect("write the muxed file");
    let audio_index = two
        .media
        .tracks
        .iter()
        .position(|track| matches!(track.spec.config, CodecConfig::Aac { .. }))
        .expect("the fixture carries AAC audio");
    if let Some(audio) = crate::discontinuity::ffprobe_dts(&path, "a:0") {
        crate::discontinuity::assert_non_decreasing("audio", &audio);
        assert_eq!(
            audio.len(),
            two.media.tracks[audio_index].samples.len(),
            "ffprobe reads every audio packet, so the file plays to the end"
        );
        let split = two.split[audio_index];
        let gap = audio[split] - audio[split - 1];
        assert!(
            gap <= 1.0,
            "a rebased break continues the timeline, gap {gap} seconds"
        );
    }
    let video_index = two
        .media
        .tracks
        .iter()
        .position(|track| matches!(track.spec.config, CodecConfig::Avc { .. }))
        .expect("the fixture carries AVC video");
    if let Some(video) = crate::discontinuity::ffprobe_pts(&path, "v:0") {
        assert_eq!(
            video.len(),
            two.media.tracks[video_index].samples.len(),
            "ffprobe reads every video packet"
        );
        let split = two.split[video_index];
        let first_end = video[..split]
            .iter()
            .copied()
            .fold(f64::NEG_INFINITY, f64::max);
        let second_start = video[split..].iter().copied().fold(f64::INFINITY, f64::min);
        assert!(
            second_start >= first_end,
            "the rebased second source sits at or after the first one's end: \
             first ends {first_end}, second starts {second_start}"
        );
    }
    let _ = std::fs::remove_file(&path);
}

/// A break that falls inside a GOP (no video keyframe there) is the case the
/// writer's own rules cannot see: only the explicit `Cluster` cut can start a
/// new one at the break. A long cluster span removes the span rule, so the
/// assertion pins the discontinuity handling itself.
#[test]
fn a_break_inside_a_gop_cuts_a_cluster_where_no_natural_rule_would() {
    use broadcast_common::stage::{Stage, Timestamp};
    use transmux::mux_stage::{MuxInput, MuxOutput};

    let two = crate::discontinuity::forward();
    let video = two
        .media
        .tracks
        .iter()
        .find(|track| matches!(track.spec.config, CodecConfig::Avc { .. }))
        .expect("the fixture carries AVC video");
    let break_after = 5usize;
    let sample = &video.samples[break_after];
    assert!(
        !sample.flags.is_sync,
        "the break must fall on a non-keyframe, or a natural Cluster cut hides it"
    );
    let pts = sample
        .timing
        .and_then(|timing| timing.pts())
        .expect("a timed video sample");
    let nanos = i128::from(pts) * i128::from(broadcast_common::units::NANOS_PER_SEC)
        / i128::from(video.spec.timescale.get());
    let break_tick = i64::try_from(nanos / 1_000_000).expect("a small tick");

    let mut stage = MkvMuxStage::new(std::time::Duration::from_secs(60));
    let mut outputs = Vec::new();
    stage
        .feed(
            MuxInput::Track(Box::new(video.spec.clone())),
            Timestamp::ZERO,
        )
        .expect("announce video");
    for (index, sample) in video.samples.iter().enumerate() {
        if index == break_after {
            stage
                .feed(MuxInput::Discontinuity { track: None }, Timestamp::ZERO)
                .expect("a break is expressible");
        }
        stage
            .feed(
                MuxInput::Sample {
                    track: video.spec.track_id,
                    sample: sample.clone(),
                },
                Timestamp::ZERO,
            )
            .expect("video sample");
        while let Some(output) = stage.poll() {
            outputs.push(output);
        }
    }
    stage.finish().expect("finish the file");
    while let Some(output) = stage.poll() {
        outputs.push(output);
    }
    assert!(
        outputs
            .iter()
            .any(|output| matches!(output, MuxOutput::Bytes(_)))
    );
    let bytes = crate::mux_bytes::apply(outputs);
    let clusters = crate::discontinuity::cluster_timestamps(&bytes);
    assert!(
        clusters.contains(&break_tick),
        "a Cluster must start at the mid-GOP break (tick {break_tick}), got {clusters:?}"
    );
}
