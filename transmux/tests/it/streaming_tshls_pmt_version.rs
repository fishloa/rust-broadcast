//! `StreamingTsHlsSegmenter::add_track` PMT `version_number` gate (issue #1699).
//!
//! ISO/IEC 13818-1:2023 §2.4.4.9 requires `version_number` to increment
//! (modulo 32) whenever a PMT section's content changes; a conforming demuxer,
//! `StreamingTsDemux` included, ignores a same-version PMT. Real fixture
//! `fixtures/ts/h264-aac.ts`: the segmenter is built video-only, cuts a
//! segment, then audio joins through `add_track`.

use broadcast_common::Unpackage;
use broadcast_common::crc32_mpeg2;
use transmux::pipeline::{CodecConfig, Sample, TrackSpec};
use transmux::{DemuxEvent, StreamingTsDemux, StreamingTsHlsSegmenter, TsDemux, TsSegment};

const TS_PACKET_SIZE: usize = 188;
const PMT_PID: u16 = 0x1000;
const PUSI_MASK: u8 = 0x40;
const POINTER_FIELD_LEN: usize = 1;
const TS_HEADER_LEN: usize = 4;
const SECTION_HEADER_LEN: usize = 3;
const VERSION_BYTE_OFFSET_IN_SECTION: usize = 5;
const VERSION_MASK: u8 = 0x1F;
const VERSION_SHIFT: u32 = 1;
const CURRENT_NEXT_MASK: u8 = 0x01;
const SECTION_LENGTH_HI_MASK: u8 = 0x0F;

struct PmtObservation {
    version: u8,
    current_next: bool,
    crc_ok: bool,
}

fn pmts(ts: &[u8]) -> Vec<PmtObservation> {
    let mut out = Vec::new();
    for pkt in ts.chunks_exact(TS_PACKET_SIZE) {
        let pid = (u16::from(pkt[1] & 0x1F) << 8) | u16::from(pkt[2]);
        if pid != PMT_PID || pkt[1] & PUSI_MASK == 0 {
            continue;
        }
        let s = &pkt[TS_HEADER_LEN + POINTER_FIELD_LEN..];
        let len = (usize::from(s[1] & SECTION_LENGTH_HI_MASK) << 8) | usize::from(s[2]);
        let section = &s[..SECTION_HEADER_LEN + len];
        let vb = section[VERSION_BYTE_OFFSET_IN_SECTION];
        out.push(PmtObservation {
            version: (vb >> VERSION_SHIFT) & VERSION_MASK,
            current_next: vb & CURRENT_NEXT_MASK == 1,
            crc_ok: crc32_mpeg2::compute(section) == 0,
        });
    }
    out
}

fn is_avc(c: &CodecConfig) -> bool {
    matches!(c, CodecConfig::Avc { .. })
}

struct Run {
    before: Vec<TsSegment>,
    after: Vec<TsSegment>,
    audio_spec: TrackSpec,
}

fn run() -> Run {
    let path = test_fixtures::path("ts/h264-aac.ts");
    let ts = std::fs::read(path).expect("h264-aac.ts fixture");
    let ir = TsDemux::new().unpackage(&ts).expect("demux fixture");
    let vpos = ir
        .tracks
        .iter()
        .position(|t| is_avc(&t.spec.config))
        .expect("video track");
    let apos = 1 - vpos;
    let video_spec = ir.tracks[vpos].spec.clone();
    let audio_spec = ir.tracks[apos].spec.clone();
    let video: Vec<Sample> = ir.tracks[vpos].samples.clone();
    let audio: Vec<Sample> = ir.tracks[apos].samples.clone();
    let vid = video_spec.track_id;
    let aid = audio_spec.track_id;
    let vscale = u64::from(video_spec.timescale.get());
    let ascale = u64::from(audio_spec.timescale.get());

    let mut seg = StreamingTsHlsSegmenter::new(vec![video_spec], 1, usize::MAX).unwrap();
    let mut before = Vec::new();
    let mut split = None;
    for (i, s) in video.iter().enumerate() {
        seg.push(vid, s.clone()).unwrap();
        let ready = seg.take_ready();
        if !ready.is_empty() {
            before.extend(ready);
            split = Some(i);
            break;
        }
    }
    let split = split.expect("a video-only segment is cut");
    seg.add_track(audio_spec.clone()).unwrap();

    let mut acc: u64 = video[..=split]
        .iter()
        .map(|s| u64::from(s.timing.and_then(|t| t.duration()).unwrap_or(0)))
        .sum();
    let mut items: Vec<(f64, bool, usize)> = Vec::new();
    for (i, s) in video.iter().enumerate().skip(split + 1) {
        items.push((acc as f64 / vscale as f64, true, i));
        acc += u64::from(s.timing.and_then(|t| t.duration()).unwrap_or(0));
    }
    let mut aacc = 0u64;
    for (i, s) in audio.iter().enumerate() {
        items.push((aacc as f64 / ascale as f64, false, i));
        aacc += u64::from(s.timing.and_then(|t| t.duration()).unwrap_or(0));
    }
    items.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap().then(a.1.cmp(&b.1)));
    for (_, is_video, idx) in items {
        if is_video {
            seg.push(vid, video[idx].clone()).unwrap();
        } else {
            seg.push(aid, audio[idx].clone()).unwrap();
        }
    }
    let mut after = seg.take_ready();
    seg.finish().unwrap();
    after.extend(seg.take_ready());
    Run {
        before,
        after,
        audio_spec,
    }
}

#[test]
fn pmt_version_bumps_once_when_a_track_is_added() {
    let r = run();
    assert!(
        r.after.len() >= 2,
        "need at least two segments after add_track"
    );
    for (i, s) in r.before.iter().enumerate() {
        let p = pmts(&s.bytes);
        assert!(!p.is_empty(), "segment {i} carries a PMT");
        for o in p {
            assert_eq!(o.version, 0, "pre-add segment {i} PMT version");
            assert!(o.current_next && o.crc_ok);
        }
    }
    for (i, s) in r.after.iter().enumerate() {
        let p = pmts(&s.bytes);
        assert!(!p.is_empty(), "segment {i} carries a PMT");
        for o in p {
            assert_eq!(
                o.version, 1,
                "post-add segment {i} keeps the bumped version"
            );
            assert!(o.current_next, "current_next_indicator stays 1");
            assert!(o.crc_ok, "post-add segment {i} PMT CRC");
        }
    }
}

#[test]
fn first_segment_after_add_track_is_flagged_discontinuous() {
    let r = run();
    assert!(r.after[0].discontinuous, "RFC 8216bis section 4.4.4.3");
    assert!(!r.after[1].discontinuous, "only the changing segment");
    assert!(r.before.iter().all(|s| !s.discontinuous));
}

#[test]
fn late_track_reaches_a_streaming_demux_that_joined_before_it() {
    let r = run();
    let mut ts = Vec::new();
    for s in r.before.iter().chain(r.after.iter()) {
        ts.extend_from_slice(&s.bytes);
    }
    let mut d = StreamingTsDemux::new();
    let mut added = Vec::new();
    for c in ts.chunks(TS_PACKET_SIZE * 64) {
        d.feed(c);
        while let Some(e) = d.poll_event() {
            if let DemuxEvent::TrackAdded(spec) = e {
                added.push(spec.config.clone());
            }
        }
    }
    d.finish();
    while let Some(e) = d.poll_event() {
        if let DemuxEvent::TrackAdded(spec) = e {
            added.push(spec.config.clone());
        }
    }
    assert!(added.iter().any(is_avc), "video announced");
    assert!(
        added
            .iter()
            .any(|c| std::mem::discriminant(c) == std::mem::discriminant(&r.audio_spec.config)),
        "the late audio track must be announced; got {} tracks",
        added.len()
    );
}
