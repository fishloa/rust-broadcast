# transmux 0.25.5

_Released 2026-10-13._

### Fixed

- `StreamingTsDemux` no longer raises a `TrackUpdated` per step of a rotating PPS set (H.264 and HEVC) (#1166). PPS change detection compared each arriving NAL against a single "last seen" slot, so an encoder cycling several PPS ids (orf1: three, one per access unit) flipped it on every access unit and re-emitted an identical config 15-17 times per 10 s. A PPS now counts as a change only when its bytes were never seen before on the track (bounded to 64). SPS/VPS are unchanged: any difference from the current set is a change, so an SPS A -> B -> A switch (resolution change and back) still raises one update per switch, as does a same-length different-bytes SPS.

---

Published from tag `transmux-v0.25.5`.
