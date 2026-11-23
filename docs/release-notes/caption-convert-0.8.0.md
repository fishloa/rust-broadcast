# caption-convert 0.8.0

_Released 2026-11-22._

Minor for a 0.x crate. No functional change of its own: the crate builds against new caret epochs of `broadcast-common` 14, `cc-data` 0.11, `dvb-vbi` 0.9 and `timed-metadata` 0.12, which is the breaking part. The work is a behavioural test pass over the conversion matrix.

## Breaking changes

- Builds against new caret epochs of workspace siblings (`broadcast-common` 13 -> 14, `cc-data` 0.10 -> 0.11, `dvb-vbi` 0.8 -> 0.9, `timed-metadata` 0.11 -> 0.12). A caret-epoch change is major-class under the epoch-purity rule (`docs/RELEASE-AUDIT.md` §2) whatever this crate's own API diff; consumers must resolve a single copy of each at the jiff 1.0 epoch, and the `cc_data()`/timed-metadata values this crate takes are those versions' types.

## Tests

- A WebVTT timing line with only trailing whitespace is not cue settings (the document stays lossless), and the malformed-timing error names the offending line literally (#3036).
- An Inverted Teletext (`0xC0`) data unit carrying a valid subtitle page yields no cues, while the same bytes as EBU Teletext subtitle (`0x03`) do (#3035).
- The CEA-708 path has a behavioural test: a real DTVCC service-1 window paints text and is cleared, producing a timed WebVTT cue, and invalid `cc_data()` bytes propagate `Error::CcData` (#3034).

## Pre-release audit

Issues fixed in this crate by the 2026-11-22 pre-release audit: #3992 (epoch line in the CHANGELOG).

---

Published from tag `caption-convert-v0.8.0`.
