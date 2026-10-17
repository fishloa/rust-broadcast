# timed-metadata 0.7.0

_Released 2026-10-16._

**Epoch-forced major-class release with additive API and one behavioural fix; no API break of its own.** The WebVTT/SRT text helpers (`parse_timestamp`, `format_timestamp_with`, `hms_ms`, `unescape_payload`, `normalize_line_endings`, `payload_lines`, `TimestampError`) now live in one place, a new `webvtt::text` module (previously the hour/minute/second math was duplicated in `caption-convert`), and the CEA-608/708 and Teletext cue extractors no longer emit zero-length or negative cues. The crate rebuilds against `broadcast-common` 10, `broadcast-hls` 0.4, `cc-data` 0.7, `dvb-vbi` 0.5, `mp4-emsg` 0.5 and `scte35-splice` 4, which is what makes the version `0.7.0`. The README and crate-root docs are restructured (scope, standards table, SCTE-35 / DATERANGE / emsg conversion matrix, feature table, compiled quick start).

## Why this version

- **Proposed version:** 0.6.1 -> **0.7.0** (major-class: 0.x minor-epoch bump).
- **Own API/behaviour change class:** minor (webvtt text helpers (additive)).
- **cargo-semver-checks vs live 0.6.1 (all features):** none flagged. (Lint-based: it misses type changes of existing fields/arguments, so the CHANGELOG BREAKING list is the primary evidence.)
- **Epoch cascade:** epoch cascade from broadcast-common. Siblings it depends on that move epoch in this wave: broadcast-common, broadcast-hls, cc-data, dvb-vbi, mp4-emsg, scte35-splice.
- **Caret epoch:** 0.6 -> 0.7; every workspace dependent must move with it.

## Dependency changes

```toml
-broadcast-common = "9.4"
+broadcast-common = "10.0"
-broadcast-hls = "0.3"
+broadcast-hls = "0.4"
-cc-data = "0.6"
+cc-data = "0.7"
-dvb-vbi = "0.4"
+dvb-vbi = "0.5"
-mp4-emsg = "0.4"
+mp4-emsg = "0.5"
-scte35-splice = "3.0"
+scte35-splice = "4.0"
```

## Breaking changes and migration

No API break of its own. What to do:

- Resolve a single copy of each moved sibling in your dependency graph: `broadcast-common` 10, `broadcast-hls` 0.4, `mp4-emsg` 0.5, `scte35-splice` 4 and, with the optional `cc-data` / `teletext` features, `cc-data` 0.7 and `dvb-vbi` 0.5. Types from these crates (SCTE-35 commands, `emsg` boxes, cc_data triplets, Teletext data fields) are consumed by this crate's API, so a mixed graph fails to type-check.
- `Serialize::to_bytes` was removed in `broadcast-common` 10 (it could panic); use `try_to_bytes()?`.
- Behavioural: caption cue streams can contain fewer cues than before. Several caption updates under one access-unit PTS (21 of them on CC1 of the mux.js multi-channel TS) used to produce intermediate cues whose end equalled their start; those are no longer emitted, so counts or goldens that included them change.
- If you carry your own copy of WebVTT/SRT timestamp or line-ending handling, switch to the shared helpers:

```rust
use timed_metadata::webvtt::{parse_timestamp, normalize_line_endings, payload_lines};
```

(`format_timestamp` and `cue_block` are now built on the same helpers; `teletext::{encode,decode}_odd_parity` keep their signatures and delegate to `broadcast_common::parity`.)

## What changed (from CHANGELOG `[Unreleased]`)

#### Documentation
- README and crate-root docs restructured: scope, standards table, SCTE-35 / DATERANGE / emsg conversion matrix, feature table, and a compiled quick-start doctest.

#### Changed (BREAKING: epoch — requires broadcast-common 10)
- Builds against new caret epochs of workspace siblings (`broadcast-common` 9 -> 10, `broadcast-hls` 0.3 -> 0.4, `cc-data` 0.6 -> 0.7, `dvb-vbi` 0.4 -> 0.5, `mp4-emsg` 0.4 -> 0.5, `scte35-splice` 3 -> 4). A caret-epoch change is major-class for this crate under the epoch-purity rule (`docs/RELEASE-AUDIT.md` §2), whatever its own API diff. `Serialize::to_bytes` was removed (use `try_to_bytes`); consumers must resolve a single `broadcast-common` 10.

#### Fixed
- The 608/708/Teletext cue extractors no longer emit zero-length (or negative) cues. Several caption updates under one access-unit PTS (common in real captures: 21 on CC1 of the mux.js multi-channel TS) produced intermediate cues whose end equalled their start, which are never on screen (#1220).

#### Added
- `webvtt` shared text helpers (new `webvtt::text` module, re-exported): `parse_timestamp` / `format_timestamp_with` / `hms_ms` (the `(hh:)?mm:ss<sep>ttt` grammar both directions, `.` WebVTT and `,` SRT, with the #1109 checked-overflow hardening), `unescape_payload` (inverse of `escape_payload`), `normalize_line_endings` (CRLF / lone CR / LF -> LF, borrowed when there is no CR) and `payload_lines`, plus `TimestampError`. They were split between this crate and `caption-convert` (which kept a second copy of the hour/minute/second math "in lock-step" by a comment); this is now the single owner and `format_timestamp` / `cue_block` are built on it (#1210).

#### Changed
- Internal: odd-parity encode/check comes from `broadcast_common::parity` instead of a local copy (`timed_metadata::webvtt::teletext::{encode,decode}_odd_parity` stay as thin wrappers, public API unchanged); no behaviour change (#1212).

## Merged changes since `timed-metadata-v0.6.1`

- docs: correct false 'not vendored' / 'edition unverified' claims (#1311)
- docs: cluster C crate READMEs + crate-root docs (standards tables, feature flags, scope, verified editions) (#1284)
- Fix the confirmed signalling/captions review findings (supersedes #1279) (#1282)
- tests: one fixture resolver (public -> private/fixtures -> .test-streams); move local-only captures to the private repo (#1259) (#1260)
- Real CEA-608/708 captures: fixtures, tests, and the bugs they exposed (#1220) (#1221)
- broadcast_common::parity: shared odd-parity helpers; migrate cc-data, timed-metadata, caption-convert (#1212) (#1213)
- test: shared test-fixtures loader crate, migrate hand-written loaders, drop CRC copies (#1214) (#1215)
- clock33: shared PCR constants + modular distance; migrate 5 crates; fix ts-fix post-wrap observations (#1206) [stacked on #1196] (#1207)
- Audit sweep of the nine small crates: fit_mask serializers, hostility + guard tests (#1202) [stacked on #1199] (#1203)
- timed-metadata::webvtt shared text helpers (timestamp parse/format, unescape, line endings); caption-convert drops its duplicates; write_srt folds lone CR (#1210) (#1211)
- Cleanups: remove panicking Serialize::to_bytes (BREAKING, #1143) + serde default-features off in 32 no_std manifests with CI thumbv7em serde matrix (#1183) (#1185)
- Audit per-crate rollups: verified against main, 54 commits of real fixes across ~40 crates (BREAKING items marked, NO release) (#1179)
- Typed ids wave 1: Pid newtype (mpeg-ts and all non-dvb-si APIs) + reserved-bit preservation for all dvb-si descriptors and tables (BREAKING, NO release) (#1177)
