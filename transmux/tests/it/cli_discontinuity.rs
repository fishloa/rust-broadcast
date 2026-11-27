//! The CLI carries a demux discontinuity into the output (issue #4147).
//!
//! `fixtures/ts/h264-aac.ts` is a real committed capture with a continuous
//! 90 kHz timeline. Playing it twice back to back (the same bytes, once) puts a
//! real backward jump of that timeline at the seam, and `StreamingTsDemux`
//! raises `DemuxEvent::Discontinuity` there — the precondition the first test
//! pins. Repackaging those bytes through [`run_bytes`] must forward that mark
//! to the output stage: an output that can express it writes
//! `#EXT-X-DISCONTINUITY` or sets the adaptation-field
//! `discontinuity_indicator`, and one that cannot refuses with a typed error
//! rather than dropping the event.
#![cfg(feature = "cli")]

use transmux::cli::{Opts, Output, OutputFormat, run_bytes};
use transmux::{DemuxEvent, DiscontinuityKind, StreamingTsDemux};

/// One real capture, played twice: the seam is the timeline break under test.
const CAPTURE: &str = "ts/h264-aac.ts";

/// Two plays of [`CAPTURE`] back to back.
fn broken_capture() -> Vec<u8> {
    let one = test_fixtures::read(CAPTURE);
    [one.as_slice(), one.as_slice()].concat()
}

fn opts(format: OutputFormat) -> Opts {
    Opts {
        format,
        segment_duration: 1,
        ..Opts::default()
    }
}

/// `(every discontinuity, the signalled ones)` `data` raises when demuxed.
fn discontinuity_counts(data: &[u8]) -> (usize, usize) {
    let mut demux = StreamingTsDemux::new();
    demux.feed(data).expect("valid timing");
    demux.finish().expect("valid timing");
    let (mut all, mut signalled) = (0, 0);
    while let Some(event) = demux.poll_event() {
        if let DemuxEvent::Discontinuity { kind, .. } = event {
            all += 1;
            if matches!(kind, DiscontinuityKind::Signalled) {
                signalled += 1;
            }
        }
    }
    (all, signalled)
}

fn counts_discontinuities(data: &[u8]) -> usize {
    discontinuity_counts(data).0
}

fn manifest(data: &[u8], format: OutputFormat) -> String {
    manifest_with(data, &opts(format))
}

fn manifest_with(data: &[u8], opts: &Opts) -> String {
    match run_bytes(data, opts)
        .unwrap_or_else(|error| panic!("{} over the capture: {error}", opts.format))
    {
        Output::Manifest { text, .. } => text,
        Output::Bytes(_) => panic!("{}: expected a manifest", opts.format),
        _ => panic!("{}: unexpected output", opts.format),
    }
}

fn ts_bytes(data: &[u8]) -> Vec<u8> {
    match run_bytes(data, &opts(OutputFormat::Ts))
        .unwrap_or_else(|error| panic!("ts over the capture: {error}"))
    {
        Output::Bytes(bytes) => bytes,
        Output::Manifest { .. } => panic!("ts: expected bytes"),
        _ => panic!("ts: unexpected output"),
    }
}

#[test]
fn the_input_carries_a_real_timeline_break() {
    assert!(
        counts_discontinuities(&broken_capture()) > 0,
        "the doubled capture must break its timeline at the seam"
    );
    assert_eq!(
        counts_discontinuities(&test_fixtures::read(CAPTURE)),
        0,
        "one play of the capture must stay continuous"
    );
}

#[test]
fn ts_hls_manifest_signals_the_break() {
    let text = manifest(&broken_capture(), OutputFormat::TsHls);
    assert!(
        text.contains("#EXT-X-DISCONTINUITY\n"),
        "a TS-HLS playlist over a broken timeline must carry #EXT-X-DISCONTINUITY, got:\n{text}"
    );
}

#[test]
fn cmaf_hls_manifest_signals_the_break() {
    let text = manifest(&broken_capture(), OutputFormat::Hls);
    assert!(
        text.contains("#EXT-X-DISCONTINUITY\n"),
        "a CMAF-HLS playlist over a broken timeline must carry #EXT-X-DISCONTINUITY, got:\n{text}"
    );
}

#[test]
fn continuous_capture_writes_no_discontinuity_tag() {
    for format in [OutputFormat::TsHls, OutputFormat::Hls] {
        let text = manifest(&test_fixtures::read(CAPTURE), format);
        assert!(
            !text.contains("#EXT-X-DISCONTINUITY"),
            "a continuous capture must not be marked discontinuous ({format}):\n{text}"
        );
    }
}

#[test]
fn ts_output_signals_the_break_on_the_wire() {
    let bytes = ts_bytes(&broken_capture());
    let (all, signalled) = discontinuity_counts(&bytes);
    assert!(
        signalled > 0,
        "a TS remux must set the adaptation-field discontinuity_indicator, \
         got {signalled} signalled of {all} discontinuities"
    );
}

#[test]
fn continuous_capture_ts_output_signals_no_break() {
    let bytes = ts_bytes(&test_fixtures::read(CAPTURE));
    assert_eq!(
        discontinuity_counts(&bytes),
        (0, 0),
        "a continuous capture must remux without a discontinuity_indicator"
    );
}

#[test]
fn dash_output_writes_a_period_per_break() {
    assert_period_per_break(&manifest(&broken_capture(), OutputFormat::Dash));
}

#[test]
fn ll_dash_output_writes_a_period_per_break() {
    let mut opts = opts(OutputFormat::Dash);
    opts.low_latency = true;
    assert_period_per_break(&manifest_with(&broken_capture(), &opts));
}

fn assert_period_per_break(text: &str) {
    let mpd = broadcast_dash::Mpd::parse(text.as_bytes()).expect("the rendered MPD parses");
    assert!(
        mpd.periods.len() >= 2,
        "the break must open a new Period, got {} period(s):\n{text}",
        mpd.periods.len()
    );
    let mut expected_start = std::time::Duration::ZERO;
    let mut offsets = Vec::new();
    for (index, period) in mpd.periods.iter().enumerate() {
        assert_eq!(
            period.start,
            Some(expected_start),
            "period {index} starts where the previous one closed"
        );
        let duration = period
            .duration
            .unwrap_or_else(|| panic!("period {index} states a duration"));
        let mut period_offsets = Vec::new();
        for set in &period.adaptation_sets {
            for representation in &set.representations {
                let template = representation
                    .segment
                    .template
                    .as_ref()
                    .expect("every Representation has a SegmentTemplate");
                period_offsets.push(
                    template
                        .multi
                        .base
                        .presentation_time_offset
                        .unwrap_or_else(|| {
                            panic!("period {index} states a presentationTimeOffset")
                        }),
                );
            }
        }
        assert!(
            !period_offsets.is_empty(),
            "period {index} carries a Representation"
        );
        if index == 0 {
            assert!(
                period_offsets.iter().all(|offset| *offset == 0),
                "the first Period starts at media zero"
            );
        } else {
            assert!(
                period_offsets.iter().all(|offset| *offset > 0),
                "period {index}'s media clock continues past the break: {period_offsets:?}"
            );
        }
        offsets.push(period_offsets);
        expected_start += duration;
    }
    assert!(offsets.len() >= 2);
}
