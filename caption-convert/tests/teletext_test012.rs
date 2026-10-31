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
