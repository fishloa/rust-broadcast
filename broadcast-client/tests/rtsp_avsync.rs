mod support;

use std::collections::BTreeMap;

use broadcast_client::{AvSync, Config, Event};
use broadcast_common::Unpackage;
use std::path::PathBuf;
use std::process::Command;
use support::rtsp_host::{
    BASE_URL, Host, MediaFixture, Server, channel_of, interleaved, sender_report_at,
};

use transmux::rtp::RtpMediaKind;
use transmux::{CodecConfig, TsDemux};

const FRAME_STEP_US: u64 = 10_000;
const RTP_TIMESTAMP_AT: usize = 4;
const RTP_TIMESTAMP_LEN: usize = 4;
const VIDEO_BASE: u32 = 146_072_311;
const AUDIO_BASE: u32 = 3_448_381_561;
const VIDEO_RATE: f64 = 90_000.0;
const AUDIO_RATE: f64 = 44_100.0;
const WALL_ORIGIN_SECONDS: f64 = 3_900_000_000.0;
const SR_SSRC: u32 = 0x1234_5678;
const SR_AFTER_DIVISOR: usize = 3;
const MEASURE_AFTER_NUM: usize = 2;
const MEASURE_AFTER_DEN: usize = 3;
const AAC_FRAME_SECONDS: f64 = 1024.0 / 44_100.0;
const ARRIVAL_SKEW_BOUND_SECONDS: f64 = 0.025;
const WITHIN_TOLERANCE_SKEW_SECONDS: f64 = 0.060;
const BEYOND_TOLERANCE_SKEW_SECONDS: f64 = 0.600;

struct Lane {
    video: bool,
    frames: Vec<Vec<u8>>,
    packet_ts: Vec<u32>,
    base: u32,
    channel: u8,
}

fn lanes(fixture: &MediaFixture) -> Vec<Lane> {
    fixture
        .tracks
        .iter()
        .enumerate()
        .map(|(n, track)| {
            let video = track.kind == RtpMediaKind::H264;
            let base = if video { VIDEO_BASE } else { AUDIO_BASE };
            let at = RTP_TIMESTAMP_AT..RTP_TIMESTAMP_AT + RTP_TIMESTAMP_LEN;
            let first = u32::from_be_bytes(track.packets[0][at.clone()].try_into().expect("ts"));
            let mut packet_ts = Vec::new();
            let frames = track
                .packets
                .iter()
                .map(|packet| {
                    let mut packet = packet.clone();
                    let ts = u32::from_be_bytes(packet[at.clone()].try_into().expect("ts"));
                    let shifted = base.wrapping_add(ts.wrapping_sub(first));
                    packet[at.clone()].copy_from_slice(&shifted.to_be_bytes());
                    packet_ts.push(shifted);
                    interleaved(channel_of(n), &packet)
                })
                .collect();
            Lane {
                video,
                frames,
                packet_ts,
                base,
                channel: channel_of(n),
            }
        })
        .collect()
}

struct Outcome {
    audio: Vec<f64>,
    video: Vec<f64>,
    audio_rel_seconds: Vec<f64>,
    sync: Vec<AvSync>,
    discontinuities: usize,
    audio_timescale: f64,
    video_timescale: f64,
}

fn send_sender_reports(host: &mut Host, lanes: &[Lane], i: usize, skew: f64) {
    for lane in lanes {
        let ts = lane.packet_ts[i.min(lane.packet_ts.len() - 1)];
        let rel = f64::from(ts.wrapping_sub(lane.base));
        let (rate, offset) = if lane.video {
            (VIDEO_RATE, 0.0)
        } else {
            (AUDIO_RATE, skew)
        };
        let wall = WALL_ORIGIN_SECONDS + rel / rate + offset;
        host.deliver(&sender_report_at(lane.channel + 1, SR_SSRC, wall, ts));
        host.drain();
    }
}

fn run(skew_seconds: Option<f64>) -> Outcome {
    run_at(&MediaFixture::load(), skew_seconds, SrAt::Third)
}

#[derive(Clone, Copy, Debug)]
enum SrAt {
    Third,
    BeforeFirstSample,
    Packet(usize),
    AfterFirstVideoAu,
}

fn run_at(fixture: &MediaFixture, skew_seconds: Option<f64>, when: SrAt) -> Outcome {
    let lanes = lanes(fixture);
    let mut host = Host::new(BASE_URL, Config::default(), Server::new(&fixture.sdp));
    host.connect();
    let longest = lanes.iter().map(|l| l.frames.len()).max().unwrap_or(0);
    let first_video_au_end = lanes
        .iter()
        .find(|l| l.video)
        .map(|l| {
            l.packet_ts
                .iter()
                .take_while(|t| **t == l.packet_ts[0])
                .count()
        })
        .unwrap_or(0);
    let sr_at = match when {
        SrAt::Third => longest / SR_AFTER_DIVISOR,
        SrAt::BeforeFirstSample => 0,
        SrAt::Packet(n) => n,
        SrAt::AfterFirstVideoAu => first_video_au_end,
    };
    let before_media = matches!(when, SrAt::BeforeFirstSample);
    let mut all = Vec::new();
    for i in 0..longest {
        if let (Some(skew), true, true) = (skew_seconds, before_media, i == 0) {
            send_sender_reports(&mut host, &lanes, 0, skew);
        }
        for lane in &lanes {
            if let Some(frame) = lane.frames.get(i) {
                host.now += FRAME_STEP_US;
                host.deliver(frame);
                host.drain();
            }
        }
        if let (Some(skew), false, true) = (skew_seconds, before_media, i == sr_at) {
            send_sender_reports(&mut host, &lanes, i, skew);
        }
        all.extend(host.events());
    }

    let audio_lane = lanes.iter().find(|l| !l.video).expect("audio lane");
    let mut distinct = Vec::new();
    for ts in &audio_lane.packet_ts {
        if distinct.last() != Some(ts) {
            distinct.push(*ts);
        }
    }
    let audio_rel_seconds = distinct
        .iter()
        .map(|ts| f64::from(ts.wrapping_sub(AUDIO_BASE)) / AUDIO_RATE)
        .collect();
    let mut out = Outcome {
        audio: Vec::new(),
        video: Vec::new(),
        audio_rel_seconds,
        sync: Vec::new(),
        discontinuities: 0,
        audio_timescale: 0.0,
        video_timescale: 0.0,
    };
    let mut is_video = BTreeMap::new();
    for e in &all {
        match e {
            Event::Tracks(specs) => {
                for s in specs {
                    let video = matches!(s.config, CodecConfig::Avc { .. });
                    is_video.insert(s.track_id, video);
                    if video {
                        out.video_timescale = f64::from(s.timescale);
                    } else {
                        out.audio_timescale = f64::from(s.timescale);
                    }
                }
            }
            Event::Sample { track, sample, .. } => {
                let pts = sample.pts.expect("pts") as f64;
                if is_video[track] {
                    out.video.push(pts);
                } else {
                    out.audio.push(pts);
                }
            }
            Event::AvSync(state) => out.sync.push(*state),
            Event::Discontinuity => out.discontinuities += 1,
            _ => {}
        }
    }
    assert!(
        out.audio.len() > 40 && out.video.len() > 40,
        "too few samples"
    );
    assert_eq!(out.audio_timescale, AUDIO_RATE);
    assert_eq!(out.video_timescale, VIDEO_RATE);
    out
}

fn seconds(o: &Outcome, ticks: f64, video: bool) -> f64 {
    ticks
        / if video {
            o.video_timescale
        } else {
            o.audio_timescale
        }
}

fn assert_audio_on_sender_report_timeline(o: &Outcome, skew: f64) {
    let video_first = seconds(o, o.video[0], true);
    let from = o.audio.len() * MEASURE_AFTER_NUM / MEASURE_AFTER_DEN;
    for k in from..o.audio.len() {
        let expected = video_first + o.audio_rel_seconds[k] + skew;
        let got = seconds(o, o.audio[k], false);
        assert!(
            (got - expected).abs() < AAC_FRAME_SECONDS,
            "audio sample {k}: {got} s, sender reports say {expected} s"
        );
    }
}

fn assert_monotonic(o: &Outcome) {
    for pair in o.audio.windows(2) {
        assert!(pair[1] > pair[0], "audio pts went backwards: {pair:?}");
    }
}

#[test]
fn before_sender_reports_first_audio_and_video_share_a_timeline() {
    let o = run(None);
    assert_eq!(o.video[0], f64::from(VIDEO_BASE), "video pts unchanged");
    let skew = seconds(&o, o.audio[0], false) - seconds(&o, o.video[0], true);
    assert!(
        skew.abs() <= ARRIVAL_SKEW_BOUND_SECONDS,
        "first audio is {skew} s from first video"
    );
    assert_monotonic(&o);
}

#[test]
fn sender_reports_set_the_true_audio_video_offset() {
    let o = run(Some(WITHIN_TOLERANCE_SKEW_SECONDS));
    assert_audio_on_sender_report_timeline(&o, WITHIN_TOLERANCE_SKEW_SECONDS);
    assert_eq!(
        o.discontinuities, 0,
        "a correction within tolerance is smooth"
    );
    assert_monotonic(&o);
}

#[test]
fn a_correction_beyond_tolerance_is_one_discontinuity() {
    let o = run(Some(BEYOND_TOLERANCE_SKEW_SECONDS));
    assert_eq!(o.discontinuities, 1);
    assert_audio_on_sender_report_timeline(&o, BEYOND_TOLERANCE_SKEW_SECONDS);
    assert_monotonic(&o);
}

#[test]
fn the_sync_state_goes_provisional_then_sender_report() {
    assert_eq!(run(None).sync, vec![AvSync::Provisional]);
    let expected = vec![AvSync::Provisional, AvSync::SenderReport];
    assert_eq!(run(Some(WITHIN_TOLERANCE_SKEW_SECONDS)).sync, expected);
    assert_eq!(run(Some(BEYOND_TOLERANCE_SKEW_SECONDS)).sync, expected);
}

#[test]
fn the_final_offset_is_the_true_offset_whenever_the_first_sender_report_arrives() {
    let fixture = MediaFixture::load();
    let whens = [
        SrAt::BeforeFirstSample,
        SrAt::Packet(3),
        SrAt::Packet(14),
        SrAt::AfterFirstVideoAu,
        SrAt::Third,
    ];
    for skew in [
        0.0,
        WITHIN_TOLERANCE_SKEW_SECONDS,
        -WITHIN_TOLERANCE_SKEW_SECONDS,
        BEYOND_TOLERANCE_SKEW_SECONDS,
    ] {
        for when in whens {
            let o = run_at(&fixture, Some(skew), when);
            assert_audio_on_sender_report_timeline(&o, skew);
            assert_monotonic(&o);
        }
    }
}

const GENERATED_SECONDS: &str = "12";
const GENERATED_LOOPS: &str = "6";

fn three_b_frame_fixture() -> Option<MediaFixture> {
    let dir = std::env::temp_dir().join(format!("rtsp-avsync-3b-{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("scratch dir");
    let out: PathBuf = dir.join("source.ts");
    let status = Command::new("ffmpeg")
        .args(["-v", "error", "-y", "-stream_loop", GENERATED_LOOPS, "-i"])
        .arg(test_fixtures::path("ts/h264_aac.ts"))
        .args(["-t", GENERATED_SECONDS])
        .args(["-c:v", "libx264", "-preset", "veryfast", "-bf", "3"])
        .args(["-b_strategy", "0", "-g", "50", "-keyint_min", "50"])
        .args([
            "-sc_threshold",
            "0",
            "-r",
            "25",
            "-c:a",
            "aac",
            "-f",
            "mpegts",
        ])
        .arg(&out)
        .status();
    let fixture = match status {
        Ok(s) if s.success() => {
            let data = std::fs::read(&out).expect("read generated source");
            let media = TsDemux::new()
                .unpackage(&data[..])
                .expect("demux generated");
            Some(MediaFixture::from_media(&media))
        }
        _ => None,
    };
    let _ = std::fs::remove_dir_all(&dir);
    fixture
}

#[test]
fn a_three_b_frame_two_second_gop_source_lands_on_the_sender_report_timeline() {
    let Some(fixture) = three_b_frame_fixture() else {
        assert!(
            std::env::var_os("REQUIRE_MEDIAMTX_E2E").is_none(),
            "ffmpeg with libx264 is required"
        );
        eprintln!("SKIP: ffmpeg with libx264 not available");
        return;
    };
    let video = fixture
        .tracks
        .iter()
        .find(|t| t.kind == RtpMediaKind::H264)
        .expect("video");
    let ts: Vec<u32> = video
        .packets
        .iter()
        .map(|p| {
            u32::from_be_bytes(
                p[RTP_TIMESTAMP_AT..RTP_TIMESTAMP_AT + RTP_TIMESTAMP_LEN]
                    .try_into()
                    .expect("ts"),
            )
        })
        .collect();
    assert!(
        ts.windows(2)
            .any(|w| w[1].wrapping_sub(w[0]) > u32::MAX / 2),
        "the generated video must carry reordered (B-frame) timestamps"
    );
    let whens = [
        SrAt::BeforeFirstSample,
        SrAt::Packet(3),
        SrAt::Packet(14),
        SrAt::AfterFirstVideoAu,
        SrAt::Third,
    ];
    for skew in [
        0.0,
        WITHIN_TOLERANCE_SKEW_SECONDS,
        BEYOND_TOLERANCE_SKEW_SECONDS,
    ] {
        for when in whens {
            let o = run_at(&fixture, Some(skew), when);
            assert_audio_on_sender_report_timeline(&o, skew);
            assert_monotonic(&o);
        }
    }
}
