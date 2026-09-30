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

use broadcast_hls::{DecimalSeconds, LowLatencyConfig, MediaPlaylist, MediaSegment, PartSpec};
use transmux::ll_hls::LlHlsSegmenter;
use transmux::{
    AVCConfigurationBox, AVCDecoderConfigurationRecord, CodecConfig, DecoderConfigDescriptor,
    DecoderSpecificInfo, ESDescriptor, EsdsBox, FragmentTrackData, MovieFragmentBox,
    SLConfigDescriptor, Sample, TrackSpec,
};

// ---------------------------------------------------------------------------
// Track specs (minimal but structurally-real configs).
// ---------------------------------------------------------------------------

fn dummy_avc_config() -> AVCConfigurationBox {
    AVCConfigurationBox::new(AVCDecoderConfigurationRecord {
        configuration_version: 1,
        profile_indication: 66,
        profile_compatibility: 0,
        level_indication: 30,
        length_size_minus_one: 3,
        sps: vec![transmux::AvcSps(vec![0x67, 66, 0, 30, 0x00])],
        pps: vec![transmux::AvcPps(vec![0x68, 0xCE, 0x3C, 0x80])],
        chroma_format: None,
        bit_depth_luma_minus8: None,
        bit_depth_chroma_minus8: None,
        sps_ext: vec![],
    })
}

fn dummy_esds() -> EsdsBox {
    EsdsBox::new(ESDescriptor::new(
        1,
        0,
        Some(DecoderConfigDescriptor::new(
            0x40,
            0x05,
            false,
            0,
            0,
            0,
            Some(DecoderSpecificInfo::new(vec![0x12, 0x10])),
        )),
        Some(SLConfigDescriptor::predefined_two()),
    ))
}

fn video_track() -> TrackSpec {
    TrackSpec::new(
        1,
        90_000,
        CodecConfig::Avc {
            config: dummy_avc_config(),
            width: 320,
            height: 240,
        },
    )
}

fn audio_track() -> TrackSpec {
    TrackSpec::new(
        2,
        48_000,
        CodecConfig::Aac {
            esds: dummy_esds(),
            channel_count: 2,
            sample_rate: 48_000,
            sample_size: 16,
        },
    )
}

const VID_DUR: u32 = 3000; // 90 kHz, 1/30 s per AU

fn vsample(is_sync: bool, byte: u8) -> Sample {
    // Synthetic sample for the duration-driven segmenter: dts/pts stay `None`
    // (media plane step 2c — never fabricate a timestamp); the timeline comes
    // from `duration`, exactly as before.
    Sample::new(vec![byte; 32], None, None, Some(VID_DUR), is_sync)
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
    let mut seg = LlHlsSegmenter::with_part_target(vec![video_track()], 1000, 1.0, 334).unwrap();

    // ~1 s of 30 fps video: 30 AUs @ 3000 ticks. First AU is a keyframe.
    for i in 0..30u8 {
        seg.push(1, vsample(i == 0, i)).unwrap();
    }
    // Next keyframe past the target closes the segment; then flush the tail.
    seg.push(1, vsample(true, 200)).unwrap();
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
    let parts_sum: f64 = seg1_parts.iter().map(|p| p.duration).sum();
    assert!(
        (parts_sum - seg1.duration).abs() < 1e-6,
        "parts sum {parts_sum} != segment duration {}",
        seg1.duration
    );
    // And the segment is a whole 30-AU segment = 1.0 s.
    assert!((seg1.duration - 1.0).abs() < 1e-6, "segment ~1 s");
    assert_eq!(seg1.part_count, 3, "segment records 3 parts");
}

// ===========================================================================
// Test 2 — INDEPENDENT flag bites (sync first sample => YES, mid-GOP => no)
// ===========================================================================

#[test]
fn independent_flag_tracks_sync_first_sample() {
    let mut seg = LlHlsSegmenter::with_part_target(vec![video_track()], 1000, 1.0, 334).unwrap();

    // 30 AUs: keyframe only at index 0. Part 1 starts at a sync sample; parts 2
    // and 3 start mid-GOP (no sync).
    for i in 0..30u8 {
        seg.push(1, vsample(i == 0, i)).unwrap();
    }
    seg.push(1, vsample(true, 200)).unwrap();
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
        .map(|(i, p)| PartSpec {
            uri: format!("seg1.{i}.m4s"),
            // The segmenter's own computed part duration (issue #1140):
            // always finite and non-negative.
            duration: DecimalSeconds::new(p.duration)
                .expect("segmenter part duration is finite, >= 0"),
            independent: p.independent,
            ..Default::default()
        })
        .collect();
    let pl = MediaPlaylist {
        version: 9,
        target_duration: 1,
        media_sequence: 0,
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
        low_latency: Some(LowLatencyConfig {
            part_target: Some(DecimalSeconds::new(0.334).unwrap()),
            part_hold_back: Some(DecimalSeconds::new(1.002).unwrap()),
            preload_hint_part: None,
            ..Default::default()
        }),
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

// ===========================================================================
// Test 3 — playlist directive text bites (opt-in)
// ===========================================================================

#[test]
fn playlist_low_latency_directives_present_and_opt_in() {
    let parts = vec![
        PartSpec {
            uri: "seg0.0.m4s".into(),
            duration: DecimalSeconds::new(0.334).unwrap(),
            independent: true,
            ..Default::default()
        },
        PartSpec {
            uri: "seg0.1.m4s".into(),
            duration: DecimalSeconds::new(0.334).unwrap(),
            independent: false,
            ..Default::default()
        },
    ];
    let ll = LowLatencyConfig {
        part_target: Some(DecimalSeconds::new(0.334).unwrap()),
        // Deliberately too small; renderer must raise to 3 x 0.334 = 1.002.
        part_hold_back: Some(DecimalSeconds::new(0.5).unwrap()),
        preload_hint_part: Some("seg0.2.m4s".into()),
        ..Default::default()
    };
    let pl = MediaPlaylist {
        version: 9,
        target_duration: 1,
        media_sequence: 0,
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
    let mut seg =
        LlHlsSegmenter::with_part_target(vec![video_track(), audio_track()], 1000, 1.0, 334)
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
            seg.push(1, s).unwrap();
        } else {
            let s = Sample::from_raw(
                vec![(*idx as u8).wrapping_add(1); 20],
                None,
                None,
                Some(AUD_DUR),
            );
            aud_samples.push(s.clone());
            seg.push(2, s).unwrap();
        }
    }
    // Close the (single) segment and flush.
    seg.push(1, vsample(true, 250)).unwrap();
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
                    if tid == 1 {
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
            FragmentTrackData::new(1, 0, &vid_samples),
            FragmentTrackData::new(2, 0, &aud_samples),
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
                if tid == 1 {
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
                if tid == 1 {
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

/// `timescale` is a wire `u32` (`mdhd.timescale`, ISO/IEC 14496-12:2015
/// §8.4.2) that `LlHlsSegmenter::with_part_target` does not itself validate —
/// only `target_duration_secs`/`part_target_ms` are checked at construction.
/// `part_target_secs`/the part- and segment-duration computations divide by
/// the anchor track's timescale directly; before the fix, a zero timescale
/// turned every one of those into `f64::INFINITY`/`NaN`, rendered verbatim
/// into `#EXT-X-PART-INF`/`#EXT-X-PART`/`#EXTINF` text (Rust's `Display` for
/// `f64` renders those as the literal substrings `"inf"`/`"NaN"`) — a wrong
/// value shipped to every client with no panic to flag it. This asserts on
/// the actual rendered playlist text, not on an intermediate float.
#[test]
fn zero_anchor_timescale_does_not_render_inf_or_nan_into_playlist() {
    let zero_ts_video = TrackSpec::new(
        1,
        0, // malformed mdhd.timescale
        CodecConfig::Avc {
            config: dummy_avc_config(),
            width: 320,
            height: 240,
        },
    );
    let mut seg = LlHlsSegmenter::with_part_target(vec![zero_ts_video], 1000, 1.0, 334)
        .expect("construction does not itself validate per-track timescale");

    assert!(
        seg.part_target_secs().is_finite(),
        "part_target_secs() must not be inf/NaN even with a zero timescale"
    );

    for i in 0..10u8 {
        seg.push(1, vsample(i == 0, i)).unwrap();
    }
    seg.flush().unwrap();

    let parts = seg.take_ready_parts();
    let segments = seg.take_ready_segments();
    assert!(!parts.is_empty(), "some part must have been produced");
    assert!(!segments.is_empty(), "some segment must have been produced");
    for p in &parts {
        assert!(
            p.duration.is_finite(),
            "PartInfo::duration must be finite, got {}",
            p.duration
        );
    }
    for s in &segments {
        assert!(
            s.duration.is_finite(),
            "SegmentInfo::duration must be finite, got {}",
            s.duration
        );
    }

    // Render exactly as a real caller would, and assert on the TEXT.
    let parts_spec: Vec<PartSpec> = parts
        .iter()
        .enumerate()
        .map(|(i, p)| PartSpec {
            uri: format!("seg1.{i}.m4s"),
            // Same reasoning as above: segmenter-computed, finite, >= 0.
            duration: DecimalSeconds::new(p.duration)
                .expect("segmenter part duration is finite, >= 0"),
            independent: p.independent,
            ..Default::default()
        })
        .collect();
    let seg1_duration = segments.first().map(|s| s.duration).unwrap_or(0.0);
    let pl = MediaPlaylist {
        version: 9,
        target_duration: 1,
        media_sequence: 0,
        discontinuity_sequence: 0,
        segments: vec![MediaSegment {
            uri: "seg1.m4s".into(),
            // The segmenter's own computed segment duration (issue #1140):
            // always finite and non-negative.
            duration: DecimalSeconds::new(seg1_duration)
                .expect("segmenter duration is finite, >= 0"),
            discontinuous: false,
            parts: parts_spec,
            ..Default::default()
        }],
        endlist: false,
        extra_tags: vec![],
        low_latency: Some(LowLatencyConfig {
            // `part_target_secs` divides a `u64` tick count by a
            // `.max(1)`-guarded timescale (issue #1140): always finite
            // and non-negative.
            part_target: Some(
                DecimalSeconds::new(seg.part_target_secs())
                    .expect("part_target_secs is finite, >= 0"),
            ),
            part_hold_back: Some(
                DecimalSeconds::new(3.0 * seg.part_target_secs())
                    .expect("part_target_secs is finite, >= 0"),
            ),
            preload_hint_part: None,
            ..Default::default()
        }),
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
    let mut seg =
        LlHlsSegmenter::with_part_target(vec![video_track(), audio_track()], 1000, 1.0, 334)
            .unwrap();

    // 1 s of 30 fps video; 2 audio frames per video frame (1024 @ 48 kHz).
    let mut ai: usize = 0;
    for i in 0..30u8 {
        seg.push(1, vsample(i == 0, i)).unwrap();
        for _ in 0..2 {
            seg.push(
                2,
                Sample::new(vec![ai as u8; 8], None, None, Some(1024), true),
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
            "part {i} (dur {}) must carry both video (track 1) and audio (track 2)",
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
    let mut seg = LlHlsSegmenter::with_part_target(vec![video_track()], 1000, 1.0, 334).unwrap();
    for i in 0..30u8 {
        seg.push(1, vsample(i == 0, i)).unwrap();
    }
    seg.flush().unwrap();
    let parts = seg.take_ready_parts();
    assert!(parts.len() >= 2, "expect several parts");
    // The *final* part of a segment is exempt from the 85% floor but not from
    // the ceiling; the last part here is the segment tail, checked too.
    for (i, p) in parts.iter().enumerate() {
        assert!(
            p.duration <= 0.334 + 1e-9,
            "part {i} duration {} exceeds the 334 ms part target",
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
    let mut seg =
        LlHlsSegmenter::with_part_target(vec![video_track(), audio_track()], 6000, 6.0, 334)
            .unwrap();

    const N_VID: usize = 150; // 6 s at 25 fps
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
            seg.push(1, vsample(i == 0, (i % 251) as u8)).unwrap();
        } else {
            seg.push(
                2,
                Sample::new(vec![(i % 251) as u8; 8], None, None, Some(AUD_DUR), true),
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
            p.duration <= 0.334 + 1e-9,
            "part {i} duration {} exceeds the 334 ms part target",
            p.duration
        );
        assert_eq!(
            traf_count(&p.bytes),
            2,
            "part {i} (duration {}) must carry both video and audio",
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
