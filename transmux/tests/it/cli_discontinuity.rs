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

use transmux::cli::{CliError, Opts, Output, OutputFormat, run_bytes};
use transmux::error::Error;
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
    match run_bytes(data, &opts(format))
        .unwrap_or_else(|error| panic!("{format} over the capture: {error}"))
    {
        Output::Manifest { text, .. } => text,
        Output::Bytes(_) => panic!("{format}: expected a manifest"),
        _ => panic!("{format}: unexpected output"),
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

fn is_unsupported(error: &CliError) -> bool {
    matches!(error, CliError::Transmux(Error::UnsupportedFeature(_)))
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
fn dash_output_refuses_the_break_with_a_typed_error() {
    let error = run_bytes(&broken_capture(), &opts(OutputFormat::Dash))
        .expect_err("a single-period MPD has no way to signal the break");
    assert!(
        is_unsupported(&error),
        "expected a typed refusal, got: {error}"
    );
}
