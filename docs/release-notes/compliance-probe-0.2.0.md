# compliance-probe 0.2.0

_Released 2026-10-05._

**Minor-epoch release (0.x); three correctness fixes, no API change.** The probe's SCTE-35 and PCR-drift checks judged some real streams wrongly. Metric values and verdicts will change on streams that use `pts_adjustment`, signal PCR discontinuities, or run on a long-lived `Trunk`. The crate is unpublished (workspace-internal). Dependencies move with the wave:

```toml
-broadcast-common = { version = "9.3", default-features = false }
+broadcast-common = { version = "9.4", default-features = false }
-dvb-conformance  = { version = "10",   default-features = false }
+dvb-conformance  = { version = "11.0", default-features = false }
-mpeg-ts          = { version = "0.4",  default-features = false }
+mpeg-ts          = { version = "0.5",  default-features = false }
-scte35-splice    = { version = "2.1",  default-features = false }
+scte35-splice    = { version = "3.0",  default-features = false }
-timed-metadata   = { version = "0.5",  default-features = false }
+timed-metadata   = { version = "0.6",  default-features = false }
-media-doctor     = { version = "0.8",  default-features = false }
+media-doctor     = { version = "0.9",  default-features = false }
-media-plane      = { version = "0.4",  default-features = false }
+media-plane      = { version = "0.5",  default-features = false }
```

See [dvb-conformance 11.0.0](dvb-conformance-11.0.0.md) (the TR 101 290 indicator changes it drives), [scte35-splice 3.0.0](scte35-splice-3.0.0.md) and [timed-metadata 0.6.0](timed-metadata-0.6.0.md).

## Fixes

- `scte35::check_section` now applies `pts_adjustment` (ANSI/SCTE 35 §9.6.1) to `pts_time` before judging future versus past. A cue passed through equipment that rebases splice times via `pts_adjustment` was previously judged on the raw `pts_time` (#1097, W-CP-1).
- The PCR drift tracker resets a PID's baseline when the adaptation field's `discontinuity_indicator` is set, instead of measuring drift across a broadcaster-signalled discontinuity such as an ad-insertion splice. That used to report tens of millions of ppm of spurious drift for several seconds after every such splice (#1097, W-CP-2).
- The Trunk-cursor SCTE-35 path (`trunk_bridge::check_event`) compares its already wrap-unrolled `target` and `now` 90 kHz values directly instead of reusing `scte35::judge`, which reduces modulo 2^33. On a long-running Trunk, a `target` more than about 13.3 hours ahead of `now` was folded back onto the 33-bit ring and misjudged `InPast` (#1097, W-CP-3).

---

Published from tag `compliance-probe-v0.2.0`.
