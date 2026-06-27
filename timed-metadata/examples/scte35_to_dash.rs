//! Demonstrate SCTE-35 → DASH `emsg` conversion.
//!
//! Parses a hex `splice_info_section` (Unified Streaming splice ID 2002),
//! builds a [`Timeline`], and serializes the event as a DASH `emsg` box,
//! printing the hex bytes.
//!
//! Run with:
//! ```text
//! cargo run -p timed-metadata --example scte35_to_dash
//! ```

use mp4_emsg::PresentationTime;
use timed_metadata::{convert::EmsgConfig, TimeAnchor, Timeline};

fn main() {
    // Real Unified Streaming splice: ID 2002, out-of-network, break_duration 24 s.
    let hex = "FC302100000000000000FFF01005000007D27FEF7F7E0020F580C0000000000088B9661D";
    let raw: Vec<u8> = (0..hex.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&hex[i..i + 2], 16).unwrap())
        .collect();

    let anchor = TimeAnchor {
        pts_90k: 0,
        utc_epoch_ms: 1_705_320_000_000,
    };
    let mut timeline = Timeline::with_anchor(anchor);
    let event = timeline.push_scte35(&raw).expect("valid splice");

    // Build emsg config: 90 kHz timescale, segment-relative delta 0, 24 s duration.
    let cfg = EmsgConfig {
        timescale: 90_000,
        presentation: PresentationTime::Delta(0),
        event_duration: 2_160_000, // 24s * 90000
        value: "34".to_string(),   // segmentation_type_id for Provider Ad Start
        id: event.id.unwrap_or(0),
    };

    let emsg_bytes = timeline
        .to_emsg(&event, &cfg)
        .expect("SCTE-35-sourced event");

    println!("Event kind  : {}", event.kind);
    println!("Event id    : {:?}", event.id);
    println!("emsg length : {} bytes", emsg_bytes.len());
    println!();
    println!(
        "emsg hex: {}",
        emsg_bytes
            .iter()
            .map(|b| format!("{:02X}", b))
            .collect::<Vec<_>>()
            .join("")
    );

    // Verify round-trip: extract the splice from the emsg and compare.
    use timed_metadata::convert::emsg_to_scte35;
    let extracted = emsg_to_scte35(&emsg_bytes).expect("round-trip");
    assert_eq!(extracted, raw, "splice bytes must survive verbatim");
    println!();
    println!("Round-trip verified: splice bytes survive verbatim in emsg message_data.");
}
