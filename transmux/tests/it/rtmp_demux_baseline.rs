//! Streaming-first Phase 6 slice 2 gate: the streaming RTMP reassembly (the
//! chunk stream plus `StreamingFlvDemux`) reproduces the deleted whole-input
//! `RtmpDemux`'s output line for line.
//!
//! `fixtures/rtmp/demux-baseline.txt` was recorded from `origin/main`'s
//! `RtmpDemux` before this slice deleted it (see
//! `fixtures/rtmp/PROVENANCE.md`). Every capture is reassembled here at three
//! feed granularities — 1 byte, 20 bytes and 64 KiB — and its per-track fields
//! and summary must equal the recorded lines exactly; a capture the demuxer
//! refused whole must be refused with the same error.

#[test]
fn every_rtmp_capture_reproduces_the_whole_input_baseline() {
    let text = test_fixtures::read_to_string("rtmp/demux-baseline.txt");
    let sections = crate::demux_baseline::parse(&text);
    assert!(
        !sections.is_empty(),
        "the golden must cover every RTMP capture"
    );
    for section in &sections {
        let bytes = test_fixtures::read(&format!("rtmp/{}", section.file));
        let bytes = &bytes[section.skip.min(bytes.len())..];
        for chunk in [1usize, 20, 64 * 1024] {
            match test_harness::collect::media_from_rtmp_chunked(bytes, chunk) {
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
}
