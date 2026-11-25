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

/// Feed one baseline section through the streaming stage at three chunk sizes.
fn check(section: &crate::demux_baseline::Section, bytes: &[u8]) {
    for chunk in [1usize, 188, 64 * 1024] {
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
    for section in &sections {
        let Some(fixture) = test_fixtures::private(&format!("ts/{}", section.file)) else {
            panic!(
                "the private baseline names a missing fixture: {}",
                section.file
            );
        };
        let bytes = std::fs::read(&fixture).expect("read a private TS capture");
        check(section, &bytes);
    }
}
