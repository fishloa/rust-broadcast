//! Streaming-first Phase 6 slice 2 gate: the streaming `StreamingFlvDemux`
//! reproduces the deleted whole-input `FlvDemux`'s output line for line.
//!
//! `fixtures/flv/demux-baseline.txt` was recorded from `origin/main`'s
//! `FlvDemux` before this slice deleted it (see `fixtures/flv/PROVENANCE.md`).
//! Every fixture is demuxed here at three feed granularities — 1 byte, 188
//! bytes and 64 KiB — and its per-track fields and summary must equal the
//! recorded lines exactly; an input the demuxer refused whole must be refused
//! with the same error.

#[path = "support/demux_baseline.rs"]
mod demux_baseline;

#[test]
fn every_flv_fixture_reproduces_the_whole_input_baseline() {
    let text = test_fixtures::read_to_string("flv/demux-baseline.txt");
    let sections = demux_baseline::parse(&text);
    assert!(
        sections.len() >= 3,
        "the golden must cover every FLV fixture"
    );
    for section in &sections {
        let bytes = test_fixtures::read(&format!("flv/{}", section.file));
        for chunk in [1usize, 188, 64 * 1024] {
            match test_harness::collect::media_from_flv_chunked(&bytes, chunk) {
                Ok(media) => {
                    assert!(
                        section.error.is_none(),
                        "{} at chunk {chunk}: streamed but the baseline records an error",
                        section.file
                    );
                    let (tracks, summary) = demux_baseline::summarize(&media);
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
