//! The fragmented [`Reader`]: one `moof` at a time, no `mdat` payload byte
//! requested, every fragment's samples addressed from box headers alone.
//!
//! The expected sample ranges come from [`isobmff::frag_offsets`] given the
//! whole file's `mdat` ranges, so the header-only path and the whole-file path
//! must agree byte for byte.

use broadcast_common::Parse;
use isobmff::frag_offsets::{FragmentSampleRange, MdatRange, mdat_ranges, sample_ranges_in};
use isobmff::movie_fragment::MovieFragmentBox;
use isobmff::sample_table::SampleRef;
use isobmff::timing::{MovieFragmentRandomAccessBox, SegmentIndexBox};
use isobmff::{
    ByteRange, FragmentInfo, Limits, Reader, ReaderConfig, SampleOrder, Source, Step, TrackInfo,
};

/// Fragmented fixtures the walk must exercise: enough that a checkout with no
/// usable files fails rather than passing vacuously.
const MIN_FRAGMENTED_FIXTURES: usize = 6;

#[derive(Debug, Default)]
struct Outcome {
    tracks: Vec<TrackInfo>,
    fragments: Vec<FragmentInfo>,
    samples: Vec<SampleRef>,
    requests: Vec<ByteRange>,
}

fn drive(bytes: &[u8], config: ReaderConfig) -> isobmff::Result<Outcome> {
    let mut reader = Reader::new(config);
    let mut outcome = Outcome::default();
    loop {
        match reader.step()? {
            Step::Need(range) => {
                outcome.requests.push(range);
                let start = usize::try_from(range.offset).unwrap_or(usize::MAX);
                let end = usize::try_from(range.end()).unwrap_or(usize::MAX);
                reader.supply(range.offset, bytes.get(start..end).unwrap_or(&[]))?;
            }
            Step::Tracks(tracks) => outcome.tracks = tracks.to_vec(),
            Step::Fragment(info) => outcome.fragments.push(info),
            Step::Sample(sample) => outcome.samples.push(sample),
            Step::Done => break,
            _ => break,
        }
    }
    Ok(outcome)
}

fn config(bytes: &[u8], order: SampleOrder) -> ReaderConfig {
    ReaderConfig::default()
        .with_total_len(Some(bytes.len() as u64))
        .with_order(order)
}

/// One fragment: its `moof` offset and, per track it carries, the samples the
/// whole-file resolver finds.
type ExpectedFragment = (u64, Vec<(u32, Vec<FragmentSampleRange>)>);

/// One fragment's samples per track, resolved against the whole file the way a
/// demuxer does, in `(moof offset, sequence of (track id, ranges))` order.
fn expected(bytes: &[u8]) -> Vec<ExpectedFragment> {
    let mdats: Vec<MdatRange> = mdat_ranges(bytes);
    let budget = isobmff::frag_offsets::sample_bytes_budget(bytes.len() as u64);
    let mut out = Vec::new();
    for item in isobmff::BoxIter::clamped(bytes).filter_map(|b| b.ok()) {
        if !item.header.box_type.is(b"moof") {
            continue;
        }
        let Ok(moof) = MovieFragmentBox::parse_body(item.body) else {
            continue;
        };
        let mut per_track = Vec::new();
        for traf in &moof.traf {
            let Ok(ranges) =
                sample_ranges_in(item.start as u64, &moof, traf.tfhd.track_id, &mdats, budget)
            else {
                continue;
            };
            per_track.push((traf.tfhd.track_id.value(), ranges));
        }
        out.push((item.start as u64, per_track));
    }
    out
}

fn fragments_fixtures() -> Vec<(String, Vec<u8>)> {
    let mut out = Vec::new();
    let names = [
        "mp4/h264-aac-frag.mp4",
        "mp4/cmaf/av-frag.mp4",
        "mp4/cmaf-av-subtitle/av-subtitle-frag.mp4",
        "mp4/llcmaf/ll-chunked.mp4",
        "mp4/frag/h264-high.frag.mp4",
        "mp4/frag/av1.frag.mp4",
        "mp4/frag/opus.frag.mp4",
    ];
    for name in names {
        out.push((name.to_string(), test_fixtures::read(name)));
    }
    out.push((
        "hls/cmaf-fmp4/init.mp4+index0.m4s".to_string(),
        [
            test_fixtures::read("hls/cmaf-fmp4/init.mp4"),
            test_fixtures::read("hls/cmaf-fmp4/index0.m4s"),
        ]
        .concat(),
    ));
    out.push((
        "dash/init-stream0.m4s+chunks".to_string(),
        [
            test_fixtures::read("dash/init-stream0.m4s"),
            test_fixtures::read("dash/chunk-stream0-00001.m4s"),
            test_fixtures::read("dash/chunk-stream0-00002.m4s"),
        ]
        .concat(),
    ));
    out
}

#[test]
fn every_fragment_matches_the_whole_file_resolver() {
    let mut exercised = 0usize;
    for (key, bytes) in fragments_fixtures() {
        let Ok(outcome) = drive(&bytes, config(&bytes, SampleOrder::DecodeTime)) else {
            continue;
        };
        let expected = expected(&bytes);
        if expected.is_empty() || outcome.samples.is_empty() {
            continue;
        }
        exercised += 1;
        assert_eq!(
            outcome.fragments.len(),
            expected.len(),
            "{key}: one Fragment step per moof"
        );
        for (info, (offset, per_track)) in outcome.fragments.iter().zip(&expected) {
            assert_eq!(info.moof.offset, *offset, "{key}: moof offset");
            let named: Vec<u32> = info.tracks.iter().map(|id| id.value()).collect();
            for (track, _) in per_track {
                assert!(named.contains(track), "{key}: fragment names track {track}");
            }
        }

        // Per track, the reader's samples in decode order must be exactly the
        // whole-file resolver's ranges, fragment after fragment.
        for info in &outcome.tracks {
            let track = info.track_id.value();
            let got: Vec<&SampleRef> = outcome
                .samples
                .iter()
                .filter(|sample| sample.track.value() == track)
                .collect();
            let mut want: Vec<&FragmentSampleRange> = Vec::new();
            for (_, per_track) in &expected {
                for (id, ranges) in per_track {
                    if *id == track {
                        want.extend(ranges.iter());
                    }
                }
            }
            if want.is_empty() {
                assert!(
                    got.is_empty(),
                    "{key}: track {track} has no fragment samples"
                );
                continue;
            }
            assert_eq!(
                got.len(),
                want.len(),
                "{key}: track {track} sample count (decoded order)"
            );
            let mut previous_dts: Option<u64> = None;
            for (sample, range) in got.iter().zip(&want) {
                assert_eq!(sample.offset, range.start, "{key}: track {track} offset");
                assert_eq!(
                    sample.size,
                    u32::try_from(range.end - range.start).unwrap_or(u32::MAX),
                    "{key}: track {track} size"
                );
                assert_eq!(sample.duration, range.duration, "{key}: duration");
                assert_eq!(
                    i64::from(sample.composition_offset),
                    range.composition_offset,
                    "{key}: composition offset"
                );
                assert_eq!(
                    sample.sync,
                    range.flags & isobmff::movie_fragment::SAMPLE_FLAG_IS_NON_SYNC == 0,
                    "{key}: sync flag"
                );
                if let Some(previous) = previous_dts {
                    assert!(
                        sample.decode_time >= previous,
                        "{key}: track {track} decode time is non-decreasing"
                    );
                }
                previous_dts = Some(sample.decode_time);
            }
        }
    }
    assert!(
        exercised >= MIN_FRAGMENTED_FIXTURES,
        "only {exercised} fragmented fixtures exercised"
    );
}

#[test]
fn no_mdat_payload_byte_is_requested() {
    for (key, bytes) in fragments_fixtures() {
        let Ok(outcome) = drive(&bytes, config(&bytes, SampleOrder::File)) else {
            continue;
        };
        if outcome.samples.is_empty() {
            continue;
        }
        let payloads: Vec<(u64, u64)> = mdat_ranges(&bytes)
            .iter()
            .map(|range| (range.start(), range.end()))
            .collect();
        for request in &outcome.requests {
            for (start, end) in &payloads {
                assert!(
                    request.end() <= *start || request.offset >= *end,
                    "{key}: request {request:?} overlaps the mdat payload [{start}, {end})"
                );
            }
        }
    }
}

#[test]
fn file_order_is_ascending_offset_within_each_fragment() {
    for (key, bytes) in fragments_fixtures() {
        let Ok(outcome) = drive(&bytes, config(&bytes, SampleOrder::File)) else {
            continue;
        };
        let mut by_fragment: Vec<&SampleRef> = Vec::new();
        for (index, sample) in outcome.samples.iter().enumerate() {
            let _ = index;
            by_fragment.push(sample);
        }
        let mut previous: Option<u64> = None;
        for sample in by_fragment {
            if let Some(offset) = previous {
                assert!(sample.offset >= offset, "{key}: file order is ascending");
            }
            previous = Some(sample.offset);
        }
    }
}

#[test]
fn sample_indices_are_contiguous_per_track() {
    let bytes = test_fixtures::read("mp4/h264-aac-frag.mp4");
    let outcome = drive(&bytes, config(&bytes, SampleOrder::DecodeTime)).expect("drive");
    let mut next: std::collections::BTreeMap<u32, u32> = std::collections::BTreeMap::new();
    for sample in &outcome.samples {
        let expected_index = next.entry(sample.track.value()).or_insert(0);
        assert_eq!(sample.index, *expected_index, "index in decode order");
        *expected_index += 1;
    }
}

#[test]
fn a_moof_larger_than_the_limit_is_a_typed_error() {
    let bytes = test_fixtures::read("mp4/h264-aac-frag.mp4");
    let limits = Limits::default().with_max_fragment_bytes(16);
    let config = config(&bytes, SampleOrder::DecodeTime).with_limits(limits);
    let mut reader = Reader::new(config);
    let mut error = None;
    for _ in 0..1_000_000 {
        match reader.step() {
            Ok(Step::Need(range)) => {
                let start = usize::try_from(range.offset).unwrap_or(usize::MAX);
                let end = usize::try_from(range.end()).unwrap_or(usize::MAX);
                if let Err(err) = reader.supply(range.offset, bytes.get(start..end).unwrap_or(&[]))
                {
                    error = Some(err);
                    break;
                }
            }
            Err(err) => {
                error = Some(err);
                break;
            }
            Ok(Step::Done) => break,
            Ok(_) => {}
        }
    }
    assert!(
        matches!(
            error,
            Some(isobmff::Error::LimitExceeded {
                what: "fragment size",
                ..
            })
        ),
        "a fragment past the cap is a typed limit error, got {error:?}"
    );
}

#[test]
fn a_truncated_final_fragment_is_a_typed_error() {
    let bytes = test_fixtures::read("mp4/h264-aac-frag.mp4");
    let outcome = drive(&bytes, config(&bytes, SampleOrder::DecodeTime)).expect("full drive");
    let first_moof = outcome.fragments[0].moof;
    let cut = usize::try_from(first_moof.offset + first_moof.len).expect("offset") + 8;
    let truncated = &bytes[..cut];
    assert!(
        drive(truncated, config(truncated, SampleOrder::DecodeTime)).is_err(),
        "a fragment whose mdat is cut short must fail, not silently yield fewer samples"
    );
}

/// Drive to [`Step::Tracks`], seek, and return the first material step after
/// the seek.
fn drive_seek(
    bytes: &[u8],
    track: u32,
    decode_time: u64,
    source: Source,
) -> isobmff::Result<FragmentInfo> {
    let config = ReaderConfig::default()
        .with_total_len(Some(bytes.len() as u64))
        .with_order(SampleOrder::DecodeTime)
        .with_source(source);
    let mut reader = Reader::new(config);
    let mut sought = false;
    let mut tracks = 0usize;
    for _ in 0..1_000_000 {
        let mut seek_now = false;
        match reader.step()? {
            Step::Need(range) => {
                let start = usize::try_from(range.offset).unwrap_or(usize::MAX);
                let end = usize::try_from(range.end()).unwrap_or(usize::MAX);
                reader.supply(range.offset, bytes.get(start..end).unwrap_or(&[]))?;
            }
            Step::Tracks(_) => {
                tracks += 1;
                seek_now = tracks == 1;
            }
            Step::Fragment(info) => {
                if sought {
                    return Ok(info);
                }
            }
            Step::Sample(_) => {
                if sought {
                    return Err(isobmff::Error::InvalidInput(
                        "a sample arrived before the fragment the seek named",
                    ));
                }
            }
            Step::Done => break,
            _ => break,
        }
        if seek_now {
            reader.seek_to(isobmff::TrackId::new(track), decode_time)?;
            sought = true;
        }
    }
    Err(isobmff::Error::InvalidInput(
        "no fragment after the seek, or the seek never ran",
    ))
}

/// The offset of every top-level `moof`, in file order.
fn moof_offsets(bytes: &[u8]) -> Vec<u64> {
    isobmff::BoxIter::clamped(bytes)
        .filter_map(|b| b.ok())
        .filter(|b| b.header.box_type.is(b"moof"))
        .map(|b| b.start as u64)
        .collect()
}

#[test]
fn seek_through_sidx_lands_on_the_fragment_it_names() {
    for name in [
        "dash/ondemand-sidx/v_dashinit.mp4",
        "dash/ondemand-sidx/a_dashinit.mp4",
    ] {
        let bytes = test_fixtures::read(name);
        let sidx = isobmff::BoxIter::clamped(&bytes)
            .filter_map(|b| b.ok())
            .find(|b| b.header.box_type.is(b"sidx"))
            .expect("the fixture carries a sidx");
        let index = SegmentIndexBox::parse(sidx.bytes).expect("sidx parses");
        let moofs = moof_offsets(&bytes);
        let mut time = index.earliest_presentation_time;
        for (slot, reference) in index.references.iter().enumerate() {
            // The time inside this subsegment, in the index's timescale.
            let probe = time + u64::from(reference.subsegment_duration) / 2;
            let info = drive_seek(&bytes, index.reference_id, probe, Source::Seekable)
                .unwrap_or_else(|err| panic!("{name}: seek to {probe}: {err}"));
            assert_eq!(
                info.moof.offset, moofs[slot],
                "{name}: subsegment {slot} is fragment {slot}"
            );
            time += u64::from(reference.subsegment_duration);
        }
    }
}

#[test]
fn seek_through_sidx_works_over_a_forward_only_source() {
    let bytes = test_fixtures::read("dash/ondemand-sidx/v_dashinit.mp4");
    let moofs = moof_offsets(&bytes);
    let info = drive_seek(&bytes, 1, 1_000, Source::ForwardOnly).expect("forward seek");
    assert_eq!(info.moof.offset, moofs[0]);
}

#[test]
fn seek_through_mfra_lands_on_the_listed_moof() {
    let bytes = test_fixtures::read("mp4/h264-aac-frag.mp4");
    let moofs = moof_offsets(&bytes);
    let mfra = isobmff::BoxIter::clamped(&bytes)
        .filter_map(|b| b.ok())
        .find(|b| b.header.box_type.is(b"mfra"))
        .expect("the fixture carries an mfra");
    let parsed = MovieFragmentRandomAccessBox::parse(mfra.bytes).expect("mfra parses");
    let mut checked = 0usize;
    for tfra in &parsed.tfra {
        if tfra.entries.len() < 2 {
            continue;
        }
        for entry in &tfra.entries {
            let info = drive_seek(&bytes, tfra.track_id.value(), entry.time, Source::Seekable)
                .unwrap_or_else(|err| panic!("seek to {}: {err}", entry.time));
            assert_eq!(
                info.moof.offset, entry.moof_offset,
                "the entry's own time lands on its fragment"
            );
            assert!(moofs.contains(&info.moof.offset));
            checked += 1;
        }
        // A time between two listed entries lands on the earlier one.
        let first = tfra.entries[0];
        let second = tfra.entries[1];
        if second.time > first.time + 1 {
            let probe = first.time + 1;
            let info = drive_seek(&bytes, tfra.track_id.value(), probe, Source::Seekable)
                .expect("seek between entries");
            assert_eq!(
                info.moof.offset, first.moof_offset,
                "the earlier entry wins"
            );
            checked += 1;
        }
    }
    assert!(
        checked >= 2,
        "the mfra seek was exercised {checked} time(s)"
    );
}

#[test]
fn a_file_with_no_index_reports_no_index() {
    let bytes = test_fixtures::read("mp4/cmaf/av-frag.mp4");
    let mfra = isobmff::BoxIter::clamped(&bytes)
        .filter_map(|b| b.ok())
        .find(|b| b.header.box_type.is(b"mfra"))
        .expect("the fixture carries an mfra");
    let truncated = &bytes[..mfra.start];
    let config = ReaderConfig::default()
        .with_total_len(Some(truncated.len() as u64))
        .with_order(SampleOrder::DecodeTime);
    let mut reader = Reader::new(config);
    let mut seek_error = None;
    for _ in 0..100_000 {
        match reader.step() {
            Ok(Step::Need(range)) => {
                let start = usize::try_from(range.offset).unwrap_or(usize::MAX);
                let end = usize::try_from(range.end()).unwrap_or(usize::MAX);
                if let Err(err) =
                    reader.supply(range.offset, truncated.get(start..end).unwrap_or(&[]))
                {
                    seek_error = Some(err);
                    break;
                }
            }
            Ok(Step::Tracks(tracks)) => {
                let track = tracks[0].track_id;
                if let Err(err) = reader.seek_to(track, 0) {
                    seek_error = Some(err);
                    break;
                }
            }
            Ok(Step::Done) => break,
            Err(err) => {
                seek_error = Some(err);
                break;
            }
            Ok(_) => {}
        }
    }
    assert!(
        matches!(seek_error, Some(isobmff::Error::NoIndex { .. })),
        "a file with neither sidx nor mfra is NoIndex, got {seek_error:?}"
    );
}
