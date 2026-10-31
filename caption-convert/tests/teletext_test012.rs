//! Real-capture incremental teletext decode of TSDuck's `test-012.ts`.
//!
//! The PMT lists page 888 (`teletext_type` 0x05, hearing impaired) and page
//! 889 (type 0x02, subtitles); only 889 carries the text of TSDuck's
//! reference SRT, so that is the page decoded here.
#![cfg(feature = "teletext")]

use broadcast_common::Unpackage;
use caption_convert::{Cue, TeletextToWebVtt};
use transmux::{CodecConfig, TsDemux};

const WHO: &str = "caption-convert teletext_test012";
const TELETEXT_PID: u16 = 0x42C;
const MAGAZINE: u8 = 8;
const PAGE: u8 = 0x89;

fn samples() -> Option<Vec<(u64, Vec<u8>)>> {
    let path = test_fixtures::tsduck("input/test-012.ts", WHO)?;
    let bytes = std::fs::read(&path).expect("read test-012.ts");
    let media = TsDemux::new().unpackage(&bytes).expect("demux test-012.ts");
    let track = media
        .tracks
        .iter()
        .find(|t| {
            t.spec.source_pid.map(|p| p.value()) == Some(TELETEXT_PID)
                && matches!(t.config(), CodecConfig::Data { .. })
        })
        .expect("Data track on the teletext PID");
    Some(
        track
            .samples
            .iter()
            .map(|s| {
                let pts = s.pts.expect("teletext PES carries a PTS").max(0) as u64;
                (pts, s.data.to_vec())
            })
            .collect(),
    )
}

fn end_pts(samples: &[(u64, Vec<u8>)]) -> u64 {
    samples.last().map_or(0, |s| s.0) + 1
}

fn whole_stream(samples: &[(u64, Vec<u8>)]) -> Vec<Cue> {
    let mut conv = TeletextToWebVtt::new(MAGAZINE, PAGE);
    for (pts, data) in samples {
        conv.push_pes_data_field(*pts, data)
            .expect("parse PES data field");
    }
    conv.finalize(end_pts(samples));
    conv.into_cues()
}

#[test]
fn incremental_cues_equal_whole_stream_cues() {
    let Some(samples) = samples() else { return };
    let whole = whole_stream(&samples);
    assert!(!whole.is_empty(), "fixture must yield cues");

    let mut conv = TeletextToWebVtt::new(MAGAZINE, PAGE);
    let mut collected = Vec::new();
    for (pts, data) in &samples {
        conv.push_pes_data_field(*pts, data)
            .expect("parse PES data field");
        collected.extend(conv.take_closed_cues());
    }
    conv.finalize(end_pts(&samples));
    collected.extend(conv.take_closed_cues());

    assert_eq!(collected, whole);
    assert!(conv.cues().is_empty());
}

#[test]
fn open_cue_is_reported_while_text_is_on_screen() {
    let Some(samples) = samples() else { return };
    let whole = whole_stream(&samples);
    let first = whole.first().expect("fixture must yield cues");

    let mut conv = TeletextToWebVtt::new(MAGAZINE, PAGE);
    let mut seen = None;
    for (pts, data) in &samples {
        conv.push_pes_data_field(*pts, data)
            .expect("parse PES data field");
        if let Some(open) = conv.open_cue() {
            seen = Some(open);
            break;
        }
    }
    let open = seen.expect("open_cue must report the cue while on screen");
    assert_eq!(open.start_pts_90k, first.start.0);
    assert_eq!(open.text, first.text);
}

fn normalise_text(text: &str) -> String {
    text.lines()
        .map(str::trim_end)
        .collect::<Vec<_>>()
        .join("\n")
}

fn reference_texts() -> Option<Vec<String>> {
    let path = test_fixtures::tsduck("reference/test-012/test-012.srt", WHO)?;
    let srt = std::fs::read_to_string(path)
        .expect("read reference SRT")
        .replace("\r\n", "\n");
    Some(
        srt.split("\n\n")
            .filter(|block| !block.trim().is_empty())
            .map(|block| {
                let text: Vec<&str> = block.lines().skip(2).collect();
                normalise_text(&text.join("\n"))
            })
            .collect(),
    )
}

#[test]
fn cue_text_matches_tsduck_reference() {
    let (Some(samples), Some(want)) = (samples(), reference_texts()) else {
        return;
    };
    let got: Vec<String> = whole_stream(&samples)
        .iter()
        .map(|c| normalise_text(&c.text))
        .collect();
    assert_eq!(got, want);
}

struct SrtTiming {
    start_ms: i64,
    end_ms: i64,
}

fn parse_srt_time(s: &str) -> i64 {
    let (hms, ms) = s.trim().split_once(',').expect("SRT time has a comma");
    let mut parts = hms
        .split(':')
        .map(|p| p.parse::<i64>().expect("SRT time field"));
    let (h, m, sec) = (
        parts.next().expect("hours"),
        parts.next().expect("minutes"),
        parts.next().expect("seconds"),
    );
    ((h * 60 + m) * 60 + sec) * 1000 + ms.parse::<i64>().expect("SRT milliseconds")
}

fn reference_timings() -> Option<Vec<SrtTiming>> {
    let path = test_fixtures::tsduck("reference/test-012/test-012.tsp344.srt", WHO)?;
    let srt = std::fs::read_to_string(path)
        .expect("read timing reference SRT")
        .replace("\r\n", "\n");
    Some(
        srt.split("\n\n")
            .filter(|block| !block.trim().is_empty())
            .map(|block| {
                let (a, b) = block
                    .lines()
                    .nth(1)
                    .and_then(|l| l.split_once("-->"))
                    .expect("SRT timing line");
                SrtTiming {
                    start_ms: parse_srt_time(a),
                    end_ms: parse_srt_time(b),
                }
            })
            .collect(),
    )
}

fn max_pes_gap_ms(samples: &[(u64, Vec<u8>)]) -> i64 {
    samples
        .windows(2)
        .map(|w| (w[1].0 as i64 - w[0].0 as i64) / 90)
        .max()
        .expect("at least two PES packets")
}

fn assert_times_within_one_pes_interval(pick: fn(&Cue) -> u64, field: fn(&SrtTiming) -> i64) {
    let (Some(samples), Some(reference)) = (samples(), reference_timings()) else {
        return;
    };
    let origin = samples[0].0 as i64;
    let tolerance_ms = max_pes_gap_ms(&samples);
    let cues = whole_stream(&samples);
    assert_eq!(cues.len(), reference.len());
    for (cue, want) in cues.iter().zip(&reference) {
        let got_ms = (pick(cue) as i64 - origin) / 90;
        assert!(
            (got_ms - field(want)).abs() <= tolerance_ms,
            "{got_ms} ms vs reference {} ms (tolerance {tolerance_ms} ms)",
            field(want)
        );
    }
}

/// Times are seconds since the first teletext PES PTS: TSDuck's
/// `TimeTrackerDemux` keeps the first PTS it sees on the PID and reports
/// `last - first` (the stream has no PCR). The reference is
/// `test-012.tsp344.srt`, produced by TSDuck 3.44; the older committed
/// `test-012.srt` has the same text but cue times 320 ms earlier from an older
/// TSDuck time origin, so it is only used for the text test. Tolerance: the
/// largest gap between consecutive teletext PES PTS values, computed from the
/// stream.
#[test]
fn cue_start_times_match_tsduck_reference_within_one_pes_interval() {
    assert_times_within_one_pes_interval(|c| c.start.0, |t| t.start_ms);
}

/// Same time origin, reference and tolerance as the start-time test. TSDuck
/// ends a cue 40 ms before the next page header; that is within the tolerance.
#[test]
fn cue_end_times_match_tsduck_reference_within_one_pes_interval() {
    assert_times_within_one_pes_interval(|c| c.end.0, |t| t.end_ms);
}
