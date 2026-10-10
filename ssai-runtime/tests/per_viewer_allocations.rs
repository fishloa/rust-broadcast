//! Audit r14-SSAI-O1 (issue #1125): rendering a per-viewer playlist from a
//! [`SessionPlaylistBase`] must not scale with the segment count, while the
//! clone-based [`render_session_playlist`] does. Measured with a counting
//! global allocator — deterministic, no timing.
//!
//! Single `#[test]` in this binary on purpose: the allocation counter is
//! process-global, so a second concurrently-running test would pollute it.

use broadcast_hls::{DecimalSeconds, MediaPlaylist, MediaSegment};
use ssai_runtime::playlist::{InterstitialDateRange, SessionPlaylistBase};
use ssai_runtime::{AssetSource, render_session_playlist};

#[global_allocator]
static A: test_alloc::ProcessCounting = test_alloc::ProcessCounting::new();

fn allocs_during<R>(f: impl FnOnce() -> R) -> (usize, R) {
    A.allocs_during(f)
}

const SEGMENTS: usize = 500;

#[test]
fn per_viewer_render_does_not_scale_with_segment_count() {
    let mut base = MediaPlaylist {
        target_duration: 6,
        ..Default::default()
    };
    for i in 0..SEGMENTS {
        base.segments.push(MediaSegment {
            duration: DecimalSeconds::new(6.0).unwrap(),
            uri: format!("main{i}.ts"),
            pre_tags: if i == 0 {
                vec!["#EXT-X-PROGRAM-DATE-TIME:2020-01-02T21:55:40.000Z".to_string()]
            } else {
                Vec::new()
            },
            ..Default::default()
        });
    }
    let dr = InterstitialDateRange {
        id: "ad1".into(),
        start_date: "2020-01-02T21:55:44.000Z".into(),
        duration: Some(15.0),
        asset: AssetSource::Uri("http://ads.example/a.m3u8".into()),
        resume_offset: None,
        playout_limit: None,
        snap: Vec::new(),
        restrict: Vec::new(),
    };

    let rendered = SessionPlaylistBase::new(&base).unwrap();
    let mut out = String::with_capacity(base.to_m3u8().unwrap().len() + 1024);

    // Clone path: at least one allocation per segment URI.
    let (clone_allocs, pl) = allocs_during(|| render_session_playlist(&base, Some(&dr)).unwrap());
    assert!(
        clone_allocs >= SEGMENTS,
        "clone path made only {clone_allocs} allocations; the counter is not measuring"
    );
    drop(pl);

    // Splice path into a pre-sized buffer: only the tag-line construction.
    let (splice_allocs, ()) = allocs_during(|| rendered.render_into(&mut out, Some(&dr)).unwrap());
    assert!(
        splice_allocs < 64,
        "spliced render made {splice_allocs} allocations for {SEGMENTS} segments"
    );
    assert!(out.contains("X-ASSET-URI=\"http://ads.example/a.m3u8\""));
}
