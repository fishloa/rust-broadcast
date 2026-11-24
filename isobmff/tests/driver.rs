//! The blocking drivers: a seekable source over sparse multi-gigabyte files,
//! and a forward-only source over pipes, including its typed refusals.

use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::path::Path;

use isobmff::driver::{DriverError, DriverStep, FileReader, ForwardReader};
use isobmff::{Error, ReaderConfig, SampleOrder, TrackId};

#[global_allocator]
static PEAK: test_alloc::ProcessPeak = test_alloc::ProcessPeak::new();

/// Peak live bytes allowed per byte of `moov`: the supplied `moov` buffer, the
/// parsed box tree, the raw sample-entry bodies, and the driver's cloned track
/// table.
const PEAK_PER_MOOV: usize = 8;
/// Fixed peak allowance above `moov * PEAK_PER_MOOV` for the driver and the
/// test harness.
const PEAK_SLACK: usize = 4 << 20;
/// Fixed peak allowance for a fragmented file, independent of fragment count:
/// one `moof` in flight plus the resolver's rows.
const FRAGMENTED_PEAK_SLACK: usize = 8 << 20;

/// A `Read` source with no `Seek` impl, so only [`ForwardReader`] can drive it.
struct ReadOnly(File);

impl Read for ReadOnly {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        self.0.read(buf)
    }
}

fn open(path: &Path) -> File {
    File::open(path).expect("open fixture")
}

fn count_samples(path: &Path, order: SampleOrder) -> (Vec<(TrackId, u64)>, u64) {
    let mut reader = FileReader::new(
        open(path),
        ReaderConfig::default()
            .with_total_len(None)
            .with_order(order),
    )
    .expect("wrap file");
    let mut counts: Vec<(TrackId, u64)> = Vec::new();
    let mut samples = 0u64;
    loop {
        match reader.step().expect("driver step") {
            DriverStep::Tracks(tracks) => {
                counts = tracks.iter().map(|track| (track.track_id, 0)).collect();
            }
            DriverStep::Sample(sample) => {
                samples += 1;
                if let Some(entry) = counts.iter_mut().find(|(id, _)| *id == sample.track) {
                    entry.1 += 1;
                }
            }
            DriverStep::Fragment(_) => {}
            DriverStep::Done => break,
            _ => {}
        }
    }
    (counts, samples)
}

#[test]
fn a_multi_gigabyte_progressive_file_costs_only_its_headers() {
    let base = test_harness::big_media::BigMp4Options::default();
    for moov_first in [true, false] {
        let path = std::env::temp_dir().join(format!(
            "isobmff-driver-big-{}-{}.mp4",
            std::process::id(),
            if moov_first { "first" } else { "last" }
        ));
        let mut options = base.clone();
        options.moov_first = moov_first;
        let written = test_harness::big_media::write_big_mp4(&path, &options);
        PEAK.reset();
        let (peak, (counts, samples)) =
            PEAK.peak_during(|| count_samples(&path, SampleOrder::File));
        std::fs::remove_file(&path).expect("remove generated file");
        assert_eq!(samples, u64::from(base.sample_count) * 2);
        for (track, expected) in &written.samples_per_track {
            let found = counts
                .iter()
                .find(|(id, _)| id.value() == *track)
                .map(|(_, count)| *count);
            assert_eq!(found, Some(*expected), "track {track}");
        }
        let budget =
            usize::try_from(written.moov_bytes).unwrap_or(usize::MAX) * PEAK_PER_MOOV + PEAK_SLACK;
        assert!(
            peak < budget,
            "moov_first={moov_first}: peak {peak} bytes, budget {budget} (moov {})",
            written.moov_bytes
        );
    }
}

#[test]
fn a_multi_gigabyte_fragmented_file_holds_one_moof() {
    let options = test_harness::big_media::BigFmp4Options::default();
    let path = std::env::temp_dir().join(format!(
        "isobmff-driver-big-frag-{}.mp4",
        std::process::id()
    ));
    let written = test_harness::big_media::write_big_fmp4(&path, &options);
    PEAK.reset();
    let (peak, (samples, fragments)) = PEAK.peak_during(|| {
        let mut reader = FileReader::new(
            open(&path),
            ReaderConfig::default().with_order(SampleOrder::File),
        )
        .expect("wrap file");
        let mut samples = 0u64;
        let mut fragments = 0u64;
        loop {
            match reader.step().expect("driver step") {
                DriverStep::Tracks(tracks) => assert_eq!(tracks.len(), 1),
                DriverStep::Fragment(_) => fragments += 1,
                DriverStep::Sample(_) => samples += 1,
                DriverStep::Done => break,
                _ => {}
            }
        }
        (samples, fragments)
    });
    std::fs::remove_file(&path).expect("remove generated file");
    assert_eq!(samples, u64::from(written.fragment_count));
    assert_eq!(fragments, u64::from(written.fragment_count));
    let budget = usize::try_from(written.moov_bytes).unwrap_or(usize::MAX) * PEAK_PER_MOOV
        + FRAGMENTED_PEAK_SLACK;
    assert!(
        peak < budget,
        "peak {peak} bytes, budget {budget} (moov {}, {} fragments)",
        written.moov_bytes,
        written.fragment_count
    );
}

/// Drive a forward-only source over a `Read`-only wrapper, read every sample and
/// compare it with the fixture on disk. Returns `(tracks, samples, fragments)`.
fn read_forward(path: &Path, order: SampleOrder) -> (usize, u64, u64) {
    let bytes = std::fs::read(path).expect("read fixture");
    let mut reader = ForwardReader::new(
        ReadOnly(open(path)),
        ReaderConfig::default().with_order(order),
    );
    let mut tracks = 0usize;
    let mut samples = 0u64;
    let mut fragments = 0u64;
    let mut out = Vec::new();
    loop {
        let sample = match reader.step().expect("forward step") {
            DriverStep::Tracks(info) => {
                tracks = info.len();
                continue;
            }
            DriverStep::Fragment(_) => {
                fragments += 1;
                continue;
            }
            DriverStep::Sample(sample) => sample,
            DriverStep::Done => break,
            _ => continue,
        };
        reader
            .read_sample(&sample, &mut out)
            .expect("read sample data");
        let start = usize::try_from(sample.offset).unwrap_or(usize::MAX);
        let end = start.saturating_add(usize::try_from(sample.size).unwrap_or(usize::MAX));
        assert_eq!(
            out.as_slice(),
            bytes.get(start..end).expect("sample range"),
            "sample {} of track {}",
            sample.index,
            sample.track.value()
        );
        samples += 1;
    }
    (tracks, samples, fragments)
}

#[test]
fn a_forward_only_source_reads_progressive_media_in_file_order() {
    let path = test_fixtures::path("mp4/progressive/av-faststart.mp4");
    let (tracks, samples, fragments) = read_forward(&path, SampleOrder::File);
    assert!(tracks > 0);
    assert!(samples > 0);
    assert_eq!(fragments, 0);
}

#[test]
fn a_forward_only_source_reads_a_fragmented_fixture() {
    let path = test_fixtures::path("mp4/frag/h264-high.frag.mp4");
    let (tracks, samples, fragments) = read_forward(&path, SampleOrder::File);
    assert!(tracks > 0);
    assert!(samples > 0);
    assert!(fragments > 0);
}

#[test]
fn a_pipe_reads_every_fragmented_sample_byte() {
    for name in [
        "mp4/frag/h264-high.frag.mp4",
        "mp4/cmaf/av-frag.mp4",
        "mp4/llcmaf/ll-chunked.mp4",
        "mp4/cenc-layouts/enc-default-explicit.mp4",
        "mp4/h264-aac-frag.mp4",
    ] {
        let path = test_fixtures::path(name);
        let (tracks, samples, fragments) = read_forward(&path, SampleOrder::File);
        assert!(tracks > 0, "{name}: no tracks");
        assert!(samples > 0, "{name}: no samples");
        assert!(fragments > 0, "{name}: no fragments");
    }
}

#[test]
fn a_forward_only_source_emits_file_order_whatever_was_asked_for() {
    let path = test_fixtures::path("mp4/cmaf/av-frag.mp4");
    let reader = ForwardReader::new(
        ReadOnly(open(&path)),
        ReaderConfig::default().with_order(SampleOrder::DecodeTime),
    );
    assert_eq!(reader.order(), SampleOrder::File);
    let (tracks, samples, _) = read_forward(&path, SampleOrder::DecodeTime);
    assert!(tracks > 0);
    assert!(samples > 0);
}

#[test]
fn a_moov_last_file_is_a_typed_forward_only_error() {
    let path = test_fixtures::path("mp4/progressive/av-prog.mp4");
    let mut reader = ForwardReader::new(
        ReadOnly(open(&path)),
        ReaderConfig::default().with_order(SampleOrder::File),
    );
    loop {
        match reader.step() {
            Ok(DriverStep::Done) => panic!("moov-last file completed on a forward-only source"),
            Ok(_) => {}
            Err(error) => {
                assert!(
                    matches!(error, DriverError::Format(Error::MoovAfterMdatForwardOnly)),
                    "{error:?}"
                );
                return;
            }
        }
    }
}

#[test]
fn a_seekable_source_reads_every_sample_byte() {
    let path = test_fixtures::path("mp4/progressive/av-faststart.mp4");
    let bytes = std::fs::read(&path).expect("read fixture");
    let mut reader = FileReader::new(
        open(&path),
        ReaderConfig::default().with_order(SampleOrder::File),
    )
    .expect("wrap file");
    let mut out = Vec::new();
    let mut samples = 0u64;
    loop {
        let sample = match reader.step().expect("step") {
            DriverStep::Sample(sample) => sample,
            DriverStep::Tracks(_) | DriverStep::Fragment(_) => continue,
            DriverStep::Done => break,
            _ => continue,
        };
        reader.read_sample(&sample, &mut out).expect("read sample");
        let start = usize::try_from(sample.offset).unwrap_or(usize::MAX);
        let end = start.saturating_add(usize::try_from(sample.size).unwrap_or(usize::MAX));
        assert_eq!(out.as_slice(), bytes.get(start..end).expect("range"));
        samples += 1;
    }
    assert!(samples > 0);
}

#[test]
fn a_seekable_source_seeks_to_the_sync_sample_at_or_before_a_time() {
    let path = test_fixtures::path("mp4/progressive/av-faststart.mp4");
    let mut reader = FileReader::new(open(&path), ReaderConfig::default()).expect("wrap file");
    let track = loop {
        match reader.step().expect("step") {
            DriverStep::Tracks(tracks) => break tracks.first().map(|info| info.track_id),
            _ => continue,
        }
    };
    let track = track.expect("a video track");
    reader
        .seek_to(track, 0)
        .expect("seek to the first sync sample");
    let sample = loop {
        match reader.step().expect("step after seek") {
            DriverStep::Sample(sample) => break sample,
            DriverStep::Tracks(_) | DriverStep::Fragment(_) => continue,
            DriverStep::Done => panic!("no sample after seek"),
            _ => continue,
        }
    };
    assert!(sample.sync);
    assert_eq!(sample.decode_time, 0);
}

#[test]
fn a_forward_only_source_refuses_a_sample_behind_its_position() {
    let path = test_fixtures::path("mp4/progressive/av-faststart.mp4");
    let mut reader = ForwardReader::new(
        ReadOnly(open(&path)),
        ReaderConfig::default().with_order(SampleOrder::File),
    );
    let first = loop {
        match reader.step().expect("step") {
            DriverStep::Sample(sample) => break sample,
            DriverStep::Tracks(_) | DriverStep::Fragment(_) => continue,
            DriverStep::Done => panic!("no sample"),
            _ => continue,
        }
    };
    let mut out = Vec::new();
    reader.read_sample(&first, &mut out).expect("first read");
    let error = reader
        .read_sample(&first, &mut out)
        .expect_err("second read is behind");
    assert!(matches!(error, DriverError::SampleBehindPosition { .. }));
}

#[test]
fn the_drivers_report_every_go_pro_track_and_its_count() {
    let Some(path) = test_fixtures::require("mp4/camera/gopro-karma.mp4", "driver") else {
        return;
    };
    let mut reader = FileReader::new(
        open(&path),
        ReaderConfig::default().with_order(SampleOrder::File),
    )
    .expect("wrap gopro");
    let mut per_fourcc: Vec<([u8; 4], TrackId, u64)> = Vec::new();
    loop {
        match reader.step().expect("gopro step") {
            DriverStep::Tracks(tracks) => {
                for track in tracks {
                    let fourcc = track
                        .entries
                        .first()
                        .map(|entry| entry.fourcc)
                        .unwrap_or([0; 4]);
                    per_fourcc.push((fourcc, track.track_id, 0));
                }
            }
            DriverStep::Sample(sample) => {
                if let Some(entry) = per_fourcc.iter_mut().find(|(_, id, _)| *id == sample.track) {
                    entry.2 += 1;
                }
            }
            DriverStep::Fragment(_) => {}
            DriverStep::Done => break,
            _ => {}
        }
    }
    assert_eq!(per_fourcc.len(), 5);
    let expected: [([u8; 4], u64); 5] = [
        (*b"avc1", 362),
        (*b"mp4a", 566),
        (*b"tmcd", 1),
        (*b"gpmd", 13),
        (*b"fdsc", 943),
    ];
    for (fourcc, count) in expected {
        let found = per_fourcc
            .iter()
            .find(|(found, _, _)| *found == fourcc)
            .map(|(_, _, count)| *count);
        assert_eq!(found, Some(count), "{}", String::from_utf8_lossy(&fourcc));
    }
}

#[test]
fn a_seekable_driver_recovers_its_source() {
    let path = test_fixtures::path("mp4/progressive/av-faststart.mp4");
    let file = open(&path);
    let before = file.metadata().expect("metadata").len();
    let reader = FileReader::new(file, ReaderConfig::default()).expect("wrap");
    let mut file = reader.into_inner();
    let after = file.seek(SeekFrom::End(0)).expect("seek recovered file");
    assert_eq!(before, after);
}

/// Drive a source to its metadata step and return the movie title.
fn metadata_title(path: &Path) -> Option<String> {
    let mut reader = FileReader::new(
        open(path),
        ReaderConfig::default().with_order(SampleOrder::File),
    )
    .expect("wrap file");
    loop {
        match reader.step().expect("driver step") {
            DriverStep::Metadata(metadata) => {
                return metadata
                    .movie
                    .text(&isobmff::metadata::ItemKey::TITLE)
                    .map(str::to_string);
            }
            DriverStep::Done => return None,
            _ => {}
        }
    }
}

/// Drive a forward-only source to its metadata step and return its movie items.
fn forward_metadata_items(path: &Path) -> Option<Vec<isobmff::metadata::MetadataItem>> {
    let mut reader = ForwardReader::new(
        ReadOnly(open(path)),
        ReaderConfig::default().with_order(SampleOrder::File),
    );
    loop {
        match reader.step().expect("forward step") {
            DriverStep::Metadata(metadata) => return Some(metadata.movie.items.clone()),
            DriverStep::Done => return None,
            _ => {}
        }
    }
}

#[test]
fn a_seekable_driver_reports_the_movie_metadata() {
    let Some(path) = test_fixtures::require("mp4/tags/lavf-tagged.m4a", "driver") else {
        return;
    };
    assert_eq!(metadata_title(&path).as_deref(), Some("Test Tone"));
}

#[test]
fn a_forward_only_driver_reports_the_movie_metadata() {
    // `lavf-tagged.m4a` puts its `moov` after its `mdat`, so a pipe cannot
    // reach its tags at all: that file is the reader's typed refusal.
    let Some(path) = test_fixtures::require("mp4/tags/lavf-tagged.m4a", "driver") else {
        return;
    };
    let mut reader = ForwardReader::new(
        ReadOnly(open(&path)),
        ReaderConfig::default().with_order(SampleOrder::File),
    );
    let refused = loop {
        match reader.step() {
            Ok(DriverStep::Done) => panic!("a moov-last file completed on a pipe"),
            Ok(_) => {}
            Err(error) => break error,
        }
    };
    assert!(
        matches!(
            refused,
            DriverError::Format(Error::MoovAfterMdatForwardOnly)
        ),
        "{refused:?}"
    );

    // The moov-first fixture's freeform item is reported through the driver.
    let Some(path) = test_fixtures::require("mp4/tags/afconvert-aac.m4a", "driver") else {
        return;
    };
    let items = forward_metadata_items(&path).expect("a metadata step");
    assert!(
        items.iter().any(|item| matches!(
            item,
            isobmff::metadata::MetadataItem::Freeform { mean, name, .. }
                if mean == "com.apple.iTunes" && name == "iTunSMPB"
        )),
        "{items:?}"
    );
}
