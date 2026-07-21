//! Origin <-> client in-process loop (issue #717 slices 2-4).
//!
//! No real IO: a `transmux::ll_hls::LlHlsSegmenter` plays the origin, its
//! `MediaPlaylist`/part/segment bytes are handed to `LlHlsClient` exactly as a
//! caller's HTTP fetch loop would, and the assertions bite on the client's
//! *behaviour*, not just that it parses without panicking:
//!
//! - it requests the right resources (blocking reload with the correct
//!   `_HLS_msn`/`_HLS_part`, the preload-hinted part prefetched ahead of its
//!   own numbered appearance);
//! - it emits the init segment exactly once, then every sample in the exact
//!   order/bytes the segmenter was fed — a no-op or non-deduping client fails
//!   this;
//! - once a segment's parts have all been individually delivered, its closure
//!   in a later playlist reload produces **zero** new fetch actions (dedup);
//! - a non-LL playlist (no PART tags at all) still plays via the full-segment
//!   fallback path.

use ll_hls_runtime::client::{Action, LlHlsClient, Output, ResourceId};
use transmux::hls::{LowLatencyConfig, MapTag, MediaPlaylist, MediaSegment, OpenSegment, PartSpec};
use transmux::ll_hls::{LlHlsSegmenter, PartInfo, SegmentInfo};
use transmux::{
    AVCConfigurationBox, AVCDecoderConfigurationRecord, AvcPps, AvcSps, CodecConfig, Sample,
    TrackSpec,
};

const INIT_URL: &str = "http://origin/live/init.mp4";
const PLAYLIST_URL: &str = "http://origin/live/stream.m3u8";
const VID_DUR: u32 = 3000; // 90 kHz / 30 fps

fn dummy_avc_config() -> AVCConfigurationBox {
    AVCConfigurationBox::new(AVCDecoderConfigurationRecord {
        configuration_version: 1,
        profile_indication: 66,
        profile_compatibility: 0,
        level_indication: 30,
        length_size_minus_one: 3,
        sps: vec![AvcSps(vec![0x67, 66, 0, 30, 0x00])],
        pps: vec![AvcPps(vec![0x68, 0xCE, 0x3C, 0x80])],
        chroma_format: None,
        bit_depth_luma_minus8: None,
        bit_depth_chroma_minus8: None,
        sps_ext: vec![],
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

fn vsample(is_sync: bool, byte: u8) -> Sample {
    Sample::new(vec![byte; 32], VID_DUR, is_sync, 0)
}

/// Part URI convention: `seg<segment_seq>.<part_index>.m4s`.
fn part_uri(p: &PartInfo) -> String {
    format!("seg{}.{}.m4s", p.segment_seq, p.part_index)
}

/// Segment URI convention: `seg<segment_seq>.m4s`.
fn segment_uri(s: &SegmentInfo) -> String {
    format!("seg{}.m4s", s.segment_seq)
}

/// Resolve a relative playlist URI against the fixed playlist directory
/// (`http://origin/live/`) — matching what `LlHlsClient` itself resolves
/// relative part/segment/map URIs to, so the origin fixture's lookup keys
/// agree with the `url` field on the client's `Action::FetchResource`.
fn abs_uri(relative: &str) -> String {
    format!("http://origin/live/{relative}")
}

fn part_spec(p: &PartInfo) -> PartSpec {
    PartSpec {
        uri: part_uri(p),
        duration: p.duration,
        independent: p.independent,
        byte_range: None,
        gap: false,
    }
}

/// Render a full LL-HLS media playlist from whatever the segmenter has
/// produced so far: `closed` segments (each carrying all its parts), an
/// optional trailing `open_parts` (the in-progress segment, segment number
/// `open_seq`), and an optional preload-hint part index right after those.
struct PlaylistBuilder<'a> {
    media_sequence: u64,
    part_target_secs: f64,
    closed: &'a [(SegmentInfo, Vec<PartInfo>)],
    open_seq: Option<u32>,
    open_parts: &'a [PartInfo],
    preload_hint_next_index: Option<u64>,
    endlist: bool,
}

impl PlaylistBuilder<'_> {
    fn build(&self) -> MediaPlaylist {
        let segments: Vec<MediaSegment> = self
            .closed
            .iter()
            .enumerate()
            .map(|(i, (info, parts))| MediaSegment {
                uri: segment_uri(info),
                duration: info.duration,
                discontinuous: false,
                parts: parts.iter().map(part_spec).collect(),
                byte_range: None,
                map: if i == 0 {
                    Some(MapTag {
                        uri: INIT_URL.to_string(),
                        byte_range: None,
                    })
                } else {
                    None
                },
            })
            .collect();
        // EXT-X-MAP carries forward: stamp every segment after the first with
        // the same map (mirrors `MediaPlaylist::parse`'s carry-forward view).
        let mut segments = segments;
        for i in 1..segments.len() {
            if segments[i].map.is_none() {
                segments[i].map = segments[i - 1].map.clone();
            }
        }

        let open_segment = self
            .open_seq
            .map(|_| OpenSegment::new(self.open_parts.iter().map(part_spec).collect()));

        let preload_hint_part = self.preload_hint_next_index.map(|idx| {
            let seq = self.open_seq.unwrap_or(self.closed.len() as u32 + 1);
            format!("seg{seq}.{idx}.m4s")
        });

        MediaPlaylist {
            version: 9,
            target_duration: 1,
            media_sequence: self.media_sequence,
            discontinuity_sequence: 0,
            segments,
            open_segment,
            endlist: self.endlist,
            extra_tags: vec![],
            low_latency: Some(LowLatencyConfig {
                part_target: self.part_target_secs,
                part_hold_back: 3.0 * self.part_target_secs,
                preload_hint_part,
                ..Default::default()
            }),
            iframes_only: false,
            rendition_reports: vec![],
            skip: None,
        }
    }
}

/// A tiny in-process "HTTP" fixture: resolves a resource id/URL to bytes.
struct Origin {
    init_bytes: Vec<u8>,
    parts_by_uri: std::collections::HashMap<String, Vec<u8>>,
    segments_by_uri: std::collections::HashMap<String, Vec<u8>>,
}

impl Origin {
    fn fetch(&self, action: &Action) -> Option<(ResourceId, Vec<u8>)> {
        match action {
            Action::FetchResource { id, url, .. } => {
                let bytes = if *id == ResourceId::Init {
                    self.init_bytes.clone()
                } else if let Some(b) = self.parts_by_uri.get(url) {
                    b.clone()
                } else {
                    self.segments_by_uri.get(url).cloned()?
                };
                Some((*id, bytes))
            }
            _ => None,
        }
    }
}

/// Drain every currently-pending `Action`, returning them (does not perform
/// any IO/feeding — a test helper to inspect what the client asked for).
fn drain_actions(client: &mut LlHlsClient) -> Vec<Action> {
    let mut out = Vec::new();
    while let Some(a) = client.poll() {
        out.push(a);
    }
    out
}

fn drain_outputs(client: &mut LlHlsClient) -> Vec<Output> {
    let mut out = Vec::new();
    while let Some(o) = client.next_output() {
        out.push(o);
    }
    out
}

#[test]
fn origin_client_loop_blocking_reload_prefetch_dedup_and_ordered_output() {
    // --- Build the origin: one closed segment (3 parts) + an open second
    // segment with 1 known part and a preload hint for its next (not-yet-
    // available) part. ---
    let mut seg = LlHlsSegmenter::with_part_target(vec![video_track()], 1000, 1.0, 334).unwrap();
    let mut fed_samples: Vec<Sample> = Vec::new();
    for i in 0..30u8 {
        let s = vsample(i == 0, i);
        fed_samples.push(s.clone());
        seg.push(1, s).unwrap();
    }
    // Keyframe past target closes segment 1 (3 parts @ ~334ms).
    let s = vsample(true, 200);
    fed_samples.push(s.clone());
    seg.push(1, s).unwrap();
    // Exactly 10 more samples: together with the keyframe above that's 11
    // samples since the last part flush, which (at 334ms/30060-tick parts)
    // crosses the part-target boundary with zero remainder — segment 2's
    // first part closes cleanly with no unflushed tail left buffered inside
    // the segmenter (every pushed sample ends up in a fetchable part).
    for i in 0..10u8 {
        let s = vsample(false, 100 + i);
        fed_samples.push(s.clone());
        seg.push(1, s).unwrap();
    }
    // 11 more (again exactly hitting the part-target boundary with zero
    // remainder): this becomes segment 2's *second* part. The playlist built
    // below only numbers part 0 as a known `EXT-X-PART` and merely
    // preload-hints part 1's URI — but the origin fixture already has part
    // 1's bytes ready, exactly as a real server's preload-hint GET would
    // block until the part exists, then return it.
    for i in 0..11u8 {
        let s = vsample(false, 120 + i);
        fed_samples.push(s.clone());
        seg.push(1, s).unwrap();
    }
    let init_bytes = seg.init_segment().unwrap();
    let part_target_secs = seg.part_target_secs();

    let mut ready_parts = seg.take_ready_parts();
    let ready_segments = seg.take_ready_segments();
    assert_eq!(ready_segments.len(), 1, "segment 1 must have closed");
    assert!(
        ready_parts.iter().any(|p| p.segment_seq == 2),
        "segment 2 must have an open part"
    );

    let seg1_parts: Vec<PartInfo> = ready_parts
        .iter()
        .filter(|p| p.segment_seq == 1)
        .cloned()
        .collect();
    let seg2_all_parts: Vec<PartInfo> = ready_parts
        .iter()
        .filter(|p| p.segment_seq == 2)
        .cloned()
        .collect();
    assert_eq!(seg1_parts.len(), 3);
    assert_eq!(seg2_all_parts.len(), 2, "segment 2 has parts 0 and 1 ready");
    // Only part 0 is "known" (numbered) in playlist #1 — part 1 is reachable
    // only via the preload hint at this point.
    let seg2_known_parts: Vec<PartInfo> = seg2_all_parts
        .iter()
        .filter(|p| p.part_index == 0)
        .cloned()
        .collect();
    assert_eq!(seg2_known_parts.len(), 1);

    // --- Build the origin's playlist #1: segment 1 closed, segment 2 open
    // with its 1 known part + a preload hint for part index 1. ---
    let closed = vec![(ready_segments[0].clone(), seg1_parts.clone())];
    let pl1 = PlaylistBuilder {
        media_sequence: 1,
        part_target_secs,
        closed: &closed,
        open_seq: Some(2),
        open_parts: &seg2_known_parts,
        preload_hint_next_index: Some(1),
        endlist: false,
    }
    .build();
    let pl1_text = pl1.to_m3u8();

    // --- Wire an origin fixture serving byte content by URI (the fixture
    // knows about BOTH of segment 2's parts, even though playlist #1 only
    // numbers part 0 — part 1 is available to the preload-hint fetch). ---
    let mut parts_by_uri = std::collections::HashMap::new();
    for p in &seg1_parts {
        parts_by_uri.insert(abs_uri(&part_uri(p)), p.bytes.clone());
    }
    for p in &seg2_all_parts {
        parts_by_uri.insert(abs_uri(&part_uri(p)), p.bytes.clone());
    }
    let mut segments_by_uri = std::collections::HashMap::new();
    for s in &ready_segments {
        segments_by_uri.insert(abs_uri(&segment_uri(s)), s.bytes.clone());
    }
    let origin = Origin {
        init_bytes: init_bytes.clone(),
        parts_by_uri,
        segments_by_uri,
    };

    // === Drive the client ===
    let mut client = LlHlsClient::new(PLAYLIST_URL);

    // 1. First action: fetch the playlist (no LL info known yet).
    let first = client.poll().expect("first action");
    match &first {
        Action::FetchPlaylist {
            url,
            blocking,
            skip,
        } => {
            assert_eq!(url, PLAYLIST_URL);
            assert!(blocking.is_none());
            assert!(!skip);
        }
        other => panic!("expected FetchPlaylist, got {other:?}"),
    }
    assert!(client.poll().is_none(), "nothing else queued yet");

    // 2. Feed playlist #1.
    client.on_playlist(pl1_text.as_bytes()).unwrap();
    let actions = drain_actions(&mut client);

    // Assert: init fetch requested.
    assert!(
        actions
            .iter()
            .any(|a| matches!(a, Action::FetchResource { id: ResourceId::Init, url, .. } if url == INIT_URL)),
        "must request the init segment: {actions:#?}"
    );
    // Assert: every part of segment 1 requested.
    for p in &seg1_parts {
        let want_url = format!("http://origin/live/{}", part_uri(p));
        assert!(
            actions.iter().any(|a| matches!(a,
                Action::FetchResource { id: ResourceId::Part { msn: 1, part }, url, .. }
                    if *part == p.part_index as u64 && *url == want_url
            )),
            "must request seg1 part {}: {actions:#?}",
            p.part_index
        );
    }
    // Assert: segment 2's known part (index 0) requested.
    assert!(
        actions.iter().any(|a| matches!(
            a,
            Action::FetchResource {
                id: ResourceId::Part { msn: 2, part: 0 },
                ..
            }
        )),
        "must request seg2 part 0: {actions:#?}"
    );
    // Assert: the PRELOAD HINT (seg2 part index 1, not yet numbered as
    // EXT-X-PART) is prefetched too — this is the bar that a naive client
    // (which only requests numbered #EXT-X-PART lines) fails.
    assert!(
        actions.iter().any(|a| matches!(a,
            Action::FetchResource { id: ResourceId::Part { msn: 2, part: 1 }, url, .. }
                if url == "http://origin/live/seg2.1.m4s"
        )),
        "must prefetch the EXT-X-PRELOAD-HINT part (seg2 part 1): {actions:#?}"
    );
    // Assert: the next reload is a BLOCKING reload naming seg2/part 1 (the
    // next not-yet-available part, i.e. one past the known/prefetched part 0).
    let reload = actions
        .iter()
        .find(|a| matches!(a, Action::FetchPlaylist { .. }))
        .expect("a reload action must be queued");
    match reload {
        Action::FetchPlaylist { url, blocking, .. } => {
            assert_eq!(url, PLAYLIST_URL);
            let b = blocking.expect("must be a blocking reload (LL playlist)");
            assert_eq!(b.msn, 2, "blocking reload must target seg 2");
            assert_eq!(
                b.part,
                Some(1),
                "blocking reload must target the next unseen part"
            );
        }
        _ => unreachable!(),
    }
    let reload_url = reload.playlist_request_url().unwrap();
    assert!(reload_url.contains("_HLS_msn=2"), "{reload_url}");
    assert!(reload_url.contains("_HLS_part=1"), "{reload_url}");

    // 3. Deliver every FetchResource the client asked for, from the origin
    //    fixture (init, seg1's 3 parts, seg2's part 0, and the prefetched
    //    seg2 part 1 — delivered here even though it wasn't yet a numbered
    //    #EXT-X-PART, exactly as a real preload-hint fetch would complete
    //    before the next playlist reload confirms it).
    for a in &actions {
        if let Some((id, bytes)) = origin.fetch(a) {
            client.on_resource(id, &bytes).unwrap();
        }
    }

    // 4. Drain output: exactly one Init, then every sample fed to the
    //    segmenter, in order, byte-identical.
    let outputs = drain_outputs(&mut client);
    let mut got_init = false;
    let mut got_samples: Vec<Sample> = Vec::new();
    for o in outputs {
        match o {
            Output::Init(bytes) => {
                assert!(!got_init, "init must be emitted exactly once");
                assert_eq!(
                    bytes, init_bytes,
                    "init bytes must match the origin's init segment"
                );
                got_init = true;
            }
            Output::Samples { track_id, samples } => {
                assert!(got_init, "samples must follow init");
                assert_eq!(track_id, 1);
                got_samples.extend(samples);
            }
            other => panic!("unexpected output: {other:?}"),
        }
    }
    assert!(got_init);
    assert_eq!(
        got_samples.len(),
        fed_samples.len(),
        "must reconstruct every sample fed to the segmenter (no gaps, no duplicates)"
    );
    for (got, want) in got_samples.iter().zip(fed_samples.iter()) {
        assert_eq!(got.data, want.data, "sample bytes must match exactly");
        assert_eq!(got.duration, want.duration);
        assert_eq!(got.is_sync, want.is_sync);
    }

    // === Dedup: playlist #2 shows segment 2 closed with the SAME parts
    // (0 and 1, both already delivered) — must produce ZERO new fetch
    // actions for segment 2 (no re-fetch of the whole segment, no re-fetch
    // of its already-delivered parts), only the next reload. ===
    let seg2_closed = SegmentInfo {
        bytes: Vec::new(), // never fetched whole in the LL (parts-coalesce) path
        duration: seg2_all_parts.iter().map(|p| p.duration).sum(),
        segment_seq: 2,
        part_count: 2,
    };
    let closed2 = vec![
        (ready_segments[0].clone(), seg1_parts.clone()),
        (seg2_closed, seg2_all_parts.clone()),
    ];
    let pl2 = PlaylistBuilder {
        media_sequence: 1,
        part_target_secs,
        closed: &closed2,
        open_seq: None,
        open_parts: &[],
        preload_hint_next_index: None,
        endlist: true, // end the stream so we can assert EndOfStream too.
    }
    .build();
    client.on_playlist(pl2.to_m3u8().as_bytes()).unwrap();
    let actions2 = drain_actions(&mut client);
    assert!(
        actions2.is_empty(),
        "closing an already-fully-delivered segment must not trigger new fetches: {actions2:#?}"
    );
    let outputs2 = drain_outputs(&mut client);
    assert_eq!(
        outputs2.len(),
        1,
        "no new samples (dedup) — just EndOfStream: {outputs2:#?}"
    );
    assert!(matches!(outputs2[0], Output::EndOfStream));

    let _ = seg;
    let _ = ready_parts.drain(..); // silence unused-mut if the compiler flags it
}

// ===========================================================================
// Issue #717 slice 1 fix: `CAN-BLOCK-RELOAD=NO` must not be blocked on.
// ===========================================================================

/// An origin that carries LL-HLS tags (`PART-INF`/`PART`) but explicitly
/// declines blocking reload (`CAN-BLOCK-RELOAD=NO`) must get a plain,
/// non-blocking reload + a `WaitMs` backoff hint — not a blocking
/// `_HLS_msn`/`_HLS_part` request. A client that infers blocking support
/// from `low_latency.is_some()` alone (the pre-fix behaviour) fails this.
#[test]
fn can_block_reload_no_yields_non_blocking_reload_with_backoff() {
    let pl = MediaPlaylist {
        version: 9,
        target_duration: 2,
        media_sequence: 0,
        discontinuity_sequence: 0,
        segments: vec![MediaSegment {
            uri: "seg0.m4s".to_string(),
            duration: 1.0,
            discontinuous: false,
            parts: vec![PartSpec {
                uri: "seg0.0.m4s".to_string(),
                duration: 1.0,
                independent: true,
                byte_range: None,
                gap: false,
            }],
            byte_range: None,
            map: Some(MapTag {
                uri: INIT_URL.to_string(),
                byte_range: None,
            }),
        }],
        open_segment: None,
        endlist: false,
        extra_tags: vec![],
        low_latency: Some(LowLatencyConfig {
            part_target: 0.5,
            part_hold_back: 1.5,
            can_block_reload: false,
            ..Default::default()
        }),
        iframes_only: false,
        rendition_reports: vec![],
        skip: None,
    };

    let mut client = LlHlsClient::new(PLAYLIST_URL);
    let _ = client.poll(); // discard the initial plain GET
    client.on_playlist(pl.to_m3u8().as_bytes()).unwrap();
    let actions = drain_actions(&mut client);

    let reload = actions
        .iter()
        .find(|a| matches!(a, Action::FetchPlaylist { .. }))
        .expect("a reload action must be queued");
    match reload {
        Action::FetchPlaylist { blocking, .. } => {
            assert!(
                blocking.is_none(),
                "CAN-BLOCK-RELOAD=NO must never produce a blocking reload: {reload:?}"
            );
        }
        _ => unreachable!(),
    }
    assert!(
        actions.iter().any(|a| matches!(a, Action::WaitMs(_))),
        "a non-blocking reload must be paced with a WaitMs backoff hint: {actions:#?}"
    );
}

// ===========================================================================
// Non-LL (full-segment) fallback.
// ===========================================================================

#[test]
fn non_ll_playlist_plays_via_full_segment_fallback() {
    let mut seg =
        LlHlsSegmenter::with_part_target(vec![video_track()], 1000, 1.0, 100_000).unwrap();
    let mut fed_samples: Vec<Sample> = Vec::new();
    for i in 0..30u8 {
        let s = vsample(i == 0, i);
        fed_samples.push(s.clone());
        seg.push(1, s).unwrap();
    }
    // NOTE: this trailing keyframe closes segment 1 (30 samples) and, since
    // it is itself pushed as the first sample of segment 2, `flush()` then
    // closes that (tiny, 1-sample) segment 2 too — so `take_ready_segments()`
    // below returns *two* segments. Only segment 1 (the 30 fed_samples) is
    // exercised by this playlist/test; segment 2 is deliberately ignored.
    seg.push(1, vsample(true, 200)).unwrap();
    seg.flush().unwrap();
    let init_bytes = seg.init_segment().unwrap();
    let segments = seg.take_ready_segments();
    let seg1_info = segments
        .into_iter()
        .find(|s| s.segment_seq == 1)
        .expect("segment 1 must have closed");
    let _ = seg.take_ready_parts(); // part target absurdly large: expect exactly the tail part; ignored here, whole-segment path only.

    // A plain (non-LL) playlist: no `low_latency`, segments carry no parts.
    let pl = MediaPlaylist {
        version: 3,
        target_duration: 2,
        media_sequence: 0,
        discontinuity_sequence: 0,
        segments: vec![MediaSegment {
            uri: "seg1.m4s".to_string(),
            duration: seg1_info.duration,
            discontinuous: false,
            parts: vec![],
            byte_range: None,
            map: Some(MapTag {
                uri: INIT_URL.to_string(),
                byte_range: None,
            }),
        }],
        open_segment: None,
        endlist: true,
        extra_tags: vec![],
        low_latency: None,
        iframes_only: false,
        rendition_reports: vec![],
        skip: None,
    };

    let mut client = LlHlsClient::new(PLAYLIST_URL);
    let _ = client.poll();
    client.on_playlist(pl.to_m3u8().as_bytes()).unwrap();
    let actions = drain_actions(&mut client);

    // Non-LL playlist must not request a blocking reload.
    assert!(
        !actions.iter().any(|a| matches!(
            a,
            Action::FetchPlaylist {
                blocking: Some(_),
                ..
            }
        )),
        "non-LL playlist must never request a blocking reload: {actions:#?}"
    );
    // Must request the init + the whole segment (no parts to request).
    assert!(actions.iter().any(|a| matches!(
        a,
        Action::FetchResource {
            id: ResourceId::Init,
            ..
        }
    )));
    assert!(actions.iter().any(|a| matches!(
        a,
        Action::FetchResource { id: ResourceId::Segment { msn: 0 }, url, .. }
            if url == "http://origin/live/seg1.m4s"
    )));

    for a in &actions {
        match a {
            Action::FetchResource {
                id: ResourceId::Init,
                ..
            } => {
                client.on_resource(ResourceId::Init, &init_bytes).unwrap();
            }
            Action::FetchResource {
                id: id @ ResourceId::Segment { msn: 0 },
                ..
            } => {
                client.on_resource(*id, &seg1_info.bytes).unwrap();
            }
            _ => {}
        }
    }

    let outputs = drain_outputs(&mut client);
    let mut got_samples: Vec<Sample> = Vec::new();
    let mut saw_init = false;
    let mut saw_end = false;
    for o in outputs {
        match o {
            Output::Init(bytes) => {
                assert_eq!(bytes, init_bytes);
                saw_init = true;
            }
            Output::Samples { samples, .. } => got_samples.extend(samples),
            Output::EndOfStream => saw_end = true,
            other => panic!("unexpected output: {other:?}"),
        }
    }
    assert!(saw_init, "must emit init even on the fallback path");
    assert!(saw_end, "endlist playlist must emit EndOfStream");
    assert_eq!(got_samples.len(), fed_samples.len());
    for (got, want) in got_samples.iter().zip(fed_samples.iter()) {
        assert_eq!(got.data, want.data);
    }
}
