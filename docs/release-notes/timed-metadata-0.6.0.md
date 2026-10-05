# timed-metadata 0.6.0

_Released 2026-10-05._

**Minor-epoch breaking release (0.x), with several SCTE-35 conversion fixes that change output.** `DateRange::to_tag_line` now returns `Result<String>`, `DateRange` gains a public `extra_attrs` field, and `scte35-splice` moves to the 3.0 epoch (its types appear in this crate's public API, for example `TimedEvent::from_scte35`). The fixes matter more than the breaks for most users: `time_signal()` cues were previously lost, every SCTE-35 time was off by `pts_adjustment`, and DVB Teletext decoded to garbage. Output for the same input will differ. Act if you call `to_tag_line`, build `DateRange` with a struct literal, or use SCTE-35 conversion. Read with [scte35-splice 3.0.0](scte35-splice-3.0.0.md), [broadcast-common 9.4.0](broadcast-common-9.4.0.md) and [mp4-emsg 0.4.1](mp4-emsg-0.4.1.md).

## Dependency and feature changes

```toml
-scte35-splice = { version = "2.1", default-features = false }
+scte35-splice = { version = "3.0", default-features = false }
-broadcast-common = { version = "9.3", default-features = false }
+broadcast-common = { version = "9.4", default-features = false }
-cc-data = { version = "0.5", default-features = false, optional = true }
+cc-data = { version = "0.6", default-features = false, optional = true }
+broadcast-hls = { version = "0.3", default-features = false }                 # new
+jiff  = { version = "0.2", default-features = false, features = ["alloc"] }   # new, RFC 3339 formatting
+hex   = { version = "0.4", default-features = false, features = ["alloc"] }   # new, SCTE35-* hex

-std   = ["scte35-splice/std", "mp4-emsg/std", "chrono?/std", "serde?/std", "cc-data?/std", "dvb-vbi?/std"]
+std   = ["jiff/std", "scte35-splice/std", "mp4-emsg/std", "chrono?/std", "serde?/std", "cc-data?/std", "dvb-vbi?/std", "broadcast-hls/std"]
-serde = ["dep:serde", "scte35-splice/serde", "mp4-emsg/serde", "cc-data?/serde", "dvb-vbi?/serde"]
+serde = ["dep:serde", "scte35-splice/serde", "mp4-emsg/serde", "cc-data?/serde", "dvb-vbi?/serde", "broadcast-hls/serde"]
```

All new dependencies are `no_std` + `alloc`, so the crate stays `no_std`. `broadcast-hls` is a new dependency edge; it is also what `ssai-runtime` uses for the same attribute-list tokenizer.

## Breaking changes

### 1. `DateRange::to_tag_line` returns `Result<String>` (#1140, audit r12-TM-W4)

`ID`, `CLASS` and the `SCTE35-*` hex token are now built through `broadcast_hls::AttrValue`'s checked constructors and rendered with the shared `broadcast_hls::render_attribute_list`, instead of hand-formatting `,NAME="VALUE"` with no validation. `ID` and `CLASS` are often sourced from an upstream SCTE-35 `segmentation_upid` (network data), so a `"`, CR or LF in either used to break the attribute list; it is now an `Err`. `DURATION` and `PLANNED-DURATION` that are NaN, infinite or negative are rejected with the new `Error::InvalidDuration { what, value }`, both on parse and on render (NaN previously rendered as the bare token `NaN`). The crate's own quoted-comma splitter is replaced by `broadcast_hls::parse_attribute_list`.

```rust
// before
let line: String = range.to_tag_line();
// after
let line: String = range.to_tag_line()?;
```

### 2. `DateRange` gains `extra_attrs: Vec<(String, AttrValue)>`

`parse_tag_line` used to drop every attribute it did not model (`X-*` client extensions, `END-DATE`, `END-ON-NEXT`, and so on). Unknown attributes are now preserved, sorted by name, and rendered back after the fixed-order fields, so a real `EXT-X-DATERANGE` with unrecognised attributes round-trips byte-identically. Struct-literal construction of `DateRange` needs the new field (`extra_attrs: Vec::new()` keeps the old behaviour).

### 3. New `Error` variants

`EmsgPresentationTimeOverflow`, `UnsupportedPresentationTime` (#1105), `InvalidDuration` and `TimestampOutOfRange(i64)`. `Error` is `#[non_exhaustive]`, so a wildcard arm already covers them.

## Behaviour changes that change output

- SCTE-35 time signals (#1040). A `time_signal()` cue, the dominant modern form, lost its time, id and kind entirely, because `TimedEvent::from_scte35` only read `splice_insert`. It now reads `TimeSignal.splice_time.pts_time` and takes id, kind and duration from the cue's first uncancelled `segmentation_descriptor`. `scte35_to_daterange` now errors instead of emitting `ID=""` for a cue with no id (RFC 8216bis §4.4.5.1 requires unique IDs).
- Event-kind classification from segmentation types. Only the types that leave or return to network programming for ad insertion map to `BreakStart` / `BreakEnd`: Table 23's `Break`, `Provider`/`Distributor` `Advertisement`, `Provider`/`Distributor` `PlacementOpportunity` (not the `Overlay` variants, which composite over the network feed) and `Provider`/`Distributor` `AdBlock`. `ProgramStart`, `ProgramEnd`, `ChapterStart` and `ChapterEnd` map to `EventKind::Chapter`. Credits, promos, unscheduled or alternate content, overlay placement opportunities and `NetworkStart`/`NetworkEnd` map to `Unspecified` (id, time and duration are still populated), so a consumer that splices ads on `BreakStart` cannot mistake them for an ad avail.
- `pts_adjustment` (#1039). `TimedEvent::from_scte35` ignored `splice_info_section`'s `pts_adjustment`, so every derived `MediaTime` and DATERANGE `START-DATE` was off by that adjustment (SCTE 35 §9.6.1). Every `pts_time` is now shifted through `broadcast_common::clock33::add`.
- A cancelled `splice_insert` (`splice_event_cancel_indicator == true`) was classified as `BreakEnd`, producing a spurious `SCTE35-IN`; it is now `Unspecified`.
- Teletext (#1041). The decoder ran Hamming-8/4 and odd-parity FEC directly on `dvb_vbi::TeletextDataField::txt_data_block` bytes without reversing them. EN 300 706 transmits each byte LSB-first, so a real DVB Teletext stream decoded to garbage or produced no cues. Bytes are now bit-reversed first, through `TeletextDataField::txt_data_block_logical()` rather than a private `reverse_bits` map (#1106). The synthetic fixture was regenerated in wire bit order and independently verified against TSDuck 3.44's `tsp -P teletext` over a real PAT/PMT/PES fixture (`tests/webvtt_teletext_tsduck_oracle.rs`).
- RFC 3339 formatting now uses `jiff`. Output is byte-identical for every instant in years -9999..=9999, including the historical `{year:04}` spelling of negative years (`-001-12-31T23:59:59.999Z`); the calendar maths goes through `jiff::civil::DateTime`, since `jiff::Timestamp` stops at 9999-12-30T22:00Z. Out-of-range instants now behave differently: `format_rfc3339_ms` and `TimeAnchor::rfc3339` clamp instead of printing a many-digit year, `TimeAnchor::media_to_epoch_ms` saturates (it used sign-wrapping `u64 as i64` casts and could panic with overflow in a debug build), and `convert::scte35_to_daterange` returns `Error::TimestampOutOfRange` for an unrepresentable `START-DATE`.

## New

- `anchor::try_format_rfc3339_ms`, `TimeAnchor::try_rfc3339` and `Error::TimestampOutOfRange(i64)`: fallible RFC 3339 formatters, for callers that prefer an error to clamping.

## Fixes

- `convert::emsg_to_v1` / `emsg_to_v0`: a v0 emsg whose `earliest_presentation_time + presentation_time_delta` overflows `u64` is now `Error::EmsgPresentationTimeOverflow` (was an unchecked add: debug panic, release wrap), and an unknown `#[non_exhaustive]` `PresentationTime` variant is `Error::UnsupportedPresentationTime` instead of `unreachable!` (audit r12-TM-W2/W3, #1105).
- `webvtt::writer::cue_block`: an empty line inside cue text (`"a\n\nb"`, also with lone-CR and CRLF terminators) no longer emits a blank line, which ended the cue block early and corrupted the rest of the document (WebVTT §4.1; audit r12-TM-W5, #1105).
- `DateRange::parse_tag_line` no longer panics on a multi-byte character inside a `SCTE35-OUT/IN/CMD` hex value, and no longer accepts a `+` or `-` sign as a hex digit (the hex now goes through the `hex` crate).
- `daterange` hex rendering uses a lookup table instead of a `format!` per byte (audit r12-TM-O1).

---

Published from tag `timed-metadata-v0.6.0`.
