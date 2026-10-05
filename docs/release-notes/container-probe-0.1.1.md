# container-probe 0.1.1

_Released 2026-10-05._

### Fixed

- ADTS chain walker: a chain ending exactly at a frame boundary, or with only
  a partial next header visible at the buffer end, now reports `Insufficient`
  rather than `None`, mirroring the MP3 prober's identical branch (#1076).

---

Published from tag `container-probe-v0.1.1`.
