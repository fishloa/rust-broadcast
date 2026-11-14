//! RTMP pull criteria against the av.flv baseline.

use std::time::Duration;

use broadcast_client::{Config, Error, ErrorKind, Event, Session, Status};

use crate::support::rtmp_host::{
    PeerConfig, RtmpHost, SEC, URL, assert_complete_samples_equal_direct,
    assert_samples_equal_direct, connect_and_play, direct, peer_config, rtmp_config,
};
use crate::support::rtmp_wire::{PeerServer, Serve, WireHost};

fn statuses(events: &[Event]) -> Vec<Status> {
    events
        .iter()
        .filter_map(|e| match e {
            Event::Status(s) => Some(*s),
            _ => None,
        })
        .collect()
}

#[test]
fn rtmp_pull_matches_direct_flv_demux() {
    let cfg = rtmp_config();
    let pcfg = PeerConfig {
        recorded: true,
        ..peer_config()
    };
    let server = PeerServer::start(pcfg, vec![Serve::All], None);
    let mut host = WireHost::open(URL, cfg, server.addr.port(), None);
    host.run_until(Duration::from_secs(10), |h| {
        h.events
            .iter()
            .any(|e| matches!(e, Event::Status(Status::Ended)))
    });
    let samples = host.samples();
    assert!(samples.len() > 50, "the fixture carries real media");
    assert_complete_samples_equal_direct(&samples);
    let st = statuses(&host.events);
    assert!(st.contains(&Status::Ended));
}

use broadcast_common::stage::Timestamp;
use rtmp_runtime::amf0::Amf0Value;

/// The harness's microsecond clock as the session's `now`.
fn ts(us: u64) -> Timestamp {
    Timestamp::from_micros(us)
}

#[test]
fn rtmp_url_splits_app_stream_and_query() {
    let mut h = RtmpHost::new(
        "rtmp://origin.test/live/a/b%20c?token=s3cret",
        rtmp_config(),
        PeerConfig {
            stream_name: "a/b c?token=s3cret".to_string(),
            ..peer_config()
        },
    );
    connect_and_play(&mut h);
    let connect = h.peer.as_ref().unwrap().command_args("connect").unwrap();
    let rtmp_runtime::amf0::Amf0Value::Object(pairs) = &connect[0] else {
        panic!("connect object");
    };
    let get = |k: &str| pairs.iter().find(|(n, _)| n == k).map(|(_, v)| v.clone());
    assert_eq!(get("app"), Some(Amf0Value::String("live".to_string())));
    assert_eq!(
        get("tcUrl"),
        Some(Amf0Value::String("rtmp://origin.test/live".to_string()))
    );
    let play = h.peer.as_ref().unwrap().command_args("play").unwrap();
    assert!(
        play.iter()
            .any(|v| matches!(v, Amf0Value::String(s) if s == "a/b c?token=s3cret")),
        "{play:?}"
    );
    assert_eq!(h.connects[0].host, "origin.test");
    assert_eq!(h.connects[0].port, 1935);
    assert!(!h.connects[0].tls);
}

#[test]
fn rtmp_url_without_stream_is_invalid() {
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
                Session::open(bad, rtmp_config(), ts(0)),
                Err(Error::InvalidUrl(_))
            ),
            "{bad}"
        );
    }
}

#[test]
fn rtmp_userinfo_is_never_sent() {
    let url = "rtmp://alice:secret@origin.test/live/cam1?token=x";
    let mut h = RtmpHost::new(
        url,
        rtmp_config(),
        PeerConfig {
            stream_name: "cam1?token=x".to_string(),
            ..peer_config()
        },
    );
    connect_and_play(&mut h);
    assert_eq!(h.connects[0].host, "origin.test");
    assert!(!h.connects[0].host.contains("alice"));
    let connect = h.peer.as_ref().unwrap().command_args("connect").unwrap();
    let Amf0Value::Object(pairs) = &connect[0] else {
        panic!("connect object");
    };
    let tc = pairs
        .iter()
        .find(|(n, _)| n == "tcUrl")
        .map(|(_, v)| v.clone())
        .unwrap();
    assert_eq!(tc, Amf0Value::String("rtmp://origin.test/live".to_string()));
    let play = h.peer.as_ref().unwrap().command_args("play").unwrap();
    assert!(
        play.iter()
            .any(|v| matches!(v, Amf0Value::String(s) if s == "cam1?token=x")),
        "{play:?}"
    );
}

#[test]
fn rtmps_requests_tls_and_carries_identity() {
    use broadcast_client::{ClientIdentity, ClientIdentityScope};
    let handle = 0xC0FFEEu64;
    let mut cfg = Config::default();
    cfg.client_identity = Some(ClientIdentity::handle(handle));
    cfg.client_identity_scope = ClientIdentityScope::SessionOrigin;
    let mut s = Session::open("rtmps://origin.test/live/cam1", cfg, ts(0)).unwrap();
    let mut tls = false;
    let mut port = 0u16;
    let mut host = String::new();
    let mut ident = None;
    while let Some(a) = s.poll(ts(0)) {
        if let broadcast_client::Action::TcpConnect {
            tls: t,
            port: p,
            host: h,
            identity,
            ..
        } = a
        {
            tls = t;
            port = p;
            host = h;
            ident = identity.as_ref().and_then(|i| i.handle_id());
        }
    }
    assert!(tls);
    assert_eq!(port, 443);
    assert_eq!(host, "origin.test");
    assert_eq!(ident, Some(handle));
    // plain rtmp never carries the identity
    let mut s2 = Session::open("rtmp://origin.test/live/cam1", Config::default(), ts(0)).unwrap();
    let mut ident2 = None;
    while let Some(a) = s2.poll(ts(0)) {
        if let broadcast_client::Action::TcpConnect { identity, .. } = a {
            ident2 = identity.as_ref().and_then(|i| i.handle_id());
        }
    }
    assert!(ident2.is_none());
}

#[test]
fn rtmp_error_status_is_nonfatal_and_reconnects() {
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
    assert_eq!(h.closes.len(), 1);
}

#[test]
fn rtmp_live_stream_eof_reconnects_with_discontinuity() {
    let mut cfg = rtmp_config();
    cfg.timeline_jump = None;
    let mut h = RtmpHost::new(URL, cfg, peer_config());
    connect_and_play(&mut h);
    h.play_all();
    h.now = 3 * SEC;
    let before = h.connects.len();
    let eof = h.peer.as_mut().unwrap().stream_eof().unwrap();
    h.send_to_session(&eof);
    assert!(!statuses(&h.events).contains(&Status::Ended));
    assert_eq!(h.closes.len(), 1);
    assert!(statuses(&h.events).contains(&Status::Reconnecting { attempt: 0 }));
    assert_eq!(h.connects.len(), before + 1);
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
}

#[test]
fn rtmp_server_disconnect_reconnects_with_discontinuity() {
    let mut cfg = rtmp_config();
    cfg.timeline_jump = None;
    let mut h = RtmpHost::new(URL, cfg, peer_config());
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
fn rtmp_tracks_announced_before_first_sample() {
    let mut h = host();
    connect_and_play(&mut h);
    h.play_all();
    let tracks: Vec<&Vec<transmux::TrackSpec>> = h
        .events
        .iter()
        .filter_map(|e| match e {
            Event::Tracks(t) => Some(t),
            _ => None,
        })
        .collect();
    assert_eq!(tracks.len(), 2, "video then audio track announcements");
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

fn host() -> RtmpHost {
    RtmpHost::new(URL, rtmp_config(), peer_config())
}
