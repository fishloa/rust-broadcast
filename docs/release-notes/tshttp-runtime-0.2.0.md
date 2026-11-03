# tshttp-runtime 0.2.0

_Released 2026-11-03._

Patch release: version bump only, following `transmux`.

### Changed
- The `live_loop` example is now `tshttp_loop`: its name collided with `buffering-runtime`'s `live_loop`, and cargo warned about the clashing output file in a workspace build.
- Builds against `transmux` 0.29 (was 0.28); no functional change in this crate.

---

Published from tag `tshttp-runtime-v0.2.0`.
