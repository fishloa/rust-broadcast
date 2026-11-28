//! The fragmented [`Reader`]: one `moof` at a time, no `mdat` payload byte
//! requested, every fragment's samples addressed from box headers alone.
//!
//! The expected sample ranges come from [`isobmff::frag_offsets`] given the
//! whole file's `mdat` ranges, so the header-only path and the whole-file path
//! must agree byte for byte.

use std::collections::BTreeMap;

use broadcast_core::Parse;
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
    damages: Vec<broadcast_core::damage::Damage>,
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
            Step::Metadata(_) => {}
            Step::Fragment(info) => outcome.fragments.push(info),
            Step::Sample(sample) => outcome.samples.push(sample),
            Step::Damage(damage) => outcome.damages.push(damage),
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
        "mp4/h264-aac-frag.mp4",
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
fn a_cut_fragment_plays_what_is_complete_and_reports_the_rest() {
    let bytes = test_fixtures::read("mp4/h264-aac-frag.mp4");
    let outcome = drive(&bytes, config(&bytes, SampleOrder::DecodeTime)).expect("full drive");
    assert!(!outcome.samples.is_empty(), "the clean file has samples");
    let first_moof = outcome.fragments[0].moof;
    let mdat = usize::try_from(first_moof.offset + first_moof.len).expect("offset");
    // The `mdat` header, then half its payload.
    let cut = mdat + 8 + (bytes.len() - mdat - 8) / 8;
    let truncated = &bytes[..cut];
    let strict = config(truncated, SampleOrder::DecodeTime).with_mode(Mode::Strict);
    assert!(
        drive(truncated, strict).is_err(),
        "strict refuses a fragment whose mdat is cut short"
    );
    let lenient = drive(truncated, config(truncated, SampleOrder::DecodeTime))
        .expect("a best-effort read carries the complete samples");
    assert!(
        lenient
            .damages
            .iter()
            .any(|damage| damage.kind() == broadcast_core::damage::DamageKind::InputTruncated),
        "the cut tail is reported: {:?}",
        lenient.damages
    );
    let cut64 = u64::try_from(cut).expect("cut");
    let expected: Vec<_> = outcome
        .samples
        .iter()
        .filter(|sample| u64::from(sample.size).saturating_add(sample.offset) <= cut64)
        .map(|sample| {
            (
                sample.track,
                sample.index,
                sample.decode_time,
                sample.offset,
            )
        })
        .collect();
    let got: Vec<_> = lenient
        .samples
        .iter()
        .map(|sample| {
            (
                sample.track,
                sample.index,
                sample.decode_time,
                sample.offset,
            )
        })
        .collect();
    assert_eq!(got, expected, "every whole sample inside the cut plays");
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
            Step::Metadata(_) => {}
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
    let bytes = test_fixtures::read("mp4/h264-aac-frag.mp4");
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

/// One event of the walk, in the order the reader produced it.
#[derive(Debug, Clone, Copy)]
enum Event {
    Need(ByteRange),
    Fragment,
    Sample,
    Other,
}

/// Drive `bytes`, recording every event in order.
fn record(bytes: &[u8], order: SampleOrder) -> Vec<Event> {
    let mut reader = Reader::new(config(bytes, order));
    let mut events = Vec::new();
    for _ in 0..1_000_000 {
        match reader.step() {
            Ok(Step::Need(range)) => {
                events.push(Event::Need(range));
                let start = usize::try_from(range.offset).unwrap_or(usize::MAX);
                let end = usize::try_from(range.end()).unwrap_or(usize::MAX);
                if reader
                    .supply(range.offset, bytes.get(start..end).unwrap_or(&[]))
                    .is_err()
                {
                    break;
                }
            }
            Ok(Step::Fragment(_)) => events.push(Event::Fragment),
            Ok(Step::Sample(_)) => events.push(Event::Sample),
            Ok(Step::Done) | Err(_) => break,
            Ok(_) => events.push(Event::Other),
        }
    }
    events
}

/// Every top-level `mdat`'s box start and payload start.
fn mdat_starts(bytes: &[u8]) -> Vec<(u64, u64)> {
    isobmff::BoxIter::clamped(bytes)
        .filter_map(|b| b.ok())
        .filter(|b| b.header.box_type.is(b"mdat"))
        .map(|b| (b.start as u64, b.payload_start() as u64))
        .collect()
}

#[test]
fn a_fragment_is_emitted_before_the_walk_leaves_its_mdat() {
    let mut exercised = 0usize;
    for (key, bytes) in fragments_fixtures() {
        let events = record(&bytes, SampleOrder::DecodeTime);
        if !events.iter().any(|event| matches!(event, Event::Fragment)) {
            continue;
        }
        let mdats = mdat_starts(&bytes);
        let mut fragments = 0usize;
        for (index, event) in events.iter().enumerate() {
            if !matches!(event, Event::Fragment) {
                continue;
            }
            let Some((anchor_index, anchor)) =
                events[..index]
                    .iter()
                    .enumerate()
                    .rev()
                    .find_map(|(at, earlier)| match earlier {
                        Event::Need(range) => Some((at, *range)),
                        _ => None,
                    })
            else {
                continue;
            };
            let Some((_, payload_start)) = mdats
                .iter()
                .find(|(box_start, _)| *box_start == anchor.offset)
            else {
                panic!(
                    "{key}: fragment {fragments} was emitted after a need at {} that is not an \
                     mdat header: the mdat payload was already passed",
                    anchor.offset
                );
            };
            let last_sample = events[index + 1..]
                .iter()
                .position(|later| !matches!(later, Event::Sample))
                .map_or(events.len() - 1, |offset| index + offset);
            for later in &events[anchor_index + 1..=last_sample] {
                if let Event::Need(range) = later {
                    assert!(
                        range.offset < *payload_start,
                        "{key}: fragment {fragments} asked for {} before its samples, past the \
                         mdat payload start {payload_start}",
                        range.offset
                    );
                }
            }
            fragments += 1;
        }
        if fragments > 0 {
            exercised += 1;
        }
    }
    assert!(
        exercised >= MIN_FRAGMENTED_FIXTURES,
        "only {exercised} fragmented fixtures exercised"
    );
}

/// The absolute offset of the first `trun`'s `data_offset` field inside `moof`.
fn first_trun_data_offset(moof: &isobmff::BoxItem<'_>) -> Option<usize> {
    const DATA_OFFSET_PRESENT: u32 = 1;
    const TRUN_FIXED_BODY: usize = 8;
    let moof_body = moof.start + (moof.bytes.len() - moof.body.len());
    for traf in moof.children().filter_map(|child| child.ok()) {
        if traf.four_cc() != *b"traf" {
            continue;
        }
        let traf_body = moof_body + traf.start + (traf.bytes.len() - traf.body.len());
        for trun in traf.children().filter_map(|child| child.ok()) {
            if trun.four_cc() != *b"trun" {
                continue;
            }
            let flags = broadcast_core::wire::u32_at(trun.body, 0) & 0x00ff_ffff;
            if flags & DATA_OFFSET_PRESENT == 0 {
                return None;
            }
            let header = trun.bytes.len() - trun.body.len();
            return Some(traf_body + trun.start + header + TRUN_FIXED_BODY);
        }
    }
    None
}

#[test]
fn a_run_pointing_into_the_second_mdat_after_the_moof_still_resolves() {
    const PAYLOAD_LEN: usize = 16;
    let bytes = test_fixtures::read("mp4/frag/h264-high.frag.mp4");
    let moof = isobmff::BoxIter::clamped(&bytes)
        .filter_map(|b| b.ok())
        .find(|b| b.header.box_type.is(b"moof"))
        .expect("the fixture has a moof");
    let mdat = isobmff::BoxIter::clamped(&bytes)
        .filter_map(|b| b.ok())
        .find(|b| b.header.box_type.is(b"mdat"))
        .expect("the fixture has an mdat");
    let field = first_trun_data_offset(&moof).expect("the fixture's trun carries a data_offset");
    let payload_start = mdat.payload_start();

    // The decoy `mdat` sits between the `moof` and the media the run addresses,
    // so a reader that stops resolving at the first `mdat` after the `moof`
    // cannot address the samples at all.
    let decoy_size = 8 + PAYLOAD_LEN;
    let mut file = bytes[..moof.end].to_vec();
    let data_offset = i32::from_be_bytes(
        file[field..field + 4]
            .try_into()
            .expect("four bytes of data_offset"),
    );
    let shifted = data_offset + i32::try_from(decoy_size).expect("decoy size");
    file[field..field + 4].copy_from_slice(&shifted.to_be_bytes());
    assert_eq!(
        i32::from_be_bytes(file[field..field + 4].try_into().expect("four bytes")),
        shifted
    );
    let mut decoy = Vec::new();
    decoy.extend_from_slice(&u32::try_from(decoy_size).expect("size").to_be_bytes());
    decoy.extend_from_slice(b"mdat");
    decoy.resize(decoy_size, 0);
    file.extend_from_slice(&decoy);
    file.extend_from_slice(&bytes[mdat.start..]);
    let shifted_payload =
        u64::try_from(payload_start + decoy_size).expect("the shifted payload start");

    let outcome = drive(&file, config(&file, SampleOrder::File)).expect("drive the synthetic file");
    assert!(!outcome.samples.is_empty(), "the fragments carry samples");
    for sample in &outcome.samples {
        let start = usize::try_from(sample.offset).expect("offset");
        assert!(
            sample.offset >= shifted_payload,
            "sample at {} is not in the second mdat (payload starts {shifted_payload})",
            sample.offset
        );
        let end = start + usize::try_from(sample.size).expect("size");
        let original = usize::try_from(sample.offset - u64::try_from(decoy_size).expect("decoy"))
            .expect("original offset");
        assert_eq!(
            &file[start..end],
            &bytes[original..original + (end - start)],
            "the sample bytes come from the media the run addresses"
        );
    }
    let expected = expected(&file);
    assert_eq!(outcome.fragments.len(), expected.len());
}

// ---------------------------------------------------------------------------
// Sequential units: an init segment followed by media segments, each closed by
// `Reader::end_of_unit`. A damaged segment must yield its complete prefix, one
// a truncated-fragment report, and leave the next segment readable.
// ---------------------------------------------------------------------------

use isobmff::reader::Mode;

/// One damaged media segment, built from a well-formed one in the test.
enum Damage {
    TruncateHalf,
    MoofSizePastSegment,
    TruncateInsideMdat,
}

impl Damage {
    fn name(&self) -> &'static str {
        match self {
            Self::TruncateHalf => "truncate-half",
            Self::MoofSizePastSegment => "moof-size-past-segment",
            Self::TruncateInsideMdat => "truncate-inside-mdat",
        }
    }
}

/// `(start, size, header size, four-CC)` for each top-level box of `bytes`.
fn top_boxes(bytes: &[u8]) -> Vec<(usize, usize, usize, [u8; 4])> {
    let mut out = Vec::new();
    let mut at = 0usize;
    while at + 8 <= bytes.len() {
        let raw = u32::from_be_bytes([bytes[at], bytes[at + 1], bytes[at + 2], bytes[at + 3]]);
        let size = usize::try_from(raw).unwrap_or(usize::MAX);
        let fourcc: [u8; 4] = bytes[at + 4..at + 8].try_into().expect("four-CC");
        let (header, total) = match size {
            0 => (8usize, bytes.len() - at),
            1 => {
                if at + 16 > bytes.len() {
                    break;
                }
                let mut large = [0u8; 8];
                large.copy_from_slice(&bytes[at + 8..at + 16]);
                (
                    16usize,
                    usize::try_from(u64::from_be_bytes(large)).unwrap_or(usize::MAX),
                )
            }
            value => (8usize, value),
        };
        if total < header || total > bytes.len() - at {
            out.push((at, bytes.len() - at, header, fourcc));
            break;
        }
        out.push((at, total, header, fourcc));
        at += total;
    }
    out
}

fn box_of(bytes: &[u8], fourcc: &[u8; 4]) -> (usize, usize, usize) {
    for (start, size, header, ty) in top_boxes(bytes) {
        if ty == *fourcc {
            return (start, size, header);
        }
    }
    panic!("no {fourcc:?} box in the segment")
}

fn damage(segment: &[u8], damage: &Damage) -> Vec<u8> {
    match damage {
        Damage::TruncateHalf => segment[..segment.len() / 2].to_vec(),
        Damage::MoofSizePastSegment => {
            let mut damaged = segment.to_vec();
            let (start, _, _) = box_of(segment, b"moof");
            let beyond = u32::try_from(segment.len() + 1).expect("segment fits u32");
            damaged[start..start + 4].copy_from_slice(&beyond.to_be_bytes());
            damaged
        }
        Damage::TruncateInsideMdat => {
            let mut damaged = segment.to_vec();
            let (start, size, header) = box_of(segment, b"mdat");
            let payload = size - header;
            let cut = start + header + payload / 2;
            damaged.truncate(cut);
            damaged
        }
    }
}

/// The result of driving one sequential input unit by unit.
struct Units {
    samples: Vec<Vec<SampleRef>>,
    warnings: Vec<broadcast_core::damage::Damage>,
    error: Option<isobmff::Error>,
}

impl Units {
    /// The samples of `unit` whose bytes lie wholly inside that unit.
    fn complete(&self, unit: usize, end: usize) -> Vec<&SampleRef> {
        self.samples[unit]
            .iter()
            .filter(|sample| {
                sample.offset.saturating_add(u64::from(sample.size))
                    <= u64::try_from(end).unwrap_or(u64::MAX)
            })
            .collect()
    }
}

fn drive_units(whole: &[u8], units: &[(usize, usize)], config: ReaderConfig) -> Units {
    let mut reader = Reader::new(config);
    let mut outcome = Units {
        samples: Vec::new(),
        warnings: Vec::new(),
        error: None,
    };
    for &(_, end) in units {
        outcome.samples.push(Vec::new());
        loop {
            match reader.step() {
                Ok(Step::Need(range)) => {
                    if range.end() > u64::try_from(end).unwrap_or(u64::MAX) {
                        break;
                    }
                    let start = usize::try_from(range.offset).unwrap_or(usize::MAX);
                    let stop = usize::try_from(range.end()).unwrap_or(usize::MAX);
                    let supplied = whole.get(start..stop).unwrap_or(&[]);
                    if let Err(error) = reader.supply(range.offset, supplied) {
                        outcome.error = Some(error);
                        return outcome;
                    }
                }
                Ok(Step::Sample(sample)) => {
                    if let Some(unit) = outcome.samples.last_mut() {
                        unit.push(sample);
                    }
                }
                Ok(Step::Damage(warning)) => outcome.warnings.push(warning),
                Ok(Step::Done) => return outcome,
                Ok(_) => {}
                Err(error) => {
                    outcome.error = Some(error);
                    return outcome;
                }
            }
        }
        if let Err(error) = reader.end_of_unit(u64::try_from(end).unwrap_or(u64::MAX)) {
            outcome.error = Some(error);
            return outcome;
        }
        loop {
            match reader.step() {
                Ok(Step::Damage(warning)) => outcome.warnings.push(warning),
                Ok(Step::Sample(sample)) => {
                    if let Some(unit) = outcome.samples.last_mut() {
                        unit.push(sample);
                    }
                }
                Ok(Step::Need(_)) | Ok(Step::Done) => break,
                Ok(_) => {}
                Err(error) => {
                    outcome.error = Some(error);
                    return outcome;
                }
            }
        }
    }
    outcome
}

const UNIT_INIT: &str = "hls/cmaf-fmp4/init.mp4";
const UNIT_SEGMENTS: [&str; 2] = ["hls/cmaf-fmp4/index0.m4s", "hls/cmaf-fmp4/index1.m4s"];

fn unit_config() -> ReaderConfig {
    ReaderConfig::default()
        .with_order(SampleOrder::File)
        .with_source(Source::ForwardOnly)
}

fn unit_config_mode(mode: Mode) -> ReaderConfig {
    unit_config().with_mode(mode)
}

#[test]
fn end_of_unit_recovers_after_each_damage() {
    let init = test_fixtures::read(UNIT_INIT);
    let segments: Vec<Vec<u8>> = UNIT_SEGMENTS
        .iter()
        .map(|name| test_fixtures::read(name))
        .collect();

    let whole: Vec<u8> = [&init[..], &segments.concat()[..]].concat();
    let mut clean_units = vec![(0usize, init.len())];
    let mut at = init.len();
    for segment in &segments {
        clean_units.push((at, at + segment.len()));
        at += segment.len();
    }
    let clean = drive_units(&whole, &clean_units, unit_config_mode(Mode::Permissive));
    assert!(clean.error.is_none(), "a clean input: {:?}", clean.error);
    assert!(
        clean.warnings.is_empty(),
        "a clean input warns about nothing"
    );

    for victim in 0..segments.len() {
        for harm in [
            Damage::TruncateHalf,
            Damage::MoofSizePastSegment,
            Damage::TruncateInsideMdat,
        ] {
            let damaged = damage(&segments[victim], &harm);
            let mut damaged_segments = segments.clone();
            damaged_segments[victim] = damaged;
            let damaged_whole: Vec<u8> = [&init[..], &damaged_segments.concat()[..]].concat();
            let mut units = vec![(0usize, init.len())];
            let mut at = init.len();
            for segment in &damaged_segments {
                units.push((at, at + segment.len()));
                at += segment.len();
            }

            let permissive =
                drive_units(&damaged_whole, &units, unit_config_mode(Mode::Permissive));
            assert!(
                permissive.error.is_none(),
                "segment {victim} {}: permissive must recover, got {:?}",
                harm.name(),
                permissive.error
            );
            assert!(
                permissive
                    .warnings
                    .iter()
                    .any(|damage| damage.kind()
                        == broadcast_core::damage::DamageKind::InputTruncated),
                "segment {victim} {}: a truncated-fragment warning",
                harm.name()
            );
            for unit in 0..clean.samples.len() {
                if unit == victim + 1 {
                    continue;
                }
                let want: Vec<_> = clean.samples[unit]
                    .iter()
                    .map(|sample| sample_key(sample, clean_units[unit].0))
                    .collect();
                let got: Vec<_> = permissive.samples[unit]
                    .iter()
                    .map(|sample| sample_key(sample, units[unit].0))
                    .collect();
                assert_eq!(
                    got,
                    want,
                    "segment {victim} {}: unit {unit} must be complete",
                    harm.name()
                );
            }
            let end = units[victim + 1].1;
            let kept = permissive.complete(victim + 1, end);
            match harm {
                Damage::MoofSizePastSegment => assert!(
                    kept.is_empty(),
                    "segment {victim}: an unreadable moof yields no samples"
                ),
                Damage::TruncateHalf => {
                    for sample in &kept {
                        assert!(
                            sample.offset.saturating_add(u64::from(sample.size))
                                <= u64::try_from(end).unwrap_or(u64::MAX),
                            "a kept sample lies wholly inside the segment"
                        );
                    }
                    let clean_keys: Vec<_> = clean.samples[victim + 1]
                        .iter()
                        .map(|sample| sample_key(sample, clean_units[victim + 1].0))
                        .collect();
                    let kept_keys: Vec<_> = kept
                        .iter()
                        .map(|sample| sample_key(sample, units[victim + 1].0))
                        .collect();
                    assert!(
                        clean_keys.starts_with(&kept_keys),
                        "segment {victim}: kept samples are a prefix of the clean ones"
                    );
                }
                Damage::TruncateInsideMdat => {
                    let clean_keys: Vec<_> = clean
                        .complete(victim + 1, end)
                        .iter()
                        .map(|sample| sample_key(sample, clean_units[victim + 1].0))
                        .collect();
                    let kept_keys: Vec<_> = kept
                        .iter()
                        .map(|sample| sample_key(sample, units[victim + 1].0))
                        .collect();
                    assert_eq!(
                        kept_keys, clean_keys,
                        "segment {victim}: every sample wholly inside the cut mdat is kept"
                    );
                }
            }

            let strict = drive_units(&damaged_whole, &units, unit_config_mode(Mode::Strict));
            assert!(
                strict.error.is_some(),
                "segment {victim} {}: strict refuses a truncated unit",
                harm.name()
            );
        }
    }
}

/// The fields a sample comparison must agree on. The offset is relative to the
/// unit, because a damaged unit is shorter and every later unit shifts.
fn sample_key(sample: &SampleRef, unit_start: usize) -> (u32, u64, u32, u64, i32, u32, bool) {
    (
        sample.track.value(),
        sample
            .offset
            .saturating_sub(u64::try_from(unit_start).unwrap_or(0)),
        sample.size,
        sample.decode_time,
        sample.composition_offset,
        sample.duration,
        sample.sync,
    )
}

/// The first `moof` box, its `mdat`, and the prefix before it.
fn first_fragment(bytes: &[u8]) -> Option<(usize, usize, MovieFragmentBox, usize, usize)> {
    let boxes: Vec<_> = isobmff::BoxIter::new(bytes).flatten().collect();
    let moof_index = boxes.iter().position(|item| item.four_cc() == *b"moof")?;
    let moof = &boxes[moof_index];
    let mdat = boxes[moof_index + 1..]
        .iter()
        .find(|item| item.four_cc() == *b"mdat")?;
    let parsed = MovieFragmentBox::parse_body(moof.body).ok()?;
    Some((
        moof.start,
        moof.end - moof.start,
        parsed,
        mdat.start,
        mdat.end,
    ))
}

/// Rewrite the first fragment of `bytes` into the CMF2 shape: one `moof` per
/// track, each with an absolute `base_data_offset`, followed by one `mdat`
/// holding both tracks' data (ISO/IEC 14496-12:2026 §8.8.4).
///
/// Returns the new file and the first rewritten `moof`'s length.
fn cmf2_shape(bytes: &[u8]) -> Option<(Vec<u8>, u64)> {
    use isobmff::movie_fragment::{
        TFHD_BASE_DATA_OFFSET_PRESENT, TFHD_DEFAULT_BASE_IS_MOOF, TRUN_DATA_OFFSET_PRESENT,
    };
    let (moof_start, _moof_len, moof, mdat_start, mdat_end) = first_fragment(bytes)?;
    let payload = mdat_start + 8;
    let mdats = [MdatRange::new(payload as u64, mdat_end as u64)];
    let budget = isobmff::frag_offsets::sample_bytes_budget(bytes.len() as u64);

    let mut pieces = Vec::new();
    for (index, traf) in moof.traf.iter().enumerate() {
        let ranges =
            sample_ranges_in(moof_start as u64, &moof, traf.tfhd.track_id, &mdats, budget).ok()?;
        let first = ranges.first()?.start;
        let first_rel = first.checked_sub(payload as u64)?;
        let mut piece = MovieFragmentBox::clone(&moof);
        piece.mfhd.sequence_number =
            u32::try_from(index + 1).expect("a small track index fits mfhd");
        piece.traf = vec![isobmff::movie_fragment::TrackFragmentBox::clone(traf)];
        let only = &mut piece.traf[0];
        only.tfhd.flags &= !TFHD_DEFAULT_BASE_IS_MOOF;
        only.tfhd.flags |= TFHD_BASE_DATA_OFFSET_PRESENT;
        only.tfhd.base_data_offset = Some(0);
        for trun in &mut only.trun {
            trun.tr_flags |= TRUN_DATA_OFFSET_PRESENT;
            trun.data_offset = Some(i32::try_from(first_rel).ok()?);
        }
        let mut serialized = vec![0u8; broadcast_core::Serialize::serialized_len(&piece)];
        broadcast_core::Serialize::serialize_into(&piece, &mut serialized).ok()?;
        pieces.push(serialized);
    }
    let prefix_len = moof_start;
    let payload_start = prefix_len
        .checked_add(pieces.iter().map(Vec::len).sum::<usize>())?
        .checked_add(8)?;
    let mut out = bytes[..moof_start].to_vec();
    let mut first_len = None;
    for mut piece in pieces {
        // Both tracks address their bytes in the one `mdat` that follows.
        let mut bytes_out = vec![0u8; piece.len()];
        bytes_out.copy_from_slice(&piece);
        let base = payload_start as u64;
        let mut reparsed = MovieFragmentBox::parse_body(&bytes_out[8..]).ok()?;
        for traf in &mut reparsed.traf {
            traf.tfhd.base_data_offset = Some(base);
        }
        let mut serialized = vec![0u8; broadcast_core::Serialize::serialized_len(&reparsed)];
        broadcast_core::Serialize::serialize_into(&reparsed, &mut serialized).ok()?;
        if first_len.is_none() {
            first_len = Some(u64::try_from(serialized.len()).ok()?);
        }
        out.extend_from_slice(&serialized);
        piece.clear();
    }
    out.extend_from_slice(&bytes[mdat_start..mdat_end]);
    Some((out, first_len?))
}

fn sample_bytes_from_file(bytes: &[u8]) -> Vec<(u32, Vec<u8>)> {
    use isobmff::driver::{DriverStep, FileReader};
    use std::io::Cursor;
    let mut reader = FileReader::new(
        Cursor::new(bytes.to_vec()),
        config(bytes, SampleOrder::File),
    )
    .expect("file reader");
    let mut out = Vec::new();
    loop {
        match reader.step().expect("step") {
            DriverStep::Sample(sample) => {
                let mut data = Vec::new();
                reader
                    .read_sample(&sample, &mut data)
                    .expect("sample bytes");
                out.push((sample.track.value(), data));
            }
            DriverStep::Done => break,
            _ => {}
        }
    }
    out
}

fn sample_bytes_from_forward(bytes: &[u8]) -> Vec<(u32, Vec<u8>)> {
    use isobmff::driver::{DriverStep, ForwardReader};
    let mut reader = ForwardReader::new(bytes, config(bytes, SampleOrder::File));
    let mut out = Vec::new();
    loop {
        match reader.step().expect("step") {
            DriverStep::Sample(sample) => {
                let mut data = Vec::new();
                reader
                    .read_sample(&sample, &mut data)
                    .expect("sample bytes");
                out.push((sample.track.value(), data));
            }
            DriverStep::Done => break,
            _ => {}
        }
    }
    out
}

#[test]
fn cmf2_shape_keeps_every_queued_moof_on_both_drivers() {
    let bytes = test_fixtures::read("mp4/h264-aac-frag.mp4");
    let (moof_start, _, moof, mdat_start, mdat_end) = first_fragment(&bytes).expect("fragment");
    let payload = mdat_start + 8;
    let budget = isobmff::frag_offsets::sample_bytes_budget(bytes.len() as u64);
    let mdats = [MdatRange::new(payload as u64, mdat_end as u64)];
    let mut expected: BTreeMap<u32, Vec<Vec<u8>>> = BTreeMap::new();
    for traf in &moof.traf {
        let ranges = sample_ranges_in(moof_start as u64, &moof, traf.tfhd.track_id, &mdats, budget)
            .expect("original ranges");
        expected.insert(
            traf.tfhd.track_id.value(),
            ranges
                .iter()
                .map(|range| bytes[range.start as usize..range.end as usize].to_vec())
                .collect(),
        );
    }
    assert_eq!(
        expected.len(),
        2,
        "the fixture's first fragment has two tracks"
    );

    let (rewritten, _) = cmf2_shape(&bytes).expect("CMF2 rewrite");
    for (name, got) in [
        ("FileReader", sample_bytes_from_file(&rewritten)),
        ("ForwardReader", sample_bytes_from_forward(&rewritten)),
    ] {
        let mut by_track: BTreeMap<u32, Vec<Vec<u8>>> = BTreeMap::new();
        for (track, data) in got {
            by_track.entry(track).or_default().push(data);
        }
        for (track, want) in &expected {
            assert_eq!(
                by_track
                    .get(track)
                    .unwrap_or_else(|| panic!("{name}: track {track} missing")),
                want,
                "{name}: track {track} sample bytes"
            );
        }
    }
}

#[test]
fn a_queued_moof_past_max_fragment_bytes_is_the_typed_limit() {
    let bytes = test_fixtures::read("mp4/h264-aac-frag.mp4");
    let (rewritten, first_len) = cmf2_shape(&bytes).expect("CMF2 rewrite");
    let limits = Limits::default().with_max_fragment_bytes(first_len);
    let config = config(&rewritten, SampleOrder::File).with_limits(limits);
    let err = drive(&rewritten, config).expect_err("the second queued moof exceeds the budget");
    assert!(
        matches!(
            err,
            isobmff::Error::LimitExceeded {
                what: "fragment size",
                ..
            }
        ),
        "got {err:?}"
    );
}

/// One `sidx` reference entry (§8.16.3): reference word, duration, SAP word.
const SIDX_REFERENCE_LEN: usize = 12;

/// A real `sidx` with its last reference entry cut (the box shrinks, its
/// `reference_count` does not) cannot be parsed. It indexes fragments and is
/// not needed to play them: best effort reports it and still delivers every
/// sample the intact file plays; strict refuses it.
#[test]
fn a_sidx_cut_in_its_last_reference_is_reported_and_every_sample_still_plays() {
    let bytes = test_fixtures::read("dash/ondemand-sidx/v_dashinit.mp4");
    let sidx = isobmff::BoxIter::clamped(&bytes)
        .filter_map(|b| b.ok())
        .find(|b| b.header.box_type.is(b"sidx"))
        .expect("the fixture carries a sidx");
    let start = sidx.start;
    let len = sidx.bytes.len();
    let index = SegmentIndexBox::parse(sidx.bytes).expect("sidx parses");
    assert!(
        !index.references.is_empty(),
        "the sidx has a reference to cut"
    );
    let mut cut = bytes[..start + len - SIDX_REFERENCE_LEN].to_vec();
    let size = u32::try_from(len - SIDX_REFERENCE_LEN).expect("size");
    cut[start..start + 4].copy_from_slice(&size.to_be_bytes());
    cut.extend_from_slice(&bytes[start + len..]);

    let whole = drive(&bytes, config(&bytes, SampleOrder::File)).expect("intact drive");
    let strict = config(&cut, SampleOrder::File).with_mode(Mode::Strict);
    assert!(drive(&cut, strict).is_err(), "strict refuses the cut sidx");
    let lenient = drive(&cut, config(&cut, SampleOrder::File)).expect("best effort reads past it");
    assert!(
        lenient
            .damages
            .iter()
            .any(|damage| damage.fourcc() == Some(*b"sidx")
                && damage.location().byte_range().map(|r| r.start()) == Some(start as u64)),
        "the cut sidx is reported at its offset: {:?}",
        lenient.damages
    );
    let played = |outcome: &Outcome, file: &[u8]| -> Vec<(isobmff::TrackId, u32, u64, Vec<u8>)> {
        outcome
            .samples
            .iter()
            .map(|sample| {
                let at = usize::try_from(sample.offset).expect("offset");
                let end = at + usize::try_from(sample.size).expect("size");
                (
                    sample.track,
                    sample.index,
                    sample.decode_time,
                    file[at..end].to_vec(),
                )
            })
            .collect()
    };
    assert!(!whole.samples.is_empty(), "the intact file plays");
    assert_eq!(
        played(&lenient, &cut),
        played(&whole, &bytes),
        "every sample of the intact read is delivered"
    );
}
