# timed-metadata v0.1.0 — 2026-06-27

## What is timed-metadata?

`timed-metadata` is a new independently-versioned crate that translates
DPI / timed-metadata signalling between the three formats that ad-insertion
and content-segmentation signalling flows through across an OTT delivery chain:

- **SCTE-35** `splice_info_section` — the MPEG-2 TS wire format (ANSI/SCTE 35 2023r1).
- **HLS `EXT-X-DATERANGE`** — the playlist tag (RFC 8216 / draft-pantos-hls-rfc8216bis §4.4.5.1).
- **DASH `emsg`** — the ISO BMFF inband event box (SCTE 214-3; scheme `urn:scte:scte35:2013:bin`).

All conversions are **lossless**: the original `splice_info_section` bytes
travel verbatim — as the `SCTE35-OUT` hex value in a DATERANGE tag and as the
`message_data` payload in an emsg box.

## What's in v0.1.0

### Core event model

**`TimedEvent`** is the canonical hub. It carries:
- `id` — `splice_event_id` or emsg `id`.
- `kind` — abstract `EventKind` (`BreakStart`, `BreakEnd`, `Chapter`, `Unspecified`).
- `at` — optional `MediaTime` (90 kHz, PTS wrap-unrolled by `Timeline`).
- `duration` — optional `MediaDuration`.
- `source` — `SourcePayload::Scte35 { raw }` or `SourcePayload::Emsg { scheme_id_uri, value, raw }`.

### Wall-clock anchor

**`TimeAnchor`** pins a 90 kHz PTS to a UTC wall-clock millisecond, enabling
`START-DATE` computation for DATERANGE tags. `rfc3339()` maps any `MediaTime`
to an ISO-8601 string without pulling in `chrono`.

### DateRange model

**`DateRange`** models the `EXT-X-DATERANGE` tag with typed fields:
`ID`, `START-DATE`, `CLASS`, `DURATION`, `PLANNED-DURATION`, and a
`Scte35Attr { cue: Scte35Cue, raw }`. `to_tag_line()` / `parse_tag_line()`
are byte-identical round-trips.

### Pure conversion functions (`convert` module)

| Function | Edge |
|----------|------|
| `scte35_to_daterange(&TimedEvent, &TimeAnchor)` | SCTE-35 → DATERANGE |
| `scte35_to_emsg(&[u8], &EmsgConfig)` | SCTE-35 → emsg |
| `emsg_to_scte35(&[u8])` | emsg → SCTE-35 |

### Timeline session

**`Timeline`** is a stateful conversion session that holds the `TimeAnchor`
and unrolls 33-bit PTS wrap across a long stream. The standard usage pattern
is: `push_scte35()` → `to_daterange()` / `to_emsg()`.

### `no_std` + features

`timed-metadata` is `#![no_std]` with `extern crate alloc`. Features:
`std` (default), `serde` (default, Serialize+Deserialize for all public types),
`chrono` (default, reserved for future wall-clock helpers).

### Quality gates

- Full round-trip tests for all three edges.
- Real fixture interop tests (the Unified Streaming 2002 splice; real emsg bins).
- `label_coverage` drift-guard — CI fails if any public spec/field enum is added
  without `name()` + `impl_spec_display!`.

## Deferred (v0.2+)

- SCTE-104 ingest.
- ID3 timed metadata carriage.
- `segmentation_type_id`-based `EventKind` refinement.
- `chrono::DateTime` interop helpers.
