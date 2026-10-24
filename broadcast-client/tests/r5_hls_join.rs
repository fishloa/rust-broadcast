mod support;

use broadcast_client::Event;
use support::hls_host::{Host, hls_config};
use support::ts_split::{drop_leading_packets, split_at_pat};
use transmux::{DropReason, JoinDecision, LiveJoin, TrackId};

const PLAYLIST_URL: &str = "http://origin.test/live/index.m3u8";
const SEGMENTS: usize = 8;
const MID_GOP_CUT_PACKETS: usize = 60;
const SOURCE_PREFIX_BYTES: usize = 12_000_000;

fn segment_url(i: usize) -> String {
    format!("http://origin.test/live/seg{i}.ts")
}

fn playlist(segments: usize) -> Vec<u8> {
    let mut p = String::from(
        "#EXTM3U\n#EXT-X-VERSION:3\n#EXT-X-TARGETDURATION:4\n#EXT-X-MEDIA-SEQUENCE:0\n",
    );
    for i in 0..segments {
        p.push_str(&format!("#EXTINF:3.5,\nseg{i}.ts\n"));
    }
    p.push_str("#EXT-X-ENDLIST\n");
    p.into_bytes()
}

fn play(segments: Vec<Vec<u8>>) -> Vec<Event> {
    let mut h = Host::new(PLAYLIST_URL, hls_config());
    h.ok(PLAYLIST_URL, playlist(segments.len()));
    for (i, s) in segments.into_iter().enumerate() {
        h.ok(&segment_url(i), s);
    }
    h.run_to_end();
    h.events
}

fn video_tracks(events: &[Event]) -> Vec<TrackId> {
    let mut out = Vec::new();
    for e in events {
        if let Event::Tracks(specs) = e {
            for s in specs {
                if LiveJoin::for_config(&s.config).is_some() && !out.contains(&s.track_id) {
                    out.push(s.track_id);
                }
            }
        }
    }
    out
}

fn tf1_segments() -> Option<Vec<Vec<u8>>> {
    let bytes = test_fixtures::require_bytes("zenith-http/tf1-live.ts", "r5_hls_join")?;
    let prefix = &bytes[..SOURCE_PREFIX_BYTES.min(bytes.len())];
    let first_sync = (0..188)
        .find(|&o| (0..5).all(|k| prefix.get(o + k * 188) == Some(&0x47)))
        .unwrap();
    let usable = (prefix.len() - first_sync) / 188 * 188;
    Some(split_at_pat(
        &prefix[first_sync..first_sync + usable],
        SEGMENTS,
    ))
}

#[test]
fn a_pat_aligned_hls_ts_stream_opens_with_a_join_point_on_every_video_track() {
    let Some(segments) = tf1_segments() else {
        return;
    };
    let events = play(segments);
    let videos = video_tracks(&events);
    assert!(!videos.is_empty());
    for v in videos {
        let first = events
            .iter()
            .find_map(|e| match e {
                Event::Sample { track, join, .. } if *track == v => Some(*join),
                _ => None,
            })
            .expect("video samples");
        assert!(
            matches!(first, Some(JoinDecision::JoinPoint(_))),
            "first video sample join: {first:?}"
        );
    }
}

#[test]
fn a_mid_gop_hls_join_flags_the_leading_non_random_access_video_as_dropped() {
    let Some(mut segments) = tf1_segments() else {
        return;
    };
    segments[0] = drop_leading_packets(&segments[0], MID_GOP_CUT_PACKETS);
    let events = play(segments);
    let videos = video_tracks(&events);
    assert!(!videos.is_empty());
    for v in videos {
        let joins: Vec<(bool, Option<JoinDecision>)> = events
            .iter()
            .filter_map(|e| match e {
                Event::Sample {
                    track,
                    sample,
                    join,
                } if *track == v => Some((sample.flags.is_sync, *join)),
                _ => None,
            })
            .collect();
        let first_kept = joins
            .iter()
            .position(|(_, j)| !matches!(j, Some(JoinDecision::Drop(_))))
            .expect("a join point exists");
        assert!(
            joins[..first_kept].iter().all(|(sync, j)| !sync
                && *j == Some(JoinDecision::Drop(DropReason::BeforeRandomAccessPoint))),
            "leading samples: {:?}",
            &joins[..first_kept]
        );
        assert!(joins[first_kept].0, "the join point is a sync sample");
        assert!(matches!(
            joins[first_kept].1,
            Some(JoinDecision::JoinPoint(_))
        ));
        assert!(
            joins[first_kept + 1..]
                .iter()
                .all(|(_, j)| *j != Some(JoinDecision::Drop(DropReason::BeforeRandomAccessPoint))),
            "no pre-RAP drops after the join point"
        );
    }
}

#[test]
fn hls_audio_samples_carry_no_join_decision() {
    let Some(segments) = tf1_segments() else {
        return;
    };
    let events = play(segments);
    let videos = video_tracks(&events);
    let audio: Vec<_> = events
        .iter()
        .filter_map(|e| match e {
            Event::Sample { track, join, .. } if !videos.contains(track) => Some(*join),
            _ => None,
        })
        .collect();
    assert!(!audio.is_empty());
    assert!(audio.iter().all(Option::is_none));
}
