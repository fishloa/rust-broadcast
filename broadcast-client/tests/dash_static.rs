//! Static (VOD) DASH presentations through a `Session`: the real committed captures play to
//! completion, and the delivered samples equal what `Fmp4Demux` makes of the concatenated init and
//! segments (the oracle never sees the session).

#[path = "support/dash_common.rs"]
mod dash_common;
#[path = "support/dash_sim.rs"]
mod dash_sim;

use broadcast_client::{Config, Event, Status};
use dash_common::loopback::{Sig, oracle};
use dash_sim::{DashSim, SEC, assert_samples_equal_oracle};

const HOST: &str = "http://dash.test";

fn seq(prefix: &str, init: &str, pattern: &str, n: std::ops::RangeInclusive<usize>) -> Vec<String> {
    let mut out = vec![format!("{prefix}{init}")];
    for i in n {
        out.push(format!(
            "{prefix}{}",
            pattern.replace("{n}", &format!("{i}"))
        ));
    }
    out
}

fn seq5(prefix: &str, init: &str, stem: &str, n: std::ops::RangeInclusive<usize>) -> Vec<String> {
    let mut out = vec![format!("{prefix}{init}")];
    for i in n {
        out.push(format!("{prefix}{stem}{i:05}.m4s"));
    }
    out
}

fn seq3(prefix: &str, init: &str, stem: &str, n: std::ops::RangeInclusive<usize>) -> Vec<String> {
    let mut out = vec![format!("{prefix}{init}")];
    for i in n {
        out.push(format!("{prefix}{stem}{i:03}.m4s"));
    }
    out
}

fn want(sets: &[Vec<String>]) -> Vec<Vec<Sig>> {
    sets.iter()
        .flat_map(|files| {
            let refs: Vec<&str> = files.iter().map(String::as_str).collect();
            oracle(&refs)
        })
        .collect()
}

fn ffmpeg(prefix: &str) -> Vec<Vec<String>> {
    vec![
        seq5(prefix, "init-stream0.m4s", "chunk-stream0-", 1..=3),
        seq5(prefix, "init-stream1.m4s", "chunk-stream1-", 1..=4),
    ]
}

fn whole_files(dir: &str) -> Vec<Vec<Sig>> {
    let v = format!("{dir}/v_dashinit.mp4");
    let a = format!("{dir}/a_dashinit.mp4");
    let mut out = oracle(&[v.as_str()]);
    out.extend(oracle(&[a.as_str()]));
    out
}

fn play(mpd: &str) -> DashSim {
    let mut sim = DashSim::static_origin(&format!("{HOST}/{mpd}"), Config::default());
    sim.run_to_terminal(600 * SEC);
    sim
}

fn assert_clean(sim: &DashSim) {
    assert!(sim.errors().is_empty(), "{:?}", sim.errors());
    assert!(sim.ended(), "no Ended; statuses {:?}", sim.statuses());
    assert!(matches!(
        sim.events.last(),
        Some(Event::Status(Status::Ended))
    ));
}

#[test]
fn ffmpeg_timeline_manifest_equals_the_demux_oracle() {
    let sim = play("manifest.mpd");
    assert_clean(&sim);
    assert_samples_equal_oracle(&sim, &want(&ffmpeg("")));
}

#[test]
fn ffmpeg_quirk_timeline_equals_the_demux_oracle() {
    let sim = play("timeline-ffmpeg-quirk/manifest.mpd");
    assert_clean(&sim);
    assert_samples_equal_oracle(&sim, &want(&ffmpeg("timeline-ffmpeg-quirk/")));
}

#[test]
fn gpac_timeline_equals_the_demux_oracle() {
    let sets = vec![
        seq("timeline-gpac/", "seg-1-.mp4", "seg-1-{n}.m4s", 1..=6),
        seq("timeline-gpac/", "seg-2-.mp4", "seg-2-{n}.m4s", 1..=6),
    ];
    let sim = play("timeline-gpac/timeline.mpd");
    assert_clean(&sim);
    assert_samples_equal_oracle(&sim, &want(&sets));
}

#[test]
fn segment_list_equals_the_demux_oracle() {
    let sets = vec![
        seq3("segmentlist/", "init-0.m4s", "seg-0-", 1..=6),
        seq3("segmentlist/", "init-1.m4s", "seg-1-", 1..=6),
    ];
    let sim = play("segmentlist/list.mpd");
    assert_clean(&sim);
    assert_samples_equal_oracle(&sim, &want(&sets));
}

#[test]
fn on_demand_sidx_equals_the_demux_oracle_and_uses_byte_ranges() {
    let sim = play("ondemand-sidx/ondemand.mpd");
    assert_clean(&sim);
    assert_samples_equal_oracle(&sim, &whole_files("ondemand-sidx"));
    let ranged = sim.log.iter().filter(|l| l.byte_range.is_some()).count();
    assert!(
        ranged >= 2 * (1 + 1 + 6),
        "init + index + 6 subsegments per rep: {ranged}"
    );
    assert!(sim.log.iter().all(|l| l.method == "GET"));
}

fn kinds(sim: &DashSim) -> Vec<String> {
    sim.events
        .iter()
        .map(|e| match e {
            Event::Variants(v) => format!("variants({}/{})", v.variants.len(), v.renditions.len()),
            Event::Error { kind, fatal } => {
                format!("error({}{})", kind.name(), if *fatal { "!" } else { "" })
            }
            Event::Status(s) => s.to_string(),
            Event::Tracks(t) => format!("tracks({})", t.len()),
            Event::ClockSynced { .. } => "clock".into(),
            Event::Sample { .. } => "sample".into(),
            Event::Discontinuity => "discontinuity".into(),
            Event::AudioRendition { .. } => "audio".into(),
            _ => "other".into(),
        })
        .collect()
}

fn dedup(names: Vec<String>) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for n in names {
        if out.last() != Some(&n) {
            out.push(n);
        }
    }
    out
}

fn strs(v: &[&str]) -> Vec<String> {
    v.iter().map(|s| (*s).to_owned()).collect()
}

#[test]
fn the_annex_g_examples_without_media_end_in_the_typed_state_each_one_calls_for() {
    let want: [(&str, &[&str]); 9] = [
        (
            "F1",
            &[
                "establishing",
                "error(track_dropped)",
                "error(mpd_invalid!)",
            ],
        ),
        ("F2", &["establishing", "error(clock_unavailable!)"]),
        ("F3", &["establishing", "error(clock_unavailable!)"]),
        (
            "F4",
            &[
                "establishing",
                "error(unsupported_feature)",
                "reconnecting",
                "error(segment_fetch_failed!)",
            ],
        ),
        (
            "F5",
            &[
                "establishing",
                "error(unsupported_feature)",
                "reconnecting",
                "error(segment_fetch_failed!)",
            ],
        ),
        (
            "F6",
            &[
                "establishing",
                "reconnecting",
                "error(segment_fetch_failed!)",
            ],
        ),
        (
            "F7",
            &[
                "establishing",
                "error(unsupported_feature)",
                "error(clock_unavailable!)",
            ],
        ),
        (
            "F8",
            &[
                "establishing",
                "error(unsupported_feature)",
                "error(clock_unavailable!)",
            ],
        ),
        ("F9", &["establishing", "error(clock_unavailable!)"]),
    ];
    for (n, expected) in want {
        let url = format!("{HOST}/annex-g/example-{n}.mpd");
        let mut config = Config::default();
        config.max_segment_failures = 2;
        let mut sim = DashSim::static_origin(&url, config);
        sim.run_to_terminal(120 * SEC);
        assert_eq!(dedup(kinds(&sim)), strs(expected), "{n}");
        assert!(
            sim.fatal().is_some(),
            "{n}: every media-less example ends fatally"
        );
        assert!(
            sim.log.iter().filter(|l| l.is_mpd()).count() == 1,
            "{n}: the MPD is fetched once and never refetched after a fatal"
        );
    }
}

fn period_files(period: u32, track: u32, segments: usize) -> Vec<String> {
    let mut v = vec![format!(
        "multiperiod-restart/p{period}_dash_track{track}_init.mp4"
    )];
    for n in 1..=segments {
        v.push(format!(
            "multiperiod-restart/p{period}_dash_track{track}_{n}.m4s"
        ));
    }
    v
}

fn oracle_of(files: &[String]) -> Vec<Sig> {
    let refs: Vec<&str> = files.iter().map(String::as_str).collect();
    oracle(&refs).remove(0)
}

fn restart_expected() -> Vec<Vec<Sig>> {
    let mut video = oracle_of(&period_files(1, 1, 3));
    video.extend(oracle_of(&period_files(2, 1, 3)));
    let mut audio = oracle_of(&period_files(1, 2, 3));
    audio.extend(oracle_of(&period_files(2, 2, 3)));
    vec![video, audio]
}

#[test]
fn restart_periods_play_in_order_with_one_discontinuity_and_the_tail_404_is_silent() {
    let sim = play("multiperiod-restart/multiperiod.mpd");
    assert_clean(&sim);
    assert_samples_equal_oracle(&sim, &restart_expected());
    let names = kinds(&sim);
    assert_eq!(names.iter().filter(|n| *n == "discontinuity").count(), 1);
    assert_eq!(
        names.iter().filter(|n| n.starts_with("variants")).count(),
        2,
        "one selection per Period"
    );
    let disc = names.iter().position(|n| n == "discontinuity").unwrap();
    let before = names[..disc].iter().filter(|n| *n == "sample").count();
    let total = names.iter().filter(|n| *n == "sample").count();
    assert!(before > 0 && before < total, "samples on both sides");
    for tail in ["p1_dash_track1_4.m4s", "p1_dash_track2_4.m4s"] {
        assert_eq!(
            sim.requests_to(tail).len(),
            1,
            "{tail}: 404 once, never retried"
        );
    }
}

fn continuous_expected() -> Vec<Vec<Sig>> {
    let mut files = vec!["timeline-gpac/seg-1-.mp4".to_owned()];
    for n in 1..=6 {
        files.push(format!("timeline-gpac/seg-1-{n}.m4s"));
    }
    vec![oracle_of(&files)]
}

const CONTINUOUS: &str = "multiperiod-continuous/multiperiod.mpd";

fn play_text(rel: &str, edit: impl Fn(String) -> String) -> DashSim {
    let text = edit(test_fixtures::read_to_string(&format!("dash/{rel}")));
    let url = format!("{HOST}/{rel}");
    let origin = dash_common::loopback::Origin::new().doc(&url, text.into_bytes());
    let mut sim = DashSim::new(
        &url,
        Config::default(),
        dash_common::world::World::new(1_700_000_000 * 1_000_000),
        Box::new(dash_sim::StaticBackend(origin)),
    );
    sim.run_to_terminal(600 * SEC);
    sim
}

#[test]
fn continuous_periods_join_without_a_discontinuity_or_a_second_init() {
    let sim = play(CONTINUOUS);
    assert_clean(&sim);
    assert_samples_equal_oracle(&sim, &continuous_expected());
    let names = kinds(&sim);
    assert_eq!(names.iter().filter(|n| *n == "discontinuity").count(), 0);
    assert_eq!(names.iter().filter(|n| n.starts_with("tracks")).count(), 1);
    assert_eq!(
        sim.requests_to("seg-1-.mp4").len(),
        1,
        "the shared init is fetched once"
    );
}

#[test]
fn without_the_continuity_signal_the_same_boundary_is_a_discontinuity() {
    let sim = play_text(CONTINUOUS, |t| {
        let start = t.find("<SupplementalProperty").unwrap();
        let end = t[start..].find("/>").unwrap() + start + 2;
        format!("{}{}", &t[..start], &t[end..])
    });
    assert_clean(&sim);
    assert_samples_equal_oracle(&sim, &continuous_expected());
    assert_eq!(sim.count(|e| matches!(e, Event::Discontinuity)), 1);
}

fn chunk_run(mpd: &str, chunk: usize) -> Vec<Event> {
    let mut sim = DashSim::static_origin(&format!("{HOST}/{mpd}"), Config::default());
    sim.chunk = chunk;
    sim.run_to_terminal(600 * SEC);
    assert_clean(&sim);
    sim.events
}

#[test]
fn the_event_sequence_does_not_depend_on_how_the_bodies_are_chunked() {
    for mpd in [
        "manifest.mpd",
        "ondemand-sidx/ondemand.mpd",
        "segmentlist/list.mpd",
    ] {
        let reference = chunk_run(mpd, 65_536);
        for chunk in [1usize, 1316] {
            assert_eq!(chunk_run(mpd, chunk), reference, "{mpd} chunk {chunk}");
        }
    }
}

#[test]
fn reconnect_restarts_a_static_presentation_from_a_fresh_mpd_after_a_discontinuity() {
    let mut config = Config::default();
    config.max_inflight_segments = 1;
    let mut sim = DashSim::static_origin(&format!("{HOST}/manifest.mpd"), config);
    while !sim.events.iter().any(|e| matches!(e, Event::Sample { .. })) {
        sim.run_for(10_000);
    }
    let before = sim.events.len();
    sim.session.reconnect(sim.world.mono());
    sim.run_to_terminal(120 * SEC);
    assert_clean(&sim);
    let tail = &sim.events[before..];
    assert!(
        matches!(tail.first(), Some(Event::Discontinuity)),
        "the host is told first: {:?}",
        tail.first()
    );
    assert_eq!(
        sim.log.iter().filter(|l| l.is_mpd()).count(),
        2,
        "a fresh MPD request"
    );
    let second_pass: Vec<_> = tail
        .iter()
        .filter_map(|e| match e {
            Event::Sample { track, sample, .. } if track.value() == 1 => Some(sample.clone()),
            _ => None,
        })
        .collect();
    let want = want(&ffmpeg(""));
    assert_eq!(
        dash_sim::sigs_of(&second_pass),
        want[0],
        "the whole video again, in order"
    );
    assert!(
        tail.iter().any(|e| matches!(e, Event::Variants(_))),
        "the new presentation is announced again"
    );
}

#[test]
fn close_is_terminal_and_closes_every_connection_in_flight_exactly_once() {
    let mut sim = DashSim::static_origin(&format!("{HOST}/manifest.mpd"), Config::default());
    while sim.log.len() < 6 {
        sim.run_for(5_000);
    }
    sim.session.close();
    let mut closed = Vec::new();
    while let Some(a) = sim.session.poll(sim.world.mono()) {
        if let broadcast_client::Action::Close { conn } = a {
            closed.push(conn);
        }
    }
    let mut closed_sorted = closed.clone();
    closed_sorted.sort_unstable();
    closed_sorted.dedup();
    assert_eq!(closed_sorted.len(), closed.len(), "each connection once");
    assert!(!closed.is_empty(), "requests were in flight");
    assert!(
        closed.iter().all(|c| sim.log.iter().any(|l| l.conn == *c)),
        "only connections the session opened"
    );
    assert!(sim.session.poll(sim.world.mono()).is_none());
    assert!(sim.session.next_event().is_none());
    sim.session.on_response(1, 200, &[], 0);
    sim.session.on_data(1, b"x", 0);
    assert!(sim.session.next_event().is_none());
}

#[test]
fn bltrackid_dash_period_change_emits_tracks_with_unique_ids() {
    use broadcast_client::Event;
    use dash_common::loopback::Origin;
    use dash_common::world::World;
    use transmux::CodecConfig;

    let mpd = r#"<?xml version="1.0"?>
<MPD xmlns="urn:mpeg:dash:schema:mpd:2011" minBufferTime="PT2.000S" type="static" mediaPresentationDuration="PT0H0M18.000S" maxSegmentDuration="PT0H0M2.021S" profiles="urn:mpeg:dash:profile:full:2011">
 <Period id="1" duration="PT0H0M6.013S">
  <AdaptationSet segmentAlignment="true" maxWidth="320" maxHeight="240" maxFrameRate="25" par="4:3" mimeType="video/mp4" startWithSAP="1">
   <SegmentTemplate media="p1_dash_track1_$Number$.m4s" initialization="p1_dash_track1_init.mp4" timescale="12800" startNumber="1" duration="25600"/>
   <Representation id="1" codecs="avc1.4D400D" width="320" height="240" frameRate="25" sar="1:1" bandwidth="84313"/>
  </AdaptationSet>
  <AdaptationSet segmentAlignment="true" mimeType="audio/mp4" startWithSAP="1">
   <SegmentTemplate media="p1_dash_track2_$Number$.m4s" initialization="p1_dash_track2_init.mp4" timescale="44100" startNumber="1" duration="88200"/>
   <Representation id="2" codecs="mp4a.40.2" audioSamplingRate="44100" bandwidth="48523">
    <AudioChannelConfiguration schemeIdUri="urn:mpeg:dash:23003:3:audio_channel_configuration:2011" value="1"/>
   </Representation>
  </AdaptationSet>
 </Period>
 <Period id="2" start="PT0H0M6.000S" duration="PT0H0M6.000S">
  <AdaptationSet segmentAlignment="true" mimeType="audio/mp4" startWithSAP="1">
   <SegmentTemplate media="p2_dash_track2_$Number$.m4s" initialization="p2_dash_track2_init.mp4" timescale="44100" startNumber="1" duration="88200"/>
   <Representation id="4" codecs="mp4a.40.2" audioSamplingRate="44100" bandwidth="48855">
    <AudioChannelConfiguration schemeIdUri="urn:mpeg:dash:23003:3:audio_channel_configuration:2011" value="1"/>
   </Representation>
  </AdaptationSet>
 </Period>
 <Period id="3" start="PT0H0M12.000S" duration="PT0H0M6.000S">
  <AdaptationSet segmentAlignment="true" maxWidth="320" maxHeight="240" maxFrameRate="25" par="4:3" mimeType="video/mp4" startWithSAP="1">
   <SegmentTemplate media="p2_dash_track1_$Number$.m4s" initialization="p2_dash_track1_init.mp4" timescale="12800" startNumber="1" duration="25600"/>
   <Representation id="3" codecs="avc1.4D400D" width="320" height="240" frameRate="25" sar="1:1" bandwidth="115070"/>
  </AdaptationSet>
  <AdaptationSet segmentAlignment="true" mimeType="audio/mp4" startWithSAP="1">
   <SegmentTemplate media="p2_dash_track2_$Number$.m4s" initialization="p2_dash_track2_init.mp4" timescale="44100" startNumber="1" duration="88200"/>
   <Representation id="4" codecs="mp4a.40.2" audioSamplingRate="44100" bandwidth="48855">
    <AudioChannelConfiguration schemeIdUri="urn:mpeg:dash:23003:3:audio_channel_configuration:2011" value="1"/>
   </Representation>
  </AdaptationSet>
 </Period>
</MPD>
"#;

    let mut origin = Origin::new();
    origin = origin.mount("dash.test", "multiperiod-restart");
    origin = origin.doc("http://dash.test/bltrackid.mpd", mpd.as_bytes().to_vec());

    let mut sim = dash_sim::DashSim::new(
        "http://dash.test/bltrackid.mpd",
        broadcast_client::Config::default(),
        World::new(1_700_000_000 * 1_000_000),
        Box::new(dash_sim::StaticBackend(origin)),
    );
    sim.run_to_terminal(60 * dash_sim::SEC);

    let tracks_events: Vec<&Vec<broadcast_client::TrackSpec>> = sim
        .events
        .iter()
        .filter_map(|e| match e {
            Event::Tracks(s) => Some(s),
            _ => None,
        })
        .collect();
    assert!(
        tracks_events.len() >= 3,
        "a Period that changes the track set must emit Event::Tracks: got {}",
        tracks_events.len()
    );

    let mut video_ids = std::collections::BTreeSet::new();
    let mut audio_ids = std::collections::BTreeSet::new();
    for specs in &tracks_events {
        for spec in *specs {
            match &spec.config {
                CodecConfig::Avc { .. } => {
                    video_ids.insert(spec.track_id.value());
                }
                _ => {
                    audio_ids.insert(spec.track_id.value());
                }
            }
        }
    }
    assert_eq!(
        video_ids.len(),
        1,
        "video track id must stay unique per kind"
    );
    assert_eq!(
        audio_ids.len(),
        1,
        "audio track id must stay unique per kind"
    );
    assert!(
        video_ids.is_disjoint(&audio_ids),
        "a track id must not be shared between media kinds"
    );

    // The audio-only Period must announce only the audio track.
    assert!(
        tracks_events
            .iter()
            .any(|s| s.len() == 1 && s[0].config.audio_sample_rate().is_some()),
        "audio-only Period did not emit an audio-only Tracks event"
    );
}
