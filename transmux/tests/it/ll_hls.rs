//! Low-Latency HLS gate (issue #454): partial segments + playlist directives,
//! per RFC 8216bis.
//!
//! Every test bites:
//!  - part bytes are parsed with a std-only top-level box walker (`moof`+`mdat`),
//!    their `tfdt` read, and their durations summed against the full segment;
//!  - the `INDEPENDENT` flag is driven off a real sync/non-sync first sample;
//!  - the playlist text is asserted for the exact RFC 8216bis directives, and a
//!    non-low-latency playlist is asserted to carry NONE of them (opt-in);
//!  - a part's sample set is reconstructed and compared to `build_media_segment`.

use crate::hub_fixtures::{audio_track, dummy_avc_config, video_track};
use broadcast_core::timescale::Timescale;
use broadcast_hls::{DecimalSeconds, LowLatencyConfig, MediaPlaylist, MediaSegment, PartSpec};
use core::time::Duration;
use isobmff::movie_fragment::MovieFragmentBox;
use transmux::TrackId;
use transmux::ll_hls::LlHlsSegmenter;
use transmux::{CodecConfig, FragmentTrackData, Sample, TrackSpec};

fn timescale(value: u32) -> Timescale {
    Timescale::try_new(value).expect("a non-zero timescale")
}

// ---------------------------------------------------------------------------
// Track specs (minimal but structurally-real configs).
// ---------------------------------------------------------------------------

const VID_DUR: u32 = 3000; // 90 kHz, 1/30 s per AU

fn vsample(is_sync: bool, byte: u8) -> Sample {
    // Synthetic sample for the duration-driven segmenter: dts/pts stay `None`
    // (media plane step 2c — never fabricate a timestamp); the timeline comes
    // from `duration`, exactly as before.
    Sample::new(vec![byte; 32], None, None, Some(VID_DUR), is_sync).expect("valid timing")
}

/// The part target is a `Duration`, and converting it to the wire
/// `#EXT-X-PART-INF:PART-TARGET` value at the edition edge is exact: 334 ms of
/// wall time renders as `0.334`, matching the whole-millisecond value the
/// segmenter advertises. (Before the retype the accessor returned a bare
/// `f64`/`u32`; this pins the typed conversion.)
#[test]
fn part_target_duration_renders_the_exact_wire_value() {
    let seg = LlHlsSegmenter::with_part_target(
        vec![video_track()],
        timescale(1000),
        Duration::from_secs(1),
        Duration::from_millis(334),
    )
    .unwrap();
    assert_eq!(seg.part_target(), Duration::from_millis(334));
    let ll = LowLatencyConfig::default()
        .with_part_target(DecimalSeconds::from_duration(seg.part_target()));
    let pl = MediaPlaylist {
        version: 9,
        target_duration: 1,
        msn: broadcast_hls::MediaSequence::new(0),
        discontinuity_sequence: 0,
        segments: vec![],
        endlist: false,
        extra_tags: vec![],
        low_latency: Some(ll),
        iframes_only: false,
        open_segment: None,
        ..Default::default()
    };
    let m3u8 = pl.to_m3u8().unwrap();
    assert!(
        m3u8.contains("#EXT-X-PART-INF:PART-TARGET=0.334\n"),
        "PART-TARGET must render from the typed Duration:\n{m3u8}"
    );
}

// ---------------------------------------------------------------------------
// Minimal top-level box walker — no external dependency, no hardcoded offsets.
// ---------------------------------------------------------------------------

fn top_boxes(buf: &[u8]) -> Vec<([u8; 4], std::ops::Range<usize>)> {
    let mut out = Vec::new();
    let mut off = 0usize;
    while off + 8 <= buf.len() {
        let size =
            u32::from_be_bytes([buf[off], buf[off + 1], buf[off + 2], buf[off + 3]]) as usize;
        let ty = [buf[off + 4], buf[off + 5], buf[off + 6], buf[off + 7]];
        let box_len = if size == 0 { buf.len() - off } else { size };
        assert!(
            box_len >= 8 && off + box_len <= buf.len(),
            "malformed box {ty:?}"
        );
        out.push((ty, off..off + box_len));
        off += box_len;
    }
    out
}

fn find_box_body<'a>(data: &'a [u8], fourcc: &[u8; 4]) -> Option<&'a [u8]> {
    top_boxes(data)
        .into_iter()
        .find(|(t, _)| t == fourcc)
        .map(|(_, r)| &data[r.start + 8..r.end])
}

// ===========================================================================
// Test 1 — parts are valid independent fragments; durations sum to the segment
// ===========================================================================

#[test]
fn parts_are_valid_fragments_and_sum_to_segment() {
    // segment target 1000 ms, part target 334 ms.
    let mut seg = LlHlsSegmenter::with_part_target(
        vec![video_track()],
        timescale(1000),
        Duration::from_secs(1),
        Duration::from_millis(334),
    )
    .unwrap();

    // ~1 s of 30 fps video: 30 AUs @ 3000 ticks. First AU is a keyframe.
    for i in 0..30u8 {
        seg.push(TrackId::new(1), vsample(i == 0, i)).unwrap();
    }
    // Next keyframe past the target closes the segment; then flush the tail.
    seg.push(TrackId::new(1), vsample(true, 200)).unwrap();
    seg.flush().unwrap();

    let parts = seg.take_ready_parts();
    let segments = seg.take_ready_segments();

    // The 30-AU segment (=1 s at 334 ms parts) yields 3 parts.
    let seg1_parts: Vec<_> = parts.iter().filter(|p| p.segment_seq == 1).collect();
    assert_eq!(
        seg1_parts.len(),
        3,
        "1 s of 334 ms parts must be 3 parts, got {}",
        seg1_parts.len()
    );

    // Each part parses as exactly moof + mdat and carries a tfdt.
    for p in &seg1_parts {
        let tys: Vec<[u8; 4]> = top_boxes(&p.bytes).iter().map(|(t, _)| *t).collect();
        assert_eq!(tys, vec![*b"moof", *b"mdat"], "part = moof+mdat");
        let moof = find_box_body(&p.bytes, b"moof").expect("part has moof");
        let mf = MovieFragmentBox::parse_body(moof).expect("moof parses");
        let traf = mf.traf.first().expect("part traf");
        assert!(traf.tfdt.is_some(), "part carries a tfdt");
    }

    // The parts' durations sum to the full segment's duration.
    let seg1 = segments
        .iter()
        .find(|s| s.segment_seq == 1)
        .expect("segment 1");
    let parts_sum: Duration = seg1_parts.iter().map(|p| p.duration).sum();
    assert!(
        parts_sum.abs_diff(seg1.duration) < Duration::from_micros(1),
        "parts sum {parts_sum:?} != segment duration {:?}",
        seg1.duration
    );
    // And the segment is a whole 30-AU segment = 1.0 s.
    assert!(
        seg1.duration.abs_diff(Duration::from_secs(1)) < Duration::from_micros(1),
        "segment ~1 s"
    );
    assert_eq!(seg1.part_count, 3, "segment records 3 parts");
}

// ===========================================================================
// Test 2 — INDEPENDENT flag bites (sync first sample => YES, mid-GOP => no)
// ===========================================================================

#[test]
fn independent_flag_tracks_sync_first_sample() {
    let mut seg = LlHlsSegmenter::with_part_target(
        vec![video_track()],
        timescale(1000),
        Duration::from_secs(1),
        Duration::from_millis(334),
    )
    .unwrap();

    // 30 AUs: keyframe only at index 0. Part 1 starts at a sync sample; parts 2
    // and 3 start mid-GOP (no sync).
    for i in 0..30u8 {
        seg.push(TrackId::new(1), vsample(i == 0, i)).unwrap();
    }
    seg.push(TrackId::new(1), vsample(true, 200)).unwrap();
    seg.flush().unwrap();

    let parts = seg.take_ready_parts();
    let seg1: Vec<_> = parts.iter().filter(|p| p.segment_seq == 1).collect();
    assert_eq!(seg1.len(), 3);

    assert!(
        seg1[0].independent,
        "part 0 begins on a keyframe => INDEPENDENT"
    );
    assert!(
        !seg1[1].independent,
        "part 1 begins mid-GOP => not independent"
    );
    assert!(
        !seg1[2].independent,
        "part 2 begins mid-GOP => not independent"
    );

    // Render the parts into a playlist and assert the text reflects both cases.
    let parts_spec: Vec<PartSpec> = seg1
        .iter()
        .enumerate()
        .map(|(i, p)| {
            PartSpec::new(
                format!("seg1.{i}.m4s"),
                // The segmenter's own computed part duration (issue #1140):
                // always finite and non-negative.
                DecimalSeconds::from_duration(p.duration),
            )
            .with_independent(p.independent)
        })
        .collect();
    let pl = MediaPlaylist {
        version: 9,
        target_duration: 1,
        msn: broadcast_hls::MediaSequence::new(0),
        discontinuity_sequence: 0,
        segments: vec![MediaSegment {
            uri: "seg1.m4s".into(),
            duration: DecimalSeconds::new(1.0).unwrap(),
            discontinuous: false,
            parts: parts_spec,
            ..Default::default()
        }],
        endlist: false,
        extra_tags: vec![],
        low_latency: Some(
            LowLatencyConfig::default()
                .with_part_target(DecimalSeconds::new(0.334).unwrap())
                .with_part_hold_back(DecimalSeconds::new(1.002).unwrap()),
        ),
        iframes_only: false,
        open_segment: None,
        ..Default::default()
    };
    let m3u8 = pl.to_m3u8().unwrap();
    // Part 0 has INDEPENDENT=YES; parts 1 and 2 do not.
    //
    // The duration is deliberately NOT pinned here: it is an unrounded f64
    // straight out of the segmenter (11/30 s), and `broadcast-hls` now
    // renders durations losslessly rather than rounding them to
    // milliseconds (issue #872 — that rounding silently corrupted real
    // sub-millisecond durations such as Apple's `#EXTINF:9.9766`). What
    // this test is about is the INDEPENDENT flag landing on the right
    // part; the URI-anchored assertion below, the exact count, and the
    // mid-GOP negative case still pin that exactly.
    assert!(
        m3u8.contains("URI=\"seg1.0.m4s\",INDEPENDENT=YES"),
        "independent part must render INDEPENDENT=YES:\n{m3u8}"
    );
    let indep_count = m3u8.matches("INDEPENDENT=YES").count();
    assert_eq!(indep_count, 1, "exactly one part is independent");
    // The mid-GOP parts render without the flag.
    assert!(
        m3u8.contains("URI=\"seg1.1.m4s\"\n") && !m3u8.contains("seg1.1.m4s\",INDEPENDENT"),
        "mid-GOP part must NOT carry INDEPENDENT:\n{m3u8}"
    );
}

#[test]
fn part_with_a_mid_part_sync_sample_is_independent() {
    let mut seg = LlHlsSegmenter::with_part_target(
        vec![video_track()],
        timescale(1000),
        Duration::from_secs(1),
        Duration::from_millis(334),
    )
    .unwrap();
    for i in 0..30u8 {
        seg.push(TrackId::new(1), vsample(i == 0 || i == 15, i))
            .unwrap();
    }
    seg.push(TrackId::new(1), vsample(true, 200)).unwrap();
    seg.flush().unwrap();

    let parts = seg.take_ready_parts();
    let seg1: Vec<_> = parts.iter().filter(|p| p.segment_seq == 1).collect();
    assert_eq!(seg1.len(), 3);
    assert!(seg1[0].independent, "part 0 starts on a keyframe");
    assert!(
        seg1[1].independent,
        "part 1 holds the sync sample at AU 15 (RFC 8216bis 4.4.4.9: contains an independent frame)"
    );
    assert!(!seg1[2].independent, "part 2 has no sync sample");
}

// ===========================================================================
// Test 3 — playlist directive text bites (opt-in)
// ===========================================================================

#[test]
fn playlist_low_latency_directives_present_and_opt_in() {
    let parts = vec![
        PartSpec::new("seg0.0.m4s".into(), DecimalSeconds::new(0.334).unwrap())
            .with_independent(true),
        PartSpec::new("seg0.1.m4s".into(), DecimalSeconds::new(0.334).unwrap())
            .with_independent(false),
    ];
    let mut ll = LowLatencyConfig::default()
        .with_part_target(DecimalSeconds::new(0.334).unwrap())
        // Deliberately too small; renderer must raise to 3 x 0.334 = 1.002.
        .with_part_hold_back(DecimalSeconds::new(0.5).unwrap());
    ll.preload_hint_part = Some("seg0.2.m4s".into());
    let pl = MediaPlaylist {
        version: 9,
        target_duration: 1,
        msn: broadcast_hls::MediaSequence::new(0),
        discontinuity_sequence: 0,
        segments: vec![MediaSegment {
            uri: "seg0.m4s".into(),
            duration: DecimalSeconds::new(1.0).unwrap(),
            discontinuous: false,
            parts,
            ..Default::default()
        }],
        endlist: false,
        extra_tags: vec![],
        low_latency: Some(ll.clone()),
        iframes_only: false,
        open_segment: None,
        ..Default::default()
    };
    let m3u8 = pl.to_m3u8().unwrap();

    // #EXT-X-PART-INF exact.
    assert!(
        m3u8.contains("#EXT-X-PART-INF:PART-TARGET=0.334\n"),
        "PART-INF must carry PART-TARGET=0.334:\n{m3u8}"
    );
    // #EXT-X-SERVER-CONTROL with PART-HOLD-BACK >= 3 x part-target.
    let effective = ll
        .effective_part_hold_back()
        .expect("part target present")
        .get();
    assert!((effective - 1.002).abs() < 1e-6, "PHB floor = 3 x 0.334");
    assert!(
        m3u8.contains("#EXT-X-SERVER-CONTROL:CAN-BLOCK-RELOAD=YES,PART-HOLD-BACK=1.002\n"),
        "SERVER-CONTROL must carry CAN-BLOCK-RELOAD + raised PART-HOLD-BACK:\n{m3u8}"
    );
    assert!(
        effective >= 3.0 * ll.part_target.expect("part target present").get(),
        "PART-HOLD-BACK >= 3x part-target"
    );
    // #EXT-X-PART lines for the parts.
    assert!(
        m3u8.contains("#EXT-X-PART:DURATION=0.334,URI=\"seg0.0.m4s\",INDEPENDENT=YES\n"),
        "first part line:\n{m3u8}"
    );
    assert!(
        m3u8.contains("#EXT-X-PART:DURATION=0.334,URI=\"seg0.1.m4s\"\n"),
        "second (non-independent) part line:\n{m3u8}"
    );
    // Parts precede the parent #EXTINF.
    let part_pos = m3u8.find("#EXT-X-PART:").unwrap();
    let extinf_pos = m3u8.find("#EXTINF:").unwrap();
    assert!(part_pos < extinf_pos, "#EXT-X-PART must precede #EXTINF");
    // #EXT-X-PRELOAD-HINT for the next part.
    assert!(
        m3u8.contains("#EXT-X-PRELOAD-HINT:TYPE=PART,URI=\"seg0.2.m4s\"\n"),
        "preload hint:\n{m3u8}"
    );

    // Opt-in: the SAME segments with low_latency = None carry NONE of the tags.
    let plain = MediaPlaylist {
        low_latency: None,
        ..pl.clone()
    };
    let plain_m3u8 = plain.to_m3u8().unwrap();
    for tag in [
        "#EXT-X-PART-INF",
        "#EXT-X-SERVER-CONTROL",
        "#EXT-X-PART:",
        "#EXT-X-PRELOAD-HINT",
    ] {
        assert!(
            !plain_m3u8.contains(tag),
            "non-low-latency playlist must NOT contain {tag}:\n{plain_m3u8}"
        );
    }
}

// ===========================================================================
// Test 4 — part <-> segment consistency (parts aren't fabricated)
// ===========================================================================

#[test]
fn part_media_matches_whole_segment_build() {
    // Video + audio: exercise the interleaved final-part behaviour.
    let mut seg = LlHlsSegmenter::with_part_target(
        vec![video_track(), audio_track()],
        timescale(1000),
        Duration::from_secs(1),
        Duration::from_millis(334),
    )
    .unwrap();

    // Build a merged decode-order feed: 30 video AUs (3000 ticks each) + audio
    // AUs (1024 ticks @ 48 kHz), video keyframe only at index 0.
    const N_VID: usize = 30;
    const N_AUD: usize = 45;
    const AUD_DUR: u32 = 1024;

    let mut items: Vec<(f64, bool, usize)> = Vec::new();
    for i in 0..N_VID {
        items.push((i as f64 * VID_DUR as f64 / 90_000.0, true, i));
    }
    for j in 0..N_AUD {
        items.push((j as f64 * AUD_DUR as f64 / 48_000.0, false, j));
    }
    items.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap());

    // Record the exact samples we pushed, per track, so we can rebuild the whole
    // segment from the identical sample set.
    let mut vid_samples: Vec<Sample> = Vec::new();
    let mut aud_samples: Vec<Sample> = Vec::new();
    for (_, is_video, idx) in &items {
        if *is_video {
            let s = vsample(*idx == 0, *idx as u8);
            vid_samples.push(s.clone());
            seg.push(TrackId::new(1), s).unwrap();
        } else {
            let s = Sample::from_raw(
                vec![(*idx as u8).wrapping_add(1); 20],
                None,
                None,
                Some(AUD_DUR),
            )
            .expect("valid timing");
            aud_samples.push(s.clone());
            seg.push(TrackId::new(2), s).unwrap();
        }
    }
    // Close the (single) segment and flush.
    seg.push(TrackId::new(1), vsample(true, 250)).unwrap();
    seg.flush().unwrap();

    let parts = seg.take_ready_parts();
    let segments = seg.take_ready_segments();
    let seg1_parts: Vec<_> = parts.iter().filter(|p| p.segment_seq == 1).collect();
    assert!(seg1_parts.len() >= 2, "expected several parts");
    let seg1 = segments.iter().find(|s| s.segment_seq == 1).unwrap();

    // Reconstruct the sample set carried by the parts, per track (in order).
    let mut part_vid: Vec<Vec<u8>> = Vec::new();
    let mut part_aud: Vec<Vec<u8>> = Vec::new();
    for p in &seg1_parts {
        let moof = find_box_body(&p.bytes, b"moof").unwrap();
        let mdat = find_box_body(&p.bytes, b"mdat").unwrap();
        let mf = MovieFragmentBox::parse_body(moof).unwrap();
        // Walk trafs in order; each trun's sample sizes slice the mdat in order.
        let mut cursor = 0usize;
        for traf in &mf.traf {
            let tid = traf.tfhd.track_id;
            for run in &traf.trun {
                for s in &run.samples {
                    let sz = s.sample_size.expect("sample_size present") as usize;
                    let bytes = mdat[cursor..cursor + sz].to_vec();
                    cursor += sz;
                    if tid.value() == 1 {
                        part_vid.push(bytes);
                    } else {
                        part_aud.push(bytes);
                    }
                }
            }
        }
    }

    // The whole segment built from the identical sample set.
    let whole = {
        let frags = vec![
            FragmentTrackData::new(TrackId::new(1), 0, &vid_samples),
            FragmentTrackData::new(TrackId::new(2), 0, &aud_samples),
        ];
        transmux::pipeline::build_media_segment(1, &frags).unwrap()
    };

    // The parts' sample set (coded bytes, per track, in order) equals the whole
    // segment's sample set — proves the parts carry the real media, not stubs.
    let whole_moof = find_box_body(&whole, b"moof").unwrap();
    let whole_mdat = find_box_body(&whole, b"mdat").unwrap();
    let whole_mf = MovieFragmentBox::parse_body(whole_moof).unwrap();
    let mut whole_vid: Vec<Vec<u8>> = Vec::new();
    let mut whole_aud: Vec<Vec<u8>> = Vec::new();
    let mut cursor = 0usize;
    for traf in &whole_mf.traf {
        let tid = traf.tfhd.track_id;
        for run in &traf.trun {
            for s in &run.samples {
                let sz = s.sample_size.unwrap() as usize;
                let bytes = whole_mdat[cursor..cursor + sz].to_vec();
                cursor += sz;
                if tid.value() == 1 {
                    whole_vid.push(bytes);
                } else {
                    whole_aud.push(bytes);
                }
            }
        }
    }

    assert_eq!(
        part_vid, whole_vid,
        "video sample set: parts == whole segment"
    );
    assert_eq!(
        part_aud, whole_aud,
        "audio sample set: parts == whole segment"
    );

    // The whole segment emitted by the segmenter carries the identical coded
    // sample set (proves the emitted segment is not fabricated either). It differs
    // from the batch `build_media_segment(1, ..)` output only in its
    // `mfhd.sequence_number` (the segmenter numbers parts+segments contiguously),
    // so compare the parsed sample bytes, not raw bytes.
    let seg_moof = find_box_body(&seg1.bytes, b"moof").unwrap();
    let seg_mdat = find_box_body(&seg1.bytes, b"mdat").unwrap();
    let seg_mf = MovieFragmentBox::parse_body(seg_moof).unwrap();
    let mut seg_vid: Vec<Vec<u8>> = Vec::new();
    let mut seg_aud: Vec<Vec<u8>> = Vec::new();
    let mut c2 = 0usize;
    for traf in &seg_mf.traf {
        let tid = traf.tfhd.track_id;
        for run in &traf.trun {
            for s in &run.samples {
                let sz = s.sample_size.unwrap() as usize;
                let bytes = seg_mdat[c2..c2 + sz].to_vec();
                c2 += sz;
                if tid.value() == 1 {
                    seg_vid.push(bytes);
                } else {
                    seg_aud.push(bytes);
                }
            }
        }
    }
    assert_eq!(
        seg_vid, whole_vid,
        "segment video samples == build_media_segment"
    );
    assert_eq!(
        seg_aud, whole_aud,
        "segment audio samples == build_media_segment"
    );
    let _ = whole; // whole is referenced above via whole_vid/whole_aud
}

// ===========================================================================
// Test — a zero anchor-track timescale must not render `inf`/`NaN` into the
// playlist (audit finding #5).
// ===========================================================================

/// `timescale` is a wire `u32` (`mdhd.timescale`, ISO/IEC 14496-12:2026
/// §8.4.2) that `LlHlsSegmenter::with_part_target` does not itself validate —
/// only `target_duration`/`part_target` are checked at construction.
/// The part- and segment-duration computations divide by the anchor track's
/// timescale directly; before the fix, a zero timescale turned every one of
/// those into `f64::INFINITY`/`NaN`, rendered verbatim into
/// `#EXT-X-PART-INF`/`#EXT-X-PART`/`#EXTINF` text (Rust's `Display` for
/// `f64` renders those as the literal substrings `"inf"`/`"NaN"`) — a wrong
/// value shipped to every client with no panic to flag it. The durations are
/// now `Duration`s (finite and non-negative by type); this asserts on the
/// actual rendered playlist text.
#[test]
fn zero_anchor_timescale_does_not_render_inf_or_nan_into_playlist() {
    assert!(
        Timescale::try_new(0).is_err(),
        "a zero mdhd.timescale cannot reach the segmenter: it is refused at the type"
    );
    let zero_ts_video = TrackSpec::new(
        TrackId::new(1),
        timescale(1),
        CodecConfig::Avc {
            config: dummy_avc_config(),
            width: 320,
            height: 240,
        },
    );
    let mut seg = LlHlsSegmenter::with_part_target(
        vec![zero_ts_video],
        timescale(1000),
        Duration::from_secs(1),
        Duration::from_millis(334),
    )
    .expect("construction does not itself validate per-track timescale");

    for i in 0..10u8 {
        seg.push(TrackId::new(1), vsample(i == 0, i)).unwrap();
    }
    seg.flush().unwrap();

    let parts = seg.take_ready_parts();
    let segments = seg.take_ready_segments();
    assert!(!parts.is_empty(), "some part must have been produced");
    assert!(!segments.is_empty(), "some segment must have been produced");
    // Render exactly as a real caller would, and assert on the TEXT.
    let parts_spec: Vec<PartSpec> = parts
        .iter()
        .enumerate()
        .map(|(i, p)| {
            PartSpec::new(
                format!("seg1.{i}.m4s"),
                // Same reasoning as above: segmenter-computed, finite, >= 0.
                DecimalSeconds::from_duration(p.duration),
            )
            .with_independent(p.independent)
        })
        .collect();
    let seg1_duration = segments
        .first()
        .map(|s| s.duration)
        .unwrap_or(Duration::ZERO);
    let pl = MediaPlaylist {
        version: 9,
        target_duration: 1,
        msn: broadcast_hls::MediaSequence::new(0),
        discontinuity_sequence: 0,
        segments: vec![MediaSegment {
            uri: "seg1.m4s".into(),
            // The segmenter's own computed segment duration (issue #1140):
            // always finite and non-negative.
            duration: DecimalSeconds::from_duration(seg1_duration),
            discontinuous: false,
            parts: parts_spec,
            ..Default::default()
        }],
        endlist: false,
        extra_tags: vec![],
        low_latency: Some(
            // `part_target` is a `Duration`: always finite and non-negative.
            LowLatencyConfig::default()
                .with_part_target(DecimalSeconds::from_duration(seg.part_target()))
                .with_part_hold_back(DecimalSeconds::from_duration(seg.part_target() * 3)),
        ),
        iframes_only: false,
        open_segment: None,
        ..Default::default()
    };
    let m3u8 = pl.to_m3u8().unwrap();

    assert!(
        !m3u8.contains("inf"),
        "playlist must never render a literal `inf` duration:\n{m3u8}"
    );
    assert!(
        !m3u8.contains("NaN"),
        "playlist must never render a literal `NaN` duration:\n{m3u8}"
    );
    assert!(
        m3u8.contains("#EXTINF:"),
        "sanity: the playlist must still carry a real #EXTINF line:\n{m3u8}"
    );
}

// ===========================================================================
// Test — audio rides every part, and a part never exceeds PART-TARGET (r05-W27)
// ===========================================================================

/// Count the `traf` boxes in a part's `moof` (via the top-level walker).
fn traf_count(part: &[u8]) -> usize {
    let moof = find_box_body(part, b"moof").expect("moof in part");
    let mut n = 0;
    let mut off = 0usize;
    while off + 8 <= moof.len() {
        let size =
            u32::from_be_bytes([moof[off], moof[off + 1], moof[off + 2], moof[off + 3]]) as usize;
        if size < 8 || off + size > moof.len() {
            break;
        }
        if &moof[off + 4..off + 8] == b"traf" {
            n += 1;
        }
        off += size;
    }
    n
}

/// `tfdt.baseMediaDecodeTime` of the single `traf` in a part whose track_id
/// matches `want_track`; `None` when that track has no `traf` in this part.
fn part_tfdt_for(part: &[u8], want_track: u32) -> Option<u64> {
    let moof = find_box_body(part, b"moof")?;
    let mut off = 0usize;
    while off + 8 <= moof.len() {
        let size =
            u32::from_be_bytes([moof[off], moof[off + 1], moof[off + 2], moof[off + 3]]) as usize;
        if size < 8 || off + size > moof.len() {
            break;
        }
        if &moof[off + 4..off + 8] == b"traf" {
            let body = &moof[off + 8..off + size];
            // tfhd: 8-byte header + flags(3) + track_id(4).
            let mut p = 0usize;
            let mut tid = None;
            while p + 8 <= body.len() {
                let bs =
                    u32::from_be_bytes([body[p], body[p + 1], body[p + 2], body[p + 3]]) as usize;
                let bt = &body[p + 4..p + 8];
                if bs < 8 || p + bs > body.len() {
                    break;
                }
                if bt == b"tfhd" {
                    tid = Some(u32::from_be_bytes([
                        body[p + 12],
                        body[p + 13],
                        body[p + 14],
                        body[p + 15],
                    ]));
                } else if bt == b"tfdt" && tid == Some(want_track) {
                    let v = body[p + 8];
                    let raw = &body[p + 12..];
                    let v = if v == 1 {
                        u64::from_be_bytes([
                            raw[0], raw[1], raw[2], raw[3], raw[4], raw[5], raw[6], raw[7],
                        ])
                    } else {
                        u64::from(u32::from_be_bytes([raw[0], raw[1], raw[2], raw[3]]))
                    };
                    return Some(v);
                }
                p += bs;
            }
        }
        off += size;
    }
    None
}

/// Every non-final part of a muxed A/V segment carries both the video and the
/// audio track — a client playing at the live edge must have audio in each part,
/// not only in the segment's last one (RFC 8216bis §4.4.4.9, audit r05-W27).
#[test]
fn every_part_carries_both_tracks() {
    let mut seg = LlHlsSegmenter::with_part_target(
        vec![video_track(), audio_track()],
        timescale(1000),
        Duration::from_secs(1),
        Duration::from_millis(334),
    )
    .unwrap();

    // 1 s of 30 fps video; 2 audio frames per video frame (1024 @ 48 kHz).
    let mut ai: usize = 0;
    for i in 0..30u8 {
        seg.push(TrackId::new(1), vsample(i == 0, i)).unwrap();
        for _ in 0..2 {
            seg.push(
                TrackId::new(2),
                Sample::new(vec![ai as u8; 8], None, None, Some(1024), true).expect("valid timing"),
            )
            .unwrap();
            ai += 1;
        }
    }
    seg.flush().unwrap();
    let parts = seg.take_ready_parts();
    assert!(
        parts.len() >= 2,
        "expect several parts, got {}",
        parts.len()
    );

    for (i, p) in parts.iter().enumerate() {
        assert_eq!(
            traf_count(&p.bytes),
            2,
            "part {i} (dur {:?}) must carry both video (track 1) and audio (track 2)",
            p.duration
        );
        assert!(part_tfdt_for(&p.bytes, 1).is_some(), "part {i}: video tfdt");
        assert!(part_tfdt_for(&p.bytes, 2).is_some(), "part {i}: audio tfdt");
    }

    // Audio decode times must be strictly advancing and contiguous across the
    // parts — the audio is split, not duplicated.
    let mut prev: Option<u64> = None;
    for p in &parts {
        let t = part_tfdt_for(&p.bytes, 2).expect("audio tfdt");
        if let Some(prev) = prev {
            assert!(
                t > prev,
                "audio tfdt must advance across parts: {prev} -> {t}"
            );
        }
        prev = Some(t);
    }
}

/// A part's declared duration must not exceed the part target (RFC 8216bis
/// §4.4.4.9: "The duration of a Partial Segment MUST be less than or equal to
/// the Part Target Duration").
#[test]
fn part_duration_never_exceeds_part_target() {
    // 334 ms part target, 33.3 ms AUs: the pre-fix code emitted 367 ms parts.
    let mut seg = LlHlsSegmenter::with_part_target(
        vec![video_track()],
        timescale(1000),
        Duration::from_secs(1),
        Duration::from_millis(334),
    )
    .unwrap();
    for i in 0..30u8 {
        seg.push(TrackId::new(1), vsample(i == 0, i)).unwrap();
    }
    seg.flush().unwrap();
    let parts = seg.take_ready_parts();
    assert!(parts.len() >= 2, "expect several parts");
    // The *final* part of a segment is exempt from the 85% floor but not from
    // the ceiling; the last part here is the segment tail, checked too.
    for (i, p) in parts.iter().enumerate() {
        assert!(
            p.duration <= Duration::from_millis(334),
            "part {i} duration {:?} exceeds the 334 ms part target",
            p.duration
        );
    }
}

// ===========================================================================
// Item 3 — a part never exceeds PART-TARGET over a long (6 s) A/V segment
// ===========================================================================

/// A 6 s muxed segment cut at a 334 ms part target must produce parts no longer
/// than the target, every one carrying both tracks.
///
/// The old `emit_part` reset `anchor_part_dur` to 0 after a part even when
/// `take_span` had deliberately left the crossing sample (and its elapsed span)
/// in `pending`, so the leftover grew by roughly one sample per part; by the
/// end of a long segment the final "part" held all of it and ran far past the
/// part target (RFC 8216bis §4.4.4.9 bounds a part duration above by
/// PART-TARGET). A 1 s segment hides this, which is what the earlier test used.
#[test]
fn long_segment_parts_stay_within_part_target() {
    let mut seg = LlHlsSegmenter::with_part_target(
        vec![video_track(), audio_track()],
        timescale(6000),
        Duration::from_secs(6),
        Duration::from_millis(334),
    )
    .unwrap();

    const N_VID: usize = 155; // ends off a part boundary, so the tail holds anchor time
    const N_AUD: usize = 281; // ~6 s of 1024-sample frames
    const AUD_DUR: u32 = 1024;

    let mut items: Vec<(f64, bool, usize)> = Vec::new();
    for i in 0..N_VID {
        items.push((i as f64 * VID_DUR as f64 / 90_000.0, true, i));
    }
    for j in 0..N_AUD {
        items.push((j as f64 * AUD_DUR as f64 / 48_000.0, false, j));
    }
    items.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap());

    for (_, is_video, i) in items {
        if is_video {
            // Keyframe only at the segment start: one 6 s segment.
            seg.push(TrackId::new(1), vsample(i == 0, (i % 251) as u8))
                .unwrap();
        } else {
            seg.push(
                TrackId::new(2),
                Sample::new(vec![(i % 251) as u8; 8], None, None, Some(AUD_DUR), true)
                    .expect("valid timing"),
            )
            .unwrap();
        }
    }
    seg.flush().unwrap();

    let parts = seg.take_ready_parts();
    assert!(
        parts.len() > 10,
        "a 6 s segment at a 334 ms part target needs many parts, got {}",
        parts.len()
    );

    // Every part is within the target (the last part of a segment may be
    // shorter, never longer), and carries both tracks.
    for (i, p) in parts.iter().enumerate() {
        assert!(
            p.duration <= Duration::from_millis(334),
            "part {i} duration {:?} exceeds the 334 ms part target",
            p.duration
        );
        assert_eq!(
            traf_count(&p.bytes),
            2,
            "part {i} (duration {:?}) must carry both video and audio",
            p.duration
        );
    }

    // Sample conservation: the parts' video samples equal what was pushed.
    let video_samples: usize = parts.iter().map(|p| part_sample_count(&p.bytes, 1)).sum();
    assert_eq!(
        video_samples, N_VID,
        "the parts must carry every video sample exactly once"
    );
    let audio_samples: usize = parts.iter().map(|p| part_sample_count(&p.bytes, 2)).sum();
    assert_eq!(
        audio_samples, N_AUD,
        "the parts must carry every audio sample exactly once"
    );
}

/// A segment that has passed its target duration but is still waiting for the
/// next keyframe must keep cutting parts: the tail is not allowed to pile up
/// into one final part longer than PART-TARGET (RFC 8216bis §4.4.4.9; round-9
/// F7 measured 0.48-1.6 s final parts against a 0.2 s target).
///
/// Revert-check: restoring the `anchor_seg_dur < target_ticks` guard on the
/// part boundary in `push` makes the final part of each segment span the whole
/// post-target tail and this fails on the part-duration ceiling.
#[test]
fn a_segment_waiting_for_its_keyframe_keeps_cutting_parts() {
    const GOP_FRAMES: usize = 62;
    const GOPS: usize = 3;
    const PART_TARGET_MS: u32 = 200;
    let mut seg = LlHlsSegmenter::with_part_target(
        vec![video_track()],
        timescale(90_000),
        Duration::from_secs(1),
        Duration::from_millis(u64::from(PART_TARGET_MS)),
    )
    .unwrap();

    for i in 0..GOP_FRAMES * GOPS {
        seg.push(
            TrackId::new(1),
            vsample(i % GOP_FRAMES == 0, (i % 251) as u8),
        )
        .unwrap();
    }
    seg.flush().unwrap();

    let parts = seg.take_ready_parts();
    assert!(!parts.is_empty());
    let ceiling = Duration::from_millis(u64::from(PART_TARGET_MS));
    for (i, p) in parts.iter().enumerate() {
        assert!(
            p.duration <= ceiling,
            "part {i} duration {:?} exceeds the {PART_TARGET_MS} ms part target",
            p.duration
        );
    }
    let video_samples: usize = parts.iter().map(|p| part_sample_count(&p.bytes, 1)).sum();
    assert_eq!(video_samples, GOP_FRAMES * GOPS);
}

/// The sample count of `track_id`'s `trun`s in a part (`moof` walk).
fn part_sample_count(part: &[u8], want_track: u32) -> usize {
    let moof = find_box_body(part, b"moof").expect("moof in part");
    let mut total = 0usize;
    let mut off = 0usize;
    while off + 8 <= moof.len() {
        let size =
            u32::from_be_bytes([moof[off], moof[off + 1], moof[off + 2], moof[off + 3]]) as usize;
        if size < 8 || off + size > moof.len() {
            break;
        }
        if &moof[off + 4..off + 8] == b"traf" {
            let body = &moof[off + 8..off + size];
            let mut p = 0usize;
            let mut tid = None;
            while p + 8 <= body.len() {
                let bs =
                    u32::from_be_bytes([body[p], body[p + 1], body[p + 2], body[p + 3]]) as usize;
                let bt = &body[p + 4..p + 8];
                if bs < 8 || p + bs > body.len() {
                    break;
                }
                if bt == b"tfhd" {
                    tid = Some(u32::from_be_bytes([
                        body[p + 12],
                        body[p + 13],
                        body[p + 14],
                        body[p + 15],
                    ]));
                } else if bt == b"trun" && tid == Some(want_track) {
                    total += u32::from_be_bytes([
                        body[p + 12],
                        body[p + 13],
                        body[p + 14],
                        body[p + 15],
                    ]) as usize;
                }
                p += bs;
            }
        }
        off += size;
    }
    total
}

/// Issue #1597. RFC 8216bis §4.4.4.9: `EXT-X-PART` `DURATION` is the duration
/// of the Partial Segment, so a part never has zero duration, and the parts of
/// a Parent Segment add up to its duration. When the audio lags the video, the
/// part that drains the last video sample before the keyframe leaves audio
/// still to flush at the cut; that audio must ride in the final part together
/// with the video it belongs to, not in a part of its own with no video time.
#[test]
fn lagging_audio_never_makes_a_zero_duration_part() {
    const LAG_FRAMES: usize = 4;
    const FRAMES: usize = 90;
    const GOP: usize = 30;
    let mut seg = LlHlsSegmenter::with_part_target(
        vec![video_track(), audio_track()],
        timescale(1000),
        Duration::from_secs(1),
        Duration::from_millis(200),
    )
    .unwrap();
    let audio = |n: usize| {
        Sample::new(vec![n as u8; 8], None, None, Some(1024), true).expect("valid timing")
    };
    let mut audio_pushed = 0usize;
    for i in 0..FRAMES {
        seg.push(TrackId::new(1), vsample(i % GOP == 0, i as u8))
            .unwrap();
        if i >= LAG_FRAMES {
            for _ in 0..2 {
                seg.push(TrackId::new(2), audio(audio_pushed)).unwrap();
                audio_pushed += 1;
            }
        }
    }
    seg.flush().unwrap();

    let mut parts: std::collections::BTreeMap<u32, Vec<Duration>> =
        std::collections::BTreeMap::new();
    for p in seg.take_ready_parts() {
        assert!(
            !p.duration.is_zero(),
            "part {}.{} has zero duration",
            p.segment_seq,
            p.part_index
        );
        assert!(
            p.duration <= Duration::from_millis(200),
            "part {}.{}",
            p.segment_seq,
            p.part_index
        );
        parts.entry(p.segment_seq).or_default().push(p.duration);
    }
    let segments = seg.take_ready_segments();
    assert_eq!(segments.len(), FRAMES / GOP);
    for s in segments {
        let sum: Duration = parts[&s.segment_seq].iter().sum();
        assert!(
            sum.abs_diff(s.duration) < Duration::from_micros(1),
            "segment {}: parts sum to {sum:?}, segment is {:?}",
            s.segment_seq,
            s.duration
        );
    }
}

// ===========================================================================
// Issue #1621 — parts on the video frame grid meet the 85% floor
// ===========================================================================

const CLOCK_90K: u64 = 90_000;

fn frame_ticks(rate_num: u64, rate_den: u64, count: usize) -> Vec<u32> {
    let boundary = |i: u64| (i * CLOCK_90K * rate_den + rate_num / 2) / rate_num;
    (0..count as u64)
        .map(|i| (boundary(i + 1) - boundary(i)) as u32)
        .collect()
}

fn cut_parts(
    rate_num: u64,
    rate_den: u64,
    durations: Option<Vec<u32>>,
    seconds: u64,
) -> (Duration, Vec<transmux::ll_hls::PartInfo>) {
    const PART_TARGET: Duration = Duration::from_millis(200);
    const SEGMENT_TARGET: Duration = Duration::from_secs(2);
    let frames = (seconds * rate_num / rate_den) as usize;
    let durations = durations.unwrap_or_else(|| frame_ticks(rate_num, rate_den, frames));
    let gop = (SEGMENT_TARGET.as_secs_f64() * rate_num as f64 / rate_den as f64).round() as usize;
    let mut seg = LlHlsSegmenter::with_part_target(
        vec![video_track()],
        timescale(1000),
        SEGMENT_TARGET,
        PART_TARGET,
    )
    .unwrap();
    for (i, d) in durations.iter().enumerate() {
        let sample =
            Sample::new(vec![0u8; 16], None, None, Some(*d), i % gop == 0).expect("valid timing");
        seg.push(TrackId::new(1), sample).unwrap();
    }
    seg.flush().unwrap();
    (seg.part_target(), seg.take_ready_parts())
}

fn non_terminal(parts: &[transmux::ll_hls::PartInfo]) -> Vec<&transmux::ll_hls::PartInfo> {
    parts
        .iter()
        .enumerate()
        .filter(|(i, p)| {
            parts
                .get(i + 1)
                .is_some_and(|next| next.segment_seq == p.segment_seq)
        })
        .map(|(_, p)| p)
        .collect()
}

const FRAME_RATES: [(u64, u64, u32); 6] = [
    (24_000, 1001, 209),
    (25, 1, 200),
    (30_000, 1001, 201),
    (30, 1, 200),
    (50, 1, 200),
    (60_000, 1001, 200),
];

#[test]
fn non_terminal_parts_are_within_85_to_100_percent_of_part_target() {
    for (num, den, expected_ms) in FRAME_RATES {
        let (target_ms, parts) = cut_parts(num, den, None, 12);
        let target = target_ms.as_secs_f64();
        assert_eq!(
            target_ms,
            Duration::from_millis(u64::from(expected_ms)),
            "{num}/{den} part target"
        );
        let inner = non_terminal(&parts);
        assert!(inner.len() > 20, "{num}/{den}: too few parts");
        for p in &parts {
            assert!(
                p.duration <= target_ms,
                "{num}/{den}: part {} of segment {} lasts {:?} > target {target}",
                p.part_index,
                p.segment_seq,
                p.duration
            );
        }
        for p in inner {
            assert!(
                p.duration.as_secs_f64() >= 0.85 * target - 1e-9,
                "{num}/{den}: non-terminal part {} of segment {} lasts {:?} < 85% of {target}",
                p.part_index,
                p.segment_seq,
                p.duration
            );
        }
    }
}

#[test]
fn variable_frame_duration_keeps_the_configured_part_target() {
    let durations: Vec<u32> = (0..300)
        .map(|i| if i % 2 == 0 { 2000 } else { 4000 })
        .collect();
    let (target_ms, parts) = cut_parts(30, 1, Some(durations), 10);
    assert_eq!(target_ms, Duration::from_millis(200));
    assert!(
        parts
            .iter()
            .all(|p| p.duration <= Duration::from_millis(200))
    );
    assert!(parts.len() <= 100, "{} parts for 10 s", parts.len());
}

#[test]
fn samples_without_durations_keep_the_configured_part_target() {
    let mut seg = LlHlsSegmenter::with_part_target(
        vec![video_track()],
        timescale(1000),
        Duration::from_secs(2),
        Duration::from_millis(200),
    )
    .unwrap();
    for i in 0..40u8 {
        let sample = Sample::new(vec![0u8; 16], None, None, None, i == 0).expect("valid timing");
        seg.push(TrackId::new(1), sample).unwrap();
    }
    assert_eq!(seg.part_target(), Duration::from_millis(200));
}

#[test]
fn jittered_frame_timestamps_still_land_parts_within_the_band() {
    let durations: Vec<u32> = (0..288)
        .map(|i| if matches!(i % 4, 0 | 3) { 3690 } else { 3780 })
        .collect();
    let (target_ms, parts) = cut_parts(24_000, 1001, Some(durations), 12);
    let target = target_ms.as_secs_f64();
    assert!(
        target_ms > Duration::from_millis(200),
        "the 200 ms target must be raised to whole frames"
    );
    let inner = non_terminal(&parts);
    assert!(inner.len() > 20);
    for p in &parts {
        assert!(p.duration <= target_ms, "{:?} > {target}", p.duration);
    }
    for p in inner {
        assert!(
            p.duration.as_secs_f64() >= 0.85 * target - 1e-9,
            "{:?} < 85% of {target}",
            p.duration
        );
    }
}

#[test]
fn a_part_is_cut_as_soon_as_its_last_frame_arrives() {
    let durations = frame_ticks(24_000, 1001, 120);
    let mut seg = LlHlsSegmenter::with_part_target(
        vec![video_track()],
        timescale(1000),
        Duration::from_secs(60),
        Duration::from_millis(200),
    )
    .unwrap();
    let mut cut_at = Vec::new();
    for (i, d) in durations.iter().enumerate() {
        let sample =
            Sample::new(vec![0u8; 16], None, None, Some(*d), i == 0).expect("valid timing");
        seg.push(TrackId::new(1), sample).unwrap();
        if !seg.take_ready_parts().is_empty() {
            cut_at.push(i);
        }
    }
    let gaps: Vec<usize> = cut_at[1..].windows(2).map(|w| w[1] - w[0]).collect();
    assert!(gaps.len() > 10);
    assert!(gaps.iter().all(|g| *g == 5), "{cut_at:?}");
    assert!(
        cut_at[2..].iter().all(|i| i % 5 == 4),
        "a part must be cut on its fifth frame, not the sixth: {cut_at:?}"
    );
}

#[test]
fn start_time_moves_every_tfdt_and_nothing_else() {
    let tfdts = |start_ns: Option<u64>| {
        let mut seg = LlHlsSegmenter::with_part_target(
            vec![video_track()],
            timescale(1000),
            Duration::from_secs(1),
            Duration::from_millis(334),
        )
        .unwrap();
        if let Some(ns) = start_ns {
            seg.set_start_time(std::time::Duration::from_nanos(ns));
        }
        for i in 0..31u8 {
            seg.push(TrackId::new(1), vsample(i % 30 == 0, i)).unwrap();
        }
        seg.take_ready_parts()
            .iter()
            .filter_map(|p| part_tfdt_for(&p.bytes, 1))
            .collect::<Vec<_>>()
    };
    let from_zero = tfdts(None);
    let from_five_seconds = tfdts(Some(5_000_000_000));
    assert!(!from_zero.is_empty());
    assert_eq!(from_zero[0], 0);
    assert_eq!(from_zero.len(), from_five_seconds.len());
    for (a, b) in from_zero.iter().zip(&from_five_seconds) {
        assert_eq!(*b, *a + 5 * 1000 * 90, "tfdt moved by the start time");
    }
}

// ===========================================================================
// Sequence numbering, deferral, follower guards and tfdt continuity
// ===========================================================================

fn mfhd_seq(bytes: &[u8]) -> u32 {
    let body = find_box_body(bytes, b"moof").expect("fragment has a moof");
    MovieFragmentBox::parse_body(body)
        .expect("moof parses")
        .mfhd
        .sequence_number
}

fn timed_video(dts: i64, duration: u32, sync: bool) -> Sample {
    Sample::new(vec![0u8; 16], Some(dts), Some(dts), Some(duration), sync).expect("valid timing")
}

fn video_600ms_segmenter() -> LlHlsSegmenter {
    LlHlsSegmenter::with_part_target(
        vec![video_track()],
        timescale(90_000),
        Duration::from_millis(600),
        Duration::from_millis(200),
    )
    .expect("LL-HLS segmenter")
}

fn two_three_part_plans() -> [transmux::ll_hls::CutPlan; 2] {
    let cut = |seq: u32, base: i64| {
        transmux::ll_hls::CutPlan::new(
            seq,
            base,
            90_000,
            vec![
                transmux::ll_hls::PartCut::new(broadcast_hls::PartIndex::new(0), base, None),
                transmux::ll_hls::PartCut::new(
                    broadcast_hls::PartIndex::new(1),
                    base + 18_000,
                    None,
                ),
                transmux::ll_hls::PartCut::new(
                    broadcast_hls::PartIndex::new(2),
                    base + 36_000,
                    None,
                ),
            ],
        )
        .expect("cut plan")
    };
    [cut(1, 0), cut(2, 54_000)]
}

#[test]
fn next_sequence_numbers_resume_strict_monotonic_numbering() {
    let mut seg = video_600ms_segmenter();
    assert_eq!(seg.next_sequence_numbers(), (1, 1));

    for i in 0..30u8 {
        seg.push(
            TrackId::new(1),
            timed_video(i as i64 * 3600, 3600, i % 15 == 0),
        )
        .unwrap();
    }
    seg.flush().unwrap();
    let parts = seg.take_ready_parts();
    let segments = seg.take_ready_segments();
    assert_eq!(parts.len(), 6, "two segments of three parts");
    assert_eq!(segments.len(), 2);

    let last_seq = parts
        .iter()
        .map(|p| mfhd_seq(&p.bytes))
        .chain(segments.iter().map(|s| mfhd_seq(&s.bytes)))
        .max()
        .expect("emitted sequence numbers");
    let (next_seq, current_segment) = seg.next_sequence_numbers();
    assert_eq!(
        next_seq,
        last_seq + 1,
        "next_seq continues past the last moof"
    );
    assert_eq!(
        current_segment,
        segments.last().unwrap().segment_seq + 1,
        "current_segment is the segment after the last emitted one"
    );
    assert_eq!((next_seq, current_segment), (9, 3));

    let mut resumed = LlHlsSegmenter::with_part_target_at(
        vec![video_track()],
        timescale(90_000),
        Duration::from_millis(600),
        Duration::from_millis(200),
        next_seq,
        current_segment,
    )
    .expect("resumed segmenter");
    for i in 0..8u8 {
        resumed
            .push(TrackId::new(1), timed_video(i as i64 * 3600, 3600, i == 0))
            .unwrap();
    }
    let resumed_parts = resumed.take_ready_parts();
    assert_eq!(resumed_parts.len(), 1);
    assert_eq!(
        mfhd_seq(&resumed_parts[0].bytes),
        next_seq,
        "the rebuilt segmenter's first part continues the sequence"
    );
}

#[test]
fn deferred_part_is_emitted_only_on_the_next_anchor_sample() {
    let mut seg = LlHlsSegmenter::with_part_target(
        vec![video_track(), audio_track()],
        timescale(90_000),
        Duration::from_secs(2),
        Duration::from_millis(200),
    )
    .expect("LL-HLS segmenter");
    let video = TrackId::new(1);
    let audio = TrackId::new(2);

    for i in 0..49u8 {
        seg.push(video, timed_video(i as i64 * 3600, 3600, i == 0))
            .unwrap();
    }
    let _ = seg.take_ready_parts();
    // The 50th frame reaches the 2 s segment target exactly and its part
    // boundary would drain every anchor sample, so the part is held back.
    seg.push(video, timed_video(49 * 3600, 3600, false))
        .unwrap();
    assert!(
        seg.take_ready_parts().is_empty(),
        "a part that would leave an empty anchor tail is deferred"
    );

    seg.push(
        audio,
        Sample::new(vec![7u8; 8], None, None, Some(1920), true).expect("valid timing"),
    )
    .unwrap();
    assert!(
        seg.take_ready_parts().is_empty(),
        "an audio sample must not release the deferred part"
    );

    seg.push(video, timed_video(50 * 3600, 3600, false))
        .unwrap();
    let parts = seg.take_ready_parts();
    assert_eq!(parts.len(), 1, "the next anchor sample releases the part");
    assert_eq!(
        part_sample_count(&parts[0].bytes, 1),
        5,
        "the deferred part carries the five video AUs before the boundary"
    );
}

fn already_cut_error(err: transmux::Error) -> bool {
    matches!(&err, transmux::Error::InvalidInput(msg) if msg.contains("already cut on its own"))
}

#[test]
fn follow_refuses_each_already_cut_condition() {
    let new = || {
        LlHlsSegmenter::with_part_target(
            vec![video_track()],
            timescale(90_000),
            Duration::from_secs(1),
            Duration::from_millis(200),
        )
        .expect("LL-HLS segmenter")
    };
    let plan = || transmux::ll_hls::CutPlan::new(1, 0, 90_000, Vec::new()).expect("cut plan");

    // (a) a finished segment is still queued in `ready`.
    let mut ready = new();
    for i in 0..26u8 {
        ready
            .push(
                TrackId::new(1),
                timed_video(i as i64 * 3600, 3600, i % 25 == 0),
            )
            .unwrap();
    }
    assert!(already_cut_error(ready.follow(plan()).unwrap_err()));

    // (b) a part was emitted and the queue drained: `part_start > 0`.
    let mut part = new();
    for i in 0..8u8 {
        part.push(TrackId::new(1), timed_video(i as i64 * 3600, 3600, i == 0))
            .unwrap();
    }
    let _ = part.take_ready_parts();
    assert!(already_cut_error(part.follow(plan()).unwrap_err()));

    // (c) a segment closed and was drained: `seg_base_decode > 0`.
    let mut closed = new();
    for i in 0..26u8 {
        closed
            .push(
                TrackId::new(1),
                timed_video(i as i64 * 3600, 3600, i % 25 == 0),
            )
            .unwrap();
    }
    let _ = closed.take_ready_parts();
    let _ = closed.take_ready_segments();
    assert!(already_cut_error(closed.follow(plan()).unwrap_err()));

    // Control: a fresh segmenter accepts the plan.
    assert!(new().follow(plan()).is_ok());
}

#[test]
fn parts_and_segments_never_carry_a_traf_for_a_sample_less_track() {
    let mut seg = LlHlsSegmenter::with_part_target(
        vec![video_track(), audio_track()],
        timescale(90_000),
        Duration::from_secs(10),
        Duration::from_millis(200),
    )
    .expect("LL-HLS segmenter");
    for i in 0..8u8 {
        seg.push(TrackId::new(1), timed_video(i as i64 * 3600, 3600, i == 0))
            .unwrap();
    }
    let parts = seg.take_ready_parts();
    assert!(!parts.is_empty());
    assert_eq!(
        traf_count(&parts[0].bytes),
        1,
        "an audio track with no sample before the part boundary must not get an empty traf"
    );
    seg.flush().unwrap();
    let segments = seg.take_ready_segments();
    assert_eq!(
        traf_count(&segments[0].bytes),
        1,
        "an audio track with no sample in the segment must not get an empty traf"
    );
}

#[test]
fn mfhd_sequence_numbers_increase_by_one_along_every_path() {
    let expected_parts = [1u32, 2, 3, 5, 6, 7];
    let expected_segments = [4u32, 8];

    // Leader via push/flush.
    let mut leader = video_600ms_segmenter();
    let frames: Vec<Sample> = (0..30)
        .map(|i| timed_video(i * 3600, 3600, i % 15 == 0))
        .collect();
    for s in &frames {
        leader.push(TrackId::new(1), s.clone()).unwrap();
    }
    leader.flush().unwrap();
    let leader_parts = leader.take_ready_parts();
    let leader_segments = leader.take_ready_segments();
    assert_eq!(
        leader_parts
            .iter()
            .map(|p| mfhd_seq(&p.bytes))
            .collect::<Vec<_>>(),
        expected_parts
    );
    assert_eq!(
        leader_segments
            .iter()
            .map(|s| mfhd_seq(&s.bytes))
            .collect::<Vec<_>>(),
        expected_segments
    );

    // Follower via whole-segment plans.
    let plans = two_three_part_plans();
    let mut follow = video_600ms_segmenter();
    follow.follow(plans[0].clone()).unwrap();
    for s in &frames[..15] {
        follow.push(TrackId::new(1), s.clone()).unwrap();
    }
    follow.follow(plans[1].clone()).unwrap();
    for s in &frames[15..] {
        follow.push(TrackId::new(1), s.clone()).unwrap();
    }
    follow.flush().unwrap();
    assert_eq!(
        follow
            .take_ready_parts()
            .iter()
            .map(|p| mfhd_seq(&p.bytes))
            .collect::<Vec<_>>(),
        expected_parts
    );
    assert_eq!(
        follow
            .take_ready_segments()
            .iter()
            .map(|s| mfhd_seq(&s.bytes))
            .collect::<Vec<_>>(),
        expected_segments
    );

    // Follower via incremental part cuts.
    let mut increment = video_600ms_segmenter();
    let cuts = [
        (1u64, 0usize, 0i64),
        (1, 1, 18_000),
        (1, 2, 36_000),
        (2, 0, 54_000),
        (2, 1, 72_000),
        (2, 2, 90_000),
    ];
    let mut next = 0usize;
    for (msn, index, start) in cuts {
        while next < frames.len() && frames[next].timing.and_then(|t| t.dts()).unwrap() < start {
            increment
                .push(TrackId::new(1), frames[next].clone())
                .unwrap();
            next += 1;
        }
        increment
            .follow_part(
                broadcast_hls::MediaSequence::new(msn),
                broadcast_hls::PartIndex::new(index as u32),
                start,
                timescale(90_000),
            )
            .unwrap();
    }
    for s in &frames[next..] {
        increment.push(TrackId::new(1), s.clone()).unwrap();
    }
    increment.flush().unwrap();
    assert_eq!(
        increment
            .take_ready_parts()
            .iter()
            .map(|p| mfhd_seq(&p.bytes))
            .collect::<Vec<_>>(),
        expected_parts
    );
    assert_eq!(
        increment
            .take_ready_segments()
            .iter()
            .map(|s| mfhd_seq(&s.bytes))
            .collect::<Vec<_>>(),
        expected_segments
    );
}

#[test]
fn leader_tfdt_continues_across_segments_and_parts() {
    let mut seg = LlHlsSegmenter::with_part_target(
        vec![video_track()],
        timescale(90_000),
        Duration::from_secs(1),
        Duration::from_millis(200),
    )
    .expect("LL-HLS segmenter");
    for i in 0..100i64 {
        seg.push(TrackId::new(1), timed_video(i * 3600, 3600, i % 25 == 0))
            .unwrap();
    }
    seg.flush().unwrap();
    let parts = seg.take_ready_parts();
    let segments = seg.take_ready_segments();
    assert_eq!(segments.len(), 4);

    for (k, segment) in segments.iter().enumerate() {
        let segment_start = k as u64 * 90_000;
        assert_eq!(
            part_tfdt_for(&segment.bytes, 1),
            Some(segment_start),
            "segment {k} must open at its own timeline position"
        );
        let seg_parts: Vec<_> = parts
            .iter()
            .filter(|p| p.segment_seq == segment.segment_seq)
            .collect();
        assert_eq!(seg_parts.len(), 5, "segment {k} has five parts");
        for (j, part) in seg_parts.iter().enumerate() {
            assert_eq!(
                part_tfdt_for(&part.bytes, 1),
                Some(segment_start + j as u64 * 18_000),
                "segment {k} part {j} continues the part timeline"
            );
        }
    }
}

// ===========================================================================
// observe_frame: 85% band, trigger retuning and the retune give-up
// ===========================================================================

fn jittered_segmenter(part_target: Duration) -> LlHlsSegmenter {
    LlHlsSegmenter::with_part_target(
        vec![video_track()],
        timescale(90_000),
        Duration::from_secs(60),
        part_target,
    )
    .expect("LL-HLS segmenter")
}

fn push_durations(seg: &mut LlHlsSegmenter, durations: &[u32]) {
    for (i, d) in durations.iter().enumerate() {
        let sample =
            Sample::new(vec![0u8; 16], None, None, Some(*d), i == 0).expect("valid timing");
        seg.push(TrackId::new(1), sample).unwrap();
    }
}

#[test]
fn a_grid_below_the_85_percent_band_keeps_the_configured_target() {
    let durations: Vec<u32> = (0..40)
        .map(|i| if i % 2 == 0 { 3000 } else { 4000 })
        .collect();
    let mut seg = jittered_segmenter(Duration::from_millis(200));
    push_durations(&mut seg, &durations);
    assert_eq!(seg.part_target(), Duration::from_millis(200));
    for part in seg.take_ready_parts() {
        assert!(part.duration <= Duration::from_millis(200));
    }
}

#[test]
fn the_85_percent_band_edge_accepts_the_grid() {
    let durations: Vec<u32> = (0..40)
        .map(|i| if i % 2 == 0 { 1800 } else { 1900 })
        .collect();
    let mut seg = jittered_segmenter(Duration::from_millis(120));
    push_durations(&mut seg, &durations);
    assert_eq!(
        seg.part_target(),
        Duration::from_millis(134),
        "lo*100 == hi*85 is inside the band, so the target is retuned to whole frames"
    );
}

#[test]
fn a_grid_just_below_the_85_percent_band_keeps_the_configured_target() {
    let durations: Vec<u32> = (0..40)
        .map(|i| if i % 2 == 0 { 1800 } else { 1901 })
        .collect();
    let mut seg = jittered_segmenter(Duration::from_millis(120));
    push_durations(&mut seg, &durations);
    assert_eq!(seg.part_target(), Duration::from_millis(120));
}

#[test]
fn sixty_fps_parts_are_cut_on_their_eleventh_frame() {
    let durations = frame_ticks(60_000, 1001, 240);
    let mut seg = jittered_segmenter(Duration::from_millis(200));
    let mut cut_at = Vec::new();
    for (i, d) in durations.iter().enumerate() {
        let sample =
            Sample::new(vec![0u8; 16], None, None, Some(*d), i == 0).expect("valid timing");
        seg.push(TrackId::new(1), sample).unwrap();
        if !seg.take_ready_parts().is_empty() {
            cut_at.push(i);
        }
    }
    assert_eq!(seg.part_target(), Duration::from_millis(200));
    assert!(cut_at.len() > 10, "expected many parts, got {cut_at:?}");
    assert!(
        cut_at.iter().all(|i| i % 11 == 10),
        "59.94 fps parts must be cut on their eleventh frame: {cut_at:?}"
    );
}

#[test]
fn a_wide_band_grid_does_not_lower_the_trigger() {
    let durations: Vec<u32> = (0..80)
        .map(|i| if i % 2 == 0 { 20_000 } else { 19_000 })
        .collect();
    let mut seg = jittered_segmenter(Duration::from_millis(1640));
    let mut cut_at = Vec::new();
    for (i, d) in durations.iter().enumerate() {
        let sample =
            Sample::new(vec![0u8; 16], None, None, Some(*d), i == 0).expect("valid timing");
        seg.push(TrackId::new(1), sample).unwrap();
        if !seg.take_ready_parts().is_empty() {
            cut_at.push(i);
        }
    }
    assert_eq!(seg.part_target(), Duration::from_millis(1640));
    assert_eq!(
        &cut_at[..2],
        &[7, 14],
        "the worst-case floor does not fit the band, so the trigger stays at the target"
    );
}

#[test]
fn the_retune_gives_up_when_the_grid_is_too_loose() {
    let durations: Vec<u32> = (0..40)
        .map(|i| if i % 2 == 0 { 1800 } else { 1900 })
        .collect();
    let mut seg = jittered_segmenter(Duration::from_millis(144));
    push_durations(&mut seg, &durations);
    assert_eq!(
        seg.part_target(),
        Duration::from_millis(144),
        "a grid where the floor's worst case already spans frames*lo must not be retuned"
    );
}

#[test]
fn a_part_that_leaves_a_remainder_is_emitted_at_once() {
    let durations: Vec<u32> = (0..8)
        .map(|i| if i % 2 == 0 { 3000 } else { 4000 })
        .collect();
    let mut seg = LlHlsSegmenter::with_part_target(
        vec![video_track()],
        timescale(90_000),
        Duration::from_millis(100),
        Duration::from_millis(200),
    )
    .expect("LL-HLS segmenter");
    push_durations(&mut seg, &durations);
    assert!(
        !seg.take_ready_parts().is_empty(),
        "the part leaves one sample for the segment tail, so it is not deferred"
    );
}

#[test]
fn a_first_sample_longer_than_the_part_target_is_its_own_part() {
    let mut seg = LlHlsSegmenter::with_part_target(
        vec![video_track()],
        timescale(90_000),
        Duration::from_secs(60),
        Duration::from_millis(200),
    )
    .expect("LL-HLS segmenter");
    let durations = [2250u32, 45_000, 2250, 2250, 2250, 2250, 2250, 2250];
    push_durations(&mut seg, &durations);
    let _ = seg.take_ready_parts();
    push_durations(&mut seg, &[2250]);
    let parts = seg.take_ready_parts();
    assert_eq!(
        parts.len(),
        1,
        "the 500 ms sample is carried as its own part, not dropped"
    );
    assert_eq!(
        parts[0].duration,
        Duration::from_millis(500),
        "the part is the long sample's own span"
    );
}

#[test]
fn a_probe_that_gives_up_cuts_parts_without_waiting_for_eight_frames() {
    let mut seg = jittered_segmenter(Duration::from_millis(200));
    let mut dts = 0i64;
    let mut first_cut = None;
    // Sixteen samples carry only three positive dts deltas: the probe gives up.
    for i in 0..16u8 {
        if i >= 13 {
            dts += 3600;
        }
        seg.push(
            TrackId::new(1),
            Sample::new(vec![0u8; 16], Some(dts), None, None, false).expect("valid timing"),
        )
        .unwrap();
        if !seg.take_ready_parts().is_empty() {
            first_cut = Some(i);
        }
    }
    assert!(
        first_cut.is_none(),
        "no part cuts while the probe is measuring"
    );
    let mut regular = 16u8;
    loop {
        dts += 3600;
        seg.push(
            TrackId::new(1),
            Sample::new(vec![0u8; 16], Some(dts), None, None, false).expect("valid timing"),
        )
        .unwrap();
        if !seg.take_ready_parts().is_empty() {
            first_cut = Some(regular);
            break;
        }
        regular += 1;
        assert!(regular < 40, "the probe never gave up");
    }
    assert_eq!(
        first_cut,
        Some(17),
        "after give-up the part is cut when the accumulated span reaches 200 ms"
    );
}

#[test]
fn dts_only_samples_still_retune_the_part_target() {
    let durations = frame_ticks(24_000, 1001, 120);
    let mut seg = jittered_segmenter(Duration::from_millis(200));
    let mut dts = 0i64;
    for &d in &durations {
        seg.push(
            TrackId::new(1),
            Sample::new(vec![0u8; 16], Some(dts), None, None, false).expect("valid timing"),
        )
        .unwrap();
        dts += i64::from(d);
    }
    assert_eq!(
        seg.part_target(),
        Duration::from_millis(209),
        "a zero elapsed first sample must not be counted as the shortest frame"
    );
}

#[test]
fn cut_plan_starts_come_from_accumulated_durations_without_a_dts() {
    let mut seg = LlHlsSegmenter::with_part_target(
        vec![video_track()],
        timescale(90_000),
        Duration::from_secs(1),
        Duration::from_millis(200),
    )
    .expect("LL-HLS segmenter");
    seg.record_cuts();
    for i in 0..100u8 {
        let sample =
            Sample::new(vec![0u8; 16], None, None, Some(3600), i % 25 == 0).expect("valid timing");
        seg.push(TrackId::new(1), sample).unwrap();
    }
    seg.flush().unwrap();
    let plans = seg.take_ready_cut_plans();
    assert_eq!(plans.len(), 4);
    assert_eq!(
        plans.iter().map(|p| p.segment_start).collect::<Vec<_>>(),
        vec![0, 90_000, 180_000, 270_000]
    );
}

// ── Issue #4154: a break closes the open segment (and parts) and marks next ─

fn ll_stage(two: &crate::discontinuity::TwoSource) -> LlHlsSegmenter {
    LlHlsSegmenter::with_part_target(
        two.media.tracks.iter().map(|t| t.spec.clone()).collect(),
        timescale(1000),
        Duration::from_secs(2),
        Duration::from_millis(500),
    )
    .expect("LL-HLS segmenter")
}

fn split_outputs(
    outputs: Vec<transmux::ll_hls::LlHlsStageOutput>,
) -> (
    Vec<transmux::ll_hls::PartInfo>,
    Vec<transmux::ll_hls::SegmentInfo>,
) {
    let mut parts = Vec::new();
    let mut segments = Vec::new();
    for output in outputs {
        match output {
            transmux::ll_hls::LlHlsStageOutput::Part(part) => parts.push(part),
            transmux::ll_hls::LlHlsStageOutput::Segment(segment) => segments.push(segment),
            _ => {}
        }
    }
    (parts, segments)
}

/// The trailing segment of the first source is closed at the break (so the
/// jump is never inside one segment's part timeline), and only the segment
/// that follows is flagged — which `broadcast-hls` renders as
/// `#EXT-X-DISCONTINUITY` and parses back.
#[test]
fn a_forward_break_closes_the_open_segment_and_marks_the_next() {
    use broadcast_common::stage::Stage;
    use broadcast_hls::{MediaPlaylist, MediaSegment};
    use transmux::mux_stage::MuxInput;

    let two = crate::discontinuity::forward();
    let mut stage = ll_stage(&two);
    let outputs = crate::discontinuity::drive_fixed(&mut stage, &two, None).expect("LL-HLS drive");
    let (parts, segments) = split_outputs(outputs);
    assert!(
        segments.len() >= 2,
        "the break must not be inside one segment"
    );
    let marked: Vec<_> = segments.iter().filter(|s| s.discontinuous).collect();
    assert_eq!(
        marked.len(),
        1,
        "exactly one segment opens the new timeline"
    );
    let marked_seq = marked[0].segment_seq;
    assert!(
        marked_seq > segments[0].segment_seq,
        "the break's segment is not the first one"
    );
    for part in &parts {
        assert!(
            part.segment_seq < marked_seq || part.segment_seq >= marked_seq,
            "parts keep their owning segment"
        );
    }

    let media: Vec<MediaSegment> = segments
        .iter()
        .map(|segment| MediaSegment {
            uri: format!("s{}.m4s", segment.segment_seq),
            duration: DecimalSeconds::from_duration(segment.duration),
            discontinuous: segment.discontinuous,
            ..Default::default()
        })
        .collect();
    let playlist = MediaPlaylist {
        version: 9,
        target_duration: 2,
        msn: broadcast_hls::MediaSequence::new(u64::from(segments[0].segment_seq)),
        discontinuity_sequence: 0,
        segments: media,
        endlist: true,
        ..Default::default()
    };
    let text = playlist.to_m3u8().expect("playlist renders");
    assert_eq!(
        text.matches("#EXT-X-DISCONTINUITY\n").count(),
        1,
        "exactly one tag:\n{text}"
    );
    let parsed = MediaPlaylist::parse(&text).expect("broadcast-hls parses its own playlist");
    let flagged: Vec<_> = parsed
        .segments
        .iter()
        .filter(|segment| segment.discontinuous)
        .collect();
    assert_eq!(flagged.len(), 1, "the mark survives a parse");

    let mut first_break = stage;
    first_break
        .feed(
            MuxInput::Discontinuity { track: None },
            broadcast_common::Timestamp::ZERO,
        )
        .expect("a break before the first sample is expressible");
    let outputs = crate::discontinuity::drive_media_fixed(&mut first_break, &two.media)
        .expect("drive with a break at the very start");
    let (_, segments) = split_outputs(outputs);
    assert!(
        segments[0].discontinuous,
        "a break as the first input marks the first segment"
    );
    assert!(
        segments[1..].iter().all(|segment| !segment.discontinuous),
        "and only the first"
    );
}
