mod support;

use std::sync::OnceLock;

use broadcast_client::{Config, ErrorKind, Event, Status};
use sdp_types::{Media, MediaType, TransportProto};
use support::rtsp_host::{BASE_URL, Host, MediaFixture, SEC, Server, interleaved, samples_of};
use transmux::rtp::RtpMediaKind;

const PCMA_STATIC_PAYLOAD: u8 = 8;
const HEVC_PAYLOAD: u8 = 96;
const HEVC_SPROP: &str =
    "96 sprop-vps=QAEMAf//AWAAAAMAgAAAAwAAAwB4lcwJ;sprop-sps=QgEBAWAAAAMAgAAAAwAAAwB4oAeCAFB/FliZ";
const IDLE_SECS: u64 = 120;

fn fixture() -> &'static MediaFixture {
    static FIXTURE: OnceLock<MediaFixture> = OnceLock::new();
    FIXTURE.get_or_init(MediaFixture::load)
}

fn parsed() -> sdp_types::Session {
    sdp_types::Session::parse(fixture().sdp.as_bytes()).unwrap()
}

fn write(session: &sdp_types::Session) -> String {
    let mut out = Vec::new();
    session.write(&mut out).unwrap();
    String::from_utf8(out).unwrap()
}

fn hevc_media() -> Media {
    let mut m = Media::new(MediaType::Video, 0, TransportProto::RtpAvp, HEVC_PAYLOAD);
    m.add_attribute_with_value("rtpmap", "96 H265/90000");
    m.add_attribute_with_value("fmtp", HEVC_SPROP);
    m.add_attribute_with_value("control", "trackID=1");
    m
}

fn pcma_media() -> Media {
    let mut m = Media::new(
        MediaType::Audio,
        0,
        TransportProto::RtpAvp,
        PCMA_STATIC_PAYLOAD,
    );
    m.add_attribute_with_value("control", "trackID=9");
    m
}

fn errors(events: &[Event]) -> Vec<(ErrorKind, bool)> {
    events
        .iter()
        .filter_map(|e| match e {
            Event::Error { kind, fatal } => Some((kind.clone(), *fatal)),
            _ => None,
        })
        .collect()
}

fn dropped(events: &[Event]) -> Vec<u32> {
    errors(events)
        .into_iter()
        .filter_map(|(k, fatal)| match k {
            ErrorKind::TrackDropped { track_id, .. } if !fatal => Some(track_id),
            _ => None,
        })
        .collect()
}

#[test]
fn an_hevc_only_sdp_ends_with_one_fatal_unsupported_codec_and_no_reconnect_loop() {
    let mut session = parsed();
    session.medias = vec![hevc_media()];
    let mut host = Host::new(BASE_URL, Config::default(), Server::new(&write(&session)));
    host.connect();
    host.advance(IDLE_SECS * SEC);
    host.advance(IDLE_SECS * SEC);
    let events = host.events();
    let fatal: Vec<_> = errors(&events).into_iter().filter(|(_, f)| *f).collect();
    assert_eq!(
        fatal,
        vec![(
            ErrorKind::UnsupportedCodec {
                fourcc: "H265".to_string()
            },
            true
        )]
    );
    assert_eq!(dropped(&events).len(), 1);
    assert!(
        !events
            .iter()
            .any(|e| matches!(e, Event::Status(Status::Reconnecting { .. }))),
        "{events:?}"
    );
    assert_eq!(
        host.actions
            .iter()
            .filter(|a| matches!(a, broadcast_client::Action::TcpConnect { .. }))
            .count(),
        1,
        "the session must not dial again"
    );
}

#[test]
fn an_unplayable_media_line_beside_a_playable_one_is_reported_once_and_the_rest_plays() {
    let mut session = parsed();
    session.medias.push(pcma_media());
    let mut host = Host::new(BASE_URL, Config::default(), Server::new(&write(&session)));
    host.connect();
    host.deliver_all(&fixture().frames());
    host.advance(SEC);
    let events = host.events();
    let tracks: Vec<Vec<u32>> = events
        .iter()
        .filter_map(|e| match e {
            Event::Tracks(t) => Some(t.iter().map(|s| s.track_id.value()).collect()),
            _ => None,
        })
        .collect();
    assert_eq!(tracks, vec![vec![1, 2]]);
    assert_eq!(dropped(&events), vec![3]);
    assert!(errors(&events).iter().all(|(_, fatal)| !fatal));
    let per_track = |t: u32| samples_of(&events).iter().filter(|s| s.0 == t).count();
    assert!(per_track(1) > 0 && per_track(2) > 0);

    host.session.reconnect(host.now);
    host.drain();
    host.connect();
    host.deliver_all(&fixture().frames());
    host.advance(SEC);
    assert!(
        dropped(&host.events()).is_empty(),
        "reported once per source, not per connection"
    );
}

#[test]
fn a_reconnect_to_a_server_that_dropped_a_track_announces_only_the_new_tracks() {
    let mut host = Host::new(BASE_URL, Config::default(), Server::new(&fixture().sdp));
    host.connect();
    host.deliver_all(&fixture().frames());
    host.advance(SEC);
    host.events();
    let mut video_only = parsed();
    video_only.medias.retain(|m| m.media == "video");
    host.server.sdp = write(&video_only).into_bytes();
    host.session.reconnect(host.now);
    host.drain();
    host.connect();
    let video = fixture()
        .tracks
        .iter()
        .position(|t| t.kind == RtpMediaKind::H264)
        .unwrap();
    let frames: Vec<Vec<u8>> = fixture().tracks[video]
        .packets
        .iter()
        .map(|p| interleaved(0, p))
        .collect();
    host.deliver_all(&frames);
    host.advance(SEC);
    let events = host.events();
    let announced: Vec<Vec<u32>> = events
        .iter()
        .filter_map(|e| match e {
            Event::Tracks(t) => Some(t.iter().map(|s| s.track_id.value()).collect()),
            _ => None,
        })
        .collect();
    assert_eq!(announced, vec![vec![1]]);
    let after = events
        .iter()
        .rposition(|e| matches!(e, Event::Tracks(_)))
        .unwrap();
    assert!(
        samples_of(&events[after..]).iter().all(|s| s.0 == 1),
        "a sample on a track the new announcement does not name"
    );
}
