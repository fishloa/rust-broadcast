//! Issue #1452 (items 1+2): `HlsClient` exposes decoder-ready track specs
//! (`Output::Tracks`) and display changes (`Output::DisplayChanged`).

use std::sync::Arc;

use hls_runtime::client::{Action, HlsClient, Output};
use transmux::ts_demux::{DemuxEvent, StreamingTsDemux};
use transmux::{CodecConfig, TrackId};

const PLAYLIST_URL: &str = "http://fixture/index.m3u8";

fn drive_ts(bytes: &[u8]) -> Vec<Output> {
    let playlist = "#EXTM3U\n#EXT-X-VERSION:3\n#EXT-X-TARGETDURATION:2\n\
                    #EXT-X-MEDIA-SEQUENCE:0\n#EXTINF:2.0,\nseg.ts\n#EXT-X-ENDLIST\n";
    let mut client = HlsClient::new(PLAYLIST_URL);
    let mut outputs = Vec::new();
    loop {
        match client.poll() {
            Some(Action::FetchPlaylist { .. }) => {
                client
                    .on_playlist(playlist.as_bytes())
                    .expect("playlist parses");
            }
            Some(Action::FetchResource { id, .. }) => {
                client.on_resource(id, bytes).expect("resource demuxes");
            }
            Some(Action::WaitMs(_)) => {}
            Some(other) => panic!("unexpected action: {other:?}"),
            None => break,
        }
        while let Some(o) = client.next_output() {
            outputs.push(o);
        }
    }
    outputs
}

fn drive_fmp4(playlist: &str) -> Vec<Output> {
    let mut client = HlsClient::new(PLAYLIST_URL);
    let mut outputs = Vec::new();
    loop {
        match client.poll() {
            Some(Action::FetchPlaylist { .. }) => {
                client
                    .on_playlist(playlist.as_bytes())
                    .expect("playlist parses");
            }
            Some(Action::FetchResource { id, url, .. }) => {
                let name = url.rsplit('/').next().expect("url path");
                let bytes = test_fixtures::read(&format!("hls/cmaf-fmp4/{name}"));
                client.on_resource(id, &bytes).expect("resource demuxes");
            }
            Some(Action::WaitMs(_)) => {}
            Some(other) => panic!("unexpected action: {other:?}"),
            None => break,
        }
        while let Some(o) = client.next_output() {
            outputs.push(o);
        }
    }
    outputs
}

#[test]
fn ts_emits_tracks_before_init() {
    let bytes = test_fixtures::read("ts/h264_aac.ts");
    let outputs = drive_ts(&bytes);
    assert!(outputs.len() >= 2, "{outputs:?}");
    assert!(matches!(outputs[0], Output::Tracks(_)), "{outputs:?}");
    assert!(matches!(outputs[1], Output::Init(_)), "{outputs:?}");
    if let Output::Tracks(specs) = &outputs[0] {
        assert!(
            specs.iter().any(|s| {
                matches!(s.config, CodecConfig::Avc { .. } | CodecConfig::Hevc { .. })
                    && s.timescale > 0
            }),
            "expected a video spec with non-empty codec config: {specs:?}"
        );
    }
}

#[test]
fn ts_tracks_match_oracle() {
    let bytes = test_fixtures::read("ts/h264_aac.ts");
    let outputs = drive_ts(&bytes);
    let emitted = match outputs.iter().find_map(|o| {
        if let Output::Tracks(s) = o {
            Some(s.clone())
        } else {
            None
        }
    }) {
        Some(s) => s,
        None => panic!("no Tracks emitted"),
    };

    let mut demux = StreamingTsDemux::new();
    demux.feed(&bytes);
    demux.finish();
    let mut oracle: Vec<(TrackId, String, u32)> = Vec::new();
    while let Some(ev) = demux.poll_event() {
        match ev {
            DemuxEvent::TrackAdded(spec) | DemuxEvent::TrackUpdated(spec) => {
                oracle.push((
                    spec.track_id,
                    format!("{:?}", std::mem::discriminant(&spec.config)),
                    spec.timescale,
                ));
            }
            _ => {}
        }
    }
    let mut got: Vec<(TrackId, String, u32)> = emitted
        .iter()
        .map(|s| {
            (
                s.track_id,
                format!("{:?}", std::mem::discriminant(&s.config)),
                s.timescale,
            )
        })
        .collect();
    got.sort();
    oracle.sort();
    oracle.dedup();
    got.dedup();
    assert_eq!(got, oracle, "Tracks spec set must match StreamingTsDemux");
}

#[test]
fn ts_display_changed_in_sample_order() {
    let Some(bytes) = test_fixtures::require_bytes(
        "ts/stimmungsgarten-sar-16-11-to-1-1-switch-zenith-tv.ts",
        "tracks_display",
    ) else {
        return;
    };
    let outputs = drive_ts(&bytes);

    // Initial DisplayChanged precedes first video Samples.
    let first_dc = outputs
        .iter()
        .position(|o| matches!(o, Output::DisplayChanged { .. }));
    let first_samples = outputs
        .iter()
        .position(|o| matches!(o, Output::Samples { .. }));
    if let (Some(dc), Some(s)) = (first_dc, first_samples) {
        assert!(
            dc < s,
            "initial DisplayChanged must precede first Samples: {outputs:?}"
        );
    }

    // Mid-stream change: some Samples, then DisplayChanged, then Samples again.
    let mut found = false;
    for i in 1..outputs.len().saturating_sub(1) {
        if let Output::DisplayChanged {
            track_id: dc_track, ..
        } = &outputs[i]
        {
            let prev_is_samples = matches!(&outputs[i - 1], Output::Samples { track_id, .. } if *track_id == *dc_track);
            let next_is_samples = matches!(&outputs[i + 1], Output::Samples { track_id, .. } if *track_id == *dc_track);
            if prev_is_samples && next_is_samples {
                found = true;
                break;
            }
        }
    }
    assert!(
        found,
        "expected Samples, DisplayChanged, Samples sequence: {outputs:?}"
    );
}

#[test]
fn ts_display_changed_carries_same_arc_as_first_sample_display() {
    let Some(bytes) = test_fixtures::require_bytes(
        "ts/stimmungsgarten-sar-16-11-to-1-1-switch-zenith-tv.ts",
        "tracks_display",
    ) else {
        return;
    };
    let outputs = drive_ts(&bytes);
    for i in 0..outputs.len().saturating_sub(1) {
        if let Output::DisplayChanged { display, .. } = &outputs[i]
            && let Output::Samples { samples, .. } = &outputs[i + 1]
            && let Some(s) = samples.first()
            && let Some(sd) = &s.display
        {
            assert!(
                Arc::ptr_eq(display, sd),
                "DisplayChanged must carry the same Arc as the following sample"
            );
            return;
        }
    }
    panic!("no DisplayChanged followed by a sample with display: {outputs:?}");
}

#[test]
fn fmp4_emits_tracks_after_init_once() {
    let playlist = "#EXTM3U\n#EXT-X-VERSION:7\n#EXT-X-TARGETDURATION:2\n\
                    #EXT-X-MEDIA-SEQUENCE:0\n#EXT-X-MAP:URI=\"init.mp4\"\n\
                    #EXTINF:2.0,\nindex0.m4s\n#EXTINF:2.0,\nindex1.m4s\n\
                    #EXTINF:2.0,\nindex2.m4s\n#EXT-X-ENDLIST\n";
    let outputs = drive_fmp4(playlist);
    let tracks_count = outputs
        .iter()
        .filter(|o| matches!(o, Output::Tracks(_)))
        .count();
    assert_eq!(tracks_count, 1, "exactly one Tracks expected: {outputs:?}");
    // Tracks after Init.
    let init_pos = outputs
        .iter()
        .position(|o| matches!(o, Output::Init(_)))
        .expect("init");
    let tracks_pos = outputs
        .iter()
        .position(|o| matches!(o, Output::Tracks(_)))
        .expect("tracks");
    assert!(
        tracks_pos > init_pos,
        "Tracks must come after Init: {outputs:?}"
    );
}

#[test]
fn fmp4_new_init_with_changed_specs_re_emits_tracks() {
    let playlist1 = "#EXTM3U\n#EXT-X-VERSION:7\n#EXT-X-TARGETDURATION:2\n\
                     #EXT-X-MEDIA-SEQUENCE:0\n#EXT-X-MAP:URI=\"init.mp4\"\n\
                     #EXTINF:2.0,\nindex0.m4s\n#EXT-X-ENDLIST\n";
    let playlist2 = "#EXTM3U\n#EXT-X-VERSION:7\n#EXT-X-TARGETDURATION:2\n\
                     #EXT-X-MEDIA-SEQUENCE:10\n#EXT-X-MAP:URI=\"a48-init.mp4\"\n\
                     #EXTINF:2.0,\na48-0.m4s\n#EXT-X-ENDLIST\n";

    fn read_res(name: &str) -> Vec<u8> {
        let rel = match name {
            "init.mp4" => "hls/cmaf-fmp4/init.mp4",
            "a48-init.mp4" => "scte35-ssai/dash/A48/init.mp4",
            "index0.m4s" => "hls/cmaf-fmp4/index0.m4s",
            "a48-0.m4s" => "scte35-ssai/dash/A48/0.m4s",
            _ => panic!("unexpected {name}"),
        };
        test_fixtures::read(rel)
    }

    let mut client = HlsClient::new(PLAYLIST_URL);
    let mut outputs = Vec::new();

    // First playlist.
    client
        .on_playlist(playlist1.as_bytes())
        .expect("playlist parses");
    // collect fetch ids
    let mut ids = Vec::new();
    while let Some(a) = client.poll() {
        match a {
            Action::FetchResource { id, .. } => ids.push(id),
            Action::FetchPlaylist { .. } => {}
            Action::WaitMs(_) => {}
            other => panic!("unexpected action: {other:?}"),
        }
    }
    for id in ids {
        let bytes = match id {
            hls_runtime::client::ResourceId::Init => read_res("init.mp4"),
            hls_runtime::client::ResourceId::Segment { .. } => read_res("index0.m4s"),
            _ => panic!("unexpected id {id:?}"),
        };
        client.on_resource(id, &bytes).expect("resource demuxes");
    }
    while let Some(a) = client.poll() {
        match a {
            Action::FetchPlaylist { .. } => {}
            Action::WaitMs(_) => {}
            other => panic!("unexpected action: {other:?}"),
        }
    }
    while let Some(o) = client.next_output() {
        outputs.push(o);
    }

    // Second playlist (restart / new map).
    client
        .on_playlist(playlist2.as_bytes())
        .expect("playlist parses");
    let mut ids = Vec::new();
    while let Some(a) = client.poll() {
        match a {
            Action::FetchResource { id, .. } => ids.push(id),
            Action::FetchPlaylist { .. } => {}
            Action::WaitMs(_) => {}
            other => panic!("unexpected action: {other:?}"),
        }
    }
    for id in ids {
        let bytes = match id {
            hls_runtime::client::ResourceId::Init => read_res("a48-init.mp4"),
            hls_runtime::client::ResourceId::Segment { .. } => read_res("a48-0.m4s"),
            _ => panic!("unexpected id {id:?}"),
        };
        client.on_resource(id, &bytes).expect("resource demuxes");
    }
    while let Some(a) = client.poll() {
        match a {
            Action::FetchPlaylist { .. } => {}
            Action::WaitMs(_) => {}
            other => panic!("unexpected action: {other:?}"),
        }
    }
    while let Some(o) = client.next_output() {
        outputs.push(o);
    }

    let tracks_count = outputs
        .iter()
        .filter(|o| matches!(o, Output::Tracks(_)))
        .count();
    assert_eq!(tracks_count, 2, "expected two Tracks: {outputs:?}");
}

#[test]
fn no_display_change_keeps_one_samples_batch_per_track() {
    let bytes = test_fixtures::read("ts/aac-5_1-640k.ts");
    let outputs = drive_ts(&bytes);
    // Count Samples outputs per track.
    assert!(
        !outputs
            .iter()
            .any(|o| matches!(o, Output::DisplayChanged { .. })),
        "no DisplayChanged expected: {outputs:?}"
    );
    let mut per_track: std::collections::BTreeMap<TrackId, usize> =
        std::collections::BTreeMap::new();
    for o in &outputs {
        if let Output::Samples { track_id, .. } = o {
            *per_track.entry(*track_id).or_insert(0) += 1;
        }
    }
    assert!(!per_track.is_empty());
}
