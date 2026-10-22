use std::net::SocketAddr;
use std::time::Duration;

use broadcast_common::mux::{Package, Unpackage};
use bytes::Bytes;
use multimux::config::{Config, InputSpec, OutputAuthSpec, Route};
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

struct Presentation {
    mpd: String,
    reps: Vec<(String, Vec<u8>, Vec<Vec<u8>>)>,
    /// The VIDEO representation's segments, each as >= 2 CMAF chunks
    /// (`styp+moof+mdat` each) whose concatenation is the same media as the
    /// whole segment in `reps[0].2`.
    video_chunks: Vec<Vec<Vec<u8>>>,
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
    let mut video_chunks: Vec<Vec<Vec<u8>>> = Vec::new();
    for (i, t) in media.tracks.iter().enumerate() {
        let init =
            build_init_segment(std::slice::from_ref(&t.spec), media.movie_timescale).expect("init");
        // Cut at sync samples (video: three 1 s GOPs; audio: every sample is
        // sync, so thirds). Every segment must be long enough for the
        // segmenter (target 0.5 s) to close one on the next sync.
        let n = t.samples.len();
        let mut cuts: Vec<usize> = t
            .samples
            .iter()
            .enumerate()
            .filter(|(k, s)| *k > 0 && s.flags.is_sync)
            .map(|(k, _)| k)
            .collect();
        if cuts.len() + 1 > 4 {
            cuts = vec![n / 3, 2 * n / 3];
        }
        let mut bounds = vec![0];
        bounds.extend(cuts);
        bounds.push(n);
        let mut segs = Vec::new();
        for (s, w) in bounds.windows(2).enumerate() {
            let chunk = &t.samples[w[0]..w[1]];
            let offset: u64 = t.samples[..w[0]]
                .iter()
                .map(|x| u64::from(x.duration.unwrap_or(0)))
                .sum();
            let ftd = FragmentTrackData::new(t.spec.track_id, t.start_decode_time + offset, chunk);
            segs.push(build_media_segment(u32::try_from(s + 1).unwrap(), &[ftd]).expect("seg"));
            if i == 0 {
                // Split this GOP into three CMAF chunks.
                let len = w[1] - w[0];
                let sub = [0, len / 3, 2 * len / 3, len];
                let mut chunks = Vec::new();
                for c in sub.windows(2) {
                    let off: u64 = t.samples[..w[0] + c[0]]
                        .iter()
                        .map(|x| u64::from(x.duration.unwrap_or(0)))
                        .sum();
                    let ftd = FragmentTrackData::new(
                        t.spec.track_id,
                        t.start_decode_time + off,
                        &t.samples[w[0] + c[0]..w[0] + c[1]],
                    );
                    chunks.push(
                        build_media_segment(u32::try_from(s + 1).unwrap(), &[ftd]).expect("chunk"),
                    );
                }
                video_chunks.push(chunks);
            }
        }
        reps.push(((i + 1).to_string(), init, segs));
    }
    Presentation {
        mpd,
        reps,
        video_chunks,
    }
}

fn config_with(
    bind: SocketAddr,
    ingest: SocketAddr,
    prebound: multimux::config::PreboundBinds,
    publishing_point: &str,
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
                publishing_point: publishing_point.to_string(),
                auth: None,
                max_object_bytes: None,
                body_timeout_secs: None,
            },
            outputs: vec![OutputKind::LlHls],
            dvr: DvrConfig::default(),
        }],
        prebound,
        ..Config::default()
    }
}

/// A running route: ingest endpoint + the LL-HLS origin it feeds.
struct Rig {
    ingest: String,
    origin: String,
    client: reqwest::Client,
    server: tokio::task::JoinHandle<multimux::Result<()>>,
}

impl Rig {
    async fn start(publishing_point: &str) -> Rig {
        let (bind_addr, bind_listener) = bind_tcp();
        let (ingest_addr, ingest_listener) = bind_tcp();
        let mut prebound = multimux::config::PreboundBinds::default();
        prebound.with_tcp(ingest_addr.to_string(), ingest_listener);
        let config = config_with(bind_addr, ingest_addr, prebound, publishing_point);
        let server = tokio::spawn(serve_with_registry_on(
            bind_listener,
            config,
            multimux::registry::SchemeRegistry::new(),
        ));
        Rig {
            ingest: format!("http://{ingest_addr}"),
            origin: format!("http://{bind_addr}"),
            client: reqwest::Client::new(),
            server,
        }
    }

    async fn start_with(publishing_point: &str, tweak: impl FnOnce(&mut Config)) -> Rig {
        let (bind_addr, bind_listener) = bind_tcp();
        let (ingest_addr, ingest_listener) = bind_tcp();
        let mut prebound = multimux::config::PreboundBinds::default();
        prebound.with_tcp(ingest_addr.to_string(), ingest_listener);
        let mut config = config_with(bind_addr, ingest_addr, prebound, publishing_point);
        tweak(&mut config);
        let server = tokio::spawn(serve_with_registry_on(
            bind_listener,
            config,
            multimux::registry::SchemeRegistry::new(),
        ));
        Rig {
            ingest: format!("http://{ingest_addr}"),
            origin: format!("http://{bind_addr}"),
            client: reqwest::Client::new(),
            server,
        }
    }

    async fn post(&self, path: &str, body: Vec<u8>) -> reqwest::StatusCode {
        // The listener task may not be accepting yet: retry connection errors
        // only, bounded, without sleeping.
        let deadline = tokio::time::Instant::now() + Duration::from_secs(10);
        loop {
            match self
                .client
                .post(format!("{}{path}", self.ingest))
                .body(body.clone())
                .send()
                .await
            {
                Ok(r) => return r.status(),
                Err(e) if e.is_connect() && tokio::time::Instant::now() < deadline => {
                    tokio::task::yield_now().await;
                }
                Err(e) => panic!("POST {path}: {e}"),
            }
        }
    }

    /// Run the whole presentation in the order §6.2.2 prescribes: the MPD, a
    /// CMAF header for EACH track, then the segments (round-robin across
    /// tracks, as parallel connections would deliver them). Returns every
    /// status in order.
    async fn push_all(&self, p: &Presentation) -> Vec<reqwest::StatusCode> {
        let mut out = vec![self.post("/live.mpd", p.mpd.clone().into_bytes()).await];
        for (id, init, _) in &p.reps {
            out.push(
                self.post(&format!("/init-stream{id}.m4s"), init.clone())
                    .await,
            );
        }
        let n = p.reps.iter().map(|(_, _, s)| s.len()).max().unwrap_or(0);
        for i in 0..n {
            for (id, _, segs) in &p.reps {
                if let Some(seg) = segs.get(i) {
                    out.push(
                        self.post(&format!("/chunk-stream{id}-{}.m4s", i + 1), seg.clone())
                            .await,
                    );
                }
            }
        }
        out
    }

    async fn get_text(&self, path: &str) -> Option<(reqwest::StatusCode, String)> {
        let r = self
            .client
            .get(format!("{}{path}", self.origin))
            .send()
            .await
            .ok()?;
        let st = r.status();
        Some((st, r.text().await.ok()?))
    }

    /// Poll the media playlist until it lists at least one `#EXTINF` segment.
    async fn playlist_with_segments(&self) -> String {
        let deadline = tokio::time::Instant::now() + Duration::from_secs(20);
        loop {
            if let Some((st, body)) = self.get_text("/cam/media.m3u8").await
                && st.is_success()
                && body.contains("#EXTINF")
            {
                return body;
            }
            assert!(
                tokio::time::Instant::now() < deadline,
                "no segment ever appeared in the media playlist"
            );
            tokio::task::yield_now().await;
        }
    }
}

impl Drop for Rig {
    fn drop(&mut self) {
        self.server.abort();
    }
}

fn segment_uris(playlist: &str) -> Vec<String> {
    playlist
        .lines()
        .filter(|l| !l.starts_with('#') && l.ends_with(".m4s"))
        .map(str::to_string)
        .collect()
}

#[tokio::test]
async fn ingested_presentation_is_served_as_valid_llhls() {
    let rig = Rig::start("/").await;
    let p = presentation();
    let sts = rig.push_all(&p).await;
    assert!(sts.iter().all(|s| s.as_u16() == 200), "{sts:?}");

    let playlist = rig.playlist_with_segments().await;
    let uris = segment_uris(&playlist);
    assert!(!uris.is_empty(), "playlist:\n{playlist}");

    // The advertised init and every advertised segment fetch and validate.
    let map = playlist
        .lines()
        .find_map(|l| l.strip_prefix("#EXT-X-MAP:URI=\""))
        .and_then(|r| r.split('"').next())
        .expect("EXT-X-MAP present")
        .to_string();
    let init = rig
        .client
        .get(format!("{}/cam/{map}", rig.origin))
        .send()
        .await
        .unwrap();
    assert_eq!(init.status(), 200);
    let init: Bytes = init.bytes().await.unwrap();
    let errs: Vec<_> = validate_init_segment(&init)
        .into_iter()
        .filter(|i| i.severity == Severity::Error)
        .collect();
    assert!(errs.is_empty(), "{errs:?}");
    for uri in &uris {
        let r = rig
            .client
            .get(format!("{}/cam/{uri}", rig.origin))
            .send()
            .await
            .unwrap();
        assert_eq!(r.status(), 200, "{uri}");
        let b: Bytes = r.bytes().await.unwrap();
        let errs: Vec<_> = validate_media_segment(&b)
            .into_iter()
            .filter(|i| i.severity == Severity::Error)
            .collect();
        assert!(errs.is_empty(), "{uri}: {errs:?}");
    }
}

#[tokio::test]
async fn status_codes_follow_section_5_3() {
    use multimux::dashif_ingest::IngestError;
    let rig = Rig::start("/live").await;
    let p = presentation();

    // Empty body = probe (§6.2.1) at the publishing point itself.
    assert_eq!(rig.post("/live/", Vec::new()).await.as_u16(), 200);
    // Outside the publishing point: 404 (§5.3.5a).
    assert_eq!(
        rig.post("/other/x.mpd", p.mpd.clone().into_bytes())
            .await
            .as_u16(),
        404
    );
    assert_eq!(rig.post("/", Vec::new()).await.as_u16(), 404);
    // A media object before any manifest has no grouping to be placed by.
    let no_manifest = IngestError::NoManifest.status().as_u16();
    assert_eq!(
        rig.post("/live/chunk-stream1-1.m4s", p.reps[0].2[0].clone())
            .await
            .as_u16(),
        no_manifest
    );
    // Malformed MPD (not XML) => 400 (§5.3.5e).
    assert_eq!(
        rig.post("/live/x.mpd", b"not xml".to_vec()).await.as_u16(),
        400
    );
    // Valid manifest, then a segment before its header => 412 (§5.3.5c).
    assert_eq!(
        rig.post("/live/x.mpd", p.mpd.clone().into_bytes())
            .await
            .as_u16(),
        200
    );
    assert_eq!(
        rig.post("/live/chunk-stream1-1.m4s", p.reps[0].2[0].clone())
            .await
            .as_u16(),
        412
    );
    // A path that matches no template => 400.
    assert_eq!(
        rig.post("/live/nonsense.bin", vec![1, 2, 3]).await.as_u16(),
        400
    );

    // GET => 405 + Allow; PUT behaves like POST.
    let r = rig
        .client
        .get(format!("{}/live/x.mpd", rig.ingest))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 405);
    assert_eq!(r.headers().get("allow").unwrap(), "POST, PUT");
    let r = rig
        .client
        .put(format!("{}/live/", rig.ingest))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 200);
}

/// A rejected request must not poison the route: after a 400 and a 412 the
/// normal flow still ingests and serves.
#[tokio::test]
async fn a_rejected_request_does_not_poison_the_route() {
    let rig = Rig::start("/").await;
    let p = presentation();
    assert_eq!(
        rig.post("/live.mpd", b"not xml".to_vec()).await.as_u16(),
        400
    );
    assert_eq!(
        rig.post("/live.mpd", p.mpd.clone().into_bytes())
            .await
            .as_u16(),
        200
    );
    assert_eq!(
        rig.post("/chunk-stream1-1.m4s", p.reps[0].2[0].clone())
            .await
            .as_u16(),
        412
    );
    // Headers for every track first (§6.2.2), then the first TWO segments of
    // each (a segment only closes when the next sync sample arrives).
    for (id, init, _) in &p.reps {
        assert_eq!(
            rig.post(&format!("/init-stream{id}.m4s"), init.clone())
                .await
                .as_u16(),
            200
        );
    }
    for n in 0..2 {
        for (id, _, segs) in &p.reps {
            assert_eq!(
                rig.post(&format!("/chunk-stream{id}-{}.m4s", n + 1), segs[n].clone())
                    .await
                    .as_u16(),
                200
            );
        }
    }
    let playlist = rig.playlist_with_segments().await;
    assert!(!segment_uris(&playlist).is_empty());
}

/// An identical re-POST (§5.3.2: resend after a dropped connection) is
/// accepted and does not duplicate the segment.
#[tokio::test]
async fn idempotent_repost_does_not_duplicate_segments() {
    let rig = Rig::start("/").await;
    let p = presentation();
    let sts = rig.push_all(&p).await;
    assert!(sts.iter().all(|s| s.as_u16() == 200), "{sts:?}");
    let before = segment_uris(&rig.playlist_with_segments().await);
    let again = rig
        .post("/chunk-stream1-1.m4s", p.reps[0].2[0].clone())
        .await;
    assert_eq!(again.as_u16(), 200);
    let after = segment_uris(&rig.playlist_with_segments().await);
    // The live playlist may legitimately have advanced (a later segment
    // closed) between the two reads, but the re-POST must not have inserted,
    // repeated or reordered anything: every name is unique, `before` is a
    // prefix of `after`.
    assert!(after.starts_with(&before), "{before:?} vs {after:?}");
    let mut unique = after.clone();
    unique.sort();
    unique.dedup();
    assert_eq!(unique.len(), after.len(), "duplicate segment in {after:?}");
}

#[tokio::test]
async fn auth_basic_challenge_wrong_and_ingest() {
    let spec = OutputAuthSpec::Basic {
        username: "u".into(),
        password: "p".into(),
    };
    let rig = Rig::start_with("/", |cfg| {
        if let InputSpec::DashifIngest { auth, .. } = &mut cfg.routes[0].input {
            *auth = Some(spec.clone());
        }
    })
    .await;
    // No credentials => 401 + WWW-Authenticate.
    let r = rig
        .client
        .post(format!("{}/live.mpd", rig.ingest))
        .body(Vec::new())
        .send()
        .await
        .unwrap();
    assert_eq!(r.status().as_u16(), 401);
    assert!(r.headers().contains_key("www-authenticate"));
    // Wrong password => 401.
    let r = rig
        .client
        .post(format!("{}/live.mpd", rig.ingest))
        .basic_auth("u", Some("wrong"))
        .body(Vec::new())
        .send()
        .await
        .unwrap();
    assert_eq!(r.status().as_u16(), 401);
    // Unauthenticated POST of a valid MPD must not leak.
    let p = presentation();
    let r = rig
        .client
        .post(format!("{}/live.mpd", rig.ingest))
        .body(p.mpd.clone().into_bytes())
        .send()
        .await
        .unwrap();
    assert_eq!(r.status().as_u16(), 401);
    // Authenticated segment before any MPD still fails (no leak).
    use multimux::dashif_ingest::IngestError;
    let no_manifest = IngestError::NoManifest.status().as_u16();
    let r = rig
        .client
        .post(format!("{}/chunk-stream1-1.m4s", rig.ingest))
        .basic_auth("u", Some("p"))
        .body(p.reps[0].2[0].clone())
        .send()
        .await
        .unwrap();
    assert_eq!(r.status().as_u16(), no_manifest);
    // Right credentials: full ingest + serve.
    let r = rig
        .client
        .post(format!("{}/live.mpd", rig.ingest))
        .basic_auth("u", Some("p"))
        .body(p.mpd.clone().into_bytes())
        .send()
        .await
        .unwrap();
    assert_eq!(r.status().as_u16(), 200);
    for (id, init, _) in &p.reps {
        let r = rig
            .client
            .post(format!("{}/init-stream{id}.m4s", rig.ingest))
            .basic_auth("u", Some("p"))
            .body(init.clone())
            .send()
            .await
            .unwrap();
        assert_eq!(r.status().as_u16(), 200);
    }
    let n = p.reps.iter().map(|(_, _, s)| s.len()).max().unwrap_or(0);
    for i in 0..n {
        for (id, _, segs) in &p.reps {
            if let Some(seg) = segs.get(i) {
                let r = rig
                    .client
                    .post(format!("{}/chunk-stream{id}-{}.m4s", rig.ingest, i + 1))
                    .basic_auth("u", Some("p"))
                    .body(seg.clone())
                    .send()
                    .await
                    .unwrap();
                assert_eq!(r.status().as_u16(), 200);
            }
        }
    }
    let playlist = rig.playlist_with_segments().await;
    assert!(playlist.contains("#EXTINF"));
}

#[tokio::test]
async fn auth_bearer_variant() {
    let spec = OutputAuthSpec::Bearer {
        token: "tok".into(),
    };
    let rig = Rig::start_with("/", |cfg| {
        if let InputSpec::DashifIngest { auth, .. } = &mut cfg.routes[0].input {
            *auth = Some(spec.clone());
        }
    })
    .await;
    let r = rig
        .client
        .post(format!("{}/live.mpd", rig.ingest))
        .bearer_auth("wrong")
        .send()
        .await
        .unwrap();
    assert_eq!(r.status().as_u16(), 401);
    let r = rig
        .client
        .post(format!("{}/", rig.ingest))
        .bearer_auth("tok")
        .body(Vec::new())
        .send()
        .await
        .unwrap();
    assert_eq!(r.status().as_u16(), 200);
}

#[tokio::test]
async fn gzip_ingest_and_serve() {
    use flate2::Compression;
    use flate2::write::GzEncoder;
    use std::io::Write;
    fn gz(data: &[u8]) -> Vec<u8> {
        let mut e = GzEncoder::new(Vec::new(), Compression::default());
        e.write_all(data).unwrap();
        e.finish().unwrap()
    }
    let rig = Rig::start("/").await;
    let p = presentation();
    // gzip'd MPD
    let r = rig
        .client
        .post(format!("{}/live.mpd", rig.ingest))
        .header("content-encoding", "gzip")
        .body(gz(p.mpd.as_bytes()))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status().as_u16(), 200);
    for (id, init, _) in &p.reps {
        let r = rig
            .client
            .post(format!("{}/init-stream{id}.m4s", rig.ingest))
            .header("content-encoding", "gzip")
            .body(gz(init))
            .send()
            .await
            .unwrap();
        assert_eq!(r.status().as_u16(), 200);
    }
    let n = p.reps.iter().map(|(_, _, s)| s.len()).max().unwrap_or(0);
    for i in 0..n {
        for (id, _, segs) in &p.reps {
            if let Some(seg) = segs.get(i) {
                let r = rig
                    .client
                    .post(format!("{}/chunk-stream{id}-{}.m4s", rig.ingest, i + 1))
                    .header("content-encoding", "gzip")
                    .body(gz(seg))
                    .send()
                    .await
                    .unwrap();
                assert_eq!(r.status().as_u16(), 200);
            }
        }
    }
    let playlist = rig.playlist_with_segments().await;
    assert!(playlist.contains("#EXTINF"));
    // Unknown content-encoding => 415.
    let r = rig
        .client
        .post(format!("{}/live.mpd", rig.ingest))
        .header("content-encoding", "br")
        .body(b"hello".to_vec())
        .send()
        .await
        .unwrap();
    assert_eq!(r.status().as_u16(), 415);
}

#[tokio::test]
async fn gzip_bomb_rejected() {
    use flate2::Compression;
    use flate2::write::GzEncoder;
    use std::io::Write;
    let rig = Rig::start_with("/", |cfg| {
        if let InputSpec::DashifIngest {
            max_object_bytes, ..
        } = &mut cfg.routes[0].input
        {
            *max_object_bytes = Some(64 * 1024);
        }
    })
    .await;
    let zeros = vec![0u8; 1024 * 1024];
    let mut e = GzEncoder::new(Vec::new(), Compression::default());
    e.write_all(&zeros).unwrap();
    let gz = e.finish().unwrap();
    let r = rig
        .client
        .post(format!("{}/live.mpd", rig.ingest))
        .header("content-encoding", "gzip")
        .body(gz)
        .send()
        .await
        .unwrap();
    assert_eq!(r.status().as_u16(), 413);
}

#[tokio::test]
async fn max_object_bytes_limit() {
    let rig = Rig::start_with("/", |cfg| {
        if let InputSpec::DashifIngest {
            max_object_bytes, ..
        } = &mut cfg.routes[0].input
        {
            *max_object_bytes = Some(1024);
        }
    })
    .await;
    let r = rig
        .client
        .post(format!("{}/live.mpd", rig.ingest))
        .body(vec![0u8; 2048])
        .send()
        .await
        .unwrap();
    assert_eq!(r.status().as_u16(), 413);
    // route not poisoned
    let r = rig
        .client
        .post(format!("{}/", rig.ingest))
        .body(Vec::new())
        .send()
        .await
        .unwrap();
    assert_eq!(r.status().as_u16(), 200);
}

#[tokio::test]
async fn body_timeout_returns_408() {
    use tokio::io::AsyncReadExt;
    use tokio::io::AsyncWriteExt;
    let rig = Rig::start_with("/", |cfg| {
        if let InputSpec::DashifIngest {
            body_timeout_secs, ..
        } = &mut cfg.routes[0].input
        {
            *body_timeout_secs = Some(0.3);
        }
    })
    .await;
    // raw tcp
    let addr = rig.ingest.trim_start_matches("http://");
    let mut stream = tokio::net::TcpStream::connect(addr).await.unwrap();
    let req = "POST /live.mpd HTTP/1.1\r\nHost: x\r\nContent-Length: 100000\r\nConnection: close\r\n\r\nabcdefghij";
    stream.write_all(req.as_bytes()).await.unwrap();
    // read response with timeout
    let mut buf = vec![0u8; 4096];
    let n = tokio::time::timeout(Duration::from_secs(5), stream.read(&mut buf))
        .await
        .expect("response arrived")
        .expect("read ok");
    let resp = String::from_utf8_lossy(&buf[..n]);
    assert!(resp.contains("408"), "resp: {resp}");
    // following normal post works
    let r = rig
        .client
        .post(format!("{}/", rig.ingest))
        .body(Vec::new())
        .send()
        .await
        .unwrap();
    assert_eq!(r.status().as_u16(), 200);
}

// ---------------------------------------------------------------------------
// P3b: streamed (HTTP chunked) segments. The requests are written by hand over
// a raw TCP stream — allowed in tests — so a request can be left OPEN between
// chunks, which no HTTP client library lets us do deterministically.
// ---------------------------------------------------------------------------

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;

/// An in-flight chunked `POST`.
struct ChunkedPost {
    stream: TcpStream,
}

impl ChunkedPost {
    async fn open(rig: &Rig, path: &str) -> ChunkedPost {
        let addr = rig.ingest.trim_start_matches("http://").to_string();
        let mut stream = TcpStream::connect(&addr).await.expect("connect");
        let head =
            format!("POST {path} HTTP/1.1\r\nHost: {addr}\r\nTransfer-Encoding: chunked\r\n\r\n");
        stream.write_all(head.as_bytes()).await.unwrap();
        ChunkedPost { stream }
    }

    async fn chunk(&mut self, bytes: &[u8]) {
        self.stream
            .write_all(format!("{:X}\r\n", bytes.len()).as_bytes())
            .await
            .unwrap();
        self.stream.write_all(bytes).await.unwrap();
        self.stream.write_all(b"\r\n").await.unwrap();
    }

    /// Terminate the body and return the response status code.
    async fn finish(mut self) -> u16 {
        self.stream.write_all(b"0\r\n\r\n").await.unwrap();
        self.status().await
    }

    /// Read the response status line (bounded; the server answers only after
    /// the body ends or fails).
    async fn status(&mut self) -> u16 {
        let mut buf = Vec::new();
        let mut tmp = [0u8; 512];
        let deadline = tokio::time::Instant::now() + Duration::from_secs(15);
        loop {
            let n = tokio::time::timeout_at(deadline, self.stream.read(&mut tmp))
                .await
                .expect("response before the deadline")
                .expect("read");
            assert!(n > 0, "connection closed before a response: {buf:?}");
            buf.extend_from_slice(&tmp[..n]);
            if let Some(end) = buf.windows(2).position(|w| w == b"\r\n") {
                let line = String::from_utf8_lossy(&buf[..end]).to_string();
                return line
                    .split_whitespace()
                    .nth(1)
                    .and_then(|c| c.parse().ok())
                    .unwrap_or_else(|| panic!("bad status line {line:?}"));
            }
        }
    }
}

/// MPD + both headers, no media yet.
async fn manifest_and_headers(rig: &Rig, p: &Presentation) {
    assert_eq!(
        rig.post("/live.mpd", p.mpd.clone().into_bytes())
            .await
            .as_u16(),
        200
    );
    for (id, init, _) in &p.reps {
        assert_eq!(
            rig.post(&format!("/init-stream{id}.m4s"), init.clone())
                .await
                .as_u16(),
            200
        );
    }
}

/// Poll the media playlist until `pred` holds for its text (bounded).
async fn playlist_until(rig: &Rig, what: &str, pred: impl Fn(&str) -> bool) -> String {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(20);
    loop {
        if let Some((st, body)) = rig.get_text("/cam/media.m3u8").await
            && st.is_success()
            && pred(&body)
        {
            return body;
        }
        assert!(tokio::time::Instant::now() < deadline, "timed out: {what}");
        tokio::task::yield_now().await;
    }
}

fn part_uris(playlist: &str) -> Vec<String> {
    playlist
        .lines()
        .filter(|l| l.starts_with("#EXT-X-PART:"))
        .filter_map(|l| l.split("URI=\"").nth(1))
        .filter_map(|r| r.split('"').next())
        .map(str::to_string)
        .collect()
}

/// THE POINT of the increment: media from the first CMAF chunk is published
/// (and served as an LL-HLS part) while the segment's request is still open.
#[tokio::test]
async fn first_chunk_is_published_while_the_request_is_still_open() {
    let rig = Rig::start("/").await;
    let p = presentation();
    manifest_and_headers(&rig, &p).await;

    let mut req = ChunkedPost::open(&rig, "/chunk-stream1-1.m4s").await;
    req.chunk(&p.video_chunks[0][0]).await;
    // NOT finished. A part must appear in the playlist anyway.
    let playlist = playlist_until(&rig, "a part from the first chunk", |b| {
        !part_uris(b).is_empty()
    })
    .await;
    let uri = &part_uris(&playlist)[0];
    let r = rig
        .client
        .get(format!("{}/cam/{uri}", rig.origin))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 200, "{uri}");
    let bytes: Bytes = r.bytes().await.unwrap();
    let errs: Vec<_> = validate_media_segment(&bytes)
        .into_iter()
        .filter(|i| i.severity == Severity::Error)
        .collect();
    assert!(errs.is_empty(), "{uri}: {errs:?}");

    // Finish the segment; it completes normally.
    for c in &p.video_chunks[0][1..] {
        req.chunk(c).await;
    }
    assert_eq!(req.finish().await, 200);
}

/// A segment streamed as chunks and the same bytes sent with a
/// Content-Length produce the same published media.
#[tokio::test]
async fn chunked_and_content_length_ingest_publish_the_same_segments() {
    async fn run(chunked: bool) -> Vec<Bytes> {
        let rig = Rig::start("/").await;
        let p = presentation();
        manifest_and_headers(&rig, &p).await;
        for (i, chunks) in p.video_chunks.iter().enumerate() {
            let path = format!("/chunk-stream1-{}.m4s", i + 1);
            if chunked {
                let mut req = ChunkedPost::open(&rig, &path).await;
                for c in chunks {
                    req.chunk(c).await;
                }
                assert_eq!(req.finish().await, 200);
            } else {
                assert_eq!(rig.post(&path, chunks.concat()).await.as_u16(), 200);
            }
            // Audio keeps pace (whole objects) so both tracks are present.
            let seg = &p.reps[1].2[i];
            assert_eq!(
                rig.post(&format!("/chunk-stream2-{}.m4s", i + 1), seg.clone())
                    .await
                    .as_u16(),
                200
            );
        }
        let playlist = rig.playlist_with_segments().await;
        let mut out = Vec::new();
        for uri in segment_uris(&playlist) {
            out.push(
                rig.client
                    .get(format!("{}/cam/{uri}", rig.origin))
                    .send()
                    .await
                    .unwrap()
                    .bytes()
                    .await
                    .unwrap(),
            );
        }
        out
    }
    let a = run(true).await;
    let b = run(false).await;
    assert!(!a.is_empty());
    let n = a.len().min(b.len());
    assert!(n >= 1);
    assert_eq!(a[..n], b[..n], "published segments differ");
}

#[tokio::test]
async fn a_partial_trailing_box_is_400_and_the_route_survives() {
    let rig = Rig::start("/").await;
    let p = presentation();
    manifest_and_headers(&rig, &p).await;
    let mut req = ChunkedPost::open(&rig, "/chunk-stream1-1.m4s").await;
    req.chunk(&p.video_chunks[0][0]).await;
    let half = &p.video_chunks[0][1][..p.video_chunks[0][1].len() / 2];
    req.chunk(half).await;
    assert_eq!(req.finish().await, 400);
    // The route still ingests (next segment number, whole object).
    assert_eq!(
        rig.post("/chunk-stream1-2.m4s", p.reps[0].2[1].clone())
            .await
            .as_u16(),
        200
    );
}

/// A client that disconnects mid-body leaves no stuck handle: the same
/// representation can immediately start another segment.
#[tokio::test]
async fn a_client_abort_mid_body_frees_the_representation() {
    let rig = Rig::start("/").await;
    let p = presentation();
    manifest_and_headers(&rig, &p).await;
    {
        let mut req = ChunkedPost::open(&rig, "/chunk-stream1-1.m4s").await;
        req.chunk(&p.video_chunks[0][0]).await;
        // Drop without finishing: the TCP connection closes mid-body.
    }
    // The abort is processed asynchronously: retry a bounded number of times
    // while the answer is the in-progress conflict.
    let deadline = tokio::time::Instant::now() + Duration::from_secs(10);
    loop {
        let st = rig
            .post("/chunk-stream1-2.m4s", p.reps[0].2[1].clone())
            .await
            .as_u16();
        if st == 200 {
            break;
        }
        assert_eq!(st, 409, "only the in-progress conflict may delay this");
        assert!(tokio::time::Instant::now() < deadline, "handle never freed");
        tokio::task::yield_now().await;
    }
}

/// Two simultaneous segment requests for the SAME representation: the second
/// gets 409 (a track is one ordered stream).
#[tokio::test]
async fn a_second_concurrent_segment_for_the_same_rep_is_409() {
    let rig = Rig::start("/").await;
    let p = presentation();
    manifest_and_headers(&rig, &p).await;
    let mut first = ChunkedPost::open(&rig, "/chunk-stream1-1.m4s").await;
    first.chunk(&p.video_chunks[0][0]).await;
    // Give the pump a chance to have processed the first Begin: a different
    // representation's object round-trips through the same pump.
    assert_eq!(
        rig.post("/chunk-stream2-1.m4s", p.reps[1].2[0].clone())
            .await
            .as_u16(),
        200
    );
    assert_eq!(
        rig.post("/chunk-stream1-2.m4s", p.reps[0].2[1].clone())
            .await
            .as_u16(),
        409
    );
    for c in &p.video_chunks[0][1..] {
        first.chunk(c).await;
    }
    assert_eq!(first.finish().await, 200);
}

/// The per-object body deadline applies across a streamed request: stalling
/// after the first chunk gets 408, the first chunk stays published, and the
/// route keeps working.
#[tokio::test]
async fn a_stalled_chunked_body_times_out_with_408() {
    let rig = Rig::start_with("/", |cfg| {
        if let InputSpec::DashifIngest {
            body_timeout_secs, ..
        } = &mut cfg.routes[0].input
        {
            *body_timeout_secs = Some(0.3);
        }
    })
    .await;
    let p = presentation();
    manifest_and_headers(&rig, &p).await;
    let mut req = ChunkedPost::open(&rig, "/chunk-stream1-1.m4s").await;
    req.chunk(&p.video_chunks[0][0]).await;
    // ...and nothing more. The server must answer 408 on its own.
    assert_eq!(req.status().await, 408);
    // The route keeps working.
    let deadline = tokio::time::Instant::now() + Duration::from_secs(10);
    loop {
        let st = rig
            .post("/chunk-stream1-2.m4s", p.reps[0].2[1].clone())
            .await
            .as_u16();
        if st == 200 {
            break;
        }
        assert_eq!(st, 409, "only the in-progress conflict may delay this");
        assert!(tokio::time::Instant::now() < deadline, "handle never freed");
        tokio::task::yield_now().await;
    }
}
