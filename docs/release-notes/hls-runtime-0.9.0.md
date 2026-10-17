---
status: DRAFT
---
# hls-runtime 0.9.0

_Released 2026-10-16._

**Breaking release.** The HLS origin/client follows the typed ids: `DEFAULT_TRACK_ID` is a `transmux::TrackId`, `server::{SegmentSeq, PartIndex}` re-export `media_plane::ids`, and a new `server::Generation` newtype replaces the bare init-generation integer. Wire names (`seg-1-5-7.m4s`, `init-1-5-2.mp4`, `part-..`) are unchanged and pinned by `resource_names_place_every_distinct_id_in_its_own_slot`.

## Why this version

- **Proposed version:** 0.8.0 -> **0.9.0** (major-class: 0.x minor-epoch bump).
- **Own API/behaviour change class:** major (TrackId / SegmentSeq / PartIndex / Generation newtypes).
- **cargo-semver-checks vs live 0.8.0 (all features):** none flagged. (Lint-based: it misses type changes of existing fields/arguments, so the CHANGELOG BREAKING list is the primary evidence.)
- **Epoch cascade:** none needed (own breaking change). Siblings it depends on that move epoch in this wave: broadcast-auth, broadcast-common, broadcast-hls, media-plane, timed-metadata, transmux.
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
// before
origin.init_name(track_id_u32, 2);      ClosedSegment::new(5u32, ..)
// after
origin.init_name(TrackId::new(track_id_u32), Generation::new(2));      ClosedSegment::new(SegmentSeq::new(5), ..)
```

`HlsOrigin::render_playlist` and the client `Output::Samples { track_id }` take/return `TrackId`; `ClosedSegment::init_generation`/`with_init_generation` are `Generation`.

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
