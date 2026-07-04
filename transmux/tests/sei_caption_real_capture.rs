//! Real-capture SEI caption extraction gate (issue #599).
//!
//! Keyed on `.test-streams/transformers-eia608-h264.ts` — a short byte-range
//! slice of the real ffmpeg-bugs sample
//! `samples.ffmpeg.org/ffmpeg-bugs/trac/ticket2885/transformers_EIA608_H264.ts`
//! (real ATSC A/53 caption SEI / EIA-608 in H.264; licensed third-party movie
//! footage, so only ~15 MB is fetched, never committed — see
//! `tools/fetch-test-streams.sh transformers-eia608-h264`). Skips cleanly when
//! absent.
//!
//! This test exercises the **full input path** the issue specifies: IR sample
//! bytes recovered by [`TsDemux`] (unmodified — issue #599 does not touch
//! `ts_demux.rs`), fed to [`caption_cc_data`]. `timed-metadata`'s
//! `webvtt_sei_caption_fixture.rs` covers the same capture end-to-end through
//! `cc_data::CcData`/`Cea608CueExtractor`; this test stays within `transmux`
//! (no `cc-data` dependency) and checks the extraction step alone: enough
//! caption-bearing access units are found, and the demuxer's actual first
//! caption-bearing video sample (`FIRST_EXPECTED_CC_DATA` below) is
//! byte-identical to what a direct scan of the raw capture at that sample's
//! true byte offset shows.
//!
//! Note this is a *different* real SEI than `nal::tests::REAL_A53_SEI_NAL`
//! (which is the raw capture's very first `GA94` signature occurrence by file
//! byte offset, ~514 bytes in): `TsDemux` correctly does not emit that one as
//! a sample — this capture's leading bytes are a partial/truncated access
//! unit (an expected real-world artifact of slicing a byte range out of a
//! larger file, and of TS captures in general not always starting exactly on
//! an access-unit boundary), so the demuxer's first *complete* sample begins
//! later in the file. Confirmed by locating this test's expected bytes at
//! their own distinct raw file offset (~133918) during development.

use std::fs;
use std::path::PathBuf;

use broadcast_common::Unpackage;
use transmux::{CodecConfig, NalCodec, TsDemux, caption_cc_data};

const CAPTURE: &str = "transformers-eia608-h264";

fn capture_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join(".test-streams")
        .join(format!("{CAPTURE}.ts"))
}

/// `MPEG_cc_data()` bytes of `TsDemux`'s actual first caption-bearing video
/// sample in the real capture slice — byte-for-byte from the raw file (a
/// content-addressed search for this exact 63-byte sequence in the raw
/// `.ts` bytes during development located it at raw offset ~133918, inside a
/// complete `sei_message()` immediately followed by a `rbsp_trailing_bits`
/// `0x80` — a genuine, complete access unit, unlike the earlier
/// partial-leading-fragment occurrence `nal::tests::REAL_A53_SEI_NAL` uses).
#[rustfmt::skip]
const FIRST_EXPECTED_CC_DATA: [u8; 63] = [
    0xd4, 0xff, 0xfc, 0x80, 0x80, 0xfd, 0x80, 0x80, 0xfa, 0x00, 0x00, 0xfa,
    0x00, 0x00, 0xfa, 0x00, 0x00, 0xfa, 0x00, 0x00, 0xfa, 0x00, 0x00, 0xfa,
    0x00, 0x00, 0xfa, 0x00, 0x00, 0xfa, 0x00, 0x00, 0xfa, 0x00, 0x00, 0xfa,
    0x00, 0x00, 0xfa, 0x00, 0x00, 0xfa, 0x00, 0x00, 0xfa, 0x00, 0x00, 0xfa,
    0x00, 0x00, 0xfa, 0x00, 0x00, 0xfa, 0x00, 0x00, 0xfa, 0x00, 0x00, 0xfa,
    0x00, 0x00, 0xff,
];

#[test]
fn real_capture_sei_extraction() {
    let path = capture_path();
    if !path.exists() {
        eprintln!(
            "real_capture_sei_extraction: SKIPPED — {CAPTURE}.ts not in \
             .test-streams/. Run `tools/fetch-test-streams.sh {CAPTURE}` to enable \
             (fetches a short slice of a licensed third-party sample)."
        );
        return;
    }
    let data = fs::read(&path).expect("read real capture slice");

    let media = TsDemux::new()
        .unpackage(data.as_slice())
        .expect("demux the real capture slice");
    let video = media
        .tracks
        .iter()
        .find(|t| matches!(t.config(), CodecConfig::Avc { .. }))
        .expect("capture must contain an H.264 video track");
    assert!(!video.samples.is_empty(), "video track must have samples");

    let mut caption_aus = 0usize;
    let mut first_cc_data: Option<Vec<u8>> = None;
    for sample in &video.samples {
        let cc = caption_cc_data(NalCodec::Avc, &sample.data, true);
        if !cc.is_empty() {
            caption_aus += 1;
            if first_cc_data.is_none() {
                first_cc_data = Some(cc);
            }
        }
    }

    assert!(
        caption_aus >= 10,
        "expected at least 10 caption-bearing access units in the 15 MB \
         slice (401 raw signature occurrences were found by direct byte \
         scan during development), got {caption_aus}"
    );
    assert_eq!(
        first_cc_data.as_deref(),
        Some(FIRST_EXPECTED_CC_DATA.as_slice()),
        "the demuxer's first caption-bearing sample's MPEG_cc_data() must \
         match the byte-for-byte real capture bytes"
    );
}
