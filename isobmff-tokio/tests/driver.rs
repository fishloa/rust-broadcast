//! The async driver over a file and over an in-memory cursor: the same
//! moov-first, fragmented and moov-last cases the blocking driver is held to.

use std::io::Cursor;
use std::path::Path;

use isobmff::driver::DriverStep;
use isobmff::{ReaderConfig, SampleOrder};
use isobmff_tokio::AsyncFileReader;
use tokio::io::{AsyncRead, AsyncSeek};

async fn read_file(path: &Path) -> Vec<u8> {
    tokio::fs::read(path).await.expect("read fixture")
}

/// Iterate a source, reading every sample's bytes and comparing them with the
/// fixture. Returns `(tracks, samples, fragments)`.
async fn read_samples<R: AsyncRead + AsyncSeek + Unpin>(
    inner: R,
    fixture: &[u8],
) -> (usize, u64, u64) {
    let config = ReaderConfig::default().with_order(SampleOrder::File);
    let mut reader = AsyncFileReader::new(inner, config)
        .await
        .expect("wrap source");
    let mut tracks = 0usize;
    let mut samples = 0u64;
    let mut fragments = 0u64;
    let mut out = Vec::new();
    loop {
        let sample = match reader.step().await.expect("step") {
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
            .await
            .expect("read sample");
        let start = usize::try_from(sample.offset).unwrap_or(usize::MAX);
        let end = start.saturating_add(usize::try_from(sample.size).unwrap_or(usize::MAX));
        assert_eq!(out.as_slice(), fixture.get(start..end).expect("range"));
        samples += 1;
    }
    (tracks, samples, fragments)
}

async fn read_moov_last<R: AsyncRead + AsyncSeek + Unpin>(inner: R) -> u64 {
    let config = ReaderConfig::default().with_order(SampleOrder::File);
    let mut reader = AsyncFileReader::new(inner, config)
        .await
        .expect("wrap source");
    let mut samples = 0u64;
    loop {
        match reader.step().await.expect("step") {
            DriverStep::Sample(_) => samples += 1,
            DriverStep::Tracks(_) | DriverStep::Fragment(_) => {}
            DriverStep::Done => break,
            _ => {}
        }
    }
    samples
}

#[tokio::test]
async fn a_tokio_file_reads_every_progressive_sample_byte() {
    let path = test_fixtures::path("mp4/progressive/av-faststart.mp4");
    let fixture = read_file(&path).await;
    let file = tokio::fs::File::open(&path).await.expect("open fixture");
    let (tracks, samples, fragments) = read_samples(file, &fixture).await;
    assert!(tracks > 0);
    assert!(samples > 0);
    assert_eq!(fragments, 0);
}

#[tokio::test]
async fn a_tokio_file_reads_a_fragmented_fixture() {
    let path = test_fixtures::path("mp4/frag/h264-high.frag.mp4");
    let fixture = read_file(&path).await;
    let file = tokio::fs::File::open(&path).await.expect("open fixture");
    let (tracks, samples, fragments) = read_samples(file, &fixture).await;
    assert!(tracks > 0);
    assert!(samples > 0);
    assert!(fragments > 0);
}

#[tokio::test]
async fn a_tokio_file_reaches_a_moov_after_mdat() {
    let path = test_fixtures::path("mp4/progressive/av-prog.mp4");
    let file = tokio::fs::File::open(&path).await.expect("open fixture");
    assert!(read_moov_last(file).await > 0);
}

#[tokio::test]
async fn an_in_memory_cursor_reads_every_progressive_sample_byte() {
    let path = test_fixtures::path("mp4/progressive/av-faststart.mp4");
    let fixture = read_file(&path).await;
    let cursor = Cursor::new(fixture.clone());
    let (tracks, samples, fragments) = read_samples(cursor, &fixture).await;
    assert!(tracks > 0);
    assert!(samples > 0);
    assert_eq!(fragments, 0);
}

#[tokio::test]
async fn an_in_memory_cursor_reads_a_fragmented_fixture() {
    let path = test_fixtures::path("mp4/frag/h264-high.frag.mp4");
    let fixture = read_file(&path).await;
    let (tracks, samples, fragments) = read_samples(Cursor::new(fixture.clone()), &fixture).await;
    assert!(tracks > 0);
    assert!(samples > 0);
    assert!(fragments > 0);
}

#[tokio::test]
async fn an_in_memory_cursor_reaches_a_moov_after_mdat() {
    let path = test_fixtures::path("mp4/progressive/av-prog.mp4");
    let fixture = read_file(&path).await;
    assert!(read_moov_last(Cursor::new(fixture)).await > 0);
}

#[tokio::test]
async fn an_in_memory_cursor_seeks_to_the_sync_sample_at_or_before_a_time() {
    let path = test_fixtures::path("mp4/progressive/av-faststart.mp4");
    let fixture = read_file(&path).await;
    let config = ReaderConfig::default();
    let mut reader = AsyncFileReader::new(Cursor::new(fixture), config)
        .await
        .expect("wrap source");
    let track = loop {
        match reader.step().await.expect("step") {
            DriverStep::Tracks(tracks) => break tracks.first().map(|info| info.track_id),
            _ => continue,
        }
    };
    let track = track.expect("a video track");
    reader.seek_to(track, 0).await.expect("seek");
    let sample = loop {
        match reader.step().await.expect("step after seek") {
            DriverStep::Sample(sample) => break sample,
            DriverStep::Tracks(_) | DriverStep::Fragment(_) => continue,
            DriverStep::Done => panic!("no sample after seek"),
            _ => continue,
        }
    };
    assert!(sample.sync);
    assert_eq!(sample.decode_time, 0);
}

#[tokio::test]
async fn a_tokio_file_reports_the_movie_metadata() {
    let Some(path) = test_fixtures::require("mp4/tags/lavf-tagged.m4a", "isobmff-tokio") else {
        return;
    };
    let file = tokio::fs::File::open(&path).await.expect("open fixture");
    let config = ReaderConfig::default().with_order(SampleOrder::File);
    let mut reader = AsyncFileReader::new(file, config)
        .await
        .expect("wrap source");
    let mut title = None;
    loop {
        match reader.step().await.expect("step") {
            DriverStep::Metadata(metadata) => {
                title = metadata
                    .movie
                    .text(&isobmff::metadata::ItemKey::TITLE)
                    .map(str::to_string);
            }
            DriverStep::Done => break,
            _ => {}
        }
    }
    assert_eq!(title.as_deref(), Some("Test Tone"));
}

#[tokio::test]
async fn the_async_driver_passes_a_warning_through() {
    let path = test_fixtures::path("mp4/progressive/av-faststart.mp4");
    let mut bytes = std::fs::read(&path).expect("read fixture");
    let mvhd = bytes.windows(4).position(|w| w == b"mvhd").expect("mvhd") - 4;
    let timescale = mvhd + if bytes[mvhd + 8] == 1 { 28 } else { 20 };
    bytes[timescale..timescale + 4].fill(0);
    let config = ReaderConfig::default().with_order(SampleOrder::File);
    let mut reader = AsyncFileReader::new(Cursor::new(bytes), config)
        .await
        .expect("wrap source");
    loop {
        match reader.step().await.expect("step") {
            DriverStep::Damage(damage)
                if matches!(damage.what(), "zero-movie-timescale" | "synthesized-box") =>
            {
                return;
            }
            DriverStep::Done => panic!("no warning"),
            _ => {}
        }
    }
}
