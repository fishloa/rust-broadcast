//! RTMP pull through a `Session` against the sans-IO play peer on a virtual clock: URL handling,
//! connect actions, FLV media equivalence with the direct demux, the status and error mapping, the
//! policy timers and reconnects.

use broadcast_client::{
    Action, Config, Error, ErrorKind, Event, Session, SourceKind, StarveKind, Status,
};
use broadcast_common::stage::Timestamp;
use rtmp_runtime::message::UserControl;
use test_harness::rtmp_play_peer::Seen;
use url::Url;

/// The harness's microsecond clock as the session's `now`.
fn ts(us: u64) -> Timestamp {
    Timestamp::from_micros(us)
}

use crate::support::rtmp_host::{
    MS, PeerConfig, RtmpHost, SEC, URL, assert_complete_samples_equal_direct,
    assert_samples_equal_direct, connect_and_play, direct, fixture_tags, peer_config, rtmp_config,
};

fn statuses(events: &[Event]) -> Vec<Status> {
    events
        .iter()
        .filter_map(|e| match e {
            Event::Status(s) => Some(*s),
            _ => None,
        })
        .collect()
}

fn host() -> RtmpHost {
    RtmpHost::new(URL, rtmp_config(), peer_config())
}

#[test]
fn the_first_action_is_a_tcp_connect_to_the_default_port() {
    let mut s = Session::open(Url::parse(URL).unwrap(), rtmp_config(), ts(0)).unwrap();
    match s.poll(ts(0)) {
        Some(Action::TcpConnect {
            host, port, tls, ..
        }) => {
            assert_eq!(host, "origin.test");
            assert_eq!(port, 1935);
            assert!(!tls);
        }
        other => panic!("{other:?}"),
    }
}

#[test]
fn rtmps_connects_with_tls_on_443_and_a_port_in_the_url_wins() {
    let mut s = Session::open(
        Url::parse("rtmps://origin.test/live/cam1").unwrap(),
        Config::default(),
        ts(0),
    )
    .unwrap();
    assert!(matches!(
        s.poll(ts(0)),
        Some(Action::TcpConnect {
            port: 443,
            tls: true,
            ..
        })
    ));
    let mut s = Session::open(
        Url::parse("rtmp://origin.test:1936/live/cam1").unwrap(),
        Config::default(),
        ts(0),
    )
    .unwrap();
    assert!(matches!(
        s.poll(ts(0)),
        Some(Action::TcpConnect {
            port: 1936,
            tls: false,
            ..
        })
    ));
}

#[test]
fn auto_selects_rtmp_from_the_scheme() {
    let mut s = Session::open(Url::parse(URL).unwrap(), Config::default(), ts(0)).unwrap();
    assert!(matches!(s.poll(ts(0)), Some(Action::TcpConnect { .. })));
}

#[test]
fn an_rtmp_url_needs_an_app_and_a_stream_and_a_matching_kind() {
    for bad in [
        "rtmp://origin.test",
        "rtmp://origin.test/",
        "rtmp://origin.test/live",
        "rtmp://origin.test/live/",
        "rtmp://origin.test//cam1",
        "rtmp://origin.test/live/?token=x",
    ] {
        assert!(
            matches!(
                Session::open(Url::parse(bad).unwrap(), rtmp_config(), ts(0)),
                Err(Error::InvalidUrl(_))
            ),
            "{bad}"
        );
    }
    let mut cfg = rtmp_config();
    cfg.kind = SourceKind::Hls;
    assert!(matches!(
        Session::open(Url::parse(URL).unwrap(), cfg, ts(0)),
        Err(Error::UnsupportedScheme { .. })
    ));
    assert!(matches!(
        Session::open(
            Url::parse("http://origin.test/live.ts").unwrap(),
            rtmp_config(),
            ts(0)
        ),
        Err(Error::UnsupportedScheme { .. })
    ));
}

#[test]
fn app_stream_and_query_reach_the_server() {
    let mut h = RtmpHost::new(
        "rtmp://origin.test/live/a/b%20c?token=s3cret",
        rtmp_config(),
        PeerConfig {
            stream_name: "a/b c?token=s3cret".to_string(),
            ..peer_config()
        },
    );
    connect_and_play(&mut h);
    let peer = h.peer.as_ref().unwrap();
    let connect = peer.command_args("connect").unwrap();
    let rtmp_runtime::amf0::Amf0Value::Object(pairs) = &connect[0] else {
        panic!("connect object");
    };
    let get = |k: &str| pairs.iter().find(|(n, _)| n == k).map(|(_, v)| v.clone());
    assert_eq!(
        get("app"),
        Some(rtmp_runtime::amf0::Amf0Value::String("live".to_string()))
    );
    assert_eq!(
        get("tcUrl"),
        Some(rtmp_runtime::amf0::Amf0Value::String(
            "rtmp://origin.test/live".to_string()
        ))
    );
}

#[test]
fn a_tc_url_keeps_a_non_default_port_and_brackets_an_ipv6_host() {
    let mut s = Session::open(
        Url::parse("rtmp://[::1]:1936/live/cam1").unwrap(),
        Config::default(),
        ts(0),
    )
    .unwrap();
    assert!(matches!(
        s.poll( ts(0)),
        Some(Action::TcpConnect { host, port: 1936, .. }) if host == "::1"
    ));
}

#[test]
fn the_samples_equal_the_direct_flv_demux_of_the_source_capture() {
    let mut h = host();
    connect_and_play(&mut h);
    h.play_all();
    let samples = h.samples();
    assert!(samples.len() > 50, "the fixture carries real media");
    assert_samples_equal_direct(&samples);
    let tracks: Vec<&Vec<transmux::TrackSpec>> = h
        .events
        .iter()
        .filter_map(|e| match e {
            Event::Tracks(t) => Some(t),
            _ => None,
        })
        .collect();
    assert_eq!(
        tracks.len(),
        2,
        "video resolves first and is announced with the first sample, the audio track follows"
    );
    assert_eq!(tracks[0].len(), 1);
    let d = direct();
    let want: Vec<String> = d
        .tracks
        .iter()
        .map(|t| format!("{:?}", t.spec.config))
        .collect();
    let got: Vec<String> = tracks[1]
        .iter()
        .map(|t| format!("{:?}", t.config))
        .collect();
    assert_eq!(got, want);
    let audio_id = tracks[1]
        .iter()
        .find(|t| t.config.audio_sample_rate().is_some())
        .unwrap()
        .track_id;
    let announced_audio_at = h
        .events
        .iter()
        .position(|e| matches!(e, Event::Tracks(t) if t.len() == 2))
        .unwrap();
    let first_audio_at = h
        .events
        .iter()
        .position(|e| matches!(e, Event::Sample { track, .. } if *track == audio_id))
        .unwrap();
    assert!(
        announced_audio_at < first_audio_at,
        "a track is announced before its first sample"
    );
}

#[test]
fn establishing_precedes_tracks_and_live_comes_with_the_first_sample() {
    let mut h = host();
    connect_and_play(&mut h);
    h.play_all();
    let kinds: Vec<&str> = h
        .events
        .iter()
        .map(|e| match e {
            Event::Status(Status::Establishing) => "establishing",
            Event::Tracks(_) => "tracks",
            Event::Status(Status::Live) => "live",
            Event::Sample { .. } => "sample",
            _ => "other",
        })
        .filter(|k| *k != "other")
        .collect();
    assert_eq!(kinds[0], "establishing");
    assert_eq!(kinds[1], "tracks");
    assert_eq!(kinds[2], "live");
    assert_eq!(kinds[3], "sample");
    assert_eq!(
        statuses(&h.events)
            .iter()
            .filter(|s| **s == Status::Live)
            .count(),
        1
    );
}

#[test]
fn a_missing_stream_is_not_found_closes_and_backs_off_two_seconds() {
    let mut h = RtmpHost::new(
        "rtmp://origin.test/live/missing",
        rtmp_config(),
        peer_config(),
    );
    h.collect();
    let st = statuses(&h.events);
    assert!(st.contains(&Status::NotFound), "{st:?}");
    assert!(st.contains(&Status::Reconnecting { attempt: 1 }), "{st:?}");
    assert_eq!(h.closes.len(), 1);
    assert_eq!(h.connects.len(), 1);
    h.advance_to(2 * SEC - 1);
    assert_eq!(h.connects.len(), 1, "not before the backoff elapsed");
    h.advance_to(2 * SEC);
    assert_eq!(h.connects.len(), 2);
    h.collect();
    // The second failure waits the 4 s step, jittered below it.
    h.advance_to(2 * SEC + 4 * SEC * 5 / 6 - 1);
    assert_eq!(h.connects.len(), 2);
    h.advance_to(2 * SEC + 4 * SEC);
    assert_eq!(
        h.connects.len(),
        3,
        "the second failure waits the 4 s step at most"
    );
    assert_eq!(h.samples().len(), 0);
}

#[test]
fn another_error_status_is_a_typed_error_then_the_same_backoff() {
    let mut h = host();
    connect_and_play(&mut h);
    let bytes = h
        .peer
        .as_mut()
        .unwrap()
        .on_status("error", "NetStream.Play.Failed")
        .unwrap();
    h.send_to_session(&bytes);
    assert!(h.events.iter().any(|e| matches!(
        e,
        Event::Error { kind: ErrorKind::RtmpStatus { code }, fatal: false }
            if code == "NetStream.Play.Failed"
    )));
    assert!(statuses(&h.events).contains(&Status::Reconnecting { attempt: 1 }));
}

#[test]
fn a_status_code_is_stripped_of_control_characters_and_capped() {
    let mut h = host();
    connect_and_play(&mut h);
    let hostile = format!("NetStream\u{7}.\u{1b}[31m{}", "x".repeat(1_000));
    let bytes = h
        .peer
        .as_mut()
        .unwrap()
        .on_status("error", &hostile)
        .unwrap();
    h.send_to_session(&bytes);
    let code = h
        .events
        .iter()
        .find_map(|e| match e {
            Event::Error {
                kind: ErrorKind::RtmpStatus { code },
                ..
            } => Some(code.clone()),
            _ => None,
        })
        .expect("error");
    assert!(code.chars().all(|c| !c.is_control()));
    assert!(code.starts_with("NetStream.[31m"));
    assert_eq!(
        code.len(),
        broadcast_client::MAX_SERVER_REFUSED_REASON_BYTES
    );
}

#[test]
fn a_connect_failure_backs_off_and_the_attempt_count_grows() {
    let mut h = host();
    h.fail_next_connect = true;
    h.collect();
    assert!(statuses(&h.events).contains(&Status::Reconnecting { attempt: 1 }));
    assert_eq!(h.connects.len(), 1);
    h.fail_next_connect = true;
    h.advance_to(2 * SEC);
    assert_eq!(h.connects.len(), 2);
    assert!(statuses(&h.events).contains(&Status::Reconnecting { attempt: 2 }));
    h.advance_to(2 * SEC + 4 * SEC);
    assert_eq!(h.connects.len(), 3);
    assert_eq!(h.samples().len(), 0);
}

#[test]
fn a_ping_request_is_answered_through_a_send_action() {
    let mut h = host();
    connect_and_play(&mut h);
    let ping = h.peer.as_mut().unwrap().ping(0xCAFE_F00D).unwrap();
    h.send_to_session(&ping);
    assert!(
        h.peer
            .as_ref()
            .unwrap()
            .seen
            .contains(&Seen::UserControl(UserControl::PingResponse(0xCAFE_F00D)))
    );
}

#[test]
fn a_chunk_size_change_in_the_middle_of_the_stream_leaves_the_samples_unchanged() {
    let mut h = host();
    connect_and_play(&mut h);
    let tags = fixture_tags();
    let (head, tail) = tags.split_at(tags.len() / 3);
    for (t, ts, b) in head {
        let bytes = h.peer.as_mut().unwrap().media((*t).into(), *ts, b).unwrap();
        h.send_to_session(&bytes);
    }
    let mut wire = h.peer.as_mut().unwrap().set_chunk_size(31).unwrap();
    for (t, ts, b) in tail {
        wire.extend(h.peer.as_mut().unwrap().media((*t).into(), *ts, b).unwrap());
    }
    h.send_to_session(&wire);
    assert_samples_equal_direct(&h.samples());
}

#[test]
fn the_acknowledgement_window_is_honoured_through_the_session() {
    let mut cfg = peer_config();
    cfg.window_ack_size = 3_000;
    let mut h = RtmpHost::new(URL, rtmp_config(), cfg);
    connect_and_play(&mut h);
    h.play_all();
    let acks = h.peer.as_ref().unwrap().acks();
    assert!(acks.len() >= 3, "{acks:?}");
    assert!(acks.windows(2).all(|w| w[1] - w[0] >= 3_000));
}

#[test]
fn media_split_into_one_byte_reads_is_the_same() {
    let mut h = host();
    connect_and_play(&mut h);
    let mut wire = Vec::new();
    for (t, ts, b) in fixture_tags() {
        wire.extend(h.peer.as_mut().unwrap().media(t.into(), ts, &b).unwrap());
    }
    for b in wire.chunks(1) {
        h.send_to_session(b);
    }
    assert_samples_equal_direct(&h.samples());
}

#[test]
fn a_server_that_closes_after_playing_is_reconnected_and_replays_with_a_discontinuity() {
    let mut h = host();
    connect_and_play(&mut h);
    h.play_all();
    h.now = 10 * SEC;
    let before = h.connects.len();
    h.peer_closes(None);
    assert!(statuses(&h.events).contains(&Status::Reconnecting { attempt: 0 }));
    assert_eq!(h.connects.len(), before + 1, "one reconnect");
    let first_connection = h.samples();
    assert_complete_samples_equal_direct(&first_connection);
    assert!(h.peer.as_ref().unwrap().is_playing());
    h.play_all();
    let all = h.samples();
    assert_samples_equal_direct(&all[first_connection.len()..]);
    let disc: Vec<usize> = h
        .events
        .iter()
        .enumerate()
        .filter(|(_, e)| matches!(e, Event::Discontinuity))
        .map(|(i, _)| i)
        .collect();
    assert_eq!(disc.len(), 1);
    let first_new_sample = h
        .events
        .iter()
        .enumerate()
        .filter(|(_, e)| matches!(e, Event::Sample { .. }))
        .nth(first_connection.len())
        .map(|(i, _)| i)
        .unwrap();
    assert!(
        disc[0] < first_new_sample,
        "before the first sample of the new connection"
    );
    let last_old_sample = h
        .events
        .iter()
        .enumerate()
        .filter(|(_, e)| matches!(e, Event::Sample { .. }))
        .nth(first_connection.len() - 1)
        .map(|(i, _)| i)
        .unwrap();
    assert!(
        last_old_sample < disc[0],
        "after the last sample of the old one"
    );
}

#[test]
fn a_transport_error_after_playing_reconnects_too() {
    let mut h = host();
    connect_and_play(&mut h);
    h.play_all();
    h.now = 5 * SEC;
    let before = h.connects.len();
    h.peer_closes(Some("reset"));
    assert_eq!(h.connects.len(), before + 1);
}

#[test]
fn a_recorded_stream_that_ends_is_ended_and_not_retried() {
    let mut cfg = peer_config();
    cfg.recorded = true;
    let mut h = RtmpHost::new(URL, rtmp_config(), cfg);
    connect_and_play(&mut h);
    h.play_all();
    let eof = h.peer.as_mut().unwrap().stream_eof().unwrap();
    h.send_to_session(&eof);
    assert_eq!(
        statuses(&h.events)
            .iter()
            .filter(|s| **s == Status::Ended)
            .count(),
        1
    );
    assert_eq!(h.closes.len(), 1);
    let connects = h.connects.len();
    h.advance_to(120 * SEC);
    assert_eq!(
        h.connects.len(),
        connects,
        "an ended presentation is not retried"
    );
    assert_complete_samples_equal_direct(&h.samples());
}

#[test]
fn a_live_stream_that_ends_reconnects() {
    let mut h = host();
    connect_and_play(&mut h);
    h.play_all();
    h.now = 3 * SEC;
    let before = h.connects.len();
    let eof = h.peer.as_mut().unwrap().stream_eof().unwrap();
    h.send_to_session(&eof);
    assert!(!statuses(&h.events).contains(&Status::Ended));
    assert_eq!(h.closes.len(), 1);
    assert!(statuses(&h.events).contains(&Status::Reconnecting { attempt: 0 }));
    assert_eq!(
        h.connects.len(),
        before + 1,
        "the connection lasted longer than the minimum interval: reconnect at once"
    );
}

#[test]
fn a_connection_that_ended_quickly_waits_out_the_minimum_reconnect_interval() {
    let mut h = host();
    connect_and_play(&mut h);
    h.play_all();
    h.now = 300 * MS;
    let before = h.connects.len();
    h.peer_closes(None);
    assert_eq!(
        h.connects.len(),
        before,
        "not before 1 s after the connection opened"
    );
    h.advance_to(SEC - 1);
    assert_eq!(h.connects.len(), before);
    h.advance_to(SEC);
    assert_eq!(h.connects.len(), before + 1);
}

#[test]
fn silence_after_playing_notifies_then_reconnects() {
    let mut h = host();
    connect_and_play(&mut h);
    h.play_all();
    h.now = 0;
    h.advance_to(5 * SEC);
    let st = statuses(&h.events);
    assert!(
        st.iter().any(|s| matches!(s, Status::Stalled { .. })),
        "{st:?}"
    );
    assert!(
        h.events
            .iter()
            .any(|e| matches!(e, Event::Starved(StarveKind::NoData)))
    );
    let before = h.connects.len();
    h.advance_to(30 * SEC - 1);
    assert_eq!(
        h.connects.len(),
        before,
        "not before the stall reconnect threshold"
    );
    h.advance_to(30 * SEC);
    assert_eq!(h.closes.len(), 1, "stalled out: closed");
    assert_eq!(h.connects.len(), before + 1);
}

#[test]
fn a_server_that_never_answers_is_given_up_on_at_the_start_timeout() {
    let mut h = host();
    h.silent = true;
    h.collect();
    assert_eq!(h.connects.len(), 1);
    h.advance_to(15 * SEC - 1);
    assert!(h.closes.is_empty(), "not before the 15 s start timeout");
    assert!(!statuses(&h.events).contains(&Status::Reconnecting { attempt: 1 }));
    h.advance_to(15 * SEC);
    assert_eq!(h.closes.len(), 1);
    assert!(statuses(&h.events).contains(&Status::Reconnecting { attempt: 1 }));
    h.advance_to(17 * SEC - 1);
    assert_eq!(h.connects.len(), 1);
    h.advance_to(17 * SEC);
    assert_eq!(h.connects.len(), 2, "after the two second backoff");
}

#[test]
fn close_emits_one_close_for_the_live_connection_and_goes_silent() {
    let mut h = host();
    connect_and_play(&mut h);
    h.session.close(ts(h.now));
    let mut closes = Vec::new();
    while let Some(a) = h.session.poll(ts(h.now)) {
        if let Action::Close { conn } = a {
            closes.push(conn);
        }
    }
    assert_eq!(closes, vec![h.conn.unwrap()]);
    assert!(h.session.poll(ts(h.now)).is_none());
    assert!(h.session.next_event().is_none());
}

#[test]
fn caller_reconnect_restarts_the_connection() {
    let mut h = host();
    connect_and_play(&mut h);
    h.play_all();
    let before = h.connects.len();
    h.session.reconnect(ts(h.now));
    h.collect();
    assert_eq!(h.closes.len(), 1);
    assert_eq!(h.connects.len(), before + 1);
    assert!(
        h.peer.as_ref().unwrap().is_playing(),
        "the new connection plays"
    );
}

/// A lost RTMP connection is a source outage for the buffer controller (`SourceEnded` /
/// `SourceError`), not starvation with data arriving; the reconnect clears it with `Reconnected`
/// before the new connection's first media.
#[test]
fn a_closed_connection_is_hinted_to_the_buffer_and_cleared_on_reconnect() {
    use broadcast_client::{BufferHint, BufferInput};

    for (error, want) in [
        (None, BufferHint::SourceEnded),
        (Some("reset"), BufferHint::SourceError),
    ] {
        let mut config = rtmp_config();
        config.record_buffer_inputs = true;
        let mut h = RtmpHost::new(URL, config, peer_config());
        connect_and_play(&mut h);
        h.play_all();
        h.now = 10 * SEC;
        let _ = h.session.drain_buffer_inputs();
        h.peer_closes(error);
        assert!(h.peer.as_ref().unwrap().is_playing());
        h.play_all();
        let inputs: Vec<BufferInput> = h.session.drain_buffer_inputs().collect();
        let close = inputs
            .iter()
            .position(|i| matches!(i, BufferInput::Hint { hint, .. } if *hint == want));
        let reconnected = inputs.iter().position(|i| {
            matches!(
                i,
                BufferInput::Hint {
                    hint: BufferHint::Reconnected,
                    ..
                }
            )
        });
        assert!(close.is_some(), "close sent no {want:?} hint: {inputs:?}");
        let first_media = close.and_then(|c| {
            inputs
                .iter()
                .skip(c)
                .position(|i| matches!(i, BufferInput::TrackMedia { .. }))
                .map(|p| p + c)
        });
        assert!(
            close < reconnected && reconnected < first_media,
            "close hint {close:?}, Reconnected {reconnected:?}, first new media {first_media:?}"
        );
    }
}
