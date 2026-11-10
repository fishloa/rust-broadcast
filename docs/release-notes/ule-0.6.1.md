---
status: DRAFT
---
# ule 0.6.1

_Released 2026-11-10._

Patch release: reject the End Indicator.

Version: 0.6.0 -> 0.6.1.

## Fixed
- `Sndu::parse` rejects the `0xFFFF` End Indicator (`D=1`, `Length=0x7FFF`) with `Error::InvalidLength`, matching what `serialize_into` already refused.

---

Published from tag `ule-v0.6.1`.
