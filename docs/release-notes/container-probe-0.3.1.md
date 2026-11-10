---
status: DRAFT
---
# container-probe 0.3.1

_Released 2026-11-10._

Patch release: recognise the compressed top-level boxes of ISO/IEC 14496-12:2026 §8.16.

Version: 0.3.0 -> 0.3.1.

## Added
- The ISOBMFF prober and `segment::{media_fragment, progressive_movie}` recognise the compressed top-level boxes of ISO/IEC 14496-12:2026 §8.16: `!mov` counts as `moov` (progressive layout), `!mof` as `moof` (fragmented layout / media fragment), and either may be the first box. Nothing is inflated; an `otyp` after `ftyp`/`styp` is walked as an ordinary box.

---

Published from tag `container-probe-v0.3.1`.
