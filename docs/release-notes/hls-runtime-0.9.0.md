# hls-runtime 0.9.0

_Released 2026-10-16._

**Breaking release.** The HLS origin and client follow the typed ids introduced across the wave: `DEFAULT_TRACK_ID` and the client's `Output::Samples { track_id }` are `transmux::TrackId`, `server::{SegmentSeq, PartIndex}` are now re-exports of `media_plane::ids`, and a new `server::Generation` newtype replaces the bare init-generation integer. Wire names (`seg-1-5-7.m4s`, `init-1-5-2.mp4`, `part-..`) are unchanged. The crate also builds against `broadcast-common` 10, `broadcast-auth` 0.5, `broadcast-hls` 0.4, `media-plane` 0.7, `timed-metadata` 0.7 and `transmux` 0.27.

## Why this version

- **Proposed version:** 0.8.0 -> **0.9.0** (major-class: 0.x minor-epoch bump).
- **Own API/behaviour change class:** major (`TrackId` / `SegmentSeq` / `PartIndex` / `Generation` newtypes in the public origin and client API).
- **cargo-semver-checks vs the live release (all features):** none flagged. (Lint-based: it misses type changes of existing fields and arguments, so the CHANGELOG BREAKING list is the primary evidence.)
- **Epoch cascade:** also forced by the sibling epochs it depends on: `broadcast-common` 9 -> 10, `broadcast-auth` 0.4 -> 0.5, `broadcast-hls` 0.3 -> 0.4, `media-plane` 0.6 -> 0.7, `timed-metadata` 0.6 -> 0.7, `transmux` 0.26 -> 0.27.
- **Caret epoch:** 0.8 -> 0.9; every workspace dependent must move with it.

## Dependency changes

```toml
-broadcast-auth = "0.4"
+broadcast-auth = "0.5"
-broadcast-common = "9.4"
+broadcast-common = "10.0"
-broadcast-hls = "0.3"
+broadcast-hls = "0.4"
-media-plane = "0.6"
+media-plane = "0.7"
-timed-metadata = "0.6"
+timed-metadata = "0.7"
-transmux = "0.26"
+transmux = "0.27"
```

## Breaking changes and migration

```rust
use hls_runtime::server::{ClosedSegment, Generation, SegmentSeq};
use transmux::TrackId;

// before
let name = origin.init_name(1u32, 2);
let seg = ClosedSegment::new(5u32, start_ns, duration_secs, false);
// after
let name = origin.init_name(TrackId::new(1), Generation::new(2));
let seg = ClosedSegment::new(SegmentSeq::new(5), start_ns, duration_secs, false);
```

- `DEFAULT_TRACK_ID` is `TrackId::new(1)`, and the client's `Output::Samples { track_id }` carries a `TrackId`; convert with `TrackId::new(raw)` where you previously used a `u32`.
- `ClosedSegment::sequence_number` is a `SegmentSeq`; `ClosedSegment::with_init_generation` takes a `Generation`, and `ClosedSegment::new` defaults the init generation to `Generation::new(1)`.
- `server::SegmentSeq` and `server::PartIndex` are the same types as `media_plane::ids::{SegmentSeq, PartIndex}`, so values can be passed between `media-plane` and `hls-runtime` without conversion.
- Wire names are unchanged and pinned by `resource_names_place_every_distinct_id_in_its_own_slot`, so existing players and CDN caches keep working.
- The internal `segment_name`/`part_name`/`resident_part_specs` helpers take the typed ids, so a swapped argument is a compile error.

Dependencies move as listed above; resolve them in one graph (`transmux` 0.27, `media-plane` 0.7 and `broadcast-common` 10 together). `Serialize::to_bytes` was removed from `broadcast-common` 10; use `try_to_bytes`.

## What changed (from CHANGELOG `[Unreleased]`)

#### Documentation
- README restructured: what the crate does and does not do, a standards-supported table (edition, scope, status), feature-flag table, a doctested quick start and related crates; crate-root docs now render the README (or are kept in step with it). The former published name `ll-hls-runtime` is noted.

#### Changed (BREAKING: epoch — requires broadcast-common 10)
- Builds against new caret epochs of workspace siblings (`broadcast-common` 9 -> 10, `broadcast-auth` 0.4 -> 0.5, `broadcast-hls` 0.3 -> 0.4, `media-plane` 0.6 -> 0.7, `timed-metadata` 0.6 -> 0.7, `transmux` 0.26 -> 0.27). A caret-epoch change is major-class for this crate under the epoch-purity rule (`docs/RELEASE-AUDIT.md` §2), whatever its own API diff. `Serialize::to_bytes` was removed (use `try_to_bytes`); consumers must resolve a single `broadcast-common` 10.

#### Changed

- **BREAKING (#1188):** `server::SegmentSeq`/`PartIndex` are now re-exports of `media_plane::ids::{SegmentSeq, PartIndex}` (one canonical definition); `ClosedSegment::sequence_number` and `ClosedSegment::new`'s first argument are `SegmentSeq`.

- **BREAKING (#1159):** `DEFAULT_TRACK_ID` is a `transmux::TrackId`; `HlsOrigin::render_playlist` and the client `Output::Samples { track_id }` take/return `TrackId`. New `server::{Generation, SegmentSeq, PartIndex}` newtypes: `HlsOrigin::init_name(track_id, generation)` takes a `Generation` and `ClosedSegment::init_generation`/`with_init_generation` are `Generation`; the private `segment_name`/`part_name`/`resident_part_specs` take `SegmentSeq`/`PartIndex`, so a swapped argument is a compile error. Wire names (`seg-1-5-7.m4s`, `init-1-5-2.mp4`, `part-..`) are unchanged and pinned by `resource_names_place_every_distinct_id_in_its_own_slot`.

## Merged changes since `hls-runtime-v0.8.0`

- docs: clear per-crate documentation for cluster B (16 crates) (#1291)
- Fix the confirmed signalling/captions review findings (supersedes #1279) (#1282)
- chore: pre-release hygiene (README coverage, snippet tool, docsrs, examples, CLAUDE.md) (#1278)
- media-plane: SegmentSeq/PartIndex newtypes across media-plane, hls-runtime, multimux (BREAKING) (#1194)
- Cleanups: remove panicking Serialize::to_bytes (BREAKING, #1143) + serde default-features off in 32 no_std manifests with CI thumbv7em serde matrix (#1183) (#1185)
- Systemic umbrellas T1/T2/T14: serializer width guards (28 crates), hostile-count DoS fix, secrets hygiene, origin-scoped credentials, secure ingest default (BREAKING items marked, NO release) (#1182)
- Typed ids wave 2: TrackId/ProgramNumber + HLS naming newtypes, Pid inside dvb-si (BREAKING, NO release) (#1178)
