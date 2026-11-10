# scte35-splice 5.0.1

_Released 2026-11-10._

Patch release: strictness fixes for splice commands and descriptors.

Version: 5.0.0 -> 5.0.1.

## Fixed
- A typed splice command or descriptor whose declared length exceeds its structure (excess bytes in `splice_null`, `time_signal`, `avail_descriptor`, ...) is now rejected with `Error::InvalidField` instead of being accepted and silently shortened on serialize, which broke the parse-serialize byte round trip.
- `SpliceInfoSection::parse` rejects a set `section_syntax_indicator` or `private_indicator` (fixed at 0 by §9.6.1, Table 5) instead of re-serializing the section with those bits cleared.

---

Published from tag `scte35-splice-v5.0.1`.
