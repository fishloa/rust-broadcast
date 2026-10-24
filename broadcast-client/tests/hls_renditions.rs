mod support;

use std::collections::{BTreeSet, HashMap, HashSet};

use broadcast_client::{
    AUDIO_TRACK_ID_OFFSET, Config, Credentials, Error, Event, Session, SourceKind, Status,
};
use support::hls_host::*;
use support::renditions::*;
use transmux::sample_aes::aes128_encrypt_segment;

const AUDIO2_DIR: &str = "http://origin.test/hls/a2";
const AUDIO2_PLAYLIST_URL: &str = "http://origin.test/hls/a2/index.m3u8";
const LOW_PLAYLIST_URL: &str = "http://origin.test/hls/low/index.m3u8";
const LOW_DIR: &str = "http://origin.test/hls/low";
const HI_PLAYLIST_URL: &str = "http://origin.test/hls/hi/index.m3u8";
const HI_DIR: &str = "http://origin.test/hls/hi";
const CDN_AUDIO_DIR: &str = "http://cdn.test/a";
const CDN_AUDIO_PLAYLIST_URL: &str = "http://cdn.test/a/index.m3u8";
const KEY_URL: &str = "http://origin.test/hls/akey.bin";
const AUDIO_KEY: [u8; 16] = [0x5A; 16];
const AUDIO_IV: [u8; 16] = [0x07; 16];
const BEARER: &str = "rend-token";
const VIDEO_SAMPLES: usize = 75;
const AUDIO_SAMPLES: usize = 131;
const BACKOFF_FIRST_US: u64 = 2 * SEC;
const BACKOFF_SECOND_US: u64 = 4 * SEC;
const BACKOFF_THIRD_US: u64 = 8 * SEC;
const SPLIT_STEPS: usize = 10_000;

const MEDIA_EN: &str = "#EXT-X-MEDIA:TYPE=AUDIO,GROUP-ID=\"aud\",NAME=\"English\",LANGUAGE=\"en\",DEFAULT=YES,AUTOSELECT=YES,URI=\"a/index.m3u8\"\n";
const MEDIA_DE: &str = "#EXT-X-MEDIA:TYPE=AUDIO,GROUP-ID=\"aud\",NAME=\"Deutsch\",LANGUAGE=\"de-DE\",DEFAULT=NO,AUTOSELECT=YES,URI=\"a2/index.m3u8\"\n";
const STREAM_INF_AUD: &str = "#EXT-X-STREAM-INF:BANDWIDTH=800000,AUDIO=\"aud\"\nv/index.m3u8\n";

fn master(lines: &[&str]) -> Vec<u8> {
    let mut s = String::from("#EXTM3U\n");
    for l in lines {
        s.push_str(l);
    }
    s.into_bytes()
}

fn default_master() -> Vec<u8> {
    master(&[MEDIA_EN, STREAM_INF_AUD])
}

fn two_language_master() -> Vec<u8> {
    master(&[MEDIA_EN, MEDIA_DE, STREAM_INF_AUD])
}

fn add(h: &mut Host, bodies: HashMap<String, Vec<u8>>) {
    for (u, b) in bodies {
        h.ok(&u, b);
    }
}

fn host(f: &Fixture, master_body: Vec<u8>, config: Config) -> Host {
    let mut h = Host::new(MASTER_URL, config);
    h.ok(MASTER_URL, master_body);
    add(&mut h, f.video.bodies(VIDEO_DIR, VIDEO_PLAYLIST_URL));
    add(&mut h, f.audio.bodies(AUDIO_DIR, AUDIO_PLAYLIST_URL));
    add(&mut h, f.audio.bodies(AUDIO2_DIR, AUDIO2_PLAYLIST_URL));
    h
}

fn is_audio_track(t: transmux::TrackId) -> bool {
    t.value() >= AUDIO_TRACK_ID_OFFSET
}

fn video_samples(events: &[Event]) -> Vec<transmux::Sample> {
    samples_of(events, |t| !is_audio_track(t))
}

fn audio_samples(events: &[Event]) -> Vec<transmux::Sample> {
    samples_of(events, is_audio_track)
}

fn count_audio(events: &[Event]) -> usize {
    audio_samples(events).len()
}

fn count_video(events: &[Event]) -> usize {
    video_samples(events).len()
}

fn audio_renditions(events: &[Event]) -> Vec<(Option<usize>, Option<String>, Option<String>)> {
    events
        .iter()
        .filter_map(|e| match e {
            Event::AudioRendition {
                index,
                name,
                language,
            } => Some((*index, name.clone(), language.clone())),
            _ => None,
        })
        .collect()
}

fn tracks_events(events: &[Event]) -> Vec<BTreeSet<u32>> {
    events
        .iter()
        .filter_map(|e| match e {
            Event::Tracks(t) => Some(t.iter().map(|s| s.track_id.value()).collect()),
            _ => None,
        })
        .collect()
}

fn reconnecting(events: &[Event]) -> Vec<u64> {
    events
        .iter()
        .filter_map(|e| match e {
            Event::Status(Status::Reconnecting { attempt }) => Some(*attempt),
            _ => None,
        })
        .collect()
}

fn sample_events_as(
    events: &[Event],
    pick: impl Fn(transmux::TrackId) -> bool,
    shift: u32,
) -> Vec<Event> {
    events
        .iter()
        .filter_map(|e| match e {
            Event::Sample { track, sample, .. } if pick(*track) => Some(Event::Sample {
                track: transmux::TrackId::new(track.value() - shift),
                sample: sample.clone(),
                join: None,
            }),
            _ => None,
        })
        .collect()
}

fn oracle_samples(playlist_url: &str, bodies: &HashMap<String, Vec<u8>>) -> Vec<Event> {
    oracle_events(playlist_url, bodies)
        .into_iter()
        .filter(|e| matches!(e, Event::Sample { .. }))
        .collect()
}

fn step_until_hits(h: &mut Host, url: &str, hits: usize) {
    for _ in 0..SPLIT_STEPS {
        h.collect();
        if h.hits(url) >= hits {
            return;
        }
        if !h.step() {
            return;
        }
    }
}

fn index_of(events: &[Event], pred: impl Fn(&Event) -> bool) -> Option<usize> {
    events.iter().position(pred)
}

fn ended_count(events: &[Event]) -> usize {
    events.iter().filter(|e| is_ended(e)).count()
}

#[test]
fn master_with_audio_group_starts_second_adapter_for_default_rendition() {
    let f = fixture();
    let mut h = host(&f, default_master(), hls_config());
    h.run_to_end();
    assert!(h.hits(AUDIO_PLAYLIST_URL) >= 1, "audio playlist fetched");
    assert_eq!(
        h.hits(AUDIO2_PLAYLIST_URL),
        0,
        "non-selected rendition not fetched"
    );
    for (uri, _) in &f.audio.segments {
        assert_eq!(
            h.hits(&format!("{AUDIO_DIR}/{uri}")),
            1,
            "audio segment {uri}"
        );
    }
    assert_eq!(
        audio_renditions(&h.events),
        vec![(Some(0), Some("English".to_string()), Some("en".to_string()))]
    );
    assert_eq!(
        h.events
            .iter()
            .filter(|e| matches!(e, Event::Variants(_)))
            .count(),
        1
    );
    assert_eq!(count_audio(&h.events), AUDIO_SAMPLES);
    assert_eq!(count_video(&h.events), VIDEO_SAMPLES);
    assert_eq!(
        statuses(&h.events)
            .iter()
            .filter(|s| matches!(s, Status::Establishing))
            .count(),
        1
    );
    assert_eq!(ended_count(&h.events), 1);
}

#[test]
fn samples_from_both_adapters_carry_remapped_audio_track_ids() {
    let f = fixture();
    let mut h = host(&f, default_master(), hls_config());
    h.run_to_end();
    let video_id = f.video.spec.track_id.value();
    let audio_id = f.audio_stream_track_id.value();
    assert_eq!(
        video_id, audio_id,
        "the two streams reuse the same raw track id"
    );
    let seen: BTreeSet<u32> = h
        .events
        .iter()
        .filter_map(|e| match e {
            Event::Sample { track, join, .. } => {
                assert!(join.is_none());
                Some(track.value())
            }
            _ => None,
        })
        .collect();
    let expected: BTreeSet<u32> = [video_id, audio_id + AUDIO_TRACK_ID_OFFSET].into();
    assert_eq!(seen, expected);
    assert!(video_id < AUDIO_TRACK_ID_OFFSET);
}

#[test]
fn tracks_event_is_merged_set_and_reemitted_on_change() {
    let f = fixture();
    let mut h = host(&f, default_master(), hls_config());
    h.run_to_end();
    let video_id = f.video.spec.track_id.value();
    let audio_id = f.audio_stream_track_id.value() + AUDIO_TRACK_ID_OFFSET;
    let both: BTreeSet<u32> = [video_id, audio_id].into();
    let tracks = tracks_events(&h.events);
    assert!(tracks.len() >= 2, "{tracks:?}");
    assert_eq!(tracks.last(), Some(&both), "final Tracks is the merged set");
    assert!(
        tracks.iter().any(|t| t.len() == 1),
        "a partial set is emitted before the other side reports"
    );
    let first_both = tracks.iter().position(|t| *t == both).unwrap_or(usize::MAX);
    assert!(
        tracks[first_both..].iter().all(|t| *t == both),
        "once merged, never shrinks: {tracks:?}"
    );
    for t in &tracks {
        assert!(t.is_subset(&both));
    }
}

#[test]
fn merged_tracks_include_audio_when_main_reports_after_audio() {
    let f = fixture();
    let first_video = format!("{VIDEO_DIR}/{}", f.video.segments[0].0);
    let mut h = host(&f, default_master(), hls_config());
    h.route(
        &first_video,
        vec![Reply::Status(500), Reply::Ok(f.video.segments[0].1.clone())],
    );
    h.run_to_end();
    let video_id = f.video.spec.track_id.value();
    let audio_id = f.audio_stream_track_id.value() + AUDIO_TRACK_ID_OFFSET;
    let both: BTreeSet<u32> = [video_id, audio_id].into();
    let tracks = tracks_events(&h.events);
    assert_eq!(
        tracks.first().map(BTreeSet::len),
        Some(1),
        "audio reports first: {tracks:?}"
    );
    assert_eq!(tracks.first(), Some(&BTreeSet::from([audio_id])));
    assert_eq!(
        tracks.last(),
        Some(&both),
        "the main side's later report still carries the audio track"
    );
}

#[test]
fn audio_samples_equal_bare_hlsclient_oracle_per_track() {
    let f = fixture();
    let video_bodies = f.video.bodies(VIDEO_DIR, VIDEO_PLAYLIST_URL);
    let audio_bodies = f.audio.bodies(AUDIO_DIR, AUDIO_PLAYLIST_URL);
    let video_oracle = oracle_samples(VIDEO_PLAYLIST_URL, &video_bodies);
    let audio_oracle = oracle_samples(AUDIO_PLAYLIST_URL, &audio_bodies);
    assert_eq!(video_oracle.len(), f.video.samples.len());
    assert_eq!(audio_oracle.len(), f.audio.samples.len());

    let mut h = host(&f, default_master(), hls_config());
    h.run_to_end();
    let got_video = sample_events_as(&h.events, |t| !is_audio_track(t), 0);
    let got_audio = sample_events_as(&h.events, is_audio_track, AUDIO_TRACK_ID_OFFSET);
    assert_eq!(got_video, video_oracle);
    assert_eq!(got_audio, audio_oracle);
}

#[test]
fn preferred_language_selects_matching_rendition() {
    let f = fixture();
    let mut cfg = hls_config();
    cfg.preferred_audio_language = Some("DE".to_string());
    let mut h = host(&f, two_language_master(), cfg);
    h.run_to_end();
    assert!(h.hits(AUDIO2_PLAYLIST_URL) >= 1);
    assert_eq!(h.hits(AUDIO_PLAYLIST_URL), 0);
    assert_eq!(
        audio_renditions(&h.events),
        vec![(
            Some(1),
            Some("Deutsch".to_string()),
            Some("de-DE".to_string())
        )]
    );
    assert_eq!(count_audio(&h.events), AUDIO_SAMPLES);

    let mut cfg = hls_config();
    cfg.preferred_audio_language = Some("fr".to_string());
    let mut h = host(&f, two_language_master(), cfg);
    h.run_to_end();
    assert!(
        h.hits(AUDIO_PLAYLIST_URL) >= 1,
        "no match falls back to DEFAULT"
    );
    assert_eq!(h.hits(AUDIO2_PLAYLIST_URL), 0);

    let no_autoselect = "#EXT-X-MEDIA:TYPE=AUDIO,GROUP-ID=\"aud\",NAME=\"Deutsch\",LANGUAGE=\"de\",DEFAULT=NO,AUTOSELECT=NO,URI=\"a2/index.m3u8\"\n";
    let mut cfg = hls_config();
    cfg.preferred_audio_language = Some("de".to_string());
    let mut h = host(&f, master(&[MEDIA_EN, no_autoselect, STREAM_INF_AUD]), cfg);
    h.run_to_end();
    assert_eq!(
        h.hits(AUDIO2_PLAYLIST_URL),
        0,
        "a language match needs AUTOSELECT or DEFAULT"
    );
    assert!(h.hits(AUDIO_PLAYLIST_URL) >= 1);
}

#[test]
fn auto_select_disabled_starts_no_audio_adapter() {
    let f = fixture();
    let mut cfg = hls_config();
    cfg.auto_select_audio = false;
    let mut h = host(&f, default_master(), cfg);
    h.run_to_end();
    assert_eq!(h.hits(AUDIO_PLAYLIST_URL), 0);
    assert!(audio_renditions(&h.events).is_empty());
    assert_eq!(count_audio(&h.events), 0);
    assert_eq!(count_video(&h.events), VIDEO_SAMPLES);
    assert_eq!(ended_count(&h.events), 1);
    h.session
        .select_audio_rendition(Some(0))
        .expect("manual select after Ended is accepted and ignored");
    h.run_until(|_| false, SPLIT_STEPS);
    assert_eq!(h.hits(AUDIO_PLAYLIST_URL), 0);
    assert_eq!(count_audio(&h.events), 0);
    assert_eq!(ended_count(&h.events), 1);
}

#[test]
fn manual_audio_selection_before_ended_still_plays_the_rendition() {
    let f = fixture();
    let mut cfg = hls_config();
    cfg.auto_select_audio = false;
    let mut h = host(&f, default_master(), cfg);
    h.run_until(
        |evs| evs.iter().any(|e| matches!(e, Event::Variants(_))),
        SPLIT_STEPS,
    );
    h.session
        .select_audio_rendition(Some(0))
        .expect("manual select");
    h.run_until(|evs| count_audio(evs) >= AUDIO_SAMPLES, SPLIT_STEPS);
    assert_eq!(count_audio(&h.events), AUDIO_SAMPLES);
}

#[test]
fn no_audio_group_means_single_adapter() {
    let f = fixture();
    let m = master(&[
        MEDIA_EN,
        "#EXT-X-STREAM-INF:BANDWIDTH=800000\nv/index.m3u8\n",
    ]);
    let mut h = host(&f, m, hls_config());
    h.run_to_end();
    assert_eq!(h.hits(AUDIO_PLAYLIST_URL), 0);
    assert!(audio_renditions(&h.events).is_empty());
    assert_eq!(count_audio(&h.events), 0);
    assert_eq!(count_video(&h.events), VIDEO_SAMPLES);
    assert!(h.seen.iter().all(|s| !s.url.starts_with(AUDIO_DIR)));
    assert_eq!(ended_count(&h.events), 1);
    assert!(
        tracks_events(&h.events)
            .iter()
            .all(|t| t.iter().all(|id| *id < AUDIO_TRACK_ID_OFFSET))
    );
}

#[test]
fn rendition_without_uri_not_fetched() {
    let master_body = test_fixtures::read("hls/real/bipbop-ts/master.m3u8");
    let mut h = Host::new(MASTER_URL, hls_config());
    h.ok(MASTER_URL, master_body);
    h.run_until(
        |evs| evs.iter().any(|e| matches!(e, Event::Variants(_))),
        SPLIT_STEPS,
    );
    let alt = "http://origin.test/hls/alternate_audio_aac/prog_index.m3u8";
    h.run_until(|_| false, 50);
    assert_eq!(
        h.hits(alt),
        0,
        "URI-less DEFAULT rendition is muxed; nothing else is fetched"
    );
    assert_eq!(
        audio_renditions(&h.events),
        vec![(
            Some(0),
            Some("BipBop Audio 1".to_string()),
            Some("eng".to_string())
        )]
    );
    h.session
        .select_audio_rendition(Some(0))
        .expect("muxed rendition selectable");
    h.run_until(|_| false, 50);
    assert_eq!(h.hits(alt), 0);
    h.session
        .select_audio_rendition(Some(1))
        .expect("uri rendition");
    h.run_until(|_| false, 50);
    assert!(
        h.hits(alt) >= 1,
        "an explicit selection of a rendition with a URI fetches it"
    );
}

#[test]
fn rendition_url_equal_to_variant_url_ignored() {
    let f = fixture();
    let same = "#EXT-X-MEDIA:TYPE=AUDIO,GROUP-ID=\"aud\",NAME=\"Same\",DEFAULT=YES,AUTOSELECT=YES,URI=\"v/index.m3u8\"\n";
    let mut h = host(&f, master(&[same, STREAM_INF_AUD]), hls_config());
    h.run_to_end();
    assert_eq!(
        h.hits(VIDEO_PLAYLIST_URL),
        1,
        "a second adapter would fetch it again"
    );
    assert_eq!(count_audio(&h.events), 0);
    assert_eq!(count_video(&h.events), VIDEO_SAMPLES);
    assert_eq!(ended_count(&h.events), 1);
}

#[test]
fn select_audio_rendition_switches_and_emits_audio_rendition_event() {
    let f = fixture();
    let first_seg = format!("{AUDIO_DIR}/{}", f.audio.segments[1].0);
    let mut h = host(&f, two_language_master(), hls_config());
    h.route(&first_seg, vec![Reply::Hang]);
    step_until_hits(&mut h, &first_seg, 1);
    let old_conn = h.requests_to(&first_seg)[0].conn;
    assert!(count_audio(&h.events) > 0);
    h.session.select_audio_rendition(Some(1)).expect("switch");
    h.collect();
    assert!(h.closes.contains(&old_conn), "old audio connection closed");
    assert_eq!(
        audio_renditions(&h.events),
        vec![
            (Some(0), Some("English".to_string()), Some("en".to_string())),
            (
                Some(1),
                Some("Deutsch".to_string()),
                Some("de-DE".to_string())
            ),
        ]
    );
    let before = h.events.len();
    h.session.on_response(old_conn, 200, &[]);
    h.session.on_data(old_conn, &f.audio.segments[1].1, h.now);
    h.session.on_closed(old_conn, None, h.now);
    h.collect();
    assert_eq!(
        h.events.len(),
        before,
        "late callbacks of the old adapter are ignored"
    );
    h.run_until(|evs| ended_count(evs) == 1, SPLIT_STEPS);
    assert!(h.hits(AUDIO2_PLAYLIST_URL) >= 1, "new rendition started");
    let total = count_audio(&h.events);
    assert!(
        total >= AUDIO_SAMPLES,
        "new adapter delivers its full audio: {total}"
    );
    assert_eq!(ended_count(&h.events), 1);
}

#[test]
fn select_audio_none_stops_audio_adapter_and_closes_conns() {
    let f = fixture();
    let second_seg = format!("{AUDIO_DIR}/{}", f.audio.segments[1].0);
    let mut h = host(&f, default_master(), hls_config());
    h.route(&second_seg, vec![Reply::Hang]);
    step_until_hits(&mut h, &second_seg, 1);
    let audio_conn = h.requests_to(&second_seg)[0].conn;
    h.session.select_audio_rendition(None).expect("stop audio");
    h.collect();
    assert!(h.closes.contains(&audio_conn), "audio connection closed");
    let audio_before = count_audio(&h.events);
    let requests_before = h.seen.len();
    let evs_len = h.events.len();
    h.session.on_response(audio_conn, 200, &[]);
    h.session.on_data(audio_conn, &f.audio.segments[1].1, h.now);
    h.session.on_closed(audio_conn, None, h.now);
    h.collect();
    assert_eq!(h.events.len(), evs_len);
    h.run_until(|evs| ended_count(evs) == 1, SPLIT_STEPS);
    assert_eq!(ended_count(&h.events), 1, "Ended once audio is stopped");
    assert_eq!(count_audio(&h.events), audio_before, "no audio after stop");
    assert!(
        h.seen[requests_before..]
            .iter()
            .all(|s| !s.url.starts_with(AUDIO_DIR)),
        "no further audio requests"
    );
    assert_eq!(count_video(&h.events), VIDEO_SAMPLES);
    let events = audio_renditions(&h.events);
    assert_eq!(events.last(), Some(&(None, None, None)));
    let main_only: BTreeSet<u32> = [f.video.spec.track_id.value()].into();
    assert_eq!(tracks_events(&h.events).last(), Some(&main_only));
}

#[test]
fn select_audio_out_of_range_and_before_master_error() {
    let f = fixture();
    let mut h = host(&f, default_master(), hls_config());
    assert!(matches!(
        h.session.select_audio_rendition(Some(0)),
        Err(Error::NoSuchRendition)
    ));
    assert!(matches!(
        h.session.select_audio_rendition(None),
        Err(Error::NoSuchRendition)
    ));
    h.run_until(
        |evs| evs.iter().any(|e| matches!(e, Event::Variants(_))),
        SPLIT_STEPS,
    );
    assert!(matches!(
        h.session.select_audio_rendition(Some(1)),
        Err(Error::NoSuchRendition)
    ));
    assert!(matches!(
        h.session.select_audio_rendition(Some(usize::MAX)),
        Err(Error::NoSuchRendition)
    ));
    assert!(h.session.select_audio_rendition(Some(0)).is_ok());
    assert!(h.session.select_audio_rendition(None).is_ok());

    let mut ts = Session::open(
        "http://origin.test/live.ts",
        Config {
            kind: SourceKind::TsHttp,
            ..Config::default()
        },
        0,
    )
    .expect("open");
    assert!(matches!(
        ts.select_audio_rendition(Some(0)),
        Err(Error::NotHls)
    ));
}

#[test]
fn select_variant_reevaluates_audio_group() {
    let f = fixture();
    let m = master(&[
        "#EXT-X-MEDIA:TYPE=AUDIO,GROUP-ID=\"a-low\",NAME=\"Low audio\",LANGUAGE=\"en\",DEFAULT=YES,AUTOSELECT=YES,URI=\"a/index.m3u8\"\n",
        "#EXT-X-MEDIA:TYPE=AUDIO,GROUP-ID=\"a-hi\",NAME=\"Hi audio\",LANGUAGE=\"en\",DEFAULT=YES,AUTOSELECT=YES,URI=\"a2/index.m3u8\"\n",
        "#EXT-X-STREAM-INF:BANDWIDTH=200000,AUDIO=\"a-low\"\nlow/index.m3u8\n",
        "#EXT-X-STREAM-INF:BANDWIDTH=900000,AUDIO=\"a-hi\"\nhi/index.m3u8\n",
    ]);
    let hi_second_audio = format!("{AUDIO2_DIR}/{}", f.audio.segments[1].0);
    let mut h = host(&f, m, hls_config());
    add(&mut h, f.video.bodies(LOW_DIR, LOW_PLAYLIST_URL));
    add(&mut h, f.video.bodies(HI_DIR, HI_PLAYLIST_URL));
    h.route(&hi_second_audio, vec![Reply::Hang]);
    step_until_hits(&mut h, &hi_second_audio, 1);
    let chosen = h
        .events
        .iter()
        .find_map(|e| match e {
            Event::Variants(sel) => Some(sel.chosen),
            _ => None,
        })
        .expect("variants");
    assert_eq!(chosen, 1, "policy picks the high variant");
    let hung = h.requests_to(&hi_second_audio)[0].conn;
    assert_eq!(h.hits(AUDIO_PLAYLIST_URL), 0);
    h.session.select_variant(0).expect("select variant");
    h.collect();
    assert!(h.closes.contains(&hung), "previous group's audio closed");
    h.run_until(|evs| ended_count(evs) == 1, SPLIT_STEPS);
    assert!(
        h.hits(AUDIO_PLAYLIST_URL) >= 1,
        "new group's rendition started"
    );
    let events = audio_renditions(&h.events);
    assert_eq!(
        events,
        vec![
            (
                Some(0),
                Some("Hi audio".to_string()),
                Some("en".to_string())
            ),
            (
                Some(0),
                Some("Low audio".to_string()),
                Some("en".to_string())
            ),
        ]
    );
}

#[test]
fn audio_failure_does_not_stall_video() {
    let f = fixture();
    let mut h = host(&f, default_master(), hls_config());
    h.route(AUDIO_PLAYLIST_URL, vec![Reply::Status(500)]);
    h.route(
        VIDEO_PLAYLIST_URL,
        vec![
            Reply::Status(500),
            Reply::Status(500),
            Reply::Status(500),
            Reply::Ok(f.video.playlist.clone()),
        ],
    );
    h.run_until(|evs| count_video(evs) >= VIDEO_SAMPLES, SPLIT_STEPS);
    assert_eq!(count_video(&h.events), VIDEO_SAMPLES, "video keeps flowing");
    assert_eq!(count_audio(&h.events), 0);
    let video_playlist_times: Vec<u64> = h
        .requests_to(VIDEO_PLAYLIST_URL)
        .iter()
        .map(|s| s.at_us)
        .collect();
    assert_eq!(
        video_playlist_times,
        vec![
            0,
            BACKOFF_FIRST_US,
            BACKOFF_FIRST_US + BACKOFF_SECOND_US,
            BACKOFF_FIRST_US + BACKOFF_SECOND_US + BACKOFF_THIRD_US
        ],
        "video retries follow its own backoff table"
    );
    let audio_times: Vec<u64> = h
        .requests_to(AUDIO_PLAYLIST_URL)
        .iter()
        .map(|s| s.at_us)
        .collect();
    assert!(
        audio_times.len() >= 3,
        "audio kept retrying: {audio_times:?}"
    );
    assert_eq!(audio_times[1], BACKOFF_FIRST_US);
    let attempts = reconnecting(&h.events);
    assert!(
        attempts.iter().any(|a| *a >= 3),
        "audio counter advanced independently: {attempts:?}"
    );
    assert_eq!(
        ended_count(&h.events),
        0,
        "VOD does not end while audio is unfinished"
    );
}

#[test]
fn audio_playlist_404_does_not_emit_not_found_status() {
    let f = fixture();
    let mut h = host(&f, default_master(), hls_config());
    h.route(AUDIO_PLAYLIST_URL, vec![Reply::Status(404)]);
    h.run_until(|evs| count_video(evs) >= VIDEO_SAMPLES, SPLIT_STEPS);
    assert_eq!(count_video(&h.events), VIDEO_SAMPLES);
    assert!(
        !statuses(&h.events).contains(&Status::NotFound),
        "NotFound is reserved for the main playlist"
    );
    assert!(!reconnecting(&h.events).is_empty());

    let mut main_404 = Host::new(MASTER_URL, hls_config());
    main_404.route(MASTER_URL, vec![Reply::Status(404)]);
    main_404.run_until(|evs| statuses(evs).contains(&Status::NotFound), 100);
    assert!(statuses(&main_404.events).contains(&Status::NotFound));
}

fn has_header(seen: &Seen, name: &str) -> bool {
    seen.headers
        .iter()
        .any(|(k, _)| k.eq_ignore_ascii_case(name))
}

#[test]
fn auth_applies_to_rendition_requests_same_origin_only() {
    let f = fixture();
    let expected = format!("Bearer {BEARER}");
    let mut h = host(&f, default_master(), hls_config());
    h.session
        .set_credentials(Credentials::bearer(BEARER))
        .unwrap();
    h.run_to_end();
    let audio_requests: Vec<&Seen> = h
        .seen
        .iter()
        .filter(|s| s.url.starts_with(AUDIO_DIR))
        .collect();
    assert!(audio_requests.len() >= 3);
    for s in &audio_requests {
        assert_eq!(s.authorization(), Some(expected.as_str()), "{}", s.url);
    }

    let cdn_master = master(&[
        "#EXT-X-MEDIA:TYPE=AUDIO,GROUP-ID=\"aud\",NAME=\"English\",LANGUAGE=\"en\",DEFAULT=YES,AUTOSELECT=YES,URI=\"http://cdn.test/a/index.m3u8\"\n",
        STREAM_INF_AUD,
    ]);
    let mut h = host(&f, cdn_master, hls_config());
    add(
        &mut h,
        f.audio.bodies(CDN_AUDIO_DIR, CDN_AUDIO_PLAYLIST_URL),
    );
    h.session
        .set_credentials(Credentials::bearer(BEARER))
        .unwrap();
    h.run_to_end();
    let cdn: Vec<&Seen> = h
        .seen
        .iter()
        .filter(|s| s.url.starts_with(CDN_AUDIO_DIR))
        .collect();
    assert!(cdn.len() >= 3);
    for s in &cdn {
        assert_eq!(
            s.authorization(),
            None,
            "credentials never leave the origin: {}",
            s.url
        );
    }
    assert!(
        h.requests_to(VIDEO_PLAYLIST_URL)[0]
            .authorization()
            .is_some()
    );
    assert_eq!(count_audio(&h.events), AUDIO_SAMPLES);
}

#[test]
fn cookies_and_extra_headers_apply_to_rendition_requests() {
    let f = fixture();
    let mut cfg = hls_config();
    cfg.extra_headers = vec![("X-Rend".to_string(), "r-1".to_string())];
    cfg.initial_cookies = vec![("sid".to_string(), "abc".to_string())];
    let mut h = host(&f, default_master(), cfg.clone());
    h.run_to_end();
    let audio_requests: Vec<&Seen> = h
        .seen
        .iter()
        .filter(|s| s.url.starts_with(AUDIO_DIR))
        .collect();
    assert!(audio_requests.len() >= 3);
    for s in &audio_requests {
        let hdr = s
            .headers
            .iter()
            .find(|(k, _)| k == "X-Rend")
            .map(|(_, v)| v.as_str());
        assert_eq!(hdr, Some("r-1"), "{}", s.url);
        let cookie = s
            .headers
            .iter()
            .find(|(k, _)| k.eq_ignore_ascii_case("cookie"))
            .map(|(_, v)| v.as_str());
        assert!(cookie.is_some_and(|c| c.contains("sid=abc")), "{}", s.url);
    }

    let cdn_master = master(&[
        "#EXT-X-MEDIA:TYPE=AUDIO,GROUP-ID=\"aud\",NAME=\"English\",LANGUAGE=\"en\",DEFAULT=YES,AUTOSELECT=YES,URI=\"http://cdn.test/a/index.m3u8\"\n",
        STREAM_INF_AUD,
    ]);
    let mut h = host(&f, cdn_master, cfg);
    add(
        &mut h,
        f.audio.bodies(CDN_AUDIO_DIR, CDN_AUDIO_PLAYLIST_URL),
    );
    h.run_to_end();
    let cdn: Vec<&Seen> = h
        .seen
        .iter()
        .filter(|s| s.url.starts_with(CDN_AUDIO_DIR))
        .collect();
    assert!(cdn.len() >= 3);
    for s in &cdn {
        assert!(!has_header(s, "x-rend"), "{}", s.url);
        assert!(!has_header(s, "cookie"), "{}", s.url);
    }
}

fn hex(b: &[u8; 16]) -> String {
    b.iter().map(|x| format!("{x:02X}")).collect()
}

fn encrypted_audio_playlist(f: &Fixture) -> Vec<u8> {
    let mut s = format!(
        "#EXTM3U\n#EXT-X-VERSION:5\n#EXT-X-TARGETDURATION:3\n#EXT-X-MEDIA-SEQUENCE:0\n\
         #EXT-X-PLAYLIST-TYPE:VOD\n#EXT-X-KEY:METHOD=AES-128,URI=\"{KEY_URL}\",IV=0x{}\n",
        hex(&AUDIO_IV)
    );
    for (uri, _) in &f.audio.segments {
        s.push_str(&format!("#EXTINF:2.0,\n{uri}\n"));
    }
    s.push_str("#EXT-X-ENDLIST\n");
    s.into_bytes()
}

#[test]
fn rendition_key_fetch_uses_shared_machinery() {
    let f = fixture();
    let mut h = host(&f, default_master(), hls_config());
    h.ok(AUDIO_PLAYLIST_URL, encrypted_audio_playlist(&f));
    for (uri, bytes) in &f.audio.segments {
        h.ok(
            &format!("{AUDIO_DIR}/{uri}"),
            aes128_encrypt_segment(&AUDIO_KEY, &AUDIO_IV, bytes),
        );
    }
    h.ok(KEY_URL, AUDIO_KEY.to_vec());
    h.session
        .set_credentials(Credentials::bearer(BEARER))
        .unwrap();
    h.run_to_end();
    assert_eq!(h.hits(KEY_URL), 1, "key fetched once for all segments");
    let key_request = h.requests_to(KEY_URL)[0];
    assert_eq!(
        key_request.authorization(),
        Some(format!("Bearer {BEARER}").as_str()),
        "key request goes through the session's auth"
    );
    let oracle = oracle_samples(
        AUDIO_PLAYLIST_URL,
        &f.audio.bodies(AUDIO_DIR, AUDIO_PLAYLIST_URL),
    );
    assert_eq!(
        sample_events_as(&h.events, is_audio_track, AUDIO_TRACK_ID_OFFSET),
        oracle,
        "decrypted audio equals the plaintext oracle"
    );
    assert!(
        !h.events.iter().any(|e| matches!(e, Event::Error { .. })),
        "no errors"
    );
    assert_eq!(ended_count(&h.events), 1);
}

#[test]
fn fatal_rendition_error_is_fatal_for_whole_session() {
    let f = fixture();
    let mut h = host(&f, default_master(), hls_config());
    let unsupported = "#EXTM3U\n#EXT-X-VERSION:5\n#EXT-X-TARGETDURATION:3\n#EXT-X-MEDIA-SEQUENCE:0\n\
         #EXT-X-KEY:METHOD=SAMPLE-AES,URI=\"http://origin.test/hls/k.bin\"\n#EXTINF:2.0,\nseg0.ts\n#EXT-X-ENDLIST\n";
    h.ok(AUDIO_PLAYLIST_URL, unsupported.as_bytes().to_vec());
    h.run_until(
        |evs| {
            evs.iter()
                .any(|e| matches!(e, Event::Error { fatal: true, .. }))
        },
        SPLIT_STEPS,
    );
    assert!(
        h.events
            .iter()
            .any(|e| matches!(e, Event::Error { fatal: true, .. }))
    );
    let seen_before = h.seen.len();
    let video_before = count_video(&h.events);
    h.run_until(|_| false, 200);
    h.collect();
    let _ = h.session.select_audio_rendition(Some(1));
    let _ = h.session.select_variant(0);
    h.run_until(|_| false, 200);
    h.collect();
    assert_eq!(h.seen.len(), seen_before, "no requests after a fatal error");
    assert_eq!(count_video(&h.events), video_before);
}

#[test]
fn vod_ended_only_after_both_adapters_end() {
    let f = fixture();
    let second_audio = format!("{AUDIO_DIR}/{}", f.audio.segments[1].0);
    let mut h = host(&f, default_master(), hls_config());
    h.route(
        &second_audio,
        vec![Reply::Status(500), Reply::Ok(f.audio.segments[1].1.clone())],
    );
    h.run_until(|evs| count_video(evs) >= VIDEO_SAMPLES, SPLIT_STEPS);
    assert_eq!(count_video(&h.events), VIDEO_SAMPLES);
    assert!(
        count_audio(&h.events) < AUDIO_SAMPLES,
        "audio is still behind"
    );
    assert_eq!(ended_count(&h.events), 0, "main ended but audio has not");
    h.run_to_end();
    assert_eq!(ended_count(&h.events), 1);
    let ended_at = index_of(&h.events, is_ended).expect("ended");
    let last_audio = h
        .events
        .iter()
        .rposition(|e| matches!(e, Event::Sample { track, .. } if is_audio_track(*track)))
        .expect("audio sample");
    assert!(ended_at > last_audio, "Ended follows the last audio sample");
    assert_eq!(count_audio(&h.events), AUDIO_SAMPLES);

    let mut audio_first = host(&f, default_master(), hls_config());
    let second_video = format!("{VIDEO_DIR}/{}", f.video.segments[1].0);
    audio_first.route(
        &second_video,
        vec![Reply::Status(500), Reply::Ok(f.video.segments[1].1.clone())],
    );
    audio_first.run_until(|evs| count_audio(evs) >= AUDIO_SAMPLES, SPLIT_STEPS);
    assert_eq!(count_audio(&audio_first.events), AUDIO_SAMPLES);
    assert_eq!(
        ended_count(&audio_first.events),
        0,
        "audio ended but main has not"
    );
    audio_first.run_to_end();
    assert_eq!(ended_count(&audio_first.events), 1);
}

#[test]
fn late_callbacks_after_close_ignored_for_both_adapters() {
    let f = fixture();
    let video_seg = format!("{VIDEO_DIR}/{}", f.video.segments[0].0);
    let audio_seg = format!("{AUDIO_DIR}/{}", f.audio.segments[0].0);
    let mut h = host(&f, default_master(), hls_config());
    h.route(&video_seg, vec![Reply::Hang]);
    h.route(&audio_seg, vec![Reply::Hang]);
    step_until_hits(&mut h, &video_seg, 1);
    step_until_hits(&mut h, &audio_seg, 1);
    let video_conn = h.requests_to(&video_seg)[0].conn;
    let audio_conn = h.requests_to(&audio_seg)[0].conn;
    assert_ne!(video_conn, audio_conn);
    h.session.close();
    h.collect();
    assert!(h.closes.contains(&video_conn));
    assert!(h.closes.contains(&audio_conn));
    let events_before = h.events.len();
    let seen_before = h.seen.len();
    for (conn, body) in [
        (video_conn, &f.video.segments[0].1),
        (audio_conn, &f.audio.segments[0].1),
    ] {
        h.session.on_response(conn, 200, &[]);
        h.session.on_data(conn, body, h.now);
        h.session.on_closed(conn, None, h.now);
    }
    h.collect();
    assert_eq!(h.events.len(), events_before);
    assert_eq!(h.seen.len(), seen_before);
    assert!(h.session.poll(h.now + 100 * SEC).is_none());
    assert!(h.session.next_event().is_none());
}

#[test]
fn conn_ids_never_collide_between_adapters() {
    let f = fixture();
    let mut h = host(&f, two_language_master(), hls_config());
    h.run_until(|evs| count_audio(evs) > 0, SPLIT_STEPS);
    h.session.select_audio_rendition(Some(1)).expect("switch");
    h.run_to_end();
    let ids: Vec<u32> = h.seen.iter().map(|s| s.conn).collect();
    let unique: HashSet<u32> = ids.iter().copied().collect();
    assert_eq!(unique.len(), ids.len(), "every request has its own conn id");
    assert!(h.seen.iter().any(|s| s.url.starts_with(AUDIO_DIR)));
    assert!(h.seen.iter().any(|s| s.url.starts_with(AUDIO2_DIR)));
    assert!(h.seen.iter().any(|s| s.url.starts_with(VIDEO_DIR)));
}

#[test]
fn status_live_after_first_sample_from_either() {
    let f = fixture();
    let first_video = format!("{VIDEO_DIR}/{}", f.video.segments[0].0);
    let mut h = host(&f, default_master(), hls_config());
    h.route(
        &first_video,
        vec![Reply::Status(500), Reply::Ok(f.video.segments[0].1.clone())],
    );
    h.run_to_end();
    let lives: Vec<usize> = h
        .events
        .iter()
        .enumerate()
        .filter(|(_, e)| matches!(e, Event::Status(Status::Live)))
        .map(|(i, _)| i)
        .collect();
    assert_eq!(lives.len(), 1, "Live exactly once");
    let first_sample = index_of(&h.events, |e| matches!(e, Event::Sample { .. })).expect("sample");
    assert!(
        matches!(&h.events[first_sample], Event::Sample { track, .. } if is_audio_track(*track)),
        "audio delivered first"
    );
    assert!(lives[0] < first_sample, "Live precedes the first sample");
    assert!(count_video(&h.events) > 0);

    let mut v_first = host(&f, default_master(), hls_config());
    v_first.route(AUDIO_PLAYLIST_URL, vec![Reply::Status(500)]);
    v_first.run_until(|evs| count_video(evs) >= VIDEO_SAMPLES, SPLIT_STEPS);
    assert_eq!(
        v_first
            .events
            .iter()
            .filter(|e| matches!(e, Event::Status(Status::Live)))
            .count(),
        1
    );
}
