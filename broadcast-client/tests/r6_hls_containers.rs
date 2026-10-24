mod support;

use broadcast_client::{Config, ErrorKind, Event, SourceKind, Status};
use support::fmp4_split::{rebase_absolute_offsets, split};
use support::hls_host::{Host, hls_config};

const BASE: &str = "http://origin.test/v";
const PLAYLIST_URL: &str = "http://origin.test/v/index.m3u8";
const TINY_BODY_CAP: usize = 16;

fn errors(events: &[Event]) -> Vec<(ErrorKind, bool)> {
    events
        .iter()
        .filter_map(|e| match e {
            Event::Error { kind, fatal } => Some((kind.clone(), *fatal)),
            _ => None,
        })
        .collect()
}

fn samples(events: &[Event]) -> usize {
    events
        .iter()
        .filter(|e| matches!(e, Event::Sample { .. }))
        .count()
}

fn ended(events: &[Event]) -> bool {
    events
        .iter()
        .any(|e| matches!(e, Event::Status(Status::Ended)))
}

fn vod(segments: usize) -> Vec<u8> {
    let mut p = String::from(
        "#EXTM3U\n#EXT-X-VERSION:7\n#EXT-X-TARGETDURATION:10\n#EXT-X-PLAYLIST-TYPE:VOD\n\
         #EXT-X-MAP:URI=\"init.mp4\"\n",
    );
    for i in 0..segments {
        p.push_str(&format!("#EXTINF:1.0,\ns{i}.m4s\n"));
    }
    p.push_str("#EXT-X-ENDLIST\n");
    p.into_bytes()
}

fn play_fragments(fixture: &str, rebase: bool) -> (Vec<Event>, usize) {
    let bytes = test_fixtures::read(fixture);
    let parts = split(&bytes).unwrap();
    let init_len = parts.init.len() as u64;
    let mut h = Host::new(PLAYLIST_URL, hls_config());
    h.ok(PLAYLIST_URL, vod(parts.segments.len()));
    h.ok(&format!("{BASE}/init.mp4"), parts.init);
    let count = parts.segments.len();
    for (i, s) in parts.segments.into_iter().enumerate() {
        let body = if rebase {
            rebase_absolute_offsets(s, init_len)
        } else {
            s
        };
        h.ok(&format!("{BASE}/s{i}.m4s"), body);
    }
    h.run_to_end();
    (h.events, count)
}

fn damaged_reports(events: &[Event]) -> usize {
    errors(events)
        .iter()
        .filter(|(k, fatal)| {
            !*fatal
                && matches!(
                    k,
                    ErrorKind::UnsupportedContainer {
                        reason: "damaged_fragment"
                    }
                )
        })
        .count()
}

const ABSOLUTE_OFFSET_FIXTURES: [&str; 3] = [
    "mp4/prft.mp4",
    "mp4/frag/mpeg2video.frag.mp4",
    "mp4/llcmaf/ll_chunked.mp4",
];

#[test]
fn a_progressive_mp4_served_as_hls_ends_fatal_not_empty() {
    let prog = test_fixtures::read("transmux/h264_aac_prog.mp4");
    let mut h = Host::new(PLAYLIST_URL, hls_config());
    h.ok(PLAYLIST_URL, vod(1));
    h.ok(&format!("{BASE}/init.mp4"), prog.clone());
    h.ok(&format!("{BASE}/s0.m4s"), prog);
    h.run_to_end();
    let errs = errors(&h.events);
    assert!(
        errs.contains(&(
            ErrorKind::UnsupportedContainer {
                reason: "non_fragmented_mp4"
            },
            true
        )),
        "{errs:?}"
    );
    assert!(!ended(&h.events));
    assert_eq!(samples(&h.events), 0);
}

#[test]
fn a_fragment_addressed_outside_its_segment_is_reported_once_and_the_rest_plays() {
    for fixture in ABSOLUTE_OFFSET_FIXTURES {
        let (events, _) = play_fragments(fixture, false);
        assert_eq!(
            damaged_reports(&events),
            1,
            "{fixture}: {:?}",
            errors(&events)
        );
        assert!(ended(&events), "{fixture}");
    }
}

#[test]
fn the_same_fragments_with_resolvable_offsets_report_nothing() {
    for fixture in ABSOLUTE_OFFSET_FIXTURES {
        let (events, _) = play_fragments(fixture, true);
        assert!(
            errors(&events).is_empty(),
            "{fixture}: {:?}",
            errors(&events)
        );
        assert!(samples(&events) > 0, "{fixture}");
    }
}

fn tiny_cap_config(playlist: bool) -> Config {
    let mut c = Config {
        kind: SourceKind::Hls,
        ..hls_config()
    };
    if playlist {
        c.max_playlist_bytes = TINY_BODY_CAP;
    } else {
        c.max_segment_bytes = TINY_BODY_CAP;
    }
    c
}

#[test]
fn an_oversized_playlist_names_the_cap_that_was_hit() {
    let mut h = Host::new(PLAYLIST_URL, tiny_cap_config(true));
    h.ok(PLAYLIST_URL, vod(3));
    h.run_until(|evs| !errors(evs).is_empty(), 200);
    let errs = errors(&h.events);
    assert_eq!(
        errs,
        vec![(
            ErrorKind::BodyTooLarge {
                field: "max_playlist_bytes"
            },
            false
        )]
    );
}

#[test]
fn an_oversized_segment_names_the_cap_that_was_hit_once() {
    let bytes = test_fixtures::read("transmux/h264_aac_frag.mp4");
    let parts = split(&bytes).unwrap();
    let mut h = Host::new(PLAYLIST_URL, tiny_cap_config(false));
    h.ok(PLAYLIST_URL, vod(parts.segments.len()));
    h.ok(&format!("{BASE}/init.mp4"), parts.init);
    for (i, s) in parts.segments.into_iter().enumerate() {
        h.ok(&format!("{BASE}/s{i}.m4s"), s);
    }
    h.run_until(|evs| evs.len() > 40, 400);
    let too_large: Vec<_> = errors(&h.events)
        .into_iter()
        .filter(|(k, _)| matches!(k, ErrorKind::BodyTooLarge { .. }))
        .collect();
    assert_eq!(
        too_large,
        vec![(
            ErrorKind::BodyTooLarge {
                field: "max_segment_bytes"
            },
            false
        )]
    );
}
