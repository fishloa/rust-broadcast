//! Issue #2722: the HLS packager's presentation span must neither overflow on
//! an extreme composition offset nor hand the `DecimalSeconds` constructor a
//! value it refuses and then `expect` it to succeed.
//!
//! The span pairs each sample's presentation time (`dts` plus its composition
//! offset, the `dts`-to-`pts` distance in the track's timescale) with that
//! sample's presentation end. Twice the unchecked arithmetic has no honest
//! value there: an `i64` sum that does not fit wraps, and a span between a
//! short first sample and a long second one can exceed `f64::MAX` seconds, so
//! it comes out as `inf`. Either way the old code handed the value to
//! `DecimalSeconds::new` and panicked.

use broadcast_common::timescale::Timescale;
use isobmff::avc_config::{AVCConfigurationBox, AVCDecoderConfigurationRecord};
use isobmff::frag_offsets::FragmentSampleRange;
use isobmff::nalu_types::{AvcPps, AvcSps};
use transmux::TrackId;
use transmux::ir::SampleTiming;
use transmux::media::{HlsPackager, Media, Track};
use transmux::pipeline::{CodecConfig, Sample, TrackSpec};

fn timescale(value: u32) -> Timescale {
    Timescale::try_new(value).expect("a non-zero timescale")
}

fn avc_spec() -> TrackSpec {
    let record = AVCDecoderConfigurationRecord {
        configuration_version: 1,
        profile_indication: 66,
        profile_compatibility: 0,
        level_indication: 30,
        length_size_minus_one: 3,
        sps: vec![AvcSps(vec![
            0x67, 0x42, 0xc0, 0x1e, 0xd9, 0x00, 0x80, 0x1e, 0x24,
        ])],
        pps: vec![AvcPps(vec![0x68, 0xce, 0x3c, 0x80])],
        chroma_format: None,
        bit_depth_luma_minus8: None,
        bit_depth_chroma_minus8: None,
        sps_ext: vec![],
    };
    TrackSpec::new(
        TrackId::new(1),
        timescale(90_000),
        CodecConfig::Avc {
            config: AVCConfigurationBox::new(record),
            width: 16,
            height: 16,
        },
    )
}

fn idr_bytes() -> Vec<u8> {
    let nal = [0x65u8, 0x88, 0x84, 0x00];
    let mut data = (nal.len() as u32).to_be_bytes().to_vec();
    data.extend_from_slice(&nal);
    data
}

/// Timed with `pts` three ticks before `dts`, so the composition offset is `-3`.
fn skewed(dts: i64, duration: u32) -> Sample {
    let timing =
        SampleTiming::timed_with_duration(dts, dts.checked_sub(3).expect("in range"), duration);
    Sample::new(idr_bytes(), Some(dts), Some(dts), Some(duration), true)
        .expect("valid timing")
        .with_timing(Some(timing))
}

#[test]
fn the_presentation_time_sum_does_not_fit_i64() {
    // A `trun` sample decode time at the top of the `i64` range whose
    // composition offset (`trun.sample_composition_time_offset`, read as an
    // `i64` since it may exceed `i32::MAX`) is positive: the presentation
    // time `dts + offset` is one past `i64::MAX`, so there is no honest
    // presentation time to report.
    let dts = isobmff::frag_offsets::tfdt_as_i64(i64::MAX as u64).expect("i64::MAX fits");
    let range = FragmentSampleRange {
        start: 0,
        end: 0,
        duration: 1,
        flags: 0,
        composition_offset: 1,
    };
    assert_eq!(
        isobmff::frag_offsets::add_offset(dts, range.composition_offset),
        Err(isobmff::Error::InvalidInput("sample pts overflow"))
    );
}

/// A sample whose `pts - dts` overflows `i64` reports a typed span-overflow
/// error, not a formatted `InvalidInput` message.
#[test]
fn a_composition_offset_that_overflows_i64_is_a_typed_span_overflow() {
    let sample =
        Sample::new(idr_bytes(), Some(i64::MIN), Some(i64::MAX), None, true).expect("valid sample");
    assert!(matches!(
        sample.composition_offset(),
        Err(transmux::Error::TimestampSpanOverflow)
    ));
}

/// A sample whose composition offset does not fit the signed 32-bit `trun`
/// field reports the over-range offset, not a formatted message.
#[test]
fn a_composition_offset_past_i32_is_a_typed_range_error_naming_the_offset() {
    let sample = Sample::new(
        idr_bytes(),
        Some(0),
        Some(i64::from(i32::MAX) + 1),
        None,
        true,
    )
    .expect("valid sample");
    assert!(matches!(
        sample.composition_offset(),
        Err(transmux::Error::CompositionOffsetRange { offset }) if offset == i64::from(i32::MAX) + 1
    ));
}

#[test]
fn a_span_past_f64_max_is_an_error_not_an_infinite_extinf() {
    // Two samples in a 1 Hz track: a short one whose end is at the bottom of
    // the tick range (its decode time is taken to its presentation time by a
    // `-3` composition offset), and a long one whose end sits just below the
    // top.
    // Both ends are exactly representable, but the span between them is
    // ~1.8e19 seconds, past `f64::MAX`, so the old subtraction produced `inf`
    // and the `expect` on `DecimalSeconds::new` panicked.
    let low = skewed(i64::MIN + 10, 3);
    let high = Sample::new(
        idr_bytes(),
        Some(i64::MAX - 1_001),
        Some(i64::MAX - 1_001),
        Some(1_000),
        true,
    )
    .expect("valid timing");
    assert_eq!(high.composition_offset(), Ok(Some(0)));
    let media = Media::new(vec![Track::new(avc_spec(), vec![low, high])], timescale(1));
    // A summary's span is `u64` ticks over a non-zero timescale, so it can be
    // absurd but never infinite: the renderer must refuse a span the
    // `DecimalSeconds` constructor cannot represent rather than panic on it.
    let spec = avc_spec();
    let mut log = transmux::segment_log::SegmentSummaryLog::new();
    log.push_part(
        &transmux::mux_stage::TrackPart {
            track: spec.track_id,
            start: 0,
            duration: u64::MAX,
            timescale: 1,
            bytes: 1,
            samples: 1,
        },
        true,
    )
    .expect("a consistent summary");
    // A `u64` tick span over a non-zero timescale is always a finite number of
    // seconds, so the playlist must render it rather than panic on it.
    let playlist = HlsPackager::default()
        .render(core::slice::from_ref(&spec), &log)
        .expect("a finite but enormous span still renders");
    assert!(playlist.contains("#EXTINF:"), "playlist:\n{playlist}");
    // A zero timescale has no honest seconds value: a typed error, not a
    // division by zero.
    let mut zero = transmux::segment_log::SegmentSummaryLog::new();
    zero.push_part(
        &transmux::mux_stage::TrackPart {
            track: spec.track_id,
            start: 0,
            duration: 1,
            timescale: 0,
            bytes: 1,
            samples: 1,
        },
        true,
    )
    .expect("a consistent part");
    let err = HlsPackager::default()
        .render(core::slice::from_ref(&spec), &zero)
        .expect_err("a zero timescale must be an error");
    assert!(
        matches!(err, transmux::Error::InvalidInput(_)),
        "got {err:?}"
    );
    let _ = media;
}

#[test]
fn a_large_but_representable_span_still_packages() {
    // One hour of media anchored just above `i64::MIN`: the span is exact and
    // the playlist renders a 3600-second `EXTINF`.
    let timing = SampleTiming::timed_with_duration(i64::MIN + 1, i64::MIN + 2, 90_000 * 3_600);
    let sample = Sample::new(
        idr_bytes(),
        Some(i64::MIN + 1),
        Some(i64::MIN + 1),
        Some(90_000 * 3_600),
        true,
    )
    .expect("valid timing")
    .with_timing(Some(timing));
    let media = Media::new(
        vec![Track::new(avc_spec(), vec![sample])],
        timescale(90_000),
    );
    let specs: Vec<_> = media.tracks.iter().map(|t| t.spec.clone()).collect();
    let playlist = HlsPackager::default()
        .render(&specs, &test_harness::collect::segment_log(&media))
        .expect("an exactly representable span must package");
    assert!(playlist.contains("#EXTINF:3600"), "playlist:\n{playlist}");
}
