//! A PID the PMT declares but that never carries a packet must not hold the
//! whole program back (round-9 F3). The streams are real captures with one
//! extra, never-sent AAC PID written into every PMT.

use std::time::Duration;

use broadcast_common::{Stage, Timestamp, crc32_mpeg2};
use media_plane::ingress::SessionEvent;
use multimux::source::ts_program::TsIngestSession;

const TS_PACKET: usize = 188;
const SYNC_BYTE: u8 = 0x47;
const PAT_PID: u16 = 0;
const PUSI: u8 = 0x40;
const PID_HIGH_MASK: u8 = 0x1F;
const PMT_TABLE_ID: u8 = 0x02;
const SECTION_HEADER_LEN: usize = 3;
const CRC_LEN: usize = 4;
const PMT_FIXED_AFTER_HEADER: usize = 9;
const ES_ENTRY_LEN: usize = 5;
const STREAM_TYPE_AAC_ADTS: u8 = 0x0F;
const SILENT_PID: u16 = 0x1FF;
const CHUNK_PACKETS: usize = 7;
const SECTION_LENGTH_HIGH_MASK: u8 = 0x0F;
const RESERVED_PID_BITS: u8 = 0xE0;
const RESERVED_LEN_BITS: u8 = 0xF0;
const PAYLOAD_OFFSET: usize = 4;
const POINTER_FIELD_LEN: usize = 1;
const STUFFING: u8 = 0xFF;
const FIRST_OUTPUT_SLACK: Duration = Duration::from_millis(1500);

fn pid(packet: &[u8]) -> u16 {
    u16::from(packet[1] & PID_HIGH_MASK) << 8 | u16::from(packet[2])
}

#[derive(Clone, Copy, PartialEq)]
enum Position {
    First,
    Last,
}

fn with_silent_pid(ts: &[u8], position: Position) -> Vec<u8> {
    let mut pmt_pid = None;
    for packet in ts.chunks_exact(TS_PACKET) {
        if pid(packet) == PAT_PID && packet[1] & PUSI != 0 {
            let section = &packet[PAYLOAD_OFFSET + POINTER_FIELD_LEN..];
            let program_loop = &section[8..];
            pmt_pid =
                Some(u16::from(program_loop[2] & PID_HIGH_MASK) << 8 | u16::from(program_loop[3]));
            break;
        }
    }
    let pmt_pid = pmt_pid.expect("PAT with a program");
    let mut out = Vec::with_capacity(ts.len());
    for packet in ts.chunks_exact(TS_PACKET) {
        if pid(packet) != pmt_pid || packet[1] & PUSI == 0 {
            out.extend_from_slice(packet);
            continue;
        }
        let start = PAYLOAD_OFFSET + POINTER_FIELD_LEN;
        assert_eq!(packet[start], PMT_TABLE_ID);
        let section_length = usize::from(packet[start + 1] & SECTION_LENGTH_HIGH_MASK) << 8
            | usize::from(packet[start + 2]);
        let end = start + SECTION_HEADER_LEN + section_length;
        let body = &packet[start..end - CRC_LEN];
        let program_info_len = usize::from(body[SECTION_HEADER_LEN + 7] & SECTION_LENGTH_HIGH_MASK)
            << 8
            | usize::from(body[SECTION_HEADER_LEN + 8]);
        let es_loop = SECTION_HEADER_LEN + PMT_FIXED_AFTER_HEADER + program_info_len;
        let mut entry = vec![STREAM_TYPE_AAC_ADTS];
        entry.push(RESERVED_PID_BITS | (SILENT_PID >> 8) as u8);
        entry.push(SILENT_PID as u8);
        entry.push(RESERVED_LEN_BITS);
        entry.push(0);
        assert_eq!(entry.len(), ES_ENTRY_LEN);
        let mut section = body.to_vec();
        let at = match position {
            Position::First => es_loop,
            Position::Last => section.len(),
        };
        section.splice(at..at, entry);
        let new_len = section.len() - SECTION_HEADER_LEN + CRC_LEN;
        section[1] = (section[1] & !SECTION_LENGTH_HIGH_MASK) | (new_len >> 8) as u8;
        section[2] = new_len as u8;
        section.extend_from_slice(&crc32_mpeg2::compute(&section).to_be_bytes());
        let mut rebuilt = packet[..start].to_vec();
        rebuilt.extend_from_slice(&section);
        rebuilt.resize(TS_PACKET, STUFFING);
        assert_eq!(rebuilt[0], SYNC_BYTE);
        out.extend_from_slice(&rebuilt);
    }
    out
}

struct Outcome {
    first_program_at: Option<Duration>,
    samples: usize,
    tracks: usize,
}

fn drive(ts: &[u8], bytes_per_sec: Option<u64>, until: Option<Duration>) -> Outcome {
    let mut session = TsIngestSession::new();
    let mut outcome = Outcome {
        first_program_at: None,
        samples: 0,
        tracks: 0,
    };
    let mut fed = 0u64;
    for chunk in ts.chunks(CHUNK_PACKETS * TS_PACKET) {
        let at = bytes_per_sec.map_or(Duration::ZERO, |rate| {
            Duration::from_secs_f64(fed as f64 / rate as f64)
        });
        if until.is_some_and(|limit| at > limit) {
            return outcome;
        }
        session
            .feed(chunk, Timestamp::from_nanos(at.as_nanos() as u64))
            .expect("feed");
        fed += chunk.len() as u64;
        drain(&mut session, at, &mut outcome);
    }
    session.finish().expect("finish");
    drain(&mut session, Duration::MAX, &mut outcome);
    outcome
}

fn drain(session: &mut TsIngestSession, at: Duration, outcome: &mut Outcome) {
    while let Some(event) = session.poll() {
        match event {
            SessionEvent::NewProgram { tracks, .. } => {
                outcome.first_program_at.get_or_insert(at);
                outcome.tracks += tracks.len();
            }
            SessionEvent::Sample { .. } => outcome.samples += 1,
            _ => {}
        }
    }
}

fn fixture(rel: &str) -> Vec<u8> {
    std::fs::read(test_fixtures::path(rel)).expect("public fixture")
}

fn assert_released_by_time(rate: u64, position: Position) {
    let ts = with_silent_pid(&fixture("ts/h264_aac_40s.ts"), position);
    let out = drive(&ts, Some(rate), Some(Duration::from_secs(10)));
    let at = out.first_program_at.unwrap_or_else(|| {
        panic!("no program within 10 s of arrival at {rate} B/s: the silent PID held it")
    });
    assert!(
        at <= multimux::source::ts_program::SILENT_PID_SETTLE + FIRST_OUTPUT_SLACK,
        "first output at {at:?} at {rate} B/s"
    );
}

#[test]
fn silent_pid_does_not_hold_a_low_bitrate_spts() {
    assert_released_by_time(31_000, Position::Last);
    assert_released_by_time(31_000, Position::First);
}

#[test]
fn silent_pid_does_not_hold_a_mid_bitrate_spts() {
    assert_released_by_time(500_000, Position::Last);
    assert_released_by_time(500_000, Position::First);
}

#[test]
fn control_without_the_silent_pid_resolves_immediately() {
    let out = drive(
        &fixture("ts/h264_aac_40s.ts"),
        Some(31_000),
        Some(Duration::from_secs(10)),
    );
    assert!(out.first_program_at.expect("program") <= Duration::from_secs(3));
}

const HAPPY_PATH_FIXTURES: &[&str] = &[
    "ts/h264_aac.ts",
    "ts/h264_aac_40s.ts",
    "ts/gulli-opengop.ts",
    "ts/france2.ts",
    "ts/h264/baseline.ts",
    "ts/h264/high.ts",
    "ts/h264/main.ts",
    "ts/hevc/main.ts",
    "ts/m6-single.ts",
    "ts/aac-5_1-640k.ts",
];

#[test]
fn releasing_silent_heads_midway_leaves_streams_without_one_untouched() {
    for rel in HAPPY_PATH_FIXTURES {
        let ts = fixture(rel);
        let never_settles = drive(&ts, None, None);
        let settles_midway = drive(&ts, Some(ts.len() as u64 / 2), None);
        assert!(never_settles.samples > 0, "{rel}: no samples");
        assert_eq!(
            (settles_midway.samples, settles_midway.tracks),
            (never_settles.samples, never_settles.tracks),
            "{rel}: release changed the output of a stream with no silent PID"
        );
    }
}
