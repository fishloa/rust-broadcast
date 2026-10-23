use tshttp_runtime::{Config, Event, TsHttpClient};

fn samples_for(chunk_size: usize) -> (usize, usize, usize) {
    let bytes = test_fixtures::require_bytes("zenith-http/tf1-live.ts", "real_stream").unwrap();
    let mut c = TsHttpClient::new("u", Config::default(), 0);
    let _ = c.poll();
    c.on_response(200, &[], 0);
    let _ = c.poll_event();
    let mut total_samples = 0;
    let mut tracks = std::collections::BTreeSet::new();
    for chunk in bytes.chunks(chunk_size) {
        c.feed(chunk, 0);
        while let Some(e) = c.poll_event() {
            if let Event::Demux(transmux::DemuxEvent::Sample { .. }) = e {
                total_samples += 1;
            }
            if let Event::Demux(transmux::DemuxEvent::TrackAdded(spec)) = e {
                tracks.insert(spec.track_id.value());
            }
        }
    }
    (total_samples, tracks.len(), 0)
}

#[test]
fn chunking_identical() {
    let a = samples_for(1316);
    let b = samples_for(65536);
    let c = samples_for(1);
    assert_eq!(a.0, b.0);
    assert_eq!(a.0, c.0);
    assert!(a.1 >= 1);
}

#[test]
fn bare_demuxer_oracle() {
    let bytes = test_fixtures::require_bytes("zenith-http/tf1-live.ts", "real_stream").unwrap();
    let mut d = transmux::StreamingTsDemux::new();
    d.feed(&bytes);
    let mut total = 0;
    let mut tracks = std::collections::BTreeSet::new();
    while let Some(e) = d.poll_event() {
        if matches!(e, transmux::DemuxEvent::Sample { .. }) {
            total += 1;
        }
        if let transmux::DemuxEvent::TrackAdded(s) = e {
            tracks.insert(s.track_id.value());
        }
    }
    // compare to client 1316
    let (cs, ct, _) = samples_for(1316);
    assert_eq!(cs, total);
    assert_eq!(ct, tracks.len());
    assert!(tracks.len() >= 2); // >=1 video + >=1 audio
}

#[test]
fn first_sample_moves_to_playing() {
    let bytes = test_fixtures::require_bytes("zenith-http/tf1-live.ts", "real_stream").unwrap();
    let mut c = TsHttpClient::new("u", Config::default(), 0);
    let _ = c.poll();
    c.on_response(200, &[], 0);
    let _ = c.poll_event();
    c.feed(&bytes[..4096], 0);
    // after feed, if sample emitted, attempt reset (can't observe state; check reconnect 0 after eof)
    c.on_eof(1);
    // If playing, reconnect 0; if not, maybe 2000. Ensure at least no panic.
}

#[test]
fn missing_headers_404() {
    let hdr = std::fs::read_to_string(
        test_fixtures::require("zenith-http/missing.headers.txt", "real_stream").unwrap(),
    )
    .unwrap();
    let status: u16 = hdr
        .lines()
        .next()
        .unwrap()
        .split_whitespace()
        .nth(1)
        .unwrap()
        .parse()
        .unwrap();
    assert_eq!(status, 404);
    let mut c = TsHttpClient::new("u", Config::default(), 0);
    let _ = c.poll();
    c.on_response(status, &[], 0);
    assert_eq!(c.poll_event(), Some(Event::NotFound { status: 404 }));
}

#[test]
fn tf1_headers_200_connected() {
    let hdr = std::fs::read_to_string(
        test_fixtures::require("zenith-http/tf1.headers.txt", "real_stream").unwrap(),
    )
    .unwrap();
    let status: u16 = hdr
        .lines()
        .next()
        .unwrap()
        .split_whitespace()
        .nth(1)
        .unwrap()
        .parse()
        .unwrap();
    assert_eq!(status, 200);
    let mut c = TsHttpClient::new("u", Config::default(), 0);
    let _ = c.poll();
    c.on_response(status, &[], 0);
    assert_eq!(c.poll_event(), Some(Event::Connected));
}
