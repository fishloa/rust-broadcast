//! Issue #1089 (r09-W12/W13/W16): the sans-IO [`HlsClient`]'s join point,
//! exactly-once delivery, and bounded-state behaviour, driven purely through
//! its public API with hostile playlists.
//!
//! Spec: RFC 8216 §6.3.3 (join no closer than three Target Durations from the
//! end of a live Playlist), §4.3.3.2 (`EXT-X-MEDIA-SEQUENCE`, an unbounded
//! `u64` here).

use hls_runtime::client::{Action, Error, HlsClient, Output, ResourceId};

const URL: &str = "http://origin/live/media.m3u8";

/// A live (no ENDLIST) or VOD playlist of `count` whole segments of `extinf`
/// seconds starting at `first_msn`.
fn playlist(first_msn: u64, count: u64, extinf: &str, endlist: bool) -> String {
    let mut text = format!(
        "#EXTM3U\n#EXT-X-VERSION:3\n#EXT-X-TARGETDURATION:2\n#EXT-X-MEDIA-SEQUENCE:{first_msn}\n"
    );
    for i in 0..count {
        text.push_str(&format!(
            "#EXTINF:{extinf},\nseg{}.ts\n",
            first_msn.wrapping_add(i)
        ));
    }
    if endlist {
        text.push_str("#EXT-X-ENDLIST\n");
    }
    text
}

/// Drain every queued action; return the ids of the `FetchResource` ones.
fn fetched_ids(client: &mut HlsClient) -> Vec<ResourceId> {
    let mut ids = Vec::new();
    while let Some(action) = client.poll() {
        if let Action::FetchResource { id, .. } = action {
            ids.push(id);
        }
    }
    ids
}

fn has(outs: &[Output], f: impl Fn(&Output) -> bool) -> bool {
    outs.iter().any(f)
}

fn segment_ids(msns: impl IntoIterator<Item = u64>) -> Vec<ResourceId> {
    msns.into_iter()
        .map(|msn| ResourceId::Segment { msn })
        .collect()
}

fn drain_outputs(client: &mut HlsClient) -> Vec<Output> {
    std::iter::from_fn(|| client.next_output()).collect()
}

/// Bytes that are neither TS (`0x47`) nor ISOBMFF: with no init segment the
/// client just buffers them, which is all these state tests need.
const OPAQUE: &[u8] = b"opaque-bytes";

#[test]
fn live_join_starts_three_target_durations_from_the_end() {
    // RFC 8216 §6.3.3. TD = 2 s, ten 2 s segments (msn 50..=59): from the end
    // 59 is 2 s behind, 58 is 4 s, 57 is 6 s = 3 TD, so 57 is the earliest
    // segment the client may start at.
    let mut client = HlsClient::new(URL);
    let _ = fetched_ids(&mut client);
    client
        .on_playlist(playlist(50, 10, "2.0", false).as_bytes())
        .unwrap();
    assert_eq!(fetched_ids(&mut client), segment_ids([57, 58, 59]));

    // The skip is a join-time decision only: segments that appear later are
    // all fetched.
    client
        .on_playlist(playlist(50, 12, "2.0", false).as_bytes())
        .unwrap();
    assert_eq!(fetched_ids(&mut client), segment_ids([60, 61]));
}

#[test]
fn vod_playlist_with_endlist_is_played_from_its_start() {
    let mut client = HlsClient::new(URL);
    let _ = fetched_ids(&mut client);
    client
        .on_playlist(playlist(0, 10, "2.0", true).as_bytes())
        .unwrap();
    assert_eq!(fetched_ids(&mut client), segment_ids(0..10));
}

#[test]
fn a_window_shorter_than_three_target_durations_is_played_whole() {
    // 2 x 2 s = 4 s < 6 s: there is no segment starting >= 3 TD from the end,
    // so play from the first one.
    let mut client = HlsClient::new(URL);
    let _ = fetched_ids(&mut client);
    client
        .on_playlist(playlist(7, 2, "2.0", false).as_bytes())
        .unwrap();
    assert_eq!(fetched_ids(&mut client), segment_ids([7, 8]));
}

#[test]
fn hostile_endless_live_window_joins_near_the_end_quickly() {
    // 50 000 segments with a zero EXTINF: a zero duration counts as one Target
    // Duration, so the join is still the last three segments.
    let mut client = HlsClient::new(URL);
    let _ = fetched_ids(&mut client);
    client
        .on_playlist(playlist(1_000, 50_000, "0.0", false).as_bytes())
        .unwrap();
    assert_eq!(
        fetched_ids(&mut client),
        segment_ids([50_997, 50_998, 50_999])
    );
}

#[test]
fn media_sequence_overflow_is_an_error_not_a_panic() {
    for first in [u64::MAX, u64::MAX - 1, u64::MAX - 2] {
        let mut client = HlsClient::new(URL);
        let err = client
            .on_playlist(playlist(first, 3, "2.0", false).as_bytes())
            .expect_err("media_sequence + segments overflows u64");
        assert!(
            matches!(err, Error::MediaSequenceOverflow { media_sequence, segments: 3 }
                if media_sequence == first),
            "wrong error for {first}: {err:?}"
        );
    }
    // The largest playlist that still fits: msn u64::MAX - 3 .. u64::MAX - 1.
    let mut client = HlsClient::new(URL);
    let _ = fetched_ids(&mut client);
    client
        .on_playlist(playlist(u64::MAX - 3, 3, "2.0", true).as_bytes())
        .unwrap();
    assert_eq!(
        fetched_ids(&mut client),
        segment_ids([u64::MAX - 3, u64::MAX - 2, u64::MAX - 1])
    );
}

#[test]
fn regressing_media_sequence_fetches_the_new_numbers() {
    let mut client = HlsClient::new(URL);
    let _ = fetched_ids(&mut client);
    client
        .on_playlist(playlist(100, 2, "2.0", true).as_bytes())
        .unwrap();
    assert_eq!(fetched_ids(&mut client), segment_ids([100, 101]));
    // An origin restart renumbers from 0.
    client
        .on_playlist(playlist(0, 2, "2.0", true).as_bytes())
        .unwrap();
    assert_eq!(fetched_ids(&mut client), segment_ids([0, 1]));
}

/// Audit r09-C3 / #1031: a regression that lands *inside* the previous
/// window re-uses numbers the client already delivered; those segments are
/// new media and must be fetched, behind a `Discontinuity`.
#[test]
fn an_overlapping_media_sequence_regression_refetches_and_signals_a_discontinuity() {
    let mut client = HlsClient::new(URL);
    let _ = fetched_ids(&mut client);
    client
        .on_playlist(playlist(100, 3, "2.0", true).as_bytes())
        .unwrap();
    assert_eq!(fetched_ids(&mut client), segment_ids([100, 101, 102]));
    for msn in [100, 101, 102] {
        client
            .on_resource(ResourceId::Segment { msn }, OPAQUE)
            .unwrap();
    }
    assert!(!has(&drain_outputs(&mut client), |o| matches!(
        o,
        Output::Discontinuity
    )));

    // The origin restarts at 99: 100 and 101 are re-used numbers naming
    // different segments.
    client
        .on_playlist(
            playlist(99, 3, "2.0", true)
                .replace("seg", "restarted")
                .as_bytes(),
        )
        .unwrap();
    assert_eq!(fetched_ids(&mut client), segment_ids([99, 100, 101]));
    let outs = drain_outputs(&mut client);
    assert_eq!(outs.len(), 1);
    assert!(matches!(outs[0], Output::Discontinuity));
}

/// A CDN edge a poll behind another serves an older window of the *same*
/// stream: the overlapping segments are identical, so nothing is re-fetched
/// and no discontinuity is signalled.
#[test]
fn a_lagging_copy_of_the_same_window_is_not_a_restart() {
    let mut client = HlsClient::new(URL);
    let _ = fetched_ids(&mut client);
    client
        .on_playlist(playlist(100, 4, "2.0", true).as_bytes())
        .unwrap();
    assert_eq!(fetched_ids(&mut client).len(), 4);
    client
        .on_playlist(playlist(98, 4, "2.0", true).as_bytes())
        .unwrap();
    // 98 and 99 are new to this client; 100 and 101 are already requested.
    assert_eq!(fetched_ids(&mut client), segment_ids([98, 99]));
    assert!(!has(&drain_outputs(&mut client), |o| matches!(
        o,
        Output::Discontinuity
    )));
}

/// A playlist like [`playlist`] but with an `EXT-X-DISCONTINUITY-SEQUENCE`
/// and an `EXT-X-PROGRAM-DATE-TIME` (`start_ms` + 2 s per segment) before every
/// segment; segment URIs are `{prefix}{msn}.ts`.
fn pdt_playlist(
    first_msn: u64,
    count: u64,
    start_ms: u64,
    dsn: u64,
    prefix: &str,
    endlist: bool,
) -> String {
    let mut text = format!(
        "#EXTM3U\n#EXT-X-VERSION:3\n#EXT-X-TARGETDURATION:2\n#EXT-X-MEDIA-SEQUENCE:{first_msn}\n\
         #EXT-X-DISCONTINUITY-SEQUENCE:{dsn}\n"
    );
    for i in 0..count {
        let ms = start_ms + i * 2_000;
        let (secs, milli) = (ms / 1000, ms % 1000);
        text.push_str(&format!(
            "#EXT-X-PROGRAM-DATE-TIME:2026-10-02T{:02}:{:02}:{:02}.{milli:03}Z\n#EXTINF:2.0,\n{prefix}{}.ts\n",
            secs / 3600 % 24,
            secs / 60 % 60,
            secs % 60,
            first_msn + i
        ));
    }
    if endlist {
        text.push_str("#EXT-X-ENDLIST\n");
    }
    text
}

fn discontinuities(client: &mut HlsClient) -> usize {
    drain_outputs(client)
        .iter()
        .filter(|o| matches!(o, Output::Discontinuity))
        .count()
}

/// Audit r09-C3 (#1031): an edge lagging by a WHOLE window (nothing in
/// common, lower numbers) of a stream that carries `PROGRAM-DATE-TIME` is an
/// older view of the same stream, not a restart: no `Discontinuity`, none of
/// its segments delivered behind what was already delivered, and the client
/// carries on when the edge catches up.
#[test]
fn a_cdn_edge_a_whole_window_behind_is_not_a_restart() {
    let t0 = 10 * 3600 * 1000; // 10:00:00
    let mut client = HlsClient::new(URL);
    let _ = fetched_ids(&mut client);
    client
        .on_playlist(pdt_playlist(100, 4, t0, 0, "seg", true).as_bytes())
        .unwrap();
    assert_eq!(fetched_ids(&mut client), segment_ids(100..104));
    assert_eq!(discontinuities(&mut client), 0);

    // The lagging edge: 90..=93, a whole window (and 20 s) older.
    client
        .on_playlist(pdt_playlist(90, 4, t0 - 20_000, 0, "seg", true).as_bytes())
        .unwrap();
    assert_eq!(fetched_ids(&mut client), Vec::<ResourceId>::new());
    assert_eq!(discontinuities(&mut client), 0);

    // Caught up and one segment on: only the new one is fetched.
    client
        .on_playlist(pdt_playlist(101, 4, t0 + 2_000, 0, "seg", true).as_bytes())
        .unwrap();
    assert_eq!(fetched_ids(&mut client), segment_ids([104]));
    assert_eq!(discontinuities(&mut client), 0);
}

/// Without `PROGRAM-DATE-TIME` or any shared name, a wholly lower window is
/// indistinguishable from a restart and is treated as one (documented
/// residual case).
#[test]
fn a_whole_window_behind_without_pdt_reads_as_a_restart() {
    let mut client = HlsClient::new(URL);
    let _ = fetched_ids(&mut client);
    client
        .on_playlist(playlist(100, 4, "2.0", true).as_bytes())
        .unwrap();
    let _ = fetched_ids(&mut client);
    let _ = drain_outputs(&mut client);
    client
        .on_playlist(
            playlist(90, 4, "2.0", true)
                .replace("seg", "old")
                .as_bytes(),
        )
        .unwrap();
    assert_eq!(fetched_ids(&mut client), segment_ids(90..94));
    assert_eq!(discontinuities(&mut client), 1);
}

/// An origin that restarts to the SAME media sequence number with an
/// identical-looking window is caught by the discontinuity sequence, which may
/// not decrease while the number does not.
#[test]
fn an_equal_msn_restart_with_a_reset_discontinuity_sequence_is_detected() {
    let t0 = 10 * 3600 * 1000;
    let mut client = HlsClient::new(URL);
    let _ = fetched_ids(&mut client);
    client
        .on_playlist(pdt_playlist(10, 3, t0, 5, "seg", true).as_bytes())
        .unwrap();
    assert_eq!(fetched_ids(&mut client), segment_ids(10..13));
    let _ = drain_outputs(&mut client);

    client
        .on_playlist(pdt_playlist(10, 3, t0, 0, "seg", true).as_bytes())
        .unwrap();
    assert_eq!(fetched_ids(&mut client), segment_ids(10..13));
    assert_eq!(discontinuities(&mut client), 1);
}

/// The same numbers and names, but the wall-clock time of a shared segment
/// jumped: a restart (the media behind the name changed).
#[test]
fn a_pdt_jump_at_a_shared_number_is_a_restart() {
    let t0 = 10 * 3600 * 1000;
    let mut client = HlsClient::new(URL);
    let _ = fetched_ids(&mut client);
    client
        .on_playlist(pdt_playlist(10, 3, t0, 0, "seg", true).as_bytes())
        .unwrap();
    let _ = fetched_ids(&mut client);
    let _ = drain_outputs(&mut client);
    client
        .on_playlist(pdt_playlist(10, 3, t0 + 3_600_000, 0, "seg", true).as_bytes())
        .unwrap();
    assert_eq!(fetched_ids(&mut client), segment_ids(10..13));
    assert_eq!(discontinuities(&mut client), 1);
}

/// An origin restarting to a HIGHER number whose window overlaps the old one
/// but names different segments there is a restart too.
#[test]
fn a_restart_to_a_higher_msn_with_different_segments_is_detected() {
    let mut client = HlsClient::new(URL);
    let _ = fetched_ids(&mut client);
    client
        .on_playlist(playlist(10, 3, "2.0", true).as_bytes())
        .unwrap();
    let _ = fetched_ids(&mut client);
    let _ = drain_outputs(&mut client);
    // 11..=13, but 11 and 12 are different files now.
    let text = playlist(11, 3, "2.0", true).replace("seg", "new");
    client.on_playlist(text.as_bytes()).unwrap();
    assert_eq!(fetched_ids(&mut client), segment_ids(11..14));
    assert_eq!(discontinuities(&mut client), 1);
}

/// A name the previous window listed under another number, from a window with
/// lower numbers: names reused under new numbering are a restart.
#[test]
fn a_reused_name_under_a_lower_number_is_a_restart() {
    let mut client = HlsClient::new(URL);
    let _ = fetched_ids(&mut client);
    client
        .on_playlist(playlist(100, 3, "2.0", true).as_bytes())
        .unwrap();
    let _ = fetched_ids(&mut client);
    let _ = drain_outputs(&mut client);
    // MSN restarts at 0 but the file names keep counting (seg101.ts ...).
    let text = "#EXTM3U\n#EXT-X-VERSION:3\n#EXT-X-TARGETDURATION:2\n#EXT-X-MEDIA-SEQUENCE:0\n\
                #EXTINF:2.0,\nseg101.ts\n#EXTINF:2.0,\nseg102.ts\n#EXT-X-ENDLIST\n";
    client.on_playlist(text.as_bytes()).unwrap();
    assert_eq!(fetched_ids(&mut client), segment_ids([0, 1]));
    assert_eq!(discontinuities(&mut client), 1);
}

/// A reload at the *same* sequence number is the normal live case and is not
/// a restart.
#[test]
fn an_unchanged_or_advancing_media_sequence_is_not_a_restart() {
    let mut client = HlsClient::new(URL);
    let _ = fetched_ids(&mut client);
    client
        .on_playlist(playlist(10, 3, "2.0", false).as_bytes())
        .unwrap();
    let _ = fetched_ids(&mut client);
    client
        .on_playlist(playlist(10, 4, "2.0", false).as_bytes())
        .unwrap();
    client
        .on_playlist(playlist(11, 4, "2.0", false).as_bytes())
        .unwrap();
    assert!(!has(&drain_outputs(&mut client), |o| matches!(
        o,
        Output::Discontinuity
    )));
}

/// A response still in flight for the old numbering must not be delivered as
/// if it were the restarted origin's media, and fetches the caller never
/// polled are withdrawn rather than issued for the dead numbering.
#[test]
fn a_restart_withdraws_old_fetches_and_rejects_stale_responses() {
    let mut client = HlsClient::new(URL);
    let _ = fetched_ids(&mut client);
    client
        .on_playlist(playlist(5000, 3, "2.0", true).as_bytes())
        .unwrap();
    // Not polled: the three fetches for 5000.. are still queued.
    client
        .on_playlist(playlist(1, 2, "2.0", true).as_bytes())
        .unwrap();
    assert_eq!(fetched_ids(&mut client), segment_ids([1, 2]));
    let err = client
        .on_resource(ResourceId::Segment { msn: 5000 }, OPAQUE)
        .unwrap_err();
    assert!(matches!(
        err,
        Error::UnrequestedResource {
            id: ResourceId::Segment { msn: 5000 }
        }
    ));
    // The restarted numbering still delivers and the stream still ends.
    client
        .on_resource(ResourceId::Segment { msn: 1 }, OPAQUE)
        .unwrap();
    client
        .on_resource(ResourceId::Segment { msn: 2 }, OPAQUE)
        .unwrap();
    assert!(has(&drain_outputs(&mut client), |o| matches!(
        o,
        Output::EndOfStream
    )));
}

/// A restart may change the codec configuration: the init is fetched again
/// (and, once delivered, announced again) even from the same URI.
#[test]
fn a_restart_refetches_the_init_segment() {
    let fmp4 = |first: u64| {
        format!(
            "#EXTM3U\n#EXT-X-VERSION:7\n#EXT-X-TARGETDURATION:2\n#EXT-X-MEDIA-SEQUENCE:{first}\n\
             #EXT-X-MAP:URI=\"init.mp4\"\n#EXTINF:2.0,\nseg{first}.m4s\n#EXT-X-ENDLIST\n"
        )
    };
    let mut client = HlsClient::new(URL);
    let _ = fetched_ids(&mut client);
    client.on_playlist(fmp4(7).as_bytes()).unwrap();
    let mut ids = fetched_ids(&mut client);
    ids.sort();
    assert_eq!(ids, vec![ResourceId::Init, ResourceId::Segment { msn: 7 }]);
    client.on_playlist(fmp4(3).as_bytes()).unwrap();
    let mut ids = fetched_ids(&mut client);
    ids.sort();
    assert_eq!(ids, vec![ResourceId::Init, ResourceId::Segment { msn: 3 }]);
}

#[test]
fn a_second_delivery_of_the_same_id_is_rejected_and_does_not_end_the_stream_early() {
    // Two outstanding fetches under ENDLIST. A duplicate delivery of seg 0
    // must not be counted as seg 1's completion.
    let mut client = HlsClient::new(URL);
    let _ = fetched_ids(&mut client);
    client
        .on_playlist(playlist(0, 2, "2.0", true).as_bytes())
        .unwrap();
    assert_eq!(fetched_ids(&mut client), segment_ids([0, 1]));

    client
        .on_resource(ResourceId::Segment { msn: 0 }, OPAQUE)
        .unwrap();
    let err = client
        .on_resource(ResourceId::Segment { msn: 0 }, OPAQUE)
        .expect_err("second delivery of Segment 0 must be rejected");
    assert!(
        matches!(
            err,
            Error::DuplicateResource {
                id: ResourceId::Segment { msn: 0 }
            }
        ),
        "wrong error: {err:?}"
    );
    assert!(
        !drain_outputs(&mut client)
            .iter()
            .any(|o| matches!(o, Output::EndOfStream)),
        "EndOfStream fired while Segment 1 is still outstanding"
    );

    client
        .on_resource(ResourceId::Segment { msn: 1 }, OPAQUE)
        .unwrap();
    let outputs = drain_outputs(&mut client);
    assert!(
        matches!(outputs.as_slice(), [Output::EndOfStream]),
        "{outputs:?}"
    );
}

#[test]
fn state_for_segments_that_left_the_window_is_dropped_but_in_flight_fetches_survive() {
    let mut client = HlsClient::new(URL);
    let _ = fetched_ids(&mut client);
    // Window 0..=1 (a short live window is joined whole). Deliver seg 0,
    // leave seg 1 in flight.
    client
        .on_playlist(playlist(0, 2, "2.0", false).as_bytes())
        .unwrap();
    assert_eq!(fetched_ids(&mut client), segment_ids([0, 1]));
    client
        .on_resource(ResourceId::Segment { msn: 0 }, OPAQUE)
        .unwrap();

    // The window slides far past both.
    client
        .on_playlist(playlist(100, 2, "2.0", false).as_bytes())
        .unwrap();
    let _ = fetched_ids(&mut client);

    // Seg 0 was delivered and left the window: its record is gone, so a late
    // duplicate is "never requested" (it used to be accepted again).
    let err = client
        .on_resource(ResourceId::Segment { msn: 0 }, OPAQUE)
        .expect_err("pruned id must not be accepted again");
    assert!(
        matches!(
            err,
            Error::UnrequestedResource {
                id: ResourceId::Segment { msn: 0 }
            }
        ),
        "wrong error: {err:?}"
    );
    // Seg 1 was still in flight when it left the window: its delivery is
    // still accepted (otherwise the outstanding-fetch count never drains).
    client
        .on_resource(ResourceId::Segment { msn: 1 }, OPAQUE)
        .expect("an in-flight fetch must still be accepted after the window moved");
}

#[test]
fn long_lived_pull_forgets_old_segments() {
    // 20 000 reloads sliding by one segment each; every segment is delivered.
    // A driver that retries an id from long ago must find it forgotten.
    let mut client = HlsClient::new(URL);
    let _ = fetched_ids(&mut client);
    for first in 0..20_000u64 {
        client
            .on_playlist(playlist(first, 2, "2.0", false).as_bytes())
            .unwrap();
        for id in fetched_ids(&mut client) {
            client.on_resource(id, OPAQUE).unwrap();
        }
    }
    let delivered_long_ago = ResourceId::Segment { msn: 5 };
    assert!(matches!(
        client.on_resource(delivered_long_ago, OPAQUE),
        Err(Error::UnrequestedResource { .. })
    ));
    let recent = ResourceId::Segment { msn: 19_999 };
    assert!(matches!(
        client.on_resource(recent, OPAQUE),
        Err(Error::DuplicateResource { .. })
    ));
}

// ---------------------------------------------------------------------------
// r09-W13: join points and byte-range continuity
// ---------------------------------------------------------------------------

fn text_of(lines: &[&str]) -> String {
    let mut t = lines.join("\n");
    t.push('\n');
    t
}

fn fetched_ranges(client: &mut HlsClient) -> Vec<(ResourceId, Option<(u64, u64)>)> {
    let mut out = Vec::new();
    while let Some(action) = client.poll() {
        if let Action::FetchResource { id, byte_range, .. } = action {
            out.push((id, byte_range));
        }
    }
    out
}

fn fixture(rel: &str) -> String {
    let path = format!("{}/tests/fixtures/{rel}", env!("CARGO_MANIFEST_DIR"));
    std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("read {path}: {e}"))
}

/// ffmpeg's single-file byte-range playlist (7 segments of `index.ts`, 14 s,
/// TARGETDURATION 3) as a live window with every offset after the first
/// omitted — RFC 8216 §4.3.2.2 makes each continue the previous range.
fn live_byterange_playlist_with_omitted_offsets() -> String {
    let mut out = Vec::new();
    for line in fixture("byterange-hls/index.m3u8").lines() {
        if line.starts_with("#EXT-X-ENDLIST") || line.starts_with("#EXT-X-PLAYLIST-TYPE") {
            continue;
        }
        match line.strip_prefix("#EXT-X-BYTERANGE:") {
            // Keep the first offset (there is no previous range to continue).
            Some(range) if !range.ends_with("@0") => {
                out.push(format!(
                    "#EXT-X-BYTERANGE:{}",
                    range.split('@').next().unwrap()
                ));
            }
            _ => out.push(line.to_string()),
        }
    }
    let mut text = out.join("\n");
    text.push('\n');
    text
}

#[test]
fn skipped_segments_advance_the_byte_range_cursor() {
    // Live, TARGETDURATION 3: the join is the third segment (2.0 + 2.04 are
    // skipped). ffmpeg's own offset for that segment is 91932 =
    // 47752 + 44180, the two skipped lengths.
    let mut client = HlsClient::new(URL);
    let _ = fetched_ranges(&mut client);
    client
        .on_playlist(live_byterange_playlist_with_omitted_offsets().as_bytes())
        .unwrap();
    let ranges = fetched_ranges(&mut client);
    assert_eq!(
        ranges[0],
        (ResourceId::Segment { msn: 2 }, Some((91932, 68056)))
    );
    // The whole fetch list equals ffmpeg's explicit-offset ranges from msn 2.
    assert_eq!(
        ranges,
        vec![
            (ResourceId::Segment { msn: 2 }, Some((91932, 68056))),
            (ResourceId::Segment { msn: 3 }, Some((159988, 24064))),
            (ResourceId::Segment { msn: 4 }, Some((184052, 44180))),
            (ResourceId::Segment { msn: 5 }, Some((228232, 68056))),
            (ResourceId::Segment { msn: 6 }, Some((296288, 22184))),
        ]
    );
}

#[test]
fn hold_back_larger_than_three_target_durations_moves_the_join_back() {
    // TD 2 s, ten 2 s segments (msn 50..=59), HOLD-BACK=10: segments 55..=59
    // are the first 10 s from the end.
    let mut lines = vec![
        "#EXTM3U",
        "#EXT-X-VERSION:3",
        "#EXT-X-TARGETDURATION:2",
        "#EXT-X-SERVER-CONTROL:HOLD-BACK=10",
        "#EXT-X-MEDIA-SEQUENCE:50",
    ];
    let segs: Vec<String> = (50..60)
        .map(|n| format!("#EXTINF:2.0,\nseg{n}.ts"))
        .collect();
    lines.extend(segs.iter().map(String::as_str));
    let mut client = HlsClient::new(URL);
    let _ = fetched_ids(&mut client);
    client.on_playlist(text_of(&lines).as_bytes()).unwrap();
    assert_eq!(fetched_ids(&mut client), segment_ids(55..60));
}

fn ten_segment_playlist(extra: &[&str], endlist: bool) -> String {
    let mut lines = vec![
        "#EXTM3U".to_string(),
        "#EXT-X-VERSION:3".to_string(),
        "#EXT-X-TARGETDURATION:2".to_string(),
    ];
    lines.extend(extra.iter().map(|l| l.to_string()));
    lines.push("#EXT-X-MEDIA-SEQUENCE:0".to_string());
    lines.extend((0..10).map(|n| format!("#EXTINF:2.0,\nseg{n}.ts")));
    if endlist {
        lines.push("#EXT-X-ENDLIST".to_string());
    }
    let mut t = lines.join("\n");
    t.push('\n');
    t
}

#[test]
fn ext_x_start_chooses_the_join_segment() {
    let join = |extra: &[&str], endlist: bool| {
        let mut client = HlsClient::new(URL);
        let _ = fetched_ids(&mut client);
        client
            .on_playlist(ten_segment_playlist(extra, endlist).as_bytes())
            .unwrap();
        fetched_ids(&mut client)
    };
    // Live, 20 s long, hold-back index 7 (6 s from the end).
    // Negative offset from the end: 20 - 12 = 8 s -> the segment covering
    // 8..10 s is msn 4.
    assert_eq!(
        join(&["#EXT-X-START:TIME-OFFSET=-12.0"], false),
        segment_ids(4..10)
    );
    // Positive offset from the start: 4 s -> the segment covering 4..6 s.
    assert_eq!(
        join(&["#EXT-X-START:TIME-OFFSET=4.0,PRECISE=YES"], false),
        segment_ids(2..10)
    );
    // A live start closer to the end than the hold-back allows is pulled back
    // to it (RFC 8216bis §4.4.2.2: SHOULD NOT be within 3 TD of the end).
    assert_eq!(
        join(&["#EXT-X-START:TIME-OFFSET=-2.0"], false),
        segment_ids(7..10)
    );
    // VOD honours it as given; a start beyond the end is the last segment.
    assert_eq!(
        join(&["#EXT-X-START:TIME-OFFSET=6.0"], true),
        segment_ids(3..10)
    );
    assert_eq!(
        join(&["#EXT-X-START:TIME-OFFSET=500.0"], true),
        segment_ids(9..10)
    );
    // A negative start beyond the beginning is the first segment.
    assert_eq!(
        join(&["#EXT-X-START:TIME-OFFSET=-500.0"], true),
        segment_ids(0..10)
    );
}

fn low_latency_playlist(can_block_reload: &str, open_parts: u32) -> String {
    let mut lines = vec![
        "#EXTM3U".to_string(),
        "#EXT-X-VERSION:9".to_string(),
        "#EXT-X-TARGETDURATION:4".to_string(),
        format!("#EXT-X-SERVER-CONTROL:CAN-BLOCK-RELOAD={can_block_reload},PART-HOLD-BACK=1.5"),
        "#EXT-X-PART-INF:PART-TARGET=0.5".to_string(),
        "#EXT-X-MEDIA-SEQUENCE:0".to_string(),
    ];
    lines.extend((0..6).map(|n| format!("#EXTINF:4.0,\nseg{n}.m4s")));
    lines
        .extend((0..open_parts).map(|i| format!("#EXT-X-PART:DURATION=0.5,URI=\"part6.{i}.m4s\"")));
    let mut t = lines.join("\n");
    t.push('\n');
    t
}

#[test]
fn low_latency_join_honours_part_hold_back_counting_open_parts() {
    let join = |can_block: &str, open_parts: u32| {
        let mut client = HlsClient::new(URL);
        let _ = fetched_ids(&mut client);
        client
            .on_playlist(low_latency_playlist(can_block, open_parts).as_bytes())
            .unwrap();
        fetched_ids(&mut client)
    };
    let part = |part| ResourceId::Part { msn: 6, part };
    // PART-HOLD-BACK 1.5 s, 1.0 s of open parts: the open segment is not yet
    // 1.5 s from the end, so the latest allowed start is the last closed one.
    assert_eq!(
        join("YES", 2),
        vec![ResourceId::Segment { msn: 5 }, part(0), part(1)]
    );
    // 1.5 s of open parts reach the hold-back: play from the open segment.
    assert_eq!(join("YES", 3), vec![part(0), part(1), part(2)]);
    // Not in Low-Latency Mode (no blocking reload): HOLD-BACK / 3 TD (12 s)
    // applies: closed segments 3, 4, 5 are 12 s.
    assert_eq!(
        join("NO", 2),
        vec![
            ResourceId::Segment { msn: 3 },
            ResourceId::Segment { msn: 4 },
            ResourceId::Segment { msn: 5 },
            part(0),
            part(1)
        ]
    );
}

#[test]
fn hostile_start_and_hold_back_values_do_not_panic() {
    let mut client = HlsClient::new(URL);
    let _ = fetched_ids(&mut client);
    // Huge TIME-OFFSET and HOLD-BACK on a small window: everything is clamped
    // to the segment range.
    let text = ten_segment_playlist(
        &[
            "#EXT-X-SERVER-CONTROL:HOLD-BACK=99999999999999999999999999",
            "#EXT-X-START:TIME-OFFSET=-99999999999999999999999999",
        ],
        false,
    );
    client.on_playlist(text.as_bytes()).unwrap();
    // HOLD-BACK larger than the whole window: nothing is far enough from the
    // end, so the hold-back index is the first segment; the start tag (also
    // before the first) agrees.
    assert_eq!(fetched_ids(&mut client), segment_ids(0..10));
}

#[test]
fn regression_after_delivery_neither_refetches_nor_duplicates() {
    // r09-C3 shape: MSN 10 delivered, the origin then reports 9..=10 (a
    // regression of the first sequence number, 10 reused). 10 is already
    // delivered, so only 9 is fetched; the samples are the union, once.
    let ts = |name: &str| {
        std::fs::read(format!(
            "{}/tests/fixtures/ts-hls/{name}",
            env!("CARGO_MANIFEST_DIR")
        ))
        .unwrap()
    };
    let live = "#EXTM3U\n#EXT-X-VERSION:3\n#EXT-X-TARGETDURATION:2\n\
                #EXT-X-MEDIA-SEQUENCE:10\n#EXTINF:2.0,\nindex0.ts\n";
    let regressed = "#EXTM3U\n#EXT-X-VERSION:3\n#EXT-X-TARGETDURATION:2\n\
                     #EXT-X-MEDIA-SEQUENCE:9\n#EXTINF:1.0,\nindex1.ts\n#EXTINF:2.0,\nindex0.ts\n\
                     #EXT-X-ENDLIST\n";
    let mut client = HlsClient::new(URL);
    let _ = fetched_ids(&mut client);
    client.on_playlist(live.as_bytes()).unwrap();
    assert_eq!(fetched_ids(&mut client), segment_ids([10]));
    client
        .on_resource(ResourceId::Segment { msn: 10 }, &ts("index0.ts"))
        .unwrap();
    let _ = fetched_ids(&mut client);

    client.on_playlist(regressed.as_bytes()).unwrap();
    assert_eq!(fetched_ids(&mut client), segment_ids([9]));
    client
        .on_resource(ResourceId::Segment { msn: 9 }, &ts("index1.ts"))
        .unwrap();

    // ffprobe -count_packets: index0.ts = 50 video + 84 audio packets,
    // index1.ts = 25 + 47.
    let mut samples = 0;
    let mut ended = false;
    while let Some(output) = client.next_output() {
        match output {
            Output::Samples { samples: s, .. } => samples += s.len(),
            Output::EndOfStream => ended = true,
            _ => {}
        }
    }
    assert!(ended);
    assert_eq!(samples, 134 + 72);
}
