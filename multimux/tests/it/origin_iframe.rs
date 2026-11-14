//! End-to-end integration tests for the I-frame (trick-play) origin path
//! (issue #1413 P3b). Reuses the `feed_via_segmenter` pattern from
//! `origin_llhls.rs`.

use crate::support::origin::{body_bytes, body_string, get};
use crate::support::segmenter_feed::{
    PART_TARGET_MS, TARGET_DURATION_SECS, feed_via_segmenter, video_batches_n, video_track_spec,
};
use broadcast_common::timescale::Timescale;
use multimux::route::RouteConfig;
use std::collections::HashMap;
use std::sync::Arc;

use transmux::TrackId;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use tower::ServiceExt;

use multimux::origin::{AppState, router};
use multimux::output::Output;
use multimux::output::hls::HlsOutput;
use multimux::route::RouteHandle;
use transmux::ll_hls::LlHlsSegmenter;
use transmux::pipeline::{Sample, TrackSpec};

fn video_batches() -> Vec<Vec<(u32, Sample)>> {
    video_batches_n(120)
}

#[tokio::test]
async fn iframe_master_variant_gating() {
    // before any segment: no I-frame variant
    let store = Arc::new(
        RouteHandle::new(
            RouteConfig::default()
                .with_target_duration_secs(TARGET_DURATION_SECS)
                .with_part_target_ms(PART_TARGET_MS)
                .with_window_segments(8),
        )
        .with_declared_bandwidth_bps(5_000_000),
    );
    let seg = LlHlsSegmenter::with_part_target(
        vec![video_track_spec()],
        transmux::VIDEO_CLOCK_TIMESCALE,
        TARGET_DURATION_SECS,
        PART_TARGET_MS,
    )
    .expect("segmenter builds");
    let program = media_plane::ProgramId(0);
    store.publish_new_program(program);
    store.set_init(program, seg.init_segment().expect("init"));

    let mut streams = HashMap::new();
    streams.insert(
        "cam".to_string(),
        (
            store.clone(),
            vec![Arc::new(HlsOutput::default()) as Arc<dyn Output>],
        ),
    );
    let app = router(Arc::new(AppState::new(streams)));
    let resp = app.clone().oneshot(get("/cam/master.m3u8")).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let playlist = body_string(resp).await;
    assert!(
        !playlist.contains("EXT-X-I-FRAME-STREAM-INF"),
        "before segments: {playlist}"
    );

    // after segments: exactly one
    let store2 = Arc::new(RouteHandle::new(
        RouteConfig::default()
            .with_target_duration_secs(TARGET_DURATION_SECS)
            .with_part_target_ms(PART_TARGET_MS)
            .with_window_segments(8),
    ));
    feed_via_segmenter(&store2, vec![video_track_spec()], video_batches());
    let mut streams2 = HashMap::new();
    streams2.insert(
        "cam".to_string(),
        (
            store2.clone(),
            vec![Arc::new(HlsOutput::default()) as Arc<dyn Output>],
        ),
    );
    let app2 = router(Arc::new(AppState::new(streams2)));
    let resp = app2.clone().oneshot(get("/cam/master.m3u8")).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let playlist = body_string(resp).await;
    assert_eq!(
        playlist.matches("EXT-X-I-FRAME-STREAM-INF").count(),
        1,
        "playlist: {playlist}"
    );
    assert!(
        playlist.contains("URI=\"iframe.m3u8\""),
        "playlist: {playlist}"
    );

    // audio-only route: no I-frame variant
    use transmux::rtp_sdp::aac_config_from_asc_hex;
    let audio_spec = TrackSpec::new(
        TrackId::new(1),
        Timescale::from_nonzero(48000),
        aac_config_from_asc_hex("1210").expect("aac"),
    );
    let mut audio_batches = Vec::new();
    for i in 0..120u32 {
        let data = vec![0xBBu8; 64];
        let sample = Sample::new(
            data,
            Some(i64::from(i) * 1024),
            Some(i64::from(i) * 1024),
            Some(1024),
            true,
        );
        audio_batches.push(vec![(1u32, sample)]);
    }
    let audio_store = Arc::new(RouteHandle::new(
        RouteConfig::default()
            .with_target_duration_secs(TARGET_DURATION_SECS)
            .with_part_target_ms(PART_TARGET_MS)
            .with_window_segments(8),
    ));
    feed_via_segmenter(&audio_store, vec![audio_spec], audio_batches);
    let mut streams3 = HashMap::new();
    streams3.insert(
        "cam".to_string(),
        (
            audio_store.clone(),
            vec![Arc::new(HlsOutput::default()) as Arc<dyn Output>],
        ),
    );
    let app3 = router(Arc::new(AppState::new(streams3)));
    let resp = app3.clone().oneshot(get("/cam/master.m3u8")).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let playlist = body_string(resp).await;
    assert!(
        !playlist.contains("EXT-X-I-FRAME-STREAM-INF"),
        "audio-only: {playlist}"
    );
}

#[tokio::test]
async fn iframe_playlist_byterange_matches_independent_calc() {
    let store = Arc::new(
        RouteHandle::new(
            RouteConfig::default()
                .with_target_duration_secs(TARGET_DURATION_SECS)
                .with_part_target_ms(PART_TARGET_MS)
                .with_window_segments(8),
        )
        .with_declared_bandwidth_bps(5_000_000),
    );
    feed_via_segmenter(&store, vec![video_track_spec()], video_batches());

    let mut streams = HashMap::new();
    streams.insert(
        "cam".to_string(),
        (
            store.clone(),
            vec![Arc::new(HlsOutput::default()) as Arc<dyn Output>],
        ),
    );
    let app = router(Arc::new(AppState::new(streams)));

    let resp = app.clone().oneshot(get("/cam/iframe.m3u8")).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let playlist = body_string(resp).await;
    assert!(
        playlist.contains("#EXT-X-I-FRAMES-ONLY"),
        "playlist: {playlist}"
    );

    // parse entries
    let mut entries: Vec<(u64, String)> = Vec::new();
    let lines: Vec<&str> = playlist.lines().collect();
    let mut i = 0;
    while i < lines.len() {
        if let Some(rest) = lines[i].strip_prefix("#EXT-X-BYTERANGE:") {
            let len: u64 = rest.split('@').next().unwrap().trim().parse().unwrap();
            let mut j = i + 1;
            while j < lines.len()
                && (lines[j].trim().is_empty() || lines[j].trim_start().starts_with('#'))
            {
                j += 1;
            }
            if j < lines.len() {
                entries.push((len, lines[j].trim().to_string()));
            }
            i = j;
        } else {
            i += 1;
        }
    }
    assert!(!entries.is_empty(), "no entries: {playlist}");

    for (len, uri) in &entries {
        // independent calc from served segment bytes
        let resp = app
            .clone()
            .oneshot(get(&format!("/cam/{uri}")))
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let seg = body_bytes(resp).await;
        let expected = first_sample_end(&seg);
        assert_eq!(
            *len, expected,
            "uri {uri}: advertised {len} vs computed {expected}"
        );
    }
}

fn first_sample_end(seg: &[u8]) -> u64 {
    let found = transmux::box_types::clamped_boxes(seg)
        .find(|b| b.header.box_type.is(b"moof"))
        .expect("moof");
    let moof_off = found.start;
    let body = &seg[found.payload_start()..found.end];
    let moof = transmux::movie_fragment::MovieFragmentBox::parse_body(body).expect("moof parse");
    let mdats = transmux::frag_offsets::mdat_ranges(seg);
    let samples =
        transmux::frag_offsets::sample_ranges_in(seg, &mdats, moof_off, &moof, TrackId::new(1))
            .expect("sample ranges");
    samples[0].end as u64
}

#[tokio::test]
async fn iframe_range_requests() {
    let store = Arc::new(
        RouteHandle::new(
            RouteConfig::default()
                .with_target_duration_secs(TARGET_DURATION_SECS)
                .with_part_target_ms(PART_TARGET_MS)
                .with_window_segments(8),
        )
        .with_declared_bandwidth_bps(5_000_000),
    );
    feed_via_segmenter(&store, vec![video_track_spec()], video_batches());

    let mut streams = HashMap::new();
    streams.insert(
        "cam".to_string(),
        (
            store.clone(),
            vec![Arc::new(HlsOutput::default()) as Arc<dyn Output>],
        ),
    );
    let app = router(Arc::new(AppState::new(streams)));

    // get one entry
    let resp = app.clone().oneshot(get("/cam/iframe.m3u8")).await.unwrap();
    let playlist = body_string(resp).await;
    let mut entries: Vec<(u64, String)> = Vec::new();
    let lines: Vec<&str> = playlist.lines().collect();
    let mut i = 0;
    while i < lines.len() {
        if let Some(rest) = lines[i].strip_prefix("#EXT-X-BYTERANGE:") {
            let len: u64 = rest.split('@').next().unwrap().trim().parse().unwrap();
            let mut j = i + 1;
            while j < lines.len()
                && (lines[j].trim().is_empty() || lines[j].trim_start().starts_with('#'))
            {
                j += 1;
            }
            if j < lines.len() {
                entries.push((len, lines[j].trim().to_string()));
            }
            i = j;
        } else {
            i += 1;
        }
    }
    let (len, uri) = entries[0].clone();

    // full segment
    let resp = app
        .clone()
        .oneshot(get(&format!("/cam/{uri}")))
        .await
        .unwrap();
    let total_seg = body_bytes(resp).await;

    // range request
    let req = Request::builder()
        .uri(format!("/cam/{uri}"))
        .header("range", format!("bytes=0-{}", len - 1))
        .body(Body::empty())
        .unwrap();
    let resp = app.clone().oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::PARTIAL_CONTENT);
    let cr = resp
        .headers()
        .get("content-range")
        .unwrap()
        .to_str()
        .unwrap()
        .to_string();
    assert_eq!(cr, format!("bytes 0-{}/{}", len - 1, total_seg.len()));
    let body = body_bytes(resp).await;
    assert_eq!(body, total_seg[..len as usize]);
}

#[tokio::test]
async fn iframe_part_query_rejected() {
    let store = Arc::new(
        RouteHandle::new(
            RouteConfig::default()
                .with_target_duration_secs(TARGET_DURATION_SECS)
                .with_part_target_ms(PART_TARGET_MS)
                .with_window_segments(8),
        )
        .with_declared_bandwidth_bps(5_000_000),
    );
    feed_via_segmenter(&store, vec![video_track_spec()], video_batches());
    let mut streams = HashMap::new();
    streams.insert(
        "cam".to_string(),
        (
            store.clone(),
            vec![Arc::new(HlsOutput::default()) as Arc<dyn Output>],
        ),
    );
    let app = router(Arc::new(AppState::new(streams)));
    let resp = app
        .oneshot(get("/cam/iframe.m3u8?_HLS_part=0"))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
}

/// RFC 8216bis section 6.2.5.2: `_HLS_part` with `_HLS_msn` is a valid
/// blocking reload even on a playlist without Partial Segments; only
/// `_HLS_part` alone is a Bad Request. Apple's validator sends
/// `_HLS_msn=M&_HLS_part=N` to the I-frame playlist.
#[tokio::test]
async fn iframe_msn_with_part_query_served() {
    let store = Arc::new(
        RouteHandle::new(
            RouteConfig::default()
                .with_target_duration_secs(TARGET_DURATION_SECS)
                .with_part_target_ms(PART_TARGET_MS)
                .with_window_segments(8),
        )
        .with_declared_bandwidth_bps(5_000_000),
    );
    feed_via_segmenter(&store, vec![video_track_spec()], video_batches());
    let mut streams = HashMap::new();
    streams.insert(
        "cam".to_string(),
        (
            store.clone(),
            vec![Arc::new(HlsOutput::default()) as Arc<dyn Output>],
        ),
    );
    let app = router(Arc::new(AppState::new(streams)));
    let resp = app
        .oneshot(get("/cam/iframe.m3u8?_HLS_msn=1&_HLS_part=0"))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    assert!(body_string(resp).await.contains("EXT-X-I-FRAMES-ONLY"));
}

/// RFC 8216bis section 6.2.5.2: "A Server that cannot provide the requested
/// Playlist after blocking for more than three Target Durations SHOULD
/// return Service Unavailable, such as HTTP 503." A blocking reload that the
/// stream never satisfies answers 503 on the media and I-frame playlists,
/// not 404.
#[tokio::test]
async fn blocking_reload_timeout_is_503() {
    let store = Arc::new(
        RouteHandle::new(
            RouteConfig::default()
                .with_target_duration_secs(TARGET_DURATION_SECS)
                .with_part_target_ms(PART_TARGET_MS)
                .with_window_segments(8),
        )
        .with_declared_bandwidth_bps(5_000_000),
    );
    feed_via_segmenter(&store, vec![video_track_spec()], video_batches());
    let mut streams = HashMap::new();
    streams.insert(
        "cam".to_string(),
        (
            store.clone(),
            vec![Arc::new(HlsOutput::default()) as Arc<dyn Output>],
        ),
    );
    let app = router(Arc::new(AppState::new(streams)));
    let iframe = body_string(app.clone().oneshot(get("/cam/iframe.m3u8")).await.unwrap()).await;
    let first: u64 = iframe
        .lines()
        .find_map(|l| l.strip_prefix("#EXT-X-MEDIA-SEQUENCE:"))
        .unwrap()
        .parse()
        .unwrap();
    let last_closed = first + iframe.matches("#EXTINF").count() as u64 - 1;

    let iframe_resp = app.clone().oneshot(get(&format!(
        "/cam/iframe.m3u8?_HLS_msn={}",
        last_closed + 1
    )));
    let media_resp = app.clone().oneshot(get(&format!(
        "/cam/media.m3u8?_HLS_msn={}",
        last_closed + 2
    )));
    let (iframe_resp, media_resp) = tokio::join!(iframe_resp, media_resp);
    assert_eq!(
        iframe_resp.unwrap().status(),
        StatusCode::SERVICE_UNAVAILABLE
    );
    assert_eq!(
        media_resp.unwrap().status(),
        StatusCode::SERVICE_UNAVAILABLE
    );
}

#[tokio::test]
async fn ts_hls_no_iframe() {
    let store = Arc::new(
        RouteHandle::new(
            RouteConfig::default()
                .with_target_duration_secs(TARGET_DURATION_SECS)
                .with_part_target_ms(PART_TARGET_MS)
                .with_window_segments(8),
        )
        .with_declared_bandwidth_bps(5_000_000),
    );
    let program = media_plane::ProgramId(0);
    store.publish_new_program(program);

    let mut streams = HashMap::new();
    streams.insert(
        "cam".to_string(),
        (
            store.clone(),
            vec![Arc::new(HlsOutput::segmented(
                multimux::output::hls::DEFAULT_PLAYLIST_NAME,
            )) as Arc<dyn Output>],
        ),
    );
    let app = router(Arc::new(AppState::new(streams)));
    let resp = app.clone().oneshot(get("/cam/iframe.m3u8")).await.unwrap();
    assert_eq!(resp.status(), StatusCode::NOT_FOUND);
    let resp = app.clone().oneshot(get("/cam/master.m3u8")).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let playlist = body_string(resp).await;
    assert!(
        !playlist.contains("EXT-X-I-FRAME-STREAM-INF"),
        "playlist: {playlist}"
    );
}

// validator test (f)
#[tokio::test]
async fn iframe_validator_clean() {
    use std::fs;
    use std::path::PathBuf;
    use std::time::Duration;

    // availability check
    if !test_fixtures::process::msv::available() {
        eprintln!("SKIP iframe_validator_clean: `mediastreamvalidator` not on PATH ...");
        return;
    }

    let store = Arc::new(
        RouteHandle::new(
            RouteConfig::default()
                .with_target_duration_secs(TARGET_DURATION_SECS)
                .with_part_target_ms(PART_TARGET_MS)
                .with_window_segments(8),
        )
        .with_declared_bandwidth_bps(5_000_000),
    );
    let program = media_plane::ProgramId(0);
    store.publish_new_program(program);
    let mut seg = LlHlsSegmenter::with_part_target(
        vec![video_track_spec()],
        transmux::VIDEO_CLOCK_TIMESCALE,
        TARGET_DURATION_SECS,
        PART_TARGET_MS,
    )
    .expect("segmenter builds");
    store.set_init(program, seg.init_segment().expect("init"));
    for batch in video_batches_n(105) {
        for (track_id, sample) in batch {
            seg.push(TrackId::new(track_id), sample).expect("push");
        }
        for part in seg.take_ready_parts() {
            store.add_part(program, part);
        }
        for segment in seg.take_ready_segments() {
            store.add_segment(program, segment).expect("add");
        }
    }
    // leave the in-progress (open) segment unflushed so a preload hint exists

    let mut streams = HashMap::new();
    streams.insert(
        "cam".to_string(),
        (
            store.clone(),
            vec![Arc::new(HlsOutput::default()) as Arc<dyn Output>],
        ),
    );
    let app = router(Arc::new(AppState::new(streams)));

    let scratch = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../target/iframe-validator-tmp");
    let _ = fs::remove_dir_all(&scratch);
    fs::create_dir_all(&scratch).unwrap();

    // fetch playlists
    for name in ["master.m3u8", "media.m3u8", "iframe.m3u8"] {
        let resp = app
            .clone()
            .oneshot(get(&format!("/cam/{name}")))
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        fs::write(scratch.join(name), body_bytes(resp).await).unwrap();
    }

    // collect URIs from media + iframe playlists
    let mut uris: std::collections::HashSet<String> = std::collections::HashSet::new();
    for name in ["media.m3u8", "iframe.m3u8"] {
        let text = fs::read_to_string(scratch.join(name)).unwrap();
        for line in text.lines() {
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
        let resp = app
            .clone()
            .oneshot(get(&format!("/cam/{uri}")))
            .await
            .unwrap();
        if resp.status() == StatusCode::OK {
            fs::write(scratch.join(uri), body_bytes(resp).await).unwrap();
        }
    }

    // run validator
    let json =
        test_fixtures::process::msv::run_report(&scratch, "master.m3u8", Duration::from_secs(35));
    let findings = test_fixtures::process::msv::findings(&json);
    // Level-1 (MUST) findings Apple's validator still reports against the
    // live LL-HLS origin, each tied to its open issue. This is an exact
    // allow-list, not a suppression: any other level-1 finding fails the
    // test, and so does a listed one that disappears.
    const KNOWN: &[(&str, &str)] = &[
        // Issue #1595: a one-rendition low-latency route has no sibling for the
        // media playlist to report (RFC 8216bis Appendix B.1); a report for the
        // I-frame playlist is rejected as "Unknown rendition report".
        ("No rendition reports in low-latency playlist", "#1595"),
        // Issue #1594: the validator reports this on a playlist without
        // EXT-X-PART (here the I-frame playlist) whenever the primary has
        // parts, for any EXT-X-SERVER-CONTROL value; dropping the tag silences
        // it and so does a part tag. RFC 8216bis section 6.2.4 requires every
        // Media Playlist to carry the same EXT-X-SERVER-CONTROL, and section
        // 4.4.4.9 says I-frame playlists SHOULD NOT use Partial Segments.
        ("Missing server control attribute", "#1594"),
    ];
    for f in &findings {
        eprintln!("VALIDATOR: {} level={} {}", f.0, f.1, f.2);
    }
    let errors: Vec<_> = findings.iter().filter(|f| f.1 <= 1).collect();
    let unexpected: Vec<_> = errors
        .iter()
        .filter(|f| !KNOWN.iter().any(|(known, _)| *known == f.2))
        .collect();
    assert!(
        unexpected.is_empty(),
        "NEW validator level-1 findings: {unexpected:?}"
    );
    for (known, issue) in KNOWN {
        assert!(
            errors.iter().any(|f| f.2 == *known),
            "known finding {known:?} ({issue}) no longer reported: remove it from KNOWN"
        );
    }
}

// (g) SCTE-35 case
#[tokio::test]
async fn iframe_scte35_emsg_shift() {
    use media_plane::trunk::EventAnchor;
    use timed_metadata::{MediaTime, TimeAnchor, Timeline};

    fn parse_emsg_boxes(bytes: &[u8]) -> Vec<transmux::EmsgBox<'_>> {
        let mut out = Vec::new();
        let mut i = 0usize;
        while i + 8 <= bytes.len() {
            let size =
                u32::from_be_bytes([bytes[i], bytes[i + 1], bytes[i + 2], bytes[i + 3]]) as usize;
            if size < 8 || i + size > bytes.len() {
                break;
            }
            if &bytes[i + 4..i + 8] == b"emsg"
                && let Ok(e) = transmux::EmsgBox::parse(&bytes[i..i + size])
            {
                out.push(e);
            }
            i += size;
        }
        out
    }

    let store = Arc::new(
        RouteHandle::new(
            RouteConfig::default()
                .with_target_duration_secs(TARGET_DURATION_SECS)
                .with_part_target_ms(PART_TARGET_MS)
                .with_window_segments(8),
        )
        .with_declared_bandwidth_bps(5_000_000),
    );
    let program = media_plane::ProgramId(0);
    let trunk = store.publish_new_program(program);

    // Note segment boundaries up front on a short-lived segment writer.
    let seg_writer = trunk.segment_writer().expect("seg writer");
    seg_writer.set_time_anchor(TimeAnchor {
        pts_90k: 0,
        utc_epoch_ms: 1_000_000_000_000,
    });
    seg_writer.note_segment_start(media_plane::ids::SegmentSeq::new(1), MediaTime(0));
    seg_writer.note_segment_start(media_plane::ids::SegmentSeq::new(2), MediaTime(90_000));
    drop(seg_writer);

    // build segmenter init
    let mut seg = LlHlsSegmenter::with_part_target(
        vec![video_track_spec()],
        transmux::VIDEO_CLOCK_TIMESCALE,
        TARGET_DURATION_SECS,
        PART_TARGET_MS,
    )
    .expect("segmenter builds");
    store.set_init(program, seg.init_segment().expect("init"));

    for batch in video_batches() {
        for (track_id, sample) in batch {
            seg.push(TrackId::new(track_id), sample).expect("push");
        }
        for part in seg.take_ready_parts() {
            store.add_part(program, part);
        }
        for segment in seg.take_ready_segments() {
            store.add_segment(program, segment).expect("add");
        }
    }
    seg.flush().expect("flush");
    for part in seg.take_ready_parts() {
        store.add_part(program, part);
    }
    for segment in seg.take_ready_segments() {
        store.add_segment(program, segment).expect("add");
    }

    // publish scte35 event at start of segment 1
    let writer = trunk.writer().expect("writer");
    let splice_bytes = test_fixtures::read("scte35/splice-insert-2002.bin");
    let mut timeline = Timeline::new();
    let ev = timeline.push_scte35(&splice_bytes).unwrap();
    writer.publish_event(
        ev.clone(),
        EventAnchor::Media(ev.at.unwrap_or(MediaTime(0))),
    );

    let mut streams = HashMap::new();
    streams.insert(
        "cam".to_string(),
        (
            store.clone(),
            vec![Arc::new(HlsOutput::default()) as Arc<dyn Output>],
        ),
    );
    let app = router(Arc::new(AppState::new(streams)));

    // I-frame playlist URIs use the iframe-seg- prefix.
    let resp = app.clone().oneshot(get("/cam/iframe.m3u8")).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let playlist = body_string(resp).await;

    let mut len: u64 = 0;
    let mut uri = String::new();
    let lines: Vec<&str> = playlist.lines().collect();
    let mut i = 0;
    while i < lines.len() {
        if let Some(rest) = lines[i].strip_prefix("#EXT-X-BYTERANGE:") {
            len = rest.split('@').next().unwrap().trim().parse().unwrap();
            let mut j = i + 1;
            while j < lines.len()
                && (lines[j].trim().is_empty() || lines[j].trim_start().starts_with('#'))
            {
                j += 1;
            }
            if j < lines.len() {
                uri = lines[j].trim().to_string();
            }
            break;
        } else {
            i += 1;
        }
    }
    assert!(!uri.is_empty(), "no uri found");
    assert!(uri.starts_with("iframe-seg-"), "uri={uri}");

    // stored bytes behind the I-frame URI (no emsg)
    let resp = app
        .clone()
        .oneshot(get(&format!("/cam/{uri}")))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let stored = body_bytes(resp).await;
    let computed = first_sample_end(&stored);
    assert_eq!(len, computed, "advertised len must match stored geometry");
    assert!(
        parse_emsg_boxes(&stored).is_empty(),
        "stored bytes must have no emsg"
    );

    // Range request returns first len bytes of stored segment, no emsg in them.
    let req = Request::builder()
        .uri(format!("/cam/{uri}"))
        .header("range", format!("bytes=0-{}", len - 1))
        .body(Body::empty())
        .unwrap();
    let resp = app.clone().oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::PARTIAL_CONTENT);
    let range_body = body_bytes(resp).await;
    assert_eq!(range_body, stored[..len as usize]);
    assert!(parse_emsg_boxes(&range_body).is_empty());

    // nonexistent iframe-seg seq => 404
    let nonexistent = {
        let (prefix, _) = uri.rsplit_once('-').unwrap();
        format!("{prefix}-999.m4s")
    };
    let resp = app
        .clone()
        .oneshot(get(&format!("/cam/{nonexistent}")))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::NOT_FOUND);

    // media seg- fetch of same seq returns emsg-injected bytes
    let media_uri = uri.replacen("iframe-seg-", "seg-", 1);
    let resp = app
        .clone()
        .oneshot(get(&format!("/cam/{media_uri}")))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let media_served = body_bytes(resp).await;
    assert!(
        media_served.len() > stored.len(),
        "emsg injection must grow length"
    );
    assert!(
        !parse_emsg_boxes(&media_served).is_empty(),
        "media segment must contain emsg"
    );
    assert!(parse_emsg_boxes(&stored).is_empty());

    let _ = trunk;
}

#[tokio::test]
async fn iframe_rendition_report_uses_configured_playlist_name() {
    let store = Arc::new(
        RouteHandle::new(
            RouteConfig::default()
                .with_target_duration_secs(TARGET_DURATION_SECS)
                .with_part_target_ms(PART_TARGET_MS)
                .with_window_segments(8),
        )
        .with_declared_bandwidth_bps(5_000_000),
    );
    feed_via_segmenter(&store, vec![video_track_spec()], video_batches());

    let mut streams = HashMap::new();
    streams.insert(
        "cam".to_string(),
        (
            store.clone(),
            vec![Arc::new(HlsOutput::low_latency("live/main.m3u8")) as Arc<dyn Output>],
        ),
    );
    let app = router(Arc::new(AppState::new(streams)));

    let resp = app.clone().oneshot(get("/cam/iframe.m3u8")).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let playlist = body_string(resp).await;
    assert!(
        playlist.contains("#EXT-X-RENDITION-REPORT:URI=\"live/main.m3u8\""),
        "playlist: {playlist}"
    );
}
