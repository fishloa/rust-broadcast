use tshttp_runtime::{Action, Config, Event, TsHttpClient};

fn samples_for(chunk_size: usize) -> Option<(usize, usize, usize)> {
    let bytes = test_fixtures::require_bytes("zenith-http/tf1-live.ts", "real_stream")?;
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
    Some((total_samples, tracks.len(), 0))
}

#[test]
fn chunking_identical() {
    let (Some(a), Some(b), Some(c)) = (samples_for(1316), samples_for(65536), samples_for(1))
    else {
        return;
    };
    assert_eq!(a.0, b.0);
    assert_eq!(a.0, c.0);
    assert!(a.1 >= 1);
}

#[test]
fn bare_demuxer_oracle() {
    let Some(bytes) = test_fixtures::require_bytes("zenith-http/tf1-live.ts", "real_stream") else {
        return;
    };
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
    let (cs, ct, _) = samples_for(1316).expect("fixture present: required above");
    assert_eq!(cs, total);
    assert_eq!(ct, tracks.len());
    assert!(tracks.len() >= 2); // >=1 video + >=1 audio
}

#[test]
fn first_sample_moves_to_playing() {
    let Some(bytes) = test_fixtures::require_bytes("zenith-http/tf1-live.ts", "real_stream") else {
        return;
    };
    let mut c = TsHttpClient::new("u", Config::default(), 0);
    let _ = c.poll();
    c.on_response(200, &[], 0);
    let _ = c.poll_event();
    let mut sampled = false;
    for chunk in bytes.chunks(1316) {
        c.feed(chunk, 0);
        while let Some(e) = c.poll_event() {
            sampled |= matches!(e, Event::Demux(transmux::DemuxEvent::Sample { .. }));
        }
        if sampled {
            break;
        }
    }
    assert!(sampled, "no Sample in the whole fixture");
    c.on_eof(5_000);
    assert_eq!(c.poll(), Some(Action::Reconnect { after_ms: 0 }));
    assert_eq!(c.poll(), None);
}

#[test]
fn eof_before_any_sample_backs_off() {
    let mut c = TsHttpClient::new("u", Config::default(), 0);
    let _ = c.poll();
    c.on_response(200, &[], 0);
    let _ = c.poll_event();
    c.on_eof(5_000);
    assert_eq!(c.poll(), Some(Action::Reconnect { after_ms: 2_000 }));
}

#[test]
fn missing_headers_404() {
    let Some(path) = test_fixtures::require("zenith-http/missing.headers.txt", "real_stream")
    else {
        return;
    };
    let hdr = std::fs::read_to_string(path).unwrap();
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
    assert!(matches!(
        c.poll_event(),
        Some(Event::NotFound { status: 404 })
    ));
}

#[test]
fn tf1_headers_200_connected() {
    let Some(path) = test_fixtures::require("zenith-http/tf1.headers.txt", "real_stream") else {
        return;
    };
    let hdr = std::fs::read_to_string(path).unwrap();
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
    assert!(matches!(c.poll_event(), Some(Event::Connected)));
}
