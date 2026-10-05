//! Issue #1089 (r09-W10/W11/W17): the sans-IO [`HlsOrigin`]'s part-request
//! bounds and part retention, its handling of sequence gaps/lag, and its
//! master playlist, driven through the public `ServedEgress` API over a real
//! `media_plane::Trunk`.
//!
//! Spec: RFC 8216bis §6.2.2 (live playlist rules), §6.2.5.2 (blocking reload,
//! Advance Part Limit), §6.2.6 (preload hints / 404), RFC 8216 §4.3.4.2
//! (`BANDWIDTH`/`CODECS`).
//!
//! Independent oracles: Apple `mediastreamvalidator` over the playlists/files
//! the origin renders, and `MP4Box`'s RFC 6381 reading of the fixture init
//! segment (see `tests/fixtures/cmaf-fmp4/PROVENANCE.md`). A test that needs
//! the validator skips loudly when it is absent.

use std::num::NonZeroUsize;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Arc;
use std::time::Duration;

use broadcast_common::Timestamp;
use bytes::Bytes;
use hls_runtime::server::{
    BlockingQuery, DEFAULT_TRACK_ID, HlsBody, HlsOrigin, HlsOriginBuildError, HlsRequest,
};
use media_plane::egress::{AwaitPolicy, CachePolicy, EgressResponse, ServedEgress};
use media_plane::trunk::{PartEntry, SegmentEntry, SegmentWriter, Trunk, TrunkConfig};
use transmux::SegmentMeta;

use test_bounded::output_bounded;

/// The validator's own `-t` timeout, seconds.
const VALIDATOR_TIMEOUT_SECS: u64 = 3;
/// Hard deadline for a `--version` availability probe.
const PROBE_DEADLINE: Duration = Duration::from_secs(10);
/// Margin added to the validator's own `-t` timeout to form the hard kill
/// deadline (a tool that overruns it is killed and the test fails loudly).
const VALIDATOR_MARGIN: Duration = Duration::from_secs(30);

fn nz(n: usize) -> NonZeroUsize {
    NonZeroUsize::new(n).expect("non-zero")
}

fn trunk_with(segment_cap: usize, part_cap: usize) -> (Arc<Trunk>, SegmentWriter) {
    let trunk = Trunk::new(TrunkConfig::new(
        nz(64),
        nz(8),
        nz(segment_cap),
        nz(8),
        nz(part_cap),
    ));
    let writer = trunk.segment_writer().expect("first segment writer");
    (trunk, writer)
}

fn origin(trunk: &Arc<Trunk>, window: usize, low_latency: bool) -> HlsOrigin {
    let mut b = HlsOrigin::builder(Arc::clone(trunk))
        .target_duration_secs(4.0)
        .window_segments(nz(window));
    if low_latency {
        b = b.low_latency(500);
    }
    b.instance(7).build().expect("origin builds")
}

fn publish_seg(w: &SegmentWriter, seq: u32, secs: u64, discontinuous: bool) {
    publish_seg_bytes(
        w,
        seq,
        secs,
        discontinuous,
        vec![u8::try_from(seq).unwrap(); 8],
    );
}

fn publish_seg_bytes(w: &SegmentWriter, seq: u32, secs: u64, discontinuous: bool, bytes: Vec<u8>) {
    w.publish_segment(SegmentEntry::new(
        Bytes::from(bytes),
        seq,
        Duration::from_secs(secs),
        Timestamp::from_nanos(0),
        SegmentMeta { discontinuous },
    ))
    .expect("monotonic sequence number");
}

fn publish_part(w: &SegmentWriter, seg: u32, idx: u32) {
    w.publish_part(PartEntry::new(
        Bytes::from(vec![u8::try_from(idx).unwrap(); 4]),
        seg,
        idx,
        Duration::from_millis(500),
        idx == 0,
    ));
}

/// A deadline far in the future so `Await` is distinguishable from the
/// expiry `NotFound`.
fn patient() -> (Timestamp, AwaitPolicy) {
    (
        Timestamp::from_nanos(1),
        AwaitPolicy::new(Timestamp::from_nanos(60_000_000_000)),
    )
}

fn get(origin: &HlsOrigin, name: &str) -> EgressResponse<HlsBody> {
    let (now, policy) = patient();
    origin.resolve(
        HlsRequest::Resource {
            name: name.to_string(),
        },
        now,
        policy,
    )
}

fn playlist(origin: &HlsOrigin) -> String {
    let (now, policy) = patient();
    match origin.resolve(
        HlsRequest::Playlist {
            track_id: DEFAULT_TRACK_ID,
            query: BlockingQuery::default(),
        },
        now,
        policy,
    ) {
        EgressResponse::Ready {
            body: HlsBody::Playlist(m),
            ..
        } => m,
        other => panic!("expected a playlist, got {other:?}"),
    }
}

fn is_await(r: &EgressResponse<HlsBody>) -> bool {
    matches!(r, EgressResponse::Await { .. })
}

/// The playlist's lines without the computed `EXT-X-VERSION` (not under test).
fn body_lines(body: &str) -> Vec<&str> {
    body.lines()
        .filter(|l| !l.starts_with("#EXT-X-VERSION"))
        .collect()
}

// ---------------------------------------------------------------------------
// r09-W10: part requests
// ---------------------------------------------------------------------------

#[test]
fn a_part_far_beyond_the_live_edge_is_not_found_immediately() {
    let (trunk, w) = trunk_with(8, 64);
    let o = origin(&trunk, 4, true);
    publish_seg(&w, 1, 4, false);
    publish_part(&w, 2, 0); // live edge = segment 2, one part so far

    // Hinted next part of the live-edge segment and first part of the one
    // after it: held open.
    assert!(is_await(&get(&o, "part-1-7-2.1.m4s")));
    assert!(is_await(&get(&o, "part-1-7-3.0.m4s")));
    // A hostile far-future request: refused at once, not parked for the
    // blocking timeout.
    assert_eq!(
        get(&o, "part-1-7-4294967295.0.m4s"),
        EgressResponse::NotFound
    );
    assert_eq!(get(&o, "part-1-7-4.0.m4s"), EgressResponse::NotFound);
    assert_eq!(
        get(&o, "part-1-7-3.4294967295.m4s"),
        EgressResponse::NotFound
    );
}

#[test]
fn only_the_hinted_next_part_is_held_open() {
    // One part (index 0) exists in segment 1, so index 1 is the hinted next
    // part; index 0 is served, index 2 and beyond were never hinted.
    let (trunk, w) = trunk_with(8, 64);
    let o = origin(&trunk, 4, true);
    publish_part(&w, 1, 0);
    assert!(matches!(
        get(&o, "part-1-7-1.0.m4s"),
        EgressResponse::Ready { .. }
    ));
    assert!(is_await(&get(&o, "part-1-7-1.1.m4s")));
    assert_eq!(get(&o, "part-1-7-1.2.m4s"), EgressResponse::NotFound);
    assert_eq!(get(&o, "part-1-7-1.7.m4s"), EgressResponse::NotFound);
    // The segment after the open one: only its first part.
    assert!(is_await(&get(&o, "part-1-7-2.0.m4s")));
    assert_eq!(get(&o, "part-1-7-2.1.m4s"), EgressResponse::NotFound);
}

#[test]
fn an_evicted_part_of_the_open_segment_is_not_found_not_parked() {
    // A part ring of 2 holds only parts 1 and 2 of segment 1 after three
    // parts: index 0 is gone for good, index 3 is the next hinted one.
    let (trunk, w) = trunk_with(8, 2);
    let o = origin(&trunk, 4, true);
    publish_part(&w, 1, 0);
    publish_part(&w, 1, 1);
    publish_part(&w, 1, 2);
    assert_eq!(get(&o, "part-1-7-1.0.m4s"), EgressResponse::NotFound);
    assert!(matches!(
        get(&o, "part-1-7-1.2.m4s"),
        EgressResponse::Ready { .. }
    ));
    assert!(is_await(&get(&o, "part-1-7-1.3.m4s")));
}

fn playlist_request(o: &HlsOrigin, msn: u64, part: Option<u32>) -> EgressResponse<HlsBody> {
    let (now, policy) = patient();
    o.resolve(
        HlsRequest::Playlist {
            track_id: DEFAULT_TRACK_ID,
            query: BlockingQuery {
                hls_msn: Some(msn),
                hls_part: part,
            },
        },
        now,
        policy,
    )
}

#[test]
fn playlist_part_beyond_the_advance_part_limit_is_a_bad_request() {
    // RFC 8216bis §6.2.5.2: Advance Part Limit = 3 / 0.5 s = 6. One part
    // exists in the live-edge segment 1 (last index 0): `_HLS_part` 6 is
    // within the limit and parks, 7 exceeds it.
    let (trunk, w) = trunk_with(8, 64);
    let o = origin(&trunk, 4, true);
    publish_part(&w, 1, 0);
    assert!(is_await(&playlist_request(&o, 1, Some(6))));
    assert!(matches!(
        playlist_request(&o, 1, Some(7)),
        EgressResponse::BadRequest { .. }
    ));
    assert!(matches!(
        playlist_request(&o, 1, Some(u32::MAX)),
        EgressResponse::BadRequest { .. }
    ));
    // The segment after the live edge has no parts yet (last index 0).
    assert!(is_await(&playlist_request(&o, 2, Some(6))));
    assert!(matches!(
        playlist_request(&o, 2, Some(7)),
        EgressResponse::BadRequest { .. }
    ));
    // A part of a segment already passed is satisfied, whatever the number.
    publish_seg(&w, 1, 4, false);
    publish_part(&w, 2, 0);
    assert!(matches!(
        playlist_request(&o, 1, Some(u32::MAX)),
        EgressResponse::Ready { .. }
    ));
}

#[test]
fn a_classic_origin_has_no_parts_to_wait_for() {
    let (trunk, w) = trunk_with(8, 64);
    let o = origin(&trunk, 4, false);
    publish_seg(&w, 1, 4, false);
    assert_eq!(get(&o, "part-1-7-2.0.m4s"), EgressResponse::NotFound);
}

// ---------------------------------------------------------------------------
// r09-W10: parts stay in the playlist after their segment closes
// ---------------------------------------------------------------------------

fn publish_segments_with_parts(w: &SegmentWriter, upto: u32) {
    for seq in 1..=upto {
        publish_part(w, seq, 0);
        publish_part(w, seq, 1);
        publish_seg(w, seq, 4, false);
    }
}

fn part_lines(body: &str) -> Vec<String> {
    body.lines()
        .filter_map(|l| {
            let uri = l.strip_prefix("#EXT-X-PART:")?.split("URI=\"").nth(1)?;
            Some(
                uri.trim_end_matches(|c| c != 's')
                    .trim_end_matches('"')
                    .to_string(),
            )
        })
        .collect()
}

#[test]
fn closed_segments_keep_their_parts_for_three_target_durations() {
    // Target duration 4 s => 12 s. Six closed 4 s segments, no open segment:
    // segments 6,5,4,3 end 0,4,8,12 s before the end (<= 12), 2 and 1 end 16
    // and 20 s before it.
    let (trunk, w) = trunk_with(8, 64);
    let o = origin(&trunk, 6, true);
    publish_segments_with_parts(&w, 6);
    let body = playlist(&o);
    assert_eq!(
        part_lines(&body),
        vec![
            "part-1-7-3.0.m4s",
            "part-1-7-3.1.m4s",
            "part-1-7-4.0.m4s",
            "part-1-7-4.1.m4s",
            "part-1-7-5.0.m4s",
            "part-1-7-5.1.m4s",
            "part-1-7-6.0.m4s",
            "part-1-7-6.1.m4s",
        ],
        "{body}"
    );
    // The parts of segment 3 sit directly before segment 3's EXTINF/URI.
    let lines = body_lines(&body);
    let at = lines.iter().position(|l| *l == "seg-1-7-3.m4s").unwrap();
    assert!(lines[at - 1].starts_with("#EXTINF:"), "{body}");
    assert!(lines[at - 2].contains("part-1-7-3.1.m4s"), "{body}");
    assert!(lines[at - 3].contains("part-1-7-3.0.m4s"), "{body}");
    // ... and the older segments carry none.
    let at1 = lines.iter().position(|l| *l == "seg-1-7-1.m4s").unwrap();
    assert!(lines[at1 - 1].starts_with("#EXTINF:"));
    assert!(!lines[at1 - 2].starts_with("#EXT-X-PART"), "{body}");
}

#[test]
fn parts_the_ring_no_longer_holds_are_not_advertised() {
    // A part ring of 5 keeps only 4.1, 5.0, 5.1, 6.0, 6.1: segment 4's parts
    // are incomplete, so they must not be promised (RFC 8216bis §6.2.2: the
    // client MUST be able to download a listed part).
    let (trunk, w) = trunk_with(8, 5);
    let o = origin(&trunk, 6, true);
    publish_segments_with_parts(&w, 6);
    let body = playlist(&o);
    assert_eq!(
        part_lines(&body),
        vec![
            "part-1-7-5.0.m4s",
            "part-1-7-5.1.m4s",
            "part-1-7-6.0.m4s",
            "part-1-7-6.1.m4s",
        ],
        "{body}"
    );
    // Everything advertised is actually servable.
    for name in part_lines(&body) {
        assert!(
            matches!(get(&o, &name), EgressResponse::Ready { .. }),
            "{name} advertised but not served"
        );
    }
}

// ---------------------------------------------------------------------------
// r09-W11: gaps, repeats and lag in the sequence
// ---------------------------------------------------------------------------

#[test]
fn a_skipped_sequence_number_restarts_the_window_with_a_discontinuity() {
    let (trunk, w) = trunk_with(8, 64);
    let o = origin(&trunk, 6, false);
    publish_seg(&w, 1, 4, false);
    publish_seg(&w, 2, 4, true);
    publish_seg(&w, 3, 4, false);
    assert!(playlist(&o).contains("seg-1-7-1.m4s"));

    // The segmenter skips 4..=8. Appending 9 to [1,2,3] would give it the
    // implied number 4 (EXT-X-MEDIA-SEQUENCE + index).
    publish_seg(&w, 9, 4, false);
    let body = playlist(&o);
    assert_eq!(
        body_lines(&body),
        vec![
            "#EXTM3U",
            "#EXT-X-TARGETDURATION:4",
            "#EXT-X-MEDIA-SEQUENCE:9",
            "#EXT-X-DISCONTINUITY-SEQUENCE:1",
            "#EXT-X-DISCONTINUITY",
            "#EXT-X-MAP:URI=\"init-1-7-1.mp4\"",
            "#EXTINF:4,",
            "seg-1-7-9.m4s",
        ],
        "{body}"
    );
    // The dropped numbers are gone, the new one serves its own bytes.
    assert_eq!(get(&o, "seg-1-7-1.m4s"), EgressResponse::NotFound);
    match get(&o, "seg-1-7-9.m4s") {
        EgressResponse::Ready {
            body: HlsBody::Resource(b),
            ..
        } => assert_eq!(b, Bytes::from(vec![9u8; 8])),
        other => panic!("{other:?}"),
    }
}

#[test]
fn lost_segments_reported_by_the_cursor_mark_a_discontinuity() {
    // A trunk that keeps 2 segments, an origin that does not look until 5
    // have been published: the cursor reports the 3 it lost.
    let (trunk, w) = trunk_with(2, 64);
    let o = origin(&trunk, 4, false);
    for seq in 1..=5 {
        publish_seg(&w, seq, 4, false);
    }
    let body = playlist(&o);
    assert_eq!(
        body_lines(&body),
        vec![
            "#EXTM3U",
            "#EXT-X-TARGETDURATION:4",
            "#EXT-X-MEDIA-SEQUENCE:4",
            "#EXT-X-DISCONTINUITY",
            "#EXT-X-MAP:URI=\"init-1-7-1.mp4\"",
            "#EXTINF:4,",
            "seg-1-7-4.m4s",
            "#EXTINF:4,",
            "seg-1-7-5.m4s",
        ],
        "{body}"
    );
}

#[test]
fn lag_after_a_populated_window_does_not_leave_a_numbering_hole() {
    let (trunk, w) = trunk_with(2, 64);
    let o = origin(&trunk, 4, false);
    publish_seg(&w, 1, 4, false);
    publish_seg(&w, 2, 4, false);
    assert!(playlist(&o).contains("seg-1-7-2.m4s"));
    // 3..=8 published unseen: only 7 and 8 survive in the trunk.
    for seq in 3..=8 {
        publish_seg(&w, seq, 4, false);
    }
    let body = playlist(&o);
    assert_eq!(
        body_lines(&body),
        vec![
            "#EXTM3U",
            "#EXT-X-TARGETDURATION:4",
            "#EXT-X-MEDIA-SEQUENCE:7",
            "#EXT-X-DISCONTINUITY",
            "#EXT-X-MAP:URI=\"init-1-7-1.mp4\"",
            "#EXTINF:4,",
            "seg-1-7-7.m4s",
            "#EXTINF:4,",
            "seg-1-7-8.m4s",
        ],
        "{body}"
    );
    assert_eq!(get(&o, "seg-1-7-2.m4s"), EgressResponse::NotFound);
}

// ---------------------------------------------------------------------------
// r09-W11: continuing the numbering across a new origin
// ---------------------------------------------------------------------------

#[test]
fn media_sequence_offset_continues_the_numbering_everywhere() {
    let (trunk, w) = trunk_with(8, 64);
    let o = HlsOrigin::builder(Arc::clone(&trunk))
        .target_duration_secs(4.0)
        .window_segments(nz(4))
        .low_latency(500)
        .media_sequence_offset(1000)
        .instance(7)
        .build()
        .unwrap();
    publish_seg(&w, 1, 4, false);
    publish_part(&w, 2, 0);
    let body = playlist(&o);
    assert_eq!(
        body_lines(&body),
        vec![
            "#EXTM3U",
            "#EXT-X-TARGETDURATION:4",
            "#EXT-X-MEDIA-SEQUENCE:1001",
            "#EXT-X-SERVER-CONTROL:CAN-BLOCK-RELOAD=YES,PART-HOLD-BACK=1.5",
            "#EXT-X-PART-INF:PART-TARGET=0.5",
            "#EXT-X-MAP:URI=\"init-1-7-1.mp4\"",
            "#EXTINF:4,",
            "seg-1-7-1001.m4s",
            "#EXT-X-PART:DURATION=0.5,URI=\"part-1-7-1002.0.m4s\",INDEPENDENT=YES",
            "#EXT-X-PRELOAD-HINT:TYPE=PART,URI=\"part-1-7-1002.1.m4s\"",
        ],
        "{body}"
    );
    assert!(matches!(
        get(&o, "seg-1-7-1001.m4s"),
        EgressResponse::Ready { .. }
    ));
    assert!(matches!(
        get(&o, "part-1-7-1002.0.m4s"),
        EgressResponse::Ready { .. }
    ));
    // The unshifted numbers do not exist; a number below the offset cannot
    // underflow into one.
    assert_eq!(get(&o, "seg-1-7-1.m4s"), EgressResponse::NotFound);
    assert_eq!(get(&o, "seg-1-7-999.m4s"), EgressResponse::NotFound);
    // `_HLS_msn` is in the shown numbering: segment 1001 is closed, 1002 is
    // the open one.
    let (now, policy) = patient();
    let blocking = |msn, part| {
        o.resolve(
            HlsRequest::Playlist {
                track_id: DEFAULT_TRACK_ID,
                query: BlockingQuery {
                    hls_msn: Some(msn),
                    hls_part: part,
                },
            },
            now,
            policy,
        )
    };
    assert!(matches!(blocking(1001, None), EgressResponse::Ready { .. }));
    assert!(is_await_playlist(&blocking(1002, None)));
    assert!(matches!(
        blocking(1002, Some(0)),
        EgressResponse::Ready { .. }
    ));
    assert!(is_await_playlist(&blocking(1002, Some(1))));
    assert!(matches!(
        blocking(5000, None),
        EgressResponse::BadRequest { .. }
    ));
}

fn is_await_playlist(r: &EgressResponse<HlsBody>) -> bool {
    matches!(r, EgressResponse::Await { .. })
}

#[test]
fn builder_rejects_hostile_configuration() {
    let (trunk, _w) = trunk_with(8, 64);
    let base = || {
        HlsOrigin::builder(Arc::clone(&trunk))
            .target_duration_secs(4.0)
            .window_segments(nz(4))
    };
    assert_eq!(
        base().low_latency(0).build().err(),
        Some(HlsOriginBuildError::ZeroPartTarget)
    );
    assert_eq!(
        base().media_sequence_offset(u64::MAX).build().err(),
        Some(HlsOriginBuildError::MediaSequenceOffsetTooLarge)
    );
    assert_eq!(
        base()
            .media_sequence_offset(u64::MAX - u64::from(u32::MAX) + 1)
            .instance(7)
            .build()
            .err(),
        Some(HlsOriginBuildError::MediaSequenceOffsetTooLarge)
    );
    assert!(
        base()
            .media_sequence_offset(u64::MAX - u64::from(u32::MAX))
            .instance(7)
            .build()
            .is_ok()
    );
}

#[test]
fn a_window_of_one_still_advertises_three_segments() {
    // RFC 8216bis §6.2.2: never shorten a live playlist below 3 target
    // durations; `window_segments(1)` is raised to the 3-segment minimum.
    let (trunk, w) = trunk_with(8, 64);
    let o = origin(&trunk, 1, false);
    for seq in 1..=5 {
        publish_seg(&w, seq, 4, false);
    }
    let body = playlist(&o);
    assert!(body.contains("#EXT-X-MEDIA-SEQUENCE:3\n"), "{body}");
    for seq in 3..=5 {
        assert!(body.contains(&format!("seg-1-7-{seq}.m4s")), "{body}");
    }
    assert!(!body.contains("seg-1-7-2.m4s"), "{body}");
}

// ---------------------------------------------------------------------------
// r09-W17: master playlist CODECS / BANDWIDTH
// ---------------------------------------------------------------------------

fn fixture(name: &str) -> Vec<u8> {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/cmaf-fmp4")
        .join(name);
    std::fs::read(&path).unwrap_or_else(|e| panic!("read {}: {e}", path.display()))
}

fn master(origin: &HlsOrigin) -> String {
    origin
        .master_playlist("media.m3u8")
        .expect("master renders")
}

#[test]
fn master_bandwidth_is_the_measured_peak_rounded_up() {
    let (trunk, w) = trunk_with(8, 64);
    let o = origin(&trunk, 4, false);
    // Before any segment: the documented 5 Mb/s estimate.
    assert_eq!(
        master(&o),
        "#EXTM3U\n#EXT-X-STREAM-INF:BANDWIDTH=5000000\nmedia.m3u8\n"
    );
    // 1_500_000 B in 1 s = 12_000_000 b/s; 500_000 B in 2 s = 2_000_000 b/s;
    // 1001 B in 3 s = 8008 / 3 = 2669.33.. b/s (never reached by the peak).
    publish_seg_bytes(&w, 1, 1, false, vec![0; 1_500_000]);
    publish_seg_bytes(&w, 2, 2, false, vec![0; 500_000]);
    publish_seg_bytes(&w, 3, 3, false, vec![0; 1001]);
    assert_eq!(
        master(&o),
        "#EXTM3U\n#EXT-X-STREAM-INF:BANDWIDTH=12000000\nmedia.m3u8\n"
    );
}

#[test]
fn master_bandwidth_rounds_a_fractional_peak_up() {
    let (trunk, w) = trunk_with(8, 64);
    let o = origin(&trunk, 4, false);
    // 1001 B in 3 s = 8008 / 3 = 2669.33.. b/s -> 2670.
    publish_seg_bytes(&w, 1, 3, false, vec![0; 1001]);
    assert_eq!(
        master(&o),
        "#EXTM3U\n#EXT-X-STREAM-INF:BANDWIDTH=2670\nmedia.m3u8\n"
    );
}

#[test]
fn master_bandwidth_survives_the_peak_segment_leaving_the_window() {
    let (trunk, w) = trunk_with(8, 64);
    let o = origin(&trunk, 3, false);
    publish_seg_bytes(&w, 1, 1, false, vec![0; 1_000_000]); // 8 Mb/s
    for seq in 2..=6 {
        publish_seg_bytes(&w, seq, 1, false, vec![0; 100]);
    }
    assert!(!playlist(&o).contains("seg-1-7-1.m4s"));
    assert_eq!(
        master(&o),
        "#EXTM3U\n#EXT-X-STREAM-INF:BANDWIDTH=8000000\nmedia.m3u8\n"
    );
}

#[test]
fn master_codecs_come_from_the_init_segment() {
    // Oracle: `MP4Box -info init.mp4` -> avc1.4D400D and mp4a.40.2.
    let (trunk, w) = trunk_with(8, 64);
    let o = origin(&trunk, 4, false);
    o.set_init(fixture("init.mp4"));
    publish_seg_bytes(&w, 1, 1, false, fixture("index0.m4s"));
    // 19_924 B in 1 s.
    assert_eq!(
        master(&o),
        "#EXTM3U\n#EXT-X-STREAM-INF:BANDWIDTH=159392,CODECS=\"avc1.4D400D,mp4a.40.2\"\nmedia.m3u8\n"
    );
}

#[test]
fn a_new_unparseable_init_drops_the_previous_codecs() {
    let (trunk, _w) = trunk_with(8, 64);
    let o = origin(&trunk, 4, false);
    o.set_init(fixture("init.mp4"));
    assert!(master(&o).contains("CODECS=\"avc1.4D400D,mp4a.40.2\""));
    o.set_init(vec![0xAA; 16]);
    assert_eq!(
        master(&o),
        "#EXTM3U\n#EXT-X-STREAM-INF:BANDWIDTH=5000000\nmedia.m3u8\n"
    );
}

#[test]
fn next_media_sequence_continues_into_a_replacement_origin() {
    let (trunk, w) = trunk_with(8, 64);
    let first = HlsOrigin::builder(Arc::clone(&trunk))
        .target_duration_secs(4.0)
        .window_segments(nz(4))
        .media_sequence_offset(1000)
        .instance(7)
        .build()
        .unwrap();
    assert_eq!(first.next_media_sequence(), 1001);
    publish_seg(&w, 1, 4, false);
    publish_seg(&w, 2, 4, false);
    assert_eq!(first.next_media_sequence(), 1003);

    // A reconnect: a fresh Trunk restarts at 1, the offset carries on.
    let (trunk2, w2) = trunk_with(8, 64);
    let second = HlsOrigin::builder(Arc::clone(&trunk2))
        .target_duration_secs(4.0)
        .window_segments(nz(4))
        .media_sequence_offset(first.next_media_sequence().saturating_sub(1))
        .instance(7)
        .build()
        .unwrap();
    publish_seg(&w2, 1, 4, false);
    assert!(
        playlist(&second).contains("#EXT-X-MEDIA-SEQUENCE:1003\n"),
        "{}",
        playlist(&second)
    );
    assert_eq!(second.next_media_sequence(), 1004);
}

#[test]
fn unparseable_init_leaves_codecs_out_rather_than_guessing() {
    let (trunk, _w) = trunk_with(8, 64);
    let o = origin(&trunk, 4, false);
    o.set_init(vec![0xAA; 16]);
    assert_eq!(
        master(&o),
        "#EXTM3U\n#EXT-X-STREAM-INF:BANDWIDTH=5000000\nmedia.m3u8\n"
    );
}

// ---------------------------------------------------------------------------
// Apple mediastreamvalidator over what the origin renders
// ---------------------------------------------------------------------------

/// Why the validator cannot run here, or `None` when it can: it is an Apple
/// macOS-only tool, so any other OS is an explicit skip, and on macOS a
/// missing binary is the only other reason (never a silent pass).
fn validator_unavailable() -> Option<&'static str> {
    if !cfg!(target_os = "macos") {
        return Some("`mediastreamvalidator` is macOS-only and this is not macOS");
    }
    let present = match output_bounded(
        Command::new("mediastreamvalidator").arg("--version"),
        PROBE_DEADLINE,
    ) {
        Ok(o) => o.status.success(),
        Err(e) if e.kind() == std::io::ErrorKind::TimedOut => panic!("{e}"),
        Err(_) => false,
    };
    (!present).then_some("`mediastreamvalidator` is not on PATH (Additional Tools for Xcode)")
}

fn scratch(name: &str) -> PathBuf {
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../target/hls-runtime-validator-tmp")
        .join(name);
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("scratch dir");
    dir
}

/// Run the validator in `dir` on `entry`; return the MUST-level (requirement
/// level 1) messages it reports, anywhere in its JSON.
fn validator_errors(dir: &Path, entry: &str, extra: &[&str]) -> Vec<String> {
    let out = dir.join("out.json");
    let status = output_bounded(
        Command::new("mediastreamvalidator")
            .current_dir(dir)
            .args(extra)
            .args(["--quiet", "-t", &VALIDATOR_TIMEOUT_SECS.to_string(), "-O"])
            .arg(&out)
            .arg(entry),
        Duration::from_secs(VALIDATOR_TIMEOUT_SECS) + VALIDATOR_MARGIN,
    )
    .expect("run mediastreamvalidator")
    .status;
    assert!(status.success(), "validator exit {status}");
    // The JSON is machine-written with one `"key" : value` per line; compare
    // whitespace-free so the exact spacing does not matter.
    let compact: String = std::fs::read_to_string(&out)
        .expect("validator json")
        .chars()
        .filter(|c| !c.is_whitespace())
        .collect();
    let mut errors = Vec::new();
    // `errorRequirementLevel` 1 marks RFC "MUST"-level findings (see
    // media-doctor's `mediastreamvalidator_oracle` calibration notes). Each
    // finding is one flat `{...}` message object, so the block around the
    // marker is exactly that message: an object boundary on each side, never
    // a fixed window that could reach into a neighbour.
    let key = "\"errorRequirementLevel\":1";
    let mut from = 0;
    while let Some(at) = compact[from..].find(key) {
        let at = from + at;
        let open = compact[..at].rfind('{').expect("message object start");
        let close = at + compact[at..].find('}').expect("message object end") + 1;
        errors.push(compact[open..close].to_string());
        from = close;
    }
    if compact.contains("\"parseFailed\":true") {
        errors.push("parseFailed".to_string());
    }
    errors
}

/// Write the origin's own rendering (master, media playlist, init, every
/// advertised segment) to `dir` the way an HTTP origin's URL space would lay
/// them out, relative URIs resolving against the playlist.
fn dump_origin(o: &HlsOrigin, dir: &Path, segment_names: &[String]) {
    std::fs::write(dir.join("master.m3u8"), master(o)).unwrap();
    std::fs::write(dir.join("media.m3u8"), playlist(o)).unwrap();
    for name in std::iter::once("init-1-7-1.mp4".to_string()).chain(segment_names.iter().cloned()) {
        match get(o, &name) {
            EgressResponse::Ready {
                body: HlsBody::Resource(b),
                ..
            } => std::fs::write(dir.join(&name), b).unwrap(),
            other => panic!("{name}: {other:?}"),
        }
    }
}

#[test]
fn mediastreamvalidator_accepts_the_master_and_media_playlists_of_real_cmaf() {
    if let Some(reason) = validator_unavailable() {
        eprintln!(
            "SKIP origin_hardening validator oracle: {reason}; this test is a no-op \
             result on this host, not real coverage."
        );
        return;
    }
    let (trunk, w) = trunk_with(8, 64);
    let o = origin(&trunk, 4, false);
    o.set_init(fixture("init.mp4"));
    for (i, name) in ["index0.m4s", "index1.m4s", "index2.m4s"]
        .iter()
        .enumerate()
    {
        publish_seg_bytes(&w, u32::try_from(i + 1).unwrap(), 1, false, fixture(name));
    }
    let dir = scratch("cmaf-master");
    let names: Vec<String> = (1..=3).map(|n| format!("seg-1-7-{n}.m4s")).collect();
    dump_origin(&o, &dir, &names);
    // The origin's master advertises exactly what MP4Box reads from the init.
    assert!(
        std::fs::read_to_string(dir.join("master.m3u8"))
            .unwrap()
            .contains("CODECS=\"avc1.4D400D,mp4a.40.2\"")
    );
    let errors = validator_errors(&dir, "master.m3u8", &[]);
    assert!(
        errors.is_empty(),
        "validator MUST-level findings: {errors:#?}"
    );

    // The harness bites: the same tree with a BANDWIDTH below the measured
    // peak is a MUST-level finding ("Measured peak bitrate ... exceeds error
    // tolerance").
    let master_text = std::fs::read_to_string(dir.join("master.m3u8")).unwrap();
    assert!(master_text.contains("BANDWIDTH=165192"), "{master_text}");
    std::fs::write(
        dir.join("low.m3u8"),
        master_text.replace("BANDWIDTH=165192", "BANDWIDTH=1000"),
    )
    .unwrap();
    assert!(
        !validator_errors(&dir, "low.m3u8", &[]).is_empty(),
        "validator must reject an under-declared BANDWIDTH"
    );
}

#[test]
fn mediastreamvalidator_accepts_the_low_latency_playlist_syntax() {
    if let Some(reason) = validator_unavailable() {
        eprintln!(
            "SKIP origin_hardening validator oracle: {reason}; this test is a no-op \
             result on this host, not real coverage."
        );
        return;
    }
    // Playlist text only (`-p`): closed segments carry their parts, the open
    // segment ends in parts + a preload hint, after a sequence gap.
    let (trunk, w) = trunk_with(8, 64);
    let o = origin(&trunk, 6, true);
    publish_segments_with_parts(&w, 4);
    publish_seg(&w, 9, 4, false);
    publish_part(&w, 10, 0);
    let dir = scratch("ll-playlist");
    std::fs::write(dir.join("media.m3u8"), playlist(&o)).unwrap();
    let mut errors = validator_errors(&dir, "media.m3u8", &["--parse-playlist-only"]);
    // Known finding that is not a violation (RFC 8216bis §B.1: a single rendition needs no report): the validator wants an
    // `EXT-X-RENDITION-REPORT` even for a single-rendition low-latency
    // playlist ("No rendition reports in low-latency playlist", status
    // -50125); this origin renders none. Every other MUST-level finding fails.
    errors.retain(|block| !block.contains("\"errorStatusCode\":-50125"));
    assert!(
        errors.is_empty(),
        "validator MUST-level findings: {errors:#?}"
    );
}

// ---------------------------------------------------------------------------
// r09-C2 (#1030): a name served `Immutable` never maps to other bytes
// ---------------------------------------------------------------------------

fn ready(r: EgressResponse<HlsBody>) -> (Bytes, CachePolicy) {
    match r {
        EgressResponse::Ready {
            body: HlsBody::Resource(b),
            cache,
        } => (b, cache),
        other => panic!("expected a ready resource, got {other:?}"),
    }
}

/// Every `EXT-X-MAP` URI in `body`, in order.
fn map_uris(body: &str) -> Vec<&str> {
    body.lines()
        .filter_map(|l| l.strip_prefix("#EXT-X-MAP:URI=\""))
        .map(|l| l.trim_end_matches('"'))
        .collect()
}

#[test]
fn a_changed_init_gets_a_new_name_and_a_discontinuity_and_old_names_keep_their_bytes() {
    let (trunk, w) = trunk_with(8, 64);
    let o = origin(&trunk, 6, false);
    o.set_init(vec![0xA1; 8]);
    publish_seg(&w, 1, 4, false);
    let first = playlist(&o);
    assert_eq!(map_uris(&first), vec!["init-1-7-1.mp4"], "{first}");
    let (a, cache) = ready(get(&o, "init-1-7-1.mp4"));
    assert_eq!((&a[..], cache), (&[0xA1u8; 8][..], CachePolicy::Immutable));

    // A mid-stream codec change: new init, then the first segment cut with it.
    o.set_init(vec![0xB2; 8]);
    publish_seg(&w, 2, 4, false);
    let second = playlist(&o);
    assert_eq!(
        body_lines(&second),
        vec![
            "#EXTM3U",
            "#EXT-X-TARGETDURATION:4",
            "#EXT-X-MEDIA-SEQUENCE:1",
            "#EXT-X-MAP:URI=\"init-1-7-1.mp4\"",
            "#EXTINF:4,",
            "seg-1-7-1.m4s",
            "#EXT-X-DISCONTINUITY",
            "#EXT-X-MAP:URI=\"init-1-7-2.mp4\"",
            "#EXTINF:4,",
            "seg-1-7-2.m4s",
        ],
        "{second}"
    );

    // The name served immutable before the change still means the same bytes;
    // the new generation has its own; the bare name is the current one and
    // may change, so it is not immutable.
    let (a2, cache) = ready(get(&o, "init-1-7-1.mp4"));
    assert_eq!((&a2[..], cache), (&[0xA1u8; 8][..], CachePolicy::Immutable));
    let (b, cache) = ready(get(&o, "init-1-7-2.mp4"));
    assert_eq!((&b[..], cache), (&[0xB2u8; 8][..], CachePolicy::Immutable));
    let (bare, cache) = ready(get(&o, "init-1.mp4"));
    assert_eq!((&bare[..], cache), (&[0xB2u8; 8][..], CachePolicy::NoCache));
}

#[test]
fn setting_the_same_init_again_keeps_the_generation() {
    let (trunk, w) = trunk_with(8, 64);
    let o = origin(&trunk, 6, false);
    o.set_init(vec![0xA1; 8]);
    o.set_init(vec![0xA1; 8]);
    publish_seg(&w, 1, 4, false);
    publish_seg(&w, 2, 4, false);
    let body = playlist(&o);
    assert_eq!(map_uris(&body), vec!["init-1-7-1.mp4"], "{body}");
    assert!(!body.contains("EXT-X-DISCONTINUITY\n"), "{body}");
}

#[test]
fn only_a_bounded_history_of_inits_stays_addressable_and_hostile_names_are_not_found() {
    let (trunk, _w) = trunk_with(8, 64);
    let o = origin(&trunk, 6, false);
    for i in 1..=12u8 {
        o.set_init(vec![i; 8]);
    }
    // Generation 12 is current; the oldest generations rolled off.
    assert_eq!(get(&o, "init-1-7-1.mp4"), EgressResponse::NotFound);
    assert_eq!(get(&o, "init-1-7-4.mp4"), EgressResponse::NotFound);
    let (b, cache) = ready(get(&o, "init-1-7-12.mp4"));
    assert_eq!((&b[..], cache), (&[12u8; 8][..], CachePolicy::Immutable));
    let (b, _) = ready(get(&o, "init-1-7-5.mp4"));
    assert_eq!(&b[..], &[5u8; 8]);
    for hostile in [
        "init-1-.mp4",
        "init--1.mp4",
        "init-1-7-.mp4",
        "init-1-7-99999999999999999999.mp4",
        "init-1-99999999999999999999-1.mp4",
        "init-1-7-0.mp4",
        "init-1-7-12-3.mp4",
        "init-1-7.mp4",
        "init-x-7-1.mp4",
        // Another instance's token never resolves here.
        "init-1-8-12.mp4",
        "seg-1-8-1.m4s",
        "part-1-8-1.0.m4s",
    ] {
        assert_eq!(get(&o, hostile), EgressResponse::NotFound, "{hostile}");
    }
}

#[test]
fn the_current_init_generation_is_announced_before_any_init_is_set() {
    // Nothing set yet: the playlist names generation 1 (what the first
    // `set_init` will serve), never a name that later means other bytes.
    let (trunk, w) = trunk_with(8, 64);
    let o = origin(&trunk, 6, false);
    publish_seg(&w, 1, 4, false);
    assert_eq!(map_uris(&playlist(&o)), vec!["init-1-7-1.mp4"]);
    assert_eq!(get(&o, "init-1-7-1.mp4"), EgressResponse::NotFound);
    o.set_init(vec![0xC3; 8]);
    let (b, _) = ready(get(&o, "init-1-7-1.mp4"));
    assert_eq!(&b[..], &[0xC3u8; 8]);
}

/// A replacement origin over a fresh `Trunk` restarts its numbers at 1: with
/// the documented offset no name the previous origin served names different
/// bytes; without it the names collide (which is why the offset exists).
#[test]
fn a_replacement_origin_with_the_offset_reuses_no_segment_name() {
    let (trunk1, w1) = trunk_with(8, 64);
    let o1 = origin(&trunk1, 6, false);
    for seq in 1..=3 {
        publish_seg(&w1, seq, 4, false);
    }
    let names: Vec<String> = (1..=3).map(|n| format!("seg-1-7-{n}.m4s")).collect();
    let before: Vec<Bytes> = names.iter().map(|n| ready(get(&o1, n)).0).collect();

    let (trunk2, w2) = trunk_with(8, 64);
    let collide = origin(&trunk2, 6, false);
    let o2 = HlsOrigin::builder(Arc::clone(&trunk2))
        .target_duration_secs(4.0)
        .window_segments(nz(6))
        .media_sequence_offset(o1.next_media_sequence().saturating_sub(1))
        .instance(7)
        .build()
        .unwrap();
    publish_seg_bytes(&w2, 1, 4, false, vec![0xEE; 8]);
    // No offset: the old name now serves different bytes.
    assert_ne!(ready(get(&collide, &names[0])).0, before[0]);
    // With the offset: every old name is either gone or the same bytes, and
    // the new segment lives under a number the old origin never used.
    for (name, old) in names.iter().zip(&before) {
        match get(&o2, name) {
            EgressResponse::NotFound => {}
            other => assert_eq!(&ready(other).0, old, "{name}"),
        }
    }
    let (new, cache) = ready(get(&o2, "seg-1-7-4.m4s"));
    assert_eq!(
        (&new[..], cache),
        (&[0xEEu8; 8][..], CachePolicy::Immutable)
    );
}

#[test]
fn a_hostile_target_duration_is_a_build_error() {
    let (trunk, _w) = trunk_with(8, 64);
    for bad in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY, 0.0, -4.0] {
        let result = HlsOrigin::builder(Arc::clone(&trunk))
            .target_duration_secs(bad)
            .window_segments(nz(4))
            .instance(7)
            .build();
        assert!(
            matches!(result, Err(HlsOriginBuildError::InvalidTargetDuration)),
            "{bad} must be rejected"
        );
    }
}

// ---------------------------------------------------------------------------
// r09-C2 (#1030), the instance token: no name served `immutable` ever maps to
// other bytes, across origins, reconnects and restarts
// ---------------------------------------------------------------------------

/// Every resource URI `body` names (init, segments, parts, preload hint).
fn named_uris(body: &str) -> Vec<String> {
    let mut out = Vec::new();
    for line in body.lines() {
        if let Some(rest) = line.strip_prefix("#EXT-X-MAP:URI=\"") {
            out.push(rest.trim_end_matches('"').to_string());
        } else if let Some(at) = line.find("URI=\"") {
            let rest = &line[at + 5..];
            if let Some(end) = rest.find('"') {
                out.push(rest[..end].to_string());
            }
        } else if !line.starts_with('#') && !line.is_empty() {
            out.push(line.to_string());
        }
    }
    out
}

/// An LL origin with default (wall-clock) instance token over a fresh trunk.
fn fresh_origin(offset: u64) -> (Arc<Trunk>, SegmentWriter, HlsOrigin) {
    let (trunk, w) = trunk_with(8, 64);
    let o = HlsOrigin::builder(Arc::clone(&trunk))
        .target_duration_secs(4.0)
        .window_segments(nz(6))
        .low_latency(500)
        .media_sequence_offset(offset)
        .build()
        .expect("origin builds");
    (trunk, w, o)
}

/// What an origin serves `immutable` right now: (name, bytes) for every URI
/// its playlist names that resolves with `CachePolicy::Immutable`.
fn immutable_served(o: &HlsOrigin) -> Vec<(String, Bytes)> {
    let body = playlist(o);
    named_uris(&body)
        .into_iter()
        .filter_map(|name| match get(o, &name) {
            EgressResponse::Ready {
                body: HlsBody::Resource(b),
                cache: CachePolicy::Immutable,
            } => Some((name, b)),
            _ => None,
        })
        .collect()
}

#[test]
fn a_replacement_origin_after_an_init_change_never_reuses_an_immutable_name() {
    // Run 1: init A, segments 1..=2 and the open segment 3 with two parts.
    let (_t1, w1, o1) = fresh_origin(0);
    o1.set_init(vec![0xA1; 8]);
    publish_seg(&w1, 1, 4, false);
    publish_seg(&w1, 2, 4, false);
    publish_part(&w1, 3, 0);
    publish_part(&w1, 3, 1);
    let served1 = immutable_served(&o1);
    assert!(
        served1.iter().any(|(n, _)| n.starts_with("init-"))
            && served1.iter().any(|(n, _)| n.starts_with("seg-"))
            && served1.iter().any(|(n, _)| n.starts_with("part-")),
        "run 1 must have served an init, segments and parts immutably: {served1:?}"
    );

    // Run 2: the source reconnected with a DIFFERENT init, the numbers restart
    // at 1 over a new Trunk; the offset continues the MSN (skipping run 1's
    // open segment).
    let (_t2, w2, o2) = fresh_origin(o1.next_media_sequence());
    o2.set_init(vec![0xB2; 8]);
    publish_seg(&w2, 1, 4, false);
    publish_part(&w2, 2, 0);
    let served2 = immutable_served(&o2);

    assert_ne!(o1.instance(), o2.instance());
    let names1: Vec<&String> = served1.iter().map(|(n, _)| n).collect();
    for (name, _) in &served2 {
        assert!(
            !names1.contains(&name),
            "{name} was served immutably by run 1"
        );
    }
    // And run 2 does not answer for run 1's names at all (a stale cache entry
    // is the only thing that could still hold them).
    for (name, _) in &served1 {
        assert_eq!(get(&o2, name), EgressResponse::NotFound, "{name}");
    }
    // Run 1's open segment's number (its parts were served immutably) is not
    // produced again by run 2 under any form.
    let open_msn = o1.next_media_sequence();
    let body2 = playlist(&o2);
    assert!(
        !body2.contains(&format!("-{open_msn}.")) && !body2.contains(&format!("-{open_msn}\n")),
        "run 2 reused run 1's open segment number {open_msn}: {body2}"
    );
    assert!(
        body2.contains(&format!("#EXT-X-MEDIA-SEQUENCE:{}\n", open_msn + 1)),
        "{body2}"
    );
}

#[test]
fn the_instance_token_follows_the_wall_clock_so_a_restart_changes_every_name() {
    let before_ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_millis();
    let (_t1, w1, a) = fresh_origin(0);
    let (_t2, _w2, b) = fresh_origin(0);
    // Built back to back (possibly in the same millisecond): still distinct.
    assert!(b.instance() > a.instance());
    // Seeded from the clock: a process started later gets a larger token than
    // anything an earlier process handed out, with no persistence involved.
    assert!(u128::from(a.instance()) >= before_ms);
    publish_seg(&w1, 1, 4, false);
    let first = named_uris(&playlist(&a));
    assert!(
        first
            .iter()
            .any(|n| n == &format!("seg-1-{}-1.m4s", a.instance())),
        "{first:?}"
    );
}

#[test]
fn token_less_names_resolve_but_are_never_immutable() {
    let (_t, w, o) = fresh_origin(0);
    publish_seg(&w, 1, 4, false);
    publish_part(&w, 2, 0);
    for legacy in ["seg-1-1.m4s", "part-1-2.0.m4s"] {
        match get(&o, legacy) {
            EgressResponse::Ready { cache, .. } => {
                assert_eq!(cache, CachePolicy::NoCache, "{legacy}")
            }
            other => panic!("{legacy}: {other:?}"),
        }
    }
}
