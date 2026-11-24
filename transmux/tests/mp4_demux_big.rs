//! A multi-gigabyte generated MP4 demuxed through `Mp4Demux` under a fixed
//! peak-memory budget: the stage holds the reader's bounded structures and one
//! caller-sized read window, never the file.
//!
//! One `#[test]` per binary because `ProcessPeak` is process-global.

use std::collections::BTreeMap;
use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::num::NonZeroUsize;

use broadcast_common::stage::{Stage, Timestamp};
use test_harness::big_media::{BigFmp4Options, BigMp4Options, write_big_fmp4, write_big_mp4};
use transmux::mp4_demux::{Mp4Demux, Mp4DemuxConfig, Mp4Input};

#[global_allocator]
static PEAK: test_alloc::ProcessPeak = test_alloc::ProcessPeak::new();

/// The generated file's minimum size.
const TARGET_BYTES: u64 = 8 * 1024 * 1024 * 1024;
/// Peak live bytes allowed above the entry total, far below the file size.
const PEAK_BUDGET: usize = 96 * 1024 * 1024;
/// The caller's read window for the windowed runs.
const READ_WINDOW: usize = 1024 * 1024;
/// Bytes read from a `Read`-only source per feed in sequential mode.
const READ_CHUNK: usize = 64 * 1024;

fn path(tag: &str) -> std::path::PathBuf {
    std::env::temp_dir().join(format!(
        "transmux-mp4-demux-big-{}-{tag}.mp4",
        std::process::id()
    ))
}

fn drain(demux: &mut Mp4Demux, samples: &mut u64, last_dts: &mut BTreeMap<u32, i64>) {
    while let Some(event) = demux.poll() {
        if let transmux::DemuxEvent::Sample {
            track_id, sample, ..
        } = event
        {
            *samples += 1;
            if let Some(dts) = sample.timing.and_then(|timing| timing.dts()) {
                let previous = last_dts.insert(track_id.value(), dts).unwrap_or(i64::MIN);
                assert!(
                    dts >= previous,
                    "track {} dts {dts} after {previous}",
                    track_id.value()
                );
            }
        }
    }
}

fn seekable(path: &std::path::Path, window: Option<NonZeroUsize>) -> u64 {
    let mut file = File::open(path).expect("open generated mp4");
    let length = file.metadata().expect("metadata").len();
    let config = Mp4DemuxConfig::default()
        .with_total_len(Some(length))
        .with_read_window(window);
    let mut demux = Mp4Demux::seekable(config);
    let mut samples = 0u64;
    let mut last_dts = BTreeMap::new();
    let mut buffer = Vec::new();
    loop {
        drain(&mut demux, &mut samples, &mut last_dts);
        let demand = demux.demand();
        let Some(offset) = demand.offset else {
            break;
        };
        let start = offset.min(length);
        let len = u64::try_from(demand.want_bytes)
            .unwrap_or(u64::MAX)
            .min(length.saturating_sub(start));
        let len = usize::try_from(len).unwrap_or(usize::MAX);
        buffer.resize(len, 0);
        file.seek(SeekFrom::Start(start)).expect("seek range");
        file.read_exact(&mut buffer).expect("read range");
        demux
            .feed(
                Mp4Input::At {
                    offset: start,
                    bytes: core::mem::take(&mut buffer).into(),
                },
                Timestamp::ZERO,
            )
            .expect("feed range");
    }
    demux.finish().expect("finish");
    drain(&mut demux, &mut samples, &mut last_dts);
    samples
}

fn sequential(path: &std::path::Path) -> u64 {
    let mut file = File::open(path).expect("open generated mp4");
    let length = file.metadata().expect("metadata").len();
    let config = Mp4DemuxConfig::default().with_total_len(Some(length));
    let mut demux = Mp4Demux::sequential(config);
    let mut samples = 0u64;
    let mut last_dts = BTreeMap::new();
    let mut buffer = vec![0u8; READ_CHUNK];
    loop {
        drain(&mut demux, &mut samples, &mut last_dts);
        let demand = demux.demand();
        if demand.saturated {
            break;
        }
        let want = demand.want_bytes.max(1).min(buffer.len());
        let read = file.read(&mut buffer[..want]).expect("read source");
        if read == 0 {
            break;
        }
        demux
            .feed(Mp4Input::Next(&buffer[..read]), Timestamp::ZERO)
            .expect("feed source");
    }
    demux.finish().expect("finish");
    drain(&mut demux, &mut samples, &mut last_dts);
    samples
}

fn expected_samples(per_track: &[(u32, u64)]) -> u64 {
    per_track.iter().map(|(_, count)| count).sum()
}

fn assert_peak(peak: usize, tag: &str) {
    eprintln!("mp4 demux peak [{tag}]: {peak} bytes");
    assert!(
        peak <= PEAK_BUDGET,
        "{tag}: peak {peak} bytes exceeds the {PEAK_BUDGET}-byte budget for a \
         {TARGET_BYTES}-byte file"
    );
}

#[test]
fn a_multi_gigabyte_mp4_is_demuxed_in_bounded_memory() {
    if std::env::var_os("RUST_BROADCAST_SOAK").is_none() {
        eprintln!(
            "SKIPPED a_multi_gigabyte_mp4_is_demuxed_in_bounded_memory: \
             the run measures 65 s; set RUST_BROADCAST_SOAK=1 for the nightly soak"
        );
        return;
    }
    let base = BigMp4Options {
        target_bytes: TARGET_BYTES,
        ..BigMp4Options::default()
    };
    for moov_first in [true, false] {
        let tag = if moov_first {
            "moov-first"
        } else {
            "moov-last"
        };
        let file = path(tag);
        let written = write_big_mp4(
            &file,
            &BigMp4Options {
                moov_first,
                ..base.clone()
            },
        );
        let expected = expected_samples(&written.samples_per_track);
        let (peak, samples) = PEAK.peak_during(|| seekable(&file, None));
        assert_eq!(samples, expected, "{tag}: one request per sample");
        assert_peak(peak, &format!("{tag}/unset-window"));
        let window = NonZeroUsize::new(READ_WINDOW).expect("non-zero read window");
        let (peak, samples) = PEAK.peak_during(|| seekable(&file, Some(window)));
        assert_eq!(samples, expected, "{tag}: windowed");
        assert_peak(peak, &format!("{tag}/window"));
        std::fs::remove_file(&file).expect("remove generated file");
    }

    let file = path("fragmented");
    let written = write_big_fmp4(
        &file,
        &BigFmp4Options {
            target_bytes: TARGET_BYTES,
            ..BigFmp4Options::default()
        },
    );
    let expected = u64::from(written.fragment_count);
    let (peak, samples) = PEAK.peak_during(|| seekable(&file, None));
    assert_eq!(samples, expected, "fragmented: one request per sample");
    assert_peak(peak, "fragmented/unset-window");
    let window = NonZeroUsize::new(READ_WINDOW).expect("non-zero read window");
    let (peak, samples) = PEAK.peak_during(|| seekable(&file, Some(window)));
    assert_eq!(samples, expected, "fragmented: windowed");
    assert_peak(peak, "fragmented/window");
    let (peak, samples) = PEAK.peak_during(|| sequential(&file));
    assert_eq!(samples, expected, "fragmented: sequential");
    assert_peak(peak, "fragmented/sequential");
    std::fs::remove_file(&file).expect("remove generated file");
}
