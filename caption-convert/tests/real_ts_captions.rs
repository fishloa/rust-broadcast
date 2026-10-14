//! Real CEA-608 captures in MPEG-2 TS (mux.js multi-channel, shaka-player
//! `captions-test.ts`; Apache-2.0, `fixtures/captions/PROVENANCE.md`).
//!
//! 1. The workspace extractor (`tests/support/ga94.rs`) reproduces, byte for
//!    byte, the `cc_data()` list an independent Python extractor wrote
//!    (`fixtures/captions/gen_cc_data_reference.py`).
//! 2. Every real `cc_data()` parses and serializes back byte-identical
//!    (`cc-data`'s round-trip invariant on real wire bytes; the shaka frames
//!    carry `process_em_data_flag = 0`, which a Table-B.9-only header check
//!    rejects).
//! 3. `Cea608ToWebVtt` turns the real frames into cues, and the SRT/WebVTT
//!    renderings agree with each other.
#![cfg(feature = "cc-data")]

mod support;

use broadcast_common::{Parse, Serialize};
use caption_convert::Cea608ToWebVtt;
use cc_data::CcData;
use cc_data::decode::Cea608Channel;
use support::ga94::{ga94_from_ts, parse_reference};

const CAPTURES: &[(&str, &str, usize)] = &[
    (
        "captions/mux.js/multi-channel-608-captions.ts",
        "captions/mux.js/multi-channel-608-captions.cc_data.txt",
        368,
    ),
    (
        "captions/shaka/captions-test.ts",
        "captions/shaka/captions-test.cc_data.txt",
        599,
    ),
];

#[test]
fn extractor_matches_the_independent_python_reference() {
    for (ts, reference, count) in CAPTURES {
        let got = ga94_from_ts(&test_fixtures::read(ts));
        let want = parse_reference(&test_fixtures::read_to_string(reference));
        assert_eq!(got.len(), *count, "{ts}: frame count");
        assert_eq!(got, want, "{ts}: extraction differs from the reference");
    }
}

#[test]
fn every_real_cc_data_round_trips_byte_exact() {
    let mut em_clear = 0usize;
    for (ts, _, _) in CAPTURES {
        for (i, (_, bytes)) in ga94_from_ts(&test_fixtures::read(ts)).iter().enumerate() {
            let cc = CcData::parse(bytes).unwrap_or_else(|e| panic!("{ts} frame {i}: {e}"));
            assert_eq!(
                &cc.try_to_bytes().expect("serialize"),
                bytes,
                "{ts} frame {i}"
            );
            em_clear += usize::from(!cc.process_em_data_flag);
        }
    }
    // The shaka capture is what forced the tolerant header parse: all 599
    // frames have bit 7 clear. Pins that the real data really exercises it.
    assert!(em_clear >= 599, "only {em_clear} frames had bit 7 clear");
}

fn convert(ts: &str, channel: Cea608Channel) -> Cea608ToWebVtt {
    let frames = ga94_from_ts(&test_fixtures::read(ts));
    let end = frames.last().map_or(0, |(pts, _)| *pts) + 90_000;
    let mut conv = Cea608ToWebVtt::new(channel);
    for (pts, bytes) in &frames {
        conv.push_cc_data(*pts, bytes).expect("real cc_data()");
    }
    conv.finalize(end);
    conv
}

#[test]
fn real_captures_yield_cues_and_renderings_agree() {
    for (ts, _, _) in CAPTURES {
        let conv = convert(ts, Cea608Channel::Cc1);
        let cues = conv.cues().len();
        assert!(cues > 0, "{ts}: CC1 produced no cues");
        let vtt = conv.into_webvtt();
        assert!(vtt.starts_with("WEBVTT\n\n"));
        let srt = convert(ts, Cea608Channel::Cc1).into_srt();
        assert_eq!(
            srt.matches(" --> ").count(),
            cues,
            "{ts}: SRT cue count differs from the cue list"
        );
        assert_eq!(vtt.matches(" --> ").count(), cues, "{ts}: WebVTT cue count");
    }
}

/// Several caption updates share one access-unit PTS in these captures (the
/// mux.js multi-channel TS has 21 same-PTS flickers on CC1). A cue whose end
/// equals its start is never on screen, so none may be emitted, and cues stay
/// ordered.
#[test]
fn no_cue_is_zero_length_or_out_of_order_on_any_channel() {
    for (ts, _, _) in CAPTURES {
        for ch in [
            Cea608Channel::Cc1,
            Cea608Channel::Cc2,
            Cea608Channel::Cc3,
            Cea608Channel::Cc4,
        ] {
            let conv = convert(ts, ch);
            let mut prev_end = 0;
            for cue in conv.cues() {
                assert!(
                    cue.start.0 < cue.end.0,
                    "{ts} {ch:?}: zero/negative {cue:?}"
                );
                assert!(prev_end <= cue.start.0, "{ts} {ch:?}: overlap at {cue:?}");
                prev_end = cue.end.0;
            }
        }
    }
    assert!(
        convert(CAPTURES[0].0, Cea608Channel::Cc1).cues().len() > 20,
        "CC1 of the multi-channel capture is the busy one"
    );
}
