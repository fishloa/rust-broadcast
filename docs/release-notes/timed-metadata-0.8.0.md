# timed-metadata 0.8.0

_Released 2026-10-31._

### Changed (BREAKING: epoch)
- Builds against new caret epochs of workspace siblings (`broadcast-hls` 0.4 -> 0.5). A caret-epoch change is major-class for a 0.x crate under the epoch-purity rule (`docs/RELEASE-AUDIT.md` section 2), whatever its own API diff, so this release is a minor bump; consumers must resolve a single copy of each of those crates.
### Added
- `TeletextCueExtractor::take_closed_cues` removes and returns every cue whose end is known, and `open_cue` reports the cue on screen as the new `#[non_exhaustive]` `OpenCue`. Taken cues no longer appear in `cues()` or `into_cues()`.
- `NationalOption` gains `Polish`, `Rumanian`, `Turkish`, `SerbianCroatianSlovenian`, `Estonian` and `LettishLithuanian`, so every row of EN 300 706 Table 36 can be addressed. A page header's C12-C14 bits still select only the seven Level 1 options.
### Fixed
- Teletext decoding applied only the English national option, so French pages read "d#passer", "fen\tre" and "apr`s". Every Table 36 national option is now applied, selected by the page header's C12-C14 bits (#1585).
- Teletext Start Box and End Box (clause 12.2) were rendered as spaces, leaving up to eight leading spaces on every cue. On a newsflash or subtitle page (C5 or C6) only the text between a Start Box and the following End Box is now displayed, and spaces at the box edges are trimmed (#1585). A subtitle row with no Start Box is no longer displayed, as the spec requires.
- `webvtt::write_segment` now writes `X-TIMESTAMP-MAP` as `LOCAL:...,MPEGTS:...`, the field order RFC 8216 section 3.5 gives (#1590).
### Changed
- `DateRange::parse_tag_line` now propagates malformed attribute-list errors from `broadcast_hls::parse_attribute_list_strict` (unterminated quoted-string, duplicate attribute name) as `Error::HlsAttrValue` instead of silently half-parsing them (#1485 R15).

---

Published from tag `timed-metadata-v0.8.0`.
