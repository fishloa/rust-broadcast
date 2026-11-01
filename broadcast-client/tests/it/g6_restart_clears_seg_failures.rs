use crate::support;

use broadcast_client::{Config, ErrorKind, Event};
use support::hls_host::{Host, Reply, TS_PLAYLIST_URL, TS_SEG0_URL, TS_SEG1_URL, hls_config};

const TS_SEG2_URL: &str = "http://origin.test/hls/index2.ts";

fn live_playlist(dsn: u64) -> Vec<u8> {
    format!(
        "#EXTM3U\n#EXT-X-VERSION:3\n#EXT-X-TARGETDURATION:2\n#EXT-X-MEDIA-SEQUENCE:0\n\
         #EXT-X-DISCONTINUITY-SEQUENCE:{dsn}\n#EXTINF:2.000,\nindex0.ts\n#EXTINF:2.000,\nindex1.ts\n#EXTINF:2.000,\nindex2.ts\n"
    )
    .into_bytes()
}

fn cfg_with_cap(cap: u32) -> Config {
    let mut c = hls_config();
    c.max_segment_failures = cap;
    c
}

fn non_fatal_seg_errors(events: &[Event]) -> usize {
    events
        .iter()
        .filter(|e| {
            matches!(
                e,
                Event::Error {
                    kind: ErrorKind::SegmentFetchFailed { .. },
                    fatal: false
                }
            )
        })
        .count()
}

#[test]
fn g6_restart_clears_seg_failure_count() {
    const RELOADS_BEFORE_RESTART: usize = 1;
    const CAP: u32 = 2;

    // Restart case: playlist DSN drops after several reloads -> restart -> clear_all.
    let mut playlists = vec![Reply::Ok(live_playlist(5)); RELOADS_BEFORE_RESTART];
    playlists.push(Reply::Ok(live_playlist(0)));
    // keep serving the restarted playlist afterwards
    playlists.push(Reply::Ok(live_playlist(0)));

    let mut h = Host::new(TS_PLAYLIST_URL, cfg_with_cap(CAP));
    h.route(TS_PLAYLIST_URL, playlists);
    h.ok(TS_SEG0_URL, support::hls_host::ts_segment(0));
    h.ok(TS_SEG2_URL, support::hls_host::ts_segment(0));
    // bad segment: 3 failures before restart, 1 after, then success
    h.route(
        TS_SEG1_URL,
        vec![
            Reply::Status(404),
            Reply::Status(404),
            Reply::Ok(support::hls_host::ts_segment(1)),
        ],
    );

    h.run_until(|_| false, 2000);

    let restart_hits = h.hits(TS_SEG1_URL);
    let restart_errors = non_fatal_seg_errors(&h.events);
    assert!(
        restart_hits >= 2,
        "restart case should fetch bad seg at least 2 times, got {restart_hits}; events={:?}",
        h.events
    );
    assert_eq!(
        restart_errors, 0,
        "restart case must not give up (count restarted), got {restart_errors}"
    );

    // Control case: no restart -> same 4 failures accumulate -> give up.
    let mut h2 = Host::new(TS_PLAYLIST_URL, cfg_with_cap(CAP));
    h2.ok(TS_PLAYLIST_URL, live_playlist(5));
    h2.ok(TS_SEG0_URL, support::hls_host::ts_segment(0));
    h2.ok(TS_SEG2_URL, support::hls_host::ts_segment(0));
    h2.route(
        TS_SEG1_URL,
        vec![
            Reply::Status(404),
            Reply::Status(404),
            Reply::Ok(support::hls_host::ts_segment(1)),
        ],
    );
    h2.run_until(|_| false, 2000);

    let control_hits = h2.hits(TS_SEG1_URL);
    let control_errors = non_fatal_seg_errors(&h2.events);
    assert_eq!(
        control_hits, 2,
        "control case should give up on 3rd failure (no 4th request), got {control_hits}"
    );
    assert_eq!(
        control_errors, 1,
        "control case must give up (SegmentFetchFailed), got {control_errors}"
    );
}
