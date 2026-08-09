//! Convert the real WebVTT fixture (`fixtures/sub/cap.vtt`) to SRT, and
//! report whether the conversion was lossless.
//!
//! Run with `cargo run -p caption-convert --example webvtt_to_srt`.

use std::fs;
use std::path::Path;

fn main() {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("fixtures")
        .join("sub")
        .join("cap.vtt");
    let vtt = fs::read_to_string(&path).unwrap_or_else(|e| panic!("read {}: {e}", path.display()));

    let (srt, lossy) = caption_convert::webvtt_to_srt(&vtt).expect("valid WebVTT");
    println!("lossy: {lossy}");
    println!("--- SRT ---\n{srt}");

    // Round-trip back to WebVTT to show the pair is genuinely two-way.
    let back = caption_convert::srt_to_webvtt(&srt).expect("valid SRT");
    println!("--- back to WebVTT ---\n{back}");
}
