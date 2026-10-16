---
status: DRAFT
---
# caption-convert 0.3.0

_Released 2026-10-16._

**Breaking release.** 608 -> WebVTT/SRT now converts real captures that re-issue RCL before EDM/EOC, WebVTT/SRT timestamp parsing is digits-only and shared with `timed-metadata`, and `matrix::lookup` no longer panics.

## Why this version

- **Proposed version:** 0.2.0 -> **0.3.0** (major-class: 0.x minor-epoch bump).
- **Own API/behaviour change class:** major (matrix::lookup returns Option).
- **cargo-semver-checks vs live 0.2.0 (all features):** none flagged. (Lint-based: it misses type changes of existing fields/arguments, so the CHANGELOG BREAKING list is the primary evidence.)
- **Epoch cascade:** none needed (own breaking change). Siblings it depends on that move epoch in this wave: broadcast-common, cc-data, dvb-vbi, timed-metadata.
- **Caret epoch:** 0.2 -> 0.3; every workspace dependent must move with it.

## Dependency changes

```toml
-broadcast-common = "9.4"
+broadcast-common = "10.0"
-cc-data = "0.6"
+cc-data = "0.7"
-dvb-vbi = "0.4"
+dvb-vbi = "0.5"
-timed-metadata = "0.6"
+timed-metadata = "0.7"
```

## Breaking changes and migration

```rust
// before
let entry: &MatrixEntry = matrix::lookup(from, to);     // panicked on a missing row
// after
let entry: Option<&'static MatrixEntry> = matrix::lookup(from, to);
```

`check` maps the absent row to `Error::Unsupported { support: NotImplemented, .. }`.

## What changed (from CHANGELOG `[Unreleased]`)

#### Changed
- Internal: the five finishing methods shared by `Cea608ToWebVtt`, `Cea708ToWebVtt` and `TeletextToWebVtt` come from one `converter_tail!` macro instead of three copies; public API unchanged (#1218).

#### Fixed
- 608 -> WebVTT/SRT: captions loaded with a repeated RCL before EDM/EOC are converted instead of dropped, and same-PTS caption updates no longer yield zero-length cues (fixes in `cc-data` / `timed-metadata`, covered here by real-capture tests) (#1220).

#### Changed (BREAKING)
- `matrix::lookup` returns `Option<&'static MatrixEntry>` instead of panicking (`.expect`) when the matrix has no row for a `(from, to)` pair; both format enums are `#[non_exhaustive]`, so a newly added format could reach the old panic. `check` maps the absent row to `Error::Unsupported { support: NotImplemented, .. }` (#1202).

#### Changed
- Internal: `time.rs` (the SRT/WebVTT timestamp parser, `normalize_line_endings`, `to_hms_ms`, its own `TICKS_PER_MS`) and the local `unescape_payload` are deleted in favour of `timed_metadata::webvtt::{parse_timestamp, format_timestamp_with, normalize_line_endings, payload_lines, unescape_payload}`; `Error::InvalidTimestamp` still carries the offending text (#1210).
- Internal: odd-parity encode/check comes from `broadcast_common::parity` instead of a local copy (`timed_metadata::webvtt::teletext::{encode,decode}_odd_parity` stay as thin wrappers, public API unchanged); no behaviour change (#1212).

#### Fixed
- `write_srt` now folds a lone CR (and CRLF) inside a cue's text to LF exactly like the WebVTT writer does (it used `str::lines()`, which only knows LF/CRLF, so a lone CR was written as a literal control character inside a payload line); `write_srt_folds_a_lone_cr_like_the_webvtt_writer` fails on the old code (#1210).

#### Fixed
- Timestamps are digits-only: a `+` sign in any field (accepted by
  `str::parse`) is now rejected (#1109 CV-O1).
- `parse_webvtt` / `parse_srt` reject a cue whose end time precedes its start
  time (#1109 CV-O2).
- Timestamp parsing splits on the millisecond separator in place instead of
  allocating a normalised `String` per timestamp (#1109 CV-O3).

#### Changed
- The conversion matrix no longer claims CEA-608/708/Teletext cue timing
  "carries over exactly": cue times are the carrying frame/packet PTS (#1109
  CV-W2).

## Merged changes since `caption-convert-v0.2.0`

- captions: mux.js/shaka media (unclear licence) moved to the private submodule; tests read private/fixtures/captions (#1261) (#1262)
- Real CEA-608/708 captures: fixtures, tests, and the bugs they exposed (#1220) (#1221)
- caption-convert: converter_tail! macro for shared converter methods (#1218) (#1219)
- broadcast_common::parity: shared odd-parity helpers; migrate cc-data, timed-metadata, caption-convert (#1212) (#1213)
- test: shared test-fixtures loader crate, migrate hand-written loaders, drop CRC copies (#1214) (#1215)
- Audit sweep of the nine small crates: fit_mask serializers, hostility + guard tests (#1202) [stacked on #1199] (#1203)
- timed-metadata::webvtt shared text helpers (timestamp parse/format, unescape, line endings); caption-convert drops its duplicates; write_srt folds lone CR (#1210) (#1211)
- Audit per-crate rollups: verified against main, 54 commits of real fixes across ~40 crates (BREAKING items marked, NO release) (#1179)
- Audit remediation: unsafe quarantine, panic removal, workspace lints/deps (#1156)
