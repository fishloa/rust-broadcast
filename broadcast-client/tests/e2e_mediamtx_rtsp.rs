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

use broadcast_client::{Action, AvSync, BufferEvent, Config, Event, Session, Status};
use transmux::CodecConfig;

const REQUIRE_ENV: &str = "REQUIRE_MEDIAMTX_E2E";
const MEDIAMTX: &str = "mediamtx";
const FFMPEG: &str = "ffmpeg";
const FIXTURE: &str = "ts/h264_aac.ts";
const STREAM_PATH: &str = "live";
const READY_DEADLINE: Duration = Duration::from_secs(10);
static SERVER_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());
const PUBLISHER_WARMUP: Duration = Duration::from_secs(4);
const SOURCE_LOOPS: &str = "12";
const SOURCE_SECONDS: &str = "30";
const RUN: Duration = Duration::from_secs(15);
const READ_TIMEOUT: Duration = Duration::from_millis(5);
const READ_CHUNK: usize = 64 * 1024;
const MIN_SAMPLES: usize = 50;
const MICROS_PER_NANO_DIVISOR: u128 = 1_000;
const COMMON_TIMELINE_BOUND_SECONDS: f64 = 0.2;

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
         api: no\nmetrics: no\npprof: no\nplayback: no\nrtmp: no\nhls: no\nwebrtc: no\nsrt: no\nmoq: no\n\
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
    run_case("fixture-1s-gop", None);
}

#[test]
fn rtsp_session_starts_on_a_two_second_gop_source() {
    run_case(
        "reencoded-2s-gop",
        Some(&[
            "-c:v",
            "libx264",
            "-preset",
            "ultrafast",
            "-g",
            "50",
            "-keyint_min",
            "50",
            "-sc_threshold",
            "0",
            "-r",
            "25",
        ]),
    );
}

fn run_case(label: &str, reencode_video: Option<&[&str]>) {
    let _serial = SERVER_LOCK
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
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
    let source = dir.0.join("source.mkv");
    let mut encode = Command::new(&ffmpeg);
    encode
        .args(["-v", "error", "-y", "-stream_loop", SOURCE_LOOPS, "-i"])
        .arg(&fixture)
        .args(["-t", SOURCE_SECONDS]);
    match reencode_video {
        Some(args) => encode.args(args),
        None => encode.args(["-c:v", "copy"]),
    };
    encode
        .args(["-c:a", "aac", "-b:a", "128k", "-f", "matroska"])
        .arg(&source);
    let status = encode.status().expect("run ffmpeg encode");
    if !status.success() {
        bail("ffmpeg could not build the test source (libx264 missing?)");
        return;
    }
    let mut publisher = Command::new(ffmpeg);
    publisher
        .args(["-v", "error", "-re", "-i"])
        .arg(&source)
        .args(["-c", "copy", "-f", "rtsp", "-rtsp_transport", "tcp"])
        .arg(&url);
    let _publisher = spawn_quiet(&mut publisher, &dir.0);
    thread::sleep(PUBLISHER_WARMUP);

    let started = Instant::now();
    let mut first_live_us: Option<u64> = None;
    let mut first_video_sample_us: Option<u64> = None;
    let mut first_video_sample_pts: Option<i64> = None;
    let mut first_video_keyframe_us: Option<u64> = None;
    let mut first_video_keyframe_pts: Option<i64> = None;
    let mut first_audio_sample_us: Option<u64> = None;
    let mut first_audio_sample_pts: Option<i64> = None;
    let mut sync_states: Vec<(u64, AvSync)> = Vec::new();
    let mut max_pts_seconds: std::collections::BTreeMap<bool, f64> =
        std::collections::BTreeMap::new();
    let mut start_us: Option<u64> = None;
    let mut start_at_27mhz: Option<u64> = None;
    let mut media_end_at_start: Option<std::collections::BTreeMap<u32, u64>> = None;
    let mut track_timescale: std::collections::BTreeMap<u32, u32> =
        std::collections::BTreeMap::new();
    let mut track_is_video: std::collections::BTreeMap<u32, bool> =
        std::collections::BTreeMap::new();
    let mut last_media_end_27: std::collections::BTreeMap<u32, u64> =
        std::collections::BTreeMap::new();
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
            match &e {
                Event::Status(crate::Status::Live) if first_live_us.is_none() => {
                    first_live_us = Some(micros(started));
                }
                Event::Tracks(specs) => {
                    for s in specs {
                        track_timescale.insert(s.track_id.value(), s.timescale);
                        let is_video = matches!(s.config, transmux::CodecConfig::Avc { .. });
                        track_is_video.insert(s.track_id.value(), is_video);
                    }
                }
                Event::AvSync(state) => sync_states.push((micros(started), *state)),
                Event::Sample { track, sample, .. } => {
                    let tid = track.value();
                    let is_video = track_is_video.get(&tid).copied().unwrap_or(false);
                    let now = micros(started);
                    if is_video {
                        if first_video_sample_us.is_none() {
                            first_video_sample_us = Some(now);
                            first_video_sample_pts = sample.pts;
                        }
                        if sample.flags.is_sync && first_video_keyframe_us.is_none() {
                            first_video_keyframe_us = Some(now);
                            first_video_keyframe_pts = sample.pts;
                        }
                    } else if first_audio_sample_us.is_none() {
                        first_audio_sample_us = Some(now);
                        first_audio_sample_pts = sample.pts;
                    }
                    if let Some(pts) = sample.pts {
                        let ts = track_timescale.get(&tid).copied().unwrap_or(1).max(1);
                        let seconds = pts as f64 / f64::from(ts);
                        let slot = max_pts_seconds.entry(is_video).or_insert(seconds);
                        *slot = slot.max(seconds);
                        let dur = i64::from(sample.duration.unwrap_or(0));
                        let end27 = (i128::from(pts + dur) * i128::from(27_000_000i64)
                            / i128::from(ts)) as u64;
                        last_media_end_27.insert(tid, end27);
                    }
                }
                Event::Buffer(crate::BufferEvent::Start { at_27mhz }) if start_us.is_none() => {
                    start_us = Some(micros(started));
                    start_at_27mhz = Some(*at_27mhz);
                    media_end_at_start = Some(last_media_end_27.clone());
                }
                _ => {}
            }
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
    let zero_indices: Vec<usize> = samples
        .iter()
        .enumerate()
        .filter(|(_, (_, s))| s.duration.unwrap_or(0) == 0)
        .map(|(i, _)| i)
        .collect();
    if !zero_indices.is_empty() {
        eprintln!(
            "ZERO-DURATION SAMPLES: {} of {}",
            zero_indices.len(),
            samples.len()
        );
        for &i in &zero_indices {
            let (tr, s) = &samples[i];
            let prev = i
                .checked_sub(1)
                .map(|j| {
                    let (ptr, ps) = &samples[j];
                    format!(
                        "prev track={} pts={} dts={} dur={:?}",
                        ptr,
                        ps.pts.unwrap_or(0),
                        ps.dts.unwrap_or(0),
                        ps.duration
                    )
                })
                .unwrap_or_else(|| "prev=none".to_string());
            let next = samples
                .get(i + 1)
                .map(|(ntr, ns)| {
                    format!(
                        "next track={} pts={} dts={} dur={:?}",
                        ntr,
                        ns.pts.unwrap_or(0),
                        ns.dts.unwrap_or(0),
                        ns.duration
                    )
                })
                .unwrap_or_else(|| "next=none".to_string());
            eprintln!(
                "  idx={} track={} pts={} dts={} dur={:?} | {} | {}",
                i,
                tr,
                s.pts.unwrap_or(0),
                s.dts.unwrap_or(0),
                s.duration,
                prev,
                next
            );
        }
    }
    let zero = zero_indices.len();
    assert_eq!(
        zero,
        0,
        "{zero} of {} samples have zero duration",
        samples.len()
    );
    let states: Vec<AvSync> = sync_states.iter().map(|(_, s)| *s).collect();
    assert_eq!(
        states.last(),
        Some(&AvSync::SenderReport),
        "mediamtx sends Sender Reports, the sync state must reach SenderReport: {sync_states:?}"
    );
    let first_gap = match (first_audio_sample_pts, first_video_sample_pts) {
        (Some(a), Some(v)) => {
            let scale = |is_video: bool| {
                f64::from(
                    track_is_video
                        .iter()
                        .find(|(_, video)| **video == is_video)
                        .and_then(|(t, _)| track_timescale.get(t))
                        .copied()
                        .expect("track timescale"),
                )
            };
            a as f64 / scale(false) - v as f64 / scale(true)
        }
        _ => panic!("no first audio and video sample"),
    };
    let last_gap = max_pts_seconds[&false] - max_pts_seconds[&true];
    eprintln!(
        "AVSYNC[{label}] states={sync_states:?} first_audio_minus_first_video_s={first_gap:.4} last_audio_minus_last_video_s={last_gap:.4}"
    );
    assert!(
        first_gap.abs() < COMMON_TIMELINE_BOUND_SECONDS,
        "first audio is {first_gap} s from first video on the common timeline"
    );
    assert!(
        last_gap.abs() < COMMON_TIMELINE_BOUND_SECONDS,
        "after {RUN:?} the latest audio is {last_gap} s from the latest video"
    );
    let configured_start_lead_us = Config::default().buffer.policy.initial_us;
    let origin_27 = first_video_keyframe_pts.map(|pts| {
        let ts = track_timescale
            .values()
            .next()
            .copied()
            .unwrap_or(90_000)
            .max(1);
        (i128::from(pts) * i128::from(27_000_000i64) / i128::from(ts)) as u64
    });
    eprintln!(
        "TIMELINE[{}] (us relative to open): live={} first_video_sample={} (pts={:?}) first_video_keyframe={} (pts={:?}) first_audio_sample={} (pts={:?}) buffer_start={} (at_27mhz={:?}) configured_start_lead_us={} media_end_at_start={:?} origin_27mhz={:?} samples={}",
        label,
        first_live_us.unwrap_or(0),
        first_video_sample_us.unwrap_or(0),
        first_video_sample_pts,
        first_video_keyframe_us.unwrap_or(0),
        first_video_keyframe_pts,
        first_audio_sample_us.unwrap_or(0),
        first_audio_sample_pts,
        start_us.unwrap_or(0),
        start_at_27mhz,
        configured_start_lead_us,
        media_end_at_start,
        origin_27,
        samples.len(),
    );
    #[cfg(feature = "test-seams")]
    print_lead_series(label, &session);
    eprintln!(
        "e2e: {w}x{h}, {} samples, 0 with zero duration, Start emitted",
        samples.len()
    );
}

#[cfg(feature = "test-seams")]
fn print_lead_series(label: &str, session: &Session) {
    use broadcast_client::BufferInput;
    use std::collections::BTreeMap;
    let mut end: BTreeMap<u32, u64> = BTreeMap::new();
    let mut origin: Option<u64> = None;
    let mut next_print = 0u64;
    for input in session.buffer_input_log() {
        match input {
            BufferInput::Register(t) => eprintln!("LEAD[{label}] register track {t}"),
            BufferInput::Unregister(t) => eprintln!("LEAD[{label}] unregister track {t}"),
            BufferInput::TrackMedia {
                now_us,
                track,
                end: e,
            } => {
                origin = origin.or(Some(*e));
                end.insert(*track, *e);
                if *now_us >= next_print {
                    next_print = now_us + 250_000;
                    let o = origin.unwrap_or(0);
                    let per: Vec<String> = end
                        .iter()
                        .map(|(t, e)| format!("t{t}_end={}ms", (*e as i128 - o as i128) / 27_000))
                        .collect();
                    let lead = end
                        .values()
                        .min()
                        .map_or(0, |m| (*m as i128 - o as i128) / 27_000);
                    eprintln!(
                        "LEAD[{label}] at={}ms {} min_lead={}ms",
                        now_us / 1000,
                        per.join(" "),
                        lead
                    );
                }
            }
            _ => {}
        }
    }
}
