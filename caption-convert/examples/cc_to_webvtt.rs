//! Convert the real CEA-608 CC1 fixture (`fixtures/cc/cea608_cc1_synthetic.txt`)
//! to a standalone WebVTT document, using `caption_convert::Cea608ToWebVtt`
//! -- a raw-`cc_data()`-bytes-in wrapper over `timed-metadata`'s
//! roll-up/pop-on/paint-on cue extraction (issue #568).
//!
//! Run with `cargo run -p caption-convert --example cc_to_webvtt`.

use caption_convert::Cea608ToWebVtt;
use cc_data::decode::Cea608Channel;
use std::fs;
use std::path::Path;

fn fixture_path() -> std::path::PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("fixtures")
        .join("cc")
        .join("cea608_cc1_synthetic.txt")
}

/// Parse the fixture into `(pts_90k, cc_data_bytes)` frames, skipping
/// comment/blank lines -- the same format `timed-metadata`'s own fixture
/// test uses (see its doc comment for the fixture's provenance).
fn load_frames() -> Vec<(u64, Vec<u8>)> {
    let text = fs::read_to_string(fixture_path()).expect("read cea608_cc1_synthetic.txt fixture");
    let mut frames = Vec::new();
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let mut parts = line.split_whitespace();
        let pts: u64 = parts
            .next()
            .expect("pts field")
            .parse()
            .expect("pts is a u64");
        let hex = parts.next().expect("hex field");
        let bytes = (0..hex.len())
            .step_by(2)
            .map(|i| u8::from_str_radix(&hex[i..i + 2], 16).expect("valid hex byte"))
            .collect();
        frames.push((pts, bytes));
    }
    frames
}

fn main() {
    let frames = load_frames();
    println!("loaded {} cc_data() frames from the fixture", frames.len());

    let mut conv = Cea608ToWebVtt::new(Cea608Channel::Cc1);
    for (pts, bytes) in &frames {
        conv.push_cc_data(*pts, bytes)
            .expect("valid cc_data() Table B.9 bytes");
    }
    conv.finalize(45_000);

    let vtt = conv.into_webvtt();
    println!("--- WebVTT ---\n{vtt}");
}
