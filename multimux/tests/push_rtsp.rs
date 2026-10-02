//! RTSP push output — real-server oracle (audit run-07 C3 / run-09 C1, #1025).
//!
//! Before this fix, `multimux::push::RtspTransport` could not push to any
//! real RTSP server: every request went to a hard-coded `rtsp://localhost/
//! push` regardless of the configured URL, no response status was ever
//! checked, the SDP had no `c=` line and mismatched the single SETUP/
//! interleaved-channel it actually used, and the interleaved payload was
//! raw MPEG-2 TS bytes with no RTP header (RFC 2326 §10.12 requires
//! interleaved data to be RTP/RTCP). Separately, `rtsp-runtime`'s auth
//! retry (audit run-09 C1) rebuilt a 401 retry with an empty body, so even
//! after fixing this transport, an authenticated ANNOUNCE never carried its
//! SDP.
//!
//! This test exercises both fixes together, end-to-end, over a real,
//! independent RTSP server (`mediamtx`, MIT-licensed) — not a mock, not an
//! inspection of our own client's byte construction: `mediamtx` decodes the
//! MPEG-2 TS this transport ships (its own doc calls this out: it has a
//! dedicated `rtsp.MPEGTSDemuxer` for exactly this SDP shape, RFC 2250 §2's
//! static-payload-type-33 "MP2T/90000"), and a real, independent reader
//! (`ffprobe`) reads the stream back from `mediamtx` and recovers real
//! H.264 + AAC streams. Both a Digest and a Basic challenge are exercised,
//! so the auth-retry fix (body + `Content-Type` survival) is proven against
//! Digest specifically (the case audit run-09 C1 called out), not just an
//! unauthenticated push.
//!
//! Both external tools are optional: this file skips itself — loudly, via
//! `eprintln!` naming the reason — when either is missing (macOS dev-host
//! and CI without them stay green; `brew install mediamtx` is MIT-licensed
//! and safe to add locally). Run with `--nocapture` to see the skip line or
//! the pre-fix failure this test is designed to catch.

use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::time::Duration;

use multimux::push::{PushTransport, RtspTransport, RtspTransportConfig};
use transmux::{CodecConfig, TsDemux};

/// HANG GUARD (workspace precedent, issue #826): every blocking wait below
/// is bounded, so a broken connect/push path fails the test rather than
/// hanging the suite.
const GUARD: Duration = Duration::from_secs(20);

fn fixture_path() -> PathBuf {
    PathBuf::from(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../fixtures/ts/h264_aac.ts"
    ))
}

/// Workspace-convention scratch dir (mirrors `golden_gate.rs`'s own
/// `scratch_dir` — no `tempfile` dependency needed).
fn scratch_dir(name: &str) -> PathBuf {
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../target/push-rtsp-tmp")
        .join(name);
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create push_rtsp scratch dir");
    dir
}

// ── External-tool availability gates ────────────────────────────────────────

fn mediamtx_available() -> bool {
    Command::new("mediamtx")
        .arg("--version")
        .output()
        .is_ok_and(|o| o.status.success())
}

fn ffprobe_available() -> bool {
    Command::new("ffprobe")
        .arg("-version")
        .output()
        .is_ok_and(|o| o.status.success())
}

/// Skip this test cleanly, but LOUDLY: a silently-skipped oracle reads as a
/// pass, which is worse than no oracle at all (workspace precedent, issue
/// #870's `mediastreamvalidator_oracle`).
macro_rules! skip_unless_tools_available {
    () => {
        if !mediamtx_available() || !ffprobe_available() {
            eprintln!(
                "SKIP push_rtsp: `mediamtx` and/or `ffprobe` not on PATH \
                 (real-server RTSP push oracle, audit run-07 C3 / run-09 C1, \
                 #1025). `brew install mediamtx` + `brew install ffmpeg` to \
                 get the genuine check. This test is a no-op result on this \
                 host, not real coverage."
            );
            return;
        }
    };
}

fn free_port() -> u16 {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("bind ephemeral port");
    listener.local_addr().expect("local addr").port()
}

/// A running `mediamtx` child process configured with one publish+read user,
/// on ephemeral ports, RTSP-TCP-only (every other server it can run is
/// disabled so it never contends for a well-known port with anything else
/// on the host).
struct MediaMtx {
    child: std::process::Child,
    rtsp_port: u16,
    /// The control API (`/v3/paths/get/{path}`), used to learn when a reader
    /// is attached instead of sleeping and hoping.
    api_port: u16,
}

impl MediaMtx {
    /// `auth_method` is `"basic"` or `"digest"` (`rtspAuthMethods`), also
    /// used to name this instance's scratch dir so the two tests' config
    /// files never collide.
    fn start(auth_method: &str, user: &str, pass: &str) -> Self {
        let rtsp_port = free_port();
        let api_port = free_port();
        let config_dir = scratch_dir(auth_method);
        let config_path = config_dir.join("mediamtx.yml");
        let config = format!(
            "logLevel: info\n\
             logDestinations: [stdout]\n\
             rtsp: true\n\
             rtspAddress: :{rtsp_port}\n\
             rtspTransports: [tcp]\n\
             rtspEncryption: \"no\"\n\
             rtspAuthMethods: [{auth_method}]\n\
             rtmp: no\n\
             hls: no\n\
             webrtc: no\n\
             srt: no\n\
             api: yes\n\
             apiAddress: 127.0.0.1:{api_port}\n\
             moq: no\n\
             authMethod: internal\n\
             authInternalUsers:\n\
             \x20\x20- user: {user}\n\
             \x20\x20\x20\x20pass: {pass}\n\
             \x20\x20\x20\x20ips: []\n\
             \x20\x20\x20\x20permissions:\n\
             \x20\x20\x20\x20\x20\x20- action: publish\n\
             \x20\x20\x20\x20\x20\x20\x20\x20path:\n\
             \x20\x20\x20\x20\x20\x20- action: read\n\
             \x20\x20\x20\x20\x20\x20\x20\x20path:\n\
             \x20\x20- user: any\n\
             \x20\x20\x20\x20pass:\n\
             \x20\x20\x20\x20ips: [127.0.0.1, \"::1\"]\n\
             \x20\x20\x20\x20permissions:\n\
             \x20\x20\x20\x20\x20\x20- action: api\n\
             paths:\n\
             \x20\x20all_others:\n"
        );
        std::fs::write(&config_path, config).expect("write mediamtx config");

        let child = Command::new("mediamtx")
            .arg(&config_path)
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .expect("spawn mediamtx");

        // Wait for the RTSP listener to actually accept connections — a
        // just-spawned process needs to parse its config and bind before
        // it does, and a bare `connect()` immediately after `spawn()` races
        // that (observed: "Connection refused" 100% of the time).
        let deadline = std::time::Instant::now() + Duration::from_secs(10);
        loop {
            if std::net::TcpStream::connect(("127.0.0.1", rtsp_port)).is_ok() {
                break;
            }
            assert!(
                std::time::Instant::now() < deadline,
                "mediamtx never opened its RTSP port {rtsp_port}"
            );
            std::thread::sleep(Duration::from_millis(50));
        }

        Self {
            child,
            rtsp_port,
            api_port,
        }
    }

    /// Waits (bounded) until `mediamtx` reports at least one reader attached
    /// to `path` — i.e. the reader's RTSP `PLAY` completed — so the publisher
    /// sends only once there is somebody to receive from the first frame.
    async fn wait_for_reader(&self, path: &str, guard: Duration) {
        let url = format!("http://127.0.0.1:{}/v3/paths/get/{path}", self.api_port);
        let client = reqwest::Client::new();
        let deadline = tokio::time::Instant::now() + guard;
        loop {
            if let Ok(resp) = client.get(&url).send().await
                && resp.status().is_success()
                && let Ok(body) = resp.json::<serde_json::Value>().await
                && body["readers"].as_array().is_some_and(|r| !r.is_empty())
            {
                return;
            }
            assert!(
                tokio::time::Instant::now() < deadline,
                "mediamtx never reported a reader on {path}"
            );
            tokio::time::sleep(Duration::from_millis(25)).await;
        }
    }

    fn url(&self, path: &str) -> String {
        format!("rtsp://127.0.0.1:{}/{path}", self.rtsp_port)
    }

    fn url_with_creds(&self, user: &str, pass: &str, path: &str) -> String {
        format!("rtsp://{user}:{pass}@127.0.0.1:{}/{path}", self.rtsp_port)
    }
}

impl Drop for MediaMtx {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

/// Runs `ffprobe` against `url`, capping the read at 3 s (`-read_intervals
/// %+3`; RTSP is a live source with no natural EOF once a publisher is
/// connected) so a regression that makes the stream unreadable fails fast
/// rather than hanging. Returns `(success, stdout, stderr)`.
fn ffprobe_read(url: &str) -> (bool, String, String) {
    let out = Command::new("ffprobe")
        .args([
            "-v",
            "error",
            "-rtsp_transport",
            "tcp",
            "-read_intervals",
            "%+3",
            "-show_entries",
            "stream=codec_name,codec_type",
            "-of",
            "json",
        ])
        .arg(url)
        .output()
        .expect("spawn ffprobe");
    (
        out.status.success(),
        String::from_utf8_lossy(&out.stdout).into_owned(),
        String::from_utf8_lossy(&out.stderr).into_owned(),
    )
}

/// Pushes the real `h264_aac.ts` fixture to `push_url` via [`RtspTransport`],
/// then reads it back from `read_url` with `ffprobe`, asserting both codecs
/// come back. Runs the push and the read concurrently: `mediamtx` only
/// serves a path once a publisher has completed RECORD, and (being a live
/// source, not a file) only from the moment a reader joins — so `ffprobe`
/// must already be connected and reading before `send_media` ships the
/// (effectively instantaneous, no real-time pacing) fixture bytes.
async fn push_and_read_back(
    server: &MediaMtx,
    path: &str,
    cfg: RtspTransportConfig,
    push_url: String,
    read_url: String,
) {
    let ts = std::fs::read(fixture_path()).expect("h264_aac.ts fixture must exist");
    let media = TsDemux::new().demux(&ts).expect("demux h264_aac.ts");
    assert!(
        media
            .tracks
            .iter()
            .any(|t| matches!(t.spec.config, CodecConfig::Avc { .. })),
        "fixture must carry AVC video"
    );
    assert!(
        media
            .tracks
            .iter()
            .any(|t| matches!(t.spec.config, CodecConfig::Aac { .. })),
        "fixture must carry AAC audio"
    );

    let mut transport = tokio::time::timeout(GUARD, RtspTransport::connect(&push_url, &cfg))
        .await
        .expect("connect must not hang")
        .unwrap_or_else(|e| panic!("connect (OPTIONS) failed: {e}"));

    let tracks: Vec<transmux::TrackSpec> = media.tracks.iter().map(|t| t.spec.clone()).collect();
    tokio::time::timeout(GUARD, transport.setup(&tracks))
        .await
        .expect("setup must not hang")
        .unwrap_or_else(|e| panic!("setup (ANNOUNCE/SETUP/RECORD) failed: {e}"));

    // `mediamtx` now has an active publisher for this path. Start the
    // reader concurrently with the push so it is mid-`PLAY` by the time
    // `send_media` ships the fixture.
    let reader_url = read_url.clone();
    let reader = tokio::task::spawn_blocking(move || ffprobe_read(&reader_url));
    server.wait_for_reader(path, GUARD).await;

    tokio::time::timeout(GUARD, transport.send_media(&media))
        .await
        .expect("send_media must not hang")
        .unwrap_or_else(|e| panic!("send_media failed: {e}"));

    let (ok, stdout, stderr) = tokio::time::timeout(GUARD, reader)
        .await
        .expect("ffprobe task must not hang")
        .expect("ffprobe task panicked");
    transport.close();

    assert!(ok, "ffprobe rejected {read_url}: stderr={stderr}");
    assert!(
        stdout.contains("h264") && stdout.contains("aac"),
        "ffprobe did not recover both H.264 and AAC from mediamtx: stdout={stdout} stderr={stderr}"
    );
}

/// **THE BITE**: before rtsp-runtime's C1 fix, the Digest-authenticated
/// ANNOUNCE retry carried `Content-Length: 0` and no SDP — `mediamtx`
/// answered with a 4xx (no SDP to parse) and RECORD was never reached, so
/// this push failed outright. Before the matching multimux fix, the
/// `AuthRetry` bytes were never even written to the socket, so the *first*
/// 401 already ended the exchange.
#[tokio::test]
async fn rtsp_push_survives_digest_auth_against_mediamtx() {
    skip_unless_tools_available!();
    let server = MediaMtx::start("digest", "pushuser", "pushpass123");
    const PATH: &str = "digest-test";
    let push_url = server.url(PATH);
    let read_url = server.url_with_creds("pushuser", "pushpass123", PATH);
    let cfg = RtspTransportConfig {
        credentials: Some(("pushuser".to_string(), "pushpass123".to_string())),
    };
    push_and_read_back(&server, PATH, cfg, push_url, read_url).await;
}

/// Same oracle, Basic auth (RFC 2326 §14 / RFC 7617) — the other scheme
/// `rtsp-runtime`'s `Authenticator` supports.
#[tokio::test]
async fn rtsp_push_survives_basic_auth_against_mediamtx() {
    skip_unless_tools_available!();
    let server = MediaMtx::start("basic", "pushuser", "pushpass123");
    const PATH: &str = "basic-test";
    let push_url = server.url(PATH);
    let read_url = server.url_with_creds("pushuser", "pushpass123", PATH);
    let cfg = RtspTransportConfig {
        credentials: Some(("pushuser".to_string(), "pushpass123".to_string())),
    };
    push_and_read_back(&server, PATH, cfg, push_url, read_url).await;
}
