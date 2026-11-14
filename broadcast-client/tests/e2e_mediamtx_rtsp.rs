//! A real wire check against mediamtx fed by ffmpeg: `fixtures/ts/h264-aac.ts` published over RTSP/TCP
//! and played through a `Session` over a real TCP socket.
//!
//! Both tools are found on `PATH`. When either is missing the test SKIPS LOUDLY, and with
//! `REQUIRE_MEDIAMTX_E2E=1` a missing tool is a hard failure, never a skip. Every file the tools
//! write lives in a per-run temp directory that is removed afterwards.

use std::fs;
use std::io::{ErrorKind, Read, Write};
use std::net::TcpStream;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::thread;
use std::time::{Duration, Instant};
use test_fixtures::process::free_tcp_port;

use broadcast_common::stage::Timestamp;

/// The test's microsecond clock as the session's `now`.
fn ts(us: u64) -> Timestamp {
    Timestamp::from_micros(us)
}
use broadcast_client::{Action, AvSync, BufferEvent, Config, Event, Session, Status};
use broadcast_common::Unpackage;
use transmux::{CodecConfig, TsDemux};

#[path = "support/truth.rs"]
mod truth;
use truth::Truth;

const REQUIRE_ENV: &str = "REQUIRE_MEDIAMTX_E2E";
const MEDIAMTX: &str = "mediamtx";
const FFMPEG: &str = "ffmpeg";
const FIXTURE: &str = "ts/h264-aac.ts";
const STREAM_PATH: &str = "live";
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
const AAC_FRAME_TICKS: i64 = 1024;
/// AAC-LC rate of `FIXTURE`, per `fixtures/ts/PROVENANCE.md` (`h264-aac.ts`: AAC-LC 44100 Hz 1ch).
const FIXTURE_AUDIO_RATE_HZ: f64 = 44_100.0;
/// Test-only fault injection: seconds added to every audio sample's emitted time before the A/V
/// checks, simulating an audio-vs-video step. Unset means no step.
const INJECT_AUDIO_STEP_ENV: &str = "E2E_INJECT_AUDIO_STEP_S";
const SR_MIN_LEN: usize = 20;
const RTCP_PT_SR: u8 = 200;
const NTP_FRACTION: f64 = 4_294_967_296.0;
/// Fraction of one AAC frame the emitted audio-vs-video timeline may sit from the Sender Report
/// mapping. Both tracks are compared against the same wallclock reference, so an honest run
/// measures 0.0000 s; the fault class guarded (#1568, a frame or more of A/V step) therefore must
/// fail, and a sub-frame bound keeps a one-frame step (about 23 ms) outside it.
const SR_FAITHFUL_AAC_FRAMES: f64 = 0.5;
const SR_FAITHFUL_SECONDS: f64 =
    SR_FAITHFUL_AAC_FRAMES * AAC_FRAME_TICKS as f64 / FIXTURE_AUDIO_RATE_HZ;
/// AAC frames the server's own audio and video Sender Report origins may move against each other
/// before the faithfulness check is skipped as unjudgeable. The server's reports jitter with CPU
/// load (idle runs span about 0.004..0.010 s), which is independent of what the client emits.
const SR_REFERENCE_DRIFT_AAC_FRAMES: f64 = 2.0;
const SR_REFERENCE_DRIFT_SECONDS: f64 =
    SR_REFERENCE_DRIFT_AAC_FRAMES * AAC_FRAME_TICKS as f64 / FIXTURE_AUDIO_RATE_HZ;
/// A deviation the unit test needs to sit clearly outside `SR_FAITHFUL_SECONDS`.
const BEYOND_BOUND_FACTOR: f64 = 1.5;
/// A deviation the unit test needs to sit clearly inside `SR_FAITHFUL_SECONDS`.
const WITHIN_BOUND_DIVISOR: f64 = 2.0;
/// mediamtx sends a Sender Report about once a second; the A/V checks interpolate the
/// audio channel's origin at each video report's time, so one report per channel would
/// leave the drift a single point (vacuous) and zero would report 0 s of drift.
const MIN_SENDER_REPORTS_PER_TRACK: usize = 2;
/// A relative move of the video channel's Sender Report origin the drift test applies.
const SR_RELATIVE_MOVE_SECONDS: f64 = 0.05;
/// The spacing the drift test's synthetic Sender Reports use.
const SR_REPORT_SPACING_SECONDS: f64 = 1.0;
/// The report time after which the drift test's video origin carries its move.
const SR_RELATIVE_MOVE_AFTER_SECONDS: f64 = 101.0;

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

const RTP_TS_OFFSET: usize = 4;
const RTP_HEADER_MIN: usize = 12;
const INTERLEAVED_MAGIC: u8 = b'$';
const INTERLEAVED_HEADER: usize = 4;
const CHANNELS_PER_TRACK: u8 = 2;
const NAL_TYPE_MASK: u8 = 0x1f;
const NAL_IDR: u8 = 5;
const NAL_FU_A: u8 = 28;
const FU_START_BIT: u8 = 0x80;

fn starts_idr(payload: &[u8]) -> bool {
    match payload.first().map(|b| b & NAL_TYPE_MASK) {
        Some(NAL_IDR) => true,
        Some(NAL_FU_A) => payload
            .get(1)
            .is_some_and(|fu| fu & FU_START_BIT != 0 && fu & NAL_TYPE_MASK == NAL_IDR),
        _ => false,
    }
}

fn rel_to(ts: u32, base: u32) -> i64 {
    i64::from(ts.wrapping_sub(base) as i32)
}

#[derive(Default)]
struct WireArrivals {
    pending: Vec<u8>,
    first_arrival: std::collections::BTreeMap<u8, std::collections::BTreeMap<i64, u64>>,
    first_ts: std::collections::BTreeMap<u8, u32>,
    idr_starts: std::collections::BTreeMap<u8, Vec<i64>>,
    order: std::collections::BTreeMap<u8, Vec<i64>>,
    sender_reports: std::collections::BTreeMap<u8, Vec<(f64, u32)>>,
}

impl WireArrivals {
    fn feed(&mut self, bytes: &[u8], now_us: u64) {
        self.pending.extend_from_slice(bytes);
        loop {
            if self.pending.first() == Some(&INTERLEAVED_MAGIC) {
                if self.pending.len() < INTERLEAVED_HEADER {
                    return;
                }
                let len = usize::from(u16::from_be_bytes([self.pending[2], self.pending[3]]));
                if self.pending.len() < INTERLEAVED_HEADER + len {
                    return;
                }
                let channel = self.pending[1];
                let frame: Vec<u8> = self
                    .pending
                    .drain(..INTERLEAVED_HEADER + len)
                    .skip(INTERLEAVED_HEADER)
                    .collect();
                if !channel.is_multiple_of(CHANNELS_PER_TRACK)
                    && frame.len() >= SR_MIN_LEN
                    && frame[1] == RTCP_PT_SR
                {
                    let word = |at: usize| {
                        u32::from_be_bytes([frame[at], frame[at + 1], frame[at + 2], frame[at + 3]])
                    };
                    let ntp = f64::from(word(8)) + f64::from(word(12)) / NTP_FRACTION;
                    self.sender_reports
                        .entry(channel - 1)
                        .or_default()
                        .push((ntp, word(16)));
                }
                if channel.is_multiple_of(CHANNELS_PER_TRACK) && frame.len() >= RTP_HEADER_MIN {
                    let ts = u32::from_be_bytes([
                        frame[RTP_TS_OFFSET],
                        frame[RTP_TS_OFFSET + 1],
                        frame[RTP_TS_OFFSET + 2],
                        frame[RTP_TS_OFFSET + 3],
                    ]);
                    let base = *self.first_ts.entry(channel).or_insert(ts);
                    {
                        let o = self.order.entry(channel).or_default();
                        let rel = rel_to(ts, base);
                        if o.last() != Some(&rel) {
                            o.push(rel);
                        }
                    }
                    if starts_idr(&frame[RTP_HEADER_MIN..]) {
                        let idr = self.idr_starts.entry(channel).or_default();
                        let rel = rel_to(ts, base);
                        if idr.last() != Some(&rel) {
                            idr.push(rel);
                        }
                    }
                    self.first_arrival
                        .entry(channel)
                        .or_default()
                        .entry(rel_to(ts, base))
                        .or_insert(now_us);
                }
            } else {
                let Some(end) = self.pending.windows(4).position(|w| w == b"\r\n\r\n") else {
                    return;
                };
                let head = String::from_utf8_lossy(&self.pending[..end]).to_ascii_lowercase();
                let body = head
                    .lines()
                    .find_map(|l| l.strip_prefix("content-length:"))
                    .and_then(|v| v.trim().parse::<usize>().ok())
                    .unwrap_or(0);
                if self.pending.len() < end + 4 + body {
                    return;
                }
                self.pending.drain(..end + 4 + body);
            }
        }
    }

    fn has_exact(&self, channel: u8, rel_ticks: i64) -> bool {
        self.first_arrival
            .get(&channel)
            .is_some_and(|m| m.contains_key(&rel_ticks))
    }

    fn arrival_of(&self, channel: u8, rel_ticks: i64) -> Option<u64> {
        self.first_arrival
            .get(&channel)?
            .range(..=rel_ticks)
            .next_back()
            .map(|(_, &t)| t)
    }
}

fn is_synced(state: AvSync) -> bool {
    matches!(state, AvSync::SenderReport | AvSync::RtpInfo)
}

fn sr_faithfulness_violation(state: Option<AvSync>, deviation: f64) -> Option<f64> {
    (state == Some(AvSync::SenderReport) && deviation.abs() >= SR_FAITHFUL_SECONDS)
        .then_some(deviation)
}

fn expected_first_gap(state: Option<AvSync>, video_sent_early_s: f64) -> f64 {
    if state == Some(AvSync::RtpInfo) {
        video_sent_early_s
    } else {
        0.0
    }
}

fn sr_wall_at_origin(wire: &WireArrivals, channel: u8, scale: f64) -> f64 {
    let first_ts = wire.first_ts[&channel];
    let mut origins: Vec<f64> = wire.sender_reports[&channel]
        .iter()
        .map(|&(ntp, sr_ts)| ntp - f64::from(sr_ts.wrapping_sub(first_ts) as i32) / scale)
        .collect();
    origins.sort_by(f64::total_cmp);
    origins[origins.len() / 2]
}

fn sr_origins(wire: &WireArrivals, channel: u8, scale: f64) -> Vec<(f64, f64)> {
    let first_ts = wire.first_ts[&channel];
    let mut origins: Vec<(f64, f64)> = wire.sender_reports[&channel]
        .iter()
        .map(|&(ntp, sr_ts)| {
            (
                ntp,
                ntp - f64::from(sr_ts.wrapping_sub(first_ts) as i32) / scale,
            )
        })
        .collect();
    origins.sort_by(|a, b| a.0.total_cmp(&b.0));
    origins
}

fn origin_at(origins: &[(f64, f64)], ntp: f64) -> f64 {
    match origins {
        [] => f64::NAN,
        [(_, only)] => *only,
        _ => {
            let after = origins
                .partition_point(|&(t, _)| t < ntp)
                .clamp(1, origins.len() - 1);
            let (t0, o0) = origins[after - 1];
            let (t1, o1) = origins[after];
            o0 + (o1 - o0) * (ntp - t0) / (t1 - t0)
        }
    }
}

fn sr_av_reference_drift(audio: &[(f64, f64)], video: &[(f64, f64)]) -> f64 {
    let (lo, hi) = audio
        .iter()
        .map(|&(ntp, origin)| origin - origin_at(video, ntp))
        .fold((f64::INFINITY, f64::NEG_INFINITY), |(lo, hi), d| {
            (lo.min(d), hi.max(d))
        });
    (hi - lo).max(0.0)
}

fn injected_audio_step_seconds() -> f64 {
    std::env::var(INJECT_AUDIO_STEP_ENV)
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(0.0)
}

fn micros(start: Instant) -> u64 {
    u64::try_from(start.elapsed().as_nanos() / MICROS_PER_NANO_DIVISOR).expect("elapsed fits u64")
}

fn soak_only(label: &str) -> bool {
    if std::env::var_os("RUST_BROADCAST_SOAK").is_some() {
        return false;
    }
    eprintln!("skipping {label}: long re-encoded case, runs only with RUST_BROADCAST_SOAK=1");
    true
}

#[test]
fn rtsp_session_plays_a_real_mediamtx_stream_with_dims_and_buffer_start() {
    run_case("fixture-1s-gop", None, true);
}

#[test]
fn rtsp_session_starts_on_a_two_second_gop_source() {
    if soak_only("reencoded-2s-gop") {
        return;
    }
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
        false,
    );
}

#[test]
fn rtsp_session_aligns_a_three_b_frame_two_second_gop_source() {
    if soak_only("reencoded-2s-gop-3b") {
        return;
    }
    run_case(
        "reencoded-2s-gop-3b",
        Some(&[
            "-c:v",
            "libx264",
            "-preset",
            "veryfast",
            "-bf",
            "3",
            "-b_strategy",
            "0",
            "-g",
            "50",
            "-keyint_min",
            "50",
            "-sc_threshold",
            "0",
            "-r",
            "25",
        ]),
        true,
    );
}

fn run_case(label: &str, reencode_video: Option<&[&str]>, expect_reorder: bool) {
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
    let port = free_tcp_port();
    let config = write_config(&dir.0, port);
    let _server = spawn_quiet(Command::new(mediamtx).arg(&config), &dir.0);
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap()
        .block_on(test_fixtures::process::wait_for_port(
            std::net::SocketAddr::from(([127, 0, 0, 1], port)),
        ));

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
    let truth_ts = dir.0.join("truth.ts");
    let status = Command::new(&ffmpeg)
        .args(["-v", "error", "-y", "-i"])
        .arg(&source)
        .args(["-c", "copy", "-f", "mpegts"])
        .arg(&truth_ts)
        .status()
        .expect("run ffmpeg remux");
    assert!(status.success(), "ffmpeg could not remux the source to TS");
    let truth = Truth::from_media(
        &TsDemux::new()
            .unpackage(&fs::read(&truth_ts).expect("read truth.ts")[..])
            .expect("demux truth.ts"),
    );
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
    let mut wire = WireArrivals::default();
    let mut track_channel: std::collections::BTreeMap<u32, u8> = std::collections::BTreeMap::new();
    let mut first_sample_pts: std::collections::BTreeMap<u32, i64> =
        std::collections::BTreeMap::new();
    let mut emitted: Vec<(u32, i64, u64)> = Vec::new();
    let mut emitted_audio_idx: Vec<i64> = Vec::new();
    let mut emitted_data: Vec<(bool, Vec<u8>, f64)> = Vec::new();
    let mut audio_count: i64 = 0;
    let mut start_us: Option<u64> = None;
    let mut start_at_27mhz: Option<u64> = None;
    let mut media_end_at_start: Option<std::collections::BTreeMap<u32, u64>> = None;
    let mut track_timescale: std::collections::BTreeMap<u32, u32> =
        std::collections::BTreeMap::new();
    let mut track_is_video: std::collections::BTreeMap<u32, bool> =
        std::collections::BTreeMap::new();
    let mut last_media_end_27: std::collections::BTreeMap<u32, u64> =
        std::collections::BTreeMap::new();
    let mut config = Config::default();
    config.record_buffer_inputs = true;
    let mut session = Session::open(&url, config, ts(0)).expect("open");
    let mut stream: Option<(u32, TcpStream)> = None;
    let mut events = Vec::new();
    let mut buf = vec![0u8; READ_CHUNK];
    while started.elapsed() < RUN {
        let now = micros(started);
        while let Some(action) = session.poll(ts(now)) {
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
                    Err(_) => session.on_closed(conn, None, ts(now)),
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
                    session.on_closed(conn, None, ts(micros(started)));
                }
                Ok(n) => {
                    wire.feed(&buf[..n], micros(started));
                    session.on_data(*conn, &buf[..n], ts(micros(started)));
                }
                Err(e) if matches!(e.kind(), ErrorKind::WouldBlock | ErrorKind::TimedOut) => {}
                Err(e) => panic!("read: {e}"),
            }
        } else {
            thread::sleep(READ_TIMEOUT);
        }
        let now = micros(started);
        session.on_playback_position(ts(now), None);
        while let Some(e) = session.next_event() {
            match &e {
                Event::Status(crate::Status::Live) if first_live_us.is_none() => {
                    first_live_us = Some(micros(started));
                }
                Event::Tracks(specs) => {
                    for (index, s) in specs.iter().enumerate() {
                        track_channel.insert(
                            s.track_id.value(),
                            u8::try_from(index).expect("track index") * CHANNELS_PER_TRACK,
                        );
                        track_timescale.insert(s.track_id.value(), s.timescale.get());
                        let is_video = matches!(s.config, transmux::CodecConfig::Avc { .. });
                        track_is_video.insert(s.track_id.value(), is_video);
                    }
                }
                Event::AvSync(state) => sync_states.push((micros(started), *state)),
                Event::Sample { track, sample, .. } => {
                    let tid = track.value();
                    let is_video = track_is_video.get(&tid).copied().unwrap_or(false);
                    let now = micros(started);
                    if !is_video && sample.timing.map(|t| t.pts).is_some() {
                        audio_count += 1;
                    }
                    if is_video {
                        if first_video_sample_us.is_none() {
                            first_video_sample_us = Some(now);
                            first_video_sample_pts = sample.timing.map(|t| t.pts);
                        }
                        if sample.flags.is_sync && first_video_keyframe_us.is_none() {
                            first_video_keyframe_us = Some(now);
                            first_video_keyframe_pts = sample.timing.map(|t| t.pts);
                        }
                    } else if first_audio_sample_us.is_none() {
                        first_audio_sample_us = Some(now);
                        first_audio_sample_pts = sample.timing.map(|t| t.pts);
                    }
                    if let Some(pts) = sample.timing.map(|t| t.pts) {
                        let ts = track_timescale.get(&tid).copied().unwrap_or(1).max(1);
                        let seconds = pts as f64 / f64::from(ts);
                        first_sample_pts.entry(tid).or_insert(pts);
                        if sync_states.last().is_some_and(|(_, s)| is_synced(*s)) {
                            emitted.push((tid, pts, now));
                            emitted_data.push((is_video, sample.data.to_vec(), seconds));
                            emitted_audio_idx.push(if is_video { -1 } else { audio_count - 1 });
                        }
                        let slot = max_pts_seconds.entry(is_video).or_insert(seconds);
                        *slot = slot.max(seconds);
                        let dur = i64::from(sample.timing.map(|t| t.duration).unwrap_or(0));
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
        .filter(|(_, (_, s))| s.timing.map(|t| t.duration).unwrap_or(0) == 0)
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
                        ps.timing.map(|t| t.pts).unwrap_or(0),
                        ps.timing.map(|t| t.dts).unwrap_or(0),
                        ps.timing.map(|t| t.duration)
                    )
                })
                .unwrap_or_else(|| "prev=none".to_string());
            let next = samples
                .get(i + 1)
                .map(|(ntr, ns)| {
                    format!(
                        "next track={} pts={} dts={} dur={:?}",
                        ntr,
                        ns.timing.map(|t| t.pts).unwrap_or(0),
                        ns.timing.map(|t| t.dts).unwrap_or(0),
                        ns.timing.map(|t| t.duration)
                    )
                })
                .unwrap_or_else(|| "next=none".to_string());
            eprintln!(
                "  idx={} track={} pts={} dts={} dur={:?} | {} | {}",
                i,
                tr,
                s.timing.map(|t| t.pts).unwrap_or(0),
                s.timing.map(|t| t.dts).unwrap_or(0),
                s.timing.map(|t| t.duration),
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
    assert!(
        states.last().is_some_and(|s| is_synced(*s)),
        "the sync state must reach RtpInfo or SenderReport: {sync_states:?}"
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
    // A/V offset as a player renders it: per sample, its common-timeline pts minus the wall time
    // its RTP packet arrived (not when the depacketiser emitted it, which a reorder window
    // delays); the offset is audio's median minus video's.
    let emission_lag = max_pts_seconds[&false] - max_pts_seconds[&true];
    let median = |v: &mut Vec<f64>| {
        v.sort_by(f64::total_cmp);
        v[v.len() / 2]
    };
    let mut offsets: std::collections::BTreeMap<bool, Vec<f64>> = std::collections::BTreeMap::new();
    let mut anchor: std::collections::BTreeMap<u32, i64> = std::collections::BTreeMap::new();
    for (&tid, &channel) in &track_channel {
        if !track_is_video[&tid] {
            anchor.insert(tid, first_sample_pts[&tid]);
            continue;
        }
        let key_pts = first_video_keyframe_pts.expect("a video keyframe sample");
        let key_rel = wire.idr_starts[&channel]
            .iter()
            .copied()
            .find(|&cand| {
                emitted
                    .iter()
                    .filter(|(t, _, _)| *t == tid)
                    .all(|&(_, pts, _)| wire.has_exact(channel, pts - key_pts + cand))
            })
            .unwrap_or_else(|| {
                let video: Vec<i64> = emitted
                    .iter()
                    .filter(|(t, _, _)| *t == tid)
                    .map(|&(_, pts, _)| pts)
                    .collect();
                let idr = &wire.idr_starts[&channel];
                let report: Vec<String> = idr
                    .iter()
                    .take(6)
                    .map(|&cand| {
                        let misses: Vec<i64> = video
                            .iter()
                            .map(|pts| pts - key_pts + cand)
                            .filter(|rel| !wire.has_exact(channel, *rel))
                            .take(4)
                            .collect();
                        format!("cand={cand} misses(first4)={misses:?}")
                    })
                    .collect();
                panic!(
                    "no IDR maps every emitted video sample to a real RTP timestamp: key_pts={key_pts} emitted_video={} first_emitted={:?} idr_count={} {report:?}",
                    video.len(),
                    video.iter().take(5).collect::<Vec<_>>(),
                    idr.len()
                )
            });
        anchor.insert(tid, key_pts - key_rel);
    }
    let injected_audio_step = injected_audio_step_seconds();
    let mut sr_vs_arrival: std::collections::BTreeMap<bool, Vec<f64>> =
        std::collections::BTreeMap::new();
    let mut sr_origin_sets: std::collections::BTreeMap<bool, Vec<(f64, f64)>> =
        std::collections::BTreeMap::new();
    let mut sr_residual: std::collections::BTreeMap<bool, Vec<f64>> =
        std::collections::BTreeMap::new();
    for (n, &(tid, pts, _)) in emitted.iter().enumerate() {
        let channel = track_channel[&tid];
        let video = track_is_video[&tid];
        let rel = if video {
            pts - anchor[&tid]
        } else {
            emitted_audio_idx[n] * AAC_FRAME_TICKS
        };
        let arrival = wire
            .arrival_of(channel, rel)
            .unwrap_or_else(|| panic!("no RTP packet for track {tid} at relative tick {rel}"));
        let seconds = pts as f64 / f64::from(track_timescale[&tid])
            + if video { 0.0 } else { injected_audio_step };
        offsets
            .entry(video)
            .or_default()
            .push(seconds - arrival as f64 / 1e6);
        let scale = f64::from(track_timescale[&tid]);
        let wall = sr_wall_at_origin(&wire, channel, scale) + rel as f64 / scale;
        sr_residual.entry(video).or_default().push(seconds - wall);
        sr_origin_sets
            .entry(video)
            .or_insert_with(|| sr_origins(&wire, channel, scale));
        sr_vs_arrival
            .entry(video)
            .or_default()
            .push(wall - arrival as f64 / 1e6);
    }
    let audio_offset = median(
        offsets
            .get_mut(&false)
            .expect("audio samples after SR sync"),
    );
    let video_offset = median(offsets.get_mut(&true).expect("video samples after SR sync"));
    let reordered = wire.order[&0].windows(2).any(|w| w[1] < w[0]);
    assert_eq!(
        reordered, expect_reorder,
        "video wire order reorder expectation for {label}"
    );
    let ground = truth.measure(emitted_data.iter().map(|(v, d, p)| (*v, d.as_slice(), *p)));
    eprintln!(
        "GROUNDTRUTH[{label}] audio_median_delta={:.4} video_median_delta={:.4} offset={:.4} (matched a={} v={})",
        ground.audio_median,
        ground.video_median,
        ground.offset(),
        ground.audio_matched,
        ground.video_matched
    );
    let sr_faithfulness = median(sr_residual.get_mut(&false).expect("audio"))
        - median(sr_residual.get_mut(&true).expect("video"));
    for (is_video, origins) in [
        (&false, &sr_origin_sets[&false]),
        (&true, &sr_origin_sets[&true]),
    ] {
        assert!(
            origins.len() >= MIN_SENDER_REPORTS_PER_TRACK,
            "{label}: the {} track carried {} Sender Reports; mediamtx sends one per second, so a run \
             with fewer is a broken run and the Sender Report and A/V checks cannot be judged",
            if *is_video { "video" } else { "audio" },
            origins.len()
        );
    }
    let reference_spread = sr_av_reference_drift(&sr_origin_sets[&false], &sr_origin_sets[&true]);
    let reference_consistent = reference_spread <= SR_REFERENCE_DRIFT_SECONDS;
    eprintln!(
        "SRFAITHFUL[{label}] emitted-timeline-minus-SR-mapping audio-vs-video={sr_faithfulness:.4} sr-av-reference-drift={reference_spread:.4}"
    );
    if reference_consistent {
        assert_eq!(
            sr_faithfulness_violation(states.last().copied(), sr_faithfulness),
            None,
            "{label}: the emitted timeline departs from the Sender Report mapping"
        );
    } else {
        eprintln!(
            "SRFAITHFUL[{label}] not checked: the server's audio and video Sender Reports moved {reference_spread:.4} s against each other, so neither they nor the A/V offset synced to them can be judged (the hermetic tests in tests/it/rtsp_avsync.rs are the precise guard)"
        );
    }
    let video_sent_early_s =
        (ground.audio_median - audio_offset) - (ground.video_median - video_offset);
    eprintln!(
        "SENDERPACING[{label}] video arrives {video_sent_early_s:.4} s earlier than audio, relative to source pts"
    );
    let sr_minus_arrival = median(sr_vs_arrival.get_mut(&false).expect("audio"))
        - median(sr_vs_arrival.get_mut(&true).expect("video"));
    eprintln!(
        "SRARRIVAL[{label}] SR wall minus client arrival, audio-vs-video={sr_minus_arrival:.4}"
    );
    let av_offset = audio_offset - video_offset;
    eprintln!(
        "AVSYNC[{label}] states={sync_states:?} first_audio_minus_first_video_s={first_gap:.4} av_offset_s={av_offset:.4} (median pts-minus-RTP-arrival: audio {audio_offset:.4}, video {video_offset:.4}) emission_lag_s={emission_lag:.4} (info only)"
    );
    let expected_first_gap = expected_first_gap(states.last().copied(), video_sent_early_s);
    assert!(
        (first_gap - expected_first_gap).abs() < COMMON_TIMELINE_BOUND_SECONDS,
        "first audio is {first_gap} s from first video on the common timeline, expected {expected_first_gap} s"
    );
    assert!(
        !reference_consistent || av_offset.abs() < COMMON_TIMELINE_BOUND_SECONDS,
        "A/V offset (median audio pts-minus-RTP-packet-arrival minus the same for video, after SenderReport sync) is {av_offset} s"
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
    use buffering_runtime::Ticks27;
    use std::collections::BTreeMap;
    let mut end: BTreeMap<u32, Ticks27> = BTreeMap::new();
    let mut origin: Option<Ticks27> = None;
    let mut next_print = 0u64;
    for input in session.buffer_input_log() {
        match input {
            BufferInput::Register { track: t, .. } => eprintln!("LEAD[{label}] register track {t}"),
            BufferInput::Unregister { track: t, .. } => {
                eprintln!("LEAD[{label}] unregister track {t}")
            }
            BufferInput::TrackMedia {
                now_us,
                track,
                end: e,
            } => {
                origin = origin.or(Some(*e));
                end.insert(*track, *e);
                if now_us.0 >= next_print {
                    next_print = now_us.0 + 250_000;
                    let o = origin.map_or(0, |o| o.0);
                    let per: Vec<String> = end
                        .iter()
                        .map(|(t, e)| format!("t{t}_end={}ms", (e.0 as i128 - o as i128) / 27_000))
                        .collect();
                    let lead = end
                        .values()
                        .min()
                        .map_or(0, |m| (m.0 as i128 - o as i128) / 27_000);
                    eprintln!(
                        "LEAD[{label}] at={}ms {} min_lead={}ms",
                        now_us.0 / 1000,
                        per.join(" "),
                        lead
                    );
                }
            }
            _ => {}
        }
    }
}

#[test]
fn sender_report_faithfulness_binds_only_in_sender_report_mode() {
    let beyond_bound = SR_FAITHFUL_SECONDS * BEYOND_BOUND_FACTOR;
    let within_bound = SR_FAITHFUL_SECONDS / WITHIN_BOUND_DIVISOR;
    assert_eq!(
        sr_faithfulness_violation(Some(AvSync::RtpInfo), beyond_bound),
        None,
        "an RTP-Info timeline is the server's statement and is not derived from Sender Reports"
    );
    assert_eq!(
        sr_faithfulness_violation(Some(AvSync::SenderReport), beyond_bound),
        Some(beyond_bound)
    );
    assert_eq!(
        sr_faithfulness_violation(Some(AvSync::SenderReport), within_bound),
        None
    );
}

#[test]
fn first_sample_gap_expectation_follows_the_sync_mode() {
    assert_eq!(expected_first_gap(Some(AvSync::SenderReport), 0.127), 0.0);
    assert_eq!(expected_first_gap(Some(AvSync::RtpInfo), 0.127), 0.127);
}

#[test]
fn origin_at_interpolates_and_holds_its_ends() {
    let origins = [(100.0, 1.0), (102.0, 1.4)];
    assert_eq!(origin_at(&origins, 101.0), 1.2, "halfway between two SRs");
    assert_eq!(
        origin_at(&origins, 99.0),
        0.8,
        "before the first SR, the first two SRs' slope extended"
    );
    assert!(
        (origin_at(&origins, 105.0) - 2.0).abs() < 1e-9,
        "after the last SR, the last two SRs' slope extended: {}",
        origin_at(&origins, 105.0)
    );
    assert_eq!(origin_at(&origins[..1], 105.0), 1.0, "one SR is that SR");
    assert!(origin_at(&[], 0.0).is_nan(), "no SR has no origin");
}

#[test]
fn sr_av_reference_drift_reads_equality_as_zero_and_a_relative_move() {
    let at = |i: u32| (100.0 + SR_REPORT_SPACING_SECONDS * f64::from(i), 5.0);
    let audio: Vec<(f64, f64)> = (0..4).map(at).collect();
    let video: Vec<(f64, f64)> = (0..4).map(at).collect();
    assert!(sr_av_reference_drift(&audio, &video).abs() < 1e-9);

    // The video channel's origin moves part-way through the reports, the shape a stream
    // whose clock was adjusted mid-run has.
    let moved: Vec<(f64, f64)> = video
        .iter()
        .map(|&(ntp, origin)| {
            (
                ntp,
                origin
                    + if ntp > SR_RELATIVE_MOVE_AFTER_SECONDS {
                        SR_RELATIVE_MOVE_SECONDS
                    } else {
                        0.0
                    },
            )
        })
        .collect();
    let drift = sr_av_reference_drift(&audio, &moved);
    assert!(
        (drift - SR_RELATIVE_MOVE_SECONDS).abs() < 1e-9,
        "a relative move of {SR_RELATIVE_MOVE_SECONDS} s reads as {drift} s"
    );
}
