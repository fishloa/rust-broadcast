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

use transmux::hls::{LowLatencyConfig, MediaPlaylist, MediaSegment, PartSpec};
use transmux::ll_hls::LlHlsSegmenter;
use transmux::{
    AVCConfigurationBox, AVCDecoderConfigurationRecord, CodecConfig, DecoderConfigDescriptor,
    DecoderSpecificInfo, ESDescriptor, EsdsBox, FragmentTrackData, MovieFragmentBox,
    ObjectTypeIndication, SLConfigDescriptor, Sample, StreamType, TrackSpec,
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
    EsdsBox::new(ESDescriptor {
        es_id: 1,
        stream_dependence_flag: false,
        url_flag: false,
        ocr_stream_flag: false,
        stream_priority: 0,
        depends_on_es_id: None,
        url: None,
        ocr_es_id: None,
        decoder_config: Some(DecoderConfigDescriptor {
            object_type_indication: ObjectTypeIndication(0x40),
            stream_type: StreamType(0x05),
            up_stream: false,
            buffer_size_db: 0,
            max_bitrate: 0,
            avg_bitrate: 0,
            decoder_specific_info: Some(DecoderSpecificInfo {
                data: vec![0x12, 0x10],
            }),
        }),
        sl_config: Some(SLConfigDescriptor { body: vec![0x02] }),
    })
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
    Sample::new(vec![byte; 32], VID_DUR, is_sync, 0)
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
            duration: p.duration,
            independent: p.independent,
        })
        .collect();
    let pl = MediaPlaylist {
        version: 9,
        target_duration: 1,
        media_sequence: 0,
        discontinuity_sequence: 0,
        segments: vec![MediaSegment {
            uri: "seg1.m4s".into(),
            duration: 1.0,
            discontinuous: false,
            parts: parts_spec,
        }],
        endlist: false,
        extra_tags: vec![],
        low_latency: Some(LowLatencyConfig {
            part_target: 0.334,
            part_hold_back: 1.002,
            preload_hint_part: None,
        }),
        iframes_only: false,
    };
    let m3u8 = pl.to_m3u8();
    // Part 0 has INDEPENDENT=YES; parts 1 and 2 do not.
    assert!(
        m3u8.contains("#EXT-X-PART:DURATION=0.367,URI=\"seg1.0.m4s\",INDEPENDENT=YES"),
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
            duration: 0.334,
            independent: true,
        },
        PartSpec {
            uri: "seg0.1.m4s".into(),
            duration: 0.334,
            independent: false,
        },
    ];
    let ll = LowLatencyConfig {
        part_target: 0.334,
        // Deliberately too small; renderer must raise to 3 x 0.334 = 1.002.
        part_hold_back: 0.5,
        preload_hint_part: Some("seg0.2.m4s".into()),
    };
    let pl = MediaPlaylist {
        version: 9,
        target_duration: 1,
        media_sequence: 0,
        discontinuity_sequence: 0,
        segments: vec![MediaSegment {
            uri: "seg0.m4s".into(),
            duration: 1.0,
            discontinuous: false,
            parts,
        }],
        endlist: false,
        extra_tags: vec![],
        low_latency: Some(ll.clone()),
        iframes_only: false,
    };
    let m3u8 = pl.to_m3u8();

    // #EXT-X-PART-INF exact.
    assert!(
        m3u8.contains("#EXT-X-PART-INF:PART-TARGET=0.334\n"),
        "PART-INF must carry PART-TARGET=0.334:\n{m3u8}"
    );
    // #EXT-X-SERVER-CONTROL with PART-HOLD-BACK >= 3 x part-target.
    let effective = ll.effective_part_hold_back();
    assert!((effective - 1.002).abs() < 1e-6, "PHB floor = 3 x 0.334");
    assert!(
        m3u8.contains("#EXT-X-SERVER-CONTROL:CAN-BLOCK-RELOAD=YES,PART-HOLD-BACK=1.002\n"),
        "SERVER-CONTROL must carry CAN-BLOCK-RELOAD + raised PART-HOLD-BACK:\n{m3u8}"
    );
    assert!(
        effective >= 3.0 * ll.part_target,
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
    let plain_m3u8 = plain.to_m3u8();
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
            let s = Sample::from_raw(vec![(*idx as u8).wrapping_add(1); 20], AUD_DUR);
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
            FragmentTrackData {
                track_id: 1,
                base_media_decode_time: 0,
                samples: &vid_samples,
            },
            FragmentTrackData {
                track_id: 2,
                base_media_decode_time: 0,
                samples: &aud_samples,
            },
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
