//! `ClientSession` in play mode against the sans-IO play peer: command sequence, FLV media
//! (checked against the one-shot FLV demux of the same real capture), chunk-size changes,
//! acknowledgements, pings, end of stream and error statuses.

use broadcast_common::Unpackage;
use rtmp_runtime::amf0::{Amf0Value, Command};
use rtmp_runtime::chunk::Message;
use rtmp_runtime::client::{ClientConfig, ClientEvent, ClientMode, ClientSession};
use rtmp_runtime::message::{UserControl, msg_type};
use transmux::{FlvDemux, Media, StreamingFlvDemux};

use super::rtmp_play_peer::{PLAY_STREAM_ID, PeerConfig, PlayPeer, Seen};

const FIXTURE: &str = "flv/av.flv";
const FLV_HEADER_LEN: usize = 9;
const PREV_TAG_LEN: usize = 4;
const TAG_HEADER_LEN: usize = 11;
const APP: &str = "live";
const STREAM: &str = "cam1";
const SMALL_WINDOW: u32 = 4_000;

fn play_config(stream: &str) -> ClientConfig {
    let mut c = ClientConfig::default();
    c.mode = ClientMode::Play;
    c.app = APP.to_string();
    c.stream_key = stream.to_string();
    c.tc_url = Some(format!("rtmp://127.0.0.1/{APP}"));
    c
}

fn peer_config() -> PeerConfig {
    PeerConfig {
        stream_name: STREAM.to_string(),
        ..PeerConfig::default()
    }
}

pub struct Rig {
    pub client: ClientSession,
    pub peer: PlayPeer,
    pub events: Vec<ClientEvent>,
}

impl Rig {
    fn new(client: ClientConfig, peer: PeerConfig) -> Self {
        Self {
            client: ClientSession::new(client),
            peer: PlayPeer::new(peer),
            events: Vec::new(),
        }
    }

    fn handshake_and_play(&mut self) {
        let mut to_peer = self.client.start();
        for _ in 0..16 {
            let to_client = self.peer.on_bytes(&to_peer).expect("peer");
            if to_client.is_empty() {
                break;
            }
            let (out, events) = self.client.handle_data(&to_client).expect("client");
            self.events.extend(events);
            if out.is_empty() {
                break;
            }
            to_peer = out;
        }
    }

    fn deliver(&mut self, bytes: &[u8]) {
        let (out, events) = self.client.handle_data(bytes).expect("client");
        self.events.extend(events);
        if !out.is_empty() {
            self.peer
                .on_bytes(&out)
                .expect("peer reads the client reply");
        }
    }

    fn flv(&self) -> Vec<u8> {
        self.events
            .iter()
            .filter_map(|e| match e {
                ClientEvent::Media { flv } => Some(flv.as_slice()),
                _ => None,
            })
            .flatten()
            .copied()
            .collect()
    }
}

fn fixture_tags() -> Vec<(u8, u32, Vec<u8>)> {
    let flv = test_fixtures::read(FIXTURE);
    let mut at = FLV_HEADER_LEN + PREV_TAG_LEN;
    let mut tags = Vec::new();
    while at + TAG_HEADER_LEN <= flv.len() {
        let tag_type = flv[at];
        let size = u32::from_be_bytes([0, flv[at + 1], flv[at + 2], flv[at + 3]]) as usize;
        let ts = u32::from_be_bytes([flv[at + 7], flv[at + 4], flv[at + 5], flv[at + 6]]);
        let body = flv[at + TAG_HEADER_LEN..at + TAG_HEADER_LEN + size].to_vec();
        tags.push((tag_type, ts, body));
        at += TAG_HEADER_LEN + size + PREV_TAG_LEN;
    }
    tags
}

fn streaming_media(flv: &[u8]) -> Vec<(u32, Vec<transmux::Sample>)> {
    let mut demux = StreamingFlvDemux::new();
    demux.feed(flv).expect("streaming demux");
    demux.finish();
    let mut by_track: std::collections::BTreeMap<u32, Vec<transmux::Sample>> = Default::default();
    while let Some(ev) = demux.poll_event() {
        if let transmux::DemuxEvent::Sample {
            track_id, sample, ..
        } = ev
        {
            by_track.entry(track_id.value()).or_default().push(sample);
        }
    }
    by_track.into_iter().collect()
}

fn direct_media() -> Media {
    let flv = test_fixtures::read(FIXTURE);
    FlvDemux::new().unpackage(&flv[..]).expect("one-shot demux")
}

fn play_all(rig: &mut Rig) {
    for (tag_type, ts, body) in fixture_tags() {
        let bytes = rig.peer.media(tag_type, ts, &body).expect("media");
        rig.deliver(&bytes);
    }
}

fn assert_matches_direct(flv: &[u8]) {
    let direct = direct_media();
    let live = streaming_media(flv);
    assert_eq!(live.len(), direct.tracks.len(), "same tracks");
    for (track, (_, samples)) in direct.tracks.iter().zip(live.iter()) {
        assert_eq!(samples.len(), track.samples.len(), "same sample count");
        for (a, b) in samples.iter().zip(track.samples.iter()) {
            assert_eq!(a.data, b.data, "same coded bytes");
            assert_eq!(
                a.timing.map(|t| t.dts),
                b.timing.map(|t| t.dts),
                "same decode time"
            );
            assert_eq!(
                a.timing.map(|t| t.pts),
                b.timing.map(|t| t.pts),
                "same presentation time"
            );
        }
    }
}

#[test]
fn the_command_sequence_is_connect_create_stream_play_without_the_publish_preamble() {
    let mut rig = Rig::new(play_config(STREAM), peer_config());
    rig.handshake_and_play();
    assert_eq!(rig.peer.commands(), ["connect", "createStream", "play"]);
    let Some(Amf0Value::Object(pairs)) = rig.peer.command_args("connect").and_then(|a| a.first())
    else {
        panic!("connect carries a command object");
    };
    let get = |k: &str| pairs.iter().find(|(n, _)| n == k).map(|(_, v)| v.clone());
    assert_eq!(get("app"), Some(Amf0Value::String(APP.to_string())));
    assert_eq!(
        get("tcUrl"),
        Some(Amf0Value::String(format!("rtmp://127.0.0.1/{APP}")))
    );
    let play = rig.peer.command_args("play").expect("play");
    assert_eq!(play[0], Amf0Value::Null);
    assert_eq!(play[1], Amf0Value::String(STREAM.to_string()));
    assert_eq!(play[2], Amf0Value::Number(-2.0), "live-or-recorded start");
    assert!(rig.peer.seen.iter().any(|s| matches!(
        s,
        Seen::Command { name, stream_id, .. } if name == "play" && *stream_id == PLAY_STREAM_ID
    )));
    assert!(
        rig.peer
            .seen
            .contains(&Seen::UserControl(UserControl::SetBufferLength {
                stream_id: PLAY_STREAM_ID,
                buffer_ms: rtmp_runtime::client::DEFAULT_PLAY_BUFFER_MS,
            }))
    );
    let order: Vec<&str> = rig
        .events
        .iter()
        .map(|e| match e {
            ClientEvent::Connected { .. } => "connected",
            ClientEvent::StreamCreated { .. } => "created",
            ClientEvent::Playing { .. } => "playing",
            _ => "other",
        })
        .collect();
    assert_eq!(order, ["connected", "created", "playing"]);
}

#[test]
fn a_publish_mode_session_still_sends_the_publish_preamble_and_never_plays() {
    let mut cfg = play_config(STREAM);
    cfg.mode = ClientMode::Publish;
    let mut rig = Rig::new(cfg, peer_config());
    rig.handshake_and_play();
    let names = rig.peer.commands();
    assert_eq!(names[..3], ["connect", "releaseStream", "FCPublish"]);
    assert!(!names.contains(&"play"));
    assert!(
        !rig.events
            .iter()
            .any(|e| matches!(e, ClientEvent::Playing { .. }))
    );
}

#[test]
fn media_is_an_flv_stream_that_demuxes_to_the_samples_of_the_source_capture() {
    let mut rig = Rig::new(play_config(STREAM), peer_config());
    rig.handshake_and_play();
    play_all(&mut rig);
    let flv = rig.flv();
    assert!(
        flv.starts_with(b"FLV"),
        "the first chunk carries the file header"
    );
    assert_matches_direct(&flv);
}

#[test]
fn media_arriving_in_one_byte_reads_demuxes_the_same() {
    let mut rig = Rig::new(play_config(STREAM), peer_config());
    rig.handshake_and_play();
    let mut wire = Vec::new();
    for (tag_type, ts, body) in fixture_tags() {
        wire.extend(rig.peer.media(tag_type, ts, &body).unwrap());
    }
    for b in wire.chunks(1) {
        rig.deliver(b);
    }
    assert_matches_direct(&rig.flv());
}

#[test]
fn a_chunk_size_change_in_the_middle_of_the_stream_takes_effect_for_the_next_chunk() {
    let mut rig = Rig::new(play_config(STREAM), peer_config());
    rig.handshake_and_play();
    let tags = fixture_tags();
    let (head, tail) = tags.split_at(tags.len() / 2);
    for (tag_type, ts, body) in head {
        let bytes = rig.peer.media(*tag_type, *ts, body).unwrap();
        rig.deliver(&bytes);
    }
    let mut wire = rig.peer.set_chunk_size(60_000).unwrap();
    for (tag_type, ts, body) in tail {
        wire.extend(rig.peer.media(*tag_type, *ts, body).unwrap());
    }
    rig.deliver(&wire);
    assert_matches_direct(&rig.flv());
}

#[test]
fn a_chunk_size_change_announced_at_connect_is_honoured() {
    let mut cfg = peer_config();
    cfg.chunk_size_after_connect = Some(512);
    let mut rig = Rig::new(play_config(STREAM), cfg);
    rig.handshake_and_play();
    play_all(&mut rig);
    assert_matches_direct(&rig.flv());
}

#[test]
fn an_acknowledgement_is_sent_each_time_the_peer_window_is_crossed_with_the_total_byte_count() {
    let mut cfg = peer_config();
    cfg.window_ack_size = SMALL_WINDOW;
    let mut rig = Rig::new(play_config(STREAM), cfg);
    rig.handshake_and_play();
    assert!(
        rig.peer.acks().is_empty(),
        "no ack before the window is crossed"
    );
    let mut total_from_peer = 0u64;
    for (tag_type, ts, body) in fixture_tags() {
        let bytes = rig.peer.media(tag_type, ts, &body).unwrap();
        total_from_peer += bytes.len() as u64;
        rig.deliver(&bytes);
    }
    let acks = rig.peer.acks();
    assert!(
        acks.len() >= 2,
        "the fixture crosses the window repeatedly: {acks:?}"
    );
    assert!(
        acks.windows(2).all(|w| w[1] > w[0]),
        "sequence numbers grow: {acks:?}"
    );
    let last = u64::from(*acks.last().unwrap());
    assert!(
        last <= total_from_peer + 4096,
        "an ack never claims bytes that were not sent"
    );
    assert!(
        last + u64::from(SMALL_WINDOW) + 8192 >= total_from_peer,
        "the last ack is within a window of the total"
    );
    for pair in acks.windows(2) {
        assert!(
            u64::from(pair[1] - pair[0]) >= u64::from(SMALL_WINDOW),
            "acks are at least a window apart: {pair:?}"
        );
    }
}

#[test]
fn the_ack_sequence_number_is_exactly_the_bytes_received_after_the_handshake() {
    let mut cfg = peer_config();
    cfg.window_ack_size = 1;
    let mut rig = Rig::new(play_config(STREAM), cfg);
    rig.handshake_and_play();
    let before = rig.peer.acks().len();
    let (tag_type, ts, body) = fixture_tags().remove(0);
    let chunk_bytes_before = rig.client_bytes_after_handshake();
    let bytes = rig.peer.media(tag_type, ts, &body).unwrap();
    rig.deliver(&bytes);
    let acks = rig.peer.acks();
    assert_eq!(acks.len(), before + 1);
    assert_eq!(
        u64::from(*acks.last().unwrap()),
        chunk_bytes_before + bytes.len() as u64
    );
}

impl Rig {
    fn client_bytes_after_handshake(&self) -> u64 {
        self.peer.bytes_sent_after_handshake
    }
}

#[test]
fn a_ping_request_is_answered_with_a_ping_response_echoing_the_timestamp() {
    let mut rig = Rig::new(play_config(STREAM), peer_config());
    rig.handshake_and_play();
    let ping = rig.peer.ping(0xDEAD_BEEF).unwrap();
    rig.deliver(&ping);
    assert!(
        rig.peer
            .seen
            .contains(&Seen::UserControl(UserControl::PingResponse(0xDEAD_BEEF)))
    );
}

#[test]
fn a_missing_stream_is_an_error_event_and_no_playing() {
    let mut rig = Rig::new(play_config("nope"), peer_config());
    rig.handshake_and_play();
    assert!(!rig.peer.is_playing());
    let errors: Vec<&ClientEvent> = rig
        .events
        .iter()
        .filter(|e| matches!(e, ClientEvent::Error { .. }))
        .collect();
    assert_eq!(errors.len(), 1);
    assert!(matches!(
        errors[0],
        ClientEvent::Error { code, .. } if code == "NetStream.Play.StreamNotFound"
    ));
    assert!(
        !rig.events
            .iter()
            .any(|e| matches!(e, ClientEvent::Playing { .. }))
    );
}

#[test]
fn a_status_with_level_error_is_an_error_even_with_an_unknown_code() {
    let mut rig = Rig::new(play_config(STREAM), peer_config());
    rig.handshake_and_play();
    let bytes = rig.peer.on_status("error", "NetStream.Play.Weird").unwrap();
    rig.deliver(&bytes);
    assert!(rig.events.iter().any(|e| matches!(
        e,
        ClientEvent::Error { code, .. } if code == "NetStream.Play.Weird"
    )));
}

#[test]
fn an_informational_status_is_not_an_error() {
    let mut rig = Rig::new(play_config(STREAM), peer_config());
    rig.handshake_and_play();
    let before = rig.events.len();
    let bytes = rig
        .peer
        .on_status("status", "NetStream.Data.Start")
        .unwrap();
    rig.deliver(&bytes);
    assert_eq!(rig.events.len(), before);
}

#[test]
fn stream_eof_and_play_stop_both_end_the_stream() {
    let mut rig = Rig::new(play_config(STREAM), peer_config());
    rig.handshake_and_play();
    let eof = rig.peer.stream_eof().unwrap();
    rig.deliver(&eof);
    assert_eq!(
        rig.events
            .iter()
            .filter(|e| matches!(e, ClientEvent::StreamEnded))
            .count(),
        1
    );
    let stop = rig.peer.on_status("status", "NetStream.Play.Stop").unwrap();
    rig.deliver(&stop);
    assert_eq!(
        rig.events
            .iter()
            .filter(|e| matches!(e, ClientEvent::StreamEnded))
            .count(),
        2
    );
}

#[test]
fn an_aggregate_message_is_unpacked_into_the_same_flv_tags() {
    let mut rig = Rig::new(play_config(STREAM), peer_config());
    rig.handshake_and_play();
    let tags = fixture_tags();
    let mut body = Vec::new();
    for (tag_type, ts, payload) in &tags {
        let start = body.len();
        body.push(*tag_type);
        body.extend_from_slice(&(payload.len() as u32).to_be_bytes()[1..]);
        body.extend_from_slice(&ts.to_be_bytes()[1..]);
        body.push((ts >> 24) as u8);
        body.extend_from_slice(&[0, 0, 0]);
        body.extend_from_slice(payload);
        let size = (body.len() - start) as u32;
        body.extend_from_slice(&size.to_be_bytes());
    }
    let msg = Message {
        chunk_stream_id: 4,
        timestamp: tags[0].1,
        message_type_id: msg_type::AGGREGATE,
        message_stream_id: PLAY_STREAM_ID,
        payload: body,
    };
    let bytes = rig.peer.raw_message(&msg).unwrap();
    rig.deliver(&bytes);
    assert_matches_direct(&rig.flv());
}

#[test]
fn media_before_the_play_command_is_not_delivered() {
    let mut client = ClientSession::new(play_config(STREAM));
    let mut peer = PlayPeer::new(peer_config());
    let first = client.start();
    let reply = peer.on_bytes(&first).unwrap();
    let (_, events) = client.handle_data(&reply).unwrap();
    assert!(
        events
            .iter()
            .all(|e| !matches!(e, ClientEvent::Media { .. }))
    );
    let (tag_type, ts, body) = fixture_tags().remove(0);
    let bytes = peer.media(tag_type, ts, &body).unwrap();
    let (_, events) = client.handle_data(&bytes).unwrap();
    assert!(
        events
            .iter()
            .all(|e| !matches!(e, ClientEvent::Media { .. }))
    );
}

#[test]
fn a_command_sent_as_amf3_is_decoded_after_its_marker_byte() {
    let mut rig = Rig::new(play_config(STREAM), peer_config());
    rig.handshake_and_play();
    let mut payload = vec![0u8];
    payload.extend(
        Command {
            name: "onStatus".to_string(),
            transaction_id: 0.0,
            arguments: vec![
                Amf0Value::Null,
                Amf0Value::Object(vec![
                    ("level".to_string(), Amf0Value::String("error".to_string())),
                    (
                        "code".to_string(),
                        Amf0Value::String("NetStream.Play.Failed".to_string()),
                    ),
                ]),
            ],
        }
        .to_body()
        .unwrap(),
    );
    let msg = Message {
        chunk_stream_id: 3,
        timestamp: 0,
        message_type_id: msg_type::COMMAND_AMF3,
        message_stream_id: PLAY_STREAM_ID,
        payload,
    };
    let bytes = rig.peer.raw_message(&msg).unwrap();
    rig.deliver(&bytes);
    assert!(rig.events.iter().any(|e| matches!(
        e,
        ClientEvent::Error { code, .. } if code == "NetStream.Play.Failed"
    )));
}

#[test]
fn playing_reports_whether_the_server_marked_the_stream_recorded() {
    for recorded in [false, true] {
        let mut cfg = peer_config();
        cfg.recorded = recorded;
        let mut rig = Rig::new(play_config(STREAM), cfg);
        rig.handshake_and_play();
        assert!(
            rig.events.contains(&ClientEvent::Playing { recorded }),
            "recorded={recorded}: {:?}",
            rig.events
        );
    }
}

#[test]
fn mode_labels_are_stable() {
    assert_eq!(ClientMode::Play.name(), "play");
    assert_eq!(ClientMode::Publish.to_string(), "publish");
}
