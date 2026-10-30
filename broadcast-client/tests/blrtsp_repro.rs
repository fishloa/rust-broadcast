mod support;

use broadcast_client::{BufferEvent, Config, Event};
use transmux::CodecConfig;
use support::rtsp_host::{BASE_URL, Host, MediaFixture, Server};
use broadcast_common::Unpackage;
use transmux::TsDemux;

#[test]
fn blrtsp_repro_dims_start_duration() {
    let fixture = MediaFixture::load();
    let media = TsDemux::new()
        .unpackage(&test_fixtures::read("ts/h264_aac.ts"))
        .expect("demux");
    let (ts_w, ts_h) = media
        .tracks
        .iter()
        .find_map(|t| match &t.spec.config {
            CodecConfig::Avc { width, height, .. } => Some((*width, *height)),
            _ => None,
        })
        .expect("ts video spec");

    let mut host = Host::new(BASE_URL, Config::default(), Server::new(&fixture.sdp));
    host.connect();
    let frames = fixture.frames();
    let mut all = Vec::new();
    for frame in &frames {
        host.now += 10_000;
        let conn = host.conn.expect("conn");
        host.session.on_data(conn, frame, host.now);
        host.session.on_playback_position(host.now, None);
        host.drain();
        all.extend(host.events());
    }
    for _ in 0..200 {
        host.now += 10_000;
        host.session.on_playback_position(host.now, None);
        host.drain();
        all.extend(host.events());
    }

    let tracks_events: Vec<_> = all
        .iter()
        .filter_map(|e| match e {
            Event::Tracks(t) => Some(t),
            _ => None,
        })
        .collect();
    assert!(!tracks_events.is_empty(), "no Tracks events: {all:?}");
    let last_tracks = tracks_events.last().unwrap();
    let video = last_tracks
        .iter()
        .find(|s| matches!(s.config, CodecConfig::Avc { .. }))
        .expect("video track");
    if let CodecConfig::Avc { width, height, .. } = &video.config {
        assert_eq!((*width, *height), (ts_w, ts_h), "dims mismatch");
        assert!(*width > 0 && *height > 0, "zero dims");
    }

    assert!(
        all.iter()
            .any(|e| matches!(e, Event::Buffer(BufferEvent::Start { .. }))),
        "no BufferEvent::Start: {all:?}"
    );

    let samples: Vec<_> = all
        .iter()
        .filter_map(|e| match e {
            Event::Sample { sample, .. } => Some(sample),
            _ => None,
        })
        .collect();
    assert!(!samples.is_empty(), "no samples");
    assert!(
        samples.iter().all(|s| s.duration.unwrap_or(0) > 0),
        "zero duration sample present"
    );
}
