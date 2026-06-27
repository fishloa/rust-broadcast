# timed-metadata

[![crates.io](https://img.shields.io/crates/v/timed-metadata.svg)](https://crates.io/crates/timed-metadata)
[![docs.rs](https://img.shields.io/docsrs/timed-metadata)](https://docs.rs/timed-metadata)
[![License](https://img.shields.io/badge/license-MIT%20OR%20Apache--2.0-blue.svg)](../LICENSE-MIT)

Convert DPI / timed-metadata signalling between **SCTE-35**, **HLS
`EXT-X-DATERANGE`**, and **DASH `emsg`** — losslessly. `no_std`, independently
versioned.

## Install

```toml
[dependencies]
timed-metadata = "0.1"
```

## What is timed-metadata?

Ad insertion and content-segmentation signalling flows through three formats
across the OTT delivery chain:

| Format | Standard | Carrier |
|--------|----------|---------|
| **SCTE-35** `splice_info_section` | ANSI/SCTE 35 2023r1 | MPEG-2 TS |
| **HLS `EXT-X-DATERANGE`** | RFC 8216 / draft-pantos-hls-rfc8216bis §4.4.5.1 | HLS playlist |
| **DASH `emsg`** | SCTE 214-3; scheme `urn:scte:scte35:2013:bin` | MP4/CMAF segment |

`timed-metadata` translates between all three. Conversions are **lossless**: the
original `splice_info_section` bytes travel verbatim as the `SCTE35-OUT` hex in
DATERANGE or as the `message_data` payload in emsg. No re-encoding, no
interpretation loss.

## The three edges

### Edge 1 — SCTE-35 → HLS `EXT-X-DATERANGE`

```rust
use timed_metadata::{TimeAnchor, Timeline};

// Wall-clock anchor: PTS 0 == 2024-01-15T12:00:00 UTC.
let anchor = TimeAnchor { pts_90k: 0, utc_epoch_ms: 1_705_320_000_000 };
let mut timeline = Timeline::with_anchor(anchor);

// Real Unified Streaming splice ID 2002 (24-second break).
let splice_hex = "FC302100000000000000FFF01005000007D27FEF7F7E0020F580C0000000000088B9661D";
let raw: Vec<u8> = (0..splice_hex.len()).step_by(2)
    .map(|i| u8::from_str_radix(&splice_hex[i..i+2], 16).unwrap())
    .collect();

let event = timeline.push_scte35(&raw).unwrap();
let daterange = timeline.to_daterange(&event).unwrap();
println!("{}", daterange.to_tag_line());
// #EXT-X-DATERANGE:ID="2002",START-DATE="2024-01-15T12:00:00.000Z",
//   PLANNED-DURATION=24,SCTE35-OUT=0xFC302100...
```

### Edge 2 — SCTE-35 → DASH `emsg`

```rust
use mp4_emsg::PresentationTime;
use timed_metadata::{convert::EmsgConfig, TimeAnchor, Timeline};

let anchor = TimeAnchor { pts_90k: 0, utc_epoch_ms: 1_705_320_000_000 };
let mut timeline = Timeline::with_anchor(anchor);
let event = timeline.push_scte35(&raw).unwrap();

let cfg = EmsgConfig {
    timescale: 90_000,
    presentation: PresentationTime::Delta(0),
    event_duration: 2_160_000, // 24 s * 90000
    value: "34".to_string(),
    id: event.id.unwrap_or(0),
};
let emsg_bytes = timeline.to_emsg(&event, &cfg).unwrap();
// emsg_bytes is an ISO BMFF 'emsg' box ready to inject into a CMAF segment.
```

### Edge 3 — DASH `emsg` → SCTE-35

```rust
use timed_metadata::convert::emsg_to_scte35;

let splice_raw: Vec<u8> = emsg_to_scte35(&emsg_bytes).unwrap();
// splice_raw is the verbatim splice_info_section; feed to scte35_splice::SpliceInfoSection.
```

## Timeline session

[`Timeline`] is a stateful conversion session. It holds a [`TimeAnchor`]
(a PTS↔UTC mapping) and unrolls 33-bit PTS wrap so timestamps stay
monotonically increasing across a long stream.

- [`Timeline::push_scte35`] — parse a section, unroll PTS, return a [`TimedEvent`].
- [`Timeline::to_daterange`] — convert to [`DateRange`] (requires an anchor).
- [`Timeline::to_emsg`] — serialize the event as an `emsg` box.

The pure conversion functions live in [`convert`] for use without a session.

## Features

| Feature | Default | Description |
|---------|---------|-------------|
| `std`   | yes     | Enable `std` in all dependencies. |
| `serde` | yes     | `Serialize`/`Deserialize` for all public types. |
| `chrono`| yes     | `chrono` dependency (future wall-clock helpers). |

`no_std` + `alloc` when built with `default-features = false`. All conversions
are available in `no_std` mode.

## Spec citations

- **ANSI/SCTE 35 2023r1** — `splice_info_section` wire format.
- **RFC 8216 / draft-pantos-hls-rfc8216bis §4.4.5.1** — `EXT-X-DATERANGE`.
- **SCTE 214-3** — SCTE-35 binary carriage in DASH `emsg`; scheme
  `urn:scte:scte35:2013:bin`.
- **ISO/IEC 23009-1 §5.10.3.3** — ISOBMFF Event Message Box (`emsg`).

## License

Licensed under either of [MIT](../LICENSE-MIT) or [Apache-2.0](../LICENSE-APACHE)
at your option.
