
# broadcast-client 0.3.0

_Released 2026-11-07._

Breaking-class release (0.2 -> 0.3) driven by ten sibling epoch moves, and the crate now contains the former `tshttp-runtime` crate as module `broadcast_client::tshttp`. The crate's own code changes are internal: shared timescale arithmetic and one shared in-flight request table for the DASH and HLS sources.

Version: 0.2.1 -> 0.3.0.

## Migration notes

### Merged crate: `tshttp-runtime` -> `broadcast_client::tshttp`

`tshttp-runtime` 0.2.0 is not released again; its sans-IO TS-over-HTTP session (`TsHttpClient`, `TsHttpPolicy`, `Config`, `Action`, `Event`, `StarveKind`, `DemuxEvent`) lives in `broadcast-client` and is `std` like the rest of the crate (the standalone `no_std` build is gone). The `serde` feature of `broadcast-client` now also derives `Serialize`/`Deserialize` on those types.

```toml
# before
tshttp-runtime = "0.2"
# after
broadcast-client = "0.3"
```
```rust
// before
use tshttp_runtime::{TsHttpClient, TsHttpPolicy, Action, Event};
// after
use broadcast_client::tshttp::{TsHttpClient, TsHttpPolicy, Action, Event};
```

### Behaviour change: zero timescale

`raw_27mhz` and the TS-over-HTTP `to_27mhz` rescale through `broadcast_common::timescale`, which treats a zero timescale as 1. `to_27mhz` with a zero clock used to return the ticks unscaled; `raw_27mhz` keeps its explicit zero-timescale-is-no-mapping rule.

## Added

- absorbs the former `tshttp-runtime` crate as module `broadcast_client::tshttp`, so `tshttp_runtime::X` becomes `broadcast_client::tshttp::X` (`TsHttpClient`, `TsHttpPolicy`, `Config`, `Action`, `Event`, `StarveKind`, `DemuxEvent`); the `serde` feature now also enables `Serialize`/`Deserialize` on those types. The module is `std` like the rest of the crate (the standalone `no_std` build is gone).

## Other changes

- The tick to microsecond conversions (`timeline::to_us`, RTSP `ticks_to_us`) use `broadcast_common::timescale::to_micros`.
- The DASH and HLS sources share the response head routing (redirect and auth replay), body accumulation with oversize reporting, refusal capture and the stalled-request resume decision in `source::fetch::inflight`.
- `raw_27mhz` and the TS-over-HTTP `to_27mhz` rescale through `broadcast_common::timescale`; a zero timescale now counts as 1 instead of yielding 0 / the unscaled value.
- `raw_27mhz` and the TS-over-HTTP `to_27mhz` rescale through `broadcast_common::timescale`; `to_27mhz` with a zero clock now counts it as 1 instead of returning the ticks unscaled, `raw_27mhz` keeps its explicit zero-timescale-is-no-mapping rule (pinned by `pts_to_27mhz_clamps_instead_of_truncating`).
- RTSP lane alignment rescales through `broadcast_common::timescale` (same rounding).
- Dropped the unused direct `serde` dependency (the `serde` feature still forwards to the sibling crates).

## Documentation

- README brought to the common section skeleton (what it does, standards, quick start, what is implemented, design notes, related crates, feature flags, MSRV and licence); the licence heading is spelled consistently.

Internal changes with no public effect (shared helpers replacing private copies, README as crate-root docs, `Clone`/`PartialEq`/`Eq` derives on error enums where every field allows it) are listed in the CHANGELOG.

## Dependencies

Workspace sibling requirements move to the new epochs of this wave:

- `broadcast-common` 10 -> 11
- `broadcast-hls` 0.5 -> 0.6
- `buffering-runtime` 0.1 -> 0.2
- `dash-runtime` 0.2 -> 0.3
- `dvb-si` 12 -> 13
- `hls-runtime` 0.11 -> 0.12
- `mpeg-ts` 0.6 -> 0.7
- `rtmp-runtime` 0.8 -> 0.9
- `rtp-packet` 0.5 -> 0.6
- `rtsp-runtime` 0.8 -> 0.9
- `srt-runtime` 0.7 -> 0.8
- `transmux` 0.29 -> 0.30

These are caret-epoch moves, so a project that names several of these crates must move them together.

---

Published from tag `broadcast-client-v0.3.0`.
