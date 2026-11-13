# container-probe 0.4.0

_Released 2026-11-14._

Major-class release (0.3 -> 0.4) for epoch purity: additions and fixes listed below; its sibling dependencies move to new caret epochs, which the workspace treats as a major-class change (docs/RELEASE-AUDIT.md section 2).

Version: 0.3.0 -> 0.4.0.

## Added
- The ISOBMFF prober and `segment::{media_fragment, progressive_movie}` recognise the compressed top-level boxes of ISO/IEC 14496-12:2026 §8.16: `!mov` counts as `moov` (progressive layout), `!mof` as `moof` (fragmented layout / media fragment), and either may be the first box. Nothing is inflated; an `otyp` after `ftyp`/`styp` is walked as an ordinary box.

## Dependencies
- Sibling dependencies move to new caret epochs: `broadcast-common` 11 -> 12.

---

Published from tag `container-probe-v0.4.0`.
