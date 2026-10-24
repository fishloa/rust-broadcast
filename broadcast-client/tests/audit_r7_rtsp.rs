//! Round-7 findings F5 and F6(d): the RTSP dropped-media report follows each DESCRIBE, and
//! `TrackDropped.track_id` is the 1-based position of the `m=` line.

mod support;

use std::sync::OnceLock;

use broadcast_client::{Config, ErrorKind, Event};
use sdp_types::{Media, MediaType, TransportProto};
use support::rtsp_host::{BASE_URL, Host, MediaFixture, SEC, Server, samples_of};

const PCMA_STATIC_PAYLOAD: u8 = 8;
const HEVC_PAYLOAD: u8 = 96;
const HEVC_SPROP: &str =
    "96 sprop-vps=QAEMAf//AWAAAAMAgAAAAwAAAwB4lcwJ;sprop-sps=QgEBAWAAAAMAgAAAAwAAAwB4oAeCAFB/FliZ";

fn fixture() -> &'static MediaFixture {
    static FIXTURE: OnceLock<MediaFixture> = OnceLock::new();
    FIXTURE.get_or_init(MediaFixture::load)
}

fn pcma() -> Media {
    let mut m = Media::new(
        MediaType::Audio,
        0,
        TransportProto::RtpAvp,
        PCMA_STATIC_PAYLOAD,
    );
    m.add_attribute_with_value("control", "trackID=9");
    m
}

fn hevc() -> Media {
    let mut m = Media::new(MediaType::Video, 0, TransportProto::RtpAvp, HEVC_PAYLOAD);
    m.add_attribute_with_value("rtpmap", "96 H265/90000");
    m.add_attribute_with_value("fmtp", HEVC_SPROP);
    m.add_attribute_with_value("control", "trackID=9");
    m
}

fn sdp_with(extra: &[(usize, Media)]) -> String {
    let mut session = sdp_types::Session::parse(fixture().sdp.as_bytes()).unwrap();
    for (at, media) in extra {
        session.medias.insert(*at, media.clone());
    }
    let mut out = Vec::new();
    session.write(&mut out).unwrap();
    String::from_utf8(out).unwrap()
}

fn dropped(events: &[Event]) -> Vec<u32> {
    events
        .iter()
        .filter_map(|e| match e {
            Event::Error {
                kind: ErrorKind::TrackDropped { track_id, .. },
                fatal: false,
            } => Some(*track_id),
            _ => None,
        })
        .collect()
}

fn play(host: &mut Host) -> Vec<Event> {
    host.connect();
    host.deliver_all(&fixture().frames());
    host.advance(SEC);
    host.events()
}

fn reconnect_with(host: &mut Host, sdp: &str) -> Vec<Event> {
    host.server.sdp = sdp.as_bytes().to_vec();
    host.session.reconnect(host.now);
    host.drain();
    play(host)
}

#[test]
fn a_different_sdp_on_reconnect_reports_its_own_dropped_media() {
    let first = sdp_with(&[(2, pcma())]);
    let second = sdp_with(&[(2, hevc())]);
    let mut host = Host::new(BASE_URL, Config::default(), Server::new(&first));
    assert_eq!(dropped(&play(&mut host)), vec![3]);
    let again = reconnect_with(&mut host, &second);
    assert_eq!(
        dropped(&again),
        vec![3],
        "a different media at the same position"
    );
}

#[test]
fn the_same_sdp_on_reconnect_reports_nothing_again() {
    let sdp = sdp_with(&[(2, pcma())]);
    let mut host = Host::new(BASE_URL, Config::default(), Server::new(&sdp));
    assert_eq!(dropped(&play(&mut host)), vec![3]);
    assert!(dropped(&reconnect_with(&mut host, &sdp)).is_empty());
    assert!(dropped(&reconnect_with(&mut host, &sdp)).is_empty());
}

#[test]
fn a_media_that_comes_back_and_is_dropped_again_is_reported_again() {
    let with = sdp_with(&[(2, pcma())]);
    let without = sdp_with(&[]);
    let mut host = Host::new(BASE_URL, Config::default(), Server::new(&with));
    assert_eq!(dropped(&play(&mut host)), vec![3]);
    assert!(dropped(&reconnect_with(&mut host, &without)).is_empty());
    assert_eq!(dropped(&reconnect_with(&mut host, &with)), vec![3]);
}

#[test]
fn track_dropped_id_is_the_media_position_not_the_playable_count() {
    let middle = sdp_with(&[(1, pcma())]);
    let mut host = Host::new(BASE_URL, Config::default(), Server::new(&middle));
    assert_eq!(dropped(&play(&mut host)), vec![2]);
}

#[test]
fn an_unsupported_first_media_does_not_stop_the_second_from_playing() {
    let sdp = sdp_with(&[(0, pcma())]);
    let mut host = Host::new(BASE_URL, Config::default(), Server::new(&sdp));
    let events = play(&mut host);
    assert_eq!(dropped(&events), vec![1]);
    let announced: Vec<Vec<u32>> = events
        .iter()
        .filter_map(|e| match e {
            Event::Tracks(t) => Some(t.iter().map(|s| s.track_id.value()).collect()),
            _ => None,
        })
        .collect();
    assert_eq!(announced, vec![vec![1, 2]]);
    let samples = samples_of(&events);
    assert!(samples.iter().any(|s| s.0 == 1), "video plays");
    assert!(samples.iter().any(|s| s.0 == 2), "audio plays");
}
