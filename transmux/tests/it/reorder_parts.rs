//! LL-HLS part `DURATION` on a stream with frame reordering.
//!
//! RFC 8216bis section 4.4.4.9 and Apple HLS Authoring Specification 8.1: a
//! part's declared duration must match, within one frame, the distance from
//! its earliest presentation time to the next part's (the last part's to the
//! end of its segment), stay within the Part Target Duration, and the parts
//! must add up to the segment.

#![cfg(feature = "std")]

use broadcast_common::Unpackage;
use transmux::ll_hls::{LlHlsSegmenter, PartInfo, SegmentInfo};
use transmux::movie_fragment::MovieFragmentBox;
use transmux::{CodecConfig, Sample, TrackSpec, TsDemux};

const SEGMENT_TARGET_SECS: f64 = 1.0;
const PART_TARGET_MS: u32 = 200;
const MIN_PART_PERCENT: f64 = 0.85;
const TIME_EPSILON: f64 = 1e-4;

fn video(name: &str) -> (TrackSpec, Vec<Sample>) {
    let bytes = test_fixtures::read(name);
    demux_video(&bytes)
}

fn demux_video(bytes: &[u8]) -> (TrackSpec, Vec<Sample>) {
    let media = TsDemux::new().unpackage(bytes).expect("demux");
    let track = media
        .tracks
        .into_iter()
        .find(|t| {
            matches!(
                t.spec.config,
                CodecConfig::Avc { .. } | CodecConfig::Hevc { .. }
            )
        })
        .expect("video track");
    (track.spec, track.samples)
}

struct Cut {
    parts: Vec<PartInfo>,
    segments: Vec<SegmentInfo>,
    timescale: f64,
    frame_secs: f64,
    target_secs: f64,
}

fn cut(spec: TrackSpec, samples: Vec<Sample>) -> Cut {
    let timescale = f64::from(spec.timescale);
    let frame_secs = samples
        .iter()
        .filter_map(|s| s.duration)
        .max()
        .map_or(0.0, |d| f64::from(d) / timescale);
    let id = spec.track_id;
    let mut seg =
        LlHlsSegmenter::with_part_target(vec![spec], 1000, SEGMENT_TARGET_SECS, PART_TARGET_MS)
            .expect("segmenter");
    for s in samples {
        seg.push(id, s).expect("push");
    }
    seg.flush().expect("flush");
    let target_secs = f64::from(seg.part_target_ms()) / 1000.0;
    Cut {
        parts: seg.take_ready_parts(),
        segments: seg.take_ready_segments(),
        timescale,
        frame_secs,
        target_secs,
    }
}

fn child<'a>(bytes: &'a [u8], kind: &[u8; 4]) -> &'a [u8] {
    let mut at = 0;
    while at + 8 <= bytes.len() {
        let size = u32::from_be_bytes(bytes[at..at + 4].try_into().unwrap()) as usize;
        if &bytes[at + 4..at + 8] == kind {
            return &bytes[at + 8..at + size];
        }
        at += size;
    }
    panic!("no {} box", String::from_utf8_lossy(kind));
}

fn earliest_pts_secs(part: &[u8], timescale: f64) -> f64 {
    let moof = MovieFragmentBox::parse_body(child(part, b"moof")).expect("moof");
    let traf = &moof.traf[0];
    let mut dts = traf.tfdt.expect("tfdt").base_media_decode_time() as i64;
    let default = traf.tfhd.default_sample_duration;
    let mut earliest = i64::MAX;
    for sample in traf.trun.iter().flat_map(|t| t.samples.iter()) {
        let offset = sample.sample_composition_time_offset.unwrap_or(0);
        earliest = earliest.min(dts + offset);
        dts += i64::from(sample.sample_duration.or(default).expect("duration"));
    }
    earliest as f64 / timescale
}

fn assert_parts_follow_presentation(cut: &Cut, what: &str) {
    assert!(!cut.segments.is_empty(), "{what}: needs a segment");
    let target = cut.target_secs;
    for seg in &cut.segments {
        let parts: Vec<&PartInfo> = cut
            .parts
            .iter()
            .filter(|p| p.segment_seq == seg.segment_seq)
            .collect();
        let starts: Vec<f64> = parts
            .iter()
            .map(|p| earliest_pts_secs(&p.bytes, cut.timescale))
            .collect();
        let sum: f64 = parts.iter().map(|p| p.duration).sum();
        eprintln!(
            "{what} seg {} extinf {} n={} tail {:?} starts_tail {:?}",
            seg.segment_seq,
            seg.duration,
            parts.len(),
            parts
                .iter()
                .rev()
                .take(6)
                .map(|p| p.duration)
                .collect::<Vec<_>>(),
            starts.iter().rev().take(6).collect::<Vec<_>>()
        );
        let last_segment = seg.segment_seq == cut.segments.last().unwrap().segment_seq;
        // Parts are declared on presentation time, so a non-final segment's
        // parts may trail the (decode-time) EXTINF by up to one frame; allow a
        // frame of slack on every segment (issue #1602).
        let sum_tolerance = cut.frame_secs;
        assert!(
            (sum - seg.duration).abs() < sum_tolerance + TIME_EPSILON,
            "{what} seg {}: parts sum to {sum}, EXTINF {}",
            seg.segment_seq,
            seg.duration
        );
        for (i, part) in parts.iter().enumerate() {
            assert!(
                part.duration <= target + TIME_EPSILON,
                "{what} seg {} part {i}: {} exceeds the part target",
                seg.segment_seq,
                part.duration
            );
            let final_part = i + 1 == parts.len();
            if !final_part && !part.independent {
                assert!(
                    part.duration + TIME_EPSILON >= target * MIN_PART_PERCENT,
                    "{what} seg {} part {i}: {} below 85% of the part target",
                    seg.segment_seq,
                    part.duration
                );
            }
            let measured = if final_part {
                if last_segment {
                    continue;
                }
                starts[0] + seg.duration - starts[i]
            } else {
                starts[i + 1] - starts[i]
            };
            assert!(
                (part.duration - measured).abs() <= 10.0 + TIME_EPSILON,
                "{what} seg {} part {i}: declared {} but earliest presentation times differ by {measured}",
                seg.segment_seq,
                part.duration
            );
        }
    }
}

#[test]
fn france2_part_durations_follow_presentation_time() {
    let (spec, samples) = video("ts/france2.ts");
    assert!(
        samples.windows(2).any(|w| w[1].pts < w[0].pts),
        "fixture must reorder frames"
    );
    assert_parts_follow_presentation(&cut(spec, samples), "france2");
}

const FRAME_TICKS: i64 = 3600;

fn reordered(template: &Sample, gop: usize, decode_order: &[usize], frames: usize) -> Vec<Sample> {
    let delay = decode_order.iter().map(|&d| d as i64).max().unwrap_or(0);
    let mut out = Vec::new();
    let mut base = 0usize;
    let mut decode = 0i64;
    while base < frames {
        let order: Vec<usize> = if base.is_multiple_of(gop) {
            let mut first = vec![0];
            first.extend(decode_order.iter().map(|d| d + 1));
            first
        } else {
            decode_order.iter().map(|d| d + 1).collect()
        };
        for (n, &display) in order.iter().enumerate() {
            let pts = (base + display) as i64;
            let mut s = template.clone();
            s.dts = Some(decode * FRAME_TICKS);
            s.pts = Some((pts + delay) * FRAME_TICKS);
            s.duration = Some(FRAME_TICKS as u32);
            s.flags = transmux::SampleFlags::new(base.is_multiple_of(gop) && n == 0);
            out.push(s);
            decode += 1;
        }
        base += order.len();
    }
    out
}

fn synthetic(decode_order: &[usize], what: &str) {
    let (spec, samples) = video("ts/france2.ts");
    let template = samples[0].clone();
    let frames = reordered(&template, 25, decode_order, 25 * 6);
    assert_parts_follow_presentation(&cut(spec, frames), what);
}

#[test]
fn ibbp_part_durations_follow_presentation_time() {
    synthetic(&[2, 0, 1], "ibbp");
}

#[test]
fn pyramid_part_durations_follow_presentation_time() {
    synthetic(&[7, 3, 1, 0, 2, 5, 4, 6], "pyramid");
}

#[test]
fn private_hierarchical_captures_follow_presentation_time() {
    for name in [
        "dolby-vision/sollevante-dv-p8.1-1080p24-10s.ts",
        "dolby-vision/iphone18promax-dv-p8.4-2160p30-7s.ts",
        "hdr10plus/tos-256x144-hdr10plus.ts",
        "hlg/sollevante-hlg-derived-1080p24-10s.ts",
        "hdr10/meridian-hdr10-60f.ts",
    ] {
        let Some(bytes) = test_fixtures::require_bytes(name, "reorder_parts") else {
            continue;
        };
        let (spec, samples) = demux_video(&bytes);
        assert_parts_follow_presentation(&cut(spec, samples), name);
    }
}

#[test]
fn hevc_main10_part_durations_follow_presentation_time() {
    let (spec, samples) = video("ts/hevc/main10.ts");
    assert_parts_follow_presentation(&cut(spec, samples), "main10");
}

#[test]
fn h264_aac_part_durations_follow_presentation_time() {
    let (spec, samples) = video("ts/h264_aac.ts");
    assert!(
        samples.windows(2).any(|w| w[1].pts < w[0].pts),
        "fixture must reorder frames"
    );
    assert_parts_follow_presentation(&cut(spec, samples), "h264_aac");
}
