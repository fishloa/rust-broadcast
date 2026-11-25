//! Convert the real WebVTT fixture (`fixtures/subtitles/cap.vtt`) to SRT, and
//! report whether the conversion was lossless.
//!
//! Run with `cargo run -p caption-convert --example webvtt_to_srt`.

use broadcast_common::bounded::{Limit, read_file_bounded_string};

/// A whole WebVTT document: subtitle text, far below 4 MiB for any real
/// programme.
const WEBVTT_LIMIT: Limit = Limit::new("WebVTT document", 4 * 1024 * 1024);

fn main() {
    let path = test_fixtures::path("subtitles/cap.vtt");
    let vtt = read_file_bounded_string(&path, WEBVTT_LIMIT)
        .unwrap_or_else(|e| panic!("read {}: {e}", path.display()));

    let (srt, lossy) = caption_convert::webvtt_to_srt(&vtt).expect("valid WebVTT");
    println!("lossy: {lossy}");
    println!("--- SRT ---\n{srt}");

    // Round-trip back to WebVTT to show the pair is genuinely two-way.
    let back = caption_convert::srt_to_webvtt(&srt).expect("valid SRT");
    println!("--- back to WebVTT ---\n{back}");
}
