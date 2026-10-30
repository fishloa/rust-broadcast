//! Round-7 finding F6: scenarios the audit could not drive. (a) byte-range and vanishing
//! EXT-X-MAP, (b) CENC fMP4 with and without EXT-X-KEY, (c) EXT-X-DISCONTINUITY while the buffer
//! holds, (e) a 40-stream PMT with a declared audio stream that never sends.

mod support;

use std::collections::VecDeque;

use broadcast_client::events::DEFAULT_START_PENDING_TRACK_WAIT_US;
use broadcast_client::{Action, BufferEvent, Config, ErrorKind, Event, Session};
use broadcast_common::cenc::CencScheme;
use support::fmp4_split::{group, split};
use support::hls_host::{Host, Reply, hls_config, is_ended};
use support::ts_host::Host as TsHost;
use support::ts_pmt::{Es, find, rewrite};

const BASE: &str = "http://origin.test/v";
const PL: &str = "http://origin.test/v/index.m3u8";
const FRAGMENT_FIXTURE: &str = "transmux/h264_aac_frag.mp4";
const SEGMENT_GROUP: usize = 3;
const STEPS: usize = 20_000;
const VOD_HEAD: &str =
    "#EXTM3U\n#EXT-X-VERSION:7\n#EXT-X-TARGETDURATION:10\n#EXT-X-PLAYLIST-TYPE:VOD\n";
const LIVE_HEAD: &str = "#EXTM3U\n#EXT-X-VERSION:7\n#EXT-X-TARGETDURATION:2\n";
const MAP_PAD: usize = 100;
const MAP_TAIL: usize = 50;
const LIVE_WINDOWS: usize = 3;
const KEY_URI: &str = "skd://key";
const KID: [u8; 16] = [7; 16];

fn parts() -> (Vec<u8>, Vec<Vec<u8>>) {
    let b = test_fixtures::read(FRAGMENT_FIXTURE);
    let s = split(&b).unwrap();
    (s.init, group(&s.segments, SEGMENT_GROUP))
}

#[derive(Debug, Default)]
struct Tally {
    tracks: usize,
    samples: usize,
    discontinuities: usize,
    ended: bool,
    fatals: Vec<ErrorKind>,
    non_fatals: Vec<ErrorKind>,
}

fn tally(events: &[Event]) -> Tally {
    let mut t = Tally::default();
    for e in events {
        match e {
            Event::Tracks(_) => t.tracks += 1,
            Event::Sample { .. } => t.samples += 1,
            Event::Discontinuity => t.discontinuities += 1,
            Event::Error { kind, fatal: true } => t.fatals.push(kind.clone()),
            Event::Error { kind, fatal: false } => t.non_fatals.push(kind.clone()),
            e if is_ended(e) => t.ended = true,
            _ => {}
        }
    }
    t
}

fn stop_on_end_or_fatal(e: &[Event]) -> bool {
    e.iter()
        .any(|x| is_ended(x) || matches!(x, Event::Error { fatal: true, .. }))
}

fn padded_map_file(init: &[u8]) -> Vec<u8> {
    let mut file = vec![0xAA; MAP_PAD];
    file.extend_from_slice(init);
    file.extend_from_slice(&[0xBB; MAP_TAIL]);
    file
}

fn vod_with_map(map_attrs: &str, init_reply: Reply) -> Tally {
    let (_, segs) = parts();
    let p = format!("{VOD_HEAD}#EXT-X-MAP:{map_attrs}\n#EXTINF:1.0,\ns0.m4s\n#EXT-X-ENDLIST\n");
    let mut h = Host::new(PL, hls_config());
    h.ok(PL, p.into_bytes());
    h.route(&format!("{BASE}/m.mp4"), vec![init_reply]);
    h.ok(&format!("{BASE}/s0.m4s"), segs[0].clone());
    h.run_until(stop_on_end_or_fatal, STEPS);
    tally(&h.events)
}

fn clean_vod_samples() -> usize {
    let (init, _) = parts();
    let t = vod_with_map("URI=\"m.mp4\"", Reply::Ok(init));
    assert!(t.ended && t.fatals.is_empty(), "{t:?}");
    t.samples
}

#[test]
fn a_byte_range_map_answered_with_the_whole_file_is_sliced() {
    let (init, _) = parts();
    let attrs = format!("URI=\"m.mp4\",BYTERANGE=\"{}@{MAP_PAD}\"", init.len());
    let t = vod_with_map(&attrs, Reply::Ok(padded_map_file(&init)));
    assert!(t.ended && t.fatals.is_empty(), "{t:?}");
    assert_eq!(t.samples, clean_vod_samples());
    assert_eq!(t.tracks, 1);
}

#[test]
fn a_byte_range_map_answered_with_206_plays() {
    let (init, _) = parts();
    let file = padded_map_file(&init);
    let end = MAP_PAD + init.len() - 1;
    let reply = Reply::Full(
        206,
        init.clone(),
        vec![(
            "Content-Range".to_string(),
            format!("bytes {MAP_PAD}-{end}/{}", file.len()),
        )],
    );
    let attrs = format!("URI=\"m.mp4\",BYTERANGE=\"{}@{MAP_PAD}\"", init.len());
    let t = vod_with_map(&attrs, reply);
    assert!(t.ended && t.fatals.is_empty(), "{t:?}");
    assert_eq!(t.samples, clean_vod_samples());
}

#[test]
fn a_byte_range_map_answered_with_the_wrong_206_range_is_a_typed_fatal() {
    let (init, _) = parts();
    let file = padded_map_file(&init);
    let wrong = Reply::Full(
        206,
        init.clone(),
        vec![(
            "Content-Range".to_string(),
            format!("bytes 0-{}/{}", init.len() - 1, file.len()),
        )],
    );
    let attrs = format!("URI=\"m.mp4\",BYTERANGE=\"{}@{MAP_PAD}\"", init.len());
    let t = vod_with_map(&attrs, wrong);
    assert!(!t.ended, "{t:?}");
    assert_eq!(t.samples, 0);
    assert_eq!(
        t.fatals,
        vec![ErrorKind::InitFetchFailed { status: Some(206) }]
    );
}

fn live_windows(map_in: impl Fn(usize) -> Option<String>) -> Tally {
    let (init, segs) = parts();
    let mut pl = vec![];
    for r in 0..LIVE_WINDOWS {
        let mut p = format!("{LIVE_HEAD}#EXT-X-MEDIA-SEQUENCE:{r}\n");
        if let Some(map) = map_in(r) {
            p.push_str(&format!("#EXT-X-MAP:{map}\n"));
        }
        p.push_str(&format!("#EXTINF:1.0,\ns{r}.m4s\n"));
        pl.push(Reply::Ok(p.into_bytes()));
    }
    let mut h = Host::new(PL, hls_config());
    h.route(PL, pl);
    h.ok(&format!("{BASE}/m.mp4"), padded_map_file(&init));
    for (i, seg) in segs.iter().enumerate().take(LIVE_WINDOWS) {
        h.ok(&format!("{BASE}/s{i}.m4s"), seg.clone());
    }
    h.run_until(|_| false, 400);
    let mut t = tally(&h.events);
    t.ended = h.hits(&format!("{BASE}/m.mp4")) == 1;
    t
}

#[test]
fn a_live_byte_range_map_repeated_in_every_window_is_fetched_once() {
    let (init, _) = parts();
    let attrs = format!("URI=\"m.mp4\",BYTERANGE=\"{}@{MAP_PAD}\"", init.len());
    let t = live_windows(|_| Some(attrs.clone()));
    assert!(t.ended, "init fetched once: {t:?}");
    assert_eq!(t.tracks, 1);
    assert!(t.fatals.is_empty() && t.non_fatals.is_empty(), "{t:?}");
    assert!(t.samples > 0);
}

#[test]
fn a_live_map_that_later_windows_no_longer_carry_keeps_the_stream_playing() {
    let (init, _) = parts();
    let attrs = format!("URI=\"m.mp4\",BYTERANGE=\"{}@{MAP_PAD}\"", init.len());
    let repeated = live_windows(|_| Some(attrs.clone()));
    let vanishing = live_windows(|r| (r == 0).then(|| attrs.clone()));
    assert!(vanishing.ended, "init fetched exactly once: {vanishing:?}");
    assert_eq!(vanishing.tracks, 1);
    assert_eq!(vanishing.discontinuities, 0);
    assert!(vanishing.fatals.is_empty() && vanishing.non_fatals.is_empty());
    assert_eq!(vanishing.samples, repeated.samples);
}

fn two_segment_vod(between: &str) -> Tally {
    let (init, segs) = parts();
    let p = format!(
        "{VOD_HEAD}#EXT-X-MAP:URI=\"m.mp4\"\n#EXTINF:1.0,\ns0.m4s\n{between}#EXTINF:1.0,\ns1.m4s\n#EXT-X-ENDLIST\n"
    );
    let mut h = Host::new(PL, hls_config());
    h.ok(PL, p.into_bytes());
    h.ok(&format!("{BASE}/m.mp4"), init);
    h.ok(&format!("{BASE}/s0.m4s"), segs[0].clone());
    h.ok(&format!("{BASE}/s1.m4s"), segs[1].clone());
    h.run_until(stop_on_end_or_fatal, STEPS);
    tally(&h.events)
}

#[test]
fn a_vod_discontinuity_without_a_new_map_keeps_the_last_init() {
    let plain = two_segment_vod("");
    let cut = two_segment_vod("#EXT-X-DISCONTINUITY\n");
    assert!(plain.ended && cut.ended, "{plain:?} {cut:?}");
    assert!(
        cut.fatals.is_empty() && cut.non_fatals.is_empty(),
        "{cut:?}"
    );
    assert_eq!(cut.samples, plain.samples);
    assert_eq!(plain.discontinuities, 0);
    assert_eq!(cut.discontinuities, 1);
    assert_eq!(cut.tracks, 1);
}

fn cenc_playlist(key_tag: Option<&str>) -> String {
    let mut p = String::from(VOD_HEAD);
    if let Some(k) = key_tag {
        p.push_str(k);
        p.push('\n');
    }
    p.push_str("#EXT-X-MAP:URI=\"init.mp4\"\n");
    for i in 0..SEGMENT_GROUP {
        p.push_str(&format!("#EXTINF:1.0,\ns{i}.m4s\n"));
    }
    p.push_str("#EXT-X-ENDLIST\n");
    p
}

fn play_cenc(fixture: &str, key_tag: Option<&str>) -> Tally {
    let bytes = test_fixtures::read(fixture);
    let parts = split(&bytes).unwrap();
    let segs = group(&parts.segments, SEGMENT_GROUP);
    let p = cenc_playlist(key_tag);
    let mut h = Host::new(PL, hls_config());
    h.ok(PL, p.into_bytes());
    h.ok(&format!("{BASE}/init.mp4"), parts.init);
    for (i, s) in segs.into_iter().enumerate() {
        h.ok(&format!("{BASE}/s{i}.m4s"), s);
    }
    h.run_until(stop_on_end_or_fatal, STEPS);
    tally(&h.events)
}

fn key_tag(scheme: CencScheme) -> String {
    broadcast_hls::cenc_ext_x_key(scheme, &KID, KEY_URI)
        .unwrap()
        .unwrap()
}

#[test]
fn cenc_fmp4_without_an_ext_x_key_reports_the_track_and_plays_the_clear_audio() {
    for fixture in ["transmux/h264_cenc.mp4", "transmux/h264_cbcs.mp4"] {
        let t = play_cenc(fixture, None);
        assert!(t.ended, "{fixture}: {t:?}");
        assert!(t.fatals.is_empty(), "{fixture}: {t:?}");
        assert_eq!(
            t.non_fatals,
            vec![ErrorKind::UnsupportedEncryption {
                method: "encv".to_string()
            }],
            "{fixture}"
        );
        assert!(t.samples > 0, "{fixture}: the clear audio plays");
    }
}

#[test]
fn cenc_fmp4_with_an_ext_x_key_is_one_fatal_unsupported_encryption_and_no_samples() {
    for (fixture, scheme) in [
        ("transmux/h264_cenc.mp4", CencScheme::Cenc),
        ("transmux/h264_cbcs.mp4", CencScheme::Cbcs),
    ] {
        let t = play_cenc(fixture, Some(&key_tag(scheme)));
        assert!(!t.ended, "{fixture}: {t:?}");
        assert_eq!(t.samples, 0, "{fixture}");
        assert_eq!(t.fatals.len(), 1, "{fixture}: {t:?}");
        assert!(t.non_fatals.is_empty(), "{fixture}: fatal at once: {t:?}");
        assert!(
            matches!(&t.fatals[0], ErrorKind::UnsupportedEncryption { .. }),
            "{fixture}: {t:?}"
        );
    }
}

const TS_FIXTURE: &str = "ts/h264_aac_40s.ts";
const TS_PACKET: usize = 188;
const SEGMENT_COUNT: usize = 8;
const DISCONTINUITY_AT: usize = 4;
const WINDOW_STEP_US: u64 = 3_500_000;
const TICK_US: u64 = 100_000;
const RUN_US: u64 = 60_000_000;
const STARVE_FROM_US: u64 = 1_000_000;
const STARVE_UNTIL_US: u64 = 30_000_000;
const INITIAL_VISIBLE: usize = 3;
const CHUNK: usize = 64 * 1024;
const PAT_PID: u16 = 0;
const PID_HIGH: u8 = 0x1f;
const PUSI: u8 = 0x40;
const TICKS_PER_US: u64 = 27;

fn split_at_pat(b: &[u8], n: usize) -> Vec<Vec<u8>> {
    let pk = b.len() / TS_PACKET;
    let pats: Vec<usize> = (0..pk)
        .filter(|&i| {
            let p = &b[i * TS_PACKET..];
            u16::from(p[1] & PID_HIGH) << 8 | u16::from(p[2]) == PAT_PID && p[1] & PUSI != 0
        })
        .collect();
    let mut cuts = vec![0usize];
    for k in 1..n {
        let target = pk * k / n;
        if let Some(&c) = pats.iter().min_by_key(|&&c| c.abs_diff(target))
            && c > *cuts.last().unwrap()
        {
            cuts.push(c);
        }
    }
    cuts.push(pk);
    cuts.windows(2)
        .map(|w| b[w[0] * TS_PACKET..w[1] * TS_PACKET].to_vec())
        .collect()
}

#[derive(Debug, Default)]
struct HeldRun {
    events: Vec<(u64, Event)>,
}

impl HeldRun {
    fn tally(&self) -> Tally {
        let events: Vec<Event> = self.events.iter().map(|(_, e)| e.clone()).collect();
        tally(&events)
    }

    fn buffer(&self) -> Vec<(u64, BufferEvent)> {
        self.events
            .iter()
            .filter_map(|(t, e)| match e {
                Event::Buffer(b) => Some((*t, b.clone())),
                _ => None,
            })
            .collect()
    }

    fn at_disc(&self) -> Option<usize> {
        self.events
            .iter()
            .position(|(_, e)| matches!(e, Event::Discontinuity))
    }
}

fn run_ts_hls(segs: &[Vec<u8>], disc_at: Option<usize>, starve: bool) -> HeldRun {
    let n = segs.len();
    let mut s = Session::open("http://h/p/index.m3u8", Config::default(), 0).unwrap();
    let mut out = HeldRun::default();
    let mut now = 0u64;
    let mut position: Option<(u64, u64)> = None;
    let mut visible = INITIAL_VISIBLE.min(n);
    while now < RUN_US {
        let frozen = starve && (STARVE_FROM_US..STARVE_UNTIL_US).contains(&now);
        if !frozen {
            let grown = usize::try_from(now / WINDOW_STEP_US).unwrap_or(usize::MAX);
            visible = INITIAL_VISIBLE.saturating_add(grown).min(n);
        }
        let mut pl = String::from(
            "#EXTM3U\n#EXT-X-VERSION:3\n#EXT-X-TARGETDURATION:4\n#EXT-X-MEDIA-SEQUENCE:0\n",
        );
        for i in 0..visible {
            if Some(i) == disc_at {
                pl.push_str("#EXT-X-DISCONTINUITY\n");
            }
            pl.push_str(&format!("#EXTINF:3.5,\nhttp://h/s/seg{i}.ts\n"));
        }
        if visible == n {
            pl.push_str("#EXT-X-ENDLIST\n");
        }
        let mut pending: VecDeque<(u32, String)> = VecDeque::new();
        loop {
            while let Some(a) = s.poll(now) {
                if let Action::HttpRequest { conn, url, .. } = a {
                    pending.push_back((conn, url));
                }
            }
            let Some((c, u)) = pending.pop_front() else {
                break;
            };
            let body: Vec<u8> = if u.ends_with(".m3u8") {
                pl.clone().into_bytes()
            } else {
                let i: usize = u
                    .trim_end_matches(".ts")
                    .rsplit("seg")
                    .next()
                    .unwrap()
                    .parse()
                    .unwrap();
                segs[i].clone()
            };
            s.on_response(
                c,
                200,
                &[("Content-Length".into(), body.len().to_string())],
                now,
            );
            for chunk in body.chunks(CHUNK) {
                s.on_data(c, chunk, now);
            }
            s.on_closed(c, None, now);
        }
        let pos = position.map(|(base, since)| base + (now - since) * TICKS_PER_US);
        s.on_playback_position(now, pos);
        while let Some(e) = s.next_event() {
            match &e {
                Event::Buffer(
                    BufferEvent::Start { at_27mhz } | BufferEvent::Resume { at_27mhz },
                ) => position = Some((*at_27mhz, now)),
                Event::Buffer(BufferEvent::Pause { .. }) => position = None,
                _ => {}
            }
            out.events.push((now, e));
        }
        now += TICK_US;
    }
    out
}

fn ts_segments() -> Vec<Vec<u8>> {
    let b = test_fixtures::read(TS_FIXTURE);
    split_at_pat(&b[..b.len() / TS_PACKET * TS_PACKET], SEGMENT_COUNT)
}

fn playing_at_end(buffer: &[(u64, BufferEvent)]) -> bool {
    matches!(
        buffer.last().map(|(_, b)| b),
        Some(BufferEvent::Start { .. } | BufferEvent::Resume { .. })
    )
}

#[test]
fn a_discontinuity_arriving_while_the_buffer_holds_loses_nothing_and_resumes_playing() {
    let segs = ts_segments();
    assert!(segs.len() > DISCONTINUITY_AT + 1);
    let free = run_ts_hls(&segs, Some(DISCONTINUITY_AT), false);
    let held = run_ts_hls(&segs, Some(DISCONTINUITY_AT), true);
    let plain_held = run_ts_hls(&segs, None, true);

    let buffer = held.buffer();
    let pause = buffer
        .iter()
        .position(|(_, b)| matches!(b, BufferEvent::Pause { .. }))
        .expect("the starved window holds the buffer");
    let disc = held.at_disc().expect("a discontinuity");
    let pause_at = buffer[pause].0;
    let disc_at = held.events[disc].0;
    assert!(
        pause_at < disc_at,
        "the discontinuity must arrive during the hold: pause {pause_at} disc {disc_at}"
    );

    let t = held.tally();
    assert_eq!(t.discontinuities, 1, "{t:?}");
    assert!(t.fatals.is_empty() && t.non_fatals.is_empty(), "{t:?}");
    assert!(t.ended, "{t:?}");
    assert_eq!(t.samples, free.tally().samples, "the hold lost samples");
    assert_eq!(plain_held.tally().discontinuities, 0);
    assert!(playing_at_end(&buffer), "left paused: {buffer:?}");

    let hold = match &buffer[pause].1 {
        BufferEvent::Pause { hold_at_27mhz, .. } => *hold_at_27mhz,
        other => panic!("{other:?}"),
    };
    let after: Vec<&BufferEvent> = buffer[pause + 1..].iter().map(|(_, b)| b).collect();
    assert!(
        matches!(after.first(), Some(BufferEvent::Resume { at_27mhz }) if *at_27mhz == hold),
        "the first event after Pause resumes at the held position: {after:?}"
    );
    assert!(
        matches!(after.last(), Some(BufferEvent::Start { .. })),
        "the new timeline starts afresh after the discontinuity: {after:?}"
    );
}

const PMT_STREAMS: usize = 40;
const FIRST_EXTRA_PID: u16 = 0x300;
const PES_PRIVATE_DATA: u8 = 0x06;
const AC3_DESCRIPTOR: [u8; 2] = [0x6a, 0x00];
const TS_CHUNK: usize = 4 * TS_PACKET;
const START_BOUND_AFTER_WAIT_US: u64 = 1_000_000;
const IDLE_STEP_US: u64 = 50_000;
const IDLE_STEPS: usize = 400;

fn pid_of(p: &[u8]) -> u16 {
    (u16::from(p[1] & PID_HIGH) << 8) | u16::from(p[2])
}

fn forty_stream_pmt(declare_private_audio: bool) -> Vec<u8> {
    let bytes = test_fixtures::read(TS_FIXTURE);
    let pmt = find(&bytes).expect("a PMT");
    let (video, sent_audio) = (pmt.es[0].clone(), pmt.es[1].pid);
    let version = pmt.version;
    let video_only: Vec<u8> = bytes
        .chunks(TS_PACKET)
        .filter(|p| pid_of(p) != sent_audio)
        .flatten()
        .copied()
        .collect();
    let mut es: Vec<Es> = vec![video];
    let mut pid = FIRST_EXTRA_PID;
    if declare_private_audio {
        es.push(Es {
            stream_type: PES_PRIVATE_DATA,
            pid,
            descriptors: AC3_DESCRIPTOR.to_vec(),
        });
        pid += 1;
    }
    while es.len() < PMT_STREAMS {
        es.push(Es {
            stream_type: PES_PRIVATE_DATA,
            pid,
            descriptors: vec![],
        });
        pid += 1;
    }
    assert_eq!(es.len(), PMT_STREAMS);
    rewrite(&video_only, &pmt, &|_| {
        Some((es.clone(), version.wrapping_add(1)))
    })
}

fn start_of(bytes: &[u8]) -> (Option<u64>, Option<u64>) {
    start_of_with(bytes, Config::default())
}

fn start_of_with(bytes: &[u8], config: Config) -> (Option<u64>, Option<u64>) {
    let mut host = TsHost::open(config);
    let mut first_sample = None;
    let mut start = None;
    let body = &bytes[..bytes.len() / TS_PACKET * TS_PACKET];
    let observe = |host: &TsHost, first: &mut Option<u64>, start: &mut Option<u64>| {
        if first.is_none() && host.log.iter().any(|e| matches!(e, Event::Sample { .. })) {
            *first = Some(host.now);
        }
        if start.is_none()
            && host
                .log
                .iter()
                .any(|e| matches!(e, Event::Buffer(BufferEvent::Start { .. })))
        {
            *start = Some(host.now);
        }
    };
    for part in body.chunks(TS_CHUNK) {
        host.feed_playing(part, TS_CHUNK);
        observe(&host, &mut first_sample, &mut start);
    }
    for _ in 0..IDLE_STEPS {
        host.now += IDLE_STEP_US;
        let pos = host.position();
        host.tick(pos);
        observe(&host, &mut first_sample, &mut start);
    }
    (first_sample, start)
}

#[test]
fn a_40_stream_pmt_with_a_never_sent_private_audio_still_starts_within_the_wait() {
    let (first, start) = start_of(&forty_stream_pmt(true));
    let first = first.expect("media flowed");
    let start = start.expect("Start must fire, never wait forever");
    assert!(
        start <= first + 4_000_000 + START_BOUND_AFTER_WAIT_US,
        "start {start} us, first sample {first} us"
    );
}

#[test]
fn the_never_sent_private_audio_is_what_delays_the_start() {
    let (_, without) = start_of(&forty_stream_pmt(false));
    let (_, with) = start_of(&forty_stream_pmt(true));
    let (without, with) = (without.expect("started"), with.expect("started"));
    assert!(with > without, "with {with} us, without {without} us");
    assert!(
        with >= DEFAULT_START_PENDING_TRACK_WAIT_US,
        "the declared audio is waited for: {with} us"
    );
}

#[test]
fn a_zero_wait_removes_the_delay_from_the_never_sent_private_audio() {
    let bytes = forty_stream_pmt(true);
    let (_, default_start) = start_of_with(&bytes, {
        let mut new_config = Config::default();
        new_config.start_pending_track_hard_cap_us = 0;
        new_config
    });
    let (_, zero_start) = start_of_with(&bytes, {
        let mut new_config = Config::default();
        new_config.start_pending_track_wait_us = 0;
        new_config.start_pending_track_hard_cap_us = 0;
        new_config
    });
    let (default_start, zero_start) = (
        default_start.expect("started"),
        zero_start.expect("started"),
    );

    assert!(
        zero_start < DEFAULT_START_PENDING_TRACK_WAIT_US,
        "{zero_start} us"
    );
    assert!(zero_start < default_start);
}

#[test]
fn an_identity_sample_aes_key_on_plain_fmp4_is_fatal_at_once_via_the_core_error() {
    let (init, segs) = parts();
    let p = format!(
        "{VOD_HEAD}#EXT-X-KEY:METHOD=SAMPLE-AES,URI=\"{KEY_URI}\"\n#EXT-X-MAP:URI=\"m.mp4\"\n#EXTINF:1.0,\ns0.m4s\n#EXT-X-ENDLIST\n"
    );
    let mut h = Host::new(PL, hls_config());
    h.ok(PL, p.into_bytes());
    h.ok(&format!("{BASE}/m.mp4"), init);
    h.ok(&format!("{BASE}/s0.m4s"), segs[0].clone());
    h.run_until(stop_on_end_or_fatal, STEPS);
    let t = tally(&h.events);
    assert_eq!(t.samples, 0, "{t:?}");
    assert!(t.non_fatals.is_empty(), "fatal at once: {t:?}");
    assert_eq!(
        t.fatals,
        vec![ErrorKind::UnsupportedEncryption {
            method: "SAMPLE-AES".to_string()
        }]
    );
}
