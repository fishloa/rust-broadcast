//! No per-segment concatenation of the init segment and a segment body
//! (issue #4057, streaming-first phase 3 slice 2).
//!
//! The whole test runs under a live-byte budget (`test_alloc::ThreadCapped`)
//! smaller than `init.len() + segment.len()`: the streaming stage is fed the
//! init once and then the segment in the same bounded pieces the HLS client
//! hands it, so a build that still built the combined `init ++ segment` buffer
//! (or buffered a whole body) would abort instead of passing.
//!
//! The generated presentation is over four mebibytes and is read from a file
//! piece by piece; the test itself never holds a whole body either.

#![cfg(target_pointer_width = "64")]

use std::fs::File;
use std::io::{Read, Seek, SeekFrom};

use broadcast_common::stage::{Stage, Timestamp};
use test_alloc::ThreadCapped;
use test_harness::big_media::{BigFmp4Options, write_big_fmp4};
use transmux::mp4_demux::{Mp4Demux, Mp4DemuxConfig, Mp4Input};

#[global_allocator]
static ALLOC: ThreadCapped<BUDGET> = ThreadCapped::new();

/// Live bytes the demux may hold: one feed piece plus the one fragment it is
/// resolving, far below `init.len() + segment.len()`.
const BUDGET: usize = 1 << 20;
/// Bytes read from the generated file per feed, the piece the client hands the
/// stage.
const FEED_BYTES: usize = 64 * 1024;
/// Bytes in each fragment's one sample.
const SAMPLE_BYTES: u32 = 4 * 1024;
/// Fragments in the generated presentation (one sample each).
const FRAGMENTS: u32 = 1024;

fn temp_path() -> std::path::PathBuf {
    std::env::temp_dir().join(format!("hls-runtime-no-concat-{}.mp4", std::process::id()))
}

/// Drain every event the stage has produced, counting the samples.
fn drain(demux: &mut Mp4Demux) -> usize {
    let mut samples = 0usize;
    while let Some(event) = demux.poll() {
        if let transmux::DemuxEvent::Sample { .. } = event {
            samples = samples.saturating_add(1);
        }
    }
    samples
}

#[test]
fn a_segment_is_never_concatenated_with_its_init() {
    let path = temp_path();
    let big = write_big_fmp4(
        &path,
        &BigFmp4Options {
            target_bytes: 0,
            fragment_count: FRAGMENTS,
            sample_bytes: SAMPLE_BYTES,
            with_mfra: false,
        },
    );
    let mut file = File::open(&path).expect("open the generated fMP4");
    let file_len = file.metadata().expect("metadata").len();
    let init_len = big.moof_offsets[0];
    let segment_len = file_len.saturating_sub(init_len);
    assert!(
        file_len > BUDGET as u64,
        "the budget must be below init.len() + segment.len(): {file_len} vs {BUDGET}"
    );

    let mut init = vec![0u8; usize::try_from(init_len).expect("init length")];
    file.read_exact(&mut init).expect("read the init");

    let mut demux = Mp4Demux::sequential(Mp4DemuxConfig::default());
    demux
        .feed(Mp4Input::Next(&init), Timestamp::ZERO)
        .expect("feed the init");
    demux
        .feed(Mp4Input::SegmentEnd, Timestamp::ZERO)
        .expect("close the init");

    let mut samples = drain(&mut demux);
    let mut piece = vec![0u8; FEED_BYTES];
    let mut read = 0u64;
    while read < segment_len {
        let take = usize::try_from(segment_len.saturating_sub(read))
            .unwrap_or(usize::MAX)
            .min(piece.len());
        file.seek(SeekFrom::Start(init_len.saturating_add(read)))
            .expect("seek the segment");
        file.read_exact(&mut piece[..take])
            .expect("read a segment piece");
        demux
            .feed(Mp4Input::Next(&piece[..take]), Timestamp::ZERO)
            .expect("feed a segment piece");
        samples = samples.saturating_add(drain(&mut demux));
        read = read.saturating_add(u64::try_from(take).unwrap_or(u64::MAX));
    }
    demux
        .feed(Mp4Input::SegmentEnd, Timestamp::ZERO)
        .expect("close the segment");
    demux.finish().expect("finish the input");
    samples = samples.saturating_add(drain(&mut demux));

    let _ = std::fs::remove_file(&path);
    assert_eq!(
        u64::try_from(samples).expect("sample count"),
        u64::from(FRAGMENTS),
        "every fragment's one sample was demuxed"
    );
}
