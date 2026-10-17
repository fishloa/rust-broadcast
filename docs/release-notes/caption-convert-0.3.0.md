# caption-convert 0.3.0

_Released 2026-10-16._

**Breaking release (its own break, plus the `broadcast-common` 10 / `cc-data` 0.7 / `dvb-vbi` 0.5 / `timed-metadata` 0.7 epoch).** `matrix::lookup` now returns an `Option` instead of panicking when the matrix has no row for a `(from, to)` pair. The release also fixes real conversion defects: CEA-608 to WebVTT/SRT no longer drops captions that re-issue RCL before EDM/EOC, same-PTS caption updates no longer yield zero-length cues, timestamp parsing is digits-only and rejects a cue whose end precedes its start, `parse_srt` keeps accepting the `.` millisecond separator, and `write_srt` folds a lone CR like the WebVTT writer does.

## Why this version

- **Proposed version: 0.2.0 -> 0.3.0** (major-class: 0.x minor-epoch bump).
- **Own API/behaviour change class:** major. `matrix::lookup` changes its return type from `&MatrixEntry` to `Option<&'static MatrixEntry>`.
- **cargo-semver-checks vs live 0.2.0 (all features):** none flagged. It does not see a changed return type, so the CHANGELOG BREAKING entry is the primary evidence.
- **Epoch cascade:** also from `broadcast-common`, `cc-data`, `dvb-vbi` and `timed-metadata`, all of which move epoch in this wave.
- **Caret epoch:** 0.2 -> 0.3; every workspace dependent must move with it.

## Dependency changes

```toml
-broadcast-common = { path = "../broadcast-common", version = "9.4",  default-features = false }
+broadcast-common = { path = "../broadcast-common", version = "10.0", default-features = false }
-timed-metadata   = { path = "../timed-metadata",   version = "0.6",  default-features = false }
+timed-metadata   = { path = "../timed-metadata",   version = "0.7",  default-features = false }
-cc-data          = { path = "../cc-data",          version = "0.6",  default-features = false, optional = true }
+cc-data          = { path = "../cc-data",          version = "0.7",  default-features = false, optional = true }
-dvb-vbi          = { path = "../dvb-vbi",          version = "0.4",  default-features = false, optional = true }
+dvb-vbi          = { path = "../dvb-vbi",          version = "0.5",  default-features = false, optional = true }
```

Dev-dependencies also gained the path-only `test-fixtures`, `mpeg-ts` and `mpeg-pes` (not published; used by the real-capture tests).

## Breaking changes and migration

`matrix::lookup` used to panic (`.expect`) when the matrix had no row for a pair. Both format enums are `#[non_exhaustive]`, so a newly added format could reach that panic. It now returns `None`. `check` is unchanged in signature and maps the absent row to `Error::Unsupported { support: NotImplemented, .. }`.

```rust
use caption_convert::matrix::{self, MatrixEntry};
use caption_convert::{SourceFormat, TargetFormat};

// before
// let entry: &MatrixEntry = matrix::lookup(from, to);   // panicked on a missing row
// after
let entry: Option<&'static MatrixEntry> = matrix::lookup(from, to);
// or keep the error-returning form:
let entry: &'static MatrixEntry = caption_convert::check(from, to)?;
```

Behaviour changes that need no code change but can alter output: `parse_webvtt` / `parse_srt` now reject a `+` sign in any timestamp field and a cue whose end time precedes its start time; `write_srt` writes a lone CR (and CRLF) inside cue text as LF.

## What changed (from CHANGELOG `[Unreleased]`)

### Documentation
- README and crate-root docs restructured: explicit what-it-does / does-not (DVB bitmap not converted to text), standards table, full conversion matrix including WebVTT <-> SRT, feature table, compiled quick-start doctest.

### Changed (BREAKING: epoch — requires broadcast-common 10)
- Builds against new caret epochs of workspace siblings (`broadcast-common` 9 -> 10, `cc-data` 0.6 -> 0.7, `dvb-vbi` 0.4 -> 0.5, `timed-metadata` 0.6 -> 0.7). A caret-epoch change is major-class for this crate under the epoch-purity rule (`docs/RELEASE-AUDIT.md` §2), whatever its own API diff. `Serialize::to_bytes` was removed (use `try_to_bytes`); consumers must resolve a single `broadcast-common` 10.

### Changed
- Internal: the five finishing methods shared by `Cea608ToWebVtt`, `Cea708ToWebVtt` and `TeletextToWebVtt` come from one `converter_tail!` macro instead of three copies; public API unchanged (#1218).

### Fixed
- 608 -> WebVTT/SRT: captions loaded with a repeated RCL before EDM/EOC are converted instead of dropped, and same-PTS caption updates no longer yield zero-length cues (fixes in `cc-data` / `timed-metadata`, covered here by real-capture tests) (#1220).

### Changed (BREAKING)
- `matrix::lookup` returns `Option<&'static MatrixEntry>` instead of panicking (`.expect`) when the matrix has no row for a `(from, to)` pair; both format enums are `#[non_exhaustive]`, so a newly added format could reach the old panic. `check` maps the absent row to `Error::Unsupported { support: NotImplemented, .. }` (#1202).

### Changed
- Internal (behaviour preserved): `time.rs` (the SRT/WebVTT timestamp parser, `normalize_line_endings`, `to_hms_ms`, its own `TICKS_PER_MS`) and the local `unescape_payload` are deleted in favour of `timed_metadata::webvtt::{parse_timestamp, format_timestamp_with, normalize_line_endings, payload_lines, unescape_payload}`; `Error::InvalidTimestamp` still carries the offending text (#1210).
- Internal: odd-parity encode/check comes from `broadcast_common::parity` instead of a local copy (`timed_metadata::webvtt::teletext::{encode,decode}_odd_parity` stay as thin wrappers, public API unchanged); no behaviour change (#1212).

### Fixed
- `parse_srt` accepts the `.` millisecond separator (`00:00:01.000 --> ...`) again alongside `,`: the move to the shared `timed_metadata` timestamp grammar (#1210) had silently started rejecting it, which 0.2.0 accepted. `write_srt` still writes `,`; `srt_with_dot_millisecond_separator_still_parses` fails on the regressed code.
- `write_srt` now folds a lone CR (and CRLF) inside a cue's text to LF exactly like the WebVTT writer does (it used `str::lines()`, which only knows LF/CRLF, so a lone CR was written as a literal control character inside a payload line); `write_srt_folds_a_lone_cr_like_the_webvtt_writer` fails on the old code (#1210).

### Fixed
- Timestamps are digits-only: a `+` sign in any field (accepted by
  `str::parse`) is now rejected (#1109 CV-O1).
- `parse_webvtt` / `parse_srt` reject a cue whose end time precedes its start
  time (#1109 CV-O2).
- Timestamp parsing splits on the millisecond separator in place instead of
  allocating a normalised `String` per timestamp (#1109 CV-O3).

### Changed
- The conversion matrix no longer claims CEA-608/708/Teletext cue timing
  "carries over exactly": cue times are the carrying frame/packet PTS (#1109
  CV-W2).
