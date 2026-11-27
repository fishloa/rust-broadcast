//! Streaming-first Phase 6 slice 2 gate: the streaming `StreamingTsDemux`
//! reproduces the deleted whole-input `TsDemux`'s output line for line.
//!
//! `fixtures/ts/demux-baseline.txt` (and the private corpus' own baseline) was
//! recorded from `origin/main`'s `TsDemux` before this slice deleted it (see
//! `fixtures/ts/PROVENANCE.md`). Every fixture is demuxed here at three feed
//! granularities — 1 byte, 188 bytes (one TS packet) and 64 KiB — and its
//! per-track sample count, first/last timestamps, sync count, coded-byte hash,
//! timing hash, `start_decode_time`, movie timescale, PCR count, degraded and
//! skipped reports must equal the recorded lines exactly.
//!
//! Feeding a byte at a time costs a `feed` call per byte, so on the ~208 MB
//! private corpus that granularity alone is tens of seconds. The normal suite
//! therefore byte-feeds every capture at or below
//! [`BYTE_AT_A_TIME_MAX_BYTES`] (the four smallest private captures) and feeds
//! the larger ones in TS packets and 64 KiB chunks; `RUST_BROADCAST_SOAK=1`
//! (nightly) byte-feeds the whole corpus.

/// One MPEG-2 TS packet.
const TS_PACKET_LEN: usize = 188;
/// A read-sized feed.
const LARGE_CHUNK: usize = 64 * 1024;
/// The two granularities every capture is checked at.
const PACKET_AND_LARGE: [usize; 2] = [TS_PACKET_LEN, LARGE_CHUNK];
/// Every granularity, including one byte at a time.
const EVERY_GRANULARITY: [usize; 3] = [1, TS_PACKET_LEN, LARGE_CHUNK];
/// Largest capture the normal suite also feeds one byte at a time; larger ones
/// byte-feed under the soak switch.
const BYTE_AT_A_TIME_MAX_BYTES: usize = 4 << 20;

/// The feed granularities `bytes` is checked at.
fn chunks_for(bytes: &[u8]) -> &'static [usize] {
    let soak = std::env::var_os("RUST_BROADCAST_SOAK").is_some_and(|value| value == "1");
    if soak || bytes.len() <= BYTE_AT_A_TIME_MAX_BYTES {
        &EVERY_GRANULARITY
    } else {
        &PACKET_AND_LARGE
    }
}

/// Feed one baseline section through the streaming stage at each of
/// [`chunks_for`]'s granularities.
fn check(section: &crate::demux_baseline::Section, bytes: &[u8]) {
    for &chunk in chunks_for(bytes) {
        match test_harness::collect::media_from_ts_chunked(bytes, chunk) {
            Ok(media) => {
                assert!(
                    section.error.is_none(),
                    "{} at chunk {chunk}: streamed but the baseline records an error",
                    section.file
                );
                let (tracks, summary) = crate::demux_baseline::summarize(&media);
                assert_eq!(
                    tracks, section.tracks,
                    "{} at chunk {chunk}: per-track samples differ from the baseline",
                    section.file
                );
                assert_eq!(
                    summary, section.summary,
                    "{} at chunk {chunk}: summary differs from the baseline",
                    section.file
                );
            }
            Err(error) => assert_eq!(
                section.error.as_deref(),
                Some(error.to_string().as_str()),
                "{} at chunk {chunk}: streamed error differs from the baseline",
                section.file
            ),
        }
    }
}

#[test]
fn every_public_ts_fixture_reproduces_the_whole_input_baseline() {
    let text = test_fixtures::read_to_string("ts/demux-baseline.txt");
    let sections = crate::demux_baseline::parse(&text);
    assert!(
        sections.len() >= 48,
        "the golden must cover every public TS fixture"
    );
    for section in &sections {
        let bytes = test_fixtures::read(&format!("ts/{}", section.file));
        check(section, &bytes);
    }
}

#[test]
fn every_private_ts_capture_reproduces_the_whole_input_baseline() {
    let Some(path) = test_fixtures::private("ts/demux-baseline.txt") else {
        eprintln!(
            "SKIP every_private_ts_capture_reproduces_the_whole_input_baseline: private fixtures not checked out"
        );
        return;
    };
    let text = std::fs::read_to_string(&path).expect("read the private TS baseline");
    let sections = crate::demux_baseline::parse(&text);
    assert!(
        sections.len() >= 19,
        "the golden must cover every private TS capture"
    );
    let mut byte_fed = 0usize;
    for section in &sections {
        let Some(fixture) = test_fixtures::private(&format!("ts/{}", section.file)) else {
            panic!(
                "the private baseline names a missing fixture: {}",
                section.file
            );
        };
        let bytes = std::fs::read(&fixture).expect("read a private TS capture");
        if chunks_for(&bytes).len() == EVERY_GRANULARITY.len() {
            byte_fed += 1;
        }
        check(section, &bytes);
    }
    assert!(
        byte_fed > 0,
        "the normal suite must byte-feed at least one private capture, or the \
         byte-at-a-time path is only ever exercised under RUST_BROADCAST_SOAK=1"
    );
}
