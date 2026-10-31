# container-probe 0.2.1

_Released 2026-10-31._

### Documentation
- Fixed documentation links/citations to resolve to existing paths (docs/tools hygiene).
### Added
- `segment::{ts_structure, media_fragment, progressive_movie}`: structural checks that a fetched HLS/DASH segment body is media (TS at any packet phase, ISO/IEC 14496-12 §8.8.4 `moof`+`mdat`, a `moov` without `moof`), moved out of `hls-runtime` so the HLS and DASH clients share one definition. Additive.

---

Published from tag `container-probe-v0.2.1`.
