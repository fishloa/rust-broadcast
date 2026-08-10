# timed-metadata 0.5.0

**Release date:** 2026-08-10

A bug fix in the shared 33-bit PTS wrap-unroll logic, a documentation
correction, and the workspace-wide MSRV bump. No public API change.

## What changed

- Fixed a wrap-unroll bug in the internal PTS unroller (now `PtsUnroller`,
  built on `broadcast_common::clock33::unwrap_delta`): the old forward-only
  epoch counter misclassified a legitimate small backward reorder straddling
  the 33-bit wrap origin (e.g. raw tick `2` followed by raw tick
  `2^33 - 3`) as a huge forward jump. It now uses the same bidirectional
  wrap-correction `transmux`'s demux-edge unroller already used. This also
  fixes the caption/Teletext diff-based cue boundary tracker
  (`webvtt::cue::DiffState`), which shared the same internal helper. Both
  were crate-internal, so there is no public API change. The bug requires a
  pathological input (a cue or caption event legitimately dipping backward
  across the wrap origin) and is not expected to affect ordinary broadcast
  captures, where PTS values only move forward.
- Doc accuracy fix: the crate-root doc comment claimed SCTE-35 is translated
  "to and from" both `EXT-X-DATERANGE` and `emsg`. Only the `emsg`
  conversion is bidirectional; `EXT-X-DATERANGE` conversion is one-way
  (`scte35_to_daterange` only). No behaviour change, and no data loss either
  way — raw payloads are preserved regardless of direction.
- MSRV raised to **1.95.0** (issue #949), the workspace-wide MSRV
  unification.

## Migration

No API changes; no action required. Consumers must build with rustc 1.95.0
or newer. Callers relying on the exact wrap-unroll output for a cue/event
that backward-reorders across the 33-bit PTS wrap origin will now see the
correct (small backward step) result instead of the previous incorrect
(huge forward jump) one.
