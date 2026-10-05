# container-probe 0.1.1

_Released 2026-10-05._

**Patch.** One fix to the ADTS prober's "read more" signal; no API change. A buffer whose ADTS frame chain ends exactly at a frame boundary, or shows only a partial next header at the buffer end, now yields `Insufficient` instead of `None`, mirroring the MP3 prober's identical branch (#1076). In practice, an ADTS stream read a few bytes at a time (a valid chain still short of the weak-confidence threshold) now tells the caller to read more, instead of this prober contributing no evidence at that point.

Dependency epochs move with the wave (no feature change; the three sibling crates are dev-dependencies only, used by the drift guard, so the runtime graph stays `broadcast-common` only):

```toml
-broadcast-common = { version = "9.3", default-features = false }
+broadcast-common = { version = "9.4", default-features = false }
-mpeg-ts = { version = "0.4" }
-mpeg-ps = { version = "0.4" }
-st377-1 = { version = "0.3" }
+mpeg-ts = { version = "0.5" }
+mpeg-ps = { version = "0.5" }
+st377-1 = { version = "0.4" }
```

See [container-probe 0.1.0](container-probe-0.1.0.md) for the crate's design.

---

Published from tag `container-probe-v0.1.1`.
