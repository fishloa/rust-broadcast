//! A real wire check against mediamtx fed by ffmpeg: `fixtures/ts/h264_aac.ts` published over RTSP/TCP
//! and played through a `Session` over a real TCP socket.
//!
//! Both tools are found on `PATH`. When either is missing the test SKIPS LOUDLY, and with
//! `REQUIRE_MEDIAMTX_E2E=1` a missing tool is a hard failure, never a skip. Every file the tools
//! write lives in a per-run temp directory that is removed afterwards.

use std::fs;
use std::io::{ErrorKind, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::thread;
use std::time::{Duration, Instant};

use broadcast_client::{Action, BufferEvent, Config, Event, Session};
use transmux::CodecConfig;

const REQUIRE_ENV: &str = "REQUIRE_MEDIAMTX_E2E";
const MEDIAMTX: &str = "mediamtx";
const FFMPEG: &str = "ffmpeg";
const FIXTURE: &str = "ts/h264_aac.ts";
const STREAM_PATH: &str = "live";
const READY_DEADLINE: Duration = Duration::from_secs(10);
const RUN: Duration = Duration::from_secs(15);
const READ_TIMEOUT: Duration = Duration::from_millis(5);
const READ_CHUNK: usize = 64 * 1024;
const MIN_SAMPLES: usize = 50;
const MICROS_PER_NANO_DIVISOR: u128 = 1_000;

struct Tool {
    child: Child,
}

impl Drop for Tool {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

struct Scratch(PathBuf);

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn on_path(name: &str) -> Option<PathBuf> {
    std::env::split_paths(&std::env::var_os("PATH")?)
        .map(|d| d.join(name))
        .find(|p| p.is_file())
}

fn bail(reason: &str) {
    assert!(
        !std::env::var(REQUIRE_ENV).is_ok_and(|v| v == "1"),
        "{REQUIRE_ENV}=1 and {reason}: a missing tool is a failure here, not a skip"
    );
    eprintln!("SKIP e2e_mediamtx_rtsp: {reason} (set {REQUIRE_ENV}=1 to make this a failure)");
}

fn free_port() -> u16 {
    TcpListener::bind("127.0.0.1:0")
        .and_then(|l| l.local_addr())
        .expect("a free port")
        .port()
}

fn write_config(dir: &Path, rtsp_port: u16) -> PathBuf {
    let config = format!(
        "logLevel: error\n\
         api: no\nmetrics: no\npprof: no\nplayback: no\nrtmp: no\nhls: no\nwebrtc: no\nsrt: no\n\
         rtspTransports: [tcp]\nrtspEncryption: \"no\"\n\
         rtspAddress: 127.0.0.1:{rtsp_port}\n\
         paths:\n  all_others:\n"
    );
    let path = dir.join("mediamtx.yml");
    fs::write(&path, config).expect("write mediamtx config");
    path
}

fn wait_for_port(port: u16) {
    let deadline = Instant::now() + READY_DEADLINE;
    while TcpStream::connect(("127.0.0.1", port)).is_err() {
        assert!(
            Instant::now() < deadline,
            "mediamtx never listened on {port}"
        );
        thread::sleep(Duration::from_millis(50));
    }
}

fn spawn_quiet(cmd: &mut Command, dir: &Path) -> Tool {
    let child = cmd
        .current_dir(dir)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .expect("spawn tool");
    Tool { child }
}

fn micros(start: Instant) -> u64 {
    u64::try_from(start.elapsed().as_nanos() / MICROS_PER_NANO_DIVISOR).expect("elapsed fits u64")
}

#[test]
fn rtsp_session_plays_a_real_mediamtx_stream_with_dims_and_buffer_start() {
    let (Some(mediamtx), Some(ffmpeg)) = (on_path(MEDIAMTX), on_path(FFMPEG)) else {
        bail("mediamtx and ffmpeg must both be on PATH");
        return;
    };
    let dir =
        Scratch(std::env::temp_dir().join(format!("e2e-mediamtx-rtsp-{}", std::process::id())));
    fs::create_dir_all(&dir.0).expect("scratch dir");
    let port = free_port();
    let config = write_config(&dir.0, port);
    let _server = spawn_quiet(Command::new(mediamtx).arg(&config), &dir.0);
    wait_for_port(port);

    let url = format!("rtsp://127.0.0.1:{port}/{STREAM_PATH}");
    let fixture = test_fixtures::path(FIXTURE);
    let _publisher = spawn_quiet(
        Command::new(ffmpeg)
            .args(["-v", "error", "-re", "-stream_loop", "-1", "-i"])
            .arg(&fixture)
            .args([
                "-c:v",
                "copy",
                "-c:a",
                "aac",
                "-f",
                "rtsp",
                "-rtsp_transport",
                "tcp",
            ])
            .arg(&url),
        &dir.0,
    );

    let started = Instant::now();
    let mut session = Session::open(&url, Config::default(), 0).expect("open");
    let mut stream: Option<(u32, TcpStream)> = None;
    let mut events = Vec::new();
    let mut buf = vec![0u8; READ_CHUNK];
    while started.elapsed() < RUN {
        let now = micros(started);
        while let Some(action) = session.poll(now) {
            match action {
                Action::TcpConnect {
                    conn, host, port, ..
                } => match TcpStream::connect((host.as_str(), port)) {
                    Ok(s) => {
                        s.set_read_timeout(Some(READ_TIMEOUT)).expect("timeout");
                        s.set_nodelay(true).expect("nodelay");
                        stream = Some((conn, s));
                        session.on_connected(conn);
                    }
                    Err(_) => session.on_closed(conn, None, now),
                },
                Action::Send { conn, bytes } => {
                    if let Some((c, s)) = stream.as_mut()
                        && *c == conn
                    {
                        s.write_all(&bytes).expect("write");
                    }
                }
                Action::Close { conn } if stream.as_ref().is_some_and(|(c, _)| *c == conn) => {
                    stream = None;
                }
                _ => {}
            }
        }
        if let Some((conn, s)) = stream.as_mut() {
            match s.read(&mut buf) {
                Ok(0) => {
                    let conn = *conn;
                    stream = None;
                    session.on_closed(conn, None, micros(started));
                }
                Ok(n) => session.on_data(*conn, &buf[..n], micros(started)),
                Err(e) if matches!(e.kind(), ErrorKind::WouldBlock | ErrorKind::TimedOut) => {}
                Err(e) => panic!("read: {e}"),
            }
        } else {
            thread::sleep(READ_TIMEOUT);
        }
        let now = micros(started);
        session.on_playback_position(now, None);
        while let Some(e) = session.next_event() {
            events.push(e);
        }
    }

    let errors: Vec<_> = events
        .iter()
        .filter(|e| matches!(e, Event::Error { .. }))
        .collect();
    assert!(errors.is_empty(), "session errors: {errors:?}");

    let dims = events.iter().rev().find_map(|e| match e {
        Event::Tracks(specs) => specs.iter().find_map(|s| match &s.config {
            CodecConfig::Avc { width, height, .. } => Some((*width, *height)),
            _ => None,
        }),
        _ => None,
    });
    let (w, h) = dims.expect("a Tracks event with an H.264 track");
    assert!(w > 0 && h > 0, "Avc dimensions {w}x{h}");

    assert!(
        events
            .iter()
            .any(|e| matches!(e, Event::Buffer(BufferEvent::Start { .. }))),
        "no BufferEvent::Start in {} events",
        events.len()
    );

    let samples: Vec<_> = events
        .iter()
        .filter_map(|e| match e {
            Event::Sample { track, sample, .. } => Some((*track, sample)),
            _ => None,
        })
        .collect();
    assert!(
        samples.len() >= MIN_SAMPLES,
        "only {} samples",
        samples.len()
    );
    let zero = samples
        .iter()
        .filter(|(_, s)| s.duration.unwrap_or(0) == 0)
        .count();
    assert_eq!(
        zero,
        0,
        "{zero} of {} samples have zero duration",
        samples.len()
    );
    eprintln!(
        "e2e: {w}x{h}, {} samples, 0 with zero duration, Start emitted",
        samples.len()
    );
}
