# Changelog — timed-metadata

All notable changes to this crate. Format: [Keep a Changelog](https://keepachangelog.com/).

## [Unreleased]

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

## [0.5.0] - 2026-08-11

### Changed
- MSRV raised to **1.95.0** (issue #949). This removes the workspace's MSRV
  split: `webrtc-runtime`'s optional `media` feature needed rustc 1.88 (via
  `rcgen`), which had grown a dedicated CI job, six `--exclude` lanes and a
  guard script to contain. Adopting let-chains and `is_multiple_of` where the
  1.95 lints require them; no functional or API change.
### Fixed
- `Timeline`'s 33-bit PTS unroll (internal `unroll_pts`, now `PtsUnroller`,
  built on `broadcast_common::clock33::unwrap_delta`) previously used a
  forward-only epoch counter that misclassified a small backward reorder
  straddling the wrap origin (e.g. raw tick `2` followed by raw tick
  `2^33 - 3`, a legitimate 5-tick backward step) as a huge forward jump
  instead. Switched to the same bidirectional wrap-correction `transmux`'s
  demux-edge unroller already used, which gets both directions right; also
  applies to the caption/Teletext diff-based cue boundary tracker
  (`webvtt::cue::DiffState`), which shared the same internal helper. No
  public API change (both were crate-internal); this is a pure internal
  duplication-audit consolidation plus the bug fix it surfaced. This edge
  case requires a pathological input (a second SCTE-35 cue or caption event
  legitimately dipping backward across the wrap origin relative to the
  first) and is not expected to affect ordinary broadcast captures, where
  cue/event PTS values only ever move forward.
- Doc accuracy (#941 row 1): the crate-root doc comment claimed SCTE-35 is
  translated "to and from" both `EXT-X-DATERANGE` and `emsg`. Only the
  `emsg` conversion is bidirectional; `EXT-X-DATERANGE` conversion is
  one-way (`scte35_to_daterange` only) — the reverse edge is out of scope.
  No behaviour change; no data loss either way (raw payloads preserved).

## [0.4.2] - 2026-07-30

### Added
- `tests/non_exhaustive_coverage.rs` drift guard (issue #806). No public API
  or behaviour change.

### Removed
- Dev-only: the `ssai_ad_stitch` example + its integration test (issue #812).
  The 0.4.1 "move" from `transmux` copied the file here but never deleted
  `transmux`'s own copy, so the two crates ended up shipping a byte-identical
  24 KB example under the same name — a cargo output-filename collision.
  `transmux`'s manifest already documents this example as the reason for its
  `scte35-splice`/`timed-metadata` dev-deps, and this crate's own half of the
  SSAI story (SCTE-35 -> `EXT-X-DATERANGE`, SCTE-35 -> `emsg`, both with
  round-trip verification) is already demonstrated more directly by the
  existing `scte35_to_hls`/`scte35_to_dash` examples, so the duplicate here
  was dropped rather than re-split. Dropped the now-unused `mpeg-ts`
  dev-dependency it needed. No public API or behaviour change.

## [0.4.1] - 2026-07-27

### Changed
- Dev-only: the `ssai_ad_stitch` example + its integration test moved here
  from `transmux` (part of the transmux<->timed-metadata circular dev-dep
  fix), and `webvtt_sei_caption_fixture`'s test helper was adapted to read
  `Sample::pts` directly instead of reconstructing presentation time from
  `Track::start_decode_time` + a running duration sum + `composition_offset`
  — following transmux's media-plane step 2c, where `Sample::dts`/`pts`
  became absolute and optional. New `mpeg-ts` dev-dependency (for the moved
  example's `SectionReassembler` use). No public API or behaviour change to
  the library itself.

## [0.4.0] - 2026-07-13

### Added
- **EBU Teletext (ETSI EN 300 706) subtitle decode -> WebVTT** (feature
  `teletext`, off by default; issue #666): `webvtt::TeletextCueExtractor`
  turns an EN 300 706 Level-1 subtitle page into a `Cue` sequence, fed
  `dvb_vbi::TeletextDataField`s (EN 301 775 §4.5, carriage-only — `dvb-vbi`
  itself does not decode EN 300 706, by its own documented scope). The new
  `webvtt::teletext` module owns that decode entirely: Hamming-8/4 FEC
  (`decode_hamming_8_4`/`encode_hamming_8_4`, §8.2 — single-bit errors
  corrected, double-bit errors rejected, implemented as a brute-force
  nearest-codeword search proven equivalent to the spec's "four parity
  tests" via the code's minimum distance of 4), odd-parity FEC
  (`decode_odd_parity`/`encode_odd_parity`, §8.1 — detect-only, corrupted
  bytes render as `'\u{FFFD}'`), the `NationalOption` C12/C13/C14 selector
  (all 8 values decoded/labelled; only `English`'s Table 36 character
  substitutions are implemented — a documented gap for the other 7), page
  header decode (`PageHeader`, Table 2's full eleven control bits + page
  number/sub-code), and basic Level-1 page composition (header + rows
  1-24, erase-page/inhibit-display handling). See
  `timed-metadata/docs/teletext-subtitles.md` for the full spec citations
  (including the Table 35/36 Latin G0 glyph charts, which render as bitmap
  images in the PDF and were read visually, not machine-extracted) and the
  architectural rationale for placing this decode in `timed-metadata`
  rather than `dvb-vbi`. Validated against a synthetic-but-spec-real fixture
  (`fixtures/teletext/teletext_subtitle_synthetic.txt` — no real DVB
  VBI-teletext capture or spec worked example was available; the fixture is
  constructed via this crate's own verified Hamming/parity encoders) with
  mutation-bite tests proving both FEC paths (Hamming correction, parity
  detection) actually run.

- **SEI-carried caption input wired to `webvtt`** (#599, follow-up to #568):
  the `Cea608CueExtractor`/`Cea708CueExtractor` API is unchanged — it already
  consumed carriage-agnostic `cc_data::CcTriplet` slices — but this release
  proves and tests the second carriage source, `transmux::nal::caption_cc_data`
  (ATSC A/53 caption SEI in H.264/HEVC access units), converges on the exact
  same cues as the PES `cc_data()` path (#568): the same committed
  `fixtures/cc/cea608_cc1_synthetic.txt` frames, re-wrapped in an SEI NAL
  instead of fed raw, produce byte-identical `Cue`s. Also validated against a
  real ATSC A/53 caption SEI capture (dev-dependency on `transmux` for its
  `TsDemux` + `caption_cc_data`, test-only), decoded text cross-checked
  against an independent `ffmpeg`-derived oracle.

## [0.3.0] - 2026-07-04
### Added
- **`webvtt`** module (feature `cc-data`, off by default): converts CEA-608/708
  closed captions to WebVTT cues (#568). `Cea608CueExtractor` /
  `Cea708CueExtractor` wrap `cc-data`'s decode-only 608/CC1 and 708-service
  models and derive cue start/end boundaries by diffing the decoded displayed
  text after each fed access-unit frame (pop-on boundaries land exactly on
  EOC/erase since `cc-data` only mutates the *displayed* buffer on those
  commands; roll-up/paint-on boundaries are best-effort per visible-text
  change — documented as a known simplification). `Cue` + `write_document` /
  `write_segment` (always available, no `cc-data` dependency) render W3C
  WebVTT §4 cue blocks and RFC 8216 §3.5 HLS segmented output with
  `X-TIMESTAMP-MAP=MPEGTS:<n>,LOCAL:00:00:00.000`, reusing `Timeline`'s
  33-bit PTS wrap-unroll. Lossy by design: no cue `line`/`position`/`align`
  settings and no inline styling (`<i>`/`<u>`/`<c>`) are emitted in this first
  pass — see the module docs for the full list of documented losses.
  Validated against a synthetic-but-spec-real CEA-608 CC1 fixture
  (`fixtures/cc/cea608_cc1_synthetic.txt`, CTA-608-E control/PAC/char codes)
  covering pop-on, roll-up, and paint-on; emitted WebVTT additionally
  cross-checked against `ffmpeg` when available.

## [0.2.0] - 2026-07-03
### Changed
- Rust **edition 2024**; MSRV raised to **1.86**; format-argument modernisation. No functional or API change.

## [0.1.2] — 2026-07-01
### Added
- emsg version 0 ↔ version 1 conversion (`emsg_to_v0` / `emsg_to_v1` + `SegmentTiming`),
  recomputing the timing field against the segment EPT (`T = EPT + delta` ==
  `presentation_time`), honouring `timescale` equality and carrying PTO for
  Movie↔Period alignment (ISO/IEC 23009-1:2022 §5.10.3.3). Byte-identical
  round-trip verified against real v0 + v1 (DASH-IF livesim) SCTE-35 emsg fixtures.

## 0.1.1 — 2026-06-29

### Changed
- Dependency `broadcast-common` bump (renamed from `dvb-common`); no API change.

## 0.1.0 — 2026-06-27

Initial release.

### Added

- **`TimedEvent`** — canonical hub type carrying the event's abstract kind
  (`EventKind`: `BreakStart`, `BreakEnd`, `Chapter`, `Unspecified`), optional
  media time, duration, and the lossless verbatim source payload (`SourcePayload::Scte35`
  / `SourcePayload::Emsg`).
- **`TimeAnchor`** — maps a 90 kHz PTS to a UTC wall-clock instant; `rfc3339()`
  converts any `MediaTime` to an ISO-8601 string.
- **`DateRange`** — typed `EXT-X-DATERANGE` model with `to_tag_line()` /
  `parse_tag_line()` (RFC 8216 / draft-pantos-hls-rfc8216bis §4.4.5.1).
- **`convert::scte35_to_daterange`** — pure SCTE-35 → DATERANGE edge.
- **`convert::scte35_to_emsg`** / **`convert::emsg_to_scte35`** — pure
  SCTE-35 ↔ DASH `emsg` edges (SCTE 214-3, scheme `urn:scte:scte35:2013:bin`).
- **`Timeline`** — stateful session: holds the `TimeAnchor`, unrolls 33-bit PTS
  wrap, and exposes `push_scte35` / `to_daterange` / `to_emsg`.
- `no_std` + `alloc`; features: `std` (default), `serde` (default), `chrono` (default).
- `label_coverage` drift-guard (CI gate for `EventKind` / `Scte35Cue` labels).

### Deferred (v0.2+)

- SCTE-104 ingest.
- ID3 timed metadata carriage.
- `segmentation_type_id`-based `EventKind` refinement beyond binary out/in.
- `chrono::DateTime` interop helpers behind the `chrono` feature.
