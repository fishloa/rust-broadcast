# media-plane 0.7.0

_Released 2026-10-17._

**Breaking release.** Segment and part numbers become `SegmentSeq` / `PartIndex` newtypes with checked arithmetic and track ids become `transmux::TrackId`, so a swapped segment/part argument is a compile error; at `u32::MAX`, `next_sequence_number` saturates and a further segment is rejected. One behaviour fix rides along: `IngestDriver::programs()` now iterates in ascending `ProgramId` order (it was backed by a `HashMap`), so per-program work is driven in a reproducible order across processes. The crate also builds against `broadcast-common` 10, `timed-metadata` 0.7, `transmux` 0.27 and (dev-dependency) `scte35-splice` 4.

## Why this version

- **Proposed version:** 0.6.0 -> **0.7.0** (major-class: 0.x minor-epoch bump).
- **Own API/behaviour change class:** major (`SegmentSeq` / `PartIndex` / `TrackId` typed ids across the public API); plus the deterministic `programs()` order.
- **cargo-semver-checks vs the live release (all features):** none flagged. (Lint-based: it misses type changes of existing fields and arguments, so the CHANGELOG BREAKING list is the primary evidence.)
- **Epoch cascade:** also forced by the sibling epochs it depends on: `broadcast-common` 9 -> 10, `timed-metadata` 0.6 -> 0.7, `transmux` 0.26 -> 0.27, `scte35-splice` 3 -> 4 (dev-dependency only).
- **Caret epoch:** 0.6 -> 0.7; every workspace dependent must move with it.

## Dependency changes

```toml
-broadcast-common = "9.4"
+broadcast-common = "10.0"
-timed-metadata = "0.6"
+timed-metadata = "0.7"
-transmux = "0.26"
+transmux = "0.27"
-scte35-splice = "3.0" # dev-dependency
+scte35-splice = "4.0" # dev-dependency
```

## Breaking changes and migration

```rust
// before
trunk.part_bytes(segment_number, part_index);          // (u32, u32)
// after
trunk.part_bytes(SegmentSeq::new(segment_number), PartIndex::new(part_index));
```

Ordering: `IngestDriver::programs()` yields programs in ascending `ProgramId` order. No code change is needed unless you relied on the previous (unspecified) hash order.

The numbers and ids affected: `SegmentEntry::sequence_number`, `PartEntry::{segment_number,part_index}` (and their `new`), `EventAnchor::Segment { segment_number }`, `Trunk::{part_bytes,parts_in_segment,events_in_segment,last_closed_segment}`, `SegmentWriter::{next_sequence_number,note_segment_start}`, `RetentionDriver::locate` and `NonMonotonicSequenceNumber::{attempted,last_published}` use the newtypes (`media_plane::ids`; `checked_next`/`checked_add`/`checked_sub`/`checked_distance_from`, explicit `wrapping_next`/`saturating_next`, no operator impls that overflow silently). `TrackSelection::track_ids`, `TrunkWriter::publish(track_id, ..)`, `SampleCursorItem::Timed { track_id }` and `SessionEvent::Sample { track_id }` use `transmux::TrackId`.

## What changed (from CHANGELOG `[Unreleased]`)

#### Documentation
- README restructured: what the crate does and does not do, a standards-supported table (edition, scope, status), feature-flag table, a doctested quick start and related crates; crate-root docs now render the README (or are kept in step with it).

#### Fixed
- `IngestDriver::programs()` iterates in ascending `ProgramId` order (the program map is a `BTreeMap`, was a `HashMap`), so the order in which a caller drives per-program work is reproducible across processes.

#### Changed (BREAKING: epoch — requires broadcast-common 10)
- Builds against new caret epochs of workspace siblings (`broadcast-common` 9 -> 10, `scte35-splice` 3 -> 4, `timed-metadata` 0.6 -> 0.7, `transmux` 0.26 -> 0.27). A caret-epoch change is major-class for this crate under the epoch-purity rule (`docs/RELEASE-AUDIT.md` §2), whatever its own API diff. `Serialize::to_bytes` was removed (use `try_to_bytes`); consumers must resolve a single `broadcast-common` 10.

#### Changed

- **BREAKING (#1188):** new `media_plane::ids::{SegmentSeq, PartIndex}` newtypes (checked `checked_next`/`checked_add`/`checked_sub`/`checked_distance_from`, explicit `wrapping_next`/`saturating_next`, no operator impls that could overflow silently). `SegmentEntry::sequence_number`, `PartEntry::segment_number`/`part_index` (and their `new` arguments), `EventAnchor::Segment { segment_number }`, `Trunk::part_bytes`/`parts_in_segment`/`events_in_segment`/`last_closed_segment`, `SegmentWriter::next_sequence_number`/`note_segment_start`, `RetentionDriver::locate` and `NonMonotonicSequenceNumber::{attempted, last_published}` use them instead of `u32`, so a swapped segment/part argument is a compile error. At `u32::MAX` `next_sequence_number` saturates and a further segment is rejected (pinned by `segment_sequence_at_u32_max_saturates_and_rejects_the_next`).

- **BREAKING (#1159):** `TrackSelection::track_ids`, `TrunkWriter::publish(track_id, ..)`, `SampleCursorItem::Timed { track_id }` and `SessionEvent::Sample { track_id }` use `transmux::TrackId` instead of `u32`.

## Merged changes since `media-plane-v0.6.0`

- fix(multimux): deterministic ProgramIds on MPTS; flush RTP depacketisers at end of input (#1332)
- docs: clear per-crate documentation for cluster B (16 crates) (#1291)
- Fix the confirmed signalling/captions review findings (supersedes #1279) (#1282)
- chore: pre-release hygiene (README coverage, snippet tool, docsrs, examples, CLAUDE.md) (#1278)
- media-plane: SegmentSeq/PartIndex newtypes across media-plane, hls-runtime, multimux (BREAKING) (#1194)
- Cleanups: remove panicking Serialize::to_bytes (BREAKING, #1143) + serde default-features off in 32 no_std manifests with CI thumbv7em serde matrix (#1183) (#1185)
- Systemic umbrellas T1/T2/T14: serializer width guards (28 crates), hostile-count DoS fix, secrets hygiene, origin-scoped credentials, secure ingest default (BREAKING items marked, NO release) (#1182)
- Audit critical-bug verification pass: 72 issues re-verified against main; fixes for #1012 #1022 #1034 #1035 #1056 + regression tests (NO release) (#1180)
- Typed ids wave 2: TrackId/ProgramNumber + HLS naming newtypes, Pid inside dvb-si (BREAKING, NO release) (#1178)
