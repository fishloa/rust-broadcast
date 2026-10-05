# timed-metadata 0.6.0

_Released 2026-10-05._

### Changed
- `webvtt::teletext` now calls `dvb_vbi::TeletextDataField::txt_data_block_logical()`
  instead of its own private `.map(u8::reverse_bits)` (issue #1106); no
  behaviour change, same bytes.
- RFC 3339 formatting uses `jiff` (`no_std` + `alloc`; `jiff/std` rides the `std` feature); output is byte-identical for every instant in years -9999..=9999, including the historical `{year:04}` spelling of negative years (`-001-12-31T23:59:59.999Z`) (the calendar maths goes through `jiff::civil::DateTime`, whose range is the full year 9999, not `jiff::Timestamp`, which stops at 9999-12-30T22:00Z). `format_rfc3339_ms`/`TimeAnchor::rfc3339` now CLAMP an out-of-range instant instead of printing a many-digit year; `TimeAnchor::media_to_epoch_ms` saturates (was: sign-wrapping `u64 as i64` casts and a debug-build overflow panic); `convert::scte35_to_daterange` returns `Error::TimestampOutOfRange` for an unrepresentable `START-DATE`.

### Changed (breaking)
- New `Error` variants `EmsgPresentationTimeOverflow` and `UnsupportedPresentationTime` (`Error` is `#[non_exhaustive]`; #1105).
- **`daterange::DateRange::to_tag_line` now returns `Result<String>`**
  (was `String`) — issue #1140 / audit r12-TM-W4 (T12): `ID`, `CLASS` and
  the `SCTE35-*` hex token are now built through
  `broadcast_hls::AttrValue`'s checked constructors and rendered via the
  new shared `broadcast_hls::render_attribute_list`, instead of
  hand-formatting `,NAME="VALUE"` with no validation. `ID`/`CLASS` are
  frequently sourced from an upstream SCTE-35 segmentation descriptor's
  `segmentation_upid` (network data, not this crate's own), so a `"`, CR
  or LF in either previously broke the attribute list; it is now `Err`.
  `DURATION`/`PLANNED-DURATION` are also rejected (the new
  `Error::InvalidDuration`) if NaN, infinite, or negative, both on parse
  and on render. The crate's own quoted-comma attribute splitter is
  replaced by the shared `broadcast_hls::parse_attribute_list` (the same
  tokenizer `ssai-runtime` now also uses — audit r14-SSAI-O1 found this
  algorithm duplicated three times across the workspace). New dependency:
  `broadcast-hls` (path, default-features = false).
- **`daterange::DateRange` gains a new public field
  `extra_attrs: Vec<(String, AttrValue)>`** (coordinator follow-up to
  r12-TM-W4): `parse_tag_line` previously dropped every attribute it
  didn't model (`X-*` caller extensions, `END-DATE`, `END-ON-NEXT`, …) —
  the other half of TM-W4's "lossy round-trip" finding. Unknown
  attributes are now preserved (sorted by name, same pattern as
  `broadcast_hls`'s own `extra_attrs` fields) and rendered back after the
  fixed-order fields, so a real `EXT-X-DATERANGE` with unrecognized
  attributes now round-trips byte-identically instead of losing them
  silently.

### Fixed
- `convert::emsg_to_v1`/`emsg_to_v0`: a v0 emsg whose `earliest_presentation_time + presentation_time_delta` overflows `u64` is now `Error::EmsgPresentationTimeOverflow` (was an unchecked add: debug panic / release wrap), and an unknown `#[non_exhaustive]` `PresentationTime` variant is `Error::UnsupportedPresentationTime` instead of `unreachable!` (audit r12-TM-W2/W3, #1105).
- `webvtt::writer::cue_block`: an empty line inside a cue's text (`"a\n\nb"`, and with lone-CR/CRLF terminators) no longer emits a blank line, which ended the cue block early and corrupted the rest of the document (WebVTT §4.1; audit r12-TM-W5, #1105).
- `daterange` hex rendering uses a lookup table instead of a `format!` per byte (audit r12-TM-O1, #1105).
- **#1039**: `TimedEvent::from_scte35` ignored `splice_info_section`'s
  `pts_adjustment`, so every derived `MediaTime`/DATERANGE `START-DATE` was
  off by that adjustment (SCTE 35 §9.6.1). Every `pts_time` is now shifted
  through `broadcast_common::clock33::add` before use.
- **#1040**: a `time_signal()` cue (the dominant modern SCTE-35 form) lost
  its time, id and kind entirely — `from_scte35` only read `splice_insert`.
  It now reads `TimeSignal.splice_time.pts_time` and takes id/kind/duration
  from the cue's first (uncancelled) `segmentation_descriptor`.
  `scte35_to_daterange` now errors instead of emitting `ID=""` for a cue
  with no id (RFC 8216bis §4.4.5.1 requires unique IDs). Only the
  segmentation types that actually leave/return to network programming for
  ad insertion (Table 23's `Break`, `Provider`/`DistributorAdvertisement`,
  `Provider`/`DistributorPlacementOpportunity` — **not** the `Overlay`
  variants, which composite over the network feed rather than break away
  from it — and `Provider`/`DistributorAdBlock`) map to
  `BreakStart`/`BreakEnd`; the program and chapter boundary types
  (`ProgramStart`/`ProgramEnd`/`ChapterStart`/`ChapterEnd`) map to
  `EventKind::Chapter`; credits, promos, unscheduled/alternate content,
  overlay placement opportunities, and `NetworkStart`/`NetworkEnd` map to
  `Unspecified` (id/time/duration are still populated) so a consumer that
  splices ads on `BreakStart` cannot mistake one of those for an ad avail.
- **#1041**: the Teletext decoder ran Hamming-8/4 and odd-parity FEC
  directly on `dvb_vbi::TeletextDataField::txt_data_block` bytes without
  reversing them first. EN 300 706 transmits each byte LSB-first, so a real
  DVB Teletext stream decoded to garbage or produced no cues at all. Bytes
  are now bit-reversed before FEC decode; the fixture
  (`fixtures/teletext/teletext_subtitle_synthetic.txt`) is regenerated in
  the corrected (wire) bit order. Independently verified against TSDuck
  3.44's own Teletext demux (`tsp -P teletext`) over a from-scratch real
  PAT/PMT/PES TS fixture (`fixtures/teletext/teletext_subtitle_boxed.ts`,
  `tests/webvtt_teletext_tsduck_oracle.rs`).
- A cancelled `splice_insert` (`splice_event_cancel_indicator == true`) was
  classified as `BreakEnd` (spurious `SCTE35-IN`); it is now `Unspecified`.
- `DateRange::parse_tag_line` no longer panics on a multi-byte character inside a `SCTE35-OUT/IN/CMD` hex value and no longer accepts a `+`/`-` sign as a hex digit (the hex now goes through the `hex` crate).

### Added
- `anchor::try_format_rfc3339_ms`, `TimeAnchor::try_rfc3339` and `Error::TimestampOutOfRange(i64)` (new variant on the `#[non_exhaustive]` enum): the fallible RFC 3339 formatters.

---

Published from tag `timed-metadata-v0.6.0`.
