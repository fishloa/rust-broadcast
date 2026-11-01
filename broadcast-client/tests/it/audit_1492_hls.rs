use crate::support;

use broadcast_client::{AUDIO_TRACK_ID_OFFSET, Event};
use broadcast_common::{Package, Unpackage};
use support::hls_host::{Host, Reply, SEC, hls_config, ts_segment};
use transmux::{CmafMux, TrackId, TsDemux, build_init_segment};

const PLAYLIST_URL: &str = "http://origin.test/hls/index.m3u8";
const BLOCKING_URL: &str = "http://origin.test/hls/index.m3u8?_HLS_msn=3&_HLS_part=0";
const BLOCKING_SKIP_URL: &str =
    "http://origin.test/hls/index.m3u8?_HLS_msn=3&_HLS_part=0&_HLS_skip=YES";
const RUN_FOR: u64 = 10 * SEC;
const MAX_STEPS: usize = 4_000;

fn full_playlist() -> Vec<u8> {
    "#EXTM3U\n#EXT-X-VERSION:9\n#EXT-X-TARGETDURATION:2\n\
     #EXT-X-SERVER-CONTROL:CAN-BLOCK-RELOAD=YES,CAN-SKIP-UNTIL=12.0\n\
     #EXT-X-PART-INF:PART-TARGET=1.0\n#EXT-X-MEDIA-SEQUENCE:0\n\
     #EXTINF:2.0,\nseg0.ts\n#EXTINF:2.0,\nseg1.ts\n#EXTINF:2.0,\nseg2.ts\n"
        .as_bytes()
        .to_vec()
}

fn unmergeable_delta() -> Vec<u8> {
    "#EXTM3U\n#EXT-X-VERSION:9\n#EXT-X-TARGETDURATION:2\n\
     #EXT-X-SERVER-CONTROL:CAN-BLOCK-RELOAD=YES,CAN-SKIP-UNTIL=12.0\n\
     #EXT-X-PART-INF:PART-TARGET=1.0\n#EXT-X-MEDIA-SEQUENCE:100\n\
     #EXT-X-SKIP:SKIPPED-SEGMENTS=50\n#EXTINF:2.0,\nseg150.ts\n"
        .as_bytes()
        .to_vec()
}

#[test]
fn an_unmergeable_delta_does_not_leave_two_playlist_reload_chains() {
    let mut h = Host::new(PLAYLIST_URL, hls_config());
    h.route(PLAYLIST_URL, vec![Reply::Ok(full_playlist())]);
    h.route(BLOCKING_URL, vec![Reply::Ok(full_playlist()), Reply::Hang]);
    h.route(
        BLOCKING_SKIP_URL,
        vec![Reply::Ok(unmergeable_delta()), Reply::Hang],
    );
    for n in 0..3 {
        h.route(
            &format!("http://origin.test/hls/seg{n}.ts"),
            vec![Reply::Hang],
        );
    }
    for _ in 0..MAX_STEPS {
        if h.now >= RUN_FOR || !h.step() {
            break;
        }
    }
    h.collect();
    assert!(
        h.hits(BLOCKING_SKIP_URL) >= 1,
        "the delta was never requested: {:?}",
        h.seen
    );
    let hung_playlists: Vec<&str> = h
        .hung()
        .iter()
        .filter(|s| s.url.starts_with(PLAYLIST_URL))
        .map(|s| s.url.as_str())
        .collect();
    assert_eq!(
        hung_playlists.len(),
        1,
        "exactly one reload may be outstanding: {hung_playlists:?}"
    );
}

const RANGED_URL: &str = "http://origin.test/hls/ranged.ts";
const RANGED_PLAYLIST_URL: &str = "http://origin.test/hls/ranged.m3u8";

fn ranged_playlist(first_len: usize, second_len: usize) -> Vec<u8> {
    format!(
        "#EXTM3U\n#EXT-X-VERSION:4\n#EXT-X-TARGETDURATION:2\n#EXT-X-MEDIA-SEQUENCE:0\n\
         #EXT-X-PLAYLIST-TYPE:VOD\n#EXTINF:2.0,\n#EXT-X-BYTERANGE:{first_len}@0\nranged.ts\n\
         #EXTINF:2.0,\n#EXT-X-BYTERANGE:{second_len}@{first_len}\nranged.ts\n#EXT-X-ENDLIST\n"
    )
    .into_bytes()
}

fn content_range(start: usize, len: usize, total: usize) -> Vec<(String, String)> {
    vec![(
        "Content-Range".to_string(),
        format!("bytes {start}-{}/{total}", start + len - 1),
    )]
}

struct Ranged {
    first: Vec<u8>,
    second: Vec<u8>,
}

impl Ranged {
    fn new() -> Self {
        Self {
            first: ts_segment(0),
            second: ts_segment(1),
        }
    }

    fn whole(&self) -> Vec<u8> {
        [self.first.clone(), self.second.clone()].concat()
    }

    fn host(&self, replies: Vec<Reply>) -> Host {
        let mut h = Host::new(RANGED_PLAYLIST_URL, hls_config());
        h.ok(
            RANGED_PLAYLIST_URL,
            ranged_playlist(self.first.len(), self.second.len()),
        );
        h.route(RANGED_URL, replies);
        h.run_to_end();
        h
    }

    fn exact_partial_replies(&self) -> Vec<Reply> {
        let total = self.first.len() + self.second.len();
        vec![
            Reply::Full(
                206,
                self.first.clone(),
                content_range(0, self.first.len(), total),
            ),
            Reply::Full(
                206,
                self.second.clone(),
                content_range(self.first.len(), self.second.len(), total),
            ),
        ]
    }
}

#[test]
fn a_range_ignoring_server_answering_200_has_the_sub_range_sliced_out() {
    let r = Ranged::new();
    let reference = r.host(r.exact_partial_replies());
    assert!(reference.samples() > 0, "the reference path must decode");
    let whole = r.whole();
    let sliced = r.host(vec![
        Reply::Full(200, whole.clone(), Vec::new()),
        Reply::Full(200, whole, Vec::new()),
    ]);
    assert_eq!(
        sliced.samples(),
        reference.samples(),
        "each segment must be its own sub-range, not the whole file"
    );
}

#[test]
fn a_200_shorter_than_the_range_end_is_a_failure() {
    let r = Ranged::new();
    let short = r.first[..r.first.len() / 2].to_vec();
    let h = r.host(vec![Reply::Full(200, short, Vec::new())]);
    assert_eq!(h.samples(), 0);
}

#[test]
fn a_206_whose_content_range_disagrees_with_the_request_is_a_failure() {
    let r = Ranged::new();
    let total = r.first.len() + r.second.len();
    let h = r.host(vec![Reply::Full(
        206,
        r.first.clone(),
        content_range(1, r.first.len(), total),
    )]);
    assert_eq!(h.samples(), 0);
}

#[test]
fn a_206_without_content_range_is_a_failure() {
    let r = Ranged::new();
    let h = r.host(vec![Reply::Full(206, r.first.clone(), Vec::new())]);
    assert_eq!(h.samples(), 0);
}

const RESERVED_PLAYLIST_URL: &str = "http://origin.test/hls/reserved.m3u8";
const FIRST_RESERVED_ID: u32 = AUDIO_TRACK_ID_OFFSET + 1;
const MOVIE_TIMESCALE: u32 = 1_000;

fn cmaf_with_track_ids(first_id: u32) -> (Vec<u8>, Vec<u8>) {
    let data = test_fixtures::read("ts/h264_aac.ts");
    let mut media = TsDemux::new().unpackage(&data[..]).expect("demux");
    for (n, track) in media.tracks.iter_mut().enumerate() {
        track.spec.track_id = TrackId::new(first_id + u32::try_from(n).expect("small"));
    }
    let specs: Vec<_> = media.tracks.iter().map(|t| t.spec.clone()).collect();
    let init = build_init_segment(&specs, MOVIE_TIMESCALE).expect("init");
    let combined = CmafMux::new(1).package(&media).expect("cmaf");
    let segment = combined[init.len()..].to_vec();
    (init, segment)
}

fn cmaf_host(first_id: u32) -> Host {
    let (init, segment) = cmaf_with_track_ids(first_id);
    let mut h = Host::new(RESERVED_PLAYLIST_URL, hls_config());
    h.ok(
        RESERVED_PLAYLIST_URL,
        "#EXTM3U\n#EXT-X-VERSION:7\n#EXT-X-TARGETDURATION:10\n#EXT-X-MEDIA-SEQUENCE:0\n\
         #EXT-X-PLAYLIST-TYPE:VOD\n#EXT-X-MAP:URI=\"init.mp4\"\n#EXTINF:10.0,\nseg0.m4s\n\
         #EXT-X-ENDLIST\n"
            .as_bytes()
            .to_vec(),
    );
    h.ok("http://origin.test/hls/init.mp4", init);
    h.ok("http://origin.test/hls/seg0.m4s", segment);
    h.run_to_end();
    h
}

#[test]
fn main_playlist_track_ids_in_the_audio_rendition_range_are_rejected() {
    let h = cmaf_host(FIRST_RESERVED_ID);
    let leaked: Vec<u32> = h
        .events
        .iter()
        .flat_map(|e| match e {
            Event::Tracks(specs) => specs.iter().map(|s| s.track_id.value()).collect(),
            Event::Sample { track, .. } => vec![track.value()],
            _ => Vec::new(),
        })
        .filter(|id| *id >= AUDIO_TRACK_ID_OFFSET)
        .collect();
    assert!(
        leaked.is_empty(),
        "reserved ids leaked to the host: {leaked:?}"
    );
    let errors: Vec<&Event> = h
        .events
        .iter()
        .filter(|e| matches!(e, Event::Error { kind, fatal: false } if kind.name() == "track_id_reserved"))
        .collect();
    assert_eq!(
        errors.len(),
        1,
        "{:?}",
        h.events
            .iter()
            .filter(|e| matches!(e, Event::Error { .. }))
            .collect::<Vec<_>>()
    );
}

#[test]
fn main_playlist_track_ids_below_the_offset_pass_through() {
    let h = cmaf_host(1);
    assert!(h.samples() > 0);
    assert!(
        !h.events.iter().any(|e| matches!(e, Event::Error { .. })),
        "{:?}",
        h.events
    );
}

const MASTER_URL: &str = "http://origin.test/hls/master.m3u8";
const VIDEO_DIR: &str = "http://origin.test/hls/v";
const AUDIO_DIR: &str = "http://origin.test/hls/a";

fn discontinuity_playlist() -> Vec<u8> {
    "#EXTM3U\n#EXT-X-VERSION:3\n#EXT-X-TARGETDURATION:10\n#EXT-X-MEDIA-SEQUENCE:0\n\
     #EXT-X-PLAYLIST-TYPE:VOD\n#EXTINF:10.0,\ns0.ts\n#EXT-X-DISCONTINUITY\n#EXTINF:10.0,\ns1.ts\n\
     #EXT-X-ENDLIST\n"
        .as_bytes()
        .to_vec()
}

fn two_adapter_host() -> Host {
    let mut h = Host::new(MASTER_URL, hls_config());
    h.ok(
        MASTER_URL,
        "#EXTM3U\n#EXT-X-MEDIA:TYPE=AUDIO,GROUP-ID=\"aud\",NAME=\"English\",LANGUAGE=\"en\",\
         DEFAULT=YES,AUTOSELECT=YES,URI=\"a/index.m3u8\"\n\
         #EXT-X-STREAM-INF:BANDWIDTH=800000,AUDIO=\"aud\"\nv/index.m3u8\n"
            .as_bytes()
            .to_vec(),
    );
    for dir in [VIDEO_DIR, AUDIO_DIR] {
        h.ok(&format!("{dir}/index.m3u8"), discontinuity_playlist());
        h.ok(&format!("{dir}/s0.ts"), ts_segment(0));
        h.ok(&format!("{dir}/s1.ts"), ts_segment(1));
    }
    h.run_to_end();
    h
}

#[test]
fn one_logical_discontinuity_reported_by_both_adapters_is_delivered_once() {
    let h = two_adapter_host();
    let discontinuities = h
        .events
        .iter()
        .filter(|e| matches!(e, Event::Discontinuity))
        .count();
    assert!(h.samples() > 0);
    let starts = h
        .events
        .iter()
        .filter(|e| {
            matches!(
                e,
                Event::Buffer(broadcast_client::BufferEvent::Start { .. })
            )
        })
        .count();
    assert_eq!(discontinuities, 1, "{} events", h.events.len());
    assert_eq!(
        starts, 2,
        "one start before the discontinuity, one after it"
    );
}
