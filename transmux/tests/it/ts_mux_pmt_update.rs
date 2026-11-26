//! A live TS push whose track set changes mid-stream (a PMT update): the TS
//! muxer stage must emit a new PMT with `version_number` incremented modulo 32
//! (ISO/IEC 13818-1 §2.4.4.9) and keep the stream going, never refuse.
//!
//! The fixture's video track is announced and fed first; the audio track is
//! announced only after a few video samples, exactly as a mid-stream PMT
//! change adds an elementary stream. The output must carry both PMT versions,
//! the audio samples from the announcement on, and every video sample.

use crate::ts_psi::{parse_pmt, payload_offset, pid_of, pusi_of};
use broadcast_common::Stage;
use broadcast_common::stage::Timestamp;
use transmux::mux_stage::{MuxInput, MuxOutput};
use transmux::{DEFAULT_PSI_INTERVAL, DemuxEvent, StreamingTsDemux, TsMuxStage};

const PMT_PID: u16 = 0x1000;

/// Every PMT section in `ts`, in order.
fn pmt_sections(ts: &[u8]) -> Vec<Vec<u8>> {
    ts.chunks_exact(188)
        .filter(|p| pid_of(p) == PMT_PID && pusi_of(p))
        .filter_map(|p| {
            let off = payload_offset(p);
            if off >= 188 {
                return None;
            }
            let payload = &p[off..];
            let start = 1 + usize::from(payload[0]);
            let section = payload.get(start..)?;
            if section.len() < 3 {
                return None;
            }
            let len = (usize::from(section[1] & 0x0F) << 8) | usize::from(section[2]);
            section.get(..3 + len).map(<[u8]>::to_vec)
        })
        .collect()
}

/// A PMT section's `version_number` (5 bits after the section header).
fn pmt_version(section: &[u8]) -> u8 {
    (section[5] >> 1) & 0x1F
}

/// Decode-ordered `(track index, sample)` feed of `media`.
fn interleaved(media: &test_harness::collect::Collected) -> Vec<(usize, transmux::Sample)> {
    let mut items: Vec<(i128, usize, transmux::Sample)> = Vec::new();
    for (index, track) in media.tracks.iter().enumerate() {
        for sample in &track.samples {
            let dts = i128::from(sample.timing.and_then(|t| t.dts()).unwrap_or(0)) * 90_000
                / i128::from(track.spec.timescale.get());
            items.push((dts, index, sample.clone()));
        }
    }
    items.sort_by_key(|(dts, index, _)| (*dts, *index));
    items
        .into_iter()
        .map(|(_, index, sample)| (index, sample))
        .collect()
}

/// Samples a demuxer recovers, keyed by a stable track fingerprint.
fn recovered(ts: &[u8]) -> (usize, usize) {
    let media = test_harness::collect::media_from_ts(ts).expect("re-muxed stream demuxes");
    let video = media
        .tracks
        .iter()
        .find(|t| matches!(t.spec.config, transmux::CodecConfig::Avc { .. }))
        .map_or(0, |t| t.samples.len());
    let audio = media
        .tracks
        .iter()
        .find(|t| matches!(t.spec.config, transmux::CodecConfig::Aac { .. }))
        .map_or(0, |t| t.samples.len());
    (video, audio)
}

#[test]
fn a_mid_stream_track_addition_emits_a_new_pmt_and_continues() {
    let bytes = test_fixtures::read("ts/h264-aac.ts");
    let media = test_harness::collect::media_from_ts(bytes.as_slice()).expect("demux");
    let video = media
        .tracks
        .iter()
        .position(|t| matches!(t.spec.config, transmux::CodecConfig::Avc { .. }))
        .expect("video track");
    let audio = media
        .tracks
        .iter()
        .position(|t| matches!(t.spec.config, transmux::CodecConfig::Aac { .. }))
        .expect("audio track");

    let mut stage = TsMuxStage::new(Some(DEFAULT_PSI_INTERVAL));
    let announce = |stage: &mut TsMuxStage, index: usize| {
        Stage::feed(
            stage,
            MuxInput::Track(Box::new(media.tracks[index].spec.clone())),
            Timestamp::ZERO,
        )
        .expect("announce track");
    };
    let mut audio_fed = 0usize;
    let mut video_fed = 0usize;
    let mut announced_audio = false;
    let switch_after = 10usize;
    announce(&mut stage, video);

    for (index, sample) in interleaved(&media) {
        // The PMT change: the audio track joins once ten video samples have
        // been pushed, so the muxer must re-announce the programme mid-stream.
        if !announced_audio && video_fed >= switch_after {
            announce(&mut stage, audio);
            announced_audio = true;
        }
        if index == audio && !announced_audio {
            continue;
        }
        if index == audio {
            audio_fed += 1;
        } else {
            video_fed += 1;
        }
        Stage::feed(
            &mut stage,
            MuxInput::Sample {
                track: media.tracks[index].spec.track_id,
                sample,
            },
            Timestamp::ZERO,
        )
        .expect("mux sample");
    }
    Stage::finish(&mut stage).expect("finish");

    let mut out = Vec::new();
    while let Some(item) = Stage::poll(&mut stage) {
        if let MuxOutput::Bytes(chunk) = item {
            out.extend_from_slice(&chunk);
        }
    }
    assert_eq!(out.len() % 188, 0, "whole TS packets");

    let pmts = pmt_sections(&out);
    assert!(pmts.len() >= 2, "the PSI must repeat past the update");
    let first = parse_pmt(&pmts[0]);
    assert_eq!(first.len(), 1, "the first PMT declares the video only");
    assert_eq!(first[0].0, 0x1B, "AVC stream_type");
    assert_eq!(pmt_version(&pmts[0]), 0);
    let updated = pmts
        .iter()
        .find(|section| parse_pmt(section).len() == 2)
        .expect("a PMT after the update declares both tracks");
    let entries = parse_pmt(updated);
    assert_eq!(entries.len(), 2, "video + audio");
    assert!(
        entries.iter().any(|&(stream_type, _)| stream_type == 0x1B)
            && entries.iter().any(|&(stream_type, _)| stream_type == 0x0F),
        "AVC video + ADTS AAC: {entries:?}"
    );
    assert_eq!(
        pmt_version(updated),
        pmt_version(&pmts[0]) + 1,
        "a changed PMT bumps version_number by one (§2.4.4.9)"
    );
    assert!(
        pmts.iter()
            .all(|section| pmt_version(section) == 0 || pmt_version(section) == 1),
        "a re-announcement that changes nothing must not keep bumping the version"
    );

    // The stream continued: every video sample from the push is recovered, and
    // the audio samples from the announcement on.
    let (video_out, audio_out) = recovered(&out);
    assert_eq!(video_out, video_fed, "every video sample survives");
    assert!(audio_fed > 0, "the test must feed audio after the update");
    assert_eq!(
        audio_out, audio_fed,
        "every post-update audio sample survives"
    );

    // And the result is still one continuous, demuxable transport stream.
    let mut streaming = StreamingTsDemux::new();
    streaming.feed(&out).expect("streaming demux feeds");
    streaming.finish().expect("streaming demux finishes");
    let mut added = 0usize;
    while let Some(event) = streaming.poll_event() {
        if let DemuxEvent::TrackAdded(_) = event {
            added += 1;
        }
    }
    assert_eq!(added, 2, "both tracks resolve from the muxed stream");
}
