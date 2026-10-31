//! WebVTT subtitle renditions from DVB teletext (issue #1428), driven through
//! the production `ProgramSegmenter::pump` path and read back over the axum
//! router the way a player would.
//!
//! TSDuck's `test-012.ts` (French teletext PID 0x42C: page 888 type 0x05, page
//! 889 type 0x02) is the real teletext source, but the committed capture holds
//! only the teletext PID with its PAT and PMT, no video. So the rig takes the
//! video and audio of a real H.264 capture (`rai-gulp-8MB.ts`, one IDR per
//! second) and adds `test-012.ts`'s teletext track, with its PMT descriptors
//! and PES bytes untouched, on a PTS timeline shifted by a constant so that it
//! starts shortly after the video. The shift is the only synthetic part, and
//! every timing check below is computed from the shifted track.

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::num::NonZeroUsize;
use std::sync::Arc;
use std::time::Duration;

use axum::Router;
use axum::body::Body;
use axum::http::{Request, StatusCode, header};
use broadcast_common::mux::Unpackage;
use broadcast_hls::{MasterPlaylist, MediaPlaylist, MediaRendition, MediaType};
use media_doctor::{Report, check_hls_playlist};
use media_plane::ProgramId;
use media_plane::trunk::{RetentionClass, Trunk, TrunkConfig};
use tower::ServiceExt;
use transmux::pipeline::CodecConfig;
use transmux::ts_demux::TsDemux;

use multimux::origin::{AppState, router};
use multimux::output::Output;
use multimux::output::llhls::LlHlsOutput;
use multimux::route::RouteHandle;
use multimux::source::segment::ProgramSegmenter;

const TARGET_DURATION_SECS: f64 = 1.0;
const PART_TARGET_MS: u32 = 200;
const WINDOW_SEGMENTS: usize = 64;
const RING: usize = 1 << 16;
const SIDE_RINGS: usize = 64;
const TELETEXT_MUX_DELAY_SECS: f64 = 0.3;
const DECLARED_BPS: u64 = 16_000_000;
const STREAM: &str = "cam";
const TELETEXT_FIXTURE: &str = "input/test-012.ts";
const VIDEO_FIXTURE: &str = "zenith-live/rai-gulp-8MB.ts";
const TELETEXT_TRACK_ID: u32 = 99;
const TELETEXT_STARTS_AFTER_VIDEO_TICKS: i64 = 27_000;
const TIMING_REFERENCE: &str = "reference/test-012/test-012.tsp344.srt";
const NO_TELETEXT_FIXTURE: &str = "ts/transformers-eia608-h264.ts";
const SUBTITLES_GROUP: &str = "subs";
const VTT_CONTENT_TYPE: &str = "text/vtt";
const TIMESTAMP_MAP_PREFIX: &str = "X-TIMESTAMP-MAP=LOCAL:00:00:00.000,MPEGTS:";
const HEARING_IMPAIRED: [&str; 2] = [
    "public.accessibility.transcribes-spoken-dialog",
    "public.accessibility.describes-music-and-sound",
];
const TICKS_PER_MS: i64 = 90;
const WEBVTT_ROUNDING_MS: f64 = 1.0;
const HOLD_MARGIN_SECS: f64 = 0.2;
const CLAMP_TOLERANCE_SECS: f64 = 0.06;
const TICKS_PER_SEC: f64 = 90_000.0;
const EXTINF_TOLERANCE_SECS: f64 = 0.001;
const VALIDATOR_BIN: &str = "/usr/local/bin/mediastreamvalidator";
const VALIDATOR_DEADLINE: Duration = Duration::from_secs(60);
const VALIDATOR_TARGET_SECS: &str = "5";
const MUST_LEVEL: i64 = 1;
/// What Apple's validator says of a low-latency playlist that carries no
/// partial segments, which is what a WebVTT subtitle playlist is today: it
/// wants `EXT-X-PART-INF` and the part-related `EXT-X-SERVER-CONTROL`
/// attributes in every playlist of a low-latency stream. These are the only
/// findings the subtitle playlists add.
const LL_TAGS_OF_PARTLESS_PLAYLIST: [&str; 2] = [
    "Missing EXT-X-PART-INF tag",
    "Missing server control attribute",
];

fn nz(n: usize) -> NonZeroUsize {
    NonZeroUsize::new(n).expect("non-zero")
}

struct Rig {
    app: Router,
    media: transmux::Media,
}

fn demux(bytes: &[u8]) -> transmux::Media {
    TsDemux::new().unpackage(bytes).expect("demux fixture")
}

fn compose(video: &[u8], teletext: &[u8]) -> transmux::Media {
    let mut media = demux(video);
    media
        .tracks
        .retain(|t| !matches!(t.spec.config, CodecConfig::Data { .. }));
    let first_video_dts = first_video_sync_dts(&media);
    let mut teletext = demux(teletext)
        .tracks
        .into_iter()
        .find(|t| matches!(t.spec.config, CodecConfig::Data { .. }))
        .expect("teletext track");
    let first_pts = teletext.samples[0].pts.expect("teletext PES PTS");
    let shift = first_video_dts + TELETEXT_STARTS_AFTER_VIDEO_TICKS - first_pts;
    for sample in &mut teletext.samples {
        sample.pts = sample.pts.map(|p| p + shift);
        sample.dts = sample.dts.map(|d| d + shift);
    }
    teletext.spec.track_id = transmux::TrackId::new(TELETEXT_TRACK_ID);
    media.tracks.push(teletext);
    media
}

fn build_from(media: transmux::Media, subtitles: bool) -> Rig {
    let trunk = Trunk::new(TrunkConfig::new(
        nz(RING),
        nz(SIDE_RINGS),
        nz(WINDOW_SEGMENTS),
        nz(SIDE_RINGS),
        nz(RING),
    ));
    let writer = trunk.writer().expect("trunk writer");
    writer.set_tracks(media.tracks.iter().map(|t| t.spec.clone()).collect());
    let route = Arc::new(
        RouteHandle::new(TARGET_DURATION_SECS, PART_TARGET_MS, WINDOW_SEGMENTS)
            .with_declared_bandwidth_bps(DECLARED_BPS)
            .with_subtitles(subtitles),
    );
    route.publish_program(ProgramId(0), Arc::clone(&trunk));
    route.set_track_specs(ProgramId(0), trunk.tracks().to_vec());
    let mut segmenter =
        ProgramSegmenter::try_new(&trunk, &route, TARGET_DURATION_SECS, PART_TARGET_MS)
            .expect("segmenter");
    if let Some(init) = segmenter.init_segment() {
        route.set_init(ProgramId(0), init);
    }
    let mut order: Vec<(f64, usize, usize)> = Vec::new();
    for (ti, track) in media.tracks.iter().enumerate() {
        for (si, s) in track.samples.iter().enumerate() {
            let at = s.dts.or(s.pts).unwrap_or(0);
            let delay = if matches!(track.spec.config, CodecConfig::Data { .. }) {
                TELETEXT_MUX_DELAY_SECS
            } else {
                0.0
            };
            order.push((at as f64 / f64::from(track.spec.timescale) + delay, ti, si));
        }
    }
    order.sort_by(|a, b| a.0.total_cmp(&b.0));
    for (_, ti, si) in order {
        let track = &media.tracks[ti];
        let mut sample = track.samples[si].clone();
        sample.dts = sample.dts.or(sample.pts);
        writer.publish(track.spec.track_id, RetentionClass::Timed, sample);
        segmenter.pump(ProgramId(0), &trunk, &route);
    }
    segmenter.pump(ProgramId(0), &trunk, &route);
    segmenter.flush();

    let mut streams = HashMap::new();
    streams.insert(
        STREAM.to_string(),
        (
            Arc::clone(&route),
            vec![Arc::new(LlHlsOutput::default()) as Arc<dyn Output>],
        ),
    );
    Rig {
        app: router(Arc::new(AppState::new(streams))),
        media,
    }
}

fn teletext_rig(subtitles: bool, who: &str) -> Option<Rig> {
    let teletext = std::fs::read(test_fixtures::tsduck(TELETEXT_FIXTURE, who)?)
        .expect("read teletext fixture");
    let video = test_fixtures::require_bytes(VIDEO_FIXTURE, who)?;
    Some(build_from(compose(&video, &teletext), subtitles))
}

fn no_teletext_rig(subtitles: bool, who: &str) -> Option<Rig> {
    let bytes = test_fixtures::require_bytes(NO_TELETEXT_FIXTURE, who)?;
    Some(build_from(demux(&bytes), subtitles))
}

impl Rig {
    async fn get_full(&self, path: &str) -> (StatusCode, Option<String>, Vec<u8>) {
        let resp = self
            .app
            .clone()
            .oneshot(
                Request::builder()
                    .uri(format!("/{STREAM}/{path}"))
                    .body(Body::empty())
                    .expect("request"),
            )
            .await
            .expect("response");
        let status = resp.status();
        let content_type = resp
            .headers()
            .get(header::CONTENT_TYPE)
            .and_then(|v| v.to_str().ok())
            .map(str::to_owned);
        let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .expect("body")
            .to_vec();
        (status, content_type, body)
    }

    async fn get(&self, path: &str) -> (StatusCode, Vec<u8>) {
        let (status, _, body) = self.get_full(path).await;
        (status, body)
    }

    async fn ok_text(&self, path: &str) -> String {
        let (status, body) = self.get(path).await;
        assert_eq!(status, StatusCode::OK, "GET {path}");
        String::from_utf8(body).expect("utf8")
    }

    async fn master_text(&self) -> String {
        self.ok_text("master.m3u8").await
    }

    async fn master(&self) -> MasterPlaylist {
        MasterPlaylist::parse(&self.master_text().await).expect("master parses")
    }

    async fn playlist(&self, path: &str) -> (String, MediaPlaylist) {
        let text = self.ok_text(path).await;
        let parsed = MediaPlaylist::parse(&text)
            .unwrap_or_else(|e| panic!("{path} must parse: {e}\n{text}"));
        (text, parsed)
    }

    async fn subtitle_uri(&self, hearing_impaired: bool) -> String {
        let master = self.master().await;
        subtitle_renditions(&master)
            .into_iter()
            .find(|m| is_hearing_impaired(m) == hearing_impaired)
            .and_then(|m| m.uri.clone())
            .unwrap_or_else(|| {
                panic!("no subtitle rendition (hearing_impaired={hearing_impaired})")
            })
    }
}

fn subtitle_renditions(master: &MasterPlaylist) -> Vec<&MediaRendition> {
    master
        .media
        .iter()
        .filter(|m| m.media_type == MediaType::Subtitles)
        .collect()
}

fn is_hearing_impaired(m: &MediaRendition) -> bool {
    HEARING_IMPAIRED
        .iter()
        .all(|c| m.characteristics.iter().any(|x| x == c))
}

struct Vtt {
    mpegts: u64,
    cues: Vec<VttCue>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct VttCue {
    start_ms: i64,
    end_ms: i64,
    text: String,
}

fn parse_vtt_time(s: &str) -> i64 {
    let (hms, ms) = s.trim().split_once('.').expect("WebVTT time has a dot");
    let mut fields = hms
        .split(':')
        .map(|p| p.parse::<i64>().expect("WebVTT time field"));
    let (h, m, sec) = match (fields.next(), fields.next(), fields.next()) {
        (Some(h), Some(m), Some(s)) => (h, m, s),
        other => panic!("WebVTT time {s:?} is not hh:mm:ss.ttt: {other:?}"),
    };
    assert_eq!(ms.len(), 3, "three millisecond digits in {s:?}");
    ((h * 60 + m) * 60 + sec) * 1000 + ms.parse::<i64>().expect("milliseconds")
}

fn parse_vtt(body: &str) -> Vtt {
    let mut lines = body.split('\n');
    assert_eq!(lines.next(), Some("WEBVTT"), "signature line:\n{body}");
    let map = lines.next().expect("timestamp map line");
    let mpegts = map
        .strip_prefix(TIMESTAMP_MAP_PREFIX)
        .unwrap_or_else(|| panic!("X-TIMESTAMP-MAP LOCAL first, then MPEGTS: {map:?}"))
        .parse::<u64>()
        .expect("MPEGTS is a decimal number");
    assert_eq!(
        lines.next(),
        Some(""),
        "blank line ends the header:\n{body}"
    );
    let rest: Vec<&str> = lines.collect();
    let mut cues = Vec::new();
    for block in rest.split(|l| l.is_empty()).filter(|b| !b.is_empty()) {
        let (start, end) = block[0]
            .split_once(" --> ")
            .unwrap_or_else(|| panic!("cue timing line {:?} in\n{body}", block[0]));
        let (start_ms, end_ms) = (parse_vtt_time(start), parse_vtt_time(end));
        assert!(end_ms > start_ms, "cue end after start: {:?}", block[0]);
        let text = block[1..]
            .join("\n")
            .replace("&lt;", "<")
            .replace("&gt;", ">")
            .replace("&amp;", "&");
        cues.push(VttCue {
            start_ms,
            end_ms,
            text,
        });
    }
    Vtt { mpegts, cues }
}

fn seq_of(uri: &str) -> u64 {
    let stem = uri.rsplit_once('.').expect("extension").0;
    stem.rsplit('-')
        .next()
        .expect("sequence")
        .parse()
        .unwrap_or_else(|_| panic!("segment sequence in {uri}"))
}

async fn fetch_vtt_segments(rig: &Rig, playlist: &MediaPlaylist) -> Vec<Vtt> {
    let mut segments = Vec::new();
    for segment in &playlist.segments {
        let (status, content_type, body) = rig.get_full(&segment.uri).await;
        assert_eq!(status, StatusCode::OK, "GET {}", segment.uri);
        assert_eq!(
            content_type.as_deref(),
            Some(VTT_CONTENT_TYPE),
            "{} content type",
            segment.uri
        );
        segments.push(parse_vtt(&String::from_utf8(body).expect("utf8 vtt")));
    }
    segments
}

fn normalise_text(text: &str) -> String {
    text.lines()
        .map(str::trim_end)
        .collect::<Vec<_>>()
        .join("\n")
}

fn parse_srt_time(s: &str) -> i64 {
    let (hms, ms) = s.trim().split_once(',').expect("SRT time has a comma");
    let mut parts = hms
        .split(':')
        .map(|p| p.parse::<i64>().expect("SRT time field"));
    let (h, m, sec) = (
        parts.next().expect("hours"),
        parts.next().expect("minutes"),
        parts.next().expect("seconds"),
    );
    ((h * 60 + m) * 60 + sec) * 1000 + ms.parse::<i64>().expect("SRT milliseconds")
}

fn reference_cues(who: &str) -> Option<Vec<VttCue>> {
    let path = test_fixtures::tsduck(TIMING_REFERENCE, who)?;
    let srt = std::fs::read_to_string(path)
        .expect("read timing reference")
        .replace("\r\n", "\n");
    Some(
        srt.split("\n\n")
            .filter(|b| !b.trim().is_empty())
            .map(|block| {
                let mut lines = block.lines();
                let _index = lines.next();
                let (a, b) = lines
                    .next()
                    .and_then(|l| l.split_once("-->"))
                    .expect("SRT timing line");
                VttCue {
                    start_ms: parse_srt_time(a),
                    end_ms: parse_srt_time(b),
                    text: normalise_text(&lines.collect::<Vec<_>>().join("\n")),
                }
            })
            .collect(),
    )
}

fn be32(b: &[u8], at: usize) -> usize {
    u32::from_be_bytes([b[at], b[at + 1], b[at + 2], b[at + 3]]) as usize
}

fn child<'a>(bytes: &'a [u8], kind: &[u8; 4]) -> Option<&'a [u8]> {
    let mut at = 0;
    while at + 8 <= bytes.len() {
        let size = be32(bytes, at);
        if size < 8 || at + size > bytes.len() {
            return None;
        }
        if &bytes[at + 4..at + 8] == kind {
            return Some(&bytes[at + 8..at + size]);
        }
        at += size;
    }
    None
}

fn first_tfdt(segment: &[u8]) -> u64 {
    let moof = child(segment, b"moof").expect("moof");
    let moof = transmux::movie_fragment::MovieFragmentBox::parse_body(moof).expect("moof parses");
    moof.traf
        .first()
        .and_then(|t| t.tfdt.as_ref())
        .expect("tfdt")
        .base_media_decode_time()
}

#[tokio::test]
async fn master_declares_teletext_subtitle_renditions() {
    let Some(rig) = teletext_rig(true, "master_declares_teletext_subtitle_renditions") else {
        return;
    };
    let text = rig.master_text().await;
    let master = MasterPlaylist::parse(&text).expect("master parses");
    let subs = subtitle_renditions(&master);
    assert_eq!(subs.len(), 2, "{text}");

    let plain = subs
        .iter()
        .find(|m| !is_hearing_impaired(m))
        .unwrap_or_else(|| panic!("no plain subtitle rendition (page 889):\n{text}"));
    assert_eq!(plain.language.as_deref(), Some("fr"));
    assert_eq!(plain.group_id, SUBTITLES_GROUP);
    assert!(plain.characteristics.is_empty(), "{text}");

    let sdh = subs
        .iter()
        .find(|m| is_hearing_impaired(m))
        .unwrap_or_else(|| panic!("no hearing-impaired rendition (page 888):\n{text}"));
    assert_eq!(sdh.language.as_deref(), Some("fr"));
    assert!(
        sdh.autoselect,
        "AUTOSELECT=YES on the SDH rendition:\n{text}"
    );
    assert_ne!(sdh.name, plain.name, "NAME unique in the group");
    assert!(sdh.name.ends_with("(SDH)"), "{}", sdh.name);
    assert!(subs.iter().all(|m| !m.default), "DEFAULT=NO:\n{text}");

    assert!(!master.variants.is_empty());
    for variant in &master.variants {
        assert_eq!(
            variant.subtitles.as_deref(),
            Some(SUBTITLES_GROUP),
            "every STREAM-INF references the group:\n{text}"
        );
    }
    assert!(text.contains("wvtt"), "wvtt in CODECS:\n{text}");
    assert_eq!(
        master.to_m3u8().expect("re-serialise"),
        text,
        "parse then serialise reproduces the served bytes"
    );

    let mut report = Report::new();
    check_hls_playlist(&text, &mut report);
    assert!(
        report.is_empty(),
        "master findings: {:?}",
        report.findings()
    );
}

#[tokio::test]
async fn subtitle_playlist_matches_video_segments() {
    let Some(rig) = teletext_rig(true, "subtitle_playlist_matches_video_segments") else {
        return;
    };
    let (_, video) = rig.playlist("media.m3u8").await;
    for hearing_impaired in [false, true] {
        let uri = rig.subtitle_uri(hearing_impaired).await;
        let (text, subs) = rig.playlist(&uri).await;
        assert_eq!(subs.media_sequence, video.media_sequence, "{uri}");
        assert_eq!(subs.target_duration, video.target_duration, "{uri}");
        assert_eq!(subs.segments.len(), video.segments.len(), "{uri}");
        assert!(!text.contains("#EXT-X-MAP"), "no init segment:\n{text}");
        assert!(video.segments.len() > 10, "a real run of segments");
        for (v, s) in video.segments.iter().zip(&subs.segments) {
            assert_eq!(seq_of(&s.uri), seq_of(&v.uri), "media sequence number");
            assert!(s.map.is_none());
            assert!(s.uri.ends_with(".vtt"), "{}", s.uri);
            assert!(
                (s.duration.get() - v.duration.get()).abs() <= EXTINF_TOLERANCE_SECS,
                "{uri}: EXTINF {} vs video {}",
                s.duration.get(),
                v.duration.get()
            );
        }
        fetch_vtt_segments(&rig, &subs).await;

        let mut report = Report::new();
        check_hls_playlist(&text, &mut report);
        assert!(report.is_empty(), "{uri} findings: {:?}", report.findings());
    }
}

#[tokio::test]
async fn every_segment_has_webvtt_header_and_timestamp_map() {
    let Some(rig) = teletext_rig(true, "every_segment_has_webvtt_header_and_timestamp_map") else {
        return;
    };
    for hearing_impaired in [false, true] {
        let uri = rig.subtitle_uri(hearing_impaired).await;
        let (_, subs) = rig.playlist(&uri).await;
        let segments = fetch_vtt_segments(&rig, &subs).await;
        let maps: BTreeSet<u64> = segments.iter().map(|s| s.mpegts).collect();
        assert_eq!(
            maps.len(),
            1,
            "{uri}: one MPEGTS for every segment: {maps:?}"
        );
    }
    let uri = rig.subtitle_uri(false).await;
    let (_, subs) = rig.playlist(&uri).await;
    let cues: usize = fetch_vtt_segments(&rig, &subs)
        .await
        .iter()
        .map(|s| s.cues.len())
        .sum();
    assert!(cues > 0, "the subtitle page carries cues");
}

fn first_video_sync_dts(media: &transmux::Media) -> i64 {
    let video = media
        .tracks
        .iter()
        .find(|t| matches!(t.spec.config, CodecConfig::Avc { .. }))
        .expect("video track");
    video
        .samples
        .iter()
        .find(|s| s.flags.is_sync)
        .and_then(|s| s.dts.or(s.pts))
        .expect("video sync sample")
}

fn teletext_pts(media: &transmux::Media) -> Vec<i64> {
    media
        .tracks
        .iter()
        .find(|t| matches!(t.spec.config, CodecConfig::Data { .. }))
        .expect("teletext track")
        .samples
        .iter()
        .map(|s| s.pts.expect("teletext PES PTS"))
        .collect()
}

fn max_pes_gap_ms(pts: &[i64]) -> i64 {
    pts.windows(2)
        .map(|w| (w[1] - w[0]) / TICKS_PER_MS)
        .max()
        .expect("two teletext PES")
}

/// Where the TSDuck reference lands on the video's output timeline.
///
/// A cue's presentation time on that timeline is
/// `cue time - LOCAL + MPEGTS / 90000`. The reference counts seconds from the
/// first teletext PES PTS (TSDuck's `TimeTrackerDemux` keeps the first PTS it
/// sees on the PID), so a reference time `r` is the source PTS
/// `first_teletext_pts + r` (on the rig's shifted teletext timeline). The
/// video's first segment begins at the first sync sample: its `tfdt` is the
/// output time of that sample's decode time, so source time `p` is at output
/// time `tfdt / timescale + (p - first_dts) / 90000`.
struct OutputMap {
    tfdt_secs: f64,
    first_dts: i64,
    reference_zero: i64,
    tolerance_ms: f64,
}

impl OutputMap {
    async fn new(rig: &Rig) -> Self {
        let (_, video) = rig.playlist("media.m3u8").await;
        let first_video_segment = rig.get(&video.segments[0].uri).await.1;
        let video_timescale = rig
            .media
            .tracks
            .iter()
            .find(|t| matches!(t.spec.config, CodecConfig::Avc { .. }))
            .map(|t| f64::from(t.spec.timescale))
            .expect("video track");
        let teletext = teletext_pts(&rig.media);
        OutputMap {
            tfdt_secs: first_tfdt(&first_video_segment) as f64 / video_timescale,
            first_dts: first_video_sync_dts(&rig.media),
            reference_zero: teletext[0],
            tolerance_ms: (max_pes_gap_ms(&teletext) as f64) + WEBVTT_ROUNDING_MS,
        }
    }

    fn reference_secs(&self, reference_ms: i64) -> f64 {
        let source = self.reference_zero as f64 + reference_ms as f64 * TICKS_PER_MS as f64;
        self.tfdt_secs + (source - self.first_dts as f64) / TICKS_PER_SEC
    }
}

fn presented_secs(vtt: &Vtt, cue_ms: i64) -> f64 {
    cue_ms as f64 / 1000.0 + vtt.mpegts as f64 / TICKS_PER_SEC
}

/// Tolerance: the largest gap between teletext PES PTS values, computed from
/// the stream, plus the 1 ms WebVTT timestamps are rounded to.
#[tokio::test]
async fn cue_times_line_up_with_the_video_timeline() {
    let who = "cue_times_line_up_with_the_video_timeline";
    let (Some(rig), Some(reference)) = (teletext_rig(true, who), reference_cues(who)) else {
        return;
    };
    let map = OutputMap::new(&rig).await;
    let uri = rig.subtitle_uri(false).await;
    let (_, subs) = rig.playlist(&uri).await;
    let mut starts: BTreeMap<String, f64> = BTreeMap::new();
    for segment in fetch_vtt_segments(&rig, &subs).await {
        for cue in &segment.cues {
            starts
                .entry(cue.text.clone())
                .or_insert(presented_secs(&segment, cue.start_ms));
        }
    }
    assert_eq!(starts.len(), reference.len(), "{starts:?}");
    for want in &reference {
        let expected = map.reference_secs(want.start_ms);
        let got = starts
            .get(&want.text)
            .unwrap_or_else(|| panic!("no cue {:?} in {starts:?}", want.text));
        let delta_ms = (got - expected) * 1000.0;
        assert!(
            delta_ms.abs() <= map.tolerance_ms,
            "{:?}: cue at {got:.3}s vs reference {expected:.3}s on the video timeline \
             ({delta_ms:.0} ms, tolerance {} ms)",
            want.text,
            map.tolerance_ms
        );
    }
}

/// Every cue is delivered with its own start and end, whichever segments it
/// crosses. The capture ends with its last cue still on screen (no erase
/// follows), so that cue's end is the end of the stream, not the reference's.
#[tokio::test]
async fn all_cues_are_delivered_once_with_full_display_times() {
    let who = "all_cues_are_delivered_once_with_full_display_times";
    let (Some(rig), Some(reference)) = (teletext_rig(true, who), reference_cues(who)) else {
        return;
    };
    let map = OutputMap::new(&rig).await;
    let uri = rig.subtitle_uri(false).await;
    let (_, subs) = rig.playlist(&uri).await;
    let mut copies: BTreeMap<String, Vec<(f64, f64)>> = BTreeMap::new();
    let mut first_copy_segment_end: BTreeMap<String, f64> = BTreeMap::new();
    let mut segment_end_secs = map.tfdt_secs;
    let mut longest_segment_secs: f64 = 0.0;
    let vtts = fetch_vtt_segments(&rig, &subs).await;
    for (segment, entry) in vtts.iter().zip(&subs.segments) {
        segment_end_secs += entry.duration.get();
        longest_segment_secs = longest_segment_secs.max(entry.duration.get());
        for cue in &segment.cues {
            first_copy_segment_end
                .entry(cue.text.clone())
                .or_insert(segment_end_secs);
            copies.entry(cue.text.clone()).or_default().push((
                presented_secs(segment, cue.start_ms),
                presented_secs(segment, cue.end_ms),
            ));
        }
    }
    let mut delivered: Vec<(String, f64, f64)> = Vec::new();
    for (text, cues) in &copies {
        let starts: BTreeSet<i64> = cues.iter().map(|c| (c.0 * 1000.0).round() as i64).collect();
        assert_eq!(
            starts.len(),
            1,
            "{text:?} keeps its start in every segment: {cues:?}"
        );
        let end = cues.iter().map(|c| c.1).fold(f64::MIN, f64::max);
        delivered.push((text.clone(), cues[0].0, end));
    }
    delivered.sort_by(|a, b| a.1.total_cmp(&b.1));
    assert_eq!(delivered.len(), reference.len(), "{delivered:?}");
    let got_text: Vec<&str> = delivered.iter().map(|c| c.0.as_str()).collect();
    let want_text: Vec<&str> = reference.iter().map(|c| c.text.as_str()).collect();
    assert_eq!(got_text, want_text);

    for (got, want) in delivered.iter().zip(&reference).take(reference.len() - 1) {
        let first_end = copies[&got.0][0].1;
        let segment_end = first_copy_segment_end[&got.0];
        let real_end = map.reference_secs(want.end_ms);
        if real_end
            <= segment_end + TARGET_DURATION_SECS - TELETEXT_MUX_DELAY_SECS - HOLD_MARGIN_SECS
        {
            assert!(
                (first_end - real_end).abs() * 1000.0 <= map.tolerance_ms,
                "{:?}: closed {:.3}s after the end of its first segment, within the hold, so the \
                 first copy must carry the real end {real_end:.3}s, not {first_end:.3}s",
                want.text,
                real_end - segment_end
            );
        } else if real_end
            >= segment_end + TARGET_DURATION_SECS + longest_segment_secs + HOLD_MARGIN_SECS
        {
            assert!(
                (first_end - segment_end).abs() <= CLAMP_TOLERANCE_SECS,
                "{:?}: still open past the hold, so the first copy ends at its segment end \
                 {segment_end:.3}s, not {first_end:.3}s",
                want.text
            );
        }
    }

    let last = reference.len() - 1;
    for (i, (got, want)) in delivered.iter().zip(&reference).enumerate() {
        let want_end = map.reference_secs(want.end_ms);
        let delta_ms = (got.2 - want_end) * 1000.0;
        if i == last {
            assert!(
                delta_ms >= -map.tolerance_ms,
                "{:?}: the cue on screen at the end of the capture ends at {:.3}s, before the \
                 reference {want_end:.3}s",
                want.text,
                got.2
            );
        } else {
            assert!(
                delta_ms.abs() <= map.tolerance_ms,
                "{:?}: end {:.3}s vs reference {want_end:.3}s ({delta_ms:.0} ms, tolerance {} ms)",
                want.text,
                got.2,
                map.tolerance_ms
            );
        }
    }
}

fn strip_subtitle_signalling(master: &str) -> String {
    master
        .lines()
        .filter(|l| !l.contains("TYPE=SUBTITLES"))
        .map(|l| l.replace(",SUBTITLES=\"subs\"", "").replace(",wvtt", ""))
        .collect::<Vec<_>>()
        .join("\n")
}

#[tokio::test]
async fn subtitles_flag_off_declares_nothing() {
    let who = "subtitles_flag_off_declares_nothing";
    let (Some(on), Some(off)) = (teletext_rig(true, who), teletext_rig(false, who)) else {
        return;
    };
    let text = off.master_text().await;
    assert!(!text.contains("SUBTITLES"), "{text}");
    assert!(!text.contains("wvtt"), "{text}");
    let master = MasterPlaylist::parse(&text).expect("master parses");
    assert!(subtitle_renditions(&master).is_empty());
    assert!(master.variants.iter().all(|v| v.subtitles.is_none()));
    assert_eq!(
        strip_subtitle_signalling(&on.master_text().await).trim_end(),
        text.trim_end(),
        "apart from the subtitle signalling the master is the same"
    );
    assert_eq!(off.get("subs-0.m3u8").await.0, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn stream_without_teletext_declares_no_subtitles() {
    let who = "stream_without_teletext_declares_no_subtitles";
    let (Some(on), Some(off)) = (no_teletext_rig(true, who), no_teletext_rig(false, who)) else {
        return;
    };
    let text = on.master_text().await;
    assert!(!text.contains("SUBTITLES"), "{text}");
    assert!(!text.contains("wvtt"), "{text}");
    assert_eq!(text, off.master_text().await);
    assert_eq!(on.get("subs-0.m3u8").await.0, StatusCode::NOT_FOUND);
}

fn collect_findings(v: &serde_json::Value, path: &str, out: &mut Vec<(i64, String)>) {
    match v {
        serde_json::Value::Object(map) => {
            let parse_failed = map
                .get("parseFailed")
                .and_then(|v| v.as_bool())
                .unwrap_or(false);
            if let Some(serde_json::Value::Array(messages)) = map.get("messages") {
                for m in messages {
                    let level = m
                        .get("errorRequirementLevel")
                        .and_then(|v| v.as_i64())
                        .unwrap_or(0);
                    let comment = m.get("errorComment").and_then(|v| v.as_str()).unwrap_or("");
                    out.push((
                        if parse_failed { MUST_LEVEL } else { level },
                        format!("{path}: {comment}"),
                    ));
                }
            }
            for (k, v) in map {
                if k != "messages" {
                    collect_findings(v, &format!("{path}/{k}"), out);
                }
            }
        }
        serde_json::Value::Array(items) => {
            for (i, v) in items.iter().enumerate() {
                collect_findings(v, &format!("{path}[{i}]"), out);
            }
        }
        _ => {}
    }
}

async fn validator_findings(rig: &Rig, tag: &str, playlists: &[&str]) -> Vec<(i64, String)> {
    use std::fs;
    use std::process::Command;

    let scratch = std::env::temp_dir().join(format!(
        "subtitle-webvtt-validator-{}-{tag}",
        std::process::id()
    ));
    let _ = fs::remove_dir_all(&scratch);
    fs::create_dir_all(&scratch).expect("scratch");

    let mut uris = BTreeSet::new();
    for name in playlists {
        let (status, body) = rig.get(name).await;
        assert_eq!(status, StatusCode::OK, "GET {name}");
        fs::write(scratch.join(name), &body).expect("write playlist");
        if *name == "master.m3u8" {
            continue;
        }
        for line in String::from_utf8(body).expect("utf8").lines() {
            if let Some(start) = line.find("URI=\"") {
                let rest = &line[start + 5..];
                if let Some(end) = rest.find('"') {
                    uris.insert(rest[..end].to_string());
                }
            }
            let t = line.trim();
            if !t.is_empty() && !t.starts_with('#') {
                uris.insert(t.to_string());
            }
        }
    }
    for uri in &uris {
        let (status, body) = rig.get(uri).await;
        if status == StatusCode::OK {
            fs::write(scratch.join(uri), body).expect("write resource");
        }
    }

    let report = scratch.join("report.json");
    let mut cmd = Command::new(VALIDATOR_BIN);
    cmd.current_dir(&scratch)
        .arg("--quiet")
        .arg("-t")
        .arg(VALIDATOR_TARGET_SECS)
        .arg("-O")
        .arg(&report)
        .arg("master.m3u8");
    let _ = test_bounded::output_bounded(&mut cmd, VALIDATOR_DEADLINE).expect("run validator");
    let json: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(&report).expect("report")).expect("json");
    let _ = fs::remove_dir_all(&scratch);
    let mut findings = Vec::new();
    collect_findings(&json, "$", &mut findings);
    findings
}

#[tokio::test]
async fn apple_validator_reports_nothing_about_subtitles() {
    let who = "apple_validator_reports_nothing_about_subtitles";
    if !std::path::Path::new(VALIDATOR_BIN).exists() {
        eprintln!("SKIP {who}: {VALIDATOR_BIN} absent");
        return;
    }
    let (Some(on), Some(off)) = (teletext_rig(true, who), teletext_rig(false, who)) else {
        return;
    };
    let playlists = [
        "master.m3u8",
        "media.m3u8",
        "iframe.m3u8",
        "subs-0.m3u8",
        "subs-1.m3u8",
    ];
    let with = validator_findings(&on, "on", &playlists).await;
    let without = validator_findings(&off, "off", &playlists[..3]).await;
    for (level, message) in &with {
        eprintln!("VALIDATOR[on] level={level} {message}");
    }
    for (level, message) in &without {
        eprintln!("VALIDATOR[off] level={level} {message}");
    }
    let about_subtitles: Vec<_> = with
        .iter()
        .filter(|(_, m)| {
            let m = m.to_ascii_lowercase();
            ["subtitle", "webvtt", "vtt", "subs-"]
                .iter()
                .any(|k| m.contains(k))
        })
        .collect();
    assert!(
        about_subtitles.is_empty(),
        "validator findings about subtitles: {about_subtitles:?}"
    );
    let baseline: BTreeSet<&(i64, String)> = without.iter().collect();
    let new: Vec<_> = with
        .iter()
        .filter(|f| !baseline.contains(f))
        .filter(|(_, m)| !LL_TAGS_OF_PARTLESS_PLAYLIST.iter().any(|k| m.ends_with(k)))
        .collect();
    assert!(new.is_empty(), "findings the subtitles introduced: {new:?}");
}
