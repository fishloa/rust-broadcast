
# dvb-conformance 13.0.0

_Released 2026-11-07._

Lockstep major (12.1.0 -> 13.0.0). The breaking changes and their migration snippets are in the lockstep note [v13.0.0](v13.0.0.md); this note lists what changed in `dvb-conformance`.

## Changed
- BREAKING: `serde` is no longer a default feature; enable `features = ["serde"]` (owner decision 2026-11-07).
- `Monitor::feed` delegates sync hysteresis, scrambling checks and PAT, PMT and SI section reassembly to private methods (internal, c3 t:S7).
- TS packet size and sync byte come from `mpeg_ts::ts::{TS_PACKET_SIZE, TS_SYNC_BYTE}` instead of a local copy (c6 C6-07).
## Dependencies
- Sibling crates move to a new epoch in this release wave: `broadcast-common` 10 -> 11, `dvb-si` 12 -> 13, `mpeg-ts` 0.6 -> 0.7. The `Cargo.toml` requirements name the new versions; no other manifest change.

---

Published from tag `v13.0.0`.
