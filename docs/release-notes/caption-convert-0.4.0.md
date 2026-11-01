# caption-convert 0.4.0

_Released 2026-11-01._

### Changed (BREAKING: epoch)
- Builds against new caret epochs of workspace siblings (`timed-metadata` 0.7 -> 0.8). A caret-epoch change is major-class for a 0.x crate under the epoch-purity rule (`docs/RELEASE-AUDIT.md` section 2), whatever its own API diff, so this release is a minor bump; consumers must resolve a single copy of each of those crates.
### Added
- `TeletextToWebVtt::push_pes_data_field` parses a teletext PES data field with `dvb_vbi::DataField` and feeds its EBU teletext data units, plus `take_closed_cues` and `open_cue` for incremental cue output. Cues taken with `take_closed_cues` no longer appear in `cues()` or the `into_*` renderers.

### Fixed
- Teletext to WebVTT/SRT conversion now uses the page's national option (French, German, Italian and the others) and only the boxed subtitle text, so cues no longer carry "#" or "\\" for accented letters or leading spaces. Covered by a real-capture comparison against TSDuck's SRT text for `test-012.ts` (#1585).

---

Published from tag `caption-convert-v0.4.0`.
