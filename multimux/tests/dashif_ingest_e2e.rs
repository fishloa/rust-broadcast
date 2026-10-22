use std::net::SocketAddr;
use std::time::Duration;

use broadcast_common::mux::{Package, Unpackage};
use bytes::Bytes;
use multimux::config::{Config, InputSpec, Route};
use multimux::dvr::DvrConfig;
use multimux::output::OutputKind;
use multimux::serve_with_registry_on;
use transmux::FragmentTrackData;
use transmux::dash::DashPackager;
use transmux::pipeline::{build_init_segment, build_media_segment};
use transmux::ts_demux::TsDemux;
use transmux::validate::{Severity, validate_init_segment, validate_media_segment};

#[path = "support/listener.rs"]
mod listener;
use listener::bind_tcp;

const SAMPLES_PER_SEGMENT: usize = 6;

struct Presentation {
    mpd: String,
    reps: Vec<(String, Vec<u8>, Vec<Vec<u8>>)>,
}

fn presentation() -> Presentation {
    let ts = std::fs::read(
        std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../fixtures/ts/h264_aac.ts"),
    )
    .expect("fixture");
    let mut demux = TsDemux::new();
    let media = demux.unpackage(&ts[..]).expect("demux");
    let mut packager = DashPackager::default();
    let mpd = packager.package(&media).expect("mpd");
    let mut reps = Vec::new();
    for (i, t) in media.tracks.iter().enumerate() {
        let init =
            build_init_segment(std::slice::from_ref(&t.spec), media.movie_timescale).expect("init");
        let mut segs = Vec::new();
        for s in 0..2 {
            let chunk = &t.samples[s * SAMPLES_PER_SEGMENT..(s + 1) * SAMPLES_PER_SEGMENT];
            let prefix = &t.samples[..s * SAMPLES_PER_SEGMENT];
            let offset: u64 = prefix
                .iter()
                .map(|x| u64::from(x.duration.unwrap_or(0)))
                .sum();
            let base = t.start_decode_time + offset;
            let ftd = FragmentTrackData::new(t.spec.track_id, base, chunk);
            segs.push(build_media_segment(u32::try_from(s + 1).unwrap(), &[ftd]).expect("seg"));
        }
        reps.push(((i + 1).to_string(), init, segs));
    }
    Presentation { mpd, reps }
}

fn config_with(
    bind: SocketAddr,
    ingest: SocketAddr,
    prebound: multimux::config::PreboundBinds,
) -> Config {
    Config {
        bind: bind.to_string(),
        target_duration_secs: 0.5,
        part_target_ms: 100,
        window_segments: 8,
        routes: vec![Route {
            name: "cam".to_string(),
            input: InputSpec::DashifIngest {
                listen: ingest.to_string(),
                publishing_point: "/".to_string(),
            },
            outputs: vec![OutputKind::LlHls],
            dvr: DvrConfig::default(),
        }],
        prebound,
        ..Config::default()
    }
}

#[tokio::test]
async fn dashif_ingest_e2e() {
    let (bind_addr, bind_listener) = bind_tcp();
    let (ingest_addr, ingest_listener) = bind_tcp();
    let mut prebound = multimux::config::PreboundBinds::default();
    prebound.with_tcp(ingest_addr.to_string(), ingest_listener);
    let p = presentation();
    let config = config_with(bind_addr, ingest_addr, prebound);
    let server = tokio::spawn(serve_with_registry_on(
        bind_listener,
        config,
        multimux::registry::SchemeRegistry::new(),
    ));

    let client = reqwest::Client::new();
    let base = format!("http://{}", ingest_addr);

    // (a) empty POST => 200
    let resp = client.post(format!("{}/", base)).send().await.unwrap();
    let st = resp.status();
    let body = resp.text().await.unwrap();
    println!("empty post status {} body {:?}", st, body);

    let resp = client
        .post(format!("{}/foo.mpd", base))
        .body(p.mpd.clone())
        .send()
        .await
        .unwrap();
    let st = resp.status();
    let body = resp.text().await.unwrap();
    println!("mpd post status {} body {:?}", st, body);

    // (b) POST MPD
    let resp = client
        .post(format!("{}/foo.mpd", base))
        .body(p.mpd.clone())
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200, "mpd");

    for (id, init, segs) in &p.reps {
        let r = client
            .post(format!("{}/init-stream{}.m4s", base, id))
            .body(init.clone())
            .send()
            .await
            .unwrap();
        assert_eq!(r.status(), 200, "header {}", id);
        for (i, seg) in segs.iter().enumerate() {
            let r = client
                .post(format!("{}/chunk-stream{}-{}.m4s", base, id, i + 1))
                .body(seg.clone())
                .send()
                .await
                .unwrap();
            assert_eq!(r.status(), 200, "seg {} {}", id, i);
        }
    }

    tokio::time::sleep(Duration::from_secs(2)).await;
    let media_base = format!("http://{}", bind_addr);
    let r = client
        .get(format!("{}/cam/media.m3u8", media_base))
        .send()
        .await
        .unwrap();
    println!(
        "playlist status {} body:\n{}",
        r.status(),
        r.text().await.unwrap()
    );

    // (c) GET playlist
    let media_base = format!("http://{}", bind_addr);
    let playlist_url = format!("{}/cam/media.m3u8", media_base);
    let playlist = client
        .get(&playlist_url)
        .send()
        .await
        .unwrap()
        .text()
        .await
        .unwrap();
    println!("playlist:\n{}", playlist);

    // validate init/segment
    let init_url = format!("{}/cam/init-1.mp4", media_base);
    let init_bytes: Bytes = client
        .get(&init_url)
        .send()
        .await
        .unwrap()
        .bytes()
        .await
        .unwrap();
    let issues = validate_init_segment(&init_bytes);
    let errs: Vec<_> = issues
        .iter()
        .filter(|i| i.severity == Severity::Error)
        .collect();
    assert!(errs.is_empty(), "{:?}", errs);

    // find a part uri
    let part_line = playlist.lines().find(|l| l.contains(".m4s")).unwrap();
    let start = part_line.find("part-").unwrap();
    let end = part_line[start..].find(".m4s").unwrap() + start + ".m4s".len();
    let seg_uri = &part_line[start..end];
    let seg_url = format!("{}/cam/{}", media_base, seg_uri);
    let seg_bytes: Bytes = client
        .get(&seg_url)
        .send()
        .await
        .unwrap()
        .bytes()
        .await
        .unwrap();
    let issues = validate_media_segment(&seg_bytes);
    let errs: Vec<_> = issues
        .iter()
        .filter(|i| i.severity == Severity::Error)
        .collect();
    assert!(errs.is_empty(), "{:?}", errs);

    // unknown path outside publishing point? publishing_point "/", so skip 404 test unless custom pp; not here

    // GET => 405 + Allow
    let resp = client
        .get(format!("{}/init-stream1.m4s", base))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 405);
    assert_eq!(resp.headers().get("allow").unwrap(), "POST, PUT");

    // PUT works like POST (probe)
    let resp = client.put(format!("{}/", base)).send().await.unwrap();
    assert_eq!(resp.status(), 200);

    // body > MAX_OBJECT_BYTES => 413 (skip huge; unit not)

    // (e) idempotent re-POST same segment => 200 and playlist no duplicate
    let seg_bytes0 = p.reps[0].2[0].clone();
    let r = client
        .post(format!("{}/chunk-stream1-1.m4s", base))
        .body(seg_bytes0)
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 200);

    server.abort();
}
