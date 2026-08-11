# Changelog — playout-runtime

All notable changes to this crate. Format: [Keep a Changelog](https://keepachangelog.com/).

## [Unreleased]

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
