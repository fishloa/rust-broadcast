//! HLS `METHOD=AES-128` through the real `run_hls_pull` drive loop (issue
//! #1477): classic-TS segments from the committed `ts-hls` fixture are
//! encrypted in the TEST with `transmux::sample_aes::aes128_encrypt_segment`
//! under keys and IVs the test picks itself, served by real loopback HTTP
//! servers, and the samples that reach the route's `Trunk` are compared with
//! the samples the same route produces from the plaintext segments.

use std::collections::HashMap;
use std::num::NonZeroUsize;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use axum::Router;
use axum::body::Body;
use axum::extract::State;
use axum::http::{HeaderMap, StatusCode, Uri, header};
use axum::response::{IntoResponse, Response};
use broadcast_auth::Credentials;
use broadcast_common::Timestamp;
use futures_util::StreamExt;
use hls_runtime::client::Error as ClientError;
use media_plane::ingress::HandshakePolicy;
use media_plane::trunk::{SampleCursorItem, TrunkConfig};
use multimux::MultimuxError;
use multimux::RouteHandle;
use multimux::source::hls_pull::{HlsPullRoute, run_hls_pull};
use transmux::sample_aes::aes128_encrypt_segment;

#[path = "support/listener.rs"]
mod listener;

const TS_PACKET_LEN: usize = 188;
const LEADING_PACKETS_DROPPED: usize = 3;
const KEY_A: [u8; 16] = [0x11; 16];
const KEY_B: [u8; 16] = [0x22; 16];
const KEY_WRONG: [u8; 16] = [0x33; 16];
const USER: &str = "keyuser";
const PASS: &str = "keypass";
const USER_PASS_BASIC: &str = "Basic a2V5dXNlcjprZXlwYXNz";
const KEY_PATH: &str = "/key.bin";
const KEY_B_PATH: &str = "/keyB.bin";
const TIMED_RING: usize = 4096;
const RING: usize = 16;
const SEGMENTS: usize = 2;
const OVERSIZED_KEY_LEN: usize = 5000;
const SHORT_KEY_LEN: usize = 15;
const RUN_CEILING: Duration = Duration::from_secs(150);

type Fingerprint = Vec<(String, Vec<u8>, Option<i64>, Option<i64>)>;

fn seg(index: u8) -> Vec<u8> {
    let name = format!("../hls-runtime/tests/fixtures/ts-hls/index{index}.ts");
    test_fixtures::read(&name)[TS_PACKET_LEN * LEADING_PACKETS_DROPPED..].to_vec()
}

fn hex(b: &[u8; 16]) -> String {
    b.iter().map(|x| format!("{x:02X}")).collect()
}

fn iv_seq(base: u8) -> [u8; 16] {
    core::array::from_fn(|i| base.wrapping_add(i as u8))
}

fn key_tag(uri: &str, iv: &[u8; 16]) -> String {
    format!("#EXT-X-KEY:METHOD=AES-128,URI=\"{uri}\",IV=0x{}", hex(iv))
}

fn playlist(entries: &[(String, &str)]) -> String {
    let mut s = String::from(
        "#EXTM3U\n#EXT-X-VERSION:5\n#EXT-X-TARGETDURATION:2\n#EXT-X-MEDIA-SEQUENCE:0\n",
    );
    for (tags, uri) in entries {
        if !tags.is_empty() {
            s.push_str(tags);
            s.push('\n');
        }
        s.push_str("#EXTINF:2.0,\n");
        s.push_str(uri);
        s.push('\n');
    }
    s.push_str("#EXT-X-ENDLIST\n");
    s
}

#[derive(Clone)]
struct Resource {
    body: Vec<u8>,
    require_auth: Option<&'static str>,
    fail_first: Arc<AtomicUsize>,
    after_segments_sent: usize,
}

impl Resource {
    fn bytes(body: Vec<u8>) -> Self {
        Resource {
            body,
            require_auth: None,
            fail_first: Arc::new(AtomicUsize::new(0)),
            after_segments_sent: 0,
        }
    }

    fn after_segments_sent(mut self, n: usize) -> Self {
        self.after_segments_sent = n;
        self
    }

    fn requiring_auth(mut self) -> Self {
        self.require_auth = Some(USER_PASS_BASIC);
        self
    }

    fn failing_first(self, n: usize) -> Self {
        self.fail_first.store(n, Ordering::SeqCst);
        self
    }
}

struct Logged {
    path: String,
    authorization: Option<String>,
}

struct Shared {
    resources: HashMap<String, Resource>,
    segments_sent: AtomicUsize,
    segments_sent_changed: tokio::sync::Notify,
    log: Mutex<Vec<Logged>>,
}

struct Origin {
    base: String,
    shared: Arc<Shared>,
    task: tokio::task::JoinHandle<()>,
}

impl Drop for Origin {
    fn drop(&mut self) {
        self.task.abort();
    }
}

impl Origin {
    fn requests(&self, path: &str) -> Vec<Option<String>> {
        self.shared
            .log
            .lock()
            .expect("log lock")
            .iter()
            .filter(|l| l.path == path)
            .map(|l| l.authorization.clone())
            .collect()
    }

    fn total_requests(&self) -> usize {
        self.shared.log.lock().expect("log lock").len()
    }
}

async fn handler(State(shared): State<Arc<Shared>>, uri: Uri, headers: HeaderMap) -> Response {
    let authorization = headers
        .get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .map(str::to_string);
    shared.log.lock().expect("log lock").push(Logged {
        path: uri.path().to_string(),
        authorization: authorization.clone(),
    });
    let Some(res) = shared.resources.get(uri.path()) else {
        return StatusCode::NOT_FOUND.into_response();
    };
    if let Some(expected) = res.require_auth
        && authorization.as_deref() != Some(expected)
    {
        return (
            StatusCode::UNAUTHORIZED,
            [(header::WWW_AUTHENTICATE, "Basic realm=\"keys\"")],
        )
            .into_response();
    }
    loop {
        let changed = shared.segments_sent_changed.notified();
        if shared.segments_sent.load(Ordering::SeqCst) >= res.after_segments_sent {
            break;
        }
        changed.await;
    }
    let failing = res
        .fail_first
        .fetch_update(Ordering::SeqCst, Ordering::SeqCst, |n| n.checked_sub(1))
        .is_ok();
    if failing {
        return StatusCode::INTERNAL_SERVER_ERROR.into_response();
    }
    if !uri.path().ends_with(".ts") {
        return res.body.clone().into_response();
    }
    let body = res.body.clone();
    let done = Arc::clone(&shared);
    let chunks = futures_util::stream::once(async move {
        Ok::<_, std::convert::Infallible>(bytes::Bytes::from(body))
    })
    .chain(
        futures_util::stream::once(async move {
            done.segments_sent.fetch_add(1, Ordering::SeqCst);
            done.segments_sent_changed.notify_waiters();
        })
        .filter_map(|()| async { None }),
    );
    Body::from_stream(chunks).into_response()
}

fn serve(resources: Vec<(&str, Resource)>) -> Origin {
    let (addr, tcp) = listener::bind_tcp();
    let shared = Arc::new(Shared {
        resources: resources
            .into_iter()
            .map(|(p, r)| (p.to_string(), r))
            .collect(),
        log: Mutex::new(Vec::new()),
        segments_sent: AtomicUsize::new(0),
        segments_sent_changed: tokio::sync::Notify::new(),
    });
    let app = Router::new()
        .fallback(handler)
        .with_state(Arc::clone(&shared));
    let task = tokio::spawn(async move {
        axum::serve(tcp, app).await.expect("axum server");
    });
    Origin {
        base: format!("http://{addr}"),
        shared,
        task,
    }
}

fn closed_port_base() -> String {
    let (addr, tcp) = listener::bind_tcp();
    drop(tcp);
    format!("http://{addr}")
}

fn two_segment_files(
    k0: &[u8; 16],
    iv0: &[u8; 16],
    k1: &[u8; 16],
    iv1: &[u8; 16],
) -> Vec<(&'static str, Resource)> {
    vec![
        (
            "/index0.ts",
            Resource::bytes(aes128_encrypt_segment(k0, iv0, &seg(0))),
        ),
        (
            "/index1.ts",
            Resource::bytes(aes128_encrypt_segment(k1, iv1, &seg(1))),
        ),
    ]
}

fn plain_files() -> Vec<(&'static str, Resource)> {
    vec![
        ("/index0.ts", Resource::bytes(seg(0))),
        ("/index1.ts", Resource::bytes(seg(1))),
    ]
}

fn nz(n: usize) -> NonZeroUsize {
    NonZeroUsize::new(n).expect("non-zero")
}

async fn ingest(
    playlist_url: &str,
    auth: Option<Credentials>,
) -> (Result<(), MultimuxError>, Fingerprint) {
    let route = HlsPullRoute::new("keys", playlist_url).with_auth(auth);
    let route_handle = Arc::new(RouteHandle::new(4.0, 500, 4));
    let config = TrunkConfig::new(nz(TIMED_RING), nz(RING), nz(RING), nz(RING), nz(RING));
    let handshake = HandshakePolicy::establish_by(Timestamp::from_nanos(u64::MAX));
    let result = tokio::time::timeout(
        RUN_CEILING,
        run_hls_pull(&route, config, handshake, &route_handle),
    )
    .await
    .expect("run_hls_pull must terminate, not stall");
    if result.is_err() {
        return (result, Vec::new());
    }
    let trunk = tokio::time::timeout(Duration::from_secs(5), route_handle.await_first_trunk())
        .await
        .expect("a finished ingest must have published a trunk");
    let mut cursor = trunk.subscribe_from_backlog();
    let mut samples: Fingerprint = Vec::new();
    while let Some(item) = cursor.poll() {
        if let SampleCursorItem::Timed { track_id, sample } = item {
            samples.push((
                format!("{track_id:?}"),
                sample.data.to_vec(),
                sample.pts,
                sample.dts,
            ));
        }
    }
    samples.sort();
    (result, samples)
}

async fn plaintext_baseline() -> Fingerprint {
    let origin = serve({
        let mut r = plain_files();
        r.push((
            "/index.m3u8",
            Resource::bytes(
                playlist(&[(String::new(), "index0.ts"), (String::new(), "index1.ts")])
                    .into_bytes(),
            ),
        ));
        r
    });
    let (result, samples) = ingest(&format!("{}/index.m3u8", origin.base), None).await;
    result.expect("plaintext ingest succeeds");
    assert!(!samples.is_empty(), "baseline must produce samples");
    samples
}

fn encrypted_origin(key_resource: Resource) -> Origin {
    let iv0 = iv_seq(0x90);
    let iv1 = iv_seq(0xB0);
    let text = playlist(&[
        (key_tag("key.bin", &iv0), "index0.ts"),
        (key_tag("key.bin", &iv1), "index1.ts"),
    ]);
    let mut resources = two_segment_files(&KEY_A, &iv0, &KEY_A, &iv1);
    resources.push(("/index.m3u8", Resource::bytes(text.into_bytes())));
    resources.push((KEY_PATH, key_resource));
    serve(resources)
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn encrypted_playlist_ingests_same_samples_as_plaintext() {
    let baseline = plaintext_baseline().await;
    let origin = encrypted_origin(Resource::bytes(KEY_A.to_vec()));
    let (result, samples) = ingest(&format!("{}/index.m3u8", origin.base), None).await;
    result.expect("encrypted ingest succeeds");
    assert_eq!(samples.len(), baseline.len(), "sample count");
    assert!(
        samples == baseline,
        "decrypted samples differ from plaintext"
    );
    assert_eq!(origin.requests(KEY_PATH).len(), 1);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn key_fetched_once_per_uri() {
    let iv0 = iv_seq(0x10);
    let iv1 = iv_seq(0x20);
    let text = playlist(&[
        (key_tag("key.bin", &iv0), "index0.ts"),
        (key_tag("keyB.bin", &iv1), "index1.ts"),
    ]);
    let mut resources = two_segment_files(&KEY_A, &iv0, &KEY_B, &iv1);
    resources.push(("/index.m3u8", Resource::bytes(text.into_bytes())));
    resources.push((KEY_PATH, Resource::bytes(KEY_A.to_vec())));
    resources.push((KEY_B_PATH, Resource::bytes(KEY_B.to_vec())));
    let origin = serve(resources);
    let (result, samples) = ingest(&format!("{}/index.m3u8", origin.base), None).await;
    result.expect("two-key ingest succeeds");
    assert!(!samples.is_empty());
    assert_eq!(origin.requests(KEY_PATH).len(), 1, "key A once");
    assert_eq!(origin.requests(KEY_B_PATH).len(), 1, "key B once");

    let shared = encrypted_origin(Resource::bytes(KEY_A.to_vec()));
    let (result, _) = ingest(&format!("{}/index.m3u8", shared.base), None).await;
    result.expect("shared-key ingest succeeds");
    assert_eq!(
        shared.requests(KEY_PATH).len(),
        1,
        "two segments naming one key URI fetch it once"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn key_request_uses_configured_basic_auth() {
    let baseline = plaintext_baseline().await;
    let origin = encrypted_origin(Resource::bytes(KEY_A.to_vec()).requiring_auth());
    let creds = Credentials::Basic {
        username: USER.to_string(),
        password: PASS.to_string(),
    };
    let (result, samples) = ingest(&format!("{}/index.m3u8", origin.base), Some(creds)).await;
    result.expect("authenticated key fetch succeeds");
    assert!(
        samples == baseline,
        "decrypted samples differ from plaintext"
    );
    let key_requests = origin.requests(KEY_PATH);
    assert_eq!(key_requests.len(), 1);
    assert_eq!(key_requests[0].as_deref(), Some(USER_PASS_BASIC));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn key_without_configured_credentials_fails_with_auth_error() {
    let origin = encrypted_origin(Resource::bytes(KEY_A.to_vec()).requiring_auth());
    let (result, samples) = ingest(&format!("{}/index.m3u8", origin.base), None).await;
    assert!(
        matches!(result, Err(MultimuxError::Auth { .. })),
        "{result:?}"
    );
    assert!(samples.is_empty());
    assert_eq!(origin.requests(KEY_PATH), vec![None]);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn key_on_other_origin_gets_no_credentials() {
    let key_origin = serve(vec![(
        KEY_PATH,
        Resource::bytes(KEY_A.to_vec()).requiring_auth(),
    )]);
    let iv0 = iv_seq(0x90);
    let iv1 = iv_seq(0xB0);
    let key_url = format!("{}{KEY_PATH}", key_origin.base);
    let text = playlist(&[
        (key_tag(&key_url, &iv0), "index0.ts"),
        (key_tag(&key_url, &iv1), "index1.ts"),
    ]);
    let mut resources = two_segment_files(&KEY_A, &iv0, &KEY_A, &iv1);
    resources.push(("/index.m3u8", Resource::bytes(text.into_bytes())));
    let media_origin = serve(resources);
    let creds = Credentials::Basic {
        username: USER.to_string(),
        password: PASS.to_string(),
    };
    let (result, _) = ingest(&format!("{}/index.m3u8", media_origin.base), Some(creds)).await;
    assert!(
        matches!(result, Err(MultimuxError::Auth { .. })),
        "{result:?}"
    );
    let playlist_requests = media_origin.requests("/index.m3u8");
    assert_eq!(
        playlist_requests[0].as_deref(),
        Some(USER_PASS_BASIC),
        "credentials are active on the configured origin"
    );
    let key_requests = key_origin.requests(KEY_PATH);
    assert!(!key_requests.is_empty(), "the key was requested");
    assert!(
        key_requests.iter().all(Option::is_none),
        "credentials leaked to another origin: {key_requests:?}"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn key_http_error_retries_then_succeeds() {
    let baseline = plaintext_baseline().await;
    let origin = encrypted_origin(Resource::bytes(KEY_A.to_vec()).failing_first(2));
    let (result, samples) = ingest(&format!("{}/index.m3u8", origin.base), None).await;
    result.expect("a transient key failure is retried");
    assert!(
        samples == baseline,
        "decrypted samples differ from plaintext"
    );
    assert_eq!(origin.requests(KEY_PATH).len(), 3);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn wrong_length_key_is_failure_not_stall() {
    let short = encrypted_origin(Resource::bytes(vec![0x44; SHORT_KEY_LEN]));
    let (result, samples) = ingest(&format!("{}/index.m3u8", short.base), None).await;
    match result {
        Err(MultimuxError::LlHls(ClientError::BadKeyLength { len, .. })) => {
            assert_eq!(len, SHORT_KEY_LEN);
        }
        other => panic!("expected BadKeyLength, got {other:?}"),
    }
    assert!(samples.is_empty());
    assert!(short.requests(KEY_PATH).len() >= 2, "a bad key is retried");
    assert_eq!(short.requests("/index0.ts").len(), 1);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn oversized_key_body_is_failure() {
    let origin = encrypted_origin(Resource::bytes(vec![0x55; OVERSIZED_KEY_LEN]));
    let (result, samples) = ingest(&format!("{}/index.m3u8", origin.base), None).await;
    match result {
        Err(MultimuxError::Connect { reason }) => {
            assert!(reason.contains("exceeds"), "{reason}");
        }
        other => panic!("expected the body cap to reject the key, got {other:?}"),
    }
    assert!(samples.is_empty());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn wrong_key_surfaces_typed_decrypt_error_not_stall() {
    let origin =
        encrypted_origin(Resource::bytes(KEY_WRONG.to_vec()).after_segments_sent(SEGMENTS));
    let (result, samples) = ingest(&format!("{}/index.m3u8", origin.base), None).await;
    assert!(
        matches!(
            result,
            Err(MultimuxError::LlHls(ClientError::DecryptFailed { .. }))
        ),
        "{result:?}"
    );
    assert!(samples.is_empty(), "no plaintext may be produced");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn sample_aes_surfaces_typed_unsupported_encryption() {
    for method in ["SAMPLE-AES", "SAMPLE-AES-CTR"] {
        let tag = format!("#EXT-X-KEY:METHOD={method},URI=\"key.bin\"");
        let text = playlist(&[(tag, "index0.ts")]);
        let mut resources = plain_files();
        resources.push(("/index.m3u8", Resource::bytes(text.into_bytes())));
        resources.push((KEY_PATH, Resource::bytes(KEY_A.to_vec())));
        let origin = serve(resources);
        let (result, samples) = ingest(&format!("{}/index.m3u8", origin.base), None).await;
        assert!(
            matches!(
                result,
                Err(MultimuxError::LlHls(
                    ClientError::UnsupportedEncryption { .. }
                ))
            ),
            "{method}: {result:?}"
        );
        assert!(samples.is_empty(), "{method}: no passthrough");
        assert!(origin.requests("/index0.ts").is_empty(), "{method}");
        assert!(origin.requests(KEY_PATH).is_empty(), "{method}");
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn plaintext_playlist_never_requests_key() {
    let mut resources = plain_files();
    resources.push((
        "/index.m3u8",
        Resource::bytes(
            playlist(&[(String::new(), "index0.ts"), (String::new(), "index1.ts")]).into_bytes(),
        ),
    ));
    resources.push((KEY_PATH, Resource::bytes(KEY_A.to_vec())));
    let origin = serve(resources);
    let (result, samples) = ingest(&format!("{}/index.m3u8", origin.base), None).await;
    result.expect("plaintext ingest succeeds");
    assert!(!samples.is_empty());
    assert!(origin.requests(KEY_PATH).is_empty());
    assert_eq!(origin.total_requests(), 3);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn ingest_does_not_hang_when_key_server_is_down() {
    let iv0 = iv_seq(0x90);
    let iv1 = iv_seq(0xB0);
    let key_url = format!("{}{KEY_PATH}", closed_port_base());
    let text = playlist(&[
        (key_tag(&key_url, &iv0), "index0.ts"),
        (key_tag(&key_url, &iv1), "index1.ts"),
    ]);
    let mut resources = two_segment_files(&KEY_A, &iv0, &KEY_A, &iv1);
    resources.push(("/index.m3u8", Resource::bytes(text.into_bytes())));
    let origin = serve(resources);
    let (result, samples) = ingest(&format!("{}/index.m3u8", origin.base), None).await;
    match result {
        Err(MultimuxError::Connect { reason }) => {
            assert!(reason.contains(KEY_PATH), "{reason}");
        }
        other => panic!("expected a key connect failure, got {other:?}"),
    }
    assert!(samples.is_empty());
}
