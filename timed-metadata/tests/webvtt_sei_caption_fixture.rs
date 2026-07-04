//! SEI-carried CEA-608 -> WebVTT test (issue #599, follow-up to #568).
//!
//! #568 wired the **PES-carried** `cc_data()` path to
//! [`Cea608CueExtractor`]/[`Cea708CueExtractor`]. #599 adds the **in-band SEI**
//! carriage path (ATSC A/53 `user_data_registered_itu_t_t35` in an H.264/HEVC
//! SEI NAL — `transmux::nal::caption_cc_data`). This test proves the two
//! carriage sources converge on the *same* extractor API: `caption_cc_data`
//! unwraps the SEI, `cc_data::CcData::parse` decodes the resulting bytes
//! exactly like the PES path, and `Cea608CueExtractor::push_frame` doesn't
//! know or care which carriage the triplets came from.
//!
//! Two fixtures:
//!
//! - **Synthetic (always runs):** the *same* per-frame `cc_data()` byte
//!   sequence as `webvtt_cc_fixture.rs`'s committed
//!   `fixtures/cc/cea608_cc1_synthetic.txt` (#568's pop-on "HELLO" / roll-up
//!   "HI"/"BYE" / paint-on "OK" plan), each frame re-wrapped in an ATSC A/53
//!   caption SEI NAL here instead of being fed as raw `cc_data()`. Decoding
//!   through the SEI-unwrap step must reproduce **exactly** the same cues as
//!   #568's PES-path test — the strongest form of the convergence claim.
//! - **Real capture (skip-gated):** `samples.ffmpeg.org` ffmpeg-bugs sample
//!   `transformers_EIA608_H264.ts` (ticket #2885) — real ATSC A/53 caption SEI
//!   in a real broadcast-style H.264 stream (confirmed: the exact
//!   `B5 00 31 47 41 39 34 03` T.35/GA94/type-code signature appears 401 times
//!   in the first 15 MB alone). Licensed third-party footage, so — per this
//!   project's fixture-licensing convention — only a short byte-range slice is
//!   fetched on demand into the gitignored `.test-streams/` (see
//!   `tools/fetch-test-streams.sh transformers-eia608-h264`), never committed.
//!   Demuxed through `transmux::TsDemux` (unmodified — issue #599 does not
//!   touch `ts_demux.rs`), then through the same `caption_cc_data` ->
//!   `CcData::parse` -> `Cea608CueExtractor` pipeline. Expected caption text
//!   was independently derived by running
//!   `ffmpeg -f lavfi -i "movie=<slice>[out+subcc]" -c:s srt` (ffmpeg's own
//!   EIA-608 decoder, `cc_dec`) against the exact same slice — an external
//!   oracle, not a value invented for this test.
#![cfg(feature = "cc-data")]

use broadcast_common::{Parse, Unpackage};
use cc_data::CcData;
use cc_data::decode::Cea608Channel;
use std::fs;
use std::path::{Path, PathBuf};
use timed_metadata::event::MediaTime;
use timed_metadata::webvtt::{Cea608CueExtractor, write_document};
use transmux::{CodecConfig, NalCodec, TsDemux, caption_cc_data};

// ---------------------------------------------------------------------------
// Shared: SEI wrap + WebVTT structural validity (mirrors webvtt_cc_fixture.rs)
// ---------------------------------------------------------------------------

/// SEI `payloadType`/`payloadSize` varint (ITU-T H.264 Annex D.1.6): a run of
/// `0xFF` bytes terminated by a byte `< 0xFF`.
fn sei_varint(mut v: u32) -> Vec<u8> {
    let mut out = Vec::new();
    while v >= 0xFF {
        out.push(0xFF);
        v -= 0xFF;
    }
    out.push(v as u8);
    out
}

/// Wrap one frame's `cc_data()` bytes in a complete Annex B H.264 SEI NAL
/// carrying an ATSC A/53 `user_data_registered_itu_t_t35` message (the exact
/// wire form `transmux::nal::caption_cc_data` recognises).
fn wrap_a53_sei(cc_data: &[u8]) -> Vec<u8> {
    const ITU_T_T35_COUNTRY_CODE_USA: u8 = 0xB5;
    const ATSC_T35_PROVIDER_CODE: [u8; 2] = [0x00, 0x31];
    const GA94: [u8; 4] = *b"GA94";
    const ATSC_USER_DATA_TYPE_CODE_CC_DATA: u8 = 0x03;
    const SEI_PAYLOAD_TYPE_USER_DATA_REGISTERED_ITU_T_T35: u32 = 4;
    const AVC_NAL_HEADER_SEI: u8 = 0x06;

    let mut nal = vec![0x00, 0x00, 0x01, AVC_NAL_HEADER_SEI];
    nal.extend(sei_varint(SEI_PAYLOAD_TYPE_USER_DATA_REGISTERED_ITU_T_T35));
    let payload_len = 1 + 2 + 4 + 1 + cc_data.len();
    nal.extend(sei_varint(payload_len as u32));
    nal.push(ITU_T_T35_COUNTRY_CODE_USA);
    nal.extend_from_slice(&ATSC_T35_PROVIDER_CODE);
    nal.extend_from_slice(&GA94);
    nal.push(ATSC_USER_DATA_TYPE_CODE_CC_DATA);
    nal.extend_from_slice(cc_data);
    nal
}

/// Standalone WebVTT well-formedness check (no external tool dependency) —
/// same grammar `webvtt_cc_fixture.rs` checks for the PES path.
fn assert_valid_webvtt(doc: &str) {
    let blocks: Vec<&str> = doc.split("\n\n").collect();
    assert_eq!(
        blocks.first().and_then(|b| b.lines().next()),
        Some("WEBVTT"),
        "must start with the signature: {doc:?}"
    );
    assert_eq!(blocks.last(), Some(&""), "must end blank-line-terminated");
    assert!(blocks.len() > 2, "expected at least one cue block: {doc:?}");
    for block in &blocks[1..blocks.len() - 1] {
        let mut lines = block.lines();
        let timings = lines
            .next()
            .unwrap_or_else(|| panic!("empty cue block: {doc:?}"));
        assert!(
            timings.contains(" --> "),
            "cue block must open with a timings line: {timings:?}"
        );
        assert!(
            lines.next().is_some(),
            "cue block must have a payload: {block:?}"
        );
    }
}

// ---------------------------------------------------------------------------
// Synthetic fixture: SEI-wrap the SAME frames #568 feeds as raw PES cc_data()
// ---------------------------------------------------------------------------

fn synthetic_fixture_path() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("fixtures")
        .join("cc")
        .join("cea608_cc1_synthetic.txt")
}

fn load_synthetic_frames() -> Vec<(u64, Vec<u8>)> {
    let text = fs::read_to_string(synthetic_fixture_path())
        .expect("read cea608_cc1_synthetic.txt fixture (shared with issue #568)");
    let mut frames = Vec::new();
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let mut parts = line.split_whitespace();
        let pts: u64 = parts.next().expect("pts field").parse().expect("u64 pts");
        let hex = parts.next().expect("hex field");
        let bytes = (0..hex.len())
            .step_by(2)
            .map(|i| u8::from_str_radix(&hex[i..i + 2], 16).expect("valid hex byte"))
            .collect();
        frames.push((pts, bytes));
    }
    frames
}

/// `caption_cc_data` must reproduce each frame's original `cc_data()` bytes
/// byte-for-byte after the SEI round trip — the pure extraction claim, before
/// any caption decode is involved.
#[test]
fn caption_cc_data_round_trips_every_synthetic_frame() {
    let frames = load_synthetic_frames();
    assert_eq!(frames.len(), 13, "fixture frame count changed unexpectedly");
    for (pts, cc_data) in &frames {
        let au = wrap_a53_sei(cc_data);
        let extracted = caption_cc_data(NalCodec::Avc, &au, false);
        assert_eq!(&extracted, cc_data, "frame at pts {pts} round-trip");
    }
}

/// The full SEI -> `CcData` -> `Cea608CueExtractor` pipeline on the
/// SEI-wrapped fixture must produce **exactly** the same cues #568's
/// `decode_to_expected_cues` test gets from the raw PES `cc_data()` path —
/// proof the two carriage sources converge on identical output.
#[test]
fn sei_path_matches_pes_path_expected_cues() {
    let frames = load_synthetic_frames();
    let mut ex = Cea608CueExtractor::new(Cea608Channel::Cc1);
    for (pts, cc_data) in &frames {
        let au = wrap_a53_sei(cc_data);
        let extracted = caption_cc_data(NalCodec::Avc, &au, false);
        let cc = CcData::parse(&extracted).expect("valid cc_data() Table B.9 bytes");
        ex.push_frame(*pts, &cc.triplets);
    }
    ex.finalize(45_000);
    let cues = ex.into_cues();

    // Identical expected list to webvtt_cc_fixture.rs::decode_to_expected_cues.
    let expected = [
        (6_000u64, 9_000u64, "HELLO"),
        (15_000, 21_000, "HI"),
        (21_000, 24_000, "HI\nBYE"),
        (24_000, 33_000, "BYE"),
        (39_000, 42_000, "OK"),
    ];
    assert_eq!(cues.len(), expected.len(), "cues: {cues:?}");
    for (cue, (start, end, text)) in cues.iter().zip(expected.iter()) {
        assert_eq!(cue.start, MediaTime(*start), "cue {text:?} start");
        assert_eq!(cue.end, MediaTime(*end), "cue {text:?} end");
        assert_eq!(cue.text, *text);
    }

    let doc = write_document(&cues);
    assert_valid_webvtt(&doc);
    assert!(doc.contains("HELLO"));
    assert!(doc.contains("HI\nBYE"));
}

/// A non-caption SEI (`recovery_point`, from #595) alongside real VCL slices
/// must not be mistaken for a caption — `caption_cc_data` returns nothing, so
/// there is nothing for the extractor to see.
#[test]
fn non_caption_sei_produces_no_cues() {
    let au = [
        0x00, 0x00, 0x01, 0x67, 0x42, 0x00, 0x0A, // SPS
        0x00, 0x00, 0x01, 0x06, 0x06, 0x00, 0x80, // recovery_point SEI (payloadType 6)
        0x00, 0x00, 0x01, 0x41, 0x9A, // non-IDR slice
    ];
    let extracted = caption_cc_data(NalCodec::Avc, &au, false);
    assert!(extracted.is_empty());

    let mut ex = Cea608CueExtractor::new(Cea608Channel::Cc1);
    ex.finalize(0);
    assert!(ex.into_cues().is_empty());
}

// ---------------------------------------------------------------------------
// Real capture (skip-gated): samples.ffmpeg.org transformers_EIA608_H264.ts
// ---------------------------------------------------------------------------

const REAL_CAPTURE: &str = "transformers-eia608-h264";

fn real_capture_path() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join(".test-streams")
        .join(format!("{REAL_CAPTURE}.ts"))
}

/// Demux `data` (a whole/partial MPEG-2 TS byte stream), extract every A/53
/// caption SEI from the H.264 video track's access units via
/// `transmux::caption_cc_data`, and return `(pts_90k, cc_data_bytes)` in
/// decode order for every access unit that carried one.
fn extract_sei_frames_from_ts(data: &[u8]) -> Vec<(u64, Vec<u8>)> {
    let media = TsDemux::new()
        .unpackage(data)
        .expect("demux the real capture slice");
    let video = media
        .tracks
        .iter()
        .find(|t| matches!(t.config(), CodecConfig::Avc { .. }))
        .expect("capture must contain an H.264 video track");

    let mut frames = Vec::new();
    let mut dts = video.start_decode_time;
    for sample in &video.samples {
        let pts = dts.wrapping_add(sample.composition_offset as i64 as u64);
        let cc = caption_cc_data(NalCodec::Avc, &sample.data, true);
        if !cc.is_empty() {
            frames.push((pts, cc));
        }
        dts += u64::from(sample.duration);
    }
    // Samples are in H.264 *decode* order; with B-frames that's not
    // *presentation* order (PTS != DTS, `composition_offset` reorders them).
    // Captions must be assembled in presentation/display order — the order a
    // viewer reads them — so sort by PTS before handing frames to the
    // extractor. Skipping this scrambles roll-up/pop-on text exactly like an
    // anagram (verified: omitting the sort here reproduces garbled output).
    frames.sort_by_key(|(pts, _)| *pts);
    frames
}

/// Real capture: SEI extraction finds captions, and decoding them via the
/// same `Cea608CueExtractor` pipeline reproduces the burned-in caption text —
/// independently confirmed by running ffmpeg's own EIA-608 decoder
/// (`-f lavfi -i "movie=<slice>[out+subcc]" -c:s srt`) against the identical
/// byte slice (see module docs). We assert on substrings rather than the
/// full SRT text/line-wrap, since ffmpeg's roll-up-to-SRT formatting
/// conventions differ from this crate's (documented) diff-based one — the
/// property under test is "the right words came out of the SEI path", not
/// "byte-identical to ffmpeg's subtitle renderer".
#[test]
fn real_capture_sei_captions_match_ffmpeg_oracle() {
    let path = real_capture_path();
    if !path.exists() {
        eprintln!(
            "real_capture_sei_captions_match_ffmpeg_oracle: SKIPPED — \
             {REAL_CAPTURE}.ts not in .test-streams/. Run \
             `tools/fetch-test-streams.sh {REAL_CAPTURE}` to enable."
        );
        return;
    }
    let data = fs::read(&path).expect("read real capture slice");

    let frames = extract_sei_frames_from_ts(&data);
    assert!(
        !frames.is_empty(),
        "expected at least one A/53 caption SEI access unit in the real capture"
    );

    let mut ex = Cea608CueExtractor::new(Cea608Channel::Cc1);
    for (pts, cc_data) in &frames {
        let cc = CcData::parse(cc_data).expect("valid cc_data() Table B.9 bytes");
        ex.push_frame(*pts, &cc.triplets);
    }
    ex.finalize(frames.last().map_or(0, |(pts, _)| pts + 90_000));
    let cues = ex.into_cues();
    assert!(!cues.is_empty(), "expected at least one decoded cue");

    let doc = write_document(&cues);
    assert_valid_webvtt(&doc);

    // ffmpeg oracle (`ffmpeg -f lavfi -i "movie=<slice>[out+subcc]" -c:s srt`)
    // on the identical byte slice, first three burned-in captions. The
    // oracle's second cue is a 2-row roll-up ("Long-range defense
    // systems"/"watch the skies."); this crate's diff-based extractor
    // (documented in the `webvtt` module docs' "roll-up granularity" note)
    // only surfaces a cue at the *final* stable state of that roll before the
    // next scroll, so only the second row is asserted here — the same
    // documented behaviour #568's own fixture exercises.
    let oracle_fragments = [
        "its cities now.",
        "watch the skies.",
        "in solving human conflicts.",
    ];
    for fragment in oracle_fragments {
        assert!(
            doc.contains(fragment),
            "expected oracle caption fragment {fragment:?} in decoded WebVTT: {doc}"
        );
    }
}
