//! Gate tests for `#EXT-X-DISCONTINUITY` and `#EXT-X-DISCONTINUITY-SEQUENCE`
//! support in HLS playlist generation (RFC 8216 §4.3.4.3 / §4.3.3.3).
//!
//! Four tests match the four acceptance criteria:
//!
//! 1. **Auto-detect bites** — `mark_init_discontinuities` detects an init
//!    change and emits exactly one `#EXT-X-DISCONTINUITY`; identical inits
//!    emit none.
//! 2. **Explicit mark bites** — `Segmenter::mark_discontinuity()` emits the
//!    tag before the explicitly-marked segment even with identical inits.
//! 3. **DISCONTINUITY-SEQUENCE bites** — the header is present (and
//!    incrementing) when discontinuous segments roll off the window; absent
//!    when 0.
//! 4. **Placement** — the tag immediately precedes `#EXTINF` (and any
//!    `EXT-X-MAP`-like prefix), never appears after it.

use transmux::hls::{MediaPlaylist, MediaSegment};
use transmux::{
    AVCConfigurationBox, AVCDecoderConfigurationRecord, CodecConfig, Sample, SegmentMeta,
    Segmenter, TrackSpec, mark_init_discontinuities,
};

// ---------------------------------------------------------------------------
// Helpers shared across tests
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

/// A second, distinct AVC config (different profile_indication = 100).
fn dummy_avc_config_b() -> AVCConfigurationBox {
    AVCConfigurationBox::new(AVCDecoderConfigurationRecord {
        configuration_version: 1,
        profile_indication: 100,
        profile_compatibility: 0,
        level_indication: 30,
        length_size_minus_one: 3,
        sps: vec![transmux::AvcSps(vec![0x67, 100, 0, 30, 0x00])],
        pps: vec![transmux::AvcPps(vec![0x68, 0xCE, 0x3C, 0x80])],
        chroma_format: None,
        bit_depth_luma_minus8: None,
        bit_depth_chroma_minus8: None,
        sps_ext: vec![],
    })
}

fn video_track_a() -> TrackSpec {
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

fn video_track_b() -> TrackSpec {
    TrackSpec::new(
        1,
        90_000,
        CodecConfig::Avc {
            config: dummy_avc_config_b(),
            width: 320,
            height: 240,
        },
    )
}

/// Build a Segmenter with a single video track and target 1 second.
/// Timescale = 90_000 ticks/s; each sample is 45_000 ticks (0.5 s).
/// Two sync samples → one cut (≥ target_ticks = 90_000).
fn make_segmenter(track: TrackSpec) -> Segmenter {
    Segmenter::new(vec![track], 1000, 1.0).unwrap()
}

// ---------------------------------------------------------------------------
// Test 1 — Auto-detect bites via `mark_init_discontinuities`
// ---------------------------------------------------------------------------

/// Positive case: init changes from A to B between segment 1 and segment 2;
/// segment 2 must be marked discontinuous. Segment 3 has the same init as
/// segment 2, so it must NOT be marked.
///
/// Negative case (woven in): with all-identical init, NO tag is emitted.
#[test]
fn autodetect_init_change_marks_discontinuity() {
    // Build init bytes from two distinct track configs.
    let init_a = transmux::build_init_segment(&[video_track_a()], 1000).unwrap();
    let init_b = transmux::build_init_segment(&[video_track_b()], 1000).unwrap();

    // Sanity: the two inits must actually differ for the test to be non-trivial.
    assert_ne!(
        init_a, init_b,
        "test precondition: distinct track configs yield distinct inits"
    );

    let mut seg0 = MediaSegment {
        uri: "s0.m4s".into(),
        duration: 1.0,
        discontinuous: false,
        parts: vec![],
    };
    let mut seg1 = MediaSegment {
        uri: "s1.m4s".into(),
        duration: 1.0,
        discontinuous: false,
        parts: vec![],
    };
    let mut seg2 = MediaSegment {
        uri: "s2.m4s".into(),
        duration: 1.0,
        discontinuous: false,
        parts: vec![],
    };

    // init_a → init_b → init_b (change at index 1, stable at index 2)
    let mut entries: Vec<(&[u8], &mut MediaSegment)> = vec![
        (init_a.as_slice(), &mut seg0),
        (init_b.as_slice(), &mut seg1),
        (init_b.as_slice(), &mut seg2),
    ];
    mark_init_discontinuities(&mut entries);

    // Segment 0: no preceding segment → never marked.
    assert!(
        !entries[0].1.discontinuous,
        "segment 0 must never be marked"
    );
    // Segment 1: init changed → must be marked.
    assert!(
        entries[1].1.discontinuous,
        "segment 1 must be discontinuous (init changed)"
    );
    // Segment 2: same init as segment 1 → must NOT be marked.
    assert!(
        !entries[2].1.discontinuous,
        "segment 2 must not be marked (same init)"
    );

    // --- Render and assert playlist text ---
    let pl = MediaPlaylist {
        version: 6,
        target_duration: 2,
        media_sequence: 0,
        discontinuity_sequence: 0,
        segments: vec![
            entries.remove(0).1.clone(),
            entries.remove(0).1.clone(),
            entries.remove(0).1.clone(),
        ],
        endlist: true,
        extra_tags: vec![],
        low_latency: None,
        iframes_only: false,
    };
    let out = pl.to_m3u8();

    // Exactly one discontinuity tag.
    assert_eq!(
        out.matches("#EXT-X-DISCONTINUITY\n").count(),
        1,
        "exactly one #EXT-X-DISCONTINUITY expected; playlist:\n{out}"
    );

    // It must immediately precede the #EXTINF of s1.m4s.
    let disc_pos = out.find("#EXT-X-DISCONTINUITY\n").unwrap();
    let extinf_after_disc = &out[disc_pos + "#EXT-X-DISCONTINUITY\n".len()..];
    assert!(
        extinf_after_disc.starts_with("#EXTINF:"),
        "#EXT-X-DISCONTINUITY must be immediately before #EXTINF; got: {:?}",
        &extinf_after_disc[..extinf_after_disc.find('\n').unwrap_or(30).min(30)]
    );

    // --- Negative case: all-identical init produces no tag ---
    let mut seg_x = MediaSegment {
        uri: "x0.m4s".into(),
        duration: 1.0,
        discontinuous: false,
        parts: vec![],
    };
    let mut seg_y = MediaSegment {
        uri: "x1.m4s".into(),
        duration: 1.0,
        discontinuous: false,
        parts: vec![],
    };
    let mut seg_z = MediaSegment {
        uri: "x2.m4s".into(),
        duration: 1.0,
        discontinuous: false,
        parts: vec![],
    };
    let mut same: Vec<(&[u8], &mut MediaSegment)> = vec![
        (init_a.as_slice(), &mut seg_x),
        (init_a.as_slice(), &mut seg_y),
        (init_a.as_slice(), &mut seg_z),
    ];
    mark_init_discontinuities(&mut same);
    assert!(!same[0].1.discontinuous);
    assert!(
        !same[1].1.discontinuous,
        "no discontinuity when init is identical"
    );
    assert!(!same[2].1.discontinuous);

    let pl_neg = MediaPlaylist {
        version: 6,
        target_duration: 2,
        media_sequence: 0,
        discontinuity_sequence: 0,
        segments: vec![
            same.remove(0).1.clone(),
            same.remove(0).1.clone(),
            same.remove(0).1.clone(),
        ],
        endlist: true,
        extra_tags: vec![],
        low_latency: None,
        iframes_only: false,
    };
    let out_neg = pl_neg.to_m3u8();
    assert!(
        !out_neg.contains("#EXT-X-DISCONTINUITY\n"),
        "no #EXT-X-DISCONTINUITY expected with identical inits; playlist:\n{out_neg}"
    );
}

// ---------------------------------------------------------------------------
// Test 2 — Explicit mark_discontinuity() bites
// ---------------------------------------------------------------------------

/// Marking segment k discontinuous via `Segmenter::mark_discontinuity()` must
/// emit the tag before that segment's `#EXTINF`, even when the init bytes are
/// identical across all segments (because the init never changes in a single
/// Segmenter instance).
///
/// Setup: one video track, target = 1 s, 90_000 ticks/s.
///
/// Cut rule: a cut fires *before* a keyframe when the buffered anchor duration
/// has reached the target. So we need at least `target_ticks` (90_000) in the
/// buffer before a sync sample arrives to trigger the cut.
///
/// Pattern per segment (3 pushes per cut):
///
/// - push 1 sync sample (opens the segment, 45_000 ticks buffered)
/// - push 1 non-sync sample (now 90_000 ticks buffered)
/// - push 1 sync sample (≥ target_ticks → cut happens before this sample)
///
/// The third sync starts the next segment.
#[test]
fn explicit_mark_discontinuity_bites() {
    let mut seg = make_segmenter(video_track_a());

    // Helper: push one sync then one non-sync to fill the buffer.
    let fill = |s: &mut Segmenter| {
        let sync = Sample::new(vec![0u8; 4], 45_000, true, 0);
        let non_sync = Sample::new(vec![0u8; 4], 45_000, false, 0);
        s.push(1, sync).unwrap();
        s.push(1, non_sync).unwrap();
    };

    // Trigger sample (sync that causes the preceding buffer to be cut).
    let trigger = || Sample::new(vec![0u8; 4], 45_000, true, 0);

    // Fill segment 0's buffer (90_000 ticks pending).
    fill(&mut seg);
    // Trigger the cut for segment 0; this sync starts segment 1's buffer.
    seg.push(1, trigger()).unwrap();

    let batch0 = seg.take_ready_with_meta();
    assert_eq!(batch0.len(), 1, "expected 1 segment after first cut");
    assert!(!batch0[0].1.discontinuous, "segment 0 must be continuous");

    // Mark the NEXT cut as discontinuous.
    seg.mark_discontinuity();

    // Fill segment 1's buffer; the anchor already has 45_000 ticks from the trigger.
    let non_sync = Sample::new(vec![0u8; 4], 45_000, false, 0);
    seg.push(1, non_sync).unwrap();
    // Now trigger the cut for segment 1 — must be discontinuous.
    seg.push(1, trigger()).unwrap();

    let batch1 = seg.take_ready_with_meta();
    assert_eq!(batch1.len(), 1, "expected 1 segment after mark + cut");
    assert!(
        batch1[0].1.discontinuous,
        "segment 1 must be discontinuous (explicitly marked)"
    );

    // No mark — flush remaining buffer as segment 2 (continuous).
    seg.flush().unwrap();
    let batch2 = seg.take_ready_with_meta();
    assert!(!batch2.is_empty(), "flush must produce a segment");
    assert!(
        !batch2[0].1.discontinuous,
        "segment 2 must be continuous (no mark)"
    );

    // Build a playlist and verify the rendered text.
    let seg_metas: Vec<(Vec<u8>, SegmentMeta)> = [batch0, batch1, batch2].concat();
    let pl_segments: Vec<MediaSegment> = seg_metas
        .iter()
        .enumerate()
        .map(|(i, (_bytes, meta))| MediaSegment {
            uri: format!("s{i}.m4s"),
            duration: 1.0,
            discontinuous: meta.discontinuous,
            parts: vec![],
        })
        .collect();

    let pl = MediaPlaylist {
        version: 6,
        target_duration: 2,
        media_sequence: 0,
        discontinuity_sequence: 0,
        segments: pl_segments,
        endlist: true,
        extra_tags: vec![],
        low_latency: None,
        iframes_only: false,
    };
    let out = pl.to_m3u8();

    // Exactly one discontinuity tag (for segment 1).
    assert_eq!(
        out.matches("#EXT-X-DISCONTINUITY\n").count(),
        1,
        "exactly one #EXT-X-DISCONTINUITY expected; playlist:\n{out}"
    );

    // The tag must be immediately before the #EXTINF of s1.m4s — verify
    // by finding s1.m4s and tracing back.
    let s1_uri_pos = out.find("s1.m4s\n").expect("s1.m4s in playlist");
    let before_s1 = &out[..s1_uri_pos];
    let extinf_pos = before_s1.rfind("#EXTINF:").expect("#EXTINF before s1");
    let disc_pos = before_s1
        .rfind("#EXT-X-DISCONTINUITY\n")
        .expect("#EXT-X-DISCONTINUITY before s1");

    // The tag must come immediately before #EXTINF (no intervening line).
    assert_eq!(
        disc_pos + "#EXT-X-DISCONTINUITY\n".len(),
        extinf_pos,
        "#EXT-X-DISCONTINUITY must be the line immediately before #EXTINF for s1"
    );
}

// ---------------------------------------------------------------------------
// Test 3 — DISCONTINUITY-SEQUENCE bites
// ---------------------------------------------------------------------------

/// In a sliding-window playlist: when discontinuous segments roll off the
/// front, the `#EXT-X-DISCONTINUITY-SEQUENCE` header must increment.
///
/// This is a pure `MediaPlaylist` rendering test — the sequence counter is
/// a field the caller manages (it equals the count of discontinuities that
/// have left the window). We verify:
///
/// - When `discontinuity_sequence == 0`: header is absent.
/// - When `discontinuity_sequence == 1`: `#EXT-X-DISCONTINUITY-SEQUENCE:1\n` present.
/// - When `discontinuity_sequence == 2`: `#EXT-X-DISCONTINUITY-SEQUENCE:2\n` present.
///
/// The "sliding off" scenario: imagine a live window that started with a
/// discontinuity at segment index 0 (now evicted). The header carries that
/// lost count so HLS clients can synchronise renditions.
#[test]
fn discontinuity_sequence_increments_as_segments_roll_off() {
    fn live_window(disc_seq: u64, segs: Vec<MediaSegment>) -> String {
        MediaPlaylist {
            version: 6,
            target_duration: 6,
            media_sequence: disc_seq, // media_sequence advances in lock-step for realism
            discontinuity_sequence: disc_seq,
            segments: segs,
            endlist: false,
            extra_tags: vec![],
            low_latency: None,
            iframes_only: false,
        }
        .to_m3u8()
    }

    let seg = |uri: &str| MediaSegment {
        uri: uri.into(),
        duration: 6.0,
        discontinuous: false,
        parts: vec![],
    };

    // Window with no evicted discontinuities: header absent.
    let out0 = live_window(0, vec![seg("s0.m4s"), seg("s1.m4s"), seg("s2.m4s")]);
    assert!(
        !out0.contains("#EXT-X-DISCONTINUITY-SEQUENCE"),
        "header must be absent when sequence == 0; playlist:\n{out0}"
    );

    // One discontinuity has rolled off: header must be present with value 1.
    let out1 = live_window(1, vec![seg("s1.m4s"), seg("s2.m4s"), seg("s3.m4s")]);
    assert!(
        out1.contains("#EXT-X-DISCONTINUITY-SEQUENCE:1\n"),
        "#EXT-X-DISCONTINUITY-SEQUENCE:1 expected; playlist:\n{out1}"
    );

    // Two discontinuities have rolled off: header value 2.
    let out2 = live_window(2, vec![seg("s2.m4s"), seg("s3.m4s"), seg("s4.m4s")]);
    assert!(
        out2.contains("#EXT-X-DISCONTINUITY-SEQUENCE:2\n"),
        "#EXT-X-DISCONTINUITY-SEQUENCE:2 expected; playlist:\n{out2}"
    );

    // The sequence header must appear BEFORE any segment entry.
    let hdr = "#EXT-X-DISCONTINUITY-SEQUENCE:1\n";
    let seq_pos = out1.find(hdr).unwrap();
    let first_extinf_pos = out1.find("#EXTINF:").unwrap();
    assert!(
        seq_pos < first_extinf_pos,
        "#EXT-X-DISCONTINUITY-SEQUENCE header must precede segment entries"
    );
}

// ---------------------------------------------------------------------------
// Test 4 — Placement: tag precedes EXTINF (and not after it)
// ---------------------------------------------------------------------------

/// The `#EXT-X-DISCONTINUITY` tag must appear on the line immediately BEFORE
/// the `#EXTINF` of the discontinuous segment — never on the line after it
/// or anywhere else in the playlist entry.
///
/// We also verify that the continuous segments in the same playlist do NOT
/// have a stray `#EXT-X-DISCONTINUITY` tag preceding them.
#[test]
fn discontinuity_tag_placement_immediately_before_extinf() {
    // 4-segment playlist: segments 0, 2 are continuous; segments 1 and 3 are
    // discontinuous. That gives two tags; both must be placed correctly.
    let pl = MediaPlaylist {
        version: 6,
        target_duration: 6,
        media_sequence: 0,
        discontinuity_sequence: 0,
        segments: vec![
            MediaSegment {
                uri: "s0.m4s".into(),
                duration: 6.0,
                discontinuous: false,
                parts: vec![],
            },
            MediaSegment {
                uri: "s1.m4s".into(),
                duration: 6.0,
                discontinuous: true,
                parts: vec![],
            },
            MediaSegment {
                uri: "s2.m4s".into(),
                duration: 6.0,
                discontinuous: false,
                parts: vec![],
            },
            MediaSegment {
                uri: "s3.m4s".into(),
                duration: 6.0,
                discontinuous: true,
                parts: vec![],
            },
        ],
        endlist: true,
        extra_tags: vec![],
        low_latency: None,
        iframes_only: false,
    };
    let out = pl.to_m3u8();

    // Exactly two discontinuity tags.
    assert_eq!(
        out.matches("#EXT-X-DISCONTINUITY\n").count(),
        2,
        "expected exactly 2 tags; playlist:\n{out}"
    );

    // For each discontinuous segment URI, verify that the line immediately
    // before its URI (one line for the URI, one line for #EXTINF above it,
    // one line for #EXT-X-DISCONTINUITY above that) is the tag.
    for uri in &["s1.m4s", "s3.m4s"] {
        let uri_line = format!("{uri}\n");
        let uri_pos = out
            .find(uri_line.as_str())
            .unwrap_or_else(|| panic!("URI {uri} not found in playlist:\n{out}"));
        let before_uri = &out[..uri_pos];
        // The line before the URI is the #EXTINF.
        let extinf_pos = before_uri.rfind("#EXTINF:").expect("EXTINF before URI");
        let before_extinf = &out[..extinf_pos];
        // The line before #EXTINF must be #EXT-X-DISCONTINUITY.
        let tag = "#EXT-X-DISCONTINUITY\n";
        assert!(
            before_extinf.ends_with(tag),
            "line before #EXTINF of {uri} must be #EXT-X-DISCONTINUITY; \
             got ending: {:?}",
            &before_extinf[before_extinf.len().saturating_sub(40)..]
        );
    }

    // For each continuous segment URI, verify that the line before its
    // #EXTINF is NOT the discontinuity tag.
    for uri in &["s0.m4s", "s2.m4s"] {
        let uri_line = format!("{uri}\n");
        let uri_pos = out
            .find(uri_line.as_str())
            .unwrap_or_else(|| panic!("URI {uri} not found in playlist:\n{out}"));
        let before_uri = &out[..uri_pos];
        let extinf_pos = before_uri.rfind("#EXTINF:").expect("EXTINF before URI");
        let before_extinf = &out[..extinf_pos];
        assert!(
            !before_extinf.ends_with("#EXT-X-DISCONTINUITY\n"),
            "continuous segment {uri} must NOT have #EXT-X-DISCONTINUITY before it"
        );
    }
}
