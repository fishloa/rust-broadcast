# Changelog — playout-runtime

All notable changes to this crate. Format: [Keep a Changelog](https://keepachangelog.com/).

## [Unreleased]
### Fixed
- **`Schedule::is_empty` is checked while the schedule is populated.** It was only asserted empty, so a constant answer could make a host skip a populated schedule (audit 22 finding 48, #3016).

## [0.7.0] - 2026-11-15
- `scte35-splice` 6 -> 7: `SpliceTime::pts_time` is a validated `broadcast_common::clock33::Clock33`, and `SpliceInsert` carries its flags and times in one `SpliceMode` (see that crate's changelog).


### Changed
- BREAKING: `build_splice_insert` takes `requested_pts: Clock33`, `candidates: &[Clock33]`, `max_delta: Duration` and `break_duration: Option<Duration>` instead of raw `u64` tick counts (audit 19, issue #2726). It now delegates to the one `ssai_runtime::splice::condition_splice_point`; the returned `ConditionedSplicePoint` reports the reduced 33-bit instant and boundary that the emitted `splice_time().pts_time` carries, so the two can no longer disagree. `Error::BreakDurationTooLong`'s field is `nanos: u64` (the rejected duration in nanoseconds).
- BREAKING: `next_transition(schedule, now: Playhead)` takes the playhead as ticks on the schedule's own timescale (`Playhead::new(ticks, Timescale)`) instead of a bare `now_pts: u64`, so a media position cannot be read in one clock while it was produced on another (#2693).

## [0.6.0] - 2026-11-14
### Fixed
- `build_splice_insert` rejects a `break_duration_ticks` of 2^33 or more with the new `Error::BreakDurationTooLong` (`Error` is `#[non_exhaustive]`); it used to build the cue and fail only when it was serialized.

### Dependencies
- Sibling dependencies move to new caret epochs: `broadcast-common` 11 -> 12, `scte35-splice` 5 -> 6, `ssai-runtime` 0.5 -> 0.6.

## [0.5.0] - 2026-11-07
### Changed
- Error enums derive `Clone`, `PartialEq` and `Eq` wherever every field allows it (c6 C6-14).
- The crate-root docs are the README (`#![doc = include_str!("../README.md")]`) instead of a hand-synced `//!` copy (c3 DOC1).
### Dependencies
- Sibling crates move to a new epoch in this release wave: `broadcast-common` 10 -> 11, `scte35-splice` 4 -> 5, `ssai-runtime` 0.4 -> 0.5. The `Cargo.toml` requirements name the new versions; no other manifest change.

## [0.4.0] - 2026-11-01

### Changed (BREAKING: epoch)
- Builds against new caret epochs of workspace siblings (`ssai-runtime` 0.3 -> 0.4). A caret-epoch change is major-class for a 0.x crate under the epoch-purity rule (`docs/RELEASE-AUDIT.md` section 2), whatever its own API diff, so this release is a minor bump; consumers must resolve a single copy of each of those crates.

## [0.3.0] - 2026-10-17
### Documentation
- README and crate-root docs rewritten to the cluster standard: what it does / does not, standards table (SCTE 35 2023r1 `splice_insert` only), feature table, compiling quick start.

### Changed (BREAKING: epoch — requires broadcast-common 10)
- Builds against new caret epochs of workspace siblings (`broadcast-common` 9 -> 10, `mp4-emsg` 0.4 -> 0.5, `scte35-splice` 3 -> 4, `ssai-runtime` 0.2 -> 0.3). A caret-epoch change is major-class for this crate under the epoch-purity rule (`docs/RELEASE-AUDIT.md` §2), whatever its own API diff. `Serialize::to_bytes` was removed (use `try_to_bytes`); consumers must resolve a single `broadcast-common` 10.

## [0.2.1] - 2026-10-13
### Changed

- Added `TransitionPlan::rebase_wrapped` (unrolls a raw 33-bit source PTS via `clock33::unwrap_delta` before rebasing; non-breaking); added `tests/clock33_guard.rs`.

## [0.2.0] - 2026-10-05
### Fixed
- `scte35::build_splice_insert` conditions on the 33-bit SCTE-35 circle
  (`ssai_runtime::splice::condition_splice_point_wrapping`): `requested_pts`
  and the candidates are measured modulo 2^33, so a cue just before the PTS wrap
  whose nearest real boundary is just after it snaps (a 150-tick `After` snap)
  instead of failing with `NoAlignedBoundary` once per ~26.5 h; the returned
  `ConditionedSplicePoint`'s delta/direction are circular, and its
  `requested_pts`/`snapped_pts` stay in the caller's units (#1126).

### Documentation
- `ScheduleEntry::planned_start`/`source_start_pts` and
  `scte35::build_splice_insert` now document, as a mandatory precondition,
  that a value reaching `build_splice_insert` must already be a 90 kHz tick
  count (ANSI/SCTE 35 §9.8.1/§9.8.2) — previously only described as "the
  natural choice", not required, so a caller using a different channel clock
  got a structurally valid but silently wrong-by-a-fixed-factor cue with no
  error (#1126).
- `TransitionPlan::rebase` now documents that `source_pts` must already be
  unwrapped: a source whose own PTS numbering wraps mid-entry (e.g. a 33-bit
  MPEG PTS) needs unrolling (`timed_metadata::Timeline` already does this) before
  being passed here, since `rebase` is pure and has no state to detect a
  wrap itself (#1126). No behaviour change — both are documentation fixes.

## [0.1.0] - 2026-08-11

### Added
- Initial release (issue #748): sans-IO linear channel playout.
  - `schedule::Schedule` / `schedule::ScheduleEntry` / `schedule::EntryKind` —
    an ordered, strictly-increasing schedule of programme/ad/slate entries.
    The format is ours to define (no applicable open standard — SCTE 224
    covers policy/blackout signalling, not channel assembly), so this is a
    plain in-memory model with no `Parse`/`Serialize` pair.
  - `transition::TransitionPlan` / `transition::next_transition` —
    transition planning across a join: a PTS-rebase offset for timeline
    continuity, and a discontinuity flag on codec-config change. Verified
    against a real, non-IDR-aligned SCTE-35 cue
    (`fixtures/scte35-ssai/`, DASH-IF `livesim2`, Apache-2.0).
  - `scte35::build_splice_insert` / `scte35::to_section` /
    `scte35::BreakEdge` — build and serialize the `splice_insert()` a
    transition implies, with its target instant conditioned via
    `ssai_runtime::splice::condition_splice_point` (reused, not
    duplicated).
