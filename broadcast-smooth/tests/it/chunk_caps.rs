//! `c@r` chunk-run caps (MS-SSTR §2.2.2.6; remote alloc-DoS defense).

use broadcast_smooth::{MAX_CHUNK_RUN, SmoothManifest, SmoothParseError};

fn single_stream(chunks: &str) -> broadcast_smooth::StreamIndex {
    let xml = format!(
        r#"<SmoothStreamingMedia MajorVersion="2" MinorVersion="0" TimeScale="100"><StreamIndex Type="video" Url="u"><QualityLevel Index="0" Bitrate="1" FourCC="H264"/>{chunks}</StreamIndex></SmoothStreamingMedia>"#
    );
    SmoothManifest::parse(&xml).unwrap().streams.remove(0)
}

#[test]
fn a_zero_length_run_between_equal_explicit_times_is_accepted() {
    // The first `c` has no @d, so its duration is (next.t - t) / r = 0; an
    // explicit t equal to the accumulated time is not non-monotonic.
    let chunks = single_stream(r#"<c t="5"/><c t="5" d="1"/>"#)
        .enumerate_chunks()
        .unwrap();
    assert_eq!(chunks, vec![(5, 0), (5, 1)]);
}

#[test]
fn one_run_of_exactly_the_cap_is_accepted() {
    assert_eq!(MAX_CHUNK_RUN, 100_000);
    let chunks = single_stream(r#"<c d="1" r="100000"/>"#)
        .enumerate_chunks()
        .unwrap();
    assert_eq!(chunks.len(), 100_000);
    assert_eq!(chunks.first(), Some(&(0, 1)));
    assert_eq!(chunks.last(), Some(&(99_999, 1)));
}

#[test]
fn one_run_above_the_cap_is_rejected() {
    assert_eq!(
        single_stream(r#"<c d="1" r="100001"/>"#)
            .enumerate_chunks()
            .unwrap_err(),
        SmoothParseError::ChunkRunTooLong {
            count_hint: 100_001
        }
    );
}

#[test]
fn the_cumulative_cap_rejects_several_runs_each_below_it() {
    assert_eq!(
        single_stream(r#"<c d="1" r="50001"/><c d="1" r="50001"/>"#)
            .enumerate_chunks()
            .unwrap_err(),
        SmoothParseError::ChunkRunTooLong {
            count_hint: 100_002
        }
    );
}
