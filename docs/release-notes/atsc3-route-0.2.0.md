# atsc3-route 0.2.0

_Released 2026-10-05._

### Changed (breaking)
- **Removed `ExtRoutePresentationTime`** and **`ExtTol`** typed decoders removed from the
  public API. No publicly-available ATSC 3.0 ROUTE capture contains either
  extension (14,000+ real packets from three independent sources scanned, zero
  hits). This crate's fixture discipline requires every implemented type to be
  exercised by a byte-exact round-trip against a real capture. The HET constants
  (`HET_EXT_ROUTE_PRESENTATION_TIME`, `HET_EXT_TOL_24`, `HET_EXT_TOL_48`)
  remain for callers walking extension chains. The typed decoders will be
  re-added when a real capture containing these extensions surfaces.

---

Published from tag `atsc3-route-v0.2.0`.
