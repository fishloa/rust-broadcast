//! Demonstrate SCTE-35 → HLS `#EXT-X-DATERANGE` conversion.
//!
//! Parses a hex `splice_info_section` (Unified Streaming splice ID 2002),
//! builds a [`Timeline`] with a wall-clock [`TimeAnchor`], and emits the
//! `#EXT-X-DATERANGE:` tag line.
//!
//! Run with:
//! ```text
//! cargo run -p timed-metadata --example scte35_to_hls
//! ```

use timed_metadata::{TimeAnchor, Timeline};

fn main() {
    // Real Unified Streaming splice: ID 2002, out-of-network, break_duration 24 s.
    let hex = "FC302100000000000000FFF01005000007D27FEF7F7E0020F580C0000000000088B9661D";
    let raw: Vec<u8> = (0..hex.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&hex[i..i + 2], 16).unwrap())
        .collect();

    // Wall-clock anchor: assume PTS 0 == 2024-01-15T12:00:00.000Z (ms since epoch).
    let utc_epoch_ms: i64 = 1_705_320_000_000; // 2024-01-15 12:00:00 UTC
    let anchor = TimeAnchor {
        pts_90k: 0,
        utc_epoch_ms,
    };

    let mut timeline = Timeline::with_anchor(anchor);

    // Parse the SCTE-35 section and unroll PTS wrap.
    let event = timeline.push_scte35(&raw).expect("valid splice");

    // Convert to a DATERANGE.
    let daterange = timeline.to_daterange(&event).expect("anchor is set");

    println!("Event kind  : {}", event.kind);
    println!("Event id    : {:?}", event.id);
    println!(
        "Duration    : {:?}s",
        event.duration.map(|d| d.as_seconds_f64())
    );
    println!();
    println!("{}", daterange.to_tag_line());
}
